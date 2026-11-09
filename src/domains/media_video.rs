//! Video for the media domain: a WebM (Matroska) demuxer, VP8 / VP9
//! decoding through the system's libvpx (loaded with dlopen, so rustudio
//! still starts without it: videos then load but show no frames), YUV →
//! RGBA, and per-video worker threads that keep the frame for the current
//! playback time in the EditableImage texture store (editable.rs
//! `external_image`), where the GUI / SurfaceGui texture caches pick each
//! new generation up.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_long, c_uint, c_void};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use rbx_dom_weak::types::Ref;

// ------------------------------------------------------------------ libvpx

const VPX_DECODER_ABI_VERSION: c_int = 3 + 4 + 5;
const FMT_PLANAR: c_int = 0x100;
const FMT_HIGHBITDEPTH: c_int = 0x800;

#[repr(C)]
struct VpxImage {
    fmt: c_int,
    cs: c_int,
    range: c_int,
    w: c_uint,
    h: c_uint,
    bit_depth: c_uint,
    d_w: c_uint,
    d_h: c_uint,
    r_w: c_uint,
    r_h: c_uint,
    x_chroma_shift: c_uint,
    y_chroma_shift: c_uint,
    planes: [*const u8; 4],
    stride: [c_int; 4],
    bps: c_int,
    user_priv: *mut c_void,
    img_data: *mut u8,
    img_data_owner: c_int,
    self_allocd: c_int,
    fb_priv: *mut c_void,
}

#[repr(C)]
struct DecCfg {
    threads: c_uint,
    w: c_uint,
    h: c_uint,
}

struct Vpx {
    vp8: unsafe extern "C" fn() -> *const c_void,
    vp9: unsafe extern "C" fn() -> *const c_void,
    init: unsafe extern "C" fn(*mut c_void, *const c_void, *const DecCfg, c_long, c_int) -> c_int,
    decode: unsafe extern "C" fn(*mut c_void, *const u8, c_uint, *mut c_void, c_long) -> c_int,
    get_frame: unsafe extern "C" fn(*mut c_void, *mut *const c_void) -> *const VpxImage,
    destroy: unsafe extern "C" fn(*mut c_void) -> c_int,
}

fn vpx() -> Option<&'static Vpx> {
    static LIB: OnceLock<Option<Vpx>> = OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let mut h = std::ptr::null_mut();
        for name in [c"libvpx.so.12", c"libvpx.so.11", c"libvpx.so.9", c"libvpx.so"] {
            h = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if !h.is_null() {
                break;
            }
        }
        if h.is_null() {
            return None;
        }
        let sym = |n: &std::ffi::CStr| {
            let p = libc::dlsym(h, n.as_ptr() as *const c_char);
            (!p.is_null()).then_some(p)
        };
        Some(Vpx {
            vp8: std::mem::transmute::<*mut c_void, _>(sym(c"vpx_codec_vp8_dx")?),
            vp9: std::mem::transmute::<*mut c_void, _>(sym(c"vpx_codec_vp9_dx")?),
            init: std::mem::transmute::<*mut c_void, _>(sym(c"vpx_codec_dec_init_ver")?),
            decode: std::mem::transmute::<*mut c_void, _>(sym(c"vpx_codec_decode")?),
            get_frame: std::mem::transmute::<*mut c_void, _>(sym(c"vpx_codec_get_frame")?),
            destroy: std::mem::transmute::<*mut c_void, _>(sym(c"vpx_codec_destroy")?),
        })
    })
    .as_ref()
}

/// Whether frames can be decoded on this machine.
pub fn decoder_available() -> bool {
    vpx().is_some()
}

// ------------------------------------------------------------------- WebM

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Codec {
    Vp8,
    Vp9,
    Other,
}

#[derive(Clone, Debug)]
pub struct Packet {
    pub t: f64,
    pub key: bool,
    pub at: usize,
    pub len: usize,
}

pub struct Track {
    /// The soundtrack (Opus), decoded at 48 kHz, if the file has one.
    pub audio: Option<Arc<crate::audio::Pcm>>,
    pub bytes: Arc<Vec<u8>>,
    pub codec: Codec,
    pub w: u32,
    pub h: u32,
    pub len: f64,
    pub packets: Vec<Packet>,
}

fn vint(b: &[u8], at: usize, strip: bool) -> Option<(u64, usize)> {
    let first = *b.get(at)?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || at + len > b.len() {
        return None;
    }
    let mut v = if strip { (first as u64) & (0xFFu64 >> len) } else { first as u64 };
    for i in 1..len {
        v = (v << 8) | b[at + i] as u64;
    }
    Some((v, len))
}

/// Parses a WebM: the first video track's codec, size and every frame.
pub fn demux(bytes: Arc<Vec<u8>>) -> Option<Track> {
    let b = &bytes[..];
    let mut scale = 1_000_000f64;
    let mut dur: Option<f64> = None;
    // Per track entry being read: (number, type, codec, w, h).
    let mut cur: (u64, u64, String, u32, u32) = (0, 0, String::new(), 0, 0);
    let mut tracks: Vec<(u64, u64, String, u32, u32)> = Vec::new();
    // Audio per track number: (channels, CodecPrivate).
    let mut audio_info: HashMap<u64, (u32, Vec<u8>)> = HashMap::new();
    let mut cur_channels = 0u32;
    let mut cur_private: Vec<u8> = Vec::new();
    let mut cluster_tc = 0u64;
    let mut raw_packets: Vec<(u64, i64, bool, usize, usize)> = Vec::new();
    let mut stack: Vec<(usize, u64)> = vec![(b.len(), 0)];
    let mut at = 0;
    while at < b.len() {
        while stack.len() > 1 && at >= stack.last().unwrap().0 {
            let (_, id) = stack.pop().unwrap();
            if id == 0xAE {
                audio_info.insert(cur.0, (cur_channels, std::mem::take(&mut cur_private)));
                cur_channels = 0;
                tracks.push(std::mem::take(&mut cur));
            }
        }
        let Some((id, il)) = vint(b, at, false) else { break };
        let Some((size, sl)) = vint(b, at + il, true) else { break };
        let body = at + il + sl;
        let unknown = size == (1u64 << (7 * sl)) - 1;
        let parent_end = stack.last().unwrap().0;
        let end = if unknown { parent_end } else { (body as u64).saturating_add(size).min(b.len() as u64) as usize };
        let data = &b[body.min(b.len())..end.max(body).min(b.len())];
        let uint = || data.iter().fold(0u64, |a, x| (a << 8) | *x as u64);
        match id {
            // Masters: Segment, Info, Tracks, TrackEntry, Video, Cluster, BlockGroup.
            0x18538067 | 0x1549A966 | 0x1654AE6B | 0xAE | 0xE0 | 0xE1 | 0x1F43B675 | 0xA0 => {
                if id == 0x1F43B675 && unknown {
                    // An unknown-size cluster ends where the next one starts.
                }
                stack.push((end, id));
                at = body;
                continue;
            }
            0x2AD7B1 => scale = uint() as f64,
            0x4489 => {
                dur = match data.len() {
                    4 => Some(f32::from_be_bytes(data.try_into().ok()?) as f64),
                    8 => Some(f64::from_be_bytes(data.try_into().ok()?)),
                    _ => None,
                }
            }
            0xD7 => cur.0 = uint(),
            0x83 => cur.1 = uint(),
            0x86 => cur.2 = String::from_utf8_lossy(data).trim_end_matches('\0').to_string(),
            0xB0 => cur.3 = uint() as u32,
            0xBA => cur.4 = uint() as u32,
            0x9F => cur_channels = uint() as u32,
            0x63A2 => cur_private = data.to_vec(),
            0xE7 => cluster_tc = uint(),
            // SimpleBlock / Block: track, relative timecode, flags, frame.
            0xA3 | 0xA1 => {
                if let Some((track, tl)) = vint(data, 0, true) {
                    if data.len() >= tl + 3 {
                        let rel = i16::from_be_bytes([data[tl], data[tl + 1]]) as i64;
                        let flags = data[tl + 2];
                        let lacing = (flags >> 1) & 3;
                        if lacing == 0 {
                            let key = id == 0xA3 && flags & 0x80 != 0;
                            let off = body + tl + 3;
                            raw_packets.push((track, cluster_tc as i64 + rel, key, off, end - off));
                        }
                    }
                }
            }
            _ => {}
        }
        // A new Cluster starting inside an unknown-size one closes it.
        at = end;
    }
    while stack.len() > 1 {
        let (_, id) = stack.pop().unwrap();
        if id == 0xAE {
            audio_info.insert(cur.0, (cur_channels, std::mem::take(&mut cur_private)));
            tracks.push(std::mem::take(&mut cur));
        }
    }
    let audio = tracks
        .iter()
        .find(|t| t.2 == "A_OPUS")
        .and_then(|t| {
            let (ch, private) = audio_info.get(&t.0).cloned().unwrap_or((2, Vec::new()));
            let mut pk: Vec<(i64, usize, usize)> = raw_packets
                .iter()
                .filter(|p| p.0 == t.0)
                .map(|p| (p.1, p.3, p.4))
                .collect();
            pk.sort_by_key(|p| p.0);
            decode_opus(&bytes, &pk, ch.max(1), &private).map(Arc::new)
        });
    let video = tracks.iter().find(|t| t.1 == 1 || t.2.starts_with("V_"))?.clone();
    let codec = match video.2.as_str() {
        "V_VP8" => Codec::Vp8,
        "V_VP9" => Codec::Vp9,
        _ => Codec::Other,
    };
    let mut packets: Vec<Packet> = raw_packets
        .into_iter()
        .filter(|p| p.0 == video.0)
        .map(|(_, tc, key, at, len)| Packet { t: tc as f64 * scale / 1e9, key, at, len })
        .collect();
    packets.sort_by(|a, b| a.t.total_cmp(&b.t));
    if let Some(p) = packets.first_mut() {
        p.key = true;
    }
    // Blocks inside BlockGroups don't flag keyframes: VP8/VP9 say it in the
    // frame header (VP8: bit 0 clear; VP9: frame_type bit).
    for p in packets.iter_mut() {
        if !p.key {
            if let Some(&h) = bytes.get(p.at) {
                p.key = match codec {
                    Codec::Vp8 => h & 1 == 0,
                    Codec::Vp9 => vp9_key(h),
                    Codec::Other => false,
                };
            }
        }
    }
    let len = dur.map(|d| d * scale / 1e9).or_else(|| packets.last().map(|p| p.t))?;
    Some(Track { audio, bytes, codec, w: video.3, h: video.4, len, packets })
}

/// VP9 uncompressed header: frame marker (2), profile bits, show_existing,
/// frame_type (0 = key).
fn vp9_key(h: u8) -> bool {
    if h >> 6 != 2 {
        return false;
    }
    let profile = ((h >> 5) & 1) | (((h >> 4) & 1) << 1);
    // Bits after the profile: [reserved_zero if profile 3] show_existing_frame, frame_type.
    let (show_existing, frame_type) = if profile == 3 { ((h >> 2) & 1, (h >> 1) & 1) } else { ((h >> 3) & 1, (h >> 2) & 1) };
    show_existing == 0 && frame_type == 0
}

// -------------------------------------------------------------------- Opus

struct Opus {
    create: unsafe extern "C" fn(i32, c_int, *mut c_int) -> *mut c_void,
    decode: unsafe extern "C" fn(*mut c_void, *const u8, i32, *mut f32, c_int, c_int) -> c_int,
    destroy: unsafe extern "C" fn(*mut c_void),
}

fn opus() -> Option<&'static Opus> {
    static LIB: OnceLock<Option<Opus>> = OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let mut h = std::ptr::null_mut();
        for name in [c"libopus.so.0", c"libopus.so"] {
            h = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if !h.is_null() {
                break;
            }
        }
        if h.is_null() {
            return None;
        }
        let sym = |n: &std::ffi::CStr| {
            let p = libc::dlsym(h, n.as_ptr() as *const c_char);
            (!p.is_null()).then_some(p)
        };
        Some(Opus {
            create: std::mem::transmute::<*mut c_void, _>(sym(c"opus_decoder_create")?),
            decode: std::mem::transmute::<*mut c_void, _>(sym(c"opus_decode_float")?),
            destroy: std::mem::transmute::<*mut c_void, _>(sym(c"opus_decoder_destroy")?),
        })
    })
    .as_ref()
}

/// A WebM's Opus track → planar f32 at 48 kHz (pre-skip from OpusHead
/// dropped). Up to 2 channels (Roblox videos are stereo at most).
fn decode_opus(bytes: &[u8], packets: &[(i64, usize, usize)], channels: u32, head: &[u8]) -> Option<crate::audio::Pcm> {
    let o = opus()?;
    let ch = channels.clamp(1, 2) as usize;
    let pre_skip = if head.len() >= 12 && &head[..8] == b"OpusHead" {
        u16::from_le_bytes([head[10], head[11]]) as usize
    } else {
        0
    };
    let mut err = 0;
    let dec = unsafe { (o.create)(48000, ch as c_int, &mut err) };
    if dec.is_null() || err != 0 {
        return None;
    }
    let mut chans = vec![Vec::new(); ch];
    let mut buf = vec![0f32; 5760 * ch];
    for &(_, at, len) in packets {
        let Some(data) = bytes.get(at..at + len) else { continue };
        let n = unsafe { (o.decode)(dec, data.as_ptr(), len as i32, buf.as_mut_ptr(), 5760, 0) };
        if n > 0 {
            for i in 0..n as usize {
                for (c, out) in chans.iter_mut().enumerate() {
                    out.push(buf[i * ch + c]);
                }
            }
        }
    }
    unsafe { (o.destroy)(dec) };
    for c in chans.iter_mut() {
        c.drain(..pre_skip.min(c.len()));
    }
    Some(crate::audio::Pcm { rate: 48000, chans })
}

// ---------------------------------------------------------------- decoding

pub struct Frame {
    pub t: f64,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

pub struct Decoder {
    track: Arc<Track>,
    ctx: Box<[u64; 32]>,
    live: bool,
    /// Index of the next packet to decode.
    next: usize,
    last: Option<Frame>,
}

unsafe impl Send for Decoder {}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.close();
    }
}

impl Decoder {
    pub fn new(track: Arc<Track>) -> Option<Decoder> {
        if track.codec == Codec::Other {
            return None;
        }
        vpx()?;
        Some(Decoder { track, ctx: Box::new([0; 32]), live: false, next: 0, last: None })
    }

    fn close(&mut self) {
        if self.live {
            if let Some(v) = vpx() {
                unsafe { (v.destroy)(self.ctx.as_mut_ptr() as *mut c_void) };
            }
            self.live = false;
        }
    }

    fn open(&mut self) -> bool {
        self.close();
        let Some(v) = vpx() else { return false };
        *self.ctx = [0; 32];
        let iface = unsafe { if self.track.codec == Codec::Vp8 { (v.vp8)() } else { (v.vp9)() } };
        let cfg = DecCfg { threads: 2, w: 0, h: 0 };
        let r = unsafe { (v.init)(self.ctx.as_mut_ptr() as *mut c_void, iface, &cfg, 0, VPX_DECODER_ABI_VERSION) };
        self.live = r == 0;
        self.live
    }

    /// Decodes one packet; the frame it shows, as RGBA.
    fn feed(&mut self, i: usize) -> Option<Frame> {
        let v = vpx()?;
        let p = self.track.packets.get(i)?.clone();
        let data = &self.track.bytes[p.at..p.at + p.len];
        let ctx = self.ctx.as_mut_ptr() as *mut c_void;
        let r = unsafe { (v.decode)(ctx, data.as_ptr(), data.len() as c_uint, std::ptr::null_mut(), 0) };
        if r != 0 {
            return None;
        }
        let mut iter: *const c_void = std::ptr::null();
        let mut out = None;
        loop {
            let img = unsafe { (v.get_frame)(ctx, &mut iter) };
            if img.is_null() {
                break;
            }
            out = unsafe { to_rgba(&*img) }.map(|(w, h, rgba)| Frame { t: p.t, w, h, rgba });
        }
        out
    }

    /// The frame showing at `t` (the last one whose time is ≤ t).
    pub fn frame_at(&mut self, t: f64) -> Option<&Frame> {
        let pk = &self.track.packets;
        if pk.is_empty() {
            return None;
        }
        let target = pk.partition_point(|p| p.t <= t + 1e-6).saturating_sub(1);
        let have = self.next.checked_sub(1);
        if have == Some(target) && self.last.is_some() {
            return self.last.as_ref();
        }
        // Forward from where we are, unless a keyframe past it is closer or
        // we'd have to go back.
        let key = (0..=target).rev().find(|&i| pk[i].key).unwrap_or(0);
        let restart = !self.live || have.is_none_or(|h| h > target || key > h);
        if restart {
            if !self.open() {
                return None;
            }
            self.next = key;
        }
        while self.next <= target {
            let i = self.next;
            self.next += 1;
            if let Some(f) = self.feed(i) {
                self.last = Some(f);
            }
        }
        self.last.as_ref()
    }
}

/// libvpx's image → RGBA8 (BT.601, limited or full range as tagged).
unsafe fn to_rgba(img: &VpxImage) -> Option<(u32, u32, Vec<u8>)> {
    if img.fmt & FMT_PLANAR == 0 {
        return None;
    }
    let (w, h) = (img.d_w as usize, img.d_h as usize);
    let hbd = img.fmt & FMT_HIGHBITDEPTH != 0;
    let shift = if hbd { img.bit_depth.saturating_sub(8) } else { 0 };
    let (xs, ys) = (img.x_chroma_shift, img.y_chroma_shift);
    let full = img.range == 1;
    let mut out = vec![0u8; w * h * 4];
    let sample = |plane: usize, x: usize, y: usize| -> f32 {
        let row = img.planes[plane].offset(img.stride[plane] as isize * y as isize);
        if hbd {
            (*(row as *const u16).add(x) >> shift) as f32
        } else {
            *row.add(x) as f32
        }
    };
    for y in 0..h {
        for x in 0..w {
            let yy = sample(0, x, y);
            let u = sample(1, x >> xs, y >> ys) - 128.0;
            let v = sample(2, x >> xs, y >> ys) - 128.0;
            let (r, g, b) = if full {
                (yy + 1.402 * v, yy - 0.344 * u - 0.714 * v, yy + 1.772 * u)
            } else {
                let c = 1.164 * (yy - 16.0);
                (c + 1.596 * v, c - 0.392 * u - 0.813 * v, c + 2.017 * u)
            };
            let o = (y * w + x) * 4;
            out[o] = r.clamp(0.0, 255.0) as u8;
            out[o + 1] = g.clamp(0.0, 255.0) as u8;
            out[o + 2] = b.clamp(0.0, 255.0) as u8;
            out[o + 3] = 255;
        }
    }
    Some((w as u32, h as u32, out))
}

/// Nearest-neighbour downscale so the larger side is at most `max`.
pub fn fit(f: &Frame, max: u32) -> (u32, u32, Vec<u8>) {
    let big = f.w.max(f.h);
    if big <= max || big == 0 {
        return (f.w, f.h, f.rgba.clone());
    }
    let s = max as f64 / big as f64;
    let (w, h) = (((f.w as f64 * s).round() as u32).max(1), ((f.h as f64 * s).round() as u32).max(1));
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let sy = ((y as f64 + 0.5) / s) as u32;
        for x in 0..w {
            let sx = ((x as f64 + 0.5) / s) as u32;
            let i = ((sy.min(f.h - 1) * f.w + sx.min(f.w - 1)) * 4) as usize;
            let o = ((y * w + x) * 4) as usize;
            out[o..o + 4].copy_from_slice(&f.rgba[i..i + 4]);
        }
    }
    (w, h, out)
}

// ---------------------------------------------------------------- playback

/// A video showing somewhere: a worker thread decodes the frame for the
/// latest requested time into the texture store under `key`.
pub struct Stream {
    pub key: Ref,
    tx: Sender<f64>,
}

impl Stream {
    pub fn new(track: Arc<Track>) -> Option<Stream> {
        let mut dec = Decoder::new(track)?;
        let key = Ref::new();
        let (tx, rx): (Sender<f64>, Receiver<f64>) = channel();
        std::thread::Builder::new()
            .name("rustudio-video".into())
            .spawn(move || {
                let mut shown = f64::NAN;
                while let Ok(mut t) = rx.recv() {
                    while let Ok(n) = rx.try_recv() {
                        t = n;
                    }
                    if let Some(f) = dec.frame_at(t) {
                        if f.t != shown {
                            shown = f.t;
                            crate::runtime::editable::external_image(key, f.w, f.h, f.rgba.clone());
                        }
                    }
                }
                crate::runtime::editable::drop_external_image(key);
            })
            .ok()?;
        Some(Stream { key, tx })
    }

    pub fn show(&self, t: f64) {
        let _ = self.tx.send(t);
    }
}

/// GUI object (VideoFrame / VideoDisplay) → the stream key it shows.
static SURFACES: Mutex<Option<HashMap<Ref, Ref>>> = Mutex::new(None);

pub fn set_surface(gui: Ref, key: Option<Ref>) {
    if let Ok(mut g) = SURFACES.lock() {
        let m = g.get_or_insert_with(HashMap::new);
        match key {
            Some(k) => {
                m.insert(gui, k);
            }
            None => {
                m.remove(&gui);
            }
        }
    }
}

/// The texture id a VideoFrame / VideoDisplay shows now (gui_walk.rs).
pub fn surface_uri(gui: Ref) -> Option<String> {
    let key = *SURFACES.lock().ok()?.as_ref()?.get(&gui)?;
    crate::runtime::editable::content_uri(key, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_vp9_and_vp8_webm() {
        // Made with ffmpeg testsrc2 320x240 (the media.md test clips).
        for id in ["8000000000099010", "8000000000099011"] {
            let Some(dir) = dirs_assets() else { return };
            let Ok(b) = std::fs::read(dir.join(format!("{id}.webm"))) else { return };
            let t = demux(Arc::new(b)).expect("demux");
            assert_eq!((t.w, t.h), (320, 240));
            assert!((t.len - 4.0).abs() < 0.02, "{}", t.len);
            assert!(t.packets.len() >= 110);
            if id.ends_with("10") && opus().is_some() {
                // 4 s, 440 Hz sine from ffmpeg: about 4 s of non-silent stereo/mono.
                let a = t.audio.as_ref().expect("opus soundtrack");
                assert!((a.len_secs() - 4.0).abs() < 0.1, "{}", a.len_secs());
                let peak = a.chans[0].iter().fold(0f32, |m, x| m.max(x.abs()));
                assert!(peak > 0.05, "{peak}");
            }
            if !decoder_available() {
                return;
            }
            let mut d = Decoder::new(Arc::new(t)).unwrap();
            let f = d.frame_at(1.5).expect("frame");
            assert_eq!((f.w, f.h), (320, 240));
            assert!((f.t - 1.5).abs() < 0.05);
            // testsrc2 is colourful: not a flat image.
            let distinct: std::collections::HashSet<[u8; 3]> =
                f.rgba.chunks(4).map(|p| [p[0] / 32, p[1] / 32, p[2] / 32]).collect();
            assert!(distinct.len() > 8);
            // Going back restarts from a keyframe.
            assert!(d.frame_at(0.1).is_some());
        }
    }

    fn dirs_assets() -> Option<std::path::PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(std::path::PathBuf::from(home).join(".local/share/rustudio/assets"))
    }
}
