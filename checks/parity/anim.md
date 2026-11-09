# anim domain — notes (parity push 2026-11-07)

Code: `src/domains/anim.rs` (dispatch, Humanoid state API, Motor stepping,
JointInstance.Active, Animator values), `anim_lib.luau` (curves, keyframes,
providers, Animator/AnimationTrack extras, node definitions, Humanoid
helpers), `anim_ik.rs` (IKControl), `anim_types.rs` (RotationCurveKey,
ValueCurveKey), `src/client_joints.rs` (client posing of server rigs).
Playback itself stays in `library.luau` (AnimationTrack section); this push
added hooks there: `__rs_anim_after` (IK after the tracks), `__rs_anim_drive`,
`__rs_anim_register` / `__rs_anim_registered`, RotationCurve channels, and the
FloatCurve mode byte mask.

All measurements: offline Studio 0.746 (own instance, `animprobe.rbxlx`,
command bar + Run/Play), 2026-11-07.

## Measured Studio behaviour

### Humanoid
- `AddStatus / RemoveStatus / HasStatus / AddCustomStatus /
  RemoveCustomStatus / HasCustomStatus` → `false` every time, `GetStatuses`
  → `{}`; `StatusAdded/Removed`, `CustomStatusAdded/Removed` never fire
  (parentless and parented-in-Workspace Humanoids alike). A spawned
  character's Humanoid does carry a `Status` child.
- `SetStateEnabled` fires `StateEnabledChanged(state, enabled)` on **every**
  call, including one that changes nothing (fired synchronously in Studio;
  rustudio queues engine events).
- `GetAppliedDescription` → a fresh clone each call (`d1 ~= d2`, Parent nil)
  of the HumanoidDescription the Humanoid keeps as a child; a Humanoid without
  one gets one (class defaults, e.g. HeadColor 0,0,0) on first call.
- `GetRelativeVelocityAtFloor` → `0,0,0` at rest.
- `ReplaceBodyPartR15(LeftHand, part)` on an R15 rig: `true`; the new part
  keeps its own name, is parented to the character, the old part is
  destroyed with everything under it (WrapTarget, grip attachment,
  OriginalSize); joints are rebuilt on the rig attachments. A part without
  the needed rig attachment → `false`, nothing changes. Non-R15 → `false`.
  `RootPart` destroyed the HumanoidRootPart and raised "Humanoid has no
  HumanoidRootPart." (rustudio answers false instead of copying that).
- **AvatarJointUpgrade** (implemented 2026-11-08, commit 2e5b46e):
  `StarterPlayer.AvatarJointUpgrade` (RobloxScript-only; saved as the token
  `AvatarJointUpgrade_SerializedRollout`, RolloutState Default 0 / Disabled 1
  / Enabled 2; no corpus place saves it). Default means on: Studio 0.746 and
  live `clientsettingscdn` PCStudioApp + PCDesktopClient have
  `FFlagAvatarJointUpgradeDefaultOn = True` (plus `…OnlyDestroyRigMotor6Ds`,
  `…IgnoreStarterCharacter`, `…R15Plus`, `…ReplaceBodyPartR15`,
  `…BuildRigFromAttachments`, `…CreateCharacter` all True). Measured:
  spawned R15 characters, BuildRigFromAttachments and ReplaceBodyPartR15 make
  each rig joint an `AnimationConstraint` (IsKinematic true, MaxForce 0,
  MaxTorque 3000, Attachment0/1 = the two `<Joint>RigAttachment`s, Part0 /
  Part1 / C0 / C1 derived from them, parented to Part1) plus a
  `BallSocketConstraint` `<Joint>BallSocket` (none for Root) with per-joint
  limits (UpperAngle / Twist: wrist 30 −10..10, elbow 20 5..120, shoulder
  110 −85..85, ankle 10 −10..10, knee 5 −120..−5, waist 20 −40..20, hip 40
  −5..80, neck 45 −40..40; Restitution 0, Radius 0.15); the shoulders' ball
  socket uses `JointRotation` child attachments (rotated, see rig.rs). An R6
  character keeps its Motor6Ds; Disabled keeps Motor6D R15s. On death
  (BreakJointsOnDeath) the AnimationConstraints go and the ball sockets are
  disabled. MaxFrictionTorque depends on the body's masses (44.99 vs 47.94 on
  the shoulder of two bodies); rustudio uses Studio's values for its default
  spawned body — the friction only acts on a simulated ball socket, and
  these are kinematic while alive and disabled when dead.

### Animator / AnimationTrack / controllers
- `EvaluationThrottled` false, `RootMotion` identity, `RootMotionWeight` 0.
- `GetTrackByAnimationId(id)`: nil for a loaded track that never played;
  the track once played (still after Stop).
- `StepAnimations(dt)`, `ApplyJointVelocities({})` → nil;
  `RegisterEvaluationParallelCallback` → "RegisterEvaluationParallelCallback
  is not enabled!".
- `AnimationController:GetPlayingAnimationTracks()` → its Animator's list
  (`{}` with no Animator).
- AnimationTrack `GetParameter` nil until `SetParameter`, then the value;
  `GetParameterDefaults` `{}`; `GetTargetNames / GetTargetInstance /
  SetTargetInstance` → "AnimationTrack:X is not available yet!".

### Curves
- FloatCurve wire (`ValuesAndTimes`, as Studio saves it): `u32 2, u32 n,
  n × {u16 mode | flags<<8, f32 value, f32 left, f32 right}, u32 1, u32 n,
  n × i32 ticks (1/2400 s)`; flags 1 = left tangent set, 2 = right tangent
  set (nil vs 0 survives). MarkerCurve: `u32 2, u32 n, n NUL strings, u32 1,
  u32 n, n × i32 ticks`.
- `InsertKey` → `{inserted, zeroBasedIndex}` (`{false, i}` replaces the key
  at the same time); `GetKeyIndicesAtTime` → `{0,0}` empty, `{1,1}` one key,
  else the segment `{i, i+1}` clamped to the first/last; `GetValueAtTime`
  nil when empty, clamps outside; Hermite with tangents in value/s (0.25 /
  0.1875 / 0.9375 checks). `RemoveKeyAtIndex(start, count=1)` → removed
  count; `start < 1` "Invalid argument provided: startingIndex.", `count < 0`
  "Invalid argument provided: count.". `SetKeys` sorts, keeps the first of
  equal times, non-keys → "Variant cast failed". Index errors "FloatCurve
  index is out of range". A left tangent after a non-cubic segment is
  dropped with the warning "Key at index %zu defines a left-tangent, but the
  previous segment isn't cubic, therefore this tangent is ignored." (sic).
- RotationCurve: same API with RotationCurveKey (rotation only, tangents
  default 0); Linear = slerp, Cubic = slerp by smoothstep; `SetKeys` with any
  non-empty array → "Variant cast failed" (Studio bug, copied). Saved form
  (ValuesAndTimes, plain like the others): `u32 1, u32 n, n × {u8 mode |
  flags << 2 (4 left / 8 right tangent; Studio sets both), f32 quaternion
  x y z w, f32 left, f32 right}, u32 1, u32 n, n × i32 ticks`. Read from
  files and written back on script edits (commit 9171f9a). (An earlier note
  here called it a "compressed stream": that was my base64 decode swallowing
  the XML `<![CDATA[` marker, which shifted every byte by 2 bits.)
- ValueCurve: `ValueType` "nil", InsertKey/InsertKeyValue `{}`, SetKeys 0 —
  inert for scripts. CompositeValueCurve: no component curves for any
  CurveType, value nil (flag `CompositeValueCurveEnabled` off).
- Euler/Vector3Curve `X()/Y()/Z()` create the axis FloatCurve when missing;
  `GetAnglesAtTime` / `GetValueAtTime` return `{x?, y?, z?}` with holes.
- MarkerCurve `InsertMarkerAtTime` like InsertKey; `GetMarkers` →
  `{{Time, Value}}`.

### Keyframes and providers
- `AddPose / AddMarker / AddKeyframe / AddSubPose` parent anything (a Part
  too); `Remove*` unparents only a child; `GetPoses / GetSubPoses` list Pose
  children only (not NumberPose), `GetMarkers` KeyframeMarkers,
  `GetKeyframes` Keyframes. `KeyframeSequence.Length` = last Keyframe.Time;
  empty CurveAnimation 0.
- `AnimationClipProvider:RegisterAnimationClip` → a content hash in Studio
  (rustudio: `rbxtemp://clip/n`); `RegisterActiveAnimationClip` →
  `active://N`; bad arg → "Invalid AnimationClip instance passed to
  registerAnimationClip". `GetAnimationClipAsync(registered)` → the clip;
  failing asset → "Animation failed to load, assetId: https://assetdelivery.
  roblox.com/v1/asset?id=N&serverplaceid=0". `GetClipEvaluatorAsync` →
  "AnimationClipProvider:GetClipEvaluatorAsync is not currently enabled".
- `GetAnimations(Async)(userId)` (both providers): `userId <= 0` →
  "X:GetAnimations userId is not a positive number!"; otherwise Studio calls
  `GET inventory.roblox.com/v2/users/{id}/inventory/24?limit=50&cursor=` and
  returns an InventoryPages.

### AnimationNodeDefinition / AnimationRigData / trackers
- Pins: ordered list, duplicates allowed; `InputPinsChanged` fires on every
  Add/Remove/Set call even when nothing changed.
- `LoadFromModel`: false for nil / non-Model / a Model whose Humanoid has no
  HumanoidRootPart; true otherwise (empty Model too).
- `RTAnimationTracker` can't be created by game scripts (RobloxScript); its
  properties default Active false, SessionName "", TrackerMode/Type None.

### Motor / joints
- `Motor`: CurrentAngle moves at 60 × MaxVelocity rad/s (0.025 per 1/240 s
  at 0.1); Part1 = Part0 · C0 · Rz(angle) · C1⁻¹. **Motor6D's
  DesiredAngle does nothing** (CurrentAngle stays 0 at MaxVelocity 100).
- `JointInstance.Active`: true only with both parts, Enabled, in Workspace.
- `Motor6D.ChildName / ParentName`: NotScriptable — "ChildName is not a valid
  member of Motor6D …" in Studio, so the coverage probe keeps counting them
  missing; rustudio raises the same error.

### IKControl (R15 right arm, see anim_ik.rs header)
- GetChainCount 2, GetChainLength 1.6431, node frames as Part0·C0·Transform,
  node indices 1…count+1, others identity; raw / smoothed identity before the
  first solve.
- Raw final target = target with the effector's rotation, carried to the
  last node; smoothing = SmoothDamp with ω = 2/SmoothTime (0.428 after the
  first 0.0375 s frame, measured).
- Unreachable: wrist at 0.9913 of full reach; reachable: effector exact.
- The solve starts from the current Transforms (no track resetting them:
  disabling / weight 0 freezes the last pose).

## rustudio-side changes worth knowing
- **Server rigs on the client**: `client_joints.rs` now poses every
  server-owned Humanoid / AnimationController rig from its replicated
  Motor6D.Transform each client frame. Before, only the local character was
  posed and server NPCs (Place A enemies) showed their rest pose.
- SetStateEnabled / GetStateEnabled / BuildRigFromAttachments moved from
  runtime_ext.rs into the domain.

### GetAnimations(Async) (commit 862688c)
- InventoryPages through the avatar domain's Pages; each page is the reply's
  `data` entries as their JSON fields, next page by `nextPageCursor`; a
  private inventory raises "HTTP 403 (Forbidden)". The signed-in account's
  own animation inventory is empty, so the field names in a non-empty page
  are the API's (assetId, name, assetType, created), not checked against
  Studio (offline Studio can't serve the inventory).

### ApplyAvatarRulesAsync effects (server; a client call on a server-made
Humanoid errors "Humanoid::ApplyAvatarRulesAsync() on a server-created
Humanoid can only be called on the server")
- AvatarBodyRules: ScaleMode CustomScale → Height/Width/Head/Depth = target
  height / 5.457 (CustomWorldHeight 5.5 → 1.0078301, 7 → 1.2826929);
  BuildMode CustomBuild → each scale clamped into its CustomBuild* range with
  "Avatar rules clamped BodyWidthScale from 1.0078301 to 1 (allowed range
  [0.7, 1])" per clamp; both applied through ApplyDescription.
- AvatarCollisionRules: SingleCollider → Part "CollisionPart"
  (SingleColliderSize, Transparency 1, Massless, Weld to the root at +0.765 Y,
  CanTouch/CanQuery false), every body part CanCollide false and Massless
  (root not massless); HitAndTouchDetectionMode UseCollider → the collider
  touches/queries and the body parts don't; Default → collider removed, root
  + UpperTorso + LowerTorso collide, the rest don't; Legacy → the head too.
- AvatarAnimationRules: no change to the Animate script or the description
  in Studio (overrides set, StandardR15 mode) — rustudio does nothing either.
- AvatarAccessoryRules / AvatarClothingRules: not measured yet (they work on
  accessories / layered clothing, which offline Studio can't load).

### PlayEmote / ApplyAvatarRulesAsync
- `PlayEmote(Async)(name)`: "No emote exists with name X" unless the
  Humanoid's HumanoidDescription has that emote (equipped or not); then
  "Character has no Animate script" without `Animate`; `false` without its
  `PlayEmote` BindableFunction; otherwise it invokes it with the emote's
  Animation (nil when the asset can't load) and returns true only for `true`.
- `ApplyAvatarRulesAsync`: nil → "Argument 1 missing or nil"; non-Instance →
  "Unable to cast value to Object"; other class → "Argument 1 is invalid:
  expected 'AvatarRules' instance type"; Humanoid outside the DataModel →
  "AvatarSettings service not found"; no sub-rule children →
  "Humanoid::ApplyAvatarRulesAsync() got invalid avatarRules: AvatarRules
  carries no sub-rules children, so it would apply nothing"; with a
  sub-rule (Animation/Body/Collision/Accessory/Clothing) it succeeds and,
  at their defaults, changes nothing on a standard rig.
- `AnimationGraphDefinition.Length` (the coverage probe's AnimationClip) 0.

## Live check: Place A live (Team Create join, read-only, headless)
Client probe (`--luau client`, picks Map 3, readies, watches
`Workspace.ZombiesClient`): enemies' Animators play `rise` (Length 1.60,
Priority Action4, weight fading 0.83 → 1, TimePosition advancing) and every
limb moves relative to the anchored root, 22/22 limbs for two enemies.
**Before this push the same run showed every track at Length 0**: the
library loaded animation assets through `InsertService:LoadAsset`, which the
assets_cloud domain turned into a server-only, yielding web worker that
fails on live with "Invalid file header" (Place A's server Factory logs that warning
for every LoadAsset too — not fixed here, assets_cloud's code). The Animator
now decodes prefetched animation assets natively (`__rs_anim_asset`), on
either side, as Studio's Animator does.

## Left, with reasons
- Humanoid `ApplyDescription*` and `ApplyDescriptionFinished`: done by the
  avatar domain (agreed split; GetAppliedDescription reads the child it
  keeps).
- `ApplyAvatarRulesAsync`: accessory and clothing sub-rules not applied
  (unmeasured: need loadable accessories).
- Humanoid.Touched is wired in play.rs (limb ↔ part, both sides) but has
  no dedicated test yet.
- AnimationStreamTrack: NotCreatable and only made by
  `Animator:LoadStreamAnimation` (face tracking); values are the defaults.
- Motor6D ChildName / ParentName: Studio itself refuses them.
