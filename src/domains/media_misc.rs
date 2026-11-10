//! Media members without a real backend offline, done the way Studio does
//! them then (measured in an offline Studio, 2026-11-07), plus video timing.
//!
//! - AudioTextToSpeech: LoadAsync prints "Failed to load text to speech" and
//!   returns AssetFetchStatus.Failure; Play / Pause only flip IsPlaying.
//! - AudioSpeechToText: never hears a voice (VoiceDetected false, Text "").
//! - AudioRecorder: Studio's RecordAsync / CanRecordAsync /
//!   GetUnrecordableInstancesAsync raise "AudioRecorder is not enabled yet".
//! - Voice: VoiceChatInternal answers as a client with no voice session
//!   (GetVoiceChatAvailable -1, JoinByGroupId true, …); eligibility for
//!   others raises Studio's error; GetChatGroupsAsync is disabled.
//! - Video: VideoPlayer / VideoFrame load the WebM (session download),
//!   read its duration and frame size from the container, and keep time
//!   (TimePosition, DidEnd / Ended, DidLoop) and their events. Frames are
//!   not decoded (no VP8/VP9 decoder in rustudio), so VideoSampler can't be
//!   made and VideoDisplay shows nothing.
//! - VideoCaptureService mocks and VideoDeviceInput are InternalTest /
//!   RobloxScript only in Studio: the same capability errors.

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::ustr;

use crate::app::Level;
use crate::runtime::*;

use std::sync::Arc;

use super::video::{self as vid, Decoder, Stream, Track};
use super::{one, pb, pd, pstr};

pub const METHODS: &[(&str, &str)] = &[
    ("Play", "AudioTextToSpeech"),
    ("Pause", "AudioTextToSpeech"),
    ("Unload", "AudioTextToSpeech"),
    ("LoadAsync", "AudioTextToSpeech"),
    ("GetWaveformAsync", "AudioTextToSpeech"),
    ("Clear", "AudioRecorder"),
    ("Stop", "AudioRecorder"),
    ("GetTemporaryContent", "AudioRecorder"),
    ("CanRecordAsync", "AudioRecorder"),
    ("GetUnrecordableInstancesAsync", "AudioRecorder"),
    ("RecordAsync", "AudioRecorder"),
    ("Play", "VideoPlayer"),
    ("Pause", "VideoPlayer"),
    ("Unload", "VideoPlayer"),
    ("LoadAsync", "VideoPlayer"),
    ("Play", "VideoFrame"),
    ("Pause", "VideoFrame"),
    ("CreateVideoSamplerAsync", "VideoService"),
    ("GetSamplesAtTimesAsync", "VideoSampler"),
    ("ClearMockInput", "VideoCaptureService"),
    ("SetMockInputAsync", "VideoCaptureService"),
    ("GetAudioProcessingSettings", "VoiceChatInternal"),
    ("GetMicDevices", "VoiceChatInternal"),
    ("GetParticipants", "VoiceChatInternal"),
    ("GetVoiceChatApiVersion", "VoiceChatInternal"),
    ("GetVoiceChatAvailable", "VoiceChatInternal"),
    ("IsPublishPaused", "VoiceChatInternal"),
    ("IsSubscribePaused", "VoiceChatInternal"),
    ("JoinByGroupId", "VoiceChatInternal"),
    ("JoinByGroupIdToken", "VoiceChatInternal"),
    ("Leave", "VoiceChatInternal"),
    ("PublishPause", "VoiceChatInternal"),
    ("SetMicDevice", "VoiceChatInternal"),
    ("SubscribePause", "VoiceChatInternal"),
    ("SubscribePauseAll", "VoiceChatInternal"),
    ("IsVoiceEnabledForUserIdAsync", "VoiceChatInternal"),
    ("IsVoiceEnabledForUserIdAsync", "VoiceChatService"),
    ("GetChatGroupsAsync", "VoiceChatService"),
];

pub const EVENTS: &[(&str, &str)] = &[
    ("Ended", "AudioTextToSpeech"),
    ("Looped", "AudioTextToSpeech"),
    ("DidEnd", "VideoPlayer"),
    ("DidLoop", "VideoPlayer"),
    ("PlayFailed", "VideoPlayer"),
    ("DidLoop", "VideoFrame"),
    ("Ended", "VideoFrame"),
    ("Loaded", "VideoFrame"),
    ("Paused", "VideoFrame"),
    ("Played", "VideoFrame"),
    ("StateChanged", "VoiceChatInternal"),
];

#[derive(Clone, Copy, Default)]
struct VideoInfo {
    len: f64,
    w: f32,
    h: f32,
}

#[derive(Default)]
struct VideoState {
    content: String,
    info: Option<VideoInfo>,
    track: Option<Arc<Track>>,
    stream: Option<Stream>,
    pos: f64,
    playing: bool,
    tried: bool,
}

struct Sampler {
    track: Arc<Track>,
    content: String,
    max: u32,
    dec: Option<Decoder>,
}

#[derive(Default)]
pub struct MiscState {
    videos: HashMap<Ref, VideoState>,
    samplers: HashMap<Ref, Sampler>,
    sample_seq: u64,
}

fn misc(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, super::Media>> {
    super::media(lua)
}

fn enum_item(lua: &Lua, e: &str, name: &str) -> mlua::Result<Value> {
    match crate::ltypes::LEnumItem::from_name(e, name) {
        Some(i) => crate::ltypes::enum_value(lua, i),
        None => Ok(Value::Nil),
    }
}

fn set_bool(lua: &Lua, r: Ref, k: &str, v: bool) -> mlua::Result<()> {
    store_prop(lua, r, k, Variant::Bool(v))
}

pub fn call(lua: &Lua, name: &str, this: Ref, class: &str, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let none = || Ok(MultiValue::new());
    let r = match (class, name) {
        // ------------------------------------------------ text to speech
        ("AudioTextToSpeech", "LoadAsync") => (|| {
            world(lua)?.output.push((Level::Error, "Failed to load text to speech".into()));
            Ok(one(enum_item(lua, "AssetFetchStatus", "Failure")?))
        })(),
        ("AudioTextToSpeech", "Play") => set_bool(lua, this, "IsPlaying", true).and_then(|_| none()),
        ("AudioTextToSpeech", "Pause") => set_bool(lua, this, "IsPlaying", false).and_then(|_| none()),
        ("AudioTextToSpeech", "Unload") => none(),
        ("AudioTextToSpeech", "GetWaveformAsync") => lua.create_table().map(|t| one(Value::Table(t))),
        // ------------------------------------------------------ recorder
        ("AudioRecorder", "Clear" | "Stop") => none(),
        ("AudioRecorder", "GetTemporaryContent") => {
            variant_to_lua(lua, &Variant::Content(rbx_dom_weak::types::Content::none()), None).map(one)
        }
        ("AudioRecorder", "CanRecordAsync" | "GetUnrecordableInstancesAsync" | "RecordAsync") => {
            Err(rt_err("AudioRecorder is not enabled yet"))
        }
        // --------------------------------------------------------- video
        ("VideoPlayer" | "VideoFrame", "LoadAsync" | "Play" | "Pause" | "Unload") => video_call(lua, this, class, name),
        ("VideoService", "CreateVideoSamplerAsync") => (|| {
            let client = world(lua)?.side == Side::Client;
            if !client {
                return Err(rt_err("VideoSampler can only be created on the Client"));
            }
            let id = match arg(args, 1) {
                Value::String(s) => s.to_str()?.to_string(),
                v => opt_ud::<crate::runtime::editable::LContent>(&v)
                    .and_then(|c| match c.variant() {
                        Variant::Content(c) => c.as_uri().map(|s| s.to_string()),
                        _ => None,
                    })
                    .unwrap_or_default(),
            };
            let (info, track) = load_video(lua, &id).map_err(|e| rt_err(format!("Unable to create VideoSampler: {e}")))?;
            let _ = info;
            // Options.Size (Enum.VideoSampleSize): the sample's larger side.
            // Studio's exact sizes aren't documented (sampling crashes Studio
            // under Wine): Small 480, Medium 960, Large 1440, Full 1920.
            let max = match arg(args, 2) {
                Value::Table(t) => match t.get::<Value>("Size")? {
                    v @ Value::UserData(_) => match opt_ud::<crate::ltypes::LEnumItem>(&v) {
                        Some(e) if e.enum_type == "VideoSampleSize" => [480, 960, 1440, 1920][(e.value as usize).min(3)],
                        _ => return Err(rt_err("Unable to create VideoSampler: Size must be a VideoSampleSize")),
                    },
                    Value::Nil => 480,
                    _ => return Err(rt_err("Unable to create VideoSampler: Size must be a VideoSampleSize")),
                },
                _ => 480,
            };
            let r = crate::runtime::ext::create_any(lua, "VideoSampler")?;
            misc(lua)?.misc.samplers.insert(r, Sampler { track, content: id, max, dec: None });
            Ok(one(inst_value(lua, r)?))
        })(),
        ("VideoCaptureService", "ClearMockInput" | "SetMockInputAsync") => Err(rt_err(format!(
            "The current thread cannot call '{name}' (lacking capability InternalTest)"
        ))),
        // --------------------------------------------------------- voice
        ("VoiceChatInternal", "GetAudioProcessingSettings" | "GetMicDevices" | "Leave" | "SetMicDevice") => none(),
        ("VoiceChatInternal", "GetParticipants") => lua.create_table().map(|t| one(Value::Table(t))),
        ("VoiceChatInternal", "GetVoiceChatApiVersion") => Ok(one(Value::Integer(0))),
        ("VoiceChatInternal", "GetVoiceChatAvailable") => Ok(one(Value::Integer(-1))),
        ("VoiceChatInternal", "JoinByGroupId") => Ok(one(Value::Boolean(true))),
        (
            "VoiceChatInternal",
            "IsPublishPaused" | "IsSubscribePaused" | "JoinByGroupIdToken" | "PublishPause" | "SubscribePause"
            | "SubscribePauseAll",
        ) => Ok(one(Value::Boolean(false))),
        ("VoiceChatInternal" | "VoiceChatService", "IsVoiceEnabledForUserIdAsync") => (|| {
            let uid = crate::ltypes::num(&arg(args, 1)).unwrap_or(0.0) as i64;
            let (server, me) = {
                let w = world(lua)?;
                let d = w.d();
                let me = d
                    .dom
                    .descendants()
                    .find(|i| i.class.as_str() == "Players")
                    .and_then(|p| match d.prop(p.referent(), "LocalPlayer") {
                        Some(Variant::Ref(r)) if !r.is_none() => match d.prop(r, "UserId") {
                            Some(Variant::Int64(u)) => Some(u),
                            _ => None,
                        },
                        _ => None,
                    });
                (w.side == Side::Server, me)
            };
            // The server may ask about any player connected to it (Studio,
            // signed in: the local player → its setting; anyone else → this
            // error). (players_chat, 2026-11-08.)
            if server {
                let connected = {
                    let w = world(lua)?;
                    let d = w.d();
                    d.dom
                        .descendants()
                        .find(|i| i.class.as_str() == "Players")
                        .map(|p| {
                            p.children().iter().any(|c| matches!(d.prop(*c, "UserId"), Some(Variant::Int64(u)) if u == uid))
                        })
                        .unwrap_or(false)
                };
                if !connected {
                    return Err(rt_err(
                        "VoiceChatService:IsVoiceEnabledForUserIdAsync() the player must be connected to the current server",
                    ));
                }
                return voice_enabled(lua, uid).map(|b| one(Value::Boolean(b))).map_err(rt_err);
            }
            if me != Some(uid) {
                return Err(rt_err(
                    "VoiceChatService:IsVoiceEnabledForUserIdAsync() cannot access eligibility info for other users",
                ));
            }
            voice_enabled(lua, uid).map(|b| one(Value::Boolean(b))).map_err(rt_err)
        })(),
        ("VideoSampler", "GetSamplesAtTimesAsync") => samples(lua, this, args),
        ("VoiceChatService", "GetChatGroupsAsync") => Err(rt_err("VoiceChatService:GetChatGroupsAsync is disabled.")),
        _ => return None,
    };
    Some(r)
}

/// A playing VideoPlayer's soundtrack and position (the audio graph).
pub fn video_audio(m: &super::Media, r: Ref) -> Option<(Arc<crate::audio::Pcm>, f64)> {
    let v = m.misc.videos.get(&r)?;
    if !v.playing {
        return None;
    }
    Some((v.track.as_ref()?.audio.clone()?, v.pos))
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    Ok(match (class, key) {
        ("AudioTextToSpeech", "IsLoaded") => Some(Value::Boolean(false)),
        ("AudioTextToSpeech", "TimeLength") => Some(Value::Number(0.0)),
        ("AudioTextToSpeech", "IsPlaying") => {
            Some(Value::Boolean(pb(world(lua)?.d(), r, "IsPlaying", false)))
        }
        ("AudioSpeechToText", "VoiceDetected") => Some(Value::Boolean(false)),
        ("AudioRecorder", "TimeLength") | ("VideoCapture", "TimeLength") => Some(Value::Number(0.0)),
        ("VideoDeviceInput", "IsReady") => Some(Value::Boolean(false)),
        ("VideoSampler", "TimeLength" | "VideoContent") => {
            let m = misc(lua)?;
            let s = m.misc.samplers.get(&r);
            let (len, content) = (s.map_or(0.0, |s| s.track.len), s.map(|s| s.content.clone()).unwrap_or_default());
            drop(m);
            Some(if key == "TimeLength" {
                Value::Number(len)
            } else {
                let c = if content.is_empty() { rbx_dom_weak::types::Content::none() } else { rbx_dom_weak::types::Content::from(content) };
                variant_to_lua(lua, &Variant::Content(c), None)?
            })
        }
        ("VoiceChatInternal", "VoiceChatState") => {
            return Err(rt_err("VoiceChatInternal API security check failed"))
        }
        ("VideoPlayer" | "VideoFrame", "IsLoaded" | "IsPlaying" | "Playing" | "Resolution" | "TimeLength" | "TimePosition") => {
            if (class, key) == ("VideoPlayer", "Playing") || (class, key) == ("VideoFrame", "IsPlaying") {
                return Ok(None);
            }
            let m = misc(lua)?;
            let v = m.misc.videos.get(&r);
            let info = v.and_then(|v| v.info);
            Some(match key {
                "IsLoaded" => Value::Boolean(info.is_some()),
                "IsPlaying" | "Playing" => Value::Boolean(v.is_some_and(|v| v.playing)),
                "Resolution" => {
                    let i = info.unwrap_or_default();
                    drop(m);
                    return Ok(Some(vec2(lua, i.w, i.h)?));
                }
                "TimeLength" => Value::Number(info.map_or(0.0, |i| i.len)),
                _ => {
                    let pos = v.map(|v| v.pos);
                    drop(m);
                    Value::Number(pos.unwrap_or_else(|| world(lua).map(|w| pd(w.d(), r, "TimePosition", 0.0)).unwrap_or(0.0)))
                }
            })
        }
        _ => None,
    })
}

fn vec2(lua: &Lua, x: f32, y: f32) -> mlua::Result<Value> {
    variant_to_lua(lua, &Variant::Vector2(rbx_dom_weak::types::Vector2::new(x, y)), None)
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    match (class, key) {
        ("VideoFrame", "Playing") => {
            let on = matches!(value, Value::Boolean(true));
            video_call(lua, r, class, if on { "Play" } else { "Pause" })?;
            Ok(true)
        }
        ("VideoPlayer" | "VideoFrame", "TimePosition") => {
            if let Some(n) = crate::ltypes::num(value) {
                misc(lua)?.misc.videos.entry(r).or_default().pos = n.max(0.0);
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

pub fn write_acts(class: &str, key: &str) -> bool {
    matches!((class, key), ("VideoFrame", "Playing") | ("VideoPlayer" | "VideoFrame", "TimePosition"))
}

fn content_of(lua: &Lua, r: Ref, class: &str) -> mlua::Result<String> {
    let w = world(lua)?;
    let d = w.d();
    let c = pstr(d, r, "VideoContent");
    Ok(if c.is_empty() && class == "VideoFrame" { pstr(d, r, "Video") } else { c })
}

/// The signed-in user's voice eligibility, as Studio asks for it: GET
/// voice.roblox.com/v1/settings with the login session (read-only),
/// `isVoiceEnabled` from the reply; cached per user for the session as
/// Studio's VoiceChatService does ("returning cached value").
fn voice_enabled(lua: &Lua, uid: i64) -> Result<bool, String> {
    static CACHE: std::sync::Mutex<Option<HashMap<i64, bool>>> = std::sync::Mutex::new(None);
    if let Some(v) = CACHE.lock().ok().and_then(|c| c.as_ref()?.get(&uid).copied()) {
        return Ok(v);
    }
    let reply = outside_budget(lua, || -> Result<(u16, String), String> {
        if !crate::assets::wait_for_session(std::time::Duration::from_secs(20)) {
            return Err("HTTP 401 (Unauthorized)".into());
        }
        let shared = crate::assets::session_client().ok_or("HTTP 401 (Unauthorized)")?;
        let guard = shared.lock().map_err(|_| "account busy".to_string())?;
        let c = guard.as_ref().ok_or("HTTP 401 (Unauthorized)")?;
        c.session.get("https://voice.roblox.com/v1/settings").map_err(|e| e.to_string())
    })?;
    let (status, body) = reply;
    if status != 200 {
        let reason = match status {
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            503 => "Service Unavailable",
            _ => "Error",
        };
        return Err(format!("HTTP {status} ({reason})"));
    }
    let v: serde_json::Value = serde_json::from_str(&body)
        .map_err(|_| "VoiceChatExtension:IsVoiceEnabledForUserIdAsync() voice settings returned invalid response.".to_string())?;
    let on = v["isVoiceEnabled"]
        .as_bool()
        .ok_or("VoiceChatExtension:IsVoiceEnabledForUserIdAsync() voice settings returned invalid response.")?;
    if let Ok(mut c) = CACHE.lock() {
        c.get_or_insert_with(HashMap::new).insert(uid, on);
    }
    Ok(on)
}

/// The video, demuxed (session download through the asset cache).
fn load_video(lua: &Lua, id: &str) -> Result<(VideoInfo, Arc<Track>), String> {
    if id.trim().is_empty() {
        return Err("Invalid Asset identifier or not found".into());
    }
    let bytes = outside_budget(lua, || crate::assets::script_content_blocking(id))?;
    let t = vid::demux(bytes).ok_or_else(|| "not a WebM video".to_string())?;
    Ok((VideoInfo { len: t.len, w: t.w as f32, h: t.h as f32 }, Arc::new(t)))
}

/// VideoSampler:GetSamplesAtTimesAsync: per time, {Time, Image} (an
/// Opaque Content) or nil when out of range or undecodable (Roblox docs).
fn samples(lua: &Lua, r: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let Value::Table(times) = arg(args, 1) else {
        return Err(rt_err("Unable to cast value to Array"));
    };
    let ts: Vec<Value> = times.sequence_values::<Value>().collect::<mlua::Result<_>>()?;
    let out = lua.create_table()?;
    for (i, v) in ts.into_iter().enumerate() {
        let Some(t) = (match v {
            Value::Number(n) => Some(n),
            Value::Integer(n) => Some(n as f64),
            _ => None,
        }) else {
            continue;
        };
        let frame = {
            let mut m = misc(lua)?;
            let Some(s) = m.misc.samplers.get_mut(&r) else { continue };
            if !(0.0..=s.track.len).contains(&t) {
                continue;
            }
            if s.dec.is_none() {
                s.dec = Decoder::new(s.track.clone());
            }
            let max = s.max;
            let f = s.dec.as_mut().and_then(|d| d.frame_at(t)).map(|f| (f.t, vid::fit(f, max)));
            m.misc.sample_seq += 1;
            f.map(|f| (f, m.misc.sample_seq))
        };
        let Some(((ft, (w, h, px)), n)) = frame else { continue };
        let Some(img) = image::RgbaImage::from_raw(w, h, px) else { continue };
        let mut png = Vec::new();
        if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            continue;
        }
        let id = format!("{}video/{n}.png", crate::runtime::editable::OPAQUE_PREFIX);
        crate::assets::register_generated(&id, png);
        let e = lua.create_table()?;
        e.set("Time", ft)?;
        e.set("Image", variant_to_lua(lua, &Variant::Content(rbx_dom_weak::types::Content::from(id)), None)?)?;
        out.raw_set(i + 1, e)?;
    }
    Ok(one(Value::Table(out)))
}

fn video_call(lua: &Lua, r: Ref, class: &str, name: &str) -> mlua::Result<MultiValue> {
    let id = content_of(lua, r, class)?;
    {
        let mut m = misc(lua)?;
        let v = m.misc.videos.entry(r).or_default();
        if v.content != id {
            *v = VideoState { content: id.clone(), ..Default::default() };
        }
    }
    let frame = class == "VideoFrame";
    let playing_key = if frame { "Playing" } else { "IsPlaying" };
    match name {
        "LoadAsync" => {
            let res = load_video(lua, &id);
            let status = match res {
                Ok((info, track)) => {
                    let mut m = misc(lua)?;
                    let v = m.misc.videos.entry(r).or_default();
                    v.info = Some(info);
                    v.track = Some(track);
                    v.tried = true;
                    "Success"
                }
                Err(e) => {
                    misc(lua)?.misc.videos.entry(r).or_default().tried = true;
                    world(lua)?.output.push((Level::Error, e));
                    "Failure"
                }
            };
            Ok(one(enum_item(lua, "AssetFetchStatus", status)?))
        }
        "Unload" => {
            {
                let mut m = misc(lua)?;
                let v = m.misc.videos.entry(r).or_default();
                v.info = None;
                v.track = None;
                v.stream = None;
                v.tried = false;
                v.playing = false;
            }
            set_bool(lua, r, playing_key, false)?;
            Ok(MultiValue::new())
        }
        "Play" | "Pause" => {
            let on = name == "Play";
            if !frame && on {
                let needs = misc(lua)?.misc.videos.get(&r).is_some_and(|v| v.info.is_none() && !v.tried);
                if needs && !id.trim().is_empty() {
                    if let Ok((info, track)) = load_video(lua, &id) {
                        let mut m = misc(lua)?;
                        let v = m.misc.videos.entry(r).or_default();
                        v.info = Some(info);
                        v.track = Some(track);
                    }
                    misc(lua)?.misc.videos.entry(r).or_default().tried = true;
                }
            }
            if frame && on {
                // VideoFrame loads its video on Play; no video: nothing happens
                // (Studio fires no Played and Playing stays false).
                let needs = misc(lua)?.misc.videos.get(&r).is_some_and(|v| v.info.is_none());
                if needs {
                    if id.trim().is_empty() {
                        return Ok(MultiValue::new());
                    }
                    match load_video(lua, &id) {
                        Ok((info, track)) => {
                            {
                                let mut m = misc(lua)?;
                                let v = m.misc.videos.entry(r).or_default();
                                v.info = Some(info);
                                v.track = Some(track);
                            }
                            let v = Value::String(lua.create_string(&id)?);
                            world(lua)?.fire(r, "Loaded", vec![v]);
                        }
                        Err(e) => {
                            let mut w = world(lua)?;
                            w.output.push((Level::Warn, format!("ContentId: {id} failed due to: {e}")));
                            w.output.push((Level::Error, format!("Failed to load video {id}: Request failed")));
                            return Ok(MultiValue::new());
                        }
                    }
                }
            }
            let changed = {
                let mut m = misc(lua)?;
                let v = m.misc.videos.entry(r).or_default();
                let c = v.playing != on;
                v.playing = on;
                c
            };
            set_bool(lua, r, playing_key, on)?;
            if frame && changed {
                let v = Value::String(lua.create_string(&id)?);
                world(lua)?.fire(r, if on { "Played" } else { "Paused" }, vec![v]);
            }
            Ok(MultiValue::new())
        }
        _ => Ok(MultiValue::new()),
    }
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    // VideoFrames playing from the file or by replication (no script call).
    let nodes = misc(lua)?.nodes.clone();
    for (r, c) in &nodes {
        if c != "VideoFrame" {
            continue;
        }
        let want = pb(world(lua)?.d(), *r, "Playing", false);
        let (have, failed) = {
            let m = misc(lua)?;
            let v = m.misc.videos.get(r);
            (v.is_some_and(|v| v.playing), v.is_some_and(|v| v.info.is_none() && v.tried))
        };
        if want != have && !(want && failed) {
            video_call(lua, *r, c, if want { "Play" } else { "Pause" })?;
            if want {
                misc(lua)?.misc.videos.entry(*r).or_default().tried = true;
            }
        }
    }
    let refs: Vec<Ref> = misc(lua)?.misc.videos.keys().copied().collect();
    for r in refs {
        let (class, looped, speed) = {
            let w = world(lua)?;
            let d = w.d();
            if !d.exists(r) {
                drop(w);
                misc(lua)?.misc.videos.remove(&r);
                continue;
            }
            let class = super::class_of(d, r);
            let looped = pb(d, r, if class == "VideoFrame" { "Looped" } else { "Looping" }, false);
            let speed = if class == "VideoPlayer" { pd(d, r, "PlaybackSpeed", 1.0) } else { 1.0 };
            (class, looped, speed)
        };
        let ev = {
            let mut m = misc(lua)?;
            let Some(v) = m.misc.videos.get_mut(&r) else { continue };
            let Some(info) = v.info else { continue };
            if !v.playing || info.len <= 0.0 {
                continue;
            }
            v.pos += dt * speed;
            if v.pos >= info.len {
                if looped {
                    v.pos %= info.len;
                    Some(true)
                } else {
                    v.pos = info.len;
                    v.playing = false;
                    Some(false)
                }
            } else {
                None
            }
        };
        let frame = class == "VideoFrame";
        match ev {
            Some(true) => {
                let id = Value::String(lua.create_string(content_of(lua, r, &class)?)?);
                world(lua)?.fire(r, "DidLoop", if frame { vec![id] } else { vec![] });
            }
            Some(false) => {
                set_bool(lua, r, if frame { "Playing" } else { "IsPlaying" }, false)?;
                let id = Value::String(lua.create_string(content_of(lua, r, &class)?)?);
                world(lua)?.fire(r, if frame { "Ended" } else { "DidEnd" }, if frame { vec![id] } else { vec![] });
            }
            None => {}
        }
    }
    show_frames(lua)
}

/// Keeps every loaded video's frame current and tells the GUI which
/// texture each VideoFrame / VideoDisplay shows. Only where GUIs are drawn
/// (the client, or a server-only Run).
fn show_frames(lua: &Lua) -> mlua::Result<()> {
    let local = {
        let w = world(lua)?;
        w.side == Side::Client || !w.has_peer
    };
    if !local {
        return Ok(());
    }
    let mut keys: HashMap<Ref, Ref> = HashMap::new();
    {
        let mut m = misc(lua)?;
        for (r, v) in m.misc.videos.iter_mut() {
            let Some(track) = v.track.clone() else {
                vid::set_surface(*r, None);
                continue;
            };
            if v.stream.is_none() {
                v.stream = Stream::new(track);
            }
            if let Some(s) = &v.stream {
                s.show(v.pos);
                keys.insert(*r, s.key);
                vid::set_surface(*r, Some(s.key));
            }
        }
    }
    // VideoDisplay: the VideoPlayer wired into its Input.
    let displays: Vec<Ref> = misc(lua)?.nodes.iter().filter(|(_, c)| c == "VideoDisplay").map(|(r, _)| *r).collect();
    for d in displays {
        let src = super::wired_source(lua, d, "Input")?;
        vid::set_surface(d, src.and_then(|s| keys.get(&s).copied()));
    }
    Ok(())
}

