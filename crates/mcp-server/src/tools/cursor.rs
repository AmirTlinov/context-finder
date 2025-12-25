use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub const CURSOR_VERSION: u32 = 1;

const MAX_CURSOR_BASE64_CHARS: usize = 8_192;
const MAX_CURSOR_JSON_BYTES: usize = 4_096;

pub fn encode_cursor<T: Serialize>(cursor: &T) -> Result<String> {
    let bytes = serde_json::to_vec(cursor).context("serialize cursor")?;
    if bytes.len() > MAX_CURSOR_JSON_BYTES {
        anyhow::bail!("Cursor payload too large ({} bytes)", bytes.len());
    }
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub fn decode_cursor<T: DeserializeOwned>(cursor: &str) -> Result<T> {
    let cursor = cursor.trim();
    if cursor.is_empty() {
        anyhow::bail!("Cursor must not be empty");
    }
    if cursor.len() > MAX_CURSOR_BASE64_CHARS {
        anyhow::bail!("Cursor too long");
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor.as_bytes())
        .context("decode cursor")?;
    if bytes.len() > MAX_CURSOR_JSON_BYTES {
        anyhow::bail!("Cursor payload too large ({} bytes)", bytes.len());
    }
    serde_json::from_slice(&bytes).context("parse cursor json")
}
