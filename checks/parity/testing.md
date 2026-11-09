# Testing domain: notes (2026-11-07)

Code: `src/domains/testing.rs` (tables, dispatch), `testing_service.rs`
(TestService, FunctionalTest, registration objects, StudioTestService,
TestCoordinatorService, RenderingTest), `testing_debug.rs` (legacy
debugger), `testing_studio.rs` (StudioService, DraggerService, Plugin…).
One hook in `src/play.rs` (`play_tick`): EndTest stops Play, and the editor's
snapping settings are mirrored for the Studio services.

Measured against Studio build 1708278 in an offline Studio (own instance,
`testprobe.rbxlx`), command bar (plugin security) and game Scripts in Run /
Play, plus strings in `Studio`.

## TestService (Studio behaviour reproduced)

- Output, exactly:
  - `Message(t)` → Info `TestService: t`; with a source →
    `TestService.<source.Name>(<line>): t`.
  - `Checkpoint(t)` → Info `TestService: checkpoint t`.
  - `Check(false, t)` → Error `TestService: t`; `Warn(false, t)` → Warning;
    `Error(t)` → Error; `Fail(t)` / `Require(false, t)` → Error
    `TestService: fatal t`. Passing checks print nothing. None of them throw
    (Fail / Require don't stop the script).
  - Condition nil → `Argument 1 missing or nil`; any non-nil, non-false
    condition passes (`Check(1)`). Number descriptions print as numbers; a
    table warns `argument #1 expects a string, but table was passed`.
- Counts: TestCount = Check + Warn + Require + Error + Fail calls;
  ErrorCount = failed Check / Require, Error, Fail; WarnCount = failed Warn.
  Each DataModel counts its own (client and server separately).
- `Enabled` is false in Play and Run (true only in UTP / RCC team-test).
- `Done()` prints nothing outside a test run.
- **Test run**: Run (F8) with `ExecuteWithStudioRun`:
  - no Script under TestService → Info `TestService: Doesn't include any
    assertions`, nothing else.
  - each enabled child Script: Warning `TestService macro parser is not
    supported anymore.`, then it runs; when every test script has finished
    (checked right after each one starts, so a script that ends without
    yielding completes the run before the next starts, as in Studio) or at
    `Done()` → Info `TestService: Run completed, tests: T, warns: W, errors: E`;
    after `Timeout` seconds → Error `TestService: Tests failed to complete in
    N seconds`. The session keeps running after either.
  - then the rendering phase: per RenderingTest under TestService: Info
    `Run rendering test : 'X'`, Error `Rendering test 'X' failed. Exception:
    HttpError: ConnectionClosed`, Error `TestService: fatal X`; then
    `Rendering tests finished. P of N has passed.` (Info for 0 of 0, Error
    otherwise) and, with any, Error `TestService: fatal Timeout waiting for
    alltestfinished reply from server`. Studio posts the frames to Roblox's
    internal image harness at 127.0.0.1:8001 (`submit_image_snapshot`),
    which isn't there; rustudio doesn't connect (port 8001 is a local Argon
    server on this machine) and fails each test the same way.
  - Verified: run_sync / run_timeout / run_done / run_empty places give the
    same lines in the same order as Studio.
- ServerCollectResult(text, script, line) /
  ServerCollectConditionalResult(condition, text, script, line): fire on the
  server for each client report in Play when ExecuteWithStudioRun and
  NumberOfPlayers > 0 (Studio's server + clients TestService run; Studio's
  plain Play / Run never fires them, measured). The script argument is nil
  (a client's LocalScript has no server counterpart). Not compared live:
  the offline Studio's server + clients launch from TestService.NumberOfPlayers hung.
- Registration API: FFlag `EnableTestServiceRegistrationAPI` is off in
  Studio: `RegisterSuite / ListTests / ResetTestRegistry / RunTestsAsync` →
  `TestService:X is not available. Feature flag is disabled.`;
  `RegisterTest` → its argument casts first (this build's order is
  `(name, options: Dictionary, callback: Function)`, api-dump.json says
  `(name, callback, options)`), then `TestService test registration is not
  available. Feature flag is disabled.` `TestCase:Check/Require` → argument
  checks, then the flag error. `ScopeTime` → `ScopeTime is not a valid member
  of TestService` (an explicit error in Studio's code). `isFeatureEnabled(x)`
  → `Flag x referenced from Lua isn't defined in C++ code`.
- InternalTest capability (all of TestCoordinatorService, StartTestSession,
  TakeSnapshot, Signal/Get/Set test controls, capture *Async,
  RegisterTestLegacy, TestCase Assert/EndTest/Message/RequireLegacy…) →
  `The current thread cannot call 'X' (lacking capability InternalTest)`.
- TestCase / TestResult / TestSuite / TestFailure / TestRunResult answer
  from what they carry (stored per object; Studio's defaults otherwise).
  No script can get one while registration is off.
- TestCoordinatorService: GetService gives nil in Studio (flag
  DebugEnableTestCoordinatorService off).

## FunctionalTest

Pass / Passed → a passing Check; Warn → a failing Warn; Error / Failed → a
failing Check (Error output, no "fatal"). Outside the DataModel → `Can't find
TestService`. **Gap outside this domain**: `doc::is_creatable` refuses
Deprecated classes, so `Instance.new("FunctionalTest")` fails in rustudio;
Studio creates deprecated classes (FunctionalTest, Hint, Message…). One
line in src/doc.rs (shared) — left for the lead.

## StudioTestService and the agent test run

- `GetTestArgs()` → nil in a plain Play/Run (Studio). In rustudio,
  `RUSTUDIO_TEST_ARGS` (JSON) is what Studio's `ExecutePlayModeAsync(args)`
  / `ExecuteRunModeAsync(args)` pass.
- `EndTest(value)` on the server stops the session (Studio stops Play/Run
  the same way; the calling script runs on until it yields). On a client →
  `EndTest: can only be called from the server DataModel of a running Studio
  play session`.
- `AddPlayers` → `AddPlayers: can only be called from the server DataModel of
  a running Studio test session`; `CanLeaveTest / LeaveTest` → `CanLeaveTest/
  LeaveTest: can only be called from the client DataModel of a running Studio
  test session` (rustudio has no server + clients test session).
- Headless agent run:

  ```
  RUSTUDIO_NO_AUDIO=1 RUSTUDIO_TEST_ARGS='{"n":42}' RUSTUDIO_TEST_RESULT=out.json \
    gamescope --backend headless -- rustudio place.rbxl --play \
    --screenshot /tmp/x.png --shot-after 600
  ```

  When a server script calls `EndTest(v)`, Play stops and rustudio writes
  `{"ended":"EndTest","value":v,"output":[[level,text]…]}` to out.json and
  exits 0 (≈2 s for a small place). Use `--action run@1` instead of
  `--play` for a Run.
- ExecutePlayModeAsync / ExecuteRunModeAsync are PluginSecurity (not in this
  push's Security None scope); rustudio has no plugin runtime to call them.

## Debugger (legacy, inert in Studio)

Measured in Studio: DebuggerManager is unreachable from game scripts
(`GetService` → nil, `DebuggerManager()` → lacking capability Plugin). From
the command bar: every script in the DataModel has a ScriptDebugger
`<Name>Debugger` under DebuggerManager (114 in a Run session), DebuggerAdded
fires as a script enters, DebuggerRemoved (then Parent = nil) as it leaves.
`AddDebugger` takes a script only (`Can only add debugger for a Script`),
returns the existing one. `SetBreakpoint(line)` → `Breakpoint<line>` (same
line → same object), Condition "", IsEnabled true, ContinueExecution false,
Line read-only; `AddWatch` → DebuggerWatch (Expression). Parent of either
can't change (`Cannot change Parent of type X`). Destroying fires
BreakpointRemoved / WatchRemoved. **Breakpoints never pause** (a breakpoint
on a running script's line did nothing; Studio's Luau debugger replaced this
one), so IsPaused false, CurrentLine 0, EncounteredBreak / Resuming never
fire, stack / variable calls → `Cannot perform this operation unless the
thread is paused`, `GetWatchValue(nonwatch)` → `bad watch argument`, steps
→ `Can't step while running`, Resume does nothing. IsDebugging turns true
once the script has run. rustudio reproduces all of that; real pausing would
diverge from Studio, so it isn't done. (rustudio's GetService still returns
DebuggerManager / StudioService / DraggerService / LiveSyncService /
TestCoordinatorService to game scripts where Studio gives nil: that's
runtime.rs's get_service, not this domain.)

## Studio-side services

- StudioService: GridSize = move increment when move snapping is on else 0;
  RotateIncrement likewise; StudioLocaleId en_US; DrawConstraintsOnTop
  true; DraggerSolveConstraints / ShowConstraintDetails false; ActiveScript
  nil (rustudio's script editor isn't mirrored into the play DataModel).
- DraggerService: LinearSnapEnabled/Increment, AngleSnapEnabled/Increment,
  DraggerCoordinateSpace (Object when local space) from the editor; the rest
  Studio's defaults (GeometrySnapColor 1,0,1, HoverThickness 4, ShowHover
  true…). Script writes stick on the service and don't move StudioService
  (Studio: `LinearSnapIncrement = 2` left GridSize at 1).
- Plugin.GridSize / CollisionEnabled from the editor (rustudio has no
  collisions toggle: false, Studio's default).
- Collaborator: what Team Create replicated, else defaults.
- LiveSyncService: GetService nil in Studio; HasSyncedInstances false,
  GetSyncState(i) → Enum.InstanceFileSyncStatus.NotSynced (nothing is
  file-synced in rustudio; the return shape beyond the status wasn't
  observable).
- Studio.LocalAssetsFolder: reading errors `Unable to get property
  LocalAssetsFolder, type QDir` in Studio and in rustudio (so the coverage
  probe counts it missing).
- PluginAction defaults ActionId "", AllowBinding true, StatusTip "";
  PluginGui:BindToClose keeps the callback (no plugin widget closes in
  rustudio). RenderingTest:RenderdocTriggerCapture → Error output
  `RenderDoc support is disabled.`

## Counts (my classes, Security None, coverage probe)

Before (coverage-start.json): 88 functions and 75 properties missing,
0 of 13 events fired. After: 0 functions, 1 property missing
(Studio.LocalAssetsFolder, an error in Studio too); 8 events fire
(ServerCollect×2, Breakpoint/Watch Added/Removed, DebuggerAdded/Removed),
5 never fire in Studio either (TestRegistryChanged, EncounteredBreak,
Resuming, TestFramesCountdownAboutToStart, SyncStatusChanged).
