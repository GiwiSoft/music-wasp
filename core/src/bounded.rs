//! Reads from untrusted sources (feeds, sidecar files, remote images) with a
//! hard size limit, so a hostile or broken file cannot exhaust memory.

use std::io::{self, Read};

/// Read at most `limit` bytes. Returns `InvalidData` when the source is longer,
/// rather than silently truncating it.
pub fn read_capped<R: Read>(reader: R, limit: u64) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    reader.take(limit + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("input exceeds {limit} bytes"),
        ));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_input_within_limit() {
        let data = read_capped(&b"hello"[..], 5).unwrap();
        assert_eq!(data, b"hello");
    }

    #[test]
    fn rejects_input_over_limit() {
        let error = read_capped(&b"hello!"[..], 5).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
