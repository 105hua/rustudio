//! Block DSP for the Audio* graph (media domain): channel layouts, every
//! Audio effect, AudioFilter's response (GetGainAt), AudioAnalyzer's levels
//! and spectrum. Everything works on plain buffers, so tests render offline
//! and stay silent.
//!
//! Measured against Studio (offline Studio, 2026-11-07; see checks/parity/media.md):
//! - The graph runs at 48 kHz.
//! - AudioFilter: RBJ cookbook biquads; the 24 / 48 dB slopes are 2 / 4
//!   identical stages with Q^(1/stages) (Lowpass24dB at 1 kHz = +0.759 dB
//!   for fc 2 kHz, Q 0.707, as Studio); Lowpass6dB is the one-pole
//!   `a = 1 - exp(-2π fc / fs)` (-2.986 dB at fc). Matches GetGainAt to 3
//!   decimals for every type.
//! - AudioAnalyzer: Hamming window of 256 / 1024 / 4096 samples (Small /
//!   Medium / Large), spectrum = 2.5·|FFT|/N per bin, N/2 bins, channels
//!   combined as the RMS of their magnitudes; RmsLevel is the RMS over every
//!   channel's window, PeakLevel the largest |sample|.
//! - Channels: mono into a stereo bus is x/√2 on L and R; stereo into mono is
//!   (L+R)/√2 on the centre channel.
//! The effects' internals are Roblox's own (FMOD-derived) DSP, not
//! published: parameters, ranges and defaults are Studio's, the algorithms
//! are standard ones with the same meaning (documented per effect).
#![allow(dead_code)]

use std::f32::consts::{FRAC_1_SQRT_2, PI, TAU};

pub const RATE: f32 = 48000.0;

/// Channels × frames.
pub type Buf = Vec<Vec<f32>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ch {
    L,
    R,
    C,
    Sub,
    SL,
    SR,
    BL,
    BR,
    TL,
    TR,
    TBL,
    TBR,
}

/// Channel order of Enum.AudioChannelLayout (Mono 0 … Surround_7_1_4 6).
pub fn layout(value: u32) -> &'static [Ch] {
    use Ch::*;
    match value {
        0 => &[C],
        2 => &[L, R, BL, BR],
        3 => &[L, R, C, SL, SR],
        4 => &[L, R, C, Sub, SL, SR],
        5 => &[L, R, C, Sub, SL, SR, BL, BR],
        6 => &[L, R, C, Sub, SL, SR, BL, BR, TL, TR, TBL, TBR],
        _ => &[L, R],
    }
}

/// The layout a buffer of `n` channels has.
pub fn layout_of(n: usize) -> &'static [Ch] {
    match n {
        1 => layout(0),
        4 => layout(2),
        5 => layout(3),
        6 => layout(4),
        8 => layout(5),
        12 => layout(6),
        _ => layout(1),
    }
}

/// Splitter / mixer pin names, as Studio lists them.
pub fn pin_channel(pin: &str) -> Option<Ch> {
    use Ch::*;
    Some(match pin {
        "Left" => L,
        "Right" => R,
        "Center" => C,
        "Sub" => Sub,
        "SurroundLeft" => SL,
        "SurroundRight" => SR,
        "BackLeft" => BL,
        "BackRight" => BR,
        "TopLeft" => TL,
        "TopRight" => TR,
        "TopBackLeft" => TBL,
        "TopBackRight" => TBR,
        _ => return None,
    })
}

pub const CHANNEL_PINS: [&str; 12] = [
    "BackLeft",
    "BackRight",
    "Center",
    "Left",
    "Right",
    "Sub",
    "SurroundLeft",
    "SurroundRight",
    "TopBackLeft",
    "TopBackRight",
    "TopLeft",
    "TopRight",
];

pub fn silence(ch: usize, n: usize) -> Buf {
    vec![vec![0.0; n]; ch.max(1)]
}

/// `b` converted to `to` channels (by channel names).
pub fn convert(b: &Buf, to: usize) -> Buf {
    let from = b.len();
    if from == to {
        return b.clone();
    }
    let n = b.first().map_or(0, |c| c.len());
    let mut out = silence(to, n);
    let src = layout_of(from);
    let dst = layout_of(to);
    if from == 1 {
        // Mono: equal power on L and R (or the centre of a mono bus).
        if dst.contains(&Ch::L) {
            for (i, c) in dst.iter().enumerate() {
                if matches!(c, Ch::L | Ch::R) {
                    for (o, s) in out[i].iter_mut().zip(&b[0]) {
                        *o = s * FRAC_1_SQRT_2;
                    }
                }
            }
        } else {
            out[0].copy_from_slice(&b[0]);
        }
        return out;
    }
    if to == 1 {
        for (i, c) in src.iter().enumerate() {
            let g = match c {
                Ch::Sub => 0.0,
                Ch::C => 1.0,
                _ => FRAC_1_SQRT_2,
            };
            for (o, s) in out[0].iter_mut().zip(&b[i]) {
                *o += s * g;
            }
        }
        return out;
    }
    for (i, c) in src.iter().enumerate() {
        let targets: &[(Ch, f32)] = match c {
            c if dst.contains(c) => &[],
            Ch::C => &[(Ch::L, FRAC_1_SQRT_2), (Ch::R, FRAC_1_SQRT_2)],
            Ch::SL | Ch::BL | Ch::TL | Ch::TBL => &[(Ch::SL, 1.0), (Ch::BL, 1.0), (Ch::L, 1.0)],
            Ch::SR | Ch::BR | Ch::TR | Ch::TBR => &[(Ch::SR, 1.0), (Ch::BR, 1.0), (Ch::R, 1.0)],
            _ => &[],
        };
        if dst.contains(c) {
            let j = dst.iter().position(|d| d == c).unwrap();
            for (o, s) in out[j].iter_mut().zip(&b[i]) {
                *o += s;
            }
            continue;
        }
        // First target the layout has.
        if let Some((j, g)) = targets
            .iter()
            .find_map(|(t, g)| dst.iter().position(|d| d == t).map(|j| (j, *g)))
        {
            if *c == Ch::C {
                for (t, g) in targets {
                    if let Some(j) = dst.iter().position(|d| d == t) {
                        for (o, s) in out[j].iter_mut().zip(&b[i]) {
                            *o += s * g;
                        }
                    }
                }
            } else {
                for (o, s) in out[j].iter_mut().zip(&b[i]) {
                    *o += s * g;
                }
            }
        }
    }
    out
}

/// Sum of buffers; the result has the most channels of any input.
pub fn mix<'a>(bufs: impl IntoIterator<Item = &'a Buf>) -> Option<Buf> {
    let bufs: Vec<&Buf> = bufs.into_iter().collect();
    let ch = bufs.iter().map(|b| b.len()).max()?;
    let n = bufs.iter().map(|b| b.first().map_or(0, |c| c.len())).max()?;
    let mut out = silence(ch, n);
    for b in bufs {
        let b = if b.len() == ch { b.clone() } else { convert(b, ch) };
        for (o, i) in out.iter_mut().zip(&b) {
            for (x, y) in o.iter_mut().zip(i) {
                *x += y;
            }
        }
    }
    Some(out)
}

pub fn gain(b: &mut Buf, g: f32) {
    if g != 1.0 {
        for c in b.iter_mut() {
            for s in c.iter_mut() {
                *s *= g;
            }
        }
    }
}

pub fn db(v: f32) -> f32 {
    10f32.powf(v / 20.0)
}

// ------------------------------------------------------------------ biquads

#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
}

#[derive(Clone, Copy, Debug, Default)]
struct BqState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn norm(b: [f64; 3], a: [f64; 3]) -> Biquad {
        Biquad {
            b: [(b[0] / a[0]) as f32, (b[1] / a[0]) as f32, (b[2] / a[0]) as f32],
            a: [(a[1] / a[0]) as f32, (a[2] / a[0]) as f32],
        }
    }
    fn identity() -> Biquad {
        Biquad { b: [1.0, 0.0, 0.0], a: [0.0, 0.0] }
    }
    /// RBJ Audio EQ Cookbook. `kind`: peak, lowshelf, highshelf, lowpass,
    /// highpass, bandpass (0 dB peak), notch.
    pub fn rbj(kind: &str, f0: f32, q: f32, gain_db: f32) -> Biquad {
        let f0 = (f0 as f64).clamp(1.0, RATE as f64 * 0.4999);
        let q = (q as f64).max(1e-3);
        let a_ = 10f64.powf(gain_db as f64 / 40.0);
        let w = std::f64::consts::TAU * f0 / RATE as f64;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        let sq = 2.0 * a_.sqrt() * al;
        let (b, a) = match kind {
            "peak" => ([1.0 + al * a_, -2.0 * c, 1.0 - al * a_], [1.0 + al / a_, -2.0 * c, 1.0 - al / a_]),
            "lowshelf" => (
                [
                    a_ * ((a_ + 1.0) - (a_ - 1.0) * c + sq),
                    2.0 * a_ * ((a_ - 1.0) - (a_ + 1.0) * c),
                    a_ * ((a_ + 1.0) - (a_ - 1.0) * c - sq),
                ],
                [
                    (a_ + 1.0) + (a_ - 1.0) * c + sq,
                    -2.0 * ((a_ - 1.0) + (a_ + 1.0) * c),
                    (a_ + 1.0) + (a_ - 1.0) * c - sq,
                ],
            ),
            "highshelf" => (
                [
                    a_ * ((a_ + 1.0) + (a_ - 1.0) * c + sq),
                    -2.0 * a_ * ((a_ - 1.0) + (a_ + 1.0) * c),
                    a_ * ((a_ + 1.0) + (a_ - 1.0) * c - sq),
                ],
                [
                    (a_ + 1.0) - (a_ - 1.0) * c + sq,
                    2.0 * ((a_ - 1.0) - (a_ + 1.0) * c),
                    (a_ + 1.0) - (a_ - 1.0) * c - sq,
                ],
            ),
            "lowpass" => ([(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0], [1.0 + al, -2.0 * c, 1.0 - al]),
            "highpass" => ([(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0], [1.0 + al, -2.0 * c, 1.0 - al]),
            "bandpass" => ([al, 0.0, -al], [1.0 + al, -2.0 * c, 1.0 - al]),
            "notch" => ([1.0, -2.0 * c, 1.0], [1.0 + al, -2.0 * c, 1.0 - al]),
            _ => return Biquad::identity(),
        };
        Biquad::norm(b, a)
    }
    /// One-pole lowpass / highpass (6 dB/oct), as Studio's Lowpass6dB.
    pub fn one_pole(high: bool, f0: f32) -> Biquad {
        let a = 1.0 - (-TAU * f0.clamp(1.0, RATE * 0.4999) / RATE).exp();
        if high {
            // y = x - lp(x)
            Biquad { b: [1.0 - a, -(1.0 - a), 0.0], a: [-(1.0 - a), 0.0] }
        } else {
            Biquad { b: [a, 0.0, 0.0], a: [-(1.0 - a), 0.0] }
        }
    }
    pub fn magnitude(&self, f: f32) -> f64 {
        let w = std::f64::consts::TAU * f as f64 / RATE as f64;
        let (s, c) = w.sin_cos();
        let (c2, s2) = ((2.0 * w).cos(), (2.0 * w).sin());
        let b = self.b.map(|x| x as f64);
        let a = self.a.map(|x| x as f64);
        let nr = b[0] + b[1] * c + b[2] * c2;
        let ni = -(b[1] * s + b[2] * s2);
        let dr = 1.0 + a[0] * c + a[1] * c2;
        let di = -(a[0] * s + a[1] * s2);
        ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
    }
    #[inline]
    fn tick(&self, st: &mut BqState, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * st.x1 + self.b[2] * st.x2 - self.a[0] * st.y1 - self.a[1] * st.y2;
        st.x2 = st.x1;
        st.x1 = x;
        st.y2 = st.y1;
        st.y1 = if y.abs() < 1e-25 { 0.0 } else { y };
        st.y1
    }
}

/// A cascade of biquads run on every channel.
#[derive(Default, Clone)]
pub struct Chain {
    pub stages: Vec<Biquad>,
    state: Vec<Vec<BqState>>,
}

impl Chain {
    pub fn set(&mut self, stages: Vec<Biquad>) {
        if stages.len() != self.stages.len() {
            self.state.clear();
        }
        self.stages = stages;
    }
    pub fn run(&mut self, b: &mut Buf) {
        if self.state.len() != b.len() {
            self.state = vec![vec![BqState::default(); self.stages.len()]; b.len()];
        }
        for (c, st) in b.iter_mut().zip(self.state.iter_mut()) {
            for s in c.iter_mut() {
                let mut x = *s;
                for (q, k) in self.stages.iter().zip(st.iter_mut()) {
                    x = q.tick(k, x);
                }
                *s = x;
            }
        }
    }
    pub fn magnitude(&self, f: f32) -> f64 {
        self.stages.iter().map(|s| s.magnitude(f)).product()
    }
}

/// Enum.AudioFilterType (value) → its stages.
pub fn filter_stages(kind: u32, freq: f32, q: f32, gain_db: f32) -> Vec<Biquad> {
    let n = |stages: usize, k: &str| -> Vec<Biquad> {
        let qs = q.max(1e-3).powf(1.0 / stages as f32);
        vec![Biquad::rbj(k, freq, qs, 0.0); stages]
    };
    match kind {
        0 => vec![Biquad::rbj("peak", freq, q, gain_db)],
        1 => vec![Biquad::rbj("lowshelf", freq, q, gain_db)],
        2 => vec![Biquad::rbj("highshelf", freq, q, gain_db)],
        3 => n(1, "lowpass"),
        4 => n(2, "lowpass"),
        5 => n(4, "lowpass"),
        6 => n(1, "highpass"),
        7 => n(2, "highpass"),
        8 => n(4, "highpass"),
        9 => vec![Biquad::rbj("bandpass", freq, q, 0.0)],
        10 => vec![Biquad::rbj("notch", freq, q, 0.0)],
        11 => vec![Biquad::one_pole(false, freq)],
        _ => vec![],
    }
}

/// AudioFilter:GetGainAt (dB).
pub fn filter_gain_at(kind: u32, freq: f32, q: f32, gain_db: f32, at: f32) -> f32 {
    let m: f64 = filter_stages(kind, freq, q, gain_db).iter().map(|s| s.magnitude(at)).product();
    (20.0 * m.max(1e-300).log10()) as f32
}

// ---------------------------------------------------------------- spectrum

/// In-place radix-2 FFT (len a power of two).
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -TAU / len as f32;
        let (ws, wc) = ang.sin_cos();
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wc - ci * ws;
                ci = cr * ws + ci * wc;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

/// Enum.AudioWindowSize → analyzer window (samples).
pub fn window_samples(v: u32) -> usize {
    match v {
        0 => 256,
        2 => 4096,
        _ => 1024,
    }
}

/// AudioAnalyzer:GetSpectrum over the last `n` samples of each channel.
pub fn spectrum(chans: &[Vec<f32>], n: usize) -> Vec<f32> {
    let mut acc = vec![0f32; n / 2];
    if chans.is_empty() {
        return acc;
    }
    for c in chans {
        let mut re: Vec<f32> = (0..n)
            .map(|i| {
                let x = c.get(c.len().wrapping_sub(n).wrapping_add(i)).copied().unwrap_or(0.0);
                let w = 0.54 - 0.46 * (TAU * i as f32 / (n as f32 - 1.0)).cos();
                if c.len() >= n { x * w } else { 0.0 }
            })
            .collect();
        if c.len() < n {
            let off = n - c.len();
            for (i, x) in c.iter().enumerate() {
                let k = off + i;
                re[k] = x * (0.54 - 0.46 * (TAU * k as f32 / (n as f32 - 1.0)).cos());
            }
        }
        let mut im = vec![0f32; n];
        fft(&mut re, &mut im);
        for k in 0..n / 2 {
            let m = 2.5 * (re[k] * re[k] + im[k] * im[k]).sqrt() / n as f32;
            acc[k] += m * m;
        }
    }
    let ch = chans.len() as f32;
    acc.iter().map(|p| (p / ch).sqrt()).collect()
}

/// (RmsLevel, PeakLevel) over the last `n` samples of every channel.
pub fn levels(chans: &[Vec<f32>], n: usize) -> (f32, f32) {
    let (mut sum, mut peak, mut count) = (0f64, 0f32, 0usize);
    for c in chans {
        for &x in &c[c.len().saturating_sub(n)..] {
            sum += (x as f64) * (x as f64);
            peak = peak.max(x.abs());
            count += 1;
        }
    }
    if count == 0 {
        return (0.0, 0.0);
    }
    ((sum / count as f64).sqrt() as f32, peak)
}

// ------------------------------------------------------------------ effects

/// Parameters of one effect for a block (read from the instance).
pub trait Params {
    fn f(&self, k: &str) -> f32;
    fn b(&self, k: &str) -> bool;
    fn range(&self, k: &str) -> (f32, f32);
    fn e(&self, k: &str) -> u32;
}

pub struct Delay {
    buf: Vec<Vec<f32>>,
    w: usize,
}

impl Delay {
    fn new(len: usize) -> Self {
        Delay { buf: Vec::new(), w: 0 }.sized(len)
    }
    fn sized(mut self, len: usize) -> Self {
        self.buf = vec![vec![0.0; len.max(2)]];
        self
    }
    fn chans(&mut self, n: usize) {
        if self.buf.len() != n {
            let len = self.buf[0].len();
            self.buf = vec![vec![0.0; len]; n];
        }
    }
    fn len(&self) -> usize {
        self.buf[0].len()
    }
    /// Sample `d` samples ago (fractional) on channel c.
    fn read(&self, c: usize, d: f32) -> f32 {
        let len = self.len() as f32;
        let p = (self.w as f32 - d).rem_euclid(len);
        let i = p as usize % self.len();
        let j = (i + 1) % self.len();
        let f = p.fract();
        self.buf[c][i] * (1.0 - f) + self.buf[c][j] * f
    }
    fn write(&mut self, c: usize, x: f32) {
        let w = self.w;
        self.buf[c][w] = x;
    }
    fn advance(&mut self) {
        self.w = (self.w + 1) % self.len();
    }
}

/// One effect node's DSP state.
pub enum Fx {
    Fader,
    Distortion,
    Chorus { d: Delay, phase: f32 },
    Flanger { d: Delay, phase: f32 },
    Echo { d: Delay, cur: f32 },
    Compressor { env: f32 },
    Limiter { env: f32 },
    Gate { env: f32, open: bool, g: f32 },
    Equalizer { low: Chain, high: Chain, key: [f32; 2] },
    Filter { chain: Chain, key: (u32, f32, f32, f32) },
    PitchShifter { d: Delay, phase: f32 },
    Reverb(Box<Reverb>),
    Tremolo { phase: f32 },
}

impl Fx {
    pub fn new(class: &str) -> Option<Fx> {
        Some(match class {
            "AudioFader" => Fx::Fader,
            "AudioDistortion" => Fx::Distortion,
            "AudioChorus" => Fx::Chorus { d: Delay::new((RATE * 0.1) as usize), phase: 0.0 },
            "AudioFlanger" => Fx::Flanger { d: Delay::new((RATE * 0.03) as usize), phase: 0.0 },
            "AudioEcho" => Fx::Echo { d: Delay::new((RATE * 5.0) as usize + 4), cur: -1.0 },
            "AudioCompressor" => Fx::Compressor { env: 0.0 },
            "AudioLimiter" => Fx::Limiter { env: 0.0 },
            "AudioGate" => Fx::Gate { env: 0.0, open: false, g: 0.0 },
            "AudioEqualizer" => Fx::Equalizer { low: Chain::default(), high: Chain::default(), key: [-1.0; 2] },
            "AudioFilter" => Fx::Filter { chain: Chain::default(), key: (u32::MAX, 0.0, 0.0, 0.0) },
            "AudioPitchShifter" => Fx::PitchShifter { d: Delay::new(8192 * 2), phase: 0.0 },
            "AudioReverb" => Fx::Reverb(Box::new(Reverb::new())),
            "AudioTremolo" => Fx::Tremolo { phase: 0.0 },
            _ => return None,
        })
    }

    /// Processes `x` in place. `side` is the Sidechain pin's input (compressor).
    pub fn process(&mut self, p: &dyn Params, x: &mut Buf, side: Option<&Buf>) {
        if p.b("Bypass") {
            return;
        }
        let n = x.first().map_or(0, |c| c.len());
        let ch = x.len();
        match self {
            Fx::Fader => gain(x, p.f("Volume")),
            Fx::Distortion => {
                // Level 0..1 → drive 1..50 into tanh, output normalised.
                let level = p.f("Level").clamp(0.0, 1.0);
                if level <= 0.0 {
                    return;
                }
                let k = 1.0 + level * 49.0;
                let norm = 1.0 / k.tanh();
                for c in x.iter_mut() {
                    for s in c.iter_mut() {
                        *s = (*s * k).tanh() * norm;
                    }
                }
            }
            Fx::Chorus { d, phase } | Fx::Flanger { d, phase } => {
                let flanger = d.len() < (RATE * 0.05) as usize;
                d.chans(ch);
                let (rate, depth, mix) = (p.f("Rate"), p.f("Depth").clamp(0.0, 1.0), p.f("Mix").clamp(0.0, 1.0));
                // Chorus: 20 ms ± depth·15 ms; flanger: 1 ms + depth·10 ms sweep with feedback.
                for i in 0..n {
                    *phase = (*phase + rate / RATE).fract();
                    for c in 0..ch {
                        let ph = *phase * TAU + c as f32 * PI / 2.0;
                        let del = if flanger {
                            (0.001 + depth * 0.005 * (1.0 + ph.sin())) * RATE
                        } else {
                            (0.02 + depth * 0.015 * ph.sin()) * RATE
                        };
                        let wet = d.read(c, del);
                        let dry = x[c][i];
                        d.write(c, if flanger { dry + wet * 0.5 } else { dry });
                        x[c][i] = dry * (1.0 - mix) + wet * mix;
                    }
                    d.advance();
                }
            }
            Fx::Echo { d, cur } => {
                d.chans(ch);
                let target = p.f("DelayTime").clamp(0.001, 5.0) * RATE;
                let ramp = p.f("RampTime").max(0.0) * RATE;
                if *cur < 0.0 {
                    *cur = target;
                }
                let fb = p.f("Feedback").clamp(0.0, 1.0);
                let (dry, wet) = (db(p.f("DryLevel")), db(p.f("WetLevel")));
                for i in 0..n {
                    // DelayTime changes glide over RampTime.
                    if ramp > 0.0 {
                        let step = (target - *cur).abs() / ramp.max(1.0);
                        *cur += (target - *cur).clamp(-step.max(1e-6), step.max(1e-6));
                    } else {
                        *cur = target;
                    }
                    for c in 0..ch {
                        let y = d.read(c, *cur);
                        let s = x[c][i];
                        d.write(c, s + y * fb);
                        x[c][i] = s * dry + y * wet;
                    }
                    d.advance();
                }
            }
            Fx::Compressor { env } => {
                let (thr, ratio) = (p.f("Threshold"), p.f("Ratio").max(1.0));
                let atk = (-1.0 / (p.f("Attack").max(1e-4) * RATE)).exp();
                let rel = (-1.0 / (p.f("Release").max(1e-4) * RATE)).exp();
                let makeup = p.f("MakeupGain");
                let key = side.unwrap_or(x);
                let mut gains = vec![0f32; n];
                for (i, g) in gains.iter_mut().enumerate() {
                    let peak = key.iter().map(|c| c.get(i).map_or(0.0, |s| s.abs())).fold(0.0, f32::max);
                    let a = if peak > *env { atk } else { rel };
                    *env = a * *env + (1.0 - a) * peak;
                    let level = 20.0 * env.max(1e-9).log10();
                    let red = (level - thr).max(0.0) * (1.0 - 1.0 / ratio);
                    *g = db(makeup - red);
                }
                for c in x.iter_mut() {
                    for (s, g) in c.iter_mut().zip(&gains) {
                        *s *= g;
                    }
                }
            }
            Fx::Limiter { env } => {
                let ceil = db(p.f("MaxLevel").min(0.0));
                let rel = (-1.0 / (p.f("Release").max(1e-4) * RATE)).exp();
                for i in 0..n {
                    let peak = x.iter().map(|c| c[i].abs()).fold(0.0, f32::max);
                    *env = if peak > *env { peak } else { rel * *env + (1.0 - rel) * peak };
                    let g = if *env > ceil { ceil / *env } else { 1.0 };
                    for c in x.iter_mut() {
                        c[i] *= g;
                    }
                }
            }
            Fx::Gate { env, open, g } => {
                let (lo, hi) = p.range("Threshold");
                let atk = 1.0 / (p.f("Attack").max(1e-3) * RATE);
                let rel = 1.0 / (p.f("Release").max(1e-3) * RATE);
                let follow = (-1.0 / (0.002 * RATE)).exp();
                for i in 0..n {
                    let peak = x.iter().map(|c| c[i].abs()).fold(0.0, f32::max);
                    *env = if peak > *env { peak } else { follow * *env + (1.0 - follow) * peak };
                    let lvl = 20.0 * env.max(1e-9).log10();
                    // Hysteresis: opens above the range's max, closes below its min.
                    if lvl > hi {
                        *open = true;
                    } else if lvl < lo {
                        *open = false;
                    }
                    *g = if *open { (*g + atk).min(1.0) } else { (*g - rel).max(0.0) };
                    for c in x.iter_mut() {
                        c[i] *= *g;
                    }
                }
            }
            Fx::Equalizer { low, high, key } => {
                let (a, b) = p.range("MidRange");
                if key[0] != a || key[1] != b {
                    *key = [a, b];
                    low.set(vec![Biquad::rbj("lowpass", a, FRAC_1_SQRT_2, 0.0); 2]);
                    high.set(vec![Biquad::rbj("lowpass", b, FRAC_1_SQRT_2, 0.0); 2]);
                }
                // Linkwitz-Riley crossovers: low = LR4(a), high = x - LR4(b).
                let mut l = x.clone();
                low.run(&mut l);
                let mut below_b = x.clone();
                high.run(&mut below_b);
                let (gl, gm, gh) = (db(p.f("LowGain")), db(p.f("MidGain")), db(p.f("HighGain")));
                for c in 0..ch {
                    for i in 0..n {
                        let lo = l[c][i];
                        let mid = below_b[c][i] - lo;
                        let hi = x[c][i] - below_b[c][i];
                        x[c][i] = lo * gl + mid * gm + hi * gh;
                    }
                }
            }
            Fx::Filter { chain, key } => {
                let k = (p.e("FilterType"), p.f("Frequency"), p.f("Q"), p.f("Gain"));
                if *key != k {
                    *key = k;
                    chain.set(filter_stages(k.0, k.1, k.2, k.3));
                }
                chain.run(x);
            }
            Fx::PitchShifter { d, phase } => {
                d.chans(ch);
                let pitch = p.f("Pitch").clamp(0.5, 2.0);
                let win = match p.e("WindowSize") {
                    0 => 1024.0,
                    2 => 8192.0,
                    _ => 4096.0,
                };
                // Two crossfaded read heads sliding through a window (granular).
                for i in 0..n {
                    *phase = (*phase + (1.0 - pitch) / win).rem_euclid(1.0);
                    for c in 0..ch {
                        let s = x[c][i];
                        d.write(c, s);
                        if (pitch - 1.0).abs() < 1e-6 {
                            continue;
                        }
                        let mut y = 0.0;
                        for ph in [*phase, (*phase + 0.5).fract()] {
                            y += d.read(c, 1.0 + ph * win) * (PI * ph).sin().powi(2);
                        }
                        x[c][i] = y;
                    }
                    d.advance();
                }
            }
            Fx::Reverb(r) => r.process(p, x),
            Fx::Tremolo { phase } => {
                let (freq, depth) = (p.f("Frequency"), p.f("Depth").clamp(0.0, 1.0));
                let (duty, shape, skew, square) = (
                    p.f("Duty").clamp(0.0, 1.0),
                    p.f("Shape").clamp(0.0, 1.0),
                    p.f("Skew").clamp(-1.0, 1.0),
                    p.f("Square").clamp(0.0, 1.0),
                );
                for i in 0..n {
                    *phase = (*phase + freq / RATE).fract();
                    let w = tremolo_wave(*phase, duty, shape, skew, square);
                    let g = 1.0 - depth * (1.0 - w);
                    for c in x.iter_mut() {
                        c[i] *= g;
                    }
                }
            }
        }
    }
}

/// Tremolo LFO in [0, 1]: Duty is the share of the cycle spent in the pulse,
/// Skew moves the pulse's peak earlier (-1) or later (+1), Shape morphs the
/// pulse from a raised cosine (0) to a triangle (1), Square flattens it
/// towards a square wave.
pub fn tremolo_wave(phase: f32, duty: f32, shape: f32, skew: f32, square: f32) -> f32 {
    if duty <= 0.0 {
        return 0.0;
    }
    if phase >= duty {
        return 0.0;
    }
    let t = phase / duty; // 0..1 inside the pulse
    let peak = (0.5 + 0.5 * skew).clamp(1e-3, 1.0 - 1e-3);
    let tri = if t < peak { t / peak } else { (1.0 - t) / (1.0 - peak) };
    let cos = 0.5 - 0.5 * (PI * tri).cos();
    let w = cos * (1.0 - shape) + tri * shape;
    // Square: push towards 1 everywhere inside the pulse.
    let sq = if square > 0.0 { (w / (1.0 - square * 0.999)).min(1.0) } else { w };
    sq.clamp(0.0, 1.0)
}

/// Feedback-delay-network reverb with Studio's AudioReverb parameters:
/// EarlyDelayTime (pre-delay of the early reflections), LateDelayTime (gap
/// to the tail), DecayTime (RT60 at ReferenceFrequency), DecayRatio (high
/// frequency decay relative to that), Density (tail line spread), Diffusion
/// (all-pass amount), HighCutFrequency / LowShelfFrequency / LowShelfGain
/// (tail tone), DryLevel / WetLevel (dB).
pub struct Reverb {
    lines: Vec<Vec<f32>>,
    pos: Vec<usize>,
    damp: Vec<f32>,
    pre: Delay,
    ap: Vec<(Vec<f32>, usize)>,
    tone: Chain,
    tone_key: [f32; 3],
}

const FDN: [f32; 8] = [0.0297, 0.0371, 0.0411, 0.0437, 0.0533, 0.0617, 0.0719, 0.0797];

impl Reverb {
    fn new() -> Self {
        Reverb {
            lines: FDN.iter().map(|t| vec![0.0; (t * RATE) as usize + 1]).collect(),
            pos: vec![0; FDN.len()],
            damp: vec![0.0; FDN.len()],
            pre: Delay::new((RATE * 0.45) as usize),
            ap: [0.0051, 0.0077, 0.0101]
                .iter()
                .map(|t| (vec![0.0; (t * RATE) as usize], 0))
                .collect(),
            tone: Chain::default(),
            tone_key: [-1.0; 3],
        }
    }
    fn process(&mut self, p: &dyn Params, x: &mut Buf) {
        let n = x.first().map_or(0, |c| c.len());
        let ch = x.len();
        self.pre.chans(1);
        let rt60 = p.f("DecayTime").max(0.1);
        let ratio = p.f("DecayRatio").clamp(0.1, 1.0);
        let density = p.f("Density").clamp(0.1, 1.0);
        let diffusion = p.f("Diffusion").clamp(0.1, 1.0);
        let early = p.f("EarlyDelayTime").clamp(0.0, 0.3) * RATE;
        let late = p.f("LateDelayTime").clamp(0.0, 0.1) * RATE;
        let (dry, wet) = (db(p.f("DryLevel")), db(p.f("WetLevel")));
        let key = [p.f("HighCutFrequency"), p.f("LowShelfFrequency"), p.f("LowShelfGain")];
        if key != self.tone_key {
            self.tone_key = key;
            self.tone.set(vec![
                Biquad::rbj("lowpass", key[0].min(RATE * 0.49), FRAC_1_SQRT_2, 0.0),
                Biquad::rbj("lowshelf", key[1], FRAC_1_SQRT_2, key[2]),
            ]);
        }
        // High frequencies decay in rt60·ratio: a one-pole damping per line.
        let ref_f = p.f("ReferenceFrequency").clamp(20.0, 20000.0);
        let damp_a = (1.0 - ratio) * (1.0 - (-TAU * ref_f / RATE).exp()).min(1.0);
        let mut out = silence(2, n);
        for i in 0..n {
            let mono = x.iter().map(|c| c[i]).sum::<f32>() / ch as f32;
            self.pre.write(0, mono);
            let er = self.pre.read(0, early.max(1.0));
            let tail_in = self.pre.read(0, (early + late).max(1.0));
            self.pre.advance();
            // Diffusion all-passes.
            let mut s = tail_in;
            for (buf, w) in self.ap.iter_mut() {
                let d = buf[*w];
                let g = 0.6 * diffusion;
                let y = -g * s + d;
                buf[*w] = s + g * y;
                *w = (*w + 1) % buf.len();
                s = y;
            }
            let k = FDN.len();
            let outs: Vec<f32> = (0..k)
                .map(|j| {
                    let len = ((self.lines[j].len() - 1) as f32 * (0.55 + 0.45 * density)) as usize;
                    let r = (self.pos[j] + self.lines[j].len() - len.max(1)) % self.lines[j].len();
                    self.lines[j][r]
                })
                .collect();
            let sum: f32 = outs.iter().sum::<f32>() * (2.0 / k as f32);
            for j in 0..k {
                let len = (self.lines[j].len() - 1) as f32 * (0.55 + 0.45 * density);
                let g = 10f32.powf(-3.0 * len / RATE / rt60);
                // Householder feedback.
                let fbk = (outs[j] - sum) * g;
                self.damp[j] += damp_a * (fbk - self.damp[j]);
                let fbk = fbk - (fbk - self.damp[j]) * (1.0 - ratio);
                let w = self.pos[j];
                self.lines[j][w] = s * 0.35 + fbk;
                self.pos[j] = (w + 1) % self.lines[j].len();
            }
            let l: f32 = outs.iter().step_by(2).sum::<f32>() * 0.5 + er * 0.3;
            let r: f32 = outs.iter().skip(1).step_by(2).sum::<f32>() * 0.5 + er * 0.3;
            out[0][i] = l;
            out[1][i] = r;
        }
        self.tone.run(&mut out);
        let out = convert(&out, ch);
        for c in 0..ch {
            for i in 0..n {
                x[c][i] = x[c][i] * dry + out[c][i] * wet;
            }
        }
    }
}

// --------------------------------------------------------------- generators

/// AudioWindSynthesizer: filtered noise whose loudness and brightness follow
/// the wind speed (Workspace.GlobalWind); Profile changes the gust pattern.
#[derive(Default)]
pub struct Wind {
    seed: u32,
    lp: [f32; 2],
    gust: f32,
    t: f32,
}

impl Wind {
    pub fn render(&mut self, n: usize, speed: f32, profile: u32, volume: f32) -> Buf {
        let mut out = vec![0f32; n];
        let s = (speed / 40.0).clamp(0.0, 1.5);
        let cutoff = 150.0 + 1200.0 * s;
        let a = 1.0 - (-TAU * cutoff / RATE).exp();
        for o in out.iter_mut() {
            self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let white = (self.seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            self.lp[0] += a * (white - self.lp[0]);
            self.lp[1] += a * (self.lp[0] - self.lp[1]);
            self.t += 1.0 / RATE;
            let gust = match profile {
                // Turbulence: fast, irregular swells.
                0 => 0.7 + 0.3 * (self.t * 1.7).sin() * (self.t * 0.37).cos(),
                // Calm / smooth profiles: slow swells.
                _ => 0.85 + 0.15 * (self.t * 0.5).sin(),
            };
            self.gust = gust;
            *o = self.lp[1] * 3.0 * s * gust * volume;
        }
        vec![out]
    }
}

/// Listener panning: (left, right) gains for a source at local direction
/// `dir` (listener space, +X right, -Z forward) and distance `d`.
/// Fitted to Studio (checks/parity/media.md): equal power, pan angle
/// 45°·(1 + F(x/d)·G(d)) with F through (0,0) (0.5,0.46) (0.866,0.84)
/// (1,1) and G (near-field collapse) through (0.25,0.056) (0.5,0.21)
/// (1,0.596) (2,0.92) (4,0.993) (8,1).
pub fn pan(dir_x: f32, d: f32) -> (f32, f32) {
    if d <= 1e-6 {
        return (FRAC_1_SQRT_2, FRAC_1_SQRT_2);
    }
    let s = (dir_x / d).clamp(-1.0, 1.0);
    let f = interp(&[(0.0, 0.0), (0.5, 0.46), (0.866, 0.84), (1.0, 1.0)], s.abs()) * s.signum();
    let g = interp(
        &[(0.0, 0.0), (0.25, 0.056), (0.5, 0.21), (1.0, 0.596), (2.0, 0.92), (4.0, 0.993), (8.0, 1.0)],
        d,
    );
    let theta = PI / 4.0 * (1.0 + f * g);
    (theta.cos(), theta.sin())
}

/// Piecewise-linear lookup, clamped at the ends.
pub fn interp(pts: &[(f32, f32)], x: f32) -> f32 {
    if pts.is_empty() {
        return 1.0;
    }
    if x <= pts[0].0 {
        return pts[0].1;
    }
    for w in pts.windows(2) {
        if x <= w[1].0 {
            let t = (x - w[0].0) / (w[1].0 - w[0].0).max(1e-9);
            return w[0].1 + (w[1].1 - w[0].1) * t;
        }
    }
    pts[pts.len() - 1].1
}

/// AudioEmitter distance attenuation (Enum.DistanceAttenuationMode value),
/// measured in Studio: Custom uses the curve (or, empty, Inverse over 4..10000);
/// Inverse = min/d; Linear = (max-d)/(max-min); LinearSquared its square;
/// InverseTapered = min(Inverse, LinearSquared); 0 at or past max.
pub fn distance_gain(mode: u32, curve: &[(f32, f32)], bounds: (f32, f32), d: f32) -> f32 {
    let (min, max) = if mode == 0 { (4.0, 10000.0) } else { bounds };
    if mode == 0 && !curve.is_empty() {
        return interp(curve, d);
    }
    let min = min.max(1e-3);
    let max = max.max(min);
    if d >= max {
        return 0.0;
    }
    if d <= min {
        return 1.0;
    }
    let inv = min / d;
    let lin = (max - d) / (max - min);
    match mode {
        2 => lin,
        3 => lin * lin,
        1 => inv.min(lin * lin),
        _ => inv,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, a: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| a * (TAU * f * i as f32 / RATE).sin()).collect()
    }

    #[test]
    fn filter_gain_matches_studio() {
        // Studio, Frequency 2000, Q 0.707, Gain 6, at 20/1000/2000/4000/20000 Hz.
        let studio: [(u32, [f32; 5]); 11] = [
            (0, [0.001, 2.810, 6.000, 2.748, 0.016]),
            (1, [6.000, 5.630, 3.000, 0.353, 0.000]),
            (2, [-0.000, 0.370, 3.000, 5.647, 6.000]),
            (3, [-0.000, -0.260, -3.012, -12.592, -58.101]),
            (4, [0.001, 0.759, -3.012, -23.936, -116.195]),
            (5, [0.001, 2.610, -3.012, -46.811, -232.385]),
            (6, [-80.058, -12.376, -3.012, -0.247, -0.000]),
            (7, [-160.191, -23.473, -3.012, 0.753, 0.006]),
            (8, [-320.415, -45.854, -3.012, 2.568, 0.018]),
            (9, [-37.038, -3.306, -0.000, -3.407, -26.039]),
            (11, [-0.000, -0.963, -2.986, -6.891, -17.415]),
        ];
        for (kind, want) in studio {
            for (f, w) in [20.0, 1000.0, 2000.0, 4000.0, 20000.0].iter().zip(want) {
                let g = filter_gain_at(kind, 2000.0, 0.707, 6.0, *f);
                let tol = if w.abs() > 50.0 { 0.2 } else { 0.01 };
                assert!((g - w).abs() < tol, "type {kind} at {f}: {g} vs Studio {w}");
            }
        }
    }

    #[test]
    fn analyzer_matches_studio() {
        // Studio: 984.375 Hz sine, amplitude 0.5 (×1.011 Studio path gain):
        // bins 21/22/23 = 0.14637 / 0.34196 / 0.14596.
        let x = sine(984.375, 0.5, 4800);
        let s = spectrum(&[x.clone()], 1024);
        assert_eq!(s.len(), 512);
        let g = 1.011;
        assert!((s[21] * g - 0.342).abs() < 0.004, "{}", s[21]);
        assert!((s[20] * g - 0.1464).abs() < 0.004, "{}", s[20]);
        let (rms, peak) = levels(&[x], 1024);
        assert!((rms - 0.3536).abs() < 0.003);
        assert!((peak - 0.5).abs() < 0.01);
        // DC 0.25: bin 0 = 0.3372 (Studio 0.34133 / its 1.0123 path gain).
        let dc = vec![0.25f32; 2048];
        let s = spectrum(&[dc], 1024);
        assert!((s[0] - 0.3372).abs() < 0.003, "{}", s[0]);
        // Stereo, sine on the left only: RMS over channels.
        let st = [sine(1000.0, 0.5, 2048), vec![0.0; 2048]];
        let (rms, _) = levels(&st, 1024);
        assert!((rms - 0.25).abs() < 0.003);
    }

    #[test]
    fn channel_conversion_matches_studio() {
        let mono = vec![vec![0.25f32; 8]];
        let st = convert(&mono, 2);
        assert!((st[0][0] - 0.1768).abs() < 1e-3 && (st[1][0] - 0.1768).abs() < 1e-3);
        let back = convert(&st, 1);
        assert!((back[0][0] - 0.25).abs() < 1e-3);
        let six = convert(&mono, 6);
        assert_eq!(six[2][0], 0.0); // centre stays empty (Studio)
        assert!((six[0][0] - 0.1768).abs() < 1e-3);
    }

    #[test]
    fn distance_curves_match_studio() {
        // Studio, bounds 2..50.
        let b = (2.0, 50.0);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(near(distance_gain(4, &[], b, 4.0), 0.5));
        assert!(near(distance_gain(4, &[], b, 50.0), 0.0));
        assert!(near(distance_gain(2, &[], b, 26.0), 0.5));
        assert!(near(distance_gain(3, &[], b, 26.0), 0.25));
        assert!(near(distance_gain(1, &[], b, 40.0), 0.0434));
        // Custom, empty: bounds ignored (Studio: 0.2 at 20 for 2..50).
        assert!(near(distance_gain(0, &[], b, 20.0), 0.2));
        let curve = [(0.0, 1.0), (10.0, 0.5), (30.0, 0.1)];
        assert!(near(distance_gain(0, &curve, b, 20.0), 0.3));
        assert!(near(distance_gain(0, &curve, b, 50.0), 0.1));
    }

    #[test]
    fn pan_matches_studio() {
        // Studio, DC 0.25 mono: (L,R) at 2 studs right = 0.0158 / 0.2525.
        let (l, r) = pan(2.0, 2.0);
        assert!((l * 0.253 - 0.0158).abs() < 0.003, "{l}");
        assert!((r * 0.253 - 0.2525).abs() < 0.003);
        let (l, _) = pan(4.0 * 0.5, 4.0);
        assert!((l * 0.253 - 0.1046).abs() < 0.006, "{l}");
    }

    #[test]
    fn effects_render_silently_and_sanely() {
        struct P;
        impl Params for P {
            fn f(&self, k: &str) -> f32 {
                match k {
                    "Volume" => 0.5,
                    "DelayTime" => 0.01,
                    "Feedback" => 0.5,
                    "DecayTime" => 1.5,
                    "DecayRatio" => 0.5,
                    "Density" | "Diffusion" => 1.0,
                    "WetLevel" => -6.0,
                    "HighCutFrequency" => 20000.0,
                    "LowShelfFrequency" => 250.0,
                    "ReferenceFrequency" => 5000.0,
                    "EarlyDelayTime" => 0.02,
                    "LateDelayTime" => 0.04,
                    "Pitch" => 1.25,
                    "Frequency" => 5.0,
                    "Depth" => 1.0,
                    "Duty" => 0.5,
                    "Threshold" => -40.0,
                    "Ratio" => 40.0,
                    "Attack" | "Release" => 0.1,
                    "Level" => 0.5,
                    "Rate" => 5.0,
                    "Mix" => 0.85,
                    "Q" => 0.707,
                    _ => 0.0,
                }
            }
            fn b(&self, _: &str) -> bool {
                false
            }
            fn range(&self, k: &str) -> (f32, f32) {
                if k == "MidRange" { (400.0, 4000.0) } else { (-36.0, -24.0) }
            }
            fn e(&self, _: &str) -> u32 {
                1
            }
        }
        for class in [
            "AudioFader", "AudioDistortion", "AudioChorus", "AudioFlanger", "AudioEcho",
            "AudioCompressor", "AudioLimiter", "AudioGate", "AudioEqualizer", "AudioFilter",
            "AudioPitchShifter", "AudioReverb", "AudioTremolo",
        ] {
            let mut fx = Fx::new(class).unwrap();
            let mut b = vec![sine(440.0, 0.5, 4800), sine(440.0, 0.5, 4800)];
            fx.process(&P, &mut b, None);
            let (rms, peak) = levels(&b, 4800);
            assert!(rms.is_finite() && peak < 4.0, "{class}: {rms} {peak}");
        }
        // Fader halves.
        let mut b = vec![vec![1.0f32; 4]];
        Fx::new("AudioFader").unwrap().process(&P, &mut b, None);
        assert_eq!(b[0][0], 0.5);
    }
}
