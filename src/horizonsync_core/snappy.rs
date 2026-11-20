//! Raw Snappy (the block format `org.xerial.snappy.Snappy.compress` writes, not the framed one),
//! through the `snap` crate (Horizon's `snappy` feature of `horizon-mod-deps`).
//!
//! HuskSync compresses every snapshot with it when `compress_data` is on.

/// Largest uncompressed snapshot read (a corrupt length cannot allocate more).
const MAX_LEN: usize = 256 * 1024 * 1024;

pub fn decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    let len = snap::raw::decompress_len(input).map_err(|e| format!("snappy: {e}"))?;
    if len > MAX_LEN {
        return Err("snappy: length too large".into());
    }
    snap::raw::Decoder::new().decompress_vec(input).map_err(|e| format!("snappy: {e}"))
}

pub fn compress(input: &[u8]) -> Vec<u8> {
    snap::raw::Encoder::new().compress_vec(input).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut data = Vec::new();
        for i in 0..5000 {
            data.extend_from_slice(format!("{{\"slot\":{},\"id\":\"minecraft:stone\"}}", i % 37).as_bytes());
        }
        let c = compress(&data);
        assert!(c.len() < data.len() / 3);
        assert_eq!(decompress(&c).unwrap(), data);
        assert_eq!(decompress(&compress(b"")).unwrap(), b"");
        assert_eq!(decompress(&compress(b"abc")).unwrap(), b"abc");
    }

    #[test]
    fn reads_reference_encoding() {
        // "Wikipedia" example from the format description: literal then a 1-byte-offset copy.
        // 0x0b = length 11; literal "abcd" (tag 0x0c), copy len 7 offset 4 (tag 0x0d = (3<<2)|1), 0x04
        let enc = [0x0b, 0x0c, b'a', b'b', b'c', b'd', 0x0d, 0x04];
        assert_eq!(decompress(&enc).unwrap(), b"abcdabcdabc");
    }

    #[test]
    fn rejects_bad_offsets() {
        assert!(decompress(&[0x04, 0x0d, 0x04]).is_err());
    }
}
