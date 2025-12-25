use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

pub(super) fn path_has_extension_ignore_ascii_case(path: &str, ext: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

pub(super) fn hex_encode_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}
