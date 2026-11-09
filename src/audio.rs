//! Audio engine for Roblox `Sound` instances, built on rodio (symphonia decoders: Ogg Vorbis,
//! MP3, WAV, FLAC; output through cpal/ALSA, which PipeWire serves).
//!
//! Every sound is keyed by a caller-chosen `u64` (normally the Sound instance's id). Each key
//! owns one rodio `Player` connected to a shared mixer. The output device is opened lazily on
//! the first `play()` from a helper thread, so `Audio::new()` never blocks or panics. When
//! `RUSTUDIO_NO_AUDIO` is set, or no device can be opened, the engine runs silently: it still
//! decodes (so bad data is reported), tracks pause/speed/loop state and times every sound with
//! its decoded length and `Instant`, so `TimePosition`, `IsPlaying`, `Ended` and `DidLoop` keep
//! working in tests and headless runs.
//!
//! Timing model: each sound keeps a clock in source seconds
//! (`base + wall_elapsed * PlaybackSpeed`, frozen while paused). With a device, the end of a
//! sound is also detected from the player draining, whichever comes first (the clock gets a
//! small slack so it does not cut off audio that is still buffered).
#![allow(dead_code)] // parts of the API are not wired into the app yet

use crate::audio_dsp::{self as dsp, Control, Process, Settings};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use rodio::mixer::Mixer;
use rodio::source::SeekError;
use rodio::{ChannelCount, Decoder, Player, SampleRate, Source};

/// How long the first `play()` waits for the output device to open. Later calls never wait;
/// sounds started before the device is ready are timed silently.
const OPEN_WAIT: Duration = Duration::from_millis(1500);
/// With a device, the clock only declares a sound ended this long after its nominal end, so
/// the player draining (the real end) normally wins.
const DEVICE_SLACK: f64 = 0.25;
const MAX_SPEED: f32 = 100.0;

/// Roblox volume is linear amplitude (individual properties clamp at the caller).
pub fn volume_gain(volume: f32) -> f32 {
    if volume.is_nan() {
        0.
    } else {
        volume.max(0.)
    }
}

fn sane_speed(s: f32) -> f32 {
    if s.is_nan() {
        0.0
    } else {
        s.clamp(0.0, MAX_SPEED)
    }
}

/// Shared, cheaply clonable byte buffer for the decoder's `Cursor`.
#[derive(Clone)]
struct Bytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

type Dec = Decoder<Cursor<Bytes>>;

/// Best-effort description of what the bytes look like, for error messages.
fn sniff(b: &[u8]) -> &'static str {
    let t = b.trim_ascii_start();
    if b.starts_with(b"OggS") {
        "Ogg"
    } else if b.starts_with(b"ID3") || (b.len() > 1 && b[0] == 0xFF && b[1] & 0xE0 == 0xE0) {
        "MP3"
    } else if b.starts_with(b"RIFF") {
        "RIFF/WAV"
    } else if b.starts_with(b"fLaC") {
        "FLAC"
    } else if b.len() > 8 && &b[4..8] == b"ftyp" {
        "MP4/M4A, which is not enabled"
    } else if t.starts_with(b"<") || t.starts_with(b"{") {
        "text/HTML/JSON, not audio (an error page from the asset fetch?)"
    } else {
        "unknown format"
    }
}

fn decoder(bytes: &Arc<Vec<u8>>) -> Result<Dec, String> {
    if bytes.is_empty() {
        return Err("cannot decode sound: the data is empty".into());
    }
    Decoder::builder()
        .with_data(Cursor::new(Bytes(bytes.clone())))
        .with_byte_len(bytes.len() as u64)
        .with_gapless(true)
        .build()
        .map_err(|e| {
            format!(
                "cannot decode sound ({} bytes, looks like {}): {e}",
                bytes.len(),
                sniff(bytes)
            )
        })
}

/// Length from the container headers, or by decoding and counting frames when the headers
/// don't say (e.g. MP3 without a Xing/Info frame).
fn length_of(dec: Dec) -> Option<f64> {
    if let Some(d) = dec.total_duration() {
        return Some(d.as_secs_f64());
    }
    let ch = dec.channels().get() as f64;
    let rate = dec.sample_rate().get() as f64;
    let n = dec.count();
    (n > 0).then(|| n as f64 / ch / rate)
}

/// Lazily computed length shared by all plays of the same byte buffer.
struct LenCell {
    secs: OnceLock<f64>,
    scanning: AtomicBool,
}

/// Sound.PlaybackRegion / LoopRegion in source seconds (with
/// PlaybackRegionsEnabled): the sound plays `play` (min..max) and, looped,
/// wraps over `looped` (or `play` when there is no loop region). Games keep
/// many clips in one long asset and pick one this way (Place A's SoundBank).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Region {
    pub play: Option<(f64, f64)>,
    pub looped: Option<(f64, f64)>,
}

impl Region {
    fn valid(r: Option<(f64, f64)>) -> Option<(f64, f64)> {
        r.filter(|(a, b)| a.is_finite() && b.is_finite() && *b > *a && *b > 0.0)
            .map(|(a, b)| (a.max(0.0), b))
    }
    pub fn play_range(&self) -> Option<(f64, f64)> {
        Self::valid(self.play)
    }
    /// Where a looped sound wraps: the loop region, else the playback region.
    pub fn loop_range(&self) -> Option<(f64, f64)> {
        Self::valid(self.looped).or_else(|| self.play_range())
    }
}

/// Puts `dec` at `t` seconds (exact seek, else by skipping samples); returns
/// the interleaved sample index it now stands at.
fn seek_to(dec: &mut Dec, t: f64) -> u64 {
    if t <= 0.0 {
        return 0;
    }
    let ch = dec.channels().get() as u64;
    let frames = (t * dec.sample_rate().get() as f64) as u64;
    if dec.try_seek(Duration::from_secs_f64(t)).is_err() {
        for _ in 0..frames * ch {
            if dec.next().is_none() {
                break;
            }
        }
    }
    frames * ch
}

/// Loop control shared with the audio thread.
struct Ctl {
    looped: AtomicBool,
    loops: AtomicU64,
}

/// Decoder wrapper whose looping can be switched while it plays (Sound.Looped
/// is mutable); it stops at the playback region's end and wraps over the loop
/// region.
struct LoopSource {
    inner: Dec,
    bytes: Arc<Vec<u8>>,
    ctl: Arc<Ctl>,
    region: Region,
    /// Interleaved samples from the start of the file.
    pos: u64,
    /// Samples per second (rate × channels) of this file.
    per_sec: f64,
}

impl LoopSource {
    fn wrap_or_end(&mut self) -> Option<rodio::Sample> {
        if !self.ctl.looped.load(Ordering::Relaxed) {
            return None;
        }
        // Rebuild instead of seeking back: symphonia's MP3/Ogg seeks are not always exact.
        let mut dec = decoder(&self.bytes).ok()?;
        let at = self.region.loop_range().map_or(0.0, |r| r.0);
        self.pos = seek_to(&mut dec, at);
        self.inner = dec;
        self.ctl.loops.fetch_add(1, Ordering::Relaxed);
        let s = self.inner.next();
        if s.is_some() {
            self.pos += 1;
        }
        s
    }
}

impl Iterator for LoopSource {
    type Item = rodio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        let end = if self.ctl.looped.load(Ordering::Relaxed) {
            self.region.loop_range()
        } else {
            self.region.play_range()
        }
        .map(|r| r.1);
        if end.is_some_and(|e| self.pos as f64 >= e * self.per_sec) {
            return self.wrap_or_end();
        }
        match self.inner.next() {
            Some(s) => {
                self.pos += 1;
                Some(s)
            }
            None => self.wrap_or_end(),
        }
    }
}

impl Source for LoopSource {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        None // may loop
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)
    }
}

enum Output {
    /// RUSTUDIO_NO_AUDIO or a test: silent forever.
    Disabled,
    Unopened,
    Opening {
        rx: Receiver<Result<Mixer, String>>,
        waited: bool,
    },
    Ready(Mixer),
    Failed(String),
}

struct Sound {
    bytes: Arc<Vec<u8>>,
    len: Arc<LenCell>,
    volume: f32,
    speed: f32,
    looped: bool,
    paused: bool,
    /// Source-seconds position at `anchor` (already wrapped when looped).
    base: f64,
    anchor: Instant,
    /// Wraps folded into `base` by `rebase`.
    wraps: u64,
    wraps_reported: u64,
    region: Region,
    player: Option<Player>,
    ctl: Option<Arc<Ctl>>,
}

impl Sound {
    fn length(&self) -> Option<f64> {
        self.len.secs.get().copied().filter(|l| *l > 0.0)
    }

    fn raw(&self, now: Instant) -> f64 {
        let rate = if self.paused { 0.0 } else { self.speed as f64 };
        self.base + now.saturating_duration_since(self.anchor).as_secs_f64() * rate
    }

    /// Where a non-looped sound ends: its playback region's end, else the file's.
    fn end(&self) -> Option<f64> {
        self.region
            .play_range()
            .map(|r| r.1)
            .or_else(|| self.length())
    }

    /// The span a looped sound wraps over.
    fn loop_span(&self) -> Option<(f64, f64)> {
        self.region
            .loop_range()
            .or_else(|| self.length().map(|l| (0.0, l)))
    }

    /// (wraps, position) for raw clock time `raw` of a looped sound.
    fn wrapped(&self, raw: f64) -> (u64, f64) {
        match self.loop_span() {
            Some((a, b)) if raw >= b => {
                let w = ((raw - a) / (b - a)).floor().max(1.0);
                (w as u64, raw - w * (b - a))
            }
            _ => (0, raw),
        }
    }

    /// Fold elapsed time into `base` so speed/pause/loop changes apply from `now` on.
    fn rebase(&mut self, now: Instant) {
        let raw = self.raw(now);
        self.base = if self.looped {
            let (w, p) = self.wrapped(raw);
            self.wraps += w;
            p
        } else {
            self.end().map_or(raw, |e| raw.min(e))
        };
        self.anchor = now;
    }

    fn position(&self, now: Instant) -> f64 {
        let raw = self.raw(now);
        if self.looped {
            self.wrapped(raw).1
        } else {
            self.end().map_or(raw, |e| raw.min(e))
        }
    }

    fn clock_wraps(&self, now: Instant) -> u64 {
        if self.looped {
            self.wraps + self.wrapped(self.raw(now)).0
        } else {
            self.wraps
        }
    }

    fn finished(&self, now: Instant) -> bool {
        if let Some(p) = &self.player {
            if p.empty() {
                return true; // the device drained it (also covers unknown lengths)
            }
            return !self.looped
                && self
                    .end()
                    .is_some_and(|e| self.raw(now) >= e + DEVICE_SLACK);
        }
        !self.looped && self.end().is_some_and(|e| self.raw(now) >= e)
    }

    fn apply(&self) {
        if let Some(p) = &self.player {
            p.set_volume(volume_gain(self.volume) * out_gain());
            p.set_speed(self.speed.max(0.01));
            if self.paused || self.speed <= 0.0 {
                p.pause();
            } else {
                p.play();
            }
        }
    }
}

/// View > Mute: every sound plays silently (timing goes on).
static MUTED: AtomicBool = AtomicBool::new(false);

pub fn muted() -> bool {
    MUTED.load(Ordering::Relaxed)
}

/// Mute before any sound exists (sounds read it as they start).
pub fn set_muted_global(on: bool) {
    MUTED.store(on, Ordering::Relaxed);
}

/// The in-game menu's Volume (UserGameSettings.MasterVolume, 0..1) as f32 bits.
static MASTER: AtomicU32 = AtomicU32::new(0x3f80_0000);

pub fn set_master_global(v: f32) {
    let v = if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) };
    MASTER.store(v.to_bits(), Ordering::Relaxed);
}

pub fn master_volume() -> f32 {
    f32::from_bits(MASTER.load(Ordering::Relaxed))
}

/// What every sound's own volume is multiplied by: 0 when muted, else the
/// master volume.
fn out_gain() -> f32 {
    if muted() {
        0.0
    } else {
        master_volume()
    }
}

pub struct Audio {
    sounds: HashMap<u64, Sound>,
    out: Output,
    /// Keeps the device thread (and its output stream) alive; dropped with `Audio`.
    hold: Option<Sender<()>>,
    /// Length cache keyed by the byte buffer's address (validated through the `Weak`).
    lengths: HashMap<usize, (Weak<Vec<u8>>, Arc<LenCell>)>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
    routes: HashMap<u64, (u64, Control)>,
    buses: HashMap<u64, (Mixer, Control)>,
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.hold.take();
        if let Some(t) = self.capture_thread.take() {
            let _ = t.join();
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Audio {
    /// Never blocks or panics: the device is opened on the first `play()`.
    pub fn new() -> Audio {
        let off = std::env::var_os("RUSTUDIO_NO_AUDIO").is_some_and(|v| !v.is_empty() && v != "0");
        let mut a = Audio::silent();
        if let Some(path) = std::env::var_os("RUSTUDIO_AUDIO_CAPTURE") {
            let (m, source) = rodio::mixer::mixer(
                ChannelCount::new(2).unwrap(),
                SampleRate::new(dsp::RATE).unwrap(),
            );
            let (tx, rx) = mpsc::channel();
            a.hold = Some(tx);
            a.out = Output::Ready(m);
            a.capture_thread = Some(std::thread::spawn(move || {
                if let Err(e) = crate::audio_capture::run(path.into(), source, rx) {
                    eprintln!("audio capture: {e}");
                }
            }));
        } else if !off {
            a.out = Output::Unopened;
        }
        a
    }

    /// An engine that never touches an output device (state and timing still work).
    pub fn silent() -> Audio {
        Audio {
            sounds: HashMap::new(),
            out: Output::Disabled,
            hold: None,
            lengths: HashMap::new(),
            capture_thread: None,
            routes: HashMap::new(),
            buses: HashMap::new(),
        }
    }

    #[cfg(test)]
    fn with_mixer(mixer: Mixer) -> Audio {
        let mut a = Audio::silent();
        a.out = Output::Ready(mixer);
        a
    }

    /// False when audio is disabled or the device failed to open. Before the first `play()`
    /// the device has not been tried yet, so this is optimistic.
    pub fn available(&self) -> bool {
        !matches!(self.out, Output::Disabled | Output::Failed(_))
    }

    /// Why output is unavailable, if it failed to open.
    pub fn output_error(&self) -> Option<&str> {
        match &self.out {
            Output::Failed(e) => Some(e),
            _ => None,
        }
    }

    fn spawn_open(&mut self) -> Output {
        let (tx, rx) = mpsc::channel();
        let (hold_tx, hold_rx) = mpsc::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("rustudio-audio".into())
            .spawn(move || {
                match rodio::DeviceSinkBuilder::open_default_sink() {
                    Ok(mut sink) => {
                        sink.log_on_drop(false);
                        if tx.send(Ok(sink.mixer().clone())).is_ok() {
                            let _ = hold_rx.recv(); // returns when `Audio` is dropped
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("no audio output device: {e}")));
                    }
                }
            });
        match spawned {
            Ok(_) => {
                self.hold = Some(hold_tx);
                Output::Opening { rx, waited: false }
            }
            Err(e) => Output::Failed(format!("cannot start audio thread: {e}")),
        }
    }

    /// The device mixer if it is (or becomes) ready; `None` means time the sound silently.
    fn mixer(&mut self) -> Option<Mixer> {
        loop {
            let next = match &mut self.out {
                Output::Disabled | Output::Failed(_) => return None,
                Output::Ready(m) => return Some(m.clone()),
                Output::Unopened => self.spawn_open(),
                Output::Opening { rx, waited } => {
                    let got = if *waited {
                        match rx.try_recv() {
                            Ok(r) => r,
                            Err(TryRecvError::Empty) => return None,
                            Err(TryRecvError::Disconnected) => Err("audio thread exited".into()),
                        }
                    } else {
                        *waited = true;
                        match rx.recv_timeout(OPEN_WAIT) {
                            Ok(r) => r,
                            Err(RecvTimeoutError::Timeout) => return None,
                            Err(RecvTimeoutError::Disconnected) => {
                                Err("audio thread exited".into())
                            }
                        }
                    };
                    match got {
                        Ok(m) => Output::Ready(m),
                        Err(e) => {
                            eprintln!("rustudio: {e}; sounds will be silent");
                            self.hold = None;
                            Output::Failed(e)
                        }
                    }
                }
            };
            self.out = next;
        }
    }

    fn length_cell(&mut self, bytes: &Arc<Vec<u8>>) -> Arc<LenCell> {
        let ptr = Arc::as_ptr(bytes) as usize;
        if let Some((w, cell)) = self.lengths.get(&ptr) {
            if w.upgrade().is_some_and(|b| Arc::ptr_eq(&b, bytes)) {
                return cell.clone();
            }
        }
        if self.lengths.len() >= 256 {
            self.lengths.retain(|_, (w, _)| w.strong_count() > 0);
        }
        let cell = Arc::new(LenCell {
            secs: OnceLock::new(),
            scanning: AtomicBool::new(false),
        });
        self.lengths
            .insert(ptr, (Arc::downgrade(bytes), cell.clone()));
        cell
    }

    /// Start (or restart) sound `key` from `bytes` at `start` seconds.
    pub fn play(
        &mut self,
        key: u64,
        bytes: Arc<Vec<u8>>,
        volume: f32,
        speed: f32,
        looped: bool,
        start: f64,
        region: Region,
    ) -> Result<(), String> {
        self.sounds.remove(&key);
        let start = if start.is_finite() {
            start.max(0.0)
        } else {
            0.0
        };
        // With a playback region the sound starts at its beginning unless
        // TimePosition already lies inside it.
        let start = match region.play_range() {
            Some((a, b)) if !(start >= a && start < b) => a,
            _ => start,
        };
        let speed = sane_speed(speed);
        let mut dec = decoder(&bytes)?;
        let len = self.length_cell(&bytes);
        if let Some(d) = dec.total_duration() {
            let _ = len.secs.set(d.as_secs_f64());
        }

        let (player, ctl) = match self.mixer() {
            None => {
                if len.secs.get().is_none() {
                    // Silent timing needs the length now: count the decoded frames.
                    match length_of(dec) {
                        Some(l) => {
                            let _ = len.secs.set(l);
                        }
                        None => {
                            return Err(format!(
                                "cannot decode sound ({} bytes, looks like {}): no audio frames",
                                bytes.len(),
                                sniff(&bytes)
                            ))
                        }
                    }
                }
                (None, None)
            }
            Some(mixer) => {
                if len.secs.get().is_none() && !len.scanning.swap(true, Ordering::AcqRel) {
                    let (cell, data) = (len.clone(), bytes.clone());
                    let _ = std::thread::Builder::new()
                        .name("rustudio-audio-len".into())
                        .spawn(move || {
                            if let Some(l) = decoder(&data).ok().and_then(length_of) {
                                let _ = cell.secs.set(l);
                            }
                        });
                }
                let per_sec = dec.sample_rate().get() as f64 * dec.channels().get() as f64;
                let pos = seek_to(&mut dec, start);
                let ctl = Arc::new(Ctl {
                    looped: AtomicBool::new(looped),
                    loops: AtomicU64::new(0),
                });
                let (bus, control) = self
                    .routes
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| (0, Arc::new(Mutex::new(Settings::default()))));
                let target = self.buses.get(&bus).map(|b| &b.0).unwrap_or(&mixer);
                let (player, output) = Player::new();
                player.set_volume(volume_gain(volume) * out_gain());
                player.set_speed(speed.max(0.01));
                if speed <= 0.0 {
                    player.pause();
                }
                let source = LoopSource {
                    inner: dec,
                    bytes: bytes.clone(),
                    ctl: ctl.clone(),
                    region,
                    pos,
                    per_sec,
                };
                let source = rodio::source::UniformSourceIterator::new(
                    source,
                    ChannelCount::new(2).unwrap(),
                    SampleRate::new(dsp::RATE).unwrap(),
                );
                player.append(Process::new(source, control, false).tagged(key));
                target.add(output);
                (Some(player), Some(ctl))
            }
        };

        let sound = Sound {
            bytes,
            len,
            volume,
            speed,
            looped,
            paused: false,
            base: start,
            anchor: Instant::now(),
            wraps: 0,
            wraps_reported: 0,
            region,
            player,
            ctl,
        };
        sound.apply();
        self.sounds.insert(key, sound);
        Ok(())
    }

    /// Configure buses root-first, then the sound; shared buses process the sum once.
    pub fn configure(&mut self, key: u64, groups: Vec<(u64, Settings)>, settings: Settings) {
        if !self.routes.contains_key(&key) {
            if let Some(path) = std::env::var_os("RUSTUDIO_AUDIO_RMS") {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(f, "graph key={key} groups={groups:?} sound={settings:?}");
                }
            }
        }
        let mut parent = self.mixer();
        let mut bus = 0;
        for (id, settings) in groups {
            if !self.buses.contains_key(&id) {
                if let Some(target) = &parent {
                    let (m, source) = rodio::mixer::mixer(
                        ChannelCount::new(2).unwrap(),
                        SampleRate::new(dsp::RATE).unwrap(),
                    );
                    let ctl = Arc::new(Mutex::new(settings.clone()));
                    target.add(Process::new(source, ctl.clone(), true));
                    self.buses.insert(id, (m, ctl));
                }
            }
            if let Some((m, c)) = self.buses.get(&id) {
                *c.lock().unwrap() = settings;
                parent = Some(m.clone());
                bus = id;
            }
        }
        let previous_bus = self.routes.get(&key).map(|e| e.0);
        let entry = self
            .routes
            .entry(key)
            .or_insert_with(|| (bus, Arc::new(Mutex::new(Settings::default()))));
        entry.0 = bus;
        *entry.1.lock().unwrap() = settings;
        if previous_bus.is_some_and(|previous| previous != bus) {
            if let Some((bytes, volume, speed, looped, position, region, paused)) =
                self.sounds.get(&key).map(|s| {
                    (
                        s.bytes.clone(),
                        s.volume,
                        s.speed,
                        s.looped,
                        s.position(Instant::now()),
                        s.region,
                        s.paused,
                    )
                })
            {
                let _ = self.play(key, bytes, volume, speed, looped, position, region);
                if paused {
                    self.pause(key);
                }
            }
        }
    }

    pub fn stop(&mut self, key: u64) {
        self.routes.remove(&key);
        self.sounds.remove(&key); // dropping the Player stops it
    }

    /// View > Mute: silences every sound now and later (they keep time).
    /// The menu's Volume: every sound, playing or not, follows it at once.
    pub fn set_master(&mut self, v: f32) {
        set_master_global(v);
        for s in self.sounds.values() {
            s.apply();
        }
    }

    pub fn set_muted(&mut self, on: bool) {
        MUTED.store(on, Ordering::Relaxed);
        for s in self.sounds.values() {
            s.apply();
        }
    }

    pub fn pause(&mut self, key: u64) {
        if let Some(s) = self.sounds.get_mut(&key) {
            if !s.paused {
                s.rebase(Instant::now());
                s.paused = true;
                s.apply();
            }
        }
    }

    /// Resume a paused sound. Unknown keys are ignored (use `play`).
    pub fn resume(&mut self, key: u64) {
        if let Some(s) = self.sounds.get_mut(&key) {
            if s.paused {
                s.rebase(Instant::now());
                s.paused = false;
                s.apply();
            }
        }
    }

    /// Roblox Sound.Volume (0..10); see [`volume_gain`] for the gain mapping.
    pub fn set_volume(&mut self, key: u64, v: f32) {
        if let Some(s) = self.sounds.get_mut(&key) {
            s.volume = v;
            s.apply();
        }
    }

    /// Roblox Sound.PlaybackSpeed (also shifts pitch, like Roblox). 0 holds the sound in place.
    pub fn set_speed(&mut self, key: u64, sp: f32) {
        if let Some(s) = self.sounds.get_mut(&key) {
            s.rebase(Instant::now());
            s.speed = sane_speed(sp);
            s.apply();
        }
    }

    pub fn set_looped(&mut self, key: u64, l: bool) {
        if let Some(s) = self.sounds.get_mut(&key) {
            s.rebase(Instant::now());
            s.looped = l;
            if let Some(c) = &s.ctl {
                c.looped.store(l, Ordering::Relaxed);
            }
        }
    }

    pub fn is_playing(&self, key: u64) -> bool {
        let now = Instant::now();
        self.sounds
            .get(&key)
            .is_some_and(|s| !s.paused && !s.finished(now))
    }

    /// Seconds into the sound (Sound.TimePosition); 0 for stopped/unknown/finished-and-polled.
    pub fn position(&self, key: u64) -> f64 {
        self.sounds
            .get(&key)
            .map_or(0.0, |s| s.position(Instant::now()))
    }

    /// Sound.TimeLength: from the container headers, or by decoding when they don't say.
    pub fn length(bytes: &[u8]) -> Option<f64> {
        decoder(&Arc::new(bytes.to_vec())).ok().and_then(length_of)
    }

    /// Keys of sounds that reached their end (not looped) since the last call, for
    /// Sound.Ended. Finished sounds are dropped, so they read as stopped afterwards.
    pub fn poll_finished(&mut self) -> Vec<u64> {
        let now = Instant::now();
        let mut done: Vec<u64> = self
            .sounds
            .iter()
            .filter(|(_, s)| s.finished(now))
            .map(|(k, _)| *k)
            .collect();
        done.sort_unstable();
        for k in &done {
            self.sounds.remove(k);
            self.routes.remove(k);
        }
        done
    }

    /// Keys of looped sounds that wrapped around since the last call (Sound.DidLoop), best
    /// effort: the decoder's wrap count with a device, the clock in silent mode.
    pub fn looped_since_poll(&mut self) -> Vec<u64> {
        let now = Instant::now();
        let mut out = Vec::new();
        for (k, s) in self.sounds.iter_mut() {
            let total = match &s.ctl {
                Some(c) => c.loops.load(Ordering::Relaxed),
                None => s.clock_wraps(now),
            };
            if total > s.wraps_reported {
                s.wraps_reported = total;
                out.push(*k);
            }
        }
        out.sort_unstable();
        out
    }

    /// How many sounds are playing now (measurement).
    pub fn playing_count(&self) -> usize {
        let now = Instant::now();
        self.sounds
            .values()
            .filter(|s| !s.paused && !s.finished(now))
            .count()
    }

    pub fn stop_all(&mut self) {
        self.sounds.clear();
        self.routes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    /// 16-bit mono PCM WAV of `secs` seconds (a quiet square wave).
    fn wav(secs: f64, rate: u32) -> Arc<Vec<u8>> {
        let n = (secs * rate as f64).round() as u32;
        let data_len = n * 2;
        let mut b = Vec::with_capacity(44 + data_len as usize);
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&1u16.to_le_bytes()); // mono
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
        b.extend_from_slice(&2u16.to_le_bytes()); // block align
        b.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..n {
            let v: i16 = if (i / 20) % 2 == 0 { 3000 } else { -3000 };
            b.extend_from_slice(&v.to_le_bytes());
        }
        Arc::new(b)
    }

    #[test]
    fn audio_length_of_wav() {
        let l = Audio::length(&wav(0.5, 8000)).expect("length");
        assert!((l - 0.5).abs() < 0.01, "{l}");
        assert_eq!(Audio::length(b"not audio at all, just text"), None);
        assert_eq!(Audio::length(&[]), None);
    }

    #[test]
    fn audio_rejects_garbage() {
        let mut a = Audio::silent();
        let err = a.play(
            1,
            Arc::new(b"<html>403 Forbidden</html>".to_vec()),
            0.5,
            1.0,
            false,
            0.0,
            Region::default(),
        );
        let err = err.unwrap_err();
        assert!(err.contains("cannot decode"), "{err}");
        assert!(err.contains("HTML"), "{err}");
        assert!(a
            .play(
                1,
                Arc::new(Vec::new()),
                0.5,
                1.0,
                false,
                0.0,
                Region::default()
            )
            .is_err());
        assert!(!a.is_playing(1));
    }

    #[test]
    fn audio_env_disables_output() {
        std::env::set_var("RUSTUDIO_NO_AUDIO", "1");
        let a = Audio::new();
        assert!(!a.available());
        assert!(!Audio::silent().available());
    }

    #[test]
    fn audio_silent_play_pause_resume_finish() {
        let mut a = Audio::silent();
        a.play(7, wav(0.3, 8000), 0.5, 1.0, false, 0.0, Region::default())
            .unwrap();
        assert!(a.is_playing(7));
        sleep(Duration::from_millis(60));
        a.pause(7);
        let p = a.position(7);
        assert!(p > 0.04 && p < 0.3, "{p}");
        assert!(!a.is_playing(7));
        sleep(Duration::from_millis(400)); // well past the length, but paused
        assert_eq!(a.position(7), p);
        assert!(a.poll_finished().is_empty());
        a.resume(7);
        assert!(a.is_playing(7));
        sleep(Duration::from_millis(350));
        assert!(!a.is_playing(7));
        assert!((a.position(7) - 0.3).abs() < 0.01);
        assert_eq!(a.poll_finished(), vec![7]);
        assert!(a.poll_finished().is_empty()); // reported once
        assert_eq!(a.position(7), 0.0);
    }

    #[test]
    fn audio_silent_start_speed_stop() {
        let mut a = Audio::silent();
        // Starts 0.25 s into a 0.3 s sound at double speed: ends after ~25 ms.
        a.play(1, wav(0.3, 8000), 1.0, 2.0, false, 0.25, Region::default())
            .unwrap();
        assert!(a.position(1) >= 0.25);
        a.play(2, wav(1.0, 8000), 1.0, 1.0, false, 0.0, Region::default())
            .unwrap();
        sleep(Duration::from_millis(80));
        assert_eq!(a.poll_finished(), vec![1]);
        a.stop(2);
        assert!(!a.is_playing(2));
        assert_eq!(a.position(2), 0.0);
        assert!(a.poll_finished().is_empty());
        // Restart keeps one entry per key.
        a.play(3, wav(1.0, 8000), 1.0, 1.0, false, 0.0, Region::default())
            .unwrap();
        a.play(3, wav(1.0, 8000), 1.0, 1.0, false, 0.5, Region::default())
            .unwrap();
        assert!(a.position(3) >= 0.5 && a.position(3) < 0.9);
        a.stop_all();
        assert!(!a.is_playing(3));
    }

    #[test]
    fn audio_silent_loop_and_unloop() {
        let mut a = Audio::silent();
        a.play(5, wav(0.1, 8000), 0.5, 1.0, true, 0.0, Region::default())
            .unwrap();
        sleep(Duration::from_millis(250));
        assert!(a.is_playing(5));
        assert!(a.position(5) < 0.1);
        assert_eq!(a.looped_since_poll(), vec![5]);
        assert!(a.looped_since_poll().is_empty());
        assert!(a.poll_finished().is_empty());
        a.set_looped(5, false);
        sleep(Duration::from_millis(150));
        assert_eq!(a.poll_finished(), vec![5]);
    }

    #[test]
    fn audio_volume_mapping() {
        assert_eq!(volume_gain(0.0), 0.0);
        assert_eq!(volume_gain(-1.0), 0.0);
        assert_eq!(volume_gain(f32::NAN), 0.0);
        assert_eq!(volume_gain(0.5), 0.5);
        assert_eq!(volume_gain(1.0), 1.0);
        assert_eq!(volume_gain(10.0), 10.0);
        assert_eq!(volume_gain(50.0), 50.0);
    }

    /// Drives the real rodio path with an in-memory mixer instead of a sound card.
    #[test]
    fn audio_nested_bus_gain_and_live_pan() {
        let (m, mut out) = rodio::mixer::mixer(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(dsp::RATE).unwrap(),
        );
        let mut a = Audio::with_mixer(m);
        let groups = vec![
            (
                10,
                Settings {
                    gain: 0.5,
                    ..Default::default()
                },
            ),
            (
                11,
                Settings {
                    gain: 0.25,
                    ..Default::default()
                },
            ),
        ];
        a.configure(
            1,
            groups.clone(),
            Settings {
                pan: Some(1.),
                ..Default::default()
            },
        );
        a.play(1, wav(1., dsp::RATE), 1., 1., true, 0., Region::default())
            .unwrap();
        let mut peak = 0f32;
        for _ in 0..2000 {
            let l = out.next().unwrap();
            let r = out.next().unwrap();
            assert!(l.abs() < 1e-6, "left={l} right={r}");
            peak = peak.max(r.abs());
        }
        assert!(peak > 0.001 && peak < 0.126, "{peak}");
        a.configure(
            1,
            groups,
            Settings {
                pan: Some(-1.),
                ..Default::default()
            },
        );
        for _ in 0..1000 {
            out.next();
        }
        let mut left = 0f32;
        for _ in 0..1000 {
            left = left.max(out.next().unwrap().abs());
            assert!(out.next().unwrap().abs() < 1e-6);
        }
        assert!(left > 0.001);
    }

    #[test]
    fn audio_device_path_with_test_mixer() {
        let rate = 8000u32;
        let (mixer, mut out) = rodio::mixer::mixer(
            ChannelCount::new(1).unwrap(),
            SampleRate::new(rate).unwrap(),
        );
        let mut a = Audio::with_mixer(mixer);
        assert!(a.available());
        a.play(1, wav(0.05, rate), 0.5, 1.0, false, 0.0, Region::default())
            .unwrap();
        a.play(2, wav(0.05, rate), 0.5, 1.0, true, 0.0, Region::default())
            .unwrap();
        assert!(a.is_playing(1) && a.is_playing(2));
        let mut loud = 0usize;
        for _ in 0..(rate as usize / 4) {
            if out.next().is_some_and(|s| s.abs() > 0.01) {
                loud += 1;
            }
        }
        assert!(loud > 0, "mixer produced only silence");
        // Sound 1 drained from the device even though its clock has barely moved.
        assert_eq!(a.poll_finished(), vec![1]);
        assert!(a.is_playing(2));
        assert_eq!(a.looped_since_poll(), vec![2]);
        a.stop(2);
        assert!(a.poll_finished().is_empty());
    }

    /// A clip picked out of a sheet by PlaybackRegion ends at the region's
    /// end (Place A's SoundBank), not at the end of the file.
    #[test]
    fn audio_playback_region_ends_at_region_end() {
        let mut a = Audio::silent();
        let r = Region {
            play: Some((0.5, 0.6)),
            looped: None,
        };
        a.play(9, wav(2.0, 8000), 0.5, 1.0, false, 0.0, r).unwrap();
        assert!(
            (a.position(9) - 0.5).abs() < 0.02,
            "starts at the region: {}",
            a.position(9)
        );
        sleep(Duration::from_millis(150));
        assert!(!a.is_playing(9), "still playing past the region end");
        assert_eq!(a.poll_finished(), vec![9]);
        assert!(a.poll_finished().is_empty(), "Ended reported twice");
    }

    /// Looped with a LoopRegion it wraps over that span, once per pass.
    #[test]
    fn audio_loop_region_wraps_inside_region() {
        let mut a = Audio::silent();
        let r = Region {
            play: Some((0.5, 0.6)),
            looped: Some((0.5, 0.6)),
        };
        a.play(4, wav(2.0, 8000), 0.5, 1.0, true, 0.0, r).unwrap();
        sleep(Duration::from_millis(250));
        let p = a.position(4);
        assert!((0.5..0.6).contains(&p), "position {p} left the loop region");
        let w = a.looped_since_poll();
        assert_eq!(w, vec![4]);
        assert!(a.is_playing(4));
        assert!(a.poll_finished().is_empty());
    }

    /// The decoder side: a region yields exactly its samples; looped, it
    /// wraps back to the loop start and counts each wrap.
    #[test]
    fn audio_loop_source_respects_regions() {
        let bytes = wav(2.0, 8000);
        let make = |looped: bool, region: Region| {
            let mut dec = decoder(&bytes).unwrap();
            let per_sec = dec.sample_rate().get() as f64 * dec.channels().get() as f64;
            let start = region.play_range().map_or(0.0, |r| r.0);
            let pos = seek_to(&mut dec, start);
            let ctl = Arc::new(Ctl {
                looped: AtomicBool::new(looped),
                loops: AtomicU64::new(0),
            });
            (
                LoopSource {
                    inner: dec,
                    bytes: bytes.clone(),
                    ctl: ctl.clone(),
                    region,
                    pos,
                    per_sec,
                },
                ctl,
                per_sec,
            )
        };
        let r = Region {
            play: Some((0.5, 0.6)),
            looped: None,
        };
        let (src, _, per_sec) = make(false, r);
        let n = src.count() as f64;
        assert!(
            (n - 0.1 * per_sec).abs() <= 2.0,
            "{n} samples for a 0.1 s region"
        );
        let (src, ctl, per_sec) = make(true, r);
        let n = src.take((0.35 * per_sec) as usize).count();
        assert_eq!(n, (0.35 * per_sec) as usize);
        assert_eq!(
            ctl.loops.load(Ordering::Relaxed),
            3,
            "3 wraps in 0.35 s of a 0.1 s loop"
        );
    }
}

/// Decoded audio for the Audio* graph (media domain): every channel at the
/// file's own rate, f32.
pub struct Pcm {
    pub rate: u32,
    pub chans: Vec<Vec<f32>>,
}

impl Pcm {
    pub fn len_secs(&self) -> f64 {
        self.chans.first().map_or(0.0, |c| c.len() as f64 / self.rate.max(1) as f64)
    }
}

/// Decodes a whole asset (Ogg / MP3 / WAV / FLAC) to planar f32.
pub fn decode_pcm(bytes: &Arc<Vec<u8>>) -> Result<Pcm, String> {
    let dec = decoder(bytes)?;
    let ch = dec.channels().get() as usize;
    let rate = dec.sample_rate().get();
    let mut chans = vec![Vec::new(); ch.max(1)];
    for (i, s) in dec.enumerate() {
        chans[i % ch.max(1)].push(s);
    }
    let n = chans.iter().map(|c| c.len()).min().unwrap_or(0);
    for c in chans.iter_mut() {
        c.truncate(n);
    }
    Ok(Pcm { rate, chans })
}

/// AudioDeviceOutput: interleaved stereo 48 kHz blocks the Audio* graph
/// renders, played on the default device. Opened on first use from a helper
/// thread; never opened with RUSTUDIO_NO_AUDIO (tests stay silent). Muted /
/// master volume apply as for Sounds.
pub fn graph_output(stereo: &[f32]) {
    use std::collections::VecDeque;
    static QUEUE: OnceLock<Option<Arc<Mutex<VecDeque<f32>>>>> = OnceLock::new();
    let q = QUEUE.get_or_init(|| {
        let off = std::env::var_os("RUSTUDIO_NO_AUDIO").is_some_and(|v| !v.is_empty() && v != "0");
        if off {
            return None;
        }
        let q = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        let src = GraphSource { q: q.clone() };
        let ok = std::thread::Builder::new()
            .name("rustudio-graph-audio".into())
            .spawn(move || {
                if let Ok(mut sink) = rodio::DeviceSinkBuilder::open_default_sink() {
                    sink.log_on_drop(false);
                    sink.mixer().add(src);
                    // The sink lives as long as the process.
                    loop {
                        std::thread::park();
                    }
                }
            })
            .is_ok();
        ok.then_some(q)
    });
    let Some(q) = q else { return };
    if let Ok(mut q) = q.lock() {
        // Keep at most ~0.25 s queued: a stalled device must not grow it.
        let max = 48000 / 2;
        let g = out_gain();
        q.extend(stereo.iter().map(|s| s * g));
        while q.len() > max {
            q.pop_front();
        }
    }
}

struct GraphSource {
    q: Arc<Mutex<std::collections::VecDeque<f32>>>,
}

impl Iterator for GraphSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        Some(self.q.lock().ok().and_then(|mut q| q.pop_front()).unwrap_or(0.0))
    }
}

impl Source for GraphSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(48000).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
