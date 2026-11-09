//! Texture decoding to RGBA8: PNG and JPEG through the `image` crate, DDS
//! (BC1/BC2/BC3 and uncompressed RGB(A)) with a small built-in decoder.
//!
//! Only the top mip level / first array slice of a DDS is decoded. All reads
//! are bounds-checked; malformed input yields `Err(String)`, never a panic.

/// A decoded image: RGBA8 pixels, row-major, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// Decodes PNG, JPEG or DDS bytes, detected by magic number.
pub fn decode(bytes: &[u8]) -> Result<Rgba, String> {
    if bytes.starts_with(b"\x89PNG") {
        decode_with_image(bytes, image::ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        decode_with_image(bytes, image::ImageFormat::Jpeg)
    } else if bytes.starts_with(b"DDS ") {
        decode_dds(bytes)
    } else {
        let head: Vec<String> = bytes.iter().take(4).map(|b| format!("{b:02x}")).collect();
        Err(format!(
            "texture: unrecognised format (first bytes: {})",
            head.join(" ")
        ))
    }
}

fn decode_with_image(bytes: &[u8], format: image::ImageFormat) -> Result<Rgba, String> {
    let img = image::load_from_memory_with_format(bytes, format)
        .map_err(|e| format!("texture: {format:?} decode failed: {e}"))?
        .into_rgba8();
    Ok(Rgba {
        width: img.width(),
        height: img.height(),
        pixels: img.into_raw(),
    })
}

// ---------------------------------------------------------------------------
// DDS
// ---------------------------------------------------------------------------

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x20000;

/// Pixel layout of the top mip level.
enum DdsFormat {
    Bc1,
    Bc2,
    Bc3,
    /// Uncompressed, `bytes` per pixel, channel masks R, G, B, A (A may be 0).
    Masked {
        bytes: usize,
        masks: [u32; 4],
        luminance: bool,
    },
}

fn le32(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "dds: truncated header".to_string())
}

/// Parses the `DDS ` magic, 124-byte header and optional DX10 header, then
/// decodes the first mip level.
fn decode_dds(bytes: &[u8]) -> Result<Rgba, String> {
    // Offsets below are relative to the file start (header begins at 4).
    if le32(bytes, 4)? != 124 {
        return Err("dds: bad header size".into());
    }
    let height = le32(bytes, 12)?;
    let width = le32(bytes, 16)?;
    let pf_flags = le32(bytes, 80)?;
    let four_cc = bytes.get(84..88).ok_or("dds: truncated header")?;
    let bit_count = le32(bytes, 88)?;
    let masks = [
        le32(bytes, 92)?,
        le32(bytes, 96)?,
        le32(bytes, 100)?,
        le32(bytes, 104)?,
    ];
    let mut data_off = 128;
    if width == 0 || height == 0 {
        return Err(format!("dds: invalid size {width}x{height}"));
    }

    let format = if pf_flags & DDPF_FOURCC != 0 {
        match four_cc {
            b"DXT1" => DdsFormat::Bc1,
            b"DXT2" | b"DXT3" => DdsFormat::Bc2,
            b"DXT4" | b"DXT5" => DdsFormat::Bc3,
            b"DX10" => {
                let dxgi = le32(bytes, 128).map_err(|_| "dds: truncated DX10 header")?;
                bytes.get(128..148).ok_or("dds: truncated DX10 header")?;
                data_off = 148;
                match dxgi {
                    71 | 72 => DdsFormat::Bc1,
                    74 | 75 => DdsFormat::Bc2,
                    77 | 78 => DdsFormat::Bc3,
                    // R8G8B8A8 UNORM(/SRGB)
                    28 | 29 => DdsFormat::Masked {
                        bytes: 4,
                        masks: [0xFF, 0xFF00, 0xFF_0000, 0xFF00_0000],
                        luminance: false,
                    },
                    // B8G8R8A8 UNORM(/SRGB), B8G8R8X8 UNORM(/SRGB)
                    87 | 91 | 88 | 93 => DdsFormat::Masked {
                        bytes: 4,
                        masks: [
                            0xFF_0000,
                            0xFF00,
                            0xFF,
                            if matches!(dxgi, 87 | 91) {
                                0xFF00_0000
                            } else {
                                0
                            },
                        ],
                        luminance: false,
                    },
                    other => return Err(format!("dds: unsupported DXGI format {other}")),
                }
            }
            other => {
                return Err(format!(
                    "dds: unsupported FourCC {:?}",
                    String::from_utf8_lossy(other)
                ));
            }
        }
    } else if pf_flags & (DDPF_RGB | DDPF_LUMINANCE) != 0 {
        if bit_count == 0 || bit_count > 32 || bit_count % 8 != 0 {
            return Err(format!("dds: unsupported bit count {bit_count}"));
        }
        let mut masks = masks;
        if pf_flags & DDPF_ALPHAPIXELS == 0 {
            masks[3] = 0;
        }
        DdsFormat::Masked {
            bytes: bit_count as usize / 8,
            masks,
            luminance: pf_flags & DDPF_RGB == 0,
        }
    } else {
        return Err(format!("dds: unsupported pixel format flags {pf_flags:#x}"));
    };

    let data = &bytes[data_off.min(bytes.len())..];
    let (w, h) = (width as usize, height as usize);
    match format {
        DdsFormat::Bc1 => decode_bc(data, w, h, 8, |b, out| decode_bc1_color(b, out, true)),
        DdsFormat::Bc2 => decode_bc(data, w, h, 16, |b, out| {
            decode_bc1_color(&b[8..], out, false);
            let bits = u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
            for (i, px) in out.iter_mut().enumerate() {
                px[3] = ((bits >> (4 * i)) & 0xF) as u8 * 17;
            }
        }),
        DdsFormat::Bc3 => decode_bc(data, w, h, 16, |b, out| {
            decode_bc1_color(&b[8..], out, false);
            decode_bc3_alpha(&b[..8], out);
        }),
        DdsFormat::Masked {
            bytes: bpp,
            masks,
            luminance,
        } => decode_masked(data, w, h, bpp, masks, luminance),
    }
    .map(|pixels| Rgba {
        width,
        height,
        pixels,
    })
}

/// Decodes a block-compressed image: `block_bytes` per 4x4 block, rows of
/// blocks top to bottom. Edge blocks are clipped to the image size.
fn decode_bc(
    data: &[u8],
    w: usize,
    h: usize,
    block_bytes: usize,
    block: impl Fn(&[u8], &mut [[u8; 4]; 16]),
) -> Result<Vec<u8>, String> {
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let need = bw
        .checked_mul(bh)
        .and_then(|n| n.checked_mul(block_bytes))
        .ok_or("dds: image too large")?;
    if data.len() < need {
        return Err(format!(
            "dds: truncated pixel data ({} of {need} bytes)",
            data.len()
        ));
    }
    let mut out = vec![0u8; w * h * 4];
    let mut texels = [[0u8; 4]; 16];
    for (bi, src) in data[..need].chunks_exact(block_bytes).enumerate() {
        let (bx, by) = (bi % bw * 4, bi / bw * 4);
        block(src, &mut texels);
        for py in 0..4.min(h - by) {
            for px in 0..4.min(w - bx) {
                let o = ((by + py) * w + bx + px) * 4;
                out[o..o + 4].copy_from_slice(&texels[py * 4 + px]);
            }
        }
    }
    Ok(out)
}

/// Expands RGB565 to RGB888.
fn rgb565(c: u16) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [
        ((r << 3) | (r >> 2)) as u8,
        ((g << 2) | (g >> 4)) as u8,
        ((b << 3) | (b >> 2)) as u8,
    ]
}

/// Decodes an 8-byte BC1 colour block. With `allow_alpha` (plain BC1) and
/// `c0 <= c1`, index 3 is transparent black; BC2/BC3 always use 4 colours.
fn decode_bc1_color(b: &[u8], out: &mut [[u8; 4]; 16], allow_alpha: bool) {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |a: u8, b: u8, wa: u16, wb: u16| ((a as u16 * wa + b as u16 * wb) / (wa + wb)) as u8;
    let mut pal = [
        [p0[0], p0[1], p0[2], 255],
        [p1[0], p1[1], p1[2], 255],
        [0; 4],
        [0; 4],
    ];
    if c0 > c1 || !allow_alpha {
        for k in 0..3 {
            pal[2][k] = mix(p0[k], p1[k], 2, 1);
            pal[3][k] = mix(p0[k], p1[k], 1, 2);
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for k in 0..3 {
            pal[2][k] = mix(p0[k], p1[k], 1, 1);
        }
        pal[2][3] = 255; // pal[3] stays transparent black
    }
    let idx = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    for (i, px) in out.iter_mut().enumerate() {
        *px = pal[((idx >> (2 * i)) & 3) as usize];
    }
}

/// Decodes an 8-byte BC3 alpha block into the alpha channel of `out`.
fn decode_bc3_alpha(b: &[u8], out: &mut [[u8; 4]; 16]) {
    let (a0, a1) = (b[0] as u16, b[1] as u16);
    let mut pal = [0u8; 8];
    pal[0] = a0 as u8;
    pal[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7u16 {
            pal[i as usize + 1] = (((7 - i) * a0 + i * a1) / 7) as u8;
        }
    } else {
        for i in 1..5u16 {
            pal[i as usize + 1] = (((5 - i) * a0 + i * a1) / 5) as u8;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let mut bits = [0u8; 8];
    bits[..6].copy_from_slice(&b[2..8]);
    let bits = u64::from_le_bytes(bits);
    for (i, px) in out.iter_mut().enumerate() {
        px[3] = pal[((bits >> (3 * i)) & 7) as usize];
    }
}

/// Extracts a channel by mask and rescales it to 8 bits (`None` if no mask).
fn channel(px: u32, mask: u32) -> Option<u8> {
    if mask == 0 {
        return None;
    }
    let shift = mask.trailing_zeros();
    let max = (mask >> shift) as u64;
    let v = ((px & mask) >> shift) as u64;
    Some(((v * 255 + max / 2) / max) as u8)
}

/// Decodes tightly packed uncompressed pixels using the format's bit masks.
fn decode_masked(
    data: &[u8],
    w: usize,
    h: usize,
    bpp: usize,
    masks: [u32; 4],
    luminance: bool,
) -> Result<Vec<u8>, String> {
    let need = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(bpp))
        .ok_or("dds: image too large")?;
    if data.len() < need {
        return Err(format!(
            "dds: truncated pixel data ({} of {need} bytes)",
            data.len()
        ));
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for src in data[..need].chunks_exact(bpp) {
        let mut word = [0u8; 4];
        word[..bpp].copy_from_slice(src);
        let px = u32::from_le_bytes(word);
        let r = channel(px, masks[0]).unwrap_or(0);
        let (g, b) = if luminance {
            (r, r)
        } else {
            (
                channel(px, masks[1]).unwrap_or(0),
                channel(px, masks[2]).unwrap_or(0),
            )
        };
        out.extend_from_slice(&[r, g, b, channel(px, masks[3]).unwrap_or(255)]);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a DDS file: header with the given pixel format, then `data`.
    fn dds(
        w: u32,
        h: u32,
        pf_flags: u32,
        four_cc: &[u8; 4],
        bits: u32,
        masks: [u32; 4],
        data: &[u8],
    ) -> Vec<u8> {
        let mut v = b"DDS ".to_vec();
        let mut hdr = [0u32; 31];
        hdr[0] = 124;
        hdr[1] = 0x1007; // CAPS | HEIGHT | WIDTH | PIXELFORMAT
        hdr[2] = h;
        hdr[3] = w;
        hdr[18] = 32; // pixel format size
        hdr[19] = pf_flags;
        hdr[20] = u32::from_le_bytes(*four_cc);
        hdr[21] = bits;
        hdr[22..26].copy_from_slice(&masks);
        hdr[26] = 0x1000; // DDSCAPS_TEXTURE
        for x in hdr {
            v.extend_from_slice(&x.to_le_bytes());
        }
        assert_eq!(v.len(), 128);
        v.extend_from_slice(data);
        v
    }

    fn fourcc(w: u32, h: u32, cc: &[u8; 4], data: &[u8]) -> Vec<u8> {
        dds(w, h, DDPF_FOURCC, cc, 0, [0; 4], data)
    }

    fn px(img: &Rgba, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * img.width + x) * 4) as usize;
        img.pixels[o..o + 4].try_into().unwrap()
    }

    /// BC1 block: red (c0) / blue (c1); row 0 uses index 0, row 1 index 1,
    /// row 2 index 2, row 3 index 3.
    fn bc1_block(c0: u16, c1: u16) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&c0.to_le_bytes());
        b.extend_from_slice(&c1.to_le_bytes());
        b.extend_from_slice(&[0b00000000, 0b01010101, 0b10101010, 0b11111111]);
        b
    }

    const RED: u16 = 0xF800;
    const BLUE: u16 = 0x001F;

    #[test]
    fn png_round_trip() {
        let mut img = image::RgbaImage::new(3, 2);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([x as u8 * 80, y as u8 * 200, 7, 255 - x as u8]);
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let out = decode(&png).unwrap();
        assert_eq!((out.width, out.height), (3, 2));
        assert_eq!(out.pixels, img.into_raw());
    }

    #[test]
    fn bc1_four_colour_block() {
        let out = decode(&fourcc(4, 4, b"DXT1", &bc1_block(RED, BLUE))).unwrap();
        assert_eq!(out.pixels.len(), 64);
        assert_eq!(px(&out, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px(&out, 3, 1), [0, 0, 255, 255]);
        assert_eq!(px(&out, 1, 2), [170, 0, 85, 255]); // 2/3 red + 1/3 blue
        assert_eq!(px(&out, 2, 3), [85, 0, 170, 255]);
    }

    #[test]
    fn bc1_punch_through_alpha() {
        // c0 <= c1 selects 3-colour mode with transparent index 3.
        let out = decode(&fourcc(4, 4, b"DXT1", &bc1_block(BLUE, RED))).unwrap();
        assert_eq!(px(&out, 0, 0), [0, 0, 255, 255]);
        assert_eq!(px(&out, 0, 1), [255, 0, 0, 255]);
        assert_eq!(px(&out, 0, 2), [127, 0, 127, 255]);
        assert_eq!(px(&out, 0, 3), [0, 0, 0, 0]);
    }

    #[test]
    fn bc1_non_multiple_of_four_and_dx10() {
        // 5x3 image = 2x1 blocks; second block is solid blue.
        let mut data = bc1_block(RED, BLUE);
        data.extend(bc1_block(BLUE, RED).iter().take(4));
        data.extend([0; 4]);
        let mut file = fourcc(5, 3, b"DX10", &[]);
        for x in [71u32, 3, 0, 1, 0] {
            file.extend_from_slice(&x.to_le_bytes());
        }
        file.extend_from_slice(&data);
        let out = decode(&file).unwrap();
        assert_eq!((out.width, out.height, out.pixels.len()), (5, 3, 60));
        assert_eq!(px(&out, 3, 0), [255, 0, 0, 255]);
        assert_eq!(px(&out, 4, 0), [0, 0, 255, 255]);
        assert_eq!(px(&out, 0, 2), [170, 0, 85, 255]);
        assert_eq!(px(&out, 4, 2), [0, 0, 255, 255]);
    }

    #[test]
    fn bc3_alpha_block() {
        // Alpha endpoints 255 / 0 (8-value mode); pixel i uses index i % 8.
        let mut bits = 0u64;
        for i in 0..16 {
            bits |= ((i % 8) as u64) << (3 * i);
        }
        let mut block = vec![255, 0];
        block.extend_from_slice(&bits.to_le_bytes()[..6]);
        block.extend(bc1_block(BLUE, RED)); // BC3 colour is always 4-colour mode
        let out = decode(&fourcc(4, 4, b"DXT5", &block)).unwrap();
        let alphas: Vec<u8> = (0..16).map(|i| out.pixels[i * 4 + 3]).collect();
        let expect8 = [255, 0, 218, 182, 145, 109, 72, 36];
        assert_eq!(&alphas[..8], &expect8);
        assert_eq!(&alphas[8..], &expect8);
        // c0 < c1 would mean punch-through in BC1, but BC3 keeps 4 colours.
        assert_eq!(&out.pixels[48..51], &[170, 0, 85]);

        // 6-value mode: a0 <= a1, indices 6 and 7 are 0 and 255.
        block[0] = 0;
        block[1] = 100;
        let out = decode(&fourcc(4, 4, b"DXT5", &block)).unwrap();
        let alphas: Vec<u8> = (0..8).map(|i| out.pixels[i * 4 + 3]).collect();
        assert_eq!(alphas, [0, 100, 20, 40, 60, 80, 0, 255]);
    }

    #[test]
    fn bc2_explicit_alpha() {
        let mut block = 0xFEDC_BA98_7654_3210u64.to_le_bytes().to_vec();
        block.extend(bc1_block(RED, BLUE));
        let out = decode(&fourcc(4, 4, b"DXT3", &block)).unwrap();
        for i in 0..16 {
            assert_eq!(out.pixels[i * 4 + 3], i as u8 * 17);
        }
        assert_eq!(&out.pixels[..3], &[255, 0, 0]);
    }

    #[test]
    fn uncompressed_bgra_and_rgb() {
        let masks = [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000];
        let data = [10, 20, 30, 40, 50, 60, 70, 80]; // B G R A per pixel
        let out = decode(&dds(
            2,
            1,
            DDPF_RGB | DDPF_ALPHAPIXELS,
            b"\0\0\0\0",
            32,
            masks,
            &data,
        ))
        .unwrap();
        assert_eq!(out.pixels, [30, 20, 10, 40, 70, 60, 50, 80]);
        // Same data without ALPHAPIXELS: alpha forced opaque.
        let out = decode(&dds(2, 1, DDPF_RGB, b"\0\0\0\0", 32, masks, &data)).unwrap();
        assert_eq!(out.pixels, [30, 20, 10, 255, 70, 60, 50, 255]);
        // RGBA byte order.
        let rgba = [0xFF, 0xFF00, 0xFF_0000, 0xFF00_0000];
        let out = decode(&dds(
            2,
            1,
            DDPF_RGB | DDPF_ALPHAPIXELS,
            b"\0\0\0\0",
            32,
            rgba,
            &data,
        ))
        .unwrap();
        assert_eq!(out.pixels, data);
    }

    #[test]
    fn truncated_dds_errors() {
        let full = fourcc(8, 8, b"DXT5", &[0; 64]);
        assert!(decode(&full).is_ok());
        for len in 0..full.len() {
            assert!(decode(&full[..len]).is_err(), "prefix {len} decoded");
        }
        let masks = [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000];
        assert!(decode(&dds(2, 2, DDPF_RGB, b"\0\0\0\0", 32, masks, &[0; 15])).is_err());
        assert!(decode(&fourcc(0, 4, b"DXT1", &[0; 8])).is_err());
        assert!(decode(&fourcc(u32::MAX, u32::MAX, b"DXT1", &[0; 8])).is_err());
        assert!(decode(&fourcc(4, 4, b"ATI2", &[0; 16])).is_err());
    }

    #[test]
    fn garbage_errors() {
        assert!(decode(b"").is_err());
        assert!(decode(b"GIF89a....").unwrap_err().contains("unrecognised"));
        assert!(decode(b"\x89PNG\r\n\x1a\nnot really").is_err());
        assert!(decode(&[0xFF, 0xD8, 0xFF, 0x00, 1, 2, 3]).is_err());
        let mut seed = 0x9E37_79B9u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for _ in 0..2000 {
            let len = 128 + (rnd() % 64) as usize;
            let mut data: Vec<u8> = (0..len).map(|_| rnd() as u8).collect();
            data[..8].copy_from_slice(b"DDS |\0\0\0");
            data[80] = [0x4, 0x40, 0x41, 0][(rnd() % 4) as usize];
            data[84..88]
                .copy_from_slice([b"DXT1", b"DXT5", b"DX10", b"DXT3"][(rnd() % 4) as usize]);
            data[12] %= 16;
            data[16] %= 16;
            for b in [13, 14, 15, 17, 18, 19] {
                data[b] = 0;
            }
            let _ = decode(&data);
        }
    }
}


/// Decodes textures on a few worker threads (a map load asks for hundreds at
/// once; decoding them in the frame that asked froze it for seconds).
pub fn decode_async(bytes: std::sync::Arc<Vec<u8>>, done: std::sync::mpsc::Sender<(String, Result<Rgba, String>)>, id: String) {
    use std::sync::{mpsc, Mutex, OnceLock};
    type Job = (std::sync::Arc<Vec<u8>>, mpsc::Sender<(String, Result<Rgba, String>)>, String);
    static POOL: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = std::sync::Arc::new(Mutex::new(rx));
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8) / 2;
        for i in 0..n.max(1) {
            let rx = rx.clone();
            let _ = std::thread::Builder::new().name(format!("tex-decode-{i}")).spawn(move || loop {
                let job = rx.lock().ok().and_then(|r| r.recv().ok());
                let Some((bytes, done, id)) = job else { return };
                let _ = done.send((id, decode(&bytes)));
            });
        }
        Mutex::new(tx)
    });
    if let Ok(tx) = pool.lock() {
        let _ = tx.send((bytes, done, id));
    }
}
