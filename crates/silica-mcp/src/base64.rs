//! Standard base64, for the MCP `image` content block.
//!
//! A screenshot is the only binary this server ever sends, and the stdio
//! transport is line-delimited JSON, so the encoder has one job: the standard
//! alphabet with `=` padding. That is not worth a dependency.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `bytes` in standard base64, padded to a multiple of four characters.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = u32::from_be_bytes([0, chunk[0], b1, b2]);
        let sextet = |i: u32| ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char;
        out.push(sextet(0));
        out.push(sextet(1));
        out.push(if chunk.len() > 1 { sextet(2) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(3) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_rfc_4648_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encodes_the_whole_byte_range() {
        let all: Vec<u8> = (0..=255).collect();
        let encoded = encode(&all);
        assert_eq!(encoded.len(), 344);
        // The high bytes are where a sign-extending shift would show up.
        assert!(encoded.starts_with("AAECAwQFBgcICQoLDA0ODxAREhMUFRYX"));
        assert!(encoded.ends_with("8PHy8/T19vf4+fr7/P3+/w=="));
    }

    #[test]
    fn uses_only_the_standard_alphabet() {
        let png = [0x89u8, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0xfe];
        assert!(encode(&png)
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c)));
    }
}
