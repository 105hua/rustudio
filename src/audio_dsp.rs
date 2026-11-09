//! Stereo DSP graph used by both the device and the silent WAV renderer.
//! Reverb is a Schroeder approximation, not FMOD's proprietary SFX implementation.
use rodio::{ChannelCount, SampleRate, Source};
use std::sync::{Arc, Mutex};

pub const RATE: u32 = 48000;
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub id: u64,
    pub kind: String,
    pub p: Vec<f32>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub gain: f32,
    pub pan: Option<f32>,
    pub effects: Vec<Effect>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            gain: 1.,
            pan: None,
            effects: vec![],
        }
    }
}
pub type Control = Arc<Mutex<Settings>>;
pub fn db(v: f32) -> f32 {
    10f32.powf(v / 20.)
}
/// Roblox enum: Inverse=0, Linear=1, LinearSquare=2, InverseTapered=3.
pub fn rolloff(mode: u32, distance: f32, min: f32, max: f32, scale: f32) -> f32 {
    let min = min.max(0.001);
    let max = max.max(min);
    if distance >= max {
        return 0.;
    }
    if distance <= min {
        return 1.;
    }
    let linear = ((max - distance) / (max - min)).clamp(0., 1.);
    let inverse = min / (min + (distance - min) * scale.max(0.));
    match mode {
        1 => linear,
        2 => linear * linear,
        3 => inverse.min(linear * linear),
        _ => inverse,
    }
}
struct State {
    spec: Effect,
    delay: Vec<[f32; 2]>,
    cursor: usize,
    env: f32,
    low: [f32; 2],
    high: [f32; 2],
    phase: f32,
}
impl State {
    fn new(spec: Effect) -> Self {
        let n = match spec.kind.as_str() {
            "EchoSoundEffect" => RATE as usize * 5,
            "ReverbSoundEffect" => RATE as usize / 5 * 4,
            "ChorusSoundEffect" | "FlangeSoundEffect" | "PitchShiftSoundEffect" => {
                RATE as usize / 10
            }
            _ => 0,
        };
        Self {
            spec,
            delay: vec![[0.; 2]; n],
            cursor: 0,
            env: 0.,
            low: [0.; 2],
            high: [0.; 2],
            phase: 0.,
        }
    }
    fn process(&mut self, mut x: [f32; 2]) -> [f32; 2] {
        let p = &self.spec.p;
        let v = |i: usize, d: f32| p.get(i).copied().unwrap_or(d);
        match self.spec.kind.as_str() {
            "EchoSoundEffect" => {
                let n = (v(0, 1.).clamp(0.001, 5.) * RATE as f32) as usize;
                let j = (self.cursor + self.delay.len() - n.min(self.delay.len() - 1))
                    % self.delay.len();
                let old = self.delay[j];
                for c in 0..2 {
                    self.delay[self.cursor][c] = x[c] + old[c] * v(1, 0.5).clamp(0., 0.99);
                    x[c] = x[c] * db(v(3, 0.)) + old[c] * db(v(2, -6.));
                }
                self.cursor = (self.cursor + 1) % self.delay.len();
            }
            "ReverbSoundEffect" => {
                let dry = db(v(4, -6.));
                let wet = db(v(3, -6.));
                let mut sum = [0.; 2];
                for (i, t) in [0.0297, 0.0371, 0.0411, 0.0437].iter().enumerate() {
                    let n = (*t * RATE as f32 * (0.7 + 0.3 * v(1, 1.).clamp(0., 1.))) as usize;
                    let j = i * (RATE as usize / 5) + self.cursor % n;
                    let g = 10f32.powf(-3. * *t / v(0, 1.).max(0.1));
                    for c in 0..2 {
                        let y = self.delay[j][c];
                        sum[c] += y * 0.25;
                        self.delay[j][c] =
                            x[c] + g * (y * (1. - 0.15 * v(2, 1.)) + self.low[c] * 0.15 * v(2, 1.));
                    }
                    let _ = i;
                }
                for c in 0..2 {
                    self.low[c] += 0.2 * (sum[c] - self.low[c]);
                    x[c] = x[c] * dry + sum[c] * wet;
                }
                self.cursor = self.cursor.wrapping_add(1);
            }
            "EqualizerSoundEffect" => {
                let a = 1. - (-std::f32::consts::TAU * 400. / RATE as f32).exp();
                let b = 1. - (-std::f32::consts::TAU * 4000. / RATE as f32).exp();
                for c in 0..2 {
                    self.low[c] += a * (x[c] - self.low[c]);
                    self.high[c] += b * (x[c] - self.high[c]);
                    x[c] = self.low[c] * db(v(0, 0.))
                        + (self.high[c] - self.low[c]) * db(v(1, 0.))
                        + (x[c] - self.high[c]) * db(v(2, 0.));
                }
            }
            "CompressorSoundEffect" => {
                let peak = x[0].abs().max(x[1].abs()).max(1e-12);
                let seconds = if peak > self.env {
                    v(2, 0.1)
                } else {
                    v(3, 0.1)
                }
                .max(0.0001);
                let a = (-1. / (seconds * RATE as f32)).exp();
                self.env = a * self.env + (1. - a) * peak;
                let level = 20. * self.env.max(1e-12).log10();
                let reduction = (level - v(0, -40.)).max(0.) * (1. - 1. / v(1, 40.).max(1.));
                let gain = db(v(4, 0.) - reduction);
                x = x.map(|s| s * gain);
            }
            "ChorusSoundEffect" | "FlangeSoundEffect" => {
                self.phase = (self.phase + v(0, 0.5).clamp(0., 20.) / RATE as f32).fract();
                let flange = self.spec.kind == "FlangeSoundEffect";
                let base = if flange { 0.002 } else { 0.02 };
                let depth = v(1, 0.15).clamp(0., 1.);
                let mix = v(2, 0.5).clamp(0., 1.);
                for c in 0..2 {
                    let phase =
                        self.phase * std::f32::consts::TAU + c as f32 * std::f32::consts::FRAC_PI_2;
                    let delay = base * (1. + depth * phase.sin()) * RATE as f32;
                    let at = (self.cursor as f32 + self.delay.len() as f32 - delay)
                        .rem_euclid(self.delay.len() as f32);
                    let a = at as usize;
                    let b = (a + 1) % self.delay.len();
                    let wet = self.delay[a][c] * (1. - at.fract()) + self.delay[b][c] * at.fract();
                    self.delay[self.cursor][c] = x[c] + if flange { wet * 0.5 } else { 0. };
                    x[c] = x[c] * (1. - mix) + wet * mix;
                }
                self.cursor = (self.cursor + 1) % self.delay.len();
            }
            "PitchShiftSoundEffect" => {
                let pitch = v(0, 1.).clamp(0.5, 2.);
                let window = (self.delay.len() / 2) as f32;
                self.phase = (self.phase + (1. - pitch) / window).rem_euclid(1.);
                for c in 0..2 {
                    self.delay[self.cursor][c] = x[c];
                    let mut wet = 0.;
                    for phase in [self.phase, (self.phase + 0.5).fract()] {
                        let at =
                            (self.cursor as f32 + self.delay.len() as f32 - 1. - phase * window)
                                .rem_euclid(self.delay.len() as f32);
                        let i = at as usize;
                        let y = self.delay[i][c] * (1. - at.fract())
                            + self.delay[(i + 1) % self.delay.len()][c] * at.fract();
                        wet += y * (1. - (2. * phase - 1.).abs());
                    }
                    x[c] = if (pitch - 1.).abs() < 1e-6 { x[c] } else { wet };
                }
                self.cursor = (self.cursor + 1) % self.delay.len();
            }
            "DistortionSoundEffect" => {
                let k = 1. + v(0, 0.5).clamp(0., 1.) * 99.;
                x = x.map(|s| (s * k).tanh() / k.tanh());
            }
            "TremoloSoundEffect" => {
                self.phase = (self.phase + v(0, 5.) / RATE as f32).fract();
                let g = 1.
                    - v(1, 1.).clamp(0., 1.)
                        * (0.5 + 0.5 * (self.phase * std::f32::consts::TAU).sin());
                x = x.map(|s| s * g);
            }
            _ => {}
        }
        x
    }
}
/// Always stereo/48 kHz. A bus remains alive when its input mixer is empty.
pub struct Process<S> {
    inner: S,
    control: Control,
    settings: Settings,
    states: Vec<State>,
    right: Option<f32>,
    frames: u32,
    continuous: bool,
    key: Option<u64>,
    energy: f64,
    peak: f32,
}
impl<S> Process<S> {
    pub fn new(inner: S, control: Control, continuous: bool) -> Self {
        Self {
            inner,
            control,
            settings: Settings::default(),
            states: vec![],
            right: None,
            frames: 0,
            continuous,
            key: None,
            energy: 0.,
            peak: 0.,
        }
    }
    pub fn tagged(mut self, key: u64) -> Self {
        self.key = Some(key);
        self
    }
}
impl<S: Source> Iterator for Process<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        if self.frames % 128 == 0 {
            let settings = self.control.lock().unwrap().clone();
            if settings.effects != self.settings.effects {
                let mut old = std::mem::take(&mut self.states);
                self.states = settings
                    .effects
                    .iter()
                    .map(|e| {
                        if let Some(i) = old
                            .iter()
                            .position(|s| s.spec.id == e.id && s.spec.kind == e.kind)
                        {
                            let mut state = old.remove(i);
                            state.spec = e.clone();
                            state
                        } else {
                            State::new(e.clone())
                        }
                    })
                    .collect();
            }
            self.settings = settings;
        }
        let left = match self.inner.next() {
            Some(s) => s,
            None if self.continuous => 0.,
            None => return None,
        };
        let mut x = [left, self.inner.next().unwrap_or(0.)];
        for s in &mut self.states {
            x = s.process(x);
        }
        if let Some(pan) = self.settings.pan {
            let mono = (x[0] + x[1]) * 0.5;
            let angle = (pan.clamp(-1., 1.) + 1.) * std::f32::consts::FRAC_PI_4;
            x = [mono * angle.cos(), mono * angle.sin()];
        }
        x = x.map(|s| s * self.settings.gain);
        self.energy += x.iter().map(|s| (*s as f64).powi(2)).sum::<f64>();
        self.peak = self.peak.max(x[0].abs()).max(x[1].abs());
        if self.frames % RATE == RATE - 1 {
            if let (Some(key), Some(path)) = (self.key, std::env::var_os("RUSTUDIO_AUDIO_RMS")) {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(
                        f,
                        "key={key} frames={} rms={} peak={} gain={} pan={:?} effects={:?}",
                        self.frames + 1,
                        (self.energy / (2. * RATE as f64)).sqrt(),
                        self.peak,
                        self.settings.gain,
                        self.settings.pan,
                        self.settings.effects
                    );
                }
            }
            self.energy = 0.;
            self.peak = 0.;
        }
        self.frames = self.frames.wrapping_add(1);
        self.right = Some(x[1]);
        Some(x[0])
    }
}
impl<S: Source> Source for Process<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(RATE).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_rolloff_curves() {
        for m in 0..4 {
            assert_eq!(rolloff(m, 0., 6., 120., 1.), 1.);
            assert_eq!(rolloff(m, 120., 6., 120., 1.), 0.);
            let mut prev = 1.;
            for d in 6..120 {
                let g = rolloff(m, d as f32, 6., 120., 1.);
                assert!(g <= prev);
                prev = g;
            }
        }
        assert!((rolloff(3, 60., 6., 120., 1.) - 0.1).abs() < 1e-6);
        assert!((rolloff(2, 63., 6., 120., 1.) - 0.25).abs() < 1e-6);
    }
    #[test]
    fn audio_echo_impulse() {
        let mut s = State::new(Effect {
            id: 1,
            kind: "EchoSoundEffect".into(),
            p: vec![0.01, 0.5, 0., 0.],
        });
        assert_eq!(s.process([1.; 2]), [1.; 2]);
        for _ in 1..480 {
            assert_eq!(s.process([0.; 2]), [0.; 2]);
        }
        assert_eq!(s.process([0.; 2]), [1.; 2]);
        for _ in 481..960 {
            s.process([0.; 2]);
        }
        assert_eq!(s.process([0.; 2]), [0.5; 2]);
    }
    #[test]
    fn audio_compressor_settles() {
        let mut s = State::new(Effect {
            id: 1,
            kind: "CompressorSoundEffect".into(),
            p: vec![-20., 2., 0.01, 0.1, 0.],
        });
        let mut y = [0.; 2];
        for _ in 0..48000 {
            y = s.process([1.; 2]);
        }
        assert!((y[0] - db(-10.)).abs() < 0.001);
    }
    #[test]
    fn audio_reverb_has_tail_and_is_stable() {
        let mut s = State::new(Effect {
            id: 1,
            kind: "ReverbSoundEffect".into(),
            p: vec![1.5, 1., 0.9, 0., -80.],
        });
        s.process([1.; 2]);
        let mut energy = 0.;
        for _ in 0..RATE * 5 {
            let y = s.process([0.; 2]);
            assert!(y[0].is_finite() && y[0].abs() < 2.);
            energy += y[0] * y[0];
        }
        assert!(energy > 0.01);
    }
}
