# World domain (physics and the 3D world API)

Code: `src/domains/world.rs` (dispatch, METHODS / EVENTS, helpers) and
`world_part.rs`, `world_camera.rs`, `world_constraints.rs`,
`world_terrain.rs`, `world_misc.rs`, `world_controls.rs`; physics side in
`src/physics.rs` (`physics::live`, Rocket law, ReachedTarget detection).
Shared-file touches: `runtime.rs` (domains may answer core methods first),
`play.rs` (StoppedTouching / LocalSimulationTouched), `effects.rs`
(Trail:Clear), `runtime_ext.rs` (Camera:Interpolate's Luau shim dropped),
`api.rs` + `ltypes.rs` (enums missing from the reflection database resolve
from api-dump.json: ProjectionType, OrthographicMode, ResamplerMode…).

Probe: `checks/parity/world/server.luau` (141 checks, every expected value
measured in Studio 0.705 in an offline Studio; 141/141 pass):

    gamescope --backend headless -- env RUSTUDIO_NO_AUDIO=1 ./target/release/rustudio \
      tools/fidelity/corpus/control.rbxlx --play --screenshot /tmp/w.png --shot-after 20 \
      --luau server checks/parity/world/server.luau@3 --log /tmp/w.log
    grep WORLD /tmp/w.log

The harness runs `--luau` code on the main thread (task.wait fails there:
"error converting Lua nil to thread"), so the probe body runs in task.spawn.

## Measured in Studio (own an offline Studio instance, Edit and Run)

Mass / inertia
- GetMass = density × volume; density from CustomPhysicalProperties else
  the material (PhysicalProperties.new(material) table matches ltypes).
  4×2×6 Plastic 33.6. Ball (2,4,6): diameter = smallest side → 2.932.
  Cylinder: X long, min(Y,Z) across → 17.593. Wedge ½ box (COM
  (0,−y/6,z/6)), CornerWedge ⅓ (COM (x/8,−y/4,−z/8)). MeshPart = box.
  Each side counts at least 0.05 stud (0.001 thick plate weighs 0.035).
- Massless: 0 unless the part is its assembly's root (alone it keeps mass).
- SpecificGravity = density. CurrentPhysicalProperties as above.
  ResizeIncrement 1 (TrussPart 2). ResizeableFaces: all six (any shape).
  ReceiveAge 0.
- AngularAccelerationToTorque(α, ω) = I·α + ω×(I·ω) (box inertia
  m/12(y²+z²)…): (1,2,3),(1,2,3) on 4×2×6 → (−425.6, 459.2, 235.2).
  TorqueToAngularAcceleration is its inverse. Grounded → inf / 0.

Assemblies (Run)
- GetConnectedParts(): itself + parts rigidly jointed to it; (true): the
  whole rigid assembly; hinge-connected parts are not included.
- GetJoints: joints and constraints naming the part (hinge included),
  only those under Workspace (an unparented weld isn't).
- AssemblyRootPart: an anchored part is its own root even welded to other
  anchored parts; otherwise Massless=false > RootPriority >
  HumanoidRootPart > largest volume (ties: longest side; 1×1×8 beats
  2×2×2). Material/mass doesn't matter (3×1×1 plastic beats 1×1×2.9 metal).

Network ownership (Run)
- Anchored or welded to anchored: "Network Ownership API cannot be called
  on Anchored parts or parts welded to Anchored parts." (GetNetworkOwnershipAuto
  and SetNetworkOwner error; CanSetNetworkOwnership returns false, msg).
- Outside Workspace: "Can only call Network Ownership API on a part that is
  descendent of Workspace".
- SetNetworkOwner("x"): "Unable to cast value to Object"; any Instance is
  accepted. After SetNetworkOwner, GetNetworkOwnershipAuto is false;
  SetNetworkOwnershipAuto makes it true again. GetNetworkOwner nil (server).

Touch events (Run)
- StoppedTouching fires with TouchEnded, LocalSimulationTouched with
  Touched (server-simulated parts), same timing. OutfitChanged never fired.

Movers
- BodyPosition / BodyVelocity GetLastForce (and lastForce): always (0,0,0).
- BodyPosition ReachedTarget: when within 0.1 stud of Position, once per
  goal; setting Position (even to nearly the same value) re-arms it.
  BodyPosition holds its part's weight (rests ≤0.0006 stud off), and its
  approach rate is mass-independent (λ = v/d: P 1e4 D 1250 → 2.47/s;
  P×4 → 9.88; D×4 → 0.154; P 1000 D 100 → 11.1). rustudio now holds the
  weight (no m·g/P sag); the exact P/D response curve is not fitted (no
  single model fits those four runs) — left.
- RocketPropulsion: defaults MaxSpeed 30, MaxThrust 4000, MaxTorque
  (4e5,4e5,0), ThrustP 5, ThrustD 0.001, TurnP 3000, TurnD 500,
  CartoonFactor 0.7, TargetRadius 4. After Fire it flies at ~MaxSpeed in
  a straight line, turns its front to the target; inside TargetRadius it
  stops dead, hovers, fires ReachedTarget once; Abort drops it.

Constraints (attachment frames in the probe)
- Active: false right after parenting, true after one frame, false at once
  when Enabled = false, false with both ends anchored or an attachment
  missing; WeldConstraint the same (welded to an anchored part: Active).
- Hinge CurrentAngle: signed angle about Attachment0's axis from its
  secondary axis to Attachment1's (degrees). Cylindrical: Attachment0's
  frame turned by InclinationAngle about its Z (WorldRotationAxis =
  cos·X0 + sin·Y0), Attachment1's not (incl 180 → 180 − hinge angle).
  TorsionSpring CurrentAngle: angle between the secondary axes (twist about
  Y reads 0). Prismatic/Cylindrical CurrentPosition: offset along
  Attachment0's X. Rod/Rope CurrentDistance, Spring CurrentLength:
  distance between attachments.
- GetDebugAppliedForce/Torque(id): "Querying applied forces is not
  currently supported." (no id: "Argument 1 missing or nil").
- Attachment SetAxis / AlignOrientation.PrimaryAxis: new X, keep the old
  secondary where possible (else old Z); SetSecondaryAxis: keep X.
  LookAtPosition = CFrame.Position.

Camera
- DiagonalFieldOfView: tan(d/2) = tan²(h/2) + tan²(v/2) (Studio's own
  formula, no square root: 70° vertical reads 88.877 at 1×1, 1.754 at 10°).
  A non-current camera's viewport is 1×1. MaxAxisFieldOfView = max(v, h).
  Setting either writes FieldOfView (inverse formulas).
- TiltUnits(n): pitch + 10n° rounded to 10°, clamped ±81°, orbiting Focus;
  returns true. PanUnits(n): yaw −45n° rounded to 45°, re-aimed at Focus
  (n is an int: 0.5 pans a full step). GetPanSpeed/GetTiltSpeed 0.
- Interpolate: "Attempted to use interpolation with a camera mode other
  than scriptable." / "Interpolation time must be positive.";
  InterpolationFinished fires at the end; a newer Interpolate replaces a
  running one silently. Studio's path curves about the focus and its final
  Focus lands elsewhere (ends looking at focus): rustudio's path is a
  straight lerp/slerp to lookAt(cframe.Position, focus) — differs mid-way.
- OrthographicMode Vertical, OrthographicSize 300, ProjectionType
  Perspective, ViewStretch 1 (kept; the renderer doesn't draw orthographic
  yet — render.rs, not this domain).

Terrain
- Cell API on smooth voxels; mapping tables in world_terrain.rs (both
  directions measured for every CellMaterial / Enum.Material). CellBlock
  occupancies 1 / 0.5 / 0.336 / 0.668 / 0.5; orientation ignored; GetCell
  always Solid / NegZ; GetWaterCell (bool, None, NegX). Missing args report
  the last one ("Argument 3 missing or nil").
- Material slots 0…63 (0…22 the smooth materials, rest empty); errors
  "slotIndex is out of range", "Air and water slots cannot be set/reset",
  "baseMaterial is not a terrain material", "Unable to cast Material to int".
- *_beta voxel APIs: "Beta is not enabled" before any argument check.

Sensors
- AtmosphereSensor: AirDensity = Workspace.AirDensity·e^(−y/33381) (fits
  y = 51, 500, 2000 exactly); RelativeWindVelocity = GlobalWind·
  min((y/5)^0.20735, 3) − part velocity, world space, 0 below y = 0.
- FluidForceSensor:EvaluateAsync → (force, torque, centerOfPressure);
  per face moving into the air: −n·1.2·ρ·(v·n)²·area (matches 14.1859 for
  a 4×1 face at 50 stud/s, y = 500). Force/Torque/CenterOfPressure are 0
  with FluidForces at Default. BuoyancyForce/Torque: "not a valid member"
  in this Studio build (rustudio errors the same way).
- PhysicsActivitySensor can't be created by scripts (InternalTest).

Misc
- Tool Grip setters (G3D lookAt): Forward = lookAt(v, old up); Right = that
  turned so X = v; Up = lookAt(v, old right) turned so Y = v. Verified on a
  non-trivial grip to 6 digits.
- Dragger: see world_misc.rs header (Studio's first MouseMove after
  MouseDown jumps parts near the origin — a Studio bug not copied).
- DragDetector GetReferenceFrame: identity unless ReferenceInstance
  (Attachment → world CFrame; workspace → its pivot).
- ControlState / Controller: see world_controls.rs header.
- Model:ScaleTo scales about the pivot by s / GetScale(); NaN warns
  "Ignoring ScaleTo(NaN) call, check your code for divide by zero errors";
  ≤ 0: "Only positive non-zero scale values are allowed."; Add/Remove
  PersistentPlayer: "PersistentPerPlayer player not specified." even with
  a non-Player instance.
- Workspace: FilteringEnabled true, InterpolationThrottling Default,
  Replication* Automatic, GetPhysicsThrottling 100, PGSIsEnabled true.
- ArePartsTouchingOthers: overlap with any part not in the list
  (default padding 1e-6); non-part: "All objects passed to
  ArePartsTouchingOthers must be BaseParts, got a Workspace.".
- JoinToOutsiders created no joints for overlapping (not face-touching)
  parts; rustudio welds face-touching ones (All) / joinable surfaces
  (Surface).

## DragDetector drags (2026-11-08)

Code: `src/domains/world_drag.rs` (server half: styles, DragFrame, limits,
functions, responses, events), `src/play_drag.rs` (Play's mouse → client
events + server), `physics::live` drag goals (Physical). Harness:
`--ldrag X1,Y1>X2,Y2@S` (left-drag in view coordinates). Probe:
`checks/parity/world/drag_server.luau` + `drag_client.luau`, the layout
of the Studio session (ten 3-stud cubes, one per style / response).

    gamescope --backend headless -- env RUSTUDIO_NO_AUDIO=1 ./target/release/rustudio \
      tools/fidelity/corpus/control.rbxlx --play --screenshot /tmp/d.png --shot-after 14 \
      --luau server checks/parity/world/drag_server.luau@2 \
      --luau client checks/parity/world/drag_client.luau@4 \
      --ldrag "726,320>776,280@6" --ldrag "290,236>390,196@7.5" --log /tmp/d.log
    grep "DRAG " /tmp/d.log

Measured in Studio (own an offline Studio instance, Play, `drag` on the 3D view; the
character must be within MaxActivationDistance 32 or nothing happens —
not even hover):
- Events: DragStart(player, cursorRay, viewFrame, hitFrame, clickedPart,
  nil, isModeSwitchKeyDown) — 7 args; DragContinue(player, cursorRay,
  viewFrame, nil, isModeSwitchKeyDown) — 5, one per cursor move;
  DragEnd(player). Client first, server the same args one network trip
  later. cursorRay: origin 0.1 stud along the ray from the camera,
  direction × 2048. hitFrame: at the hit, Y = surface normal (front face
  → Euler (90,0,0); top → identity).
- DragFrame: the server resets it before DragStart and sets it before each
  DragContinue (the client reads it one event late; rustudio replicates
  within the frame, so its client is current).
- TranslatePlane (ray ∩ plane through the hit, normal Axis), TranslateLine
  (closest point on the Axis line through the hit; 2.071 computed vs
  2.067), TranslateViewPlane (plane through the hit facing the camera:
  (3.287,1.07,−0.78) vs (3.291,1.075,−0.776)), RotateAxis (signed angle
  about the Axis line through the pivot: 98.0°/124.8° vs 98°/125.1°),
  RotateTrackball (shortest arc on the sphere about the pivot through the
  hit: 51.7° vs 52.0°; off the sphere Studio's rule is unknown — the
  hyperbolic sheet used there is 15–25° off). PlaneOrLine + LeftControl →
  line, isModeSwitchKeyDown true.
- Scriptable: the style function gets the cursor ray (length 2048) on the
  server, also for the press (DragFrame already moved at DragStart); with
  no function the server fires only DragStart / DragEnd.
- AddConstraintFunction: called for the press too, lowest priority first;
  the returned CFrame is the DragFrame. Min/MaxDragTranslation clamp per
  axis (±1 → (1,1,−1)).
- PermissionPolicy Scriptable returning false: the server fires nothing,
  the client still fires all three events (rustudio: same).
- Geometric unanchored: Anchored = true before the server's DragStart,
  restored before its DragEnd. Physical (unanchored): pulled from the grab
  point — the part yaws (≈12°), stays on the floor, ≈1.2 stud/s toward a
  0.64-stud goal, stops dead on release; rustudio's spring (rate
  Responsiveness, MaxForce, floor carries the weight) matches the shape,
  not the exact speed. Custom: DragFrame only.
- Defaults: MaxForce 1e7, MaxTorque 1e4, Responsiveness 10, Orientation
  (0,180,90) → Axis +Y, SecondaryAxis +X, Trackball factors 1.

rustudio's probe reproduces every case above (event arity and order,
DragFrame formulas, style/constraint/permission functions, limits, anchor
restore, physical response on the floor).

## Rendering: wireframes, beam offsets, orthographic cameras (2026-11-08)

Screenshots (headless, client `--luau`, `RUSTUDIO_NO_GUI=1`) against
Studio's viewport shots of the same scenes: `wireframe.luau`,
`beam_client.luau`, `ortho_client.luau` in checks/parity/world/.

- WireframeHandleAdornment (Studio, edit mode): drawn only with an Adornee
  (none: nothing, even AlwaysOnTop); points at Adornee.CFrame × CFrame ×
  (Scale ⊙ p) (checked by projecting candidates: rotation, CFrame offset
  and Scale all apply); solid 1-px unlit lines in Color3 (pure (0,253,0)
  for green), depth-tested unless AlwaysOnTop; Thickness 5 looks the
  same; AddText at CFrame × p — the Adornee is ignored for text —
  monospace, baseline-left on the point; Clear removes everything;
  AddLines drops an odd last point. rustudio: lines through the line
  pipeline (alpha flags: no grid fade / nearest depth for AlwaysOnTop),
  text by the viewport painter (egui's monospace is a little larger).
- Beam:SetTextureOffset (explosion.png, Stretch, TextureLength 1): 0.25
  moves the texture a quarter of the beam toward Attachment0 (the rest
  wraps in at Attachment1), 0.5 cuts it into halves. rustudio sets the
  scroll phase to −offset at the call (TextureSpeed carries on).
- Orthographic: Studio gates ProjectionType / OrthographicSize /
  OrthographicMode / ViewStretch behind the feature flag
  `SupportOrthoCameraApi` ( → check,
  flag byte,). the offline Studio's offline
  flags leave it off: every write is ignored silently (no Changed). With
  the flag on (a second own Studio started with it in
  ClientAppSettings.json, the file restored at once): OrthographicSize
  is the view height (Vertical), width (Horizontal), diagonal or larger
  side (MaxAxis), clamped [0.001, 100000]; ViewStretch 2 halves the
  horizontal scale; ViewportPointToRay starts on the camera plane along
  LookVector; WorldToViewportPoint's Z is the depth as in perspective;
  nothing behind the camera plane draws and there is no sky (black).
  rustudio follows the flag-on Studio (live Roblox ships the API): the
  probe gives extent 20.000, depths 12.497 / 15.073 and the ray direction
  (0, −0.25766, −0.96624) as Studio, and the same image (black below).

## Network ownership hand-off, client physics readouts (2026-11-08)

Studio (Play, server script logging GetNetworkOwner of loose 1-stud
parts at distances from the character, every 0.1 s):
- A loose part goes to the player once it is within the player's
  SimulationRadius of the character; the radius starts at 0 at join and
  grows to MaximumSimulationRadius (1000): 30 studs ≈ 5–7 s after joining,
  50 at 16.7 s (another run 27.5 s), 60 ≈ 20.6 s, 100 at 30.8 s, 120 at
  35.2 s, 1000 by 152 s. The growth adapts to the client's connection
  (runs differ, ownership even falls back to the server for a moment);
  rustudio grows it a steady 4 studs/s and gives each loose assembly to
  the closest player within its radius (world_part.rs `auto_owner`).
- SimulationRadius isn't readable from a LocalScript (the command bar
  reads it: 102.5 … 1000).
- The client reads AssemblyAngularVelocity, GetVelocityAtPosition and
  GetNumAwakeParts from the replicated simulation: rustudio mirrors the
  server's physics stats to the client document each frame
  (physics::live::mirror), so both sides read the same (13.39 rad/s on
  both after ApplyAngularImpulse(0, 50, 0) on a 2-stud cube).
- An angular impulse on a part made the same frame is kept until its body
  exists (as Studio applies it).
Probe: `checks/parity/world/net_server.luau` / `net_client.luau`.

## Skateboards (2026-11-08)

Studio 0.744 (own an offline Studio, Play): a SkateboardPlatform free in Workspace,
stood on by the character: Output "Error mounting skateboard platform.
Both Humanoid and SkateboardPlatform must be direct children of a Model";
ControllingHumanoid = the Humanoid, the Humanoid PlatformStanding
(PlatformStand true), Controller nil, no Equipped / MoveStateChanged /
AxisChanged; W / A move nothing, Space doesn't get off. A platform inside
the character model, walked onto: Studio crashed (twice, two fresh
Studios). The mount makes the controller object, sets the
humanoid's state and fires Equipped(humanoid, controller) when both are
children of one Model (else the error above). rustudio: the same mount
(server step), Equipped / equipped with a SkateboardController in the
same-model case; Steer / Throttle stay 0 and AxisChanged /
MoveStateChanged / Unequipped don't fire — Studio crashes before any of
that can be measured. Probe: `checks/parity/world/skate_server.luau`.

## BodyPosition, Interpolate, RocketPropulsion from Studio's code (2026-11-08)

- BodyPosition: (its per-step check) compares the squared gap
  with 0.01 → ReachedTarget at 0.1 stud (as implemented). The force law
  itself runs in the physics solver behind the managed-joint upgrade
  (flag BodyMoverManagedJointUpgrade) and wasn't found, so it is fitted to
  Studio's runs instead: a first-order, mass-independent approach at
  λ = 386·P / max(D, 196)², weight held, force capped per axis by
  MaxForce. Probe `checks/parity/world/bodyposition_server.luau`, gap at
  0.25 s, Studio → rustudio: base (P 1e4, D 1250) 5.155 → 5.359 (rate
  2.47 → 2.58/s), P×4 1.656 → 0.581 (Studio engages later), D×4 rate
  0.154 → 0.15, P 1e3 D 100 0.587 → 0.548, heavy cube at the default
  MaxForce falls 13.79 / 24.26 / 42.79 → 13.35 / 23.37 / 40.08.
- Camera:Interpolate's curve and RocketPropulsion's thrust / turn law:   not located in Studio's code; they stay   the approximations described above.

## Left, with reasons

- PVInstance.Origin / "Pivot Offset", Workspace.SignalBehavior,
  FluidForceSensor.BuoyancyForce/Torque: NotScriptable or absent in Studio
  — reading them errors "not a valid member" there and in rustudio, so the
  coverage probe can't count them.
- TerrainIterate/Modify/WriteOperation CommitBlock / GetBlock / Ready:
  only reachable through the *_beta APIs, which Studio refuses.
- LodDataEntity.EntityLodEnabled: can't be created in Studio to measure.
- SkateboardController Steer / Throttle / AxisChanged, SkateboardPlatform
  MoveStateChanged / Unequipped: Studio 0.744 crashes on the successful
  mount, so riding can't be measured (Steer / Throttle read 0).
- RotateTrackball off the sphere: Studio's rule isn't known (15–25° off).
- Camera:Interpolate's curved path and RocketPropulsion's thrust / turn   law: approximations.
- Physical DragDetector response: same shape as Studio (yaw, floor, stop
  on release), speed not fitted exactly.
- Ownership radius growth: Studio adapts it to the connection; rustudio
  grows it 4 studs/s.
- Workspace.PersistentLoaded, Model persistent players: streaming only;
  Studio (no streaming) never fires it.
