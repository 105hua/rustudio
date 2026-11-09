# services domain: notes (2026-11-07/08)

Code: `src/domains/services.rs` (dispatch, Stats, log capture, HTTP glue,
CustomEvent, ItemChanged / TagAdded / ServiceAdded), `services.luau`
(LogService/Logger, Instance helpers, QueryDescendants, RunService binds,
SmoothDamp, Analytics checks), `services_encoding.rs`, `services_http.rs`,
`services_reflect.rs`. Shared hooks: `Runtime::enter/leave` call
`services::on_enter/on_leave` (log capture), `World::fire` calls
`services::on_fire` for `Changed` / `Tag…` fires, `bench::section_total`
(physics step time), render.rs publishes draw calls
(`services::RENDER_DRAW_CALLS`), `report_frames` passes ScriptContext.Error a
Studio-format stack trace. `--api-coverage` now also writes
`missing_game_events_by_class`.

Oracle: Studio 0.744 under an offline Studio, own instance, a copy of the control
corpus place; members probed from the command bar (plugin) and, for game
behaviour, from a server Script in a Run (F8) test.

## Coverage, my classes (game-script members)

| | before | after |
|---|---|---|
| functions | 60 / 173 | 165 / 173 |
| properties | 37 / 75 | 71 / 75 |
| events (my classes) | 17 / 48 | 34 / 48 |
| `_unassigned` events | 60 / 101 | 86 / 101 |

## What Studio does (measured) and rustudio now does

- **LogService**: GetLogHistory = this session's lines `{message, messageType,
  timestamp}` (print = MessageOutput, Studio's blue lines and stack lines =
  MessageInfo, warn, error). MessageOut(message, type, context table) fires
  *deferred* (after the line is out), including for script errors and their
  Stack Begin/…/End lines. Info/Warn/Output/Log(type, msg, context) render
  `{key}` placeholders from the context (unknown ones stay); Log takes an
  EnumItem, its number or name. `Error` logs and then *throws a table*
  `{message, stack, template, context}` with `__tostring` = message.
  GetLogHistoryAsync(nil) → "Argument 1 missing or nil". ClearOutput clears
  Studio's window only (here: nothing to clear in the session; history stays).
- **Logger** (GetLogger): one per dotted path, name "" → "Logger name cannot
  be empty", "." → "Logger name cannot contain '.'"; FullPath "A.B";
  Logger.MessageOut fires *synchronously* on the logger and every logger above
  it, before the line is printed. Studio's Logger is a plain Object (no
  Parent, typeof "Object"); here a parentless instance of class Logger whose
  userdata (`LObj`) says typeof "Object", as for every non-Instance engine
  object (EditableImage, EditableMesh…); IsA works on them (Studio declares
  IsA on Object).
- **ScriptContext.Error(message, trace, script)**: trace is
  "Full.Name, line N\n" per frame (" - function f" when named), fires after
  the error's output lines. EnableCoverage / GetCoverageStats: "lacking
  capability PluginOrOpenCloud".
- **RunService**: BindToAnimation/BindToSimulation(fn, StepFrequency = Hz30,
  priority = 2000) → RBXScriptConnection; run inside the physics step between
  PreSimulation and PostSimulation, animation binds first, then simulation,
  each by priority; callback gets the nominal period (1/30 for Hz30). Studio
  order per frame: PreAnimation, Stepped, PreSimulation, [anim, sim binds],
  PostSimulation, Heartbeat. Bad fn: "Unable to cast double to Function".
  GetPredictionStatus: server Authoritative, edit None (client: None here,
  unmeasured). SetPredictionMode accepted, no effect (as Studio).
  FrameNumber: "lacking capability InternalTest"; RunState, IsEdit:
  "lacking capability PluginOrOpenCloud" (game scripts).
- **Stats** (engine counters via `services::perf`): ContactsCount = rapier's
  touching pairs after each step; Scene / Shadows / UI2D / UI3D draw calls
  and triangles counted where render.rs, the ScreenGui pass and world_gui.rs
  issue them, RenderCPUFrameTime = the renderer's encode time (client only:
  a server draws nothing, as Studio's Run shows 0); Data/Physics
  Send/Receive Kbps = what crosses between the session's server and client
  DataModels in the last second (property / name / parent / new-instance
  replication and remotes sized as Roblox encodes values; CFrame updates as
  physics). RenderGPUFrameTime stays 0 (no GPU timestamp queries). Also: InstanceCount / PrimitivesCount / MovingPrimitivesCount from the
  session's DataModel; FrameTime (wall time between frames), HeartbeatTime
  (step → hand-back), PhysicsStepTime (play.rs p3 section per step);
  GetTotalMemoryUsageMb = process RSS; GetMemoryUsageMbForTag: LuaHeap = Luau
  heap, Internal = RSS − heap, others 0 (Studio Run: LuaHeap 0, Internal 478 MB,
  others 0). SceneDrawcallCount on the client = renderer's last frame.
  Harmony*, GetMemoryCategoryNames, *AllCategories*, *Filtered:
  "lacking capability InternalTest". MemoryTrackingEnabled true.
- **TaskScheduler**: SchedulerDutyCycle 0, SchedulerRate 0 (Studio's values),
  ThreadPoolSize = CPU threads (Studio 16 on this machine). `settings()` itself
  is refused to game scripts in Studio ("lacking capability Plugin").
- **EncodingService**: byte-for-byte (unit tests hold Studio's outputs).
  Base64 strict (padding required, "Base64Decode: input is not a base64
  string"); hashes Blake2b-256 / Blake3 / Md5 / Sha1 / Sha256, string hash
  returns raw digest bytes; Zstd levels −7..22 ("CompressBuffer: unsupported
  compression level for Zstd"), frame with content size, no checksum;
  bad input "DecompressBuffer: decompression failed - Unspecified error
  code"; GetDecompressedBufferSize nil when unknown.
- **ReflectionService**: from api-dump.json in Studio's shapes (see header of
  services_reflect.rs). Known gaps: Studio's database has members the dump
  lacks (Part.shap, formFactorRaw…), 656 classes vs the dump's 944, and class
  capabilities (New = Basic…) aren't in the dump.
- **HttpService** (also measured in the real, signed-in Studio 0.746 against
  httpbin.org, HttpEnabled off and on): Studio sends Accept */*,
  Cache-Control no-cache, Roblox-Id (place id), a Traceparent and
  "RobloxStudio/WinInet RobloxApp/0.746.0.7461269 (GlobalDist;
  RobloxDirectDownload)" (and Accept-Encoding deflate, gzip, zstd: here gzip
  only); PostAsync's compress flag doesn't compress. Restricted headers
  (Roblox-*, User-Agent, Host, Connection, Content-Length, Accept-Encoding,
  X-Forwarded-For): 'Header "Host" is not allowed!'; non-string values:
  "Header X must have its value be a string or secret!"; non-string URL:
  "Invalid URL type, URL should be a string or secret."; non-table options:
  "Unable to cast string to Dictionary"; non-string Body: "Invalid request
  options, Body should be a string". These and the scheme / GET-body checks
  come before the HttpEnabled check. Methods are case-sensitive: "get" or
  unknown → "HttpError: Aborted". Non-2xx: "HTTP 404" over HTTP/2 (https;
  StatusMessage "") vs "HTTP 404 (Not Found)" over HTTP/1.1; rbxcdn.com is
  reachable, roblox.com isn't. Earlier: real requests (ureq, own thread, script yields) when
  HttpEnabled; off → "HTTP requests are not enabled. Enable via Experience
  Settings" after a one-time blue "Go to Experience Settings and turn on Allow
  HTTP requests.". "URL must be http", "HttpService is not allowed to access
  that Roblox resource", GET/HEAD body → "Invalid request options, Body should
  be empty for 'GET' or 'HEAD' method", non-2xx Get/Post → "HTTP 404 (Not
  Found)", transport "HttpError: ConnectFail" etc. RequestAsync →
  {Success, StatusCode, StatusMessage, Headers (lower-case), Body}.
  GetSecret → "Can't find secret with given key". Verified against a local
  server in rustudio (the offline Studio's Studio is network-isolated, so only its
  errors were comparable).
- **CollectionService**: CreateCollection "Collections are not enabled",
  GetCollection / GetCollections {}. TagAdded fires the first time a tag is
  used; Studio did *not* fire TagRemoved when the last instance lost it, nor
  ItemAdded/ItemRemoved.
- **DataModel**: GetMessage "", GetRemoteBuildMode false, IsGearTypeAllowed
  false, GearGenreSetting AllGenres, SavePlace → "Game:SavePlace placeID is not
  valid!". ItemChanged(instance, property) fires for property changes in the
  DataModel; ServiceAdded on GetService creating one.
- **Instance**: IsPropertyModified / ResetPropertyToDefault against a fresh
  instance's values, Studio's errors ("Invalid argument to …(): Class Part has
  no property named `Foo`", "… Part.ClassName is read-only"); GetStyled = the
  property's own value (no StyleSheets apply), GetStyledPropertyChangedSignal;
  QueryDescendants: class (IsA), .tag, #name, [prop], [$attr], [x = v], " " and
  ">" combinators, "," lists; "Expected at least one filter". Sandboxed
  stored. IsInSandbox / PredictionMode aren't scriptable: "not a valid member".
- **TweenService.SmoothDamp**: ported from Studio's own code (
  scalar,  vector,  CFrame, found from the "Unsupported
  type" string at ): Unity's spring in single precision, but an
  overshoot lands on the target with zero velocity; smoothTime < 1e-4 →
  (target, zero), dt < 1e-5 → (current, velocity). CFrame rotation: the
  rotation vector of cur·conj(target) (shorter arc) damps toward zero, result
  exp(v)·target; the angular velocity lives in the velocity CFrame's
  R00..R02, the rest of that matrix passed through (an identity velocity
  starts at (1, 0, 0)). 14 measured cases match within float32 rounding.
  (Studio's FFlag SmoothDampFixInitialVelocity path, off in 0.744, would read
  the velocity's rotation as a quaternion instead.)
- **Misc**: AnalyticsService sends nothing (no backend in a Play test) but
  logs "AnalyticsService: instance must be a Player object." for a non-player;
  GetDurationLoggerTimestamp InternalTest. UserSettings.IsUserFeatureEnabled →
  "Flag x does not exist"; Reset logs "Insufficient permissions to set
  ControlMode" and throws "cannot set 'camera control mode' (lacking capability
  RobloxScript)". UserGameSettings: RotationType stored, InStudioMode true,
  InFullScreen false, GetCameraYInvertValue 1, SetOnboardingCompleted →
  "Invalid onboarding ID". Selection.SelectionThickness 0.02.
  ScriptService:ResolveModulePath "is not enabled". UniqueIdLookupService
  RemoteCommand capability. CustomEvent: value clamped 0..1, receivers via
  Source (parented or not), SourceValueChanged / Receiver(Dis)Connected /
  Event(Dis)Connected. CustomLog: InternalTest. HttpRbxApiService
  RequestLimitedAsync "is disabled". SerializationService: rbxm buffers
  ("No instances provided for serialization", "Serializer::checkHeaderForXML
  can't read header"); Studio hands it to plugins only (GetService → nil in
  game scripts). Lighting.LightingChanged(false) on Lighting writes.

- Events that exist but never fire in a Play test, in Studio or here (nothing
  drives them): UserGameSettings.FullscreenChanged / StudioModeChanged,
  DataModel.ServerLifecycleChanged / ServerRestartScheduled /
  ServerLowMemoryWarning / AllowedGearTypeChanged,
  MicroProfilerService.DataChanged, CollectionService.TagRemoved.

## What's left, and why

- IsInSandbox, PredictionMode, FrameNumber, RunState, BindTo*Parallel: Studio
  errors on them from game scripts (not scriptable / capability / "not a
  valid member" for the Parallel binds in 0.744); rustudio errors the same way,
  so the probe counts them missing.
- WebSocketService / WebSocketClient, WebStreamClient (+ their events),
  LanguageService: `GetService` returns nil in this Studio for game scripts
  *and* the command bar, so there is nothing reachable to measure;
  CreateWebStreamClient gives Studio's HTTP-disabled error and, when enabled,
  an honest "not supported" error.
- (done) RunService.Misprediction: Connect/connect/Once → "cannot connect
  'Misprediction' (lacking capability PluginOrOpenCloud)", Wait → "cannot
  wait"; Rollback connects normally (Studio). The prelude's Signal checks
  `__rs_gated_events`. ConnectParallel outside an Actor ("Scripts that connect
  in parallel must be rooted under an Actor.") is the shared Signal's, not
  done.
- Selection.SelectionChanged, DataModel.GraphicsQualityChangeRequest,
  Instance.StyledPropertiesChanged (StyleSheets, gui), NetworkClient /
  NetworkMarker, TerrainReadOperation.Ready, RuntimeContentService,
  NotificationService, HopperBin, FlagStand, ImportSession, GuiButton
  .SecondaryActivated: input/UI, legacy tools or backends outside this domain.
- MicroProfilerService: no profiler in rustudio, so GetDataSize 0 (Studio
  180976).
- Object.IsA started showing as flagged in `--api-coverage` mid-push; it works
  in a real run (AccessoryDescription.IsA reads fine, no gap), so some probe-time
  stub read on a non-Instance Object subclass is counting against it; not
  traced to a cause.
