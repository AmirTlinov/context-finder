use rmcp::service::{RxJsonRpcMessage, ServiceRole, TxJsonRpcMessage};
use rmcp::transport::Transport;
use std::borrow::Cow;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Framing {
    Unknown,
    NewlineJson,
    ContentLength,
}

const MAX_BUFFER_BYTES: usize = if cfg!(test) { 4096 } else { 32 * 1024 * 1024 };
const MAX_MESSAGE_BYTES: usize = if cfg!(test) { 1024 } else { 16 * 1024 * 1024 };

const fn is_ascii_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn strip_leading_whitespace(buf: &mut Vec<u8>) {
    let first_non_ws = buf.iter().position(|b| !is_ascii_whitespace(*b));
    match first_non_ws {
        None => buf.clear(),
        Some(0) => {}
        Some(n) => {
            buf.drain(..n);
        }
    }
}

fn strip_utf8_bom(buf: &mut Vec<u8>) {
    const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    if buf.starts_with(BOM) {
        buf.drain(..BOM.len());
    }
}

fn starts_with_content_length(buf: &[u8]) -> bool {
    const PREFIX: &[u8] = b"content-length:";
    if buf.len() < PREFIX.len() {
        return false;
    }
    buf[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
}

fn find_double_newline(buf: &[u8]) -> Option<(usize, usize)> {
    // Returns: (header_end_index, newline_width)
    // Prefer CRLFCRLF, fall back to LFLF.
    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((pos + 4, 4));
    }
    if let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
        return Some((pos + 2, 2));
    }
    None
}

fn parse_content_length(headers: &str) -> Option<usize> {
    for raw_line in headers.lines() {
        let line = raw_line.trim_end_matches('\r').trim();
        if line.len() < "content-length:".len() {
            continue;
        }
        if line.as_bytes()[.."content-length:".len()].eq_ignore_ascii_case(b"content-length:") {
            let value = line["content-length:".len()..].trim();
            if let Ok(n) = value.parse::<usize>() {
                return Some(n);
            }
        }
    }
    None
}

/// Hybrid stdio transport that supports both:
/// - newline-delimited JSON-RPC (one JSON object per line)
/// - LSP-style `Content-Length: N\r\n\r\n<json>` framing
///
/// It auto-detects framing from the first non-whitespace bytes received.
pub struct HybridStdioTransport<Role: ServiceRole, R: AsyncRead, W: AsyncWrite> {
    read: R,
    write_tx: Option<mpsc::Sender<WriteRequest>>,
    write_task: Option<tokio::task::JoinHandle<()>>,
    buf: Vec<u8>,
    framing: Framing,
    _marker: PhantomData<fn() -> (Role, W)>,
}

struct WriteRequest {
    bytes: Vec<u8>,
    reply: oneshot::Sender<io::Result<()>>,
}

async fn run_write_loop<W: AsyncWrite + Unpin>(mut write: W, mut rx: mpsc::Receiver<WriteRequest>) {
    while let Some(req) = rx.recv().await {
        let result = async {
            write.write_all(&req.bytes).await?;
            write.flush().await?;
            Ok(())
        }
        .await;
        let should_stop = result.is_err();
        let _ = req.reply.send(result);
        if should_stop {
            break;
        }
    }
}

impl<Role: ServiceRole, R: AsyncRead + Unpin + Send, W: AsyncWrite + Unpin + Send + 'static>
    HybridStdioTransport<Role, R, W>
{
    pub fn new(read: R, write: W) -> Self {
        let (write_tx, write_rx) = mpsc::channel::<WriteRequest>(16);
        let write_task = tokio::spawn(run_write_loop(write, write_rx));
        Self {
            read,
            write_tx: Some(write_tx),
            write_task: Some(write_task),
            buf: Vec::new(),
            framing: Framing::Unknown,
            _marker: PhantomData,
        }
    }

    fn detect_framing(&mut self) {
        if self.framing != Framing::Unknown {
            return;
        }
        strip_utf8_bom(&mut self.buf);
        strip_leading_whitespace(&mut self.buf);
        if self.buf.is_empty() {
            return;
        }
        if starts_with_content_length(&self.buf) {
            self.framing = Framing::ContentLength;
            return;
        }
        // Heuristic: compact JSON messages start with '{' or '['.
        if matches!(self.buf[0], b'{' | b'[') {
            self.framing = Framing::NewlineJson;
            return;
        }
        // Fallback: treat as newline JSON; we will still tolerate garbage lines.
        self.framing = Framing::NewlineJson;
    }

    fn try_decode_newline(&mut self) -> Result<Option<RxJsonRpcMessage<Role>>, io::Error> {
        loop {
            let Some(nl) = self.buf.iter().position(|b| *b == b'\n') else {
                return Ok(None);
            };
            let mut line = self.buf.drain(..=nl).collect::<Vec<u8>>();
            if matches!(line.last(), Some(b'\n')) {
                line.pop();
            }
            if matches!(line.last(), Some(b'\r')) {
                line.pop();
            }

            // Skip empty/whitespace-only lines (compat).
            let trimmed = line
                .iter()
                .skip_while(|b| is_ascii_whitespace(**b))
                .copied()
                .collect::<Vec<u8>>();
            if trimmed.is_empty() {
                continue;
            }

            // If we see a Content-Length header while in newline mode, switch modes and requeue.
            if starts_with_content_length(&trimmed) {
                let mut rebuilt = trimmed;
                rebuilt.push(b'\n');
                rebuilt.extend_from_slice(&self.buf);
                self.buf = rebuilt;
                self.framing = Framing::ContentLength;
                return self.try_decode();
            }

            match serde_json::from_slice::<RxJsonRpcMessage<Role>>(&trimmed) {
                Ok(msg) => return Ok(Some(msg)),
                Err(err) => {
                    // Compat: ignore non-JSON garbage lines (but keep strict for JSON-looking lines).
                    if matches!(trimmed.first(), Some(b'{' | b'[')) {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, err));
                    }
                }
            }
        }
    }

    fn try_decode_content_length(&mut self) -> Result<Option<RxJsonRpcMessage<Role>>, io::Error> {
        let Some((header_end, _width)) = find_double_newline(&self.buf) else {
            return Ok(None);
        };
        let header_bytes = self.buf[..header_end].to_vec();
        let header_str = std::str::from_utf8(&header_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let Some(len) = parse_content_length(header_str) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing Content-Length header",
            ));
        };

        if len > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Content-Length {len} exceeds maximum supported message size {MAX_MESSAGE_BYTES}"
                ),
            ));
        }
        if header_end + len > MAX_BUFFER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "message size {} exceeds maximum buffer size {MAX_BUFFER_BYTES}",
                    header_end + len
                ),
            ));
        }

        if self.buf.len() < header_end + len {
            return Ok(None);
        }

        let body = self.buf[header_end..header_end + len].to_vec();
        self.buf.drain(..header_end + len);

        let msg = serde_json::from_slice::<RxJsonRpcMessage<Role>>(&body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Some(msg))
    }

    fn try_decode(&mut self) -> Result<Option<RxJsonRpcMessage<Role>>, io::Error> {
        self.detect_framing();

        match self.framing {
            Framing::Unknown => Ok(None),
            Framing::NewlineJson => self.try_decode_newline(),
            Framing::ContentLength => self.try_decode_content_length(),
        }
    }
}

impl<Role: ServiceRole, R, W> Transport<Role> for HybridStdioTransport<Role, R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Error = io::Error;

    fn name() -> Cow<'static, str> {
        "HybridStdioTransport".into()
    }

    fn send(
        &mut self,
        item: TxJsonRpcMessage<Role>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let framing = self.framing;
        let write_tx = self.write_tx.clone();

        async move {
            let Some(write_tx) = write_tx else {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "transport closed",
                ));
            };
            let json = serde_json::to_vec(&item).map_err(io::Error::other)?;

            let mut out = Vec::new();
            match framing {
                Framing::ContentLength => {
                    out.extend_from_slice(
                        format!("Content-Length: {}\r\n\r\n", json.len()).as_bytes(),
                    );
                    out.extend_from_slice(&json);
                }
                Framing::Unknown | Framing::NewlineJson => {
                    out.extend_from_slice(&json);
                    out.push(b'\n');
                }
            }

            let (reply_tx, reply_rx) = oneshot::channel::<io::Result<()>>();
            write_tx
                .send(WriteRequest {
                    bytes: out,
                    reply: reply_tx,
                })
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "transport closed"))?;
            reply_rx
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "transport closed"))??;
            Ok(())
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<Role>> {
        loop {
            match self.try_decode() {
                Ok(Some(msg)) => return Some(msg),
                Ok(None) => {}
                Err(err) => {
                    // Mirror rmcp behavior: log and terminate the stream.
                    log::error!("Error reading from stream: {err}");
                    return None;
                }
            }

            let mut tmp = [0u8; 8192];
            let n = match self.read.read(&mut tmp).await {
                Ok(n) => n,
                Err(err) => {
                    log::error!("Error reading from stream: {err}");
                    return None;
                }
            };
            if n == 0 {
                return None;
            }
            self.buf.extend_from_slice(&tmp[..n]);
            if self.buf.len() > MAX_BUFFER_BYTES {
                log::error!(
                    "Input buffer exceeded maximum size ({} > {MAX_BUFFER_BYTES}); closing transport",
                    self.buf.len()
                );
                return None;
            }
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let write_task = self.write_task.take();
        self.write_tx.take();
        async move {
            if let Some(task) = write_task {
                task.abort();
                let _ = task.await;
            }
            Ok(())
        }
    }
}

pub fn stdio_hybrid_server(
) -> HybridStdioTransport<rmcp::RoleServer, tokio::io::Stdin, tokio::io::Stdout> {
    HybridStdioTransport::new(tokio::io::stdin(), tokio::io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, DuplexStream};

    fn split_duplex(
        stream: DuplexStream,
    ) -> (
        tokio::io::ReadHalf<DuplexStream>,
        tokio::io::WriteHalf<DuplexStream>,
    ) {
        tokio::io::split(stream)
    }

    #[tokio::test]
    async fn rejects_excessive_content_length() {
        let (mut client, server) = tokio::io::duplex(16_384);
        let (read, write) = split_duplex(server);
        let mut transport = HybridStdioTransport::<rmcp::RoleServer, _, _>::new(read, write);

        client
            .write_all(b"Content-Length: 999999\r\n\r\n")
            .await
            .expect("write header");
        client.flush().await.expect("flush");
        drop(client);

        let msg = transport.receive().await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn closes_on_newline_mode_buffer_overflow() {
        let (mut client, server) = tokio::io::duplex(16_384);
        let (read, write) = split_duplex(server);
        let mut transport = HybridStdioTransport::<rmcp::RoleServer, _, _>::new(read, write);

        let payload = vec![b'a'; MAX_BUFFER_BYTES + 1];
        client.write_all(&payload).await.expect("write payload");
        client.flush().await.expect("flush");
        drop(client);

        let msg = transport.receive().await;
        assert!(msg.is_none());
    }
}
