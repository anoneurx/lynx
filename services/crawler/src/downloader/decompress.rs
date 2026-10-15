//! Bounded decompression.
//!
//! A compression bomb is a few kilobytes on the wire that expand to gigabytes in
//! memory. The byte cap alone is not enough: a naive decoder allocates the whole
//! output before the cap is checked. This decoder therefore feeds a bounded sink and
//! stops the moment **either** the decompressed byte cap or the ratio cap is crossed,
//! so the allocation stays bounded by the cap.
//!
//! Two encodings are advertised (`gzip`, `br`); anything else is rejected rather than
//! guessed at.

use std::cell::Cell;
use std::io::Read;
use std::rc::Rc;

/// Why a body could not be decompressed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecompressError {
    /// The decompressed size would exceed the cap.
    #[error("decompressed body exceeds {0} bytes")]
    TooLarge(u64),
    /// The decompressed/compressed ratio would exceed the cap.
    #[error("decompression ratio exceeds {0}:1")]
    RatioExceeded(u32),
    /// The stream was malformed, or used an encoding we do not advertise.
    #[error("malformed compressed stream: {0}")]
    Malformed(String),
    /// The `Content-Encoding` is one we never asked for.
    #[error("unsupported content-encoding {0:?}")]
    UnsupportedEncoding(String),
}

/// A reader that counts bytes pulled from the underlying source.
struct CountingReader<R> {
    inner: R,
    consumed: Rc<Cell<u64>>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.consumed
            .set(self.consumed.get().saturating_add(n as u64));
        Ok(n)
    }
}

/// Decode `raw` according to `encoding`, enforcing both caps.
///
/// `encoding` is the raw `Content-Encoding` value; the empty string and `identity`
/// mean "no encoding". `max_bytes` caps the decompressed output and `max_ratio` caps
/// the decompressed/consumed ratio.
///
/// # Errors
/// Returns [`DecompressError::TooLarge`] if the output cap would be crossed,
/// [`DecompressError::RatioExceeded`] for a bomb, or
/// [`DecompressError::UnsupportedEncoding`] / [`DecompressError::Malformed`].
#[allow(clippy::cast_precision_loss)] // the ratio comparison is inherently approximate
pub fn decode(
    encoding: &str,
    raw: &[u8],
    max_bytes: u64,
    max_ratio: u32,
) -> Result<Vec<u8>, DecompressError> {
    let encoding = encoding.trim().to_ascii_lowercase();
    if encoding.is_empty() || encoding == "identity" {
        if raw.len() as u64 > max_bytes {
            return Err(DecompressError::TooLarge(max_bytes));
        }
        return Ok(raw.to_vec());
    }

    let consumed = Rc::new(Cell::new(0_u64));
    let reader = CountingReader {
        inner: std::io::Cursor::new(raw),
        consumed: Rc::clone(&consumed),
    };

    let mut decoder: Box<dyn Read> = match encoding.as_str() {
        "gzip" | "x-gzip" => Box::new(flate2::read::GzDecoder::new(reader)),
        "br" => Box::new(brotli::Decompressor::new(reader, 32 * 1024)),
        other => return Err(DecompressError::UnsupportedEncoding(other.to_string())),
    };

    let mut out: Vec<u8> = Vec::new();
    // Heap-allocated: a 32 KiB stack array here would be copied into every call frame.
    let mut scratch = vec![0_u8; 32 * 1024];
    loop {
        let n = decoder
            .read(&mut scratch)
            .map_err(|error| DecompressError::Malformed(error.to_string()))?;
        if n == 0 {
            break;
        }

        let projected = out.len() as u64 + n as u64;
        if projected > max_bytes {
            return Err(DecompressError::TooLarge(max_bytes));
        }
        out.extend_from_slice(&scratch[..n]);

        let source = consumed.get();
        if source > 0 && out.len() as f64 / source as f64 > f64::from(max_ratio) {
            return Err(DecompressError::RatioExceeded(max_ratio));
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn brotli(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut writer = brotli::CompressorWriter::new(&mut out, 4096, 6, 22);
            writer.write_all(data).unwrap();
        }
        out
    }

    #[test]
    fn identity_is_a_pass_through() {
        let out = decode("", b"hello", 1024, 100).unwrap();
        assert_eq!(out, b"hello");
        let out = decode("identity", b"hello", 1024, 100).unwrap();
        assert_eq!(out, b"hello");
    }

    #[test]
    fn identity_respects_the_cap() {
        let error = decode("", &[0u8; 100], 10, 100).unwrap_err();
        assert_eq!(error, DecompressError::TooLarge(10));
    }

    #[test]
    fn gzip_round_trips() {
        let payload = b"the quick brown fox".repeat(20);
        let raw = gzip(&payload);
        let out = decode("gzip", &raw, 1 << 20, 1000).unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn brotli_round_trips() {
        let payload = b"the quick brown fox".repeat(20);
        let raw = brotli(&payload);
        let out = decode("br", &raw, 1 << 20, 1000).unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn bomb_trips_the_ratio_cap() {
        // 1 MiB of zeros compresses to well under 2 KiB.
        let raw = gzip(&vec![0_u8; 1024 * 1024]);
        assert!(raw.len() < 4 * 1024, "compressed size {}", raw.len());
        let error = decode("gzip", &raw, 64 * 1024 * 1024, 100).unwrap_err();
        assert_eq!(error, DecompressError::RatioExceeded(100));
    }

    #[test]
    fn bomb_trips_the_byte_cap_first_when_tighter() {
        let raw = gzip(&vec![0_u8; 1024 * 1024]);
        let error = decode("gzip", &raw, 4 * 1024, 10_000).unwrap_err();
        assert_eq!(error, DecompressError::TooLarge(4 * 1024));
    }

    #[test]
    fn malformed_stream_is_rejected() {
        let error = decode("gzip", b"not gzip at all", 1 << 20, 100).unwrap_err();
        assert!(matches!(error, DecompressError::Malformed(_)));
    }

    #[test]
    fn unknown_encoding_is_rejected() {
        let error = decode("zstd", b"whatever", 1 << 20, 100).unwrap_err();
        assert_eq!(
            error,
            DecompressError::UnsupportedEncoding("zstd".to_string())
        );
    }
}
