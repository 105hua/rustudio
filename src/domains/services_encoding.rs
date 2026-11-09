//! EncodingService: base64, hashes and zstd, byte-for-byte as Studio 0.744
//! (offline Studio): ComputeStringHash returns the raw digest bytes as a string,
//! ComputeBufferHash a buffer; Blake2b is BLAKE2b-256; CompressBuffer writes
//! a single zstd frame with the content size and no checksum (zstd's own
//! defaults), levels -7 to 22 (others: "unsupported compression level").

use blake2::digest::consts::U32;
use blake2::Digest;

pub fn base64_encode(input: &[u8]) -> Vec<u8> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(input.len().div_ceil(3) * 4);
    for c in input.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63]);
        out.push(T[(n >> 12) as usize & 63]);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if c.len() > 2 { T[n as usize & 63] } else { b'=' });
    }
    out
}

/// Strict: whole groups of four, '=' only as padding at the end (Studio
/// refuses "aGVsbG8" without its padding).
pub fn base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    if input.len() % 4 != 0 {
        return None;
    }
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let groups = input.len() / 4;
    for (gi, g) in input.chunks(4).enumerate() {
        let last = gi + 1 == groups;
        let pad = g.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (i, &c) in g.iter().enumerate() {
            let v = if i >= 4 - pad { 0 } else { val(c)? };
            n = n << 6 | v;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// HashAlgorithm by value: Blake2b 0, Blake3 1, Md5 2, Sha1 3, Sha256 4.
pub fn hash(alg: u32, data: &[u8]) -> Option<Vec<u8>> {
    Some(match alg {
        0 => blake2::Blake2b::<U32>::digest(data).to_vec(),
        1 => blake3::hash(data).as_bytes().to_vec(),
        2 => md5::Md5::digest(data).to_vec(),
        3 => sha1::Sha1::digest(data).to_vec(),
        4 => sha2::Sha256::digest(data).to_vec(),
        _ => return None,
    })
}

pub fn compress(data: &[u8], level: i32) -> Result<Vec<u8>, String> {
    if !(-7..=22).contains(&level) {
        return Err("CompressBuffer: unsupported compression level for Zstd".into());
    }
    zstd::bulk::compress(data, level).map_err(|e| format!("CompressBuffer: {e}"))
}

pub fn decompressed_size(data: &[u8]) -> Option<u64> {
    zstd::zstd_safe::get_frame_content_size(data).ok().flatten()
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    let fail = || "DecompressBuffer: decompression failed - Unspecified error code".to_string();
    let size = decompressed_size(data).ok_or_else(fail)?;
    // A frame claiming more than Luau's buffer limit is refused up front.
    if size > 1 << 30 {
        return Err(fail());
    }
    zstd::bulk::decompress(data, size as usize).map_err(|_| fail())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    #[test]
    fn base64_matches_studio() {
        assert_eq!(base64_encode(b"hello world"), b"aGVsbG8gd29ybGQ=");
        assert_eq!(base64_decode(b"aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert!(base64_decode(b"aGVsbG8").is_none());
        assert!(base64_decode(b"!!!").is_none());
        assert_eq!(base64_decode(b"").unwrap(), b"");
    }
    #[test]
    fn hashes_match_studio() {
        // Studio 0.744 ComputeBufferHash(buffer.fromstring("hello world"), …).
        let d = b"hello world";
        assert_eq!(hex(&hash(2, d).unwrap()), "5eb63bbbe01eeed093cb22bb8f5acdc3");
        assert_eq!(hex(&hash(3, d).unwrap()), "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
        assert_eq!(hex(&hash(4, d).unwrap()), "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
        assert_eq!(hex(&hash(0, d).unwrap()), "256c83b297114d201b30179f3f0ef0cace9783622da5974326b436178aeef610");
        assert_eq!(hex(&hash(1, d).unwrap()), "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24");
    }
    #[test]
    fn zstd_frame_matches_studio() {
        // Studio: CompressBuffer("hello world") = 28b52ffd 20 0b 590000 "hello world".
        let c = compress(b"hello world", 1).unwrap();
        assert_eq!(hex(&c[..9]), "28b52ffd200b590000");
        assert_eq!(decompress(&c).unwrap(), b"hello world");
        assert_eq!(decompressed_size(&c), Some(11));
        assert!(compress(b"x", 23).is_err() && compress(b"x", -8).is_err());
        assert!(decompress(b"hello world").is_err());
        // Studio: 300 × "abc" at level 3 = 28b52ffd602c0055000018616263010026aa6e08.
        let c = compress("abc".repeat(100).as_bytes(), 3).unwrap();
        assert_eq!(hex(&c), "28b52ffd602c0055000018616263010026aa6e08");
    }
}
