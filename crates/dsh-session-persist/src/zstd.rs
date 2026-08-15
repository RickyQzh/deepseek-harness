//! Concatenated Zstandard frames for session artifacts.

use std::io::{Read, Write};

use crate::PersistError;

fn zstd_error(error: impl std::fmt::Display) -> PersistError {
    PersistError::Corrupt(error.to_string())
}

/// Compress `plain` as one independently decodable, checksummed Zstandard frame.
///
/// # Errors
///
/// [`PersistError::Corrupt`] when the zstd encoder fails.
pub fn compress_zstd_frame(plain: &[u8]) -> Result<Vec<u8>, PersistError> {
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 0).map_err(zstd_error)?;
    encoder.include_checksum(true).map_err(zstd_error)?;
    encoder.write_all(plain).map_err(zstd_error)?;
    encoder.finish().map_err(zstd_error)
}

/// Decompress concatenated Zstandard frames in order.
///
/// Each frame must start with magic `28 B5 2F FD` (little-endian `0xFD2FB528`).
/// An empty buffer yields empty plaintext.
///
/// # Errors
///
/// [`PersistError::Corrupt`] when magic is missing at the current offset, a
/// frame fails to decode, or a decoder consumes no bytes.
pub fn decompress_zstd_frames(encoded: &[u8]) -> Result<Vec<u8>, PersistError> {
    const MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
    let mut offset = 0usize;
    let mut out = Vec::new();
    if encoded.is_empty() {
        return Ok(out);
    }
    while offset < encoded.len() {
        if encoded.len() - offset < 4 || encoded[offset..offset + 4] != MAGIC {
            return Err(PersistError::Corrupt(format!(
                "corrupt Zstandard session log: invalid frame magic at byte {offset}"
            )));
        }
        let remaining = &encoded[offset..];
        let mut decoder = zstd::stream::read::Decoder::with_buffer(remaining)
            .map_err(zstd_error)?
            .single_frame();
        decoder.read_to_end(&mut out).map_err(zstd_error)?;
        let consumed = remaining.len() - decoder.get_ref().len();
        if consumed == 0 {
            return Err(PersistError::Corrupt(
                "zstd decoder consumed no bytes".into(),
            ));
        }
        offset += consumed;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{compress_zstd_frame, decompress_zstd_frames};

    #[test]
    fn one_frame_round_trips_plaintext() {
        let plain = b"{\"type\":\"session\"}\n";
        let encoded = compress_zstd_frame(plain).expect("compress");
        assert_eq!(&encoded[..4], &[0x28, 0xB5, 0x2F, 0xFD]);
        let decoded = decompress_zstd_frames(&encoded).expect("decompress");
        assert_eq!(decoded, plain);
    }

    #[test]
    fn two_concatenated_frames_decode_in_order() {
        let a = compress_zstd_frame(b"AAA\n").expect("a");
        let b = compress_zstd_frame(b"BBB\n").expect("b");
        let mut both = a;
        both.extend_from_slice(&b);
        let decoded = decompress_zstd_frames(&both).expect("concat");
        assert_eq!(decoded, b"AAA\nBBB\n");
    }

    #[test]
    fn invalid_magic_is_corrupt() {
        let error = decompress_zstd_frames(&[0, 1, 2, 3]).expect_err("magic");
        assert!(error.to_string().contains("invalid frame magic at byte 0"));
    }
}
