# Media domain: notes (2026-11-07)

Code: `src/domains/media.rs` (wiring, graph, players, emitters/listeners,
analyzer, Sound/SoundService members), `media_dsp.rs` (block DSP, offline
unit tests), `media_misc.rs` (TTS/STT/recorder/video/voice), plus
`audio.rs` (`decode_pcm`, `graph_output`).

All measurements below were taken in an offline Studio in an offline Studio's MCP
(own instance, `media_probe.rbxlx`), mostly from the command bar; play-only
ones from an F5 client. Test tones were added to the offline Studio's local asset store
as `~/.local/share/rustudio/assets/80000000000990{01..04}.wav` (1 kHz sine 0.5,
stereo L-only sine, on-bin 984.375 Hz sine, DC 0.25).

## Pins (GetInputPins / GetOutputPins)
Sorted alphabetically, independent of Layout:
- Output only: AudioPlayer, AudioListener, AudioDeviceInput,
  AudioWindSynthesizer, AudioTextToSpeech, VideoPlayer.
- Input only: AudioEmitter, AudioAnalyzer, AudioRecorder, AudioDeviceOutput,
  AudioSpeechToText, VideoDisplay.
- AudioCompressor: Input, Sidechain → Output. (AudioGate has no Sidechain.)
- AudioChannelMixer: 12 channel pins + Input → Output; Splitter mirrored.
- GetConnectedWires(any string) never errors; unknown pin → {}.

## Wires
- Validity is decided when SourceInstance/TargetInstance/SourceName/
  TargetName is *set to a new value*; parenting never re-checks.
- Checks: output/input pin exists ("Invalid Wire Connection: \"X\" has no
  \"P\" output|input pin."), loop (source==target or target reaches source
  over connected wires: "Connecting \"S\" to \"T\" would create a loop."),
  duplicate of a connected wire ("\"S\"'s \"O\" pin is already connected to
  \"T\"'s \"I\" pin."). Errors print (Output, error level) only when the wire
  is in the DataModel at that moment. Non-audio endpoints: invalid, silent.
- Built outside the DataModel, duplicates and cycles all connect (graph is
  empty there) — so rustudio validates wires found by scanning (files,
  Clone) on pins only.
- A refused wire stays refused until one of its four properties changes.
- Connected = valid ∧ wire, source, target all descendants of game.
- WiringChanged(connected, pin, wire, other): source first, then target, on
  every change; deferred like other signals.

## AudioPlayer
Defaults: Volume 1 [0,10], PlaybackSpeed 1 [0,20], Play/LoopRegion 0..60000,
AutoLoad true, AutoPlay false. Play() with no asset sets IsPlaying (Changed)
and stays playing at 0; the asset then loads (IsReady false→true,
TimeLength) and plays. Cached assets are ready synchronously. End: IsPlaying
false *without* Changed, Ended fires, TimePosition resets later. Looping
fires Looped per wrap. Stop() keeps TimePosition; Changed(IsPlaying) fires on
every Play/Stop. Play()/Stop() return nil; Play(t)/Stop(t) schedule on
GetMixerTime and return an action id, Cancel(id) → bool.
GetWaveformAsync(range, n): point samples at a + (b-a)·i/n, range clamped to
the asset, n capped at 8192, {} before loading.

## Analyzer (calibrated)
48 kHz; WindowSize Small/Medium/Large = 256/1024/4096 samples; spectrum
N/2 bins = 2.5·|FFT(Hamming·x)|/N, channels combined as RMS; RmsLevel =
RMS over all channels' samples; PeakLevel = max |x|. No input → 0, 0, {}.
Studio's path has an unexplained ~+1.1–1.6 % gain (0.5 sine reads RMS
0.3574 not 0.3536, peak 0.508); rustudio doesn't add it.

## Channels
Mono→stereo: x/√2 on L and R (centre stays empty, also for 5.1).
Stereo→mono: (L+R)/√2. Mixer output takes the widest input.

## Emitter / Listener
- Distance modes (Enum.DistanceAttenuationMode): Custom (curve; empty =
  Inverse over 4..10000, bounds ignored), Inverse min/d, Linear, 
  LinearSquared, InverseTapered = min(Inverse, LinearSquared); 0 at ≥ max.
  Non-Custom modes ignore the curve.
- Total = emitter distance · emitter angle · listener distance · listener
  angle; empty listener curves are 1. Angles in degrees from LookVector.
- Different AudioInteractionGroup → 0. Unparented → 0.
- Saved format (Studio-saved place, 2026-11-08): BinaryString properties
  DistanceAttenuation / AngleAttenuation = one 0x00 byte, then (key, value)
  pairs as little-endian f32; an empty curve is "AA==" (the byte alone).
  e.g. {0=1,10=0.5,30=0.1} → AAAAAAAAAIA/AAAgQQAAAD8AAPBBzczMPQ==. rustudio
  keeps curves in these properties (no Changed signal; replicated), so they
  save with the place and load from Studio's files (unit tests compare the
  bytes; rbx_xml wraps the base64 in CDATA, same payload).
- Curves: ≤ 400 points ("called with too many points"), keys/values must be
  numbers ("numeric key/value expected."), distance key ≥ 0, angle key
  0..180, values clamped 0..1; non-table argument is ignored.
- Panning (fitted, see dsp::pan): equal power; pan angle 45°·(1+F(x/d)·G(d))
  with a near-field collapse below ~4 studs. Matches measured L/R to
  ~0.005 at 0.25…8 studs.

## AudioFilter (exact)
RBJ cookbook biquads at 48 kHz; 24/48 dB = 2/4 identical stages with
Q^(1/stages); Lowpass6dB = one-pole a=1-exp(-2πf/fs). GetGainAt in dB
matches Studio to 3 decimals for all 12 types (unit test).

## Other effects
Ranges/defaults are Studio's (CLAMPS/DEFAULTS tables). Their algorithms are
standard equivalents (Roblox's DSP is FMOD-derived and unpublished): chorus /
flanger modulated delays, feed-forward compressor with sidechain, peak
limiter, hysteresis gate (Threshold range), LR crossover 3-band EQ, granular
pitch shifter, FDN reverb, tremolo LFO (Duty/Shape/Skew/Square), wind noise
following Workspace.GlobalWind. EQ MidRange clamps to [200, 20000] (as
measured: 1..99999 → 200..20000).

## Sound / SoundService
- PlaybackLoudness = 1000 × RMS of the *source* around TimePosition,
  independent of Volume (DC 0.25 → 252.99; 0.5 sine → 357.44); 0 in edit
  mode / not playing.
- MaxDistance = RollOffMaxDistance; isPlaying = IsPlaying.
- Loaded fires when a newly set SoundId's audio arrives.
- GetMixerTime: Studio returns mixer seconds − 2^32 (e.g. −4294966392.5) —
  an engine quirk; rustudio returns the mixer clock in seconds.
- SetInputDevice: "The current thread cannot call 'SetInputDevice'
  (lacking capability InternalTest)".

## No-backend behaviour (copied from Studio)
- AudioTextToSpeech: LoadAsync → error "Failed to load text to speech",
  returns AssetFetchStatus.Failure (Studio also warns "Failed to load sound
  : HTTP 503"); Play/Pause flip IsPlaying; GetWaveformAsync {}.
- AudioRecorder: CanRecordAsync / GetUnrecordableInstancesAsync /
  RecordAsync → "AudioRecorder is not enabled yet"; Clear/Stop no-ops;
  GetTemporaryContent → Content none.
- AudioDeviceInput: no microphone; Set/GetUserIdAccessList keep rounded
  numbers, drop others, "Unable to cast double to Array".
- VoiceChatInternal: VoiceChatState → "VoiceChatInternal API security check
  failed"; GetVoiceChatAvailable −1, ApiVersion 0, JoinByGroupId true, the
  other pause/join calls false, GetParticipants {}.
- IsVoiceEnabledForUserIdAsync (VoiceChatService and VoiceChatInternal):
  on the server (changed by the players_chat agent, 2026-11-08, from their
  signed-in Studio measurement): a connected player's UserId → that setting,
  otherwise "… the player must be connected to the current server" (Studio
  also has "is not yet implemented on the server", which was rustudio's first
  answer); on a client, another user "… cannot access
  eligibility info for other users"; the local player: GET
  https://voice.roblox.com/v1/settings with the login session (what Studio
  requests: the offline Studio's requests.log shows `GET voice.roblox.com/v1/settings`),
  answer = `isVoiceEnabled`, cached per user ("returning cached value");
  HTTP errors as "HTTP 503 (Service Unavailable)", bad JSON as Studio's
  "voice settings returned invalid response.". Live (2026-11-08): true.
  Caveat: rustudio's Play LocalPlayer.UserId is 1, not the account's id
  (players domain), so "other user" is judged against that id.
- GetChatGroupsAsync → "VoiceChatService:GetChatGroupsAsync is disabled."
- VideoCaptureService ClearMockInput / SetMockInputAsync → InternalTest
  capability error. VideoDeviceInput can't be created by scripts.
- VideoPlayer.AutoLoadInStudio / AutoPlayInStudio are NotScriptable: Studio
  raises "is not a valid member" too (they stay "missing" in coverage).

## Video (decoded)
- `media_video.rs`: WebM/Matroska demuxer (tracks, SimpleBlock/BlockGroup
  frames, keyframes from the container or the VP8/VP9 frame header) and
  VP8/VP9 decoding through the system libvpx (dlopen `libvpx.so.12`, no
  build dependency; without it videos load but show no frames). YUV→RGBA
  BT.601 (limited / full range as tagged).
- Playing videos get a worker thread that decodes the frame for the current
  TimePosition into the EditableImage texture store
  (`editable::external_image`, a generation per frame); `gui_walk.rs` draws
  VideoFrame and VideoDisplay (the VideoPlayer wired into its Input) with
  it, so ScreenGuis and SurfaceGuis on parts show real frames.
  VideoDisplay uses VideoColor3 / VideoTransparency / VideoRect*.
- VideoFrames playing from the file or by replication (Playing set on the
  server) start on the client from the DOM value.
- Studio, measured with local test clips (`8000000000099010.webm` VP9+Opus,
  `…011.webm` VP8, ffmpeg testsrc2 320×240, 4 s): VideoPlayer:LoadAsync →
  Success, Resolution 320, 240, TimeLength 4.008 (container duration) /
  VideoFrame TimeLength 4; rustudio gives the same.
- VideoSampler (client only): CreateVideoSamplerAsync returns a
  VideoSampler (TimeLength 4.008, VideoContent = the Uri content, as Studio);
  GetSamplesAtTimesAsync per Roblox's docs: an array of {Time, Image} with
  Image an Opaque Content (here `rbxtemp://rustudio/opaque/video/N.png`,
  SourceType Opaque, Uri nil), nil for out-of-range / non-number times.
  Studio itself crashes under Wine on any GetSamplesAtTimesAsync call, so
  the sample sizes are estimates: Size Small/Medium/Large/Full = larger side
  480/960/1440/1920 (never upscaled). Rustudio's VideoSampler is an
  unparented instance (typeof "Instance"; Studio says "Object").
- Soundtrack: the WebM's Opus track is decoded with the system libopus
  (dlopen `libopus.so.0`, 48 kHz, OpusHead pre-skip dropped) and comes out
  of the VideoPlayer's Output pin at its TimePosition / Volume /
  PlaybackSpeed. Measured, VideoPlayer → AudioAnalyzer on the VP9+Opus clip
  (ffmpeg 440 Hz sine): Studio RMS 0.08783, peak 0.1251, spectrum peak bin
  10 = 0.0747, Volume 0.5 → 0.04409, paused → 0; rustudio 0.08806, 0.1255,
  bin 10 = 0.0738, 0.04407, 0.
- Not done: VideoFrame's own sound (it plays straight to the speakers with
  RollOff*; only the VideoPlayer path is wired), AV1 WebMs (dav1d is
  installed but not bound).
- Proof: `checks/parity/media-video-part.png`: a VP9 VideoFrame in a
  SurfaceGui on a part, headless Play, frames advancing.

## *Async yielding
GetWaveformAsync, LoadAsync, Record/CanRecord/GetUnrecordable…,
IsVoiceEnabledForUserIdAsync, GetChatGroupsAsync, CreateVideoSamplerAsync,
SetMockInputAsync are pre-registered in the runtime's method cache with a
wrapper that runs the call and then task.wait()s once.

## Probe
`checks/parity/media_probe.luau` (server, in Play) exercises wiring rules,
player load/Ended, analyzer levels and spectrum, GetGainAt, attenuation,
waveform and recorder errors. It uses the local test tones above
(80000000000990{03,04}); run:
`RUSTUDIO_NO_AUDIO=1 gamescope --backend headless -- rustudio PLACE --play
--from-play --luau server checks/parity/media_probe.luau@2 --screenshot
s.png --shot-after 9 --log out.txt` and grep "MP ".
Coverage after (2026-11-08): every listed member of the 39 media classes is
real except VideoPlayer.AutoLoadInStudio / AutoPlayInStudio (NotScriptable:
Studio raises "not a valid member" too), VoiceChatInternal.VoiceChatState
(Studio raises "API security check failed", reproduced) and VideoSampler
VideoContent / GetSamplesAtTimesAsync (needs a VP8/VP9 decoder).
