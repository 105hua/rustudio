//! Engine API domain: media. See src/domains/mod.rs for the hook contract.
//!
//! The Audio* API as a real processing graph: Wires connect pins
//! (GetInputPins / GetOutputPins / GetConnectedWires, Wire.Connected,
//! WiringChanged), AudioPlayers decode their asset and play it, effects
//! process it (media_dsp.rs), AudioEmitters are heard by AudioListeners
//! through their distance / angle curves and panning, AudioAnalyzers measure
//! what reaches them (RmsLevel, PeakLevel, GetSpectrum) and
//! AudioDeviceOutput plays it (audio::graph_output; silent under
//! RUSTUDIO_NO_AUDIO). The graph renders once per frame at 48 kHz.
//!
//! Studio's rules, measured in an offline Studio (checks/parity/media.md):
//! - A wire's validity is decided when its SourceInstance / TargetInstance /
//!   SourceName / TargetName are set: unknown pin, a loop (target already
//!   reaches source, or source == target) and a duplicate of a connected
//!   wire are refused, with an error in Output only when the wire is in the
//!   DataModel at that moment. Parenting doesn't re-check, so wires built
//!   outside the DataModel (or loaded from a file) all connect.
//! - Connected = valid and wire, source and target all in the DataModel.
//! - WiringChanged(connected, pin, wire, other) fires on the source, then on
//!   the target, for every change of Connected.
//! - AudioPlayer: Play() with no asset still sets IsPlaying; the asset loads
//!   (IsReady false → true, TimeLength) and plays then. At the end IsPlaying
//!   turns false without a Changed signal, Ended fires and TimePosition goes
//!   back to the start the next frame; Looping fires Looped per wrap.
//!   Stop() keeps TimePosition.
//!
//! Where Studio has no backend offline (text-to-speech, speech-to-text,
//! recording, voice, video capture), rustudio does what Studio does then:
//! see media_misc.rs.
#![allow(unused_imports, unused_variables)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use mlua::{Lua, MultiValue, Table, Value};
use rbx_dom_weak::types::{Content, Enum, Ref, Variant, VariantType};
use rbx_dom_weak::ustr;

use crate::app::Level;
use crate::audio::Pcm;
use crate::doc::Document;
use crate::math::Xform;
use crate::runtime::*;

#[path = "media_dsp.rs"]
pub mod dsp;
#[path = "media_misc.rs"]
mod misc;
#[path = "media_video.rs"]
pub mod video;

use dsp::Buf;

// ------------------------------------------------------------------ tables

/// Classes with pins.
pub const WIRABLE: [&str; 27] = [
    "AudioPlayer", "AudioEmitter", "AudioListener", "AudioAnalyzer", "AudioRecorder",
    "AudioFader", "AudioDeviceInput", "AudioDeviceOutput", "AudioChannelMixer",
    "AudioChannelSplitter", "AudioChorus", "AudioCompressor", "AudioDistortion", "AudioEcho",
    "AudioEqualizer", "AudioFilter", "AudioFlanger", "AudioGate", "AudioLimiter",
    "AudioPitchShifter", "AudioReverb", "AudioTremolo", "AudioWindSynthesizer",
    "AudioSpeechToText", "AudioTextToSpeech", "VideoPlayer", "VideoDisplay",
];

const MIXER_IN: [&str; 13] = [
    "BackLeft", "BackRight", "Center", "Input", "Left", "Right", "Sub", "SurroundLeft",
    "SurroundRight", "TopBackLeft", "TopBackRight", "TopLeft", "TopRight",
];
const SPLITTER_OUT: [&str; 13] = [
    "BackLeft", "BackRight", "Center", "Left", "Output", "Right", "Sub", "SurroundLeft",
    "SurroundRight", "TopBackLeft", "TopBackRight", "TopLeft", "TopRight",
];

/// (input pins, output pins), as Studio lists them (sorted).
pub fn pins(class: &str) -> (&'static [&'static str], &'static [&'static str]) {
    const IN: &[&str] = &["Input"];
    const OUT: &[&str] = &["Output"];
    const NONE: &[&str] = &[];
    match class {
        "AudioPlayer" | "AudioListener" | "AudioDeviceInput" | "AudioWindSynthesizer"
        | "AudioTextToSpeech" | "VideoPlayer" => (NONE, OUT),
        "AudioEmitter" | "AudioAnalyzer" | "AudioRecorder" | "AudioDeviceOutput"
        | "AudioSpeechToText" | "VideoDisplay" => (IN, NONE),
        "AudioChannelMixer" => (&MIXER_IN, OUT),
        "AudioChannelSplitter" => (IN, &SPLITTER_OUT),
        "AudioCompressor" => (&["Input", "Sidechain"], OUT),
        c if WIRABLE.contains(&c) => (IN, OUT),
        _ => (NONE, NONE),
    }
}

/// Studio's ranges (setter clamps), measured by writing ±1e9.
const CLAMPS: &[(&str, &str, f32, f32)] = &[
    ("AudioCompressor", "Attack", 0.0001, 0.5),
    ("AudioCompressor", "MakeupGain", -30.0, 30.0),
    ("AudioCompressor", "Ratio", 1.0, 50.0),
    ("AudioCompressor", "Release", 0.01, 5.0),
    ("AudioCompressor", "Threshold", -60.0, 0.0),
    ("AudioFlanger", "Depth", 0.01, 1.0),
    ("AudioFlanger", "Mix", 0.0, 1.0),
    ("AudioFlanger", "Rate", 0.0, 20.0),
    ("AudioChorus", "Depth", 0.0, 1.0),
    ("AudioChorus", "Mix", 0.0, 1.0),
    ("AudioChorus", "Rate", 0.0, 20.0),
    ("AudioWindSynthesizer", "Volume", 0.0, 3.0),
    ("AudioReverb", "DecayRatio", 0.1, 1.0),
    ("AudioReverb", "DecayTime", 0.1, 20.0),
    ("AudioReverb", "Density", 0.1, 1.0),
    ("AudioReverb", "Diffusion", 0.1, 1.0),
    ("AudioReverb", "DryLevel", -80.0, 20.0),
    ("AudioReverb", "EarlyDelayTime", 0.0, 0.3),
    ("AudioReverb", "HighCutFrequency", 20.0, 20000.0),
    ("AudioReverb", "LateDelayTime", 0.0, 0.1),
    ("AudioReverb", "LowShelfFrequency", 20.0, 20000.0),
    ("AudioReverb", "LowShelfGain", -36.0, 12.0),
    ("AudioReverb", "ReferenceFrequency", 20.0, 20000.0),
    ("AudioReverb", "WetLevel", -80.0, 20.0),
    ("AudioPitchShifter", "Pitch", 0.5, 2.0),
    ("AudioEqualizer", "HighGain", -80.0, 10.0),
    ("AudioEqualizer", "LowGain", -80.0, 10.0),
    ("AudioEqualizer", "MidGain", -80.0, 10.0),
    ("AudioEcho", "DelayTime", 0.001, 5.0),
    ("AudioEcho", "DryLevel", -80.0, 10.0),
    ("AudioEcho", "Feedback", 0.0, 1.0),
    ("AudioEcho", "RampTime", 0.0, 60000.0),
    ("AudioEcho", "WetLevel", -80.0, 10.0),
    ("AudioGate", "Attack", 0.001, 5.0),
    ("AudioGate", "Release", 0.001, 5.0),
    ("AudioTremolo", "Depth", 0.0, 1.0),
    ("AudioTremolo", "Duty", 0.0, 1.0),
    ("AudioTremolo", "Frequency", 0.0, 20.0),
    ("AudioTremolo", "Shape", 0.0, 1.0),
    ("AudioTremolo", "Skew", -1.0, 1.0),
    ("AudioTremolo", "Square", 0.0, 1.0),
    ("AudioFader", "Volume", 0.0, 3.0),
    ("AudioLimiter", "MaxLevel", -12.0, 0.0),
    ("AudioLimiter", "Release", 0.001, 1.0),
    ("AudioTextToSpeech", "Pitch", -12.0, 12.0),
    ("AudioTextToSpeech", "PlaybackSpeed", 0.0, 20.0),
    ("AudioTextToSpeech", "Speed", 0.5, 2.0),
    ("AudioTextToSpeech", "Volume", 0.0, 3.0),
    ("AudioDistortion", "Level", 0.0, 1.0),
    ("AudioPlayer", "Volume", 0.0, 10.0),
    ("AudioPlayer", "PlaybackSpeed", 0.0, 20.0),
    ("AudioFilter", "Frequency", 20.0, 22000.0),
    ("AudioFilter", "Gain", -30.0, 30.0),
    ("AudioFilter", "Q", 0.1, 10.0),
    ("AudioDeviceInput", "Volume", 0.0, 3.0),
];

/// NumberRange setters: (class, property, lowest, highest).
const RANGE_CLAMPS: &[(&str, &str, f32, f32)] = &[
    ("AudioGate", "Threshold", -80.0, 30.0),
    ("AudioEqualizer", "MidRange", 200.0, 20000.0),
];

/// Studio's defaults, for parameters the reflection database may lack.
const DEFAULTS: &[(&str, &str, f32)] = &[
    ("AudioChorus", "Depth", 0.45), ("AudioChorus", "Mix", 0.85), ("AudioChorus", "Rate", 5.0),
    ("AudioFlanger", "Depth", 0.45), ("AudioFlanger", "Mix", 0.85), ("AudioFlanger", "Rate", 5.0),
    ("AudioCompressor", "Attack", 0.1), ("AudioCompressor", "MakeupGain", 0.0),
    ("AudioCompressor", "Ratio", 40.0), ("AudioCompressor", "Release", 0.1),
    ("AudioCompressor", "Threshold", -40.0),
    ("AudioDistortion", "Level", 0.5),
    ("AudioEcho", "DelayTime", 1.0), ("AudioEcho", "DryLevel", 0.0), ("AudioEcho", "Feedback", 0.5),
    ("AudioEcho", "RampTime", 0.0), ("AudioEcho", "WetLevel", 0.0),
    ("AudioEqualizer", "HighGain", 0.0), ("AudioEqualizer", "LowGain", 0.0), ("AudioEqualizer", "MidGain", 0.0),
    ("AudioFilter", "Frequency", 2000.0), ("AudioFilter", "Gain", 0.0), ("AudioFilter", "Q", 0.707),
    ("AudioGate", "Attack", 0.01), ("AudioGate", "Release", 0.1),
    ("AudioLimiter", "MaxLevel", 0.0), ("AudioLimiter", "Release", 0.01),
    ("AudioPitchShifter", "Pitch", 1.25),
    ("AudioReverb", "DecayRatio", 0.5), ("AudioReverb", "DecayTime", 1.5), ("AudioReverb", "Density", 1.0),
    ("AudioReverb", "Diffusion", 1.0), ("AudioReverb", "DryLevel", 0.0), ("AudioReverb", "EarlyDelayTime", 0.02),
    ("AudioReverb", "HighCutFrequency", 20000.0), ("AudioReverb", "LateDelayTime", 0.04),
    ("AudioReverb", "LowShelfFrequency", 250.0), ("AudioReverb", "LowShelfGain", 0.0),
    ("AudioReverb", "ReferenceFrequency", 5000.0), ("AudioReverb", "WetLevel", -6.0),
    ("AudioTremolo", "Depth", 1.0), ("AudioTremolo", "Duty", 0.5), ("AudioTremolo", "Frequency", 5.0),
    ("AudioTremolo", "Shape", 0.0), ("AudioTremolo", "Skew", 0.0), ("AudioTremolo", "Square", 0.0),
    ("AudioFader", "Volume", 1.0),
    ("AudioWindSynthesizer", "Volume", 1.0),
    ("AudioPlayer", "Volume", 1.0), ("AudioPlayer", "PlaybackSpeed", 1.0),
    ("AudioDeviceInput", "Volume", 1.0),
];

fn default_f(class: &str, k: &str) -> f32 {
    DEFAULTS.iter().find(|(c, p, _)| *c == class && *p == k).map_or(0.0, |x| x.2)
}

// --------------------------------------------------------- property access

fn pf(d: &Document, r: Ref, k: &str, def: f32) -> f32 {
    match d.prop(r, k) {
        Some(Variant::Float32(f)) => f,
        Some(Variant::Float64(f)) => f as f32,
        Some(Variant::Int32(i)) => i as f32,
        Some(Variant::Int64(i)) => i as f32,
        _ => def,
    }
}
fn pd(d: &Document, r: Ref, k: &str, def: f64) -> f64 {
    match d.prop(r, k) {
        Some(Variant::Float32(f)) => f as f64,
        Some(Variant::Float64(f)) => f,
        Some(Variant::Int32(i)) => i as f64,
        _ => def,
    }
}
fn pb(d: &Document, r: Ref, k: &str, def: bool) -> bool {
    match d.prop(r, k) {
        Some(Variant::Bool(b)) => b,
        _ => def,
    }
}
fn pe(d: &Document, r: Ref, k: &str, def: u32) -> u32 {
    match d.prop(r, k) {
        Some(Variant::Enum(e)) => e.to_u32(),
        _ => def,
    }
}
fn prange(d: &Document, r: Ref, k: &str, def: (f32, f32)) -> (f32, f32) {
    match d.prop(r, k) {
        Some(Variant::NumberRange(n)) => (n.min, n.max),
        _ => def,
    }
}
fn pref(d: &Document, r: Ref, k: &str) -> Option<Ref> {
    match d.prop(r, k) {
        Some(Variant::Ref(x)) if !x.is_none() && d.exists(x) => Some(x),
        _ => None,
    }
}
fn pstr(d: &Document, r: Ref, k: &str) -> String {
    match d.prop(r, k) {
        Some(Variant::String(s)) => s,
        Some(Variant::ContentId(c)) => c.as_str().to_string(),
        Some(Variant::Content(c)) => c.as_uri().unwrap_or("").to_string(),
        _ => String::new(),
    }
}
fn class_of(d: &Document, r: Ref) -> String {
    d.class_of(r).map(|c| c.to_string()).unwrap_or_default()
}
fn name_of(d: &Document, r: Ref) -> String {
    d.dom.get_by_ref(r).map(|i| i.name.clone()).unwrap_or_default()
}

/// In the DataModel (a descendant of the root).
fn in_dm(d: &Document, r: Ref) -> bool {
    let root = d.dom.root_ref();
    let mut cur = r;
    for _ in 0..4096 {
        if cur == root {
            return true;
        }
        match d.dom.get_by_ref(cur) {
            Some(i) => cur = i.parent(),
            None => return false,
        }
        if cur.is_none() {
            return false;
        }
    }
    false
}

/// Effect parameters read straight from the instance.
struct NodeParams<'a> {
    d: &'a Document,
    r: Ref,
    class: &'a str,
}

impl dsp::Params for NodeParams<'_> {
    fn f(&self, k: &str) -> f32 {
        pf(self.d, self.r, k, default_f(self.class, k))
    }
    fn b(&self, k: &str) -> bool {
        pb(self.d, self.r, k, false)
    }
    fn range(&self, k: &str) -> (f32, f32) {
        let def = match (self.class, k) {
            ("AudioEqualizer", "MidRange") => (400.0, 4000.0),
            ("AudioGate", "Threshold") => (-36.0, -24.0),
            _ => (0.0, 0.0),
        };
        prange(self.d, self.r, k, def)
    }
    fn e(&self, k: &str) -> u32 {
        let def = match k {
            "WindowSize" => 1,
            _ => 0,
        };
        pe(self.d, self.r, k, def)
    }
}

// ------------------------------------------------------------------- state

#[derive(Clone, Default)]
struct WireRec {
    valid: bool,
    connected: bool,
    /// (source, source pin, target, target pin) while connected.
    ends: Option<(Ref, String, Ref, String)>,
    /// The four properties when validity was last decided: a change made
    /// elsewhere (replication) is re-validated quietly.
    seen: Option<(Option<Ref>, String, Option<Ref>, String)>,
}

enum Load {
    Fetch(u64),
    Decode(std::sync::mpsc::Receiver<Result<Arc<Pcm>, String>>),
}

#[derive(Default)]
struct PlayerState {
    asset: String,
    pcm: Option<Arc<Pcm>>,
    loading: Option<Load>,
    pos: f64,
    playing: bool,
    reset_next: bool,
    failed: bool,
}

#[derive(Default)]
struct AnalyzerState {
    ring: Vec<Vec<f32>>,
    rms: f32,
    peak: f32,
    spectrum: Vec<f32>,
    active: bool,
}

struct Action {
    id: i64,
    target: Ref,
    play: bool,
    at: f64,
}

pub struct Media {
    wires: HashMap<Ref, WireRec>,
    wire_order: Vec<Ref>,
    /// Wirable instances in the DataModel at the last scan, by class.
    nodes: Vec<(Ref, String)>,
    scanned_at: f64,
    /// Wires and wirable instances, kept from the document's change journal.
    index: Option<crate::doc::ClassIndex>,
    players: HashMap<Ref, PlayerState>,
    fx: HashMap<Ref, dsp::Fx>,
    wind: HashMap<Ref, dsp::Wind>,
    analyzers: HashMap<Ref, AnalyzerState>,
    access: HashMap<Ref, Vec<i64>>,
    mixer_time: f64,
    carry: f64,
    actions: Vec<Action>,
    next_action: i64,
    /// Sounds whose SoundId is loading for Sound.Loaded / TimeLength.
    sound_loads: HashMap<Ref, (String, Option<Load>)>,
    pub misc: misc::MiscState,
}

impl Default for Media {
    fn default() -> Self {
        Media {
            wires: HashMap::new(),
            wire_order: Vec::new(),
            nodes: Vec::new(),
            scanned_at: f64::NEG_INFINITY,
            index: None,
            players: HashMap::new(),
            fx: HashMap::new(),
            wind: HashMap::new(),
            analyzers: HashMap::new(),
            access: HashMap::new(),
            mixer_time: 0.0,
            carry: 0.0,
            actions: Vec::new(),
            next_action: 1,
            sound_loads: HashMap::new(),
            misc: Default::default(),
        }
    }
}

fn media(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, Media>> {
    lua.app_data_mut::<Media>().ok_or_else(|| rt_err("media state missing"))
}

// ------------------------------------------------------------ decoded audio

static PCM: Mutex<Option<HashMap<String, Arc<Pcm>>>> = Mutex::new(None);

fn pcm_cached(id: &str) -> Option<Arc<Pcm>> {
    if !audio_allowed(id) {
        return None;
    }
    PCM.lock().ok()?.as_ref()?.get(id).cloned()
}

fn pcm_store(id: &str, p: Arc<Pcm>) {
    if let Ok(mut g) = PCM.lock() {
        g.get_or_insert_with(HashMap::new).insert(id.to_string(), p);
    }
}

enum Got {
    Ready(Arc<Pcm>),
    Pending(Load),
    Failed(String),
}

fn decode(id: &str, bytes: Arc<Vec<u8>>) -> Got {
    // Small files decode at once (a cached asset is ready in the same frame,
    // as in Studio); long ones on a thread.
    if bytes.len() < 1 << 21 {
        return match crate::audio::decode_pcm(&bytes) {
            Ok(p) => {
                let p = Arc::new(p);
                pcm_store(id, p.clone());
                Got::Ready(p)
            }
            Err(e) => Got::Failed(e),
        };
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let key = id.to_string();
    std::thread::spawn(move || {
        let r = crate::audio::decode_pcm(&bytes).map(Arc::new);
        if let Ok(p) = &r {
            pcm_store(&key, p.clone());
        }
        let _ = tx.send(r);
    });
    Got::Pending(Load::Decode(rx))
}

/// Audio is script-readable (GetWaveformAsync, AudioAnalyzer), and a
/// runtime means scripts run: Roblox content only, unless
/// RUSTUDIO_ALLOW_LOCAL_CONTENT=1 (assets::place_content_allowed). Asked
/// before the PCM cache so cached samples never bypass it (#16).
fn audio_allowed(id: &str) -> bool {
    crate::assets::place_content_allowed(id, true)
}

fn start_load(id: &str) -> Got {
    if !audio_allowed(id) {
        return Got::Failed("Invalid Asset identifier or not found".into());
    }
    if let Some(p) = pcm_cached(id) {
        return Got::Ready(p);
    }
    if let Some(path) = crate::assets::resolve(id, None) {
        return match std::fs::read(&path) {
            Ok(b) => decode(id, Arc::new(b)),
            Err(e) => Got::Failed(format!("{}: {e}", path.display())),
        };
    }
    match crate::assets::asset_ref(id) {
        Some(a) => poll_fetch(id, a.id),
        None => Got::Failed("Invalid Asset identifier or not found".into()),
    }
}

fn poll_fetch(id: &str, asset: u64) -> Got {
    match crate::assets::public_prefetch(asset) {
        crate::assets::Fetch::Ready(b) => decode(id, b),
        crate::assets::Fetch::Pending => Got::Pending(Load::Fetch(asset)),
        crate::assets::Fetch::Unavailable(e) => Got::Failed(e),
    }
}

fn poll_load(id: &str, l: Load) -> Got {
    match l {
        Load::Fetch(a) => poll_fetch(id, a),
        Load::Decode(rx) => match rx.try_recv() {
            Ok(Ok(p)) => Got::Ready(p),
            Ok(Err(e)) => Got::Failed(e),
            Err(std::sync::mpsc::TryRecvError::Empty) => Got::Pending(Load::Decode(rx)),
            Err(_) => Got::Failed("decoder stopped".into()),
        },
    }
}

// ------------------------------------------------------------------ hooks

const fn concat<const N: usize>(parts: &[&[(&'static str, &'static str)]]) -> [(&'static str, &'static str); N] {
    let mut out = [("", ""); N];
    let mut k = 0;
    let mut p = 0;
    while p < parts.len() {
        let mut i = 0;
        while i < parts[p].len() {
            out[k] = parts[p][i];
            k += 1;
            i += 1;
        }
        p += 1;
    }
    out
}

const fn per_wirable<const N: usize>(names: &[&'static str]) -> [(&'static str, &'static str); N] {
    let mut out = [("", ""); N];
    let mut i = 0;
    while i < WIRABLE.len() {
        let mut j = 0;
        while j < names.len() {
            out[i * names.len() + j] = (names[j], WIRABLE[i]);
            j += 1;
        }
        i += 1;
    }
    out
}

const WIRE_FNS: [(&str, &str); 81] = per_wirable(&["GetConnectedWires", "GetInputPins", "GetOutputPins"]);

const OWN_FNS: &[(&str, &str)] = &[
    ("Play", "AudioPlayer"),
    ("Stop", "AudioPlayer"),
    ("Cancel", "AudioPlayer"),
    ("GetWaveformAsync", "AudioPlayer"),
    ("GetAngleAttenuation", "AudioEmitter"),
    ("SetAngleAttenuation", "AudioEmitter"),
    ("GetDistanceAttenuation", "AudioEmitter"),
    ("SetDistanceAttenuation", "AudioEmitter"),
    ("GetAudibilityFor", "AudioEmitter"),
    ("GetInteractingListeners", "AudioEmitter"),
    ("GetAngleAttenuation", "AudioListener"),
    ("SetAngleAttenuation", "AudioListener"),
    ("GetDistanceAttenuation", "AudioListener"),
    ("SetDistanceAttenuation", "AudioListener"),
    ("GetAudibilityFor", "AudioListener"),
    ("GetInteractingEmitters", "AudioListener"),
    ("GetSpectrum", "AudioAnalyzer"),
    ("GetGainAt", "AudioFilter"),
    ("GetUserIdAccessList", "AudioDeviceInput"),
    ("SetUserIdAccessList", "AudioDeviceInput"),
    ("GetMixerTime", "SoundService"),
    ("SetInputDevice", "SoundService"),
];

const N_METHODS: usize = WIRE_FNS.len() + OWN_FNS.len() + misc::METHODS.len();
const METHODS_ARR: [(&str, &str); N_METHODS] = concat(&[&WIRE_FNS, OWN_FNS, misc::METHODS]);
pub const METHODS: &[(&str, &str)] = &METHODS_ARR;

const WIRING_EVENTS: [(&str, &str); 27] = per_wirable(&["WiringChanged"]);
const PLAYER_EVENTS: &[(&str, &str)] = &[("Ended", "AudioPlayer"), ("Looped", "AudioPlayer")];
const N_EVENTS: usize = WIRING_EVENTS.len() + PLAYER_EVENTS.len() + misc::EVENTS.len();
const EVENTS_ARR: [(&str, &str); N_EVENTS] = concat(&[&WIRING_EVENTS, PLAYER_EVENTS, misc::EVENTS]);
pub const EVENTS: &[(&str, &str)] = &EVENTS_ARR;

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let class = world(lua).ok().map(|w| class_of(w.d(), this))?;
    if let Some(r) = misc::call(lua, name, this, &class, args) {
        return Some(r);
    }
    let wirable = WIRABLE.contains(&class.as_str());
    let r = match name {
        "GetInputPins" | "GetOutputPins" if wirable => pin_list(lua, this, &class, name == "GetInputPins"),
        "GetConnectedWires" if wirable => connected_wires(lua, this, args),
        "Play" | "Stop" if class == "AudioPlayer" => player_play(lua, this, name == "Play", args),
        "Cancel" if class == "AudioPlayer" => cancel(lua, args),
        "GetWaveformAsync" if class == "AudioPlayer" => waveform(lua, this, args),
        "GetDistanceAttenuation" | "GetAngleAttenuation"
            if matches!(class.as_str(), "AudioEmitter" | "AudioListener") =>
        {
            get_curve(lua, this, name == "GetAngleAttenuation")
        }
        "SetDistanceAttenuation" | "SetAngleAttenuation"
            if matches!(class.as_str(), "AudioEmitter" | "AudioListener") =>
        {
            set_curve(lua, this, &class, name, args)
        }
        "GetAudibilityFor" if matches!(class.as_str(), "AudioEmitter" | "AudioListener") => {
            audibility_call(lua, this, &class, args)
        }
        "GetInteractingListeners" if class == "AudioEmitter" => interacting(lua, this, true),
        "GetInteractingEmitters" if class == "AudioListener" => interacting(lua, this, false),
        "GetSpectrum" if class == "AudioAnalyzer" => get_spectrum(lua, this),
        "GetGainAt" if class == "AudioFilter" => gain_at(lua, this, args),
        "GetUserIdAccessList" if class == "AudioDeviceInput" => get_access(lua, this),
        "SetUserIdAccessList" if class == "AudioDeviceInput" => set_access(lua, this, args),
        "GetMixerTime" if class == "SoundService" => {
            media(lua).map(|m| one(Value::Number(m.mixer_time)))
        }
        // Studio (2026-11, game and command bar alike).
        "SetInputDevice" if class == "SoundService" => Err(rt_err(
            "The current thread cannot call 'SetInputDevice' (lacking capability InternalTest)",
        )),
        _ => return None,
    };
    Some(r)
}

fn one(v: Value) -> MultiValue {
    let mut m = MultiValue::new();
    m.push_back(v);
    m
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if let Some(v) = misc::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    Ok(match (class, key) {
        ("Wire", "Connected") => {
            refresh(lua, false)?;
            Some(Value::Boolean(connected_now(lua, r)?))
        }
        ("AudioPlayer", "IsPlaying" | "IsReady" | "TimeLength" | "TimePosition") => {
            ensure_player(lua, r)?;
            let m = media(lua)?;
            let p = m.players.get(&r);
            Some(match key {
                "IsPlaying" => Value::Boolean(p.is_some_and(|p| p.playing)),
                "IsReady" => Value::Boolean(p.is_some_and(|p| p.pcm.is_some())),
                "TimeLength" => Value::Number(p.and_then(|p| p.pcm.as_ref()).map_or(0.0, |x| x.len_secs())),
                _ => Value::Number(p.map_or(0.0, |p| p.pos)),
            })
        }
        ("AudioAnalyzer", "RmsLevel" | "PeakLevel") => {
            let m = media(lua)?;
            let a = m.analyzers.get(&r);
            Some(Value::Number(a.map_or(0.0, |a| if key == "RmsLevel" { a.rms } else { a.peak }) as f64))
        }
        ("Sound", "MaxDistance") => {
            let w = world(lua)?;
            Some(Value::Number(pf(w.d(), r, "RollOffMaxDistance", 10000.0) as f64))
        }
        ("Sound", "isPlaying") => {
            let w = world(lua)?;
            Some(Value::Boolean(w.input.sound_playing.contains(&r)))
        }
        ("Sound", "PlaybackLoudness") => Some(Value::Number(sound_loudness(lua, r)?)),
        ("Sound", "TimeLength") => {
            let known = world(lua)?.input.sound_len.get(&r).copied();
            match known {
                Some(l) => Some(Value::Number(l)),
                None => {
                    let id = pstr(world(lua)?.d(), r, "SoundId");
                    Some(Value::Number(pcm_cached(&id).map_or(0.0, |p| p.len_secs())))
                }
            }
        }
        ("AudioSearchParams", "MaxPageSize") => {
            let w = world(lua)?;
            Some(Value::Integer(match w.d().prop(r, "MaxPageSize") {
                Some(Variant::Int32(i)) => i as i64,
                _ => 0,
            }))
        }
        ("AudioSearchParams", "SortType") => {
            let v = world(lua)?.d().prop(r, "SortType");
            let item = match v {
                Some(Variant::Enum(e)) => crate::ltypes::LEnumItem::from_value("AudioSortType", e.to_u32()),
                _ => None,
            }
            .or_else(|| crate::ltypes::LEnumItem::from_name("AudioSortType", "Relevance"));
            match item {
                Some(i) => Some(crate::ltypes::enum_value(lua, i)?),
                None => None,
            }
        }
        _ => None,
    })
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if misc::newindex(lua, r, class, key, value)? {
        return Ok(true);
    }
    // Studio's clamps.
    if let Some((_, _, lo, hi)) = CLAMPS.iter().find(|(c, p, _, _)| *c == class && *p == key) {
        if let Some(n) = crate::ltypes::num(value) {
            let c = (n as f32).clamp(*lo, *hi);
            let Some((canon, dt, ro)) = prop_desc(class, key) else { return Ok(false) };
            if ro {
                return Ok(false);
            }
            let ty = match dt {
                rbx_reflection::DataType::Value(t) => t,
                _ => return Ok(false),
            };
            let v = if ty == VariantType::Float64 { Variant::Float64(c as f64) } else { Variant::Float32(c) };
            store_prop(lua, r, canon, v)?;
            return Ok(true);
        }
    }
    if let Some((_, _, lo, hi)) = RANGE_CLAMPS.iter().find(|(c, p, _, _)| *c == class && *p == key) {
        if let Some(nr) = opt_ud::<crate::ltypes::LNumberRange>(value) {
            let (a, b) = (nr.min.clamp(*lo, *hi), nr.max.clamp(*lo, *hi));
            store_prop(lua, r, key, Variant::NumberRange(rbx_dom_weak::types::NumberRange::new(a, b.max(a))))?;
            return Ok(true);
        }
    }
    match (class, key) {
        ("Wire", "SourceInstance" | "TargetInstance" | "SourceName" | "TargetName") => {
            set_wire_prop(lua, r, key, value)?;
            Ok(true)
        }
        ("AudioPlayer", "Asset" | "AudioContent" | "AssetId") => {
            ensure_player(lua, r)?;
            // Store first (normal path), then load: the hook stores it itself.
            let Some((canon, dt, _)) = prop_desc(class, key) else { return Ok(false) };
            let existing = world(lua)?.d().prop(r, canon);
            let (ty, en) = match &dt {
                rbx_reflection::DataType::Value(t) => (*t, None),
                _ => return Ok(false),
            };
            let v = lua_to_variant(key, value, ty, en, existing.as_ref())?;
            store_prop(lua, r, canon, v)?;
            asset_changed(lua, r)?;
            Ok(true)
        }
        ("AudioPlayer", "IsPlaying") => {
            ensure_player(lua, r)?;
            let on = matches!(value, Value::Boolean(true));
            set_playing(lua, r, on)?;
            Ok(true)
        }
        ("AudioPlayer", "TimePosition") => {
            ensure_player(lua, r)?;
            if let Some(n) = crate::ltypes::num(value) {
                if let Some(p) = media(lua)?.players.get_mut(&r) {
                    p.pos = n.max(0.0);
                    p.reset_next = false;
                }
            }
            Ok(false)
        }
        ("Sound", "MaxDistance") => {
            let Some(n) = crate::ltypes::num(value) else {
                return Err(rt_err(format!(
                    "Unable to assign property MaxDistance. Number expected, got {}",
                    value.type_name()
                )));
            };
            store_prop(lua, r, "RollOffMaxDistance", Variant::Float32(n as f32))?;
            Ok(true)
        }
        ("Sound", "SoundId") => {
            // Loaded fires once the new id's audio is in (step).
            let id = match value {
                Value::String(s) => s.to_str()?.to_string(),
                _ => String::new(),
            };
            media(lua)?.sound_loads.insert(r, (id, None));
            Ok(false)
        }
        _ => Ok(false),
    }
}

pub fn write_acts(class: &str, key: &str) -> bool {
    matches!(
        (class, key),
        ("Wire", "SourceInstance" | "TargetInstance" | "SourceName" | "TargetName")
            | ("AudioPlayer", "Asset" | "AudioContent" | "IsPlaying" | "TimePosition")
            | ("Sound", "SoundId" | "MaxDistance")
    ) || CLAMPS.iter().any(|(c, p, _, _)| *c == class && *p == key)
        || misc::write_acts(class, key)
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(Media::default());
    // *Async members yield on the task scheduler as Studio's do: they run,
    // then the calling thread waits a frame before getting the result.
    for name in ASYNC {
        let n = name.to_string();
        let raw = lua.create_function(move |lua, args: MultiValue| {
            crate::bench::api_timed("call", &n, || checked(lua, call_method(lua, &n, args)))
        })?;
        let f = crate::cmeta::method(lua, name, Some(raw), None, None)?;
        let maker: mlua::Function = lua
            .load(
                "local f = ... return function(...) local r = table.pack(f(...)) task.wait() return table.unpack(r, 1, r.n) end",
            )
            .into_function()?;
        let y: mlua::Function = maker.call(f)?;
        if let Some(mut c) = lua.app_data_mut::<Cache>() {
            c.overrides.insert(name, y);
        }
    }
    Ok(())
}

const ASYNC: &[&str] = &[
    "GetSamplesAtTimesAsync",
    "GetWaveformAsync",
    "CanRecordAsync",
    "GetUnrecordableInstancesAsync",
    "RecordAsync",
    "LoadAsync",
    "IsVoiceEnabledForUserIdAsync",
    "GetChatGroupsAsync",
    "CreateVideoSamplerAsync",
    "SetMockInputAsync",
];

// ------------------------------------------------------------------ wiring

fn pin_list(lua: &Lua, r: Ref, class: &str, input: bool) -> mlua::Result<MultiValue> {
    let (i, o) = pins(class);
    let t = lua.create_sequence_from(if input { i } else { o }.iter().copied())?;
    Ok(one(Value::Table(t)))
}

fn set_wire_prop(lua: &Lua, r: Ref, key: &str, value: &Value) -> mlua::Result<()> {
    let v = match key {
        "SourceInstance" | "TargetInstance" => match value {
            Value::Nil => Variant::Ref(Ref::none()),
            v => match opt_inst(v) {
                Some(x) => Variant::Ref(x),
                None => {
                    return Err(rt_err(format!(
                        "Unable to assign property {key}. Object expected, got {}",
                        value.type_name()
                    )))
                }
            },
        },
        _ => match value {
            Value::String(s) => Variant::String(s.to_str()?.to_string()),
            Value::Number(n) => Variant::String(n.to_string()),
            Value::Integer(n) => Variant::String(n.to_string()),
            v => {
                return Err(rt_err(format!(
                    "Unable to assign property {key}. string expected, got {}",
                    v.type_name()
                )))
            }
        },
    };
    let old = world(lua)?.d().prop(r, key);
    let same = match (&old, &v) {
        (Some(Variant::Ref(a)), Variant::Ref(b)) => a == b,
        (None, Variant::Ref(b)) => b.is_none(),
        (Some(Variant::String(a)), Variant::String(b)) => a == b,
        _ => false,
    };
    store_prop(lua, r, key, v)?;
    if !same {
        validate(lua, r, true)?;
        sync_wire(lua, r)?;
    }
    Ok(())
}

fn wire_ends(d: &Document, w: Ref) -> (Option<Ref>, String, Option<Ref>, String) {
    let name = |k: &str, def: &str| match d.prop(w, k) {
        Some(Variant::String(s)) => s,
        _ => def.to_string(),
    };
    (pref(d, w, "SourceInstance"), name("SourceName", "Output"), pref(d, w, "TargetInstance"), name("TargetName", "Input"))
}

/// Decides whether a wire may connect (Studio checks this when one of its
/// four properties is set). `loud`: the checks against the connected graph
/// run and errors print (a property write); quiet = discovered wire.
fn validate(lua: &Lua, wire: Ref, loud: bool) -> mlua::Result<()> {
    let (msg, valid) = {
        let w = world(lua)?;
        let d = w.d();
        let (s, sn, t, tn) = wire_ends(d, wire);
        let wire_in = in_dm(d, wire);
        match (s, t) {
            (Some(s), Some(t)) => {
                let (sc, tc) = (class_of(d, s), class_of(d, t));
                let (_, souts) = pins(&sc);
                let (tins, _) = pins(&tc);
                if !WIRABLE.contains(&sc.as_str()) || !WIRABLE.contains(&tc.as_str()) {
                    (None, false)
                } else if !souts.contains(&sn.as_str()) {
                    (Some(format!("Invalid Wire Connection: \"{}\" has no \"{sn}\" output pin.", name_of(d, s))), false)
                } else if !tins.contains(&tn.as_str()) {
                    (Some(format!("Invalid Wire Connection: \"{}\" has no \"{tn}\" input pin.", name_of(d, t))), false)
                } else if !loud {
                    (None, true)
                } else if s == t || reaches(lua, d, t, s)? {
                    (
                        Some(format!(
                            "Invalid Wire Connection: Connecting \"{}\" to \"{}\" would create a loop.",
                            name_of(d, s),
                            name_of(d, t)
                        )),
                        false,
                    )
                } else if duplicate(lua, d, wire, s, &sn, t, &tn)? {
                    (
                        Some(format!(
                            "Invalid Wire Connection: \"{}\"'s \"{sn}\" pin is already connected to \"{}\"'s \"{tn}\" pin.",
                            name_of(d, s),
                            name_of(d, t)
                        )),
                        false,
                    )
                } else {
                    (None, true)
                }
            }
            _ => (None, false),
        }
        .map_msg(wire_in && loud)
    };
    if let Some(m) = msg {
        world(lua)?.output.push((Level::Error, m));
    }
    let seen = wire_ends(world(lua)?.d(), wire);
    let mut m = media(lua)?;
    if !m.wires.contains_key(&wire) {
        m.wire_order.push(wire);
    }
    let rec = m.wires.entry(wire).or_default();
    rec.valid = valid;
    rec.seen = Some(seen);
    Ok(())
}

trait MapMsg {
    fn map_msg(self, show: bool) -> (Option<String>, bool);
}
impl MapMsg for (Option<String>, bool) {
    fn map_msg(self, show: bool) -> (Option<String>, bool) {
        (self.0.filter(|_| show), self.1)
    }
}

/// Connected wires leaving `from` (any pin), judged now (a wire parented
/// this frame counts).
fn out_wires(lua: &Lua, d: &Document, from: Ref) -> mlua::Result<Vec<Ref>> {
    let m = media(lua)?;
    Ok(m.wire_order
        .iter()
        .filter(|w| {
            m.wires.get(w).is_some_and(|r| r.valid)
                && pref(d, **w, "SourceInstance") == Some(from)
                && in_dm(d, **w)
                && in_dm(d, from)
                && pref(d, **w, "TargetInstance").is_some_and(|t| in_dm(d, t))
        })
        .copied()
        .collect())
}

/// Whether `to` is reachable from `from` over connected wires.
fn reaches(lua: &Lua, d: &Document, from: Ref, to: Ref) -> mlua::Result<bool> {
    let mut seen = HashSet::new();
    let mut stack = vec![from];
    while let Some(n) = stack.pop() {
        if n == to {
            return Ok(true);
        }
        if !seen.insert(n) {
            continue;
        }
        for w in out_wires(lua, d, n)? {
            if let Some(t) = pref(d, w, "TargetInstance") {
                stack.push(t);
            }
        }
    }
    Ok(false)
}

fn duplicate(lua: &Lua, d: &Document, wire: Ref, s: Ref, sn: &str, t: Ref, tn: &str) -> mlua::Result<bool> {
    for w in out_wires(lua, d, s)? {
        if w == wire {
            continue;
        }
        let (_, wsn, wt, wtn) = wire_ends(d, w);
        if wt == Some(t) && wsn == sn && wtn == tn {
            return Ok(true);
        }
    }
    Ok(false)
}

fn connected_now(lua: &Lua, wire: Ref) -> mlua::Result<bool> {
    let valid = media(lua)?.wires.get(&wire).is_some_and(|w| w.valid);
    if !valid {
        return Ok(false);
    }
    let w = world(lua)?;
    let d = w.d();
    let (s, _, t, _) = wire_ends(d, wire);
    Ok(match (s, t) {
        (Some(s), Some(t)) => in_dm(d, wire) && in_dm(d, s) && in_dm(d, t),
        _ => false,
    })
}

/// Brings a wire's Connected up to date, firing WiringChanged on a change.
fn sync_wire(lua: &Lua, wire: Ref) -> mlua::Result<()> {
    let now = connected_now(lua, wire)?;
    let ends = {
        let w = world(lua)?;
        let (s, sn, t, tn) = wire_ends(w.d(), wire);
        s.zip(t).map(|(s, t)| (s, sn, t, tn))
    };
    let fire = {
        let mut m = media(lua)?;
        let rec = m.wires.entry(wire).or_default();
        if rec.connected == now {
            None
        } else {
            rec.connected = now;
            let e = if now { ends.clone() } else { rec.ends.clone() };
            rec.ends = if now { ends } else { None };
            e
        }
    };
    if let Some((s, sn, t, tn)) = fire {
        let wv = inst_value(lua, wire)?;
        let (sv, tv) = (inst_value(lua, s)?, inst_value(lua, t)?);
        let (snv, tnv) = (Value::String(lua.create_string(&sn)?), Value::String(lua.create_string(&tn)?));
        let mut w = world(lua)?;
        w.fire(s, "WiringChanged", vec![Value::Boolean(now), snv, wv.clone(), tv]);
        w.fire(t, "WiringChanged", vec![Value::Boolean(now), tnv, wv, sv]);
    }
    Ok(())
}

/// Finds wires and audio nodes in the DataModel (at most once per frame
/// unless `force`): wires made by Clone or loaded from a file are only seen
/// this way.
fn refresh(lua: &Lua, force: bool) -> mlua::Result<()> {
    let now = world(lua)?.time;
    {
        let m = media(lua)?;
        if !force && m.scanned_at == now {
            return Ok(());
        }
    }
    // Only what changed in the DataModel since the last scan is visited
    // (doc::ClassIndex): walking every instance each time cost tens of ms a
    // scan in big places, on the server and the client at once.
    let mut index = media(lua)?
        .index
        .take()
        .unwrap_or_else(|| crate::doc::ClassIndex::new(crate::doc::Reader::Media));
    let (wires, nodes): (Vec<Ref>, Vec<(Ref, String)>) = {
        let mut w = world(lua)?;
        let d = w.dm();
        index.update(d, |c| c == "Wire" || WIRABLE.contains(&c) || matches!(c, "Sound" | "VideoFrame"));
        let mut wires = Vec::new();
        let mut nodes = Vec::new();
        for r in index.list() {
            let Some(i) = d.dom.get_by_ref(*r) else { continue };
            let c = i.class.as_str();
            if c == "Wire" {
                wires.push(*r);
            } else {
                nodes.push((*r, c.to_string()));
            }
        }
        (wires, nodes)
    };
    let fresh: Vec<Ref> = {
        let mut m = media(lua)?;
        m.index = Some(index);
        m.scanned_at = now;
        m.nodes = nodes;
        wires.iter().filter(|w| !m.wires.contains_key(w)).copied().collect()
    };
    for w in fresh {
        validate(lua, w, false)?;
    }
    Ok(())
}

/// The instance on the far end of the first connected wire into `pin`.
fn wired_source(lua: &Lua, node: Ref, pin: &str) -> mlua::Result<Option<Ref>> {
    let order = media(lua)?.wire_order.clone();
    for w in order {
        if !connected_now(lua, w)? {
            continue;
        }
        let wd = world(lua)?;
        let (s, _, t, tn) = wire_ends(wd.d(), w);
        if t == Some(node) && tn == pin {
            return Ok(s);
        }
    }
    Ok(None)
}

fn connected_wires(lua: &Lua, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let pin = match arg(args, 1) {
        Value::String(s) => s.to_str()?.to_string(),
        Value::Number(n) => n.to_string(),
        v => {
            return Err(rt_err(format!(
                "Argument 1 missing or nil"
            )))
        }
    };
    refresh(lua, false)?;
    let order = media(lua)?.wire_order.clone();
    let mut out = Vec::new();
    for w in order {
        if !connected_now(lua, w)? {
            continue;
        }
        let wd = world(lua)?;
        let (s, sn, t, tn) = wire_ends(wd.d(), w);
        if (s == Some(this) && sn == pin) || (t == Some(this) && tn == pin) {
            out.push(w);
        }
    }
    Ok(one(Value::Table(list(lua, out)?)))
}

// ----------------------------------------------------------------- players

fn ensure_player(lua: &Lua, r: Ref) -> mlua::Result<()> {
    let has = media(lua)?.players.contains_key(&r);
    if !has {
        media(lua)?.players.insert(r, PlayerState::default());
        // A player from a file or a Clone: its asset loads now.
        asset_changed(lua, r)?;
    }
    Ok(())
}

fn asset_changed(lua: &Lua, r: Ref) -> mlua::Result<()> {
    let id = {
        let w = world(lua)?;
        let d = w.d();
        let a = pstr(d, r, "Asset");
        if a.is_empty() { pstr(d, r, "AudioContent") } else { a }
    };
    let changed = {
        let mut m = media(lua)?;
        let p = m.players.entry(r).or_default();
        if p.asset == id && (p.pcm.is_some() || p.loading.is_some() || p.failed) {
            false
        } else {
            let had = p.pcm.is_some();
            p.asset = id.clone();
            p.pcm = None;
            p.loading = None;
            p.failed = false;
            had
        }
    };
    if changed {
        // Studio: IsReady false, TimeLength 0 (Changed both).
        store_prop(lua, r, "IsReady", Variant::Bool(false))?;
        store_prop(lua, r, "TimeLength", Variant::Float64(0.0))?;
    }
    if id.trim().is_empty() {
        return Ok(());
    }
    let got = start_load(&id);
    finish_load(lua, r, got)
}

fn finish_load(lua: &Lua, r: Ref, got: Got) -> mlua::Result<()> {
    match got {
        Got::Ready(pcm) => {
            let len = pcm.len_secs();
            let autoplay = {
                let mut m = media(lua)?;
                let Some(p) = m.players.get_mut(&r) else { return Ok(()) };
                p.pcm = Some(pcm);
                p.loading = None;
                !p.playing
            };
            store_prop(lua, r, "IsReady", Variant::Bool(true))?;
            store_prop(lua, r, "TimeLength", Variant::Float64(len))?;
            let auto = autoplay && pb(world(lua)?.d(), r, "AutoPlay", false);
            if auto {
                set_playing(lua, r, true)?;
            }
        }
        Got::Pending(l) => {
            if let Some(p) = media(lua)?.players.get_mut(&r) {
                p.loading = Some(l);
            }
        }
        Got::Failed(e) => {
            let id = {
                let mut m = media(lua)?;
                let Some(p) = m.players.get_mut(&r) else { return Ok(()) };
                p.failed = true;
                p.loading = None;
                p.asset.clone()
            };
            world(lua)?.output.push((Level::Warn, format!("Failed to load sound {id}: {e}")));
        }
    }
    Ok(())
}

fn set_playing(lua: &Lua, r: Ref, on: bool) -> mlua::Result<()> {
    {
        let mut m = media(lua)?;
        let p = m.players.entry(r).or_default();
        p.playing = on;
        if on && p.reset_next {
            p.reset_next = false;
        }
    }
    if on {
        // Starting at or past the end goes back to the start.
        let (start, end) = {
            let w = world(lua)?;
            let m = media(lua)?;
            region(w.d(), r, m.players.get(&r).and_then(|p| p.pcm.as_ref()).map(|p| p.len_secs()))
        };
        if let Some(p) = media(lua)?.players.get_mut(&r) {
            if p.pcm.is_some() && (p.pos >= end || p.pos < start) {
                p.pos = start;
            }
        }
    }
    // Studio fires Changed("IsPlaying") on every Play / Stop.
    let mut w = world(lua)?;
    if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
        i.properties.insert(ustr("IsPlaying"), Variant::Bool(on));
    }
    w.dm().mark_prop(r, ustr("IsPlaying"));
    let name = Value::String(lua.create_string("IsPlaying")?);
    w.fire(r, "Changed", vec![name]);
    w.fire(r, "Changed:IsPlaying", vec![]);
    Ok(())
}

/// PlaybackRegion clamped to the asset.
fn region(d: &Document, r: Ref, len: Option<f64>) -> (f64, f64) {
    let len = len.unwrap_or(f64::INFINITY);
    let (a, b) = prange(d, r, "PlaybackRegion", (0.0, 60000.0));
    let (a, b) = ((a as f64).clamp(0.0, len), (b as f64).clamp(0.0, len));
    if b <= a { (0.0, len) } else { (a, b) }
}

fn loop_region(d: &Document, r: Ref, play: (f64, f64)) -> (f64, f64) {
    let (a, b) = prange(d, r, "LoopRegion", (0.0, 60000.0));
    let (a, b) = ((a as f64).clamp(play.0, play.1), (b as f64).clamp(play.0, play.1));
    if b <= a { play } else { (a, b) }
}

fn player_play(lua: &Lua, r: Ref, play: bool, args: &MultiValue) -> mlua::Result<MultiValue> {
    ensure_player(lua, r)?;
    if let Some(at) = crate::ltypes::num(&arg(args, 1)) {
        let mut m = media(lua)?;
        let id = m.next_action;
        m.next_action += 1;
        m.actions.push(Action { id, target: r, play, at });
        return Ok(one(Value::Integer(id)));
    }
    set_playing(lua, r, play)?;
    Ok(MultiValue::new())
}

fn cancel(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let id = crate::ltypes::num(&arg(args, 1)).map(|n| n as i64);
    let mut m = media(lua)?;
    let before = m.actions.len();
    if let Some(id) = id {
        m.actions.retain(|a| a.id != id);
    }
    Ok(one(Value::Boolean(m.actions.len() != before)))
}

/// GetWaveformAsync(timeRange, samples): the asset's samples at evenly spaced
/// times over the range (clamped to the asset), at most 8192 (Studio). Not
/// loaded yet: an empty table.
fn waveform(lua: &Lua, r: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    ensure_player(lua, r)?;
    let range = opt_ud::<crate::ltypes::LNumberRange>(&arg(args, 1))
        .ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
    let n = crate::ltypes::num(&arg(args, 2)).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
    let n = (n.max(0.0) as usize).min(8192);
    let pcm = media(lua)?.players.get(&r).and_then(|p| p.pcm.clone());
    let t = lua.create_table()?;
    if let Some(p) = pcm {
        let len = p.len_secs();
        let a = (range.min as f64).clamp(0.0, len);
        let b = (range.max as f64).clamp(0.0, len);
        for i in 0..n {
            let at = a + (b - a) * i as f64 / n as f64;
            let k = ((at * p.rate as f64) as usize).min(p.chans[0].len().saturating_sub(1));
            let s = p.chans.iter().map(|c| c.get(k).copied().unwrap_or(0.0)).sum::<f32>() / p.chans.len() as f32;
            t.raw_set(i + 1, s as f64)?;
        }
    }
    Ok(one(Value::Table(t)))
}

/// Renders `n` frames of a player from `pos` (loop / region aware).
fn render_player(d: &Document, r: Ref, p: &PlayerState, n: usize) -> Option<Buf> {
    if !p.playing {
        return None;
    }
    let pcm = p.pcm.as_ref()?;
    let len = pcm.len_secs();
    let speed = pd(d, r, "PlaybackSpeed", 1.0).max(0.0);
    let vol = pf(d, r, "Volume", 1.0);
    let looping = pb(d, r, "Looping", false);
    let play = region(d, r, Some(len));
    let lr = loop_region(d, r, play);
    let mut out = dsp::silence(pcm.chans.len(), n);
    let mut t = p.pos;
    let step = speed / dsp::RATE as f64;
    let rate = pcm.rate as f64;
    let frames = pcm.chans[0].len();
    for i in 0..n {
        if looping && t >= lr.1 {
            t = lr.0 + (t - lr.1) % (lr.1 - lr.0).max(1e-6);
        } else if !looping && t >= play.1 {
            break;
        }
        let x = t * rate;
        let k = x as usize;
        let f = (x - k as f64) as f32;
        if k + 1 >= frames {
            if k < frames {
                for (c, o) in out.iter_mut().enumerate() {
                    o[i] = pcm.chans[c][k] * vol;
                }
            }
        } else {
            for (c, o) in out.iter_mut().enumerate() {
                let s = &pcm.chans[c];
                o[i] = (s[k] * (1.0 - f) + s[k + 1] * f) * vol;
            }
        }
        t += step;
    }
    Some(out)
}

/// `n` frames of decoded audio from `pos` (linear interpolation; silence past the end).
fn render_pcm(pcm: &Pcm, pos: f64, speed: f64, vol: f32, n: usize) -> Buf {
    let mut out = dsp::silence(pcm.chans.len(), n);
    let frames = pcm.chans.first().map_or(0, |c| c.len());
    let step = speed / dsp::RATE as f64;
    for i in 0..n {
        let x = (pos + step * i as f64) * pcm.rate as f64;
        let k = x as usize;
        if k + 1 >= frames {
            break;
        }
        let f = (x - k as f64) as f32;
        for (c, o) in out.iter_mut().enumerate() {
            let s = &pcm.chans[c];
            o[i] = (s[k] * (1.0 - f) + s[k + 1] * f) * vol;
        }
    }
    out
}

/// Sound.PlaybackLoudness: 1000 × the source's RMS around TimePosition
/// (Studio: independent of Volume; 0 when not playing).
fn sound_loudness(lua: &Lua, r: Ref) -> mlua::Result<f64> {
    let (playing, pos, id) = {
        let w = world(lua)?;
        (
            w.input.sound_playing.contains(&r),
            w.input.sound_pos.get(&r).copied().unwrap_or(0.0),
            pstr(w.d(), r, "SoundId"),
        )
    };
    if !playing {
        return Ok(0.0);
    }
    let pcm = match pcm_cached(&id) {
        Some(p) => p,
        None => {
            media(lua)?.sound_loads.entry(r).or_insert((id, None));
            return Ok(0.0);
        }
    };
    let k = (pos * pcm.rate as f64) as usize;
    let win = 1024usize;
    let a = k.saturating_sub(win);
    let (mut sum, mut cnt) = (0f64, 0usize);
    for c in &pcm.chans {
        for &x in &c[a.min(c.len())..k.min(c.len())] {
            sum += (x as f64) * (x as f64);
            cnt += 1;
        }
    }
    Ok(if cnt == 0 { 0.0 } else { 1000.0 * (sum / cnt as f64).sqrt() })
}

// ------------------------------------------------------ emitters/listeners

/// The property a curve lives in (BinaryString, as Studio saves it).
fn curve_prop(angle: bool) -> &'static str {
    if angle { "AngleAttenuation" } else { "DistanceAttenuation" }
}

/// Studio's encoding (measured from a saved place): a 0 byte, then
/// (key, value) pairs as little-endian f32. Empty curve = the 0 byte alone.
pub fn encode_curve(pts: &[(f32, f32)]) -> Vec<u8> {
    let mut b = Vec::with_capacity(1 + pts.len() * 8);
    b.push(0);
    for (k, v) in pts {
        b.extend_from_slice(&k.to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

pub fn decode_curve(b: &[u8]) -> Vec<(f32, f32)> {
    let mut pts: Vec<(f32, f32)> = b
        .get(1..)
        .unwrap_or(&[])
        .chunks_exact(8)
        .map(|c| (f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]])))
        .filter(|(k, v)| k.is_finite() && v.is_finite())
        .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts
}

/// An emitter's / listener's curve, from the instance (set by a script or
/// loaded from a file).
fn curve(d: &Document, r: Ref, angle: bool) -> Vec<(f32, f32)> {
    match d.prop(r, curve_prop(angle)) {
        Some(Variant::BinaryString(b)) => decode_curve(AsRef::<[u8]>::as_ref(&b)),
        Some(Variant::String(s)) => decode_curve(s.as_bytes()),
        _ => Vec::new(),
    }
}

fn get_curve(lua: &Lua, r: Ref, angle: bool) -> mlua::Result<MultiValue> {
    let pts = curve(world(lua)?.d(), r, angle);
    let t = lua.create_table()?;
    for (k, v) in pts {
        t.raw_set(k as f64, v as f64)?;
    }
    Ok(one(Value::Table(t)))
}

fn set_curve(lua: &Lua, r: Ref, class: &str, name: &str, args: &MultiValue) -> mlua::Result<MultiValue> {
    let angle = name == "SetAngleAttenuation";
    // Studio takes a non-table silently.
    let Value::Table(t) = arg(args, 1) else { return Ok(MultiValue::new()) };
    let mut pts: Vec<(f32, f32)> = Vec::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let k = crate::ltypes::num(&k)
            .filter(|_| matches!(k, Value::Number(_) | Value::Integer(_)))
            .ok_or_else(|| rt_err(format!("{class}:{name}: numeric key expected.")))?;
        let v = match v {
            Value::Number(n) => n,
            Value::Integer(i) => i as f64,
            _ => return Err(rt_err(format!("{class}:{name}: numeric value expected."))),
        };
        let k = if angle { (k as f32).clamp(0.0, 180.0) } else { (k as f32).max(0.0) };
        pts.retain(|p| p.0 != k);
        pts.push((k, (v as f32).clamp(0.0, 1.0)));
    }
    if pts.len() > 400 {
        return Err(rt_err(format!("{class}:{name} called with too many points")));
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Into the instance (saved with the place, replicated), without a
    // Changed signal: the property is RobloxSecurity.
    let name = curve_prop(angle);
    let mut w = world(lua)?;
    if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
        i.properties.insert(ustr(name), Variant::BinaryString(encode_curve(&pts).into()));
    }
    w.dm().mark_prop(r, ustr(name));
    w.record(RepOp::Prop(r, name.to_string()));
    Ok(MultiValue::new())
}

/// World frame an emitter / listener sounds from (PositionType Parent or
/// Instance → PositionInstance): a part, attachment, camera or model.
fn node_frame(lua: &Lua, r: Ref) -> mlua::Result<Option<Xform>> {
    let target = {
        let w = world(lua)?;
        let d = w.d();
        if !in_dm(d, r) {
            return Ok(None);
        }
        if pe(d, r, "PositionType", 0) == 1 {
            pref(d, r, "PositionInstance")
        } else {
            d.dom.get_by_ref(r).map(|i| i.parent()).filter(|p| !p.is_none())
        }
    };
    let Some(t) = target else { return Ok(None) };
    let class = class_of(world(lua)?.d(), t);
    let is = |b: &str| crate::doc::is_a(&class, b);
    Ok(if is("BasePart") {
        Some(part_xform(lua, t)?)
    } else if class == "Attachment" {
        let (local, part) = attachment_frames(lua, t)?;
        Some(compose(&part, &local))
    } else if class == "Camera" {
        match world(lua)?.d().prop(t, "CFrame") {
            Some(Variant::CFrame(cf)) => Some(Xform::from_cframe(&cf)),
            _ => None,
        }
    } else if is("Model") {
        let w = world(lua)?;
        Some(model_pivot(&w, t))
    } else {
        None
    })
}

fn look(x: &Xform) -> glam::Vec3 {
    -x.rot.z_axis
}

fn angle_between(a: glam::Vec3, b: glam::Vec3) -> f32 {
    let (a, b) = (a.normalize_or_zero(), b.normalize_or_zero());
    if a == glam::Vec3::ZERO || b == glam::Vec3::ZERO {
        return 0.0;
    }
    a.dot(b).clamp(-1.0, 1.0).acos().to_degrees()
}

/// GetAudibilityFor: emitter distance·angle curves × listener distance·angle
/// curves (empty listener curves are 1), 0 across interaction groups or
/// when either has no position.
fn audibility(lua: &Lua, em: Ref, li: Ref) -> mlua::Result<f32> {
    let (ge, gl) = {
        let w = world(lua)?;
        (pstr(w.d(), em, "AudioInteractionGroup"), pstr(w.d(), li, "AudioInteractionGroup"))
    };
    if ge != gl {
        return Ok(0.0);
    }
    let (Some(fe), Some(fl)) = (node_frame(lua, em)?, node_frame(lua, li)?) else {
        return Ok(0.0);
    };
    let delta = fl.pos - fe.pos;
    let dist = delta.length();
    let (mode, bounds) = {
        let w = world(lua)?;
        (pe(w.d(), em, "DistanceAttenuationMode", 0), prange(w.d(), em, "DistanceAttenuationBounds", (4.0, 10000.0)))
    };
    let w = world(lua)?;
    let curve = |r: Ref, a: bool| curve(w.d(), r, a);
    let ed = dsp::distance_gain(mode, &curve(em, false), bounds, dist);
    let ea = { let c = curve(em, true); if c.is_empty() { 1.0 } else { dsp::interp(&c, angle_between(look(&fe), delta)) } };
    let ld = { let c = curve(li, false); if c.is_empty() { 1.0 } else { dsp::interp(&c, dist) } };
    let la = { let c = curve(li, true); if c.is_empty() { 1.0 } else { dsp::interp(&c, angle_between(look(&fl), -delta)) } };
    Ok(ed * ea * ld * la)
}

fn audibility_call(lua: &Lua, this: Ref, class: &str, args: &MultiValue) -> mlua::Result<MultiValue> {
    let other_class = if class == "AudioEmitter" { "AudioListener" } else { "AudioEmitter" };
    let o = opt_inst(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
    let oc = class_of(world(lua)?.d(), o);
    if oc != other_class {
        return Err(rt_err(format!("Unable to cast {oc} to {other_class}")));
    }
    let (em, li) = if class == "AudioEmitter" { (this, o) } else { (o, this) };
    Ok(one(Value::Number(audibility(lua, em, li)? as f64)))
}

fn interacting(lua: &Lua, this: Ref, emitter: bool) -> mlua::Result<MultiValue> {
    refresh(lua, true)?;
    let want = if emitter { "AudioListener" } else { "AudioEmitter" };
    let others: Vec<Ref> = media(lua)?.nodes.iter().filter(|(_, c)| c == want).map(|(r, _)| *r).collect();
    let mut out = Vec::new();
    for o in others {
        let (em, li) = if emitter { (this, o) } else { (o, this) };
        if audibility(lua, em, li)? > 0.0 {
            out.push(o);
        }
    }
    Ok(one(Value::Table(list(lua, out)?)))
}

// ---------------------------------------------------------------- analyzer

fn get_spectrum(lua: &Lua, r: Ref) -> mlua::Result<MultiValue> {
    let (enabled, win) = {
        let w = world(lua)?;
        (pb(w.d(), r, "SpectrumEnabled", true), dsp::window_samples(pe(w.d(), r, "WindowSize", 1)))
    };
    let spec = {
        let mut m = media(lua)?;
        let a = m.analyzers.entry(r).or_default();
        if enabled && a.active {
            a.spectrum = dsp::spectrum(&a.ring, win);
        } else if !a.active {
            a.spectrum.clear();
        }
        a.spectrum.clone()
    };
    let t = lua.create_table_with_capacity(spec.len(), 0)?;
    for (i, v) in spec.iter().enumerate() {
        t.raw_set(i + 1, *v as f64)?;
    }
    Ok(one(Value::Table(t)))
}

fn gain_at(lua: &Lua, r: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let f = crate::ltypes::num(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
    let w = world(lua)?;
    let d = w.d();
    let g = dsp::filter_gain_at(
        pe(d, r, "FilterType", 0),
        pf(d, r, "Frequency", 2000.0),
        pf(d, r, "Q", 0.707),
        pf(d, r, "Gain", 0.0),
        f as f32,
    );
    Ok(one(Value::Number(g as f64)))
}

// ------------------------------------------------------------ device input

fn get_access(lua: &Lua, r: Ref) -> mlua::Result<MultiValue> {
    let l = media(lua)?.access.get(&r).cloned().unwrap_or_default();
    Ok(one(Value::Table(lua.create_sequence_from(l)?)))
}

fn set_access(lua: &Lua, r: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let t = match arg(args, 1) {
        Value::Table(t) => t,
        v => {
            let ty = match v {
                Value::Number(_) | Value::Integer(_) => "double",
                Value::String(_) => "string",
                Value::Boolean(_) => "bool",
                Value::Nil => "nil",
                _ => v.type_name(),
            };
            return Err(rt_err(format!("Unable to cast {ty} to Array")));
        }
    };
    // Studio keeps the numbers (rounded), drops anything else.
    let mut ids: Vec<i64> = Vec::new();
    for v in t.sequence_values::<Value>() {
        if let Some(n) = match v? {
            Value::Number(n) => Some(n.round() as i64),
            Value::Integer(i) => Some(i),
            _ => None,
        } {
            if !ids.contains(&n) {
                ids.push(n);
            }
        }
    }
    media(lua)?.access.insert(r, ids);
    Ok(MultiValue::new())
}

// -------------------------------------------------------------------- step

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    if lua.app_data_ref::<Media>().is_none() {
        return Ok(());
    }
    // Scripts' own wires / players register as they're touched; a full scan
    // (files, Clone, replication) runs 4× a second.
    let due = {
        let now = world(lua)?.time;
        now - media(lua)?.scanned_at >= 0.25
    };
    if due {
        refresh(lua, true)?;
    }
    // Scheduled Play / Stop (atTime on the mixer clock).
    let due: Vec<Action> = {
        let mut m = media(lua)?;
        m.mixer_time += dt;
        let t = m.mixer_time;
        let (due, keep): (Vec<Action>, Vec<Action>) = std::mem::take(&mut m.actions).into_iter().partition(|a| a.at <= t);
        m.actions = keep;
        due
    };
    for a in due {
        set_playing(lua, a.target, a.play)?;
    }
    // Wires: Connected may change with ancestry.
    let wires = media(lua)?.wire_order.clone();
    let exists: Vec<Ref> = {
        let w = world(lua)?;
        wires.into_iter().filter(|r| w.d().exists(*r)).collect()
    };
    for wr in &exists {
        let stale = {
            let now = wire_ends(world(lua)?.d(), *wr);
            media(lua)?.wires.get(wr).is_some_and(|r| r.seen.as_ref() != Some(&now))
        };
        if stale {
            validate(lua, *wr, false)?;
        }
        sync_wire(lua, *wr)?;
    }
    {
        // Forget destroyed wires (after their last WiringChanged).
        let mut m = media(lua)?;
        let keep: HashSet<Ref> = exists.iter().copied().collect();
        let gone: Vec<Ref> = m.wire_order.iter().filter(|w| !keep.contains(w)).copied().collect();
        for g in gone {
            if m.wires.get(&g).is_some_and(|w| !w.connected) {
                m.wires.remove(&g);
                m.wire_order.retain(|w| *w != g);
            }
        }
    }
    let nodes = media(lua)?.nodes.clone();
    // Players found in the DataModel load their assets; pending loads poll.
    for (r, c) in &nodes {
        if c == "AudioPlayer" {
            ensure_player(lua, *r)?;
        }
    }
    let pending: Vec<(Ref, String, Load)> = {
        let mut m = media(lua)?;
        m.players
            .iter_mut()
            .filter_map(|(r, p)| p.loading.take().map(|l| (*r, p.asset.clone(), l)))
            .collect()
    };
    for (r, id, l) in pending {
        let got = poll_load(&id, l);
        finish_load(lua, r, got)?;
    }
    sound_loads(lua)?;
    misc::step(lua, dt)?;

    // Render the graph for this frame.
    let n = {
        let mut m = media(lua)?;
        let exact = dt * dsp::RATE as f64 + m.carry;
        let n = exact.floor().max(0.0);
        m.carry = exact - n;
        (n as usize).min(dsp::RATE as usize / 4)
    };
    if n > 0 {
        render(lua, &nodes, n)?;
    }
    advance_players(lua, dt)?;
    Ok(())
}

fn sound_loads(lua: &Lua) -> mlua::Result<()> {
    let items: Vec<(Ref, String, Option<Load>)> = {
        let mut m = media(lua)?;
        m.sound_loads.drain().map(|(r, (id, l))| (r, id, l)).collect()
    };
    for (r, id, l) in items {
        if id.trim().is_empty() {
            continue;
        }
        let got = match l {
            Some(l) => poll_load(&id, l),
            None => start_load(&id),
        };
        match got {
            Got::Ready(_) => {
                let still = pstr(world(lua)?.d(), r, "SoundId") == id;
                if still {
                    let v = Value::String(lua.create_string(&id)?);
                    world(lua)?.fire(r, "Loaded", vec![v]);
                }
            }
            Got::Pending(l) => {
                media(lua)?.sound_loads.insert(r, (id, Some(l)));
            }
            Got::Failed(_) => {}
        }
    }
    Ok(())
}

fn advance_players(lua: &Lua, dt: f64) -> mlua::Result<()> {
    let refs: Vec<Ref> = media(lua)?.players.keys().copied().collect();
    let mut ended = Vec::new();
    let mut looped = Vec::new();
    {
        let w = world(lua)?;
        let d = w.d();
        let mut m = media(lua)?;
        for r in refs {
            if !d.exists(r) {
                m.players.remove(&r);
                continue;
            }
            let Some(p) = m.players.get_mut(&r) else { continue };
            let len = p.pcm.as_ref().map(|x| x.len_secs());
            let play = region(d, r, len);
            if p.reset_next {
                p.reset_next = false;
                p.pos = play.0;
            }
            if !p.playing || p.pcm.is_none() || !in_dm(d, r) {
                continue;
            }
            let speed = pd(d, r, "PlaybackSpeed", 1.0).max(0.0);
            p.pos += dt * speed;
            if pb(d, r, "Looping", false) {
                let lr = loop_region(d, r, play);
                let span = (lr.1 - lr.0).max(1e-6);
                let mut wraps = 0;
                while p.pos >= lr.1 && wraps < 64 {
                    p.pos -= span;
                    wraps += 1;
                    looped.push(r);
                }
            } else if p.pos >= play.1 {
                p.pos = play.1;
                p.playing = false;
                p.reset_next = true;
                ended.push(r);
            }
        }
    }
    for r in ended {
        // IsPlaying goes false with no Changed signal (Studio).
        let mut w = world(lua)?;
        if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
            i.properties.insert(ustr("IsPlaying"), Variant::Bool(false));
        }
        w.fire(r, "Ended", vec![]);
    }
    for r in looped {
        world(lua)?.fire(r, "Looped", vec![]);
    }
    Ok(())
}

struct Ctx<'a> {
    lua: &'a Lua,
    n: usize,
    /// (target, pin) → [(source, source pin)] over connected wires.
    inputs: HashMap<(Ref, String), Vec<(Ref, String)>>,
    class: HashMap<Ref, String>,
    memo: HashMap<(Ref, String), Option<Buf>>,
    visiting: HashSet<Ref>,
    emitters: Vec<Ref>,
    frames: HashMap<Ref, Option<Xform>>,
}

fn render(lua: &Lua, nodes: &[(Ref, String)], n: usize) -> mlua::Result<()> {
    let sinks: Vec<(Ref, String)> = nodes
        .iter()
        .filter(|(_, c)| matches!(c.as_str(), "AudioAnalyzer" | "AudioDeviceOutput"))
        .cloned()
        .collect();
    if sinks.is_empty() {
        return Ok(());
    }
    let mut inputs: HashMap<(Ref, String), Vec<(Ref, String)>> = HashMap::new();
    let order = media(lua)?.wire_order.clone();
    for wr in order {
        if !media(lua)?.wires.get(&wr).is_some_and(|w| w.connected) {
            continue;
        }
        let (s, sn, t, tn) = wire_ends(world(lua)?.d(), wr);
        if let (Some(s), Some(t)) = (s, t) {
            inputs.entry((t, tn)).or_default().push((s, sn));
        }
    }
    let mut ctx = Ctx {
        lua,
        n,
        inputs,
        class: nodes.iter().cloned().collect(),
        memo: HashMap::new(),
        visiting: HashSet::new(),
        emitters: nodes.iter().filter(|(_, c)| c == "AudioEmitter").map(|(r, _)| *r).collect(),
        frames: HashMap::new(),
    };
    let (side_out, me) = {
        let w = world(lua)?;
        let local = w.side == Side::Client || !w.has_peer;
        (local, local_player(&w))
    };
    for (r, c) in sinks {
        let input = in_mix(&mut ctx, r, "Input")?;
        if c == "AudioAnalyzer" {
            let win = dsp::window_samples(pe(world(lua)?.d(), r, "WindowSize", 1));
            let mut m = media(lua)?;
            let a = m.analyzers.entry(r).or_default();
            match input {
                Some(b) => {
                    if a.ring.len() != b.len() {
                        a.ring = vec![Vec::new(); b.len()];
                    }
                    for (ring, ch) in a.ring.iter_mut().zip(&b) {
                        ring.extend_from_slice(ch);
                        let extra = ring.len().saturating_sub(4096);
                        if extra > 0 {
                            ring.drain(..extra);
                        }
                    }
                    let (rms, peak) = dsp::levels(&a.ring, win);
                    a.rms = rms;
                    a.peak = peak;
                    a.active = true;
                }
                None => {
                    a.ring.clear();
                    a.rms = 0.0;
                    a.peak = 0.0;
                    a.active = false;
                }
            }
        } else if let Some(b) = input {
            // AudioDeviceOutput: on this machine's speakers when its Player is
            // unset or the local player.
            let player = pref(world(lua)?.d(), r, "Player");
            if side_out && (player.is_none() || player == me) {
                let st = dsp::convert(&b, 2);
                let mut inter = Vec::with_capacity(n * 2);
                for i in 0..n {
                    inter.push(st[0][i]);
                    inter.push(st[1][i]);
                }
                crate::audio::graph_output(&inter);
            }
        }
    }
    Ok(())
}

fn local_player(w: &World) -> Option<Ref> {
    let d = w.d();
    let players = d.dom.descendants().find(|i| i.class.as_str() == "Players")?;
    match d.prop(players.referent(), "LocalPlayer") {
        Some(Variant::Ref(r)) if !r.is_none() => Some(r),
        _ => None,
    }
}

fn in_mix(ctx: &mut Ctx, node: Ref, pin: &str) -> mlua::Result<Option<Buf>> {
    let srcs = ctx.inputs.get(&(node, pin.to_string())).cloned().unwrap_or_default();
    let mut bufs = Vec::new();
    for (s, sn) in srcs {
        if let Some(b) = output(ctx, s, &sn)? {
            bufs.push(b);
        }
    }
    Ok(dsp::mix(bufs.iter()))
}

fn output(ctx: &mut Ctx, node: Ref, pin: &str) -> mlua::Result<Option<Buf>> {
    let key = (node, pin.to_string());
    if let Some(b) = ctx.memo.get(&key) {
        return Ok(b.clone());
    }
    if !ctx.visiting.insert(node) {
        return Ok(None);
    }
    let lua = ctx.lua;
    let n = ctx.n;
    let class = match ctx.class.get(&node) {
        Some(c) => c.clone(),
        None => class_of(world(lua)?.d(), node),
    };
    let out = match class.as_str() {
        "AudioPlayer" => {
            let w = world(lua)?;
            let m = media(lua)?;
            m.players.get(&node).and_then(|p| render_player(w.d(), node, p, n))
        }
        "AudioChannelMixer" => {
            let layout = dsp::layout(pe(world(lua)?.d(), node, "Layout", 1));
            let base = in_mix(ctx, node, "Input")?.map(|b| dsp::convert(&b, layout.len()));
            let mut any = base.is_some();
            let mut out = base.unwrap_or_else(|| dsp::silence(layout.len(), n));
            for pin in dsp::CHANNEL_PINS {
                let ch = dsp::pin_channel(pin).unwrap();
                if let Some(b) = in_mix(ctx, node, pin)? {
                    any = true;
                    if let Some(i) = layout.iter().position(|c| *c == ch) {
                        let mono = dsp::convert(&b, 1);
                        for (o, s) in out[i].iter_mut().zip(&mono[0]) {
                            *o += s;
                        }
                    }
                }
            }
            any.then_some(out)
        }
        "AudioChannelSplitter" => {
            let layout = dsp::layout(pe(world(lua)?.d(), node, "Layout", 1));
            in_mix(ctx, node, "Input")?.map(|b| {
                let conv = dsp::convert(&b, layout.len());
                if pin == "Output" {
                    conv
                } else {
                    let ch = dsp::pin_channel(pin);
                    match ch.and_then(|ch| layout.iter().position(|c| *c == ch)) {
                        Some(i) => vec![conv[i].clone()],
                        None => dsp::silence(1, n),
                    }
                }
            })
        }
        "AudioListener" => render_listener(ctx, node)?,
        "AudioWindSynthesizer" => {
            let (on, vol, profile) = {
                let w = world(lua)?;
                let d = w.d();
                (pb(d, node, "Enabled", true), pf(d, node, "Volume", 1.0), pe(d, node, "Profile", 0))
            };
            if on {
                let speed = {
                    let w = world(lua)?;
                    let d = w.d();
                    let ws = d.dom.descendants().find(|i| i.class.as_str() == "Workspace").map(|i| i.referent());
                    match ws.and_then(|ws| d.prop(ws, "GlobalWind")) {
                        Some(Variant::Vector3(v)) => (v.x * v.x + v.y * v.y + v.z * v.z).sqrt(),
                        _ => 0.0,
                    }
                };
                let mut m = media(lua)?;
                Some(m.wind.entry(node).or_default().render(n, speed, profile, vol))
            } else {
                None
            }
        }
        // No device / backend here (as Studio offline): nothing comes out.
        "AudioDeviceInput" | "AudioTextToSpeech" => None,
        // A VideoPlayer's soundtrack (Opus, media_video.rs) at its TimePosition.
        "VideoPlayer" => {
            let got = { let m = media(lua)?; misc::video_audio(&m, node) };
            match got {
                Some((pcm, pos)) => {
                    let w = world(lua)?;
                    let d = w.d();
                    let speed = pf(d, node, "PlaybackSpeed", 1.0).max(0.0) as f64;
                    let vol = pf(d, node, "Volume", 1.0);
                    Some(render_pcm(&pcm, pos, speed, vol, n))
                }
                None => None,
            }
        }
        c => {
            let side = if c == "AudioCompressor" { in_mix(ctx, node, "Sidechain")? } else { None };
            match in_mix(ctx, node, "Input")? {
                Some(mut b) => {
                    let w = world(lua)?;
                    let mut m = media(lua)?;
                    if !m.fx.contains_key(&node) {
                        if let Some(fx) = dsp::Fx::new(c) {
                            m.fx.insert(node, fx);
                        }
                    }
                    if let Some(fx) = m.fx.get_mut(&node) {
                        let p = NodeParams { d: w.d(), r: node, class: c };
                        fx.process(&p, &mut b, side.as_ref());
                    }
                    Some(b)
                }
                None => None,
            }
        }
    };
    ctx.visiting.remove(&node);
    ctx.memo.insert(key, out.clone());
    Ok(out)
}

fn frame_cached(ctx: &mut Ctx, r: Ref) -> mlua::Result<Option<Xform>> {
    if let Some(f) = ctx.frames.get(&r) {
        return Ok(*f);
    }
    let f = node_frame(ctx.lua, r)?;
    ctx.frames.insert(r, f);
    Ok(f)
}

/// What a listener hears: every emitter's input, attenuated
/// (GetAudibilityFor) and panned to stereo around the listener.
fn render_listener(ctx: &mut Ctx, li: Ref) -> mlua::Result<Option<Buf>> {
    let lua = ctx.lua;
    let Some(fl) = frame_cached(ctx, li)? else { return Ok(None) };
    let mut out: Option<Buf> = None;
    for em in ctx.emitters.clone() {
        let a = audibility(lua, em, li)?;
        if a <= 0.0 {
            continue;
        }
        let Some(fe) = frame_cached(ctx, em)? else { continue };
        let Some(x) = in_mix(ctx, em, "Input")? else { continue };
        let mono = dsp::convert(&x, 1);
        let local = fl.inverse().apply(fe.pos);
        let (pl, pr) = dsp::pan(local.x, local.length());
        let o = out.get_or_insert_with(|| dsp::silence(2, ctx.n));
        for (i, s) in mono[0].iter().enumerate() {
            o[0][i] += s * a * pl;
            o[1][i] += s * a * pr;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod curve_tests {
    use super::*;

    /// #16: a running place's audio (Sound.SoundId, AudioPlayer.Asset) is
    /// readable through GetWaveformAsync / AudioAnalyzer, so local files
    /// don't load, even with their samples already decoded and cached.
    #[test]
    fn local_audio_is_denied_even_if_pcm_is_cached() {
        let path = std::env::temp_dir().join(format!("rustudio-private-audio-{}.wav", std::process::id()));
        let path = path.to_string_lossy().into_owned();
        for id in [format!("file://{path}"), path.clone(), "private.wav".to_string()] {
            pcm_store(&id, Arc::new(Pcm { rate: 8000, chans: vec![vec![0.25, -0.25]] }));
            let denied = matches!(start_load(&id), Got::Failed(_)) && pcm_cached(&id).is_none();
            PCM.lock().unwrap().as_mut().unwrap().remove(&id);
            assert!(denied || crate::assets::local_content_opt_in(), "local audio cache bypass: {id}");
        }
        // Roblox content still plays from the cache.
        for id in ["rbxasset://sounds/x.wav", "rbxassetid://123", "rbxtemp://rustudio/tts/1"] {
            pcm_store(id, Arc::new(Pcm { rate: 8000, chans: vec![vec![0.25, -0.25]] }));
            let ready = matches!(start_load(id), Got::Ready(_));
            PCM.lock().unwrap().as_mut().unwrap().remove(id);
            assert!(ready, "{id}");
        }
    }

    /// Byte-for-byte what Studio saved (curves.rbxlx, 2026-11-08).
    #[test]
    fn curve_encoding_matches_studio() {
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let unhex = |h: &str| (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect::<Vec<u8>>();
        assert_eq!(hex(&encode_curve(&[])), "00");
        assert_eq!(hex(&encode_curve(&[(0.0, 1.0), (10.0, 0.5), (30.0, 0.1)])), "00000000000000803f000020410000003f0000f041cdcccc3d");
        assert_eq!(hex(&encode_curve(&[(0.0, 1.0), (90.0, 0.25), (180.0, 0.0)])), "00000000000000803f0000b4420000803e0000344300000000");
        assert_eq!(hex(&encode_curve(&[(2.5, 0.75)])), "00000020400000403f");
        let back = decode_curve(&unhex("00000000000000803f0000004100000000"));
        assert_eq!(back, vec![(0.0, 1.0), (8.0, 0.0)]);
    }

    /// rustudio's place writer keeps it as Studio writes it.
    #[test]
    fn curve_saves_like_studio() {
        let mut dom = rbx_dom_weak::WeakDom::new(rbx_dom_weak::InstanceBuilder::new("DataModel"));
        let root = dom.root_ref();
        let e = dom.insert(
            root,
            rbx_dom_weak::InstanceBuilder::new("AudioEmitter").with_property(
                "DistanceAttenuation",
                Variant::BinaryString(encode_curve(&[(0.0, 1.0), (10.0, 0.5), (30.0, 0.1)]).into()),
            ),
        );
        let mut out = Vec::new();
        rbx_xml::to_writer_default(&mut out, &dom, &[e]).unwrap();
        let xml = String::from_utf8(out).unwrap();
        assert!(
            // rbx_xml writes BinaryStrings as CDATA; the payload is Studio's.
            xml.contains("<BinaryString name=\"DistanceAttenuation\"><![CDATA[AAAAAAAAAIA/AAAgQQAAAD8AAPBBzczMPQ==]]></BinaryString>"),
            "{xml}"
        );
        let mut bin = Vec::new();
        rbx_binary::to_writer(&mut bin, &dom, &[e]).unwrap();
        let back = rbx_binary::from_reader(&bin[..]).unwrap();
        let em = back.root().children()[0];
        match back.get_by_ref(em).unwrap().properties.get(&ustr("DistanceAttenuation")) {
            Some(Variant::BinaryString(b)) => assert_eq!(decode_curve(b.as_ref()).len(), 3),
            other => panic!("{other:?}"),
        }
    }
}
