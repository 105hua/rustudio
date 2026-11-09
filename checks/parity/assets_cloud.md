# Assets/cloud parity implementation (2026-11-07)

This is a partial implementation; the full assigned domain is not closed.

## Evidence

Created hidden an offline Studio instance slot 3 on the local materials3 fixture, started
Run, and executed probes both from the command bar and from an ordinary Script.
Closed only that instance, discarding changes. No live write requests were made.

Observed Studio:
- ContentProvider.BaseUrl: `https://www.roblox.com/`.
- GetAssetFetchStatus(""): Failure; an untouched asset ID: None.
- PreloadAsync skips the empty string; a local face texture calls back with
  its content ID and Success. A missing local texture calls back with Failure.
- GetAssetFetchStatusChangedSignal(missing-local-id) fires with Failure.
  AssetFetchFailed did not fire in this probe. No speculative implementation
  of that event is advertised.
- GetBaseMaterialOverride(Plastic) initially returns `Plastic`.
  SetBaseMaterialOverride accepts a nonexistent variant name and returns that
  name on subsequent reads. Empty override returns the empty string.
  The material argument also accepts the string `Plastic`.
- Air and Water fail with `GetBaseMaterialOverride failed: Material 'Air' is
  not supported.` (substitute method and material). Set uses the same form.
- GetMaterialVariant returns nil when absent, and the matching child
  MaterialVariant when its name and BaseMaterial match.
- Ordinary Script cannot write MaterialVariant.BaseMaterial: lacking capability
  Plugin. This patch does not change property security.

CoreScript leads (not sufficient endpoint evidence to implement the API yet):
ExtraContent/scripts/CoreScripts/ServerCoreScripts/ServerBuildExperiencePlaytestTeleport.lua
calls GetGamePlacesAsync; ExtraContent/LuaPackages/Packages/_Index/MomentsCreationFlow/
MomentsCreationFlow/Thunks/SearchMusic.lua calls SearchAudio.

## Implemented

MaterialService's three methods; ContentProvider BaseUrl, RequestQueueSize,
GetAssetFetchStatus, GetAssetFetchStatusChangedSignal, and a yielding
PreloadAsync. Fetches use the existing content loader, disk cache, and login
session downloader (now preferred over anonymous delivery when signed in).
Roblox-Place-Id / set_place_context are untouched.

Preload requests run on worker threads; step drains results into status signals.
PreloadAsync waits on the task scheduler and invokes the per-content callback.
Instance preload scans descendants for common content-bearing properties.
That scan is not yet exhaustive (Content values and uncommon content properties
need additional Studio probes); callback errors and cancellation also need parity
probes. RequestQueueSize currently counts this provider's preload requests only,
not all renderer/audio requests. The queue and fetch status therefore remain
partial parity despite having real behavior.

## Remaining

All unchanged entries of assets_cloud.json remain open. AssetService and
InsertService discovery endpoints/return shapes require binary tracing and
signed-in read-only validation. CaptureService requires a renderer frame capture
hook plus Studio client/server error probes. Import metadata, PackageLink,
SurfaceAppearance and capture properties require verified defaults and import
integration. ML/generation/feature matching/scene analysis/realtime media and
remote command/cloud handles require Studio-mode availability/error evidence;
no guessed empty successes or error strings were added. All events_to_check
remain unverified beyond the ContentProvider status signal described above.

The initial domain-owned API totals computed from api-dump and coverage-start
are 135 functions, 93 properties, 12 events; 11 functions and 62 properties were
reported real at baseline. This patch adds five previously missing functions and
two previously missing properties; PreloadAsync was already incorrectly counted
real by the old no-op. Expected static counts: functions 11 -> 16, properties
62 -> 64. Events unchanged; the baseline file does not include per-class missing
events, so a verified per-domain real-event count cannot be recovered from it.
These are static deltas, not a final rebuilt coverage measurement.

## Validation

`RUSTUDIO_NO_AUDIO=1 gamescope --backend headless -- cargo test -q
assets_cloud_material_and_preload -- --nocapture`: passed (1 test). This
compiled the shared checkout after concurrent registration edits completed.
No full self-test, live write, or authenticated endpoint parity claim is made.

## Follow-up group 1: session asset reads

Recovered bundle/package implementations at  / ;
base resolver literals are `catalog` / `inventory`. Package response uses
`assetIds`. Insert legacy search at  uses
`apis.roblox.com/toolbox-service/v1/legacy-insert-service/{Model|Decal}` with
`keyword` and `pageNumber`. Removed sets/categories immediately call the
error continuation with `LuaWebService error` (/).
CreatePlaceInPlayerInventoryAsync unconditionally errors as removed in release
471 (). No live write was attempted.

GetGamePlacesAsync constructs develop's v1 universe places URL, Asc, limit 50
( / ); records map name/id to Name/PlaceId. Real pages
are fetched before StandardPages registration. Latest versions uses develop's
`/v2/assets/{id}/versions` (), taking the first data record's `Id`.
LoadAsset/LoadAssetAsync now yield on worker downloads; LoadAssetVersion uses
asset delivery's version-id query rather than incorrectly treating a version
ID as an asset ID. Authenticated fetch_bytes now preserves Roblox-Place-Id.

Audio search v2 body recovered from : read-only POST to
`toolbox-service/v2/assets:search`, searchCategoryType Audio, searchView Core,
query, sortCategory/sortDirection, optional maxPageSize/pageToken/tags, and
audioSearchFilters. The `Core` value was essential: CreatorStore gave HTTP400.
AudioPages uses real creatorStoreAssets and nextPageToken pages. Sorting
MostUsed/Trending and Title vs SearchKeyword still need Studio validation;
they are partial parity, not claimed exact. Native record mapper 
maps asset/creator fields to the exposed record, including numeric Creator.Type.

Signed-in, read-only smoke tests: bundle 1 and free-model search succeeded;
package 27112438 returned 404, latest-version asset 1818 returned 403 (permission
refusal), audio search with Core succeeded. Responses were inspected by field
names only, never logging credentials. The account mock route/session test
passed. Metadata retrieval candidate returned HTTP404: its request schema and
availability are unresolved, so it is not registered as an engine method.
GetCreatorAssetID's legacy LuaWebService base route remains unresolved despite
tracing . CreatePlace/SavePlace publish-setting gates are present in
binary but runtime validation is blocked: two attempts to create an owned hidden
Studio both failed with MaxInstanceCount. No other agent's Studio was touched.
Other asset creation methods need their actual Play-mode gate verified; they
remain gaps rather than guessed successful uploads.

## Follow-up group 2: ContentProvider

Instance preload now walks reflection properties (including inherited ContentId
and URI Content values), rather than a fixed list of common texture properties.
Object-backed Content values are already in memory and need no download. The
shared loader records canonicalized status for renderer/audio AssetStore,
public_prefetch and script content_blocking; RequestQueueSize counts pending
keys across these paths. Status signals also observe renderer/audio transitions.
Preload queues a request without yielding; PreloadAsync waits on the scheduler.

AssetFetchFailed is delivered for new cloud-asset failures from that shared
loader. Local file failures retain the prior observed status/callback-only path.
The deprecated event descriptor is at ; installed docs describe its
asset-fetch-failure semantics. Its exact client/server ordering remains blocked
by the Studio instance limit; this behavior is meaningful but not asserted exact.

Encryption native entrypoints , ,  all immediately
return when EnableLuaApiToRegisterEncryptedAssets is false. Its initializer at
 only sets name/length; the flag storage defaults false. Implemented
the disabled Studio mode (registration/unregistration do nothing, listing is
empty). Enabled-key decryption is not implemented and is not claimed. Runtime
remote flag overrides cannot be confirmed while an owned Studio cannot start.

Compilation passed after the new paths. The focused test reached another
domain's Runtime::new initialization error (`rustudio.avatar:19: attempt to
index nil with GetService`), before executing this test's script; no unrelated
code was changed to bypass it. Later verification will retry after agent edits.

## Follow-up group 3: viewport captures

CaptureScreenshot and TakeScreenshotCaptureAsync now request rustudio's own
frame through egui screenshot readback, crop to the rendered viewport, encode
PNG and register a rbxtemp content ID that the existing texture loader can draw.
The callback runs on the task scheduler. The newer callback receives
ScreenshotCaptureResult plus a ScreenshotCapture with actual CaptureTime,
LocalId, SourcePlaceId and SourceUniverseId. CaptureType is Screenshot.
CaptureBegan/Ended are queued around the request/completion. Their precise
Studio scheduling and captureParams behavior still require a runtime probe.

Gallery saving/sharing and video capture remain blocked on exact desktop
Studio behavior. Binary strings prove server-only errors for gallery prompts
and show platform storage/video capability paths; synthesizing a client result
without probing would be speculative. Source addresses: prompt errors
/590, screenshot object not-implemented gate , gallery
permission gate , upload disabled . CaptureSaved and
UserCaptureSaved are not advertised: no gallery-save action has been implemented.
No capture upload or remote write was attempted. VideoCapture.TimeLength has
no value source because no video recorder has been connected.

## Follow-up group 4: persisted metadata and materials

Material overrides now write the MaterialService `<Material>Name` property
consumed by scene.rs, and mark the document dirty through store_prop; they no
longer exist only in an isolated API table. Get reads actual persisted overrides.
SurfaceAppearance.SurfaceSlot reads/writes the document property and emits its
property changes. Multi-slot mesh rendering itself remains a renderer gap.

PackageLink and import classes expose metadata actually present in a loaded DOM
(including Content and PackagePermission values) without substituting invented
cage validation/geometry statistics. Missing import analysis is still a gap:
rustudio imports FBX into MeshParts but does not currently construct Studio's
BaseImportData/RootImportData analysis graph. Thus no source exists for IDs,
cage-check booleans, polygon totals, dimensions, or status-report events on those
objects. No fake successful cage checks or synthetic StatusReported events added.
The yielding asset loader also preserves local FBX imports, reusing the existing
FBX converter via a surgical visibility change. Compilation passed.

## Follow-up group 5: disabled services and unavailable execution contexts

Added the shipped binary's disabled-mode gates (yielding on the scheduler for
Async entrypoints):
- MLService CreateSessionAsync / MLSession ForwardAsync: `MLService is not enabled`
  ( / ); post-process load/set: `MLPostProcess is not enabled`
  ( / ). IsPostProcessReady reads false without a model
  (). EnableMLService initializer  sets the flag name only.
- Six exported SceneAnalysis methods: `SceneAnalysisService is not enabled`
  ( through ); flag initializer  defaults off.
  GetCollisionGeometryMemoryAsync is in api-dump but its name was not found in
  this executable, so its behavior cannot be asserted from this build.
- FeatureMatchingTable.GetSchema: `FeatureMatchingTable:GetSchema is not yet enabled`
  (). The enabled table decoder and schema-derived SchemaName remain
  unimplemented; the service/session class names are absent from this executable
  string inventory, though FeatureMatchingTable is present. No fake feature rows.
- GenerateModelAsync / SegmentMeshAsync / LoadGeneratedMeshAsync:
  `GenerationService is not enabled.` Native registration maps the methods to
   /  / , respectively.
  Five video-gen methods use `VideoGen service is not enabled.` The flag overrides
  in a running signed-in Studio are still unverified; these are binary-default
  modes, not evidence of account-specific rollout settings.
- RemoteCommandService's four public context methods fail with
  `<method> can only be called from within an executed command` in ordinary Play.
  Native guards check the execution context before looking up its command
  ( /  /  / ).
- Unattached AppHandle / ParticipantHandle operations report
  `<class>:<method>: owning TestCoordinatorService is no longer available`.
  AppHandle.Call is directly traced at . Valid attached handles and
  their input/error validation need a real TestCoordinator session.

Gate/error unit test passed (1 test) in headless gamescope. Compilation passed.

Remaining genuine blockers, after source/binary inspection and read attempts:
- Generation Connect/Disconnect/GenerateMesh: connection and feature rollout
  context unresolved; no generation request was submitted (generation mutates
  remote state). Their exact disabled/allowed path needs the owned Studio probe
  currently prevented by MaxInstanceCount.
- RealtimeMedia: actual WebRTC signaling/device/session lifecycle is absent;
  binary shows eligibility checks, SDP exchange, and input permission paths
  ( onward). No invented connected state or OnMessage/WiringChanged/
  Disconnected/AudioInputRequested events. Cannot assert the right connection
  refusal without a valid Studio eligibility context.
- OpenCloud*: InvokeAsync's server gate and `Open Cloud not loaded.` are traced
  (), but Studio's registration/bootstrap state and typed CreateModel
  schemas are unresolved. Notifications are live writes and were not sent.
  No API keys were used or requested.
- ExecutedRemoteCommand: rustudio has no attached remote-command executor;
  SendUpdate/RunMoreCode/Stop and ReceivedUpdate require its actual command and
  ownership state. The ordinary RemoteCommandService context error is supported,
  but manufacturing an executed handle or signaling a fake update is not.
- BaseImportData StatusReported/StatusRemoved, CaptureSaved/UserCaptureSaved,
  RealtimeMedia events and ExecutedRemoteCommand.ReceivedUpdate still have no
  verified triggering path; they have deliberately not been marked implemented.

## Final verification follow-up and recovered metadata route

Further tracing resolved metadata:  constructs a GET with repeated
`assetIds` query parameters. The previous POST candidate's 404 was a method
mismatch, not an unavailable service. Signed-in GET returned audioMetadataResults;
an audio returned by the creator-store search supplied title, artist, duration,
and audioType metadata. GetAudioMetadataAsync is now implemented with the native
30-entry limit, order/duplicate preservation, ContentId normalization and
AssetId-only rows for unavailable metadata. It performs no write.

AudioPages constructor  resolves all sort mappings: MostUsed -> Top,
Trending -> Trending, RecentlyCreated -> CreateTime, the duration sorts ->
AudioDuration with Ascending only for ShortestToLongest. SearchKeyword is used
by the v2 constructor; the older Title filter belongs to the old v1 path.
The return shape was checked against actual creatorStoreAssets structure without
logging asset content. Bundle items now include AssetType and the DynamicHead
SupportsHeadShapes flag; arbitrary raw web Product/Creator fields are not added.

ApproveAssetId and ApproveAssetVersionId bind to an empty native thunk
( / ), agreeing with installed docs that they do nothing.
Insert's implementation at  inserts into Workspace and rejects nil
with `instance must be non-nil`. Implemented all three and extended the focused
test, which passed. Direct public_cached/remote_cached callers now participate
in queue tracking too; fast worker completion cannot be overwritten by Pending.

The initial screenshot probe exposed a harness collision: any screenshot event
caused the harness to save and exit, even before its own capture was requested.
The harness now only consumes a frame after it has requested its final shot.
After correction, a signed-in headless Play probe printed capture-content-ready,
capture-object-ready and capture-parity-probe-ok. Both callbacks and two ordered
CaptureBegan/Ended pairs were observed. Visual inspection of the saved frame
confirmed the returned content ID draws the captured viewport in an ImageLabel.
The capture metadata factory path works; a coverage probe's fabricated raw
Capture object has no capture record, so its five properties remain flagged.

Remaining unresolved write APIs were not sent to Roblox. Legacy creator lookup
was traced through  into the dynamically configured LuaWebService base;
its exact runtime URL cannot be recovered from the method fragment alone, and
owned Studio attempts were blocked by MaxInstanceCount. The native REA string
searches also timed out at 300s; direct local Ghidra provided the evidence above.

## Priority regression: Place A live InsertService model/source selection

Root cause: the worker added for `LoadAsset` used any upload-ledger source
as the delivered model. Factory's decal **80366567625947** maps to its
original PNG (294,927 bytes), not a Roblox model. Passing those bytes to
`Document::decode` reproduces `Invalid file header`. Asset delivery's copy
is a valid 645-byte XML Decal model, SHA256
`33da5e0f17e7945ee6942404aae24c0163b2377f91251ceb65219f3bd7d1cbe4`.
This was source selection, not compression, a redirect body, or a missing
place header.

`assets_cloud_load.rs` now permits local Roblox XML/binary models and FBX
sources only; other source types use `assets::remote_cached`, which reads
the delivered cache or the login-session asset downloader. The downloader
continues to send the current `Roblox-Place-Id`. XML BOMs/declarations and
leading whitespace are normalized before model decoding. Decoder failures
also identify the asset ID. The regression tests embed the exact delivered
Factory asset (no credentials/network dependency), reproduce the old PNG
failure, assert the decoded Decal texture, and round-trip the real model
through XML and binary source selection.

Validation: focused tests `assets_cloud::web::load::tests`: 2 passed;
`cargo check` passed (a transient unrelated world_misc import error in a
release build was resolved by its owner). Actual signed-in **a live place Team
Create join**, universe `10000000003`, place `100000000000004`, headless,
read-only, serialized with `flock ~/.cache/rustudio/live-join.lock`:

```
RUSTUDIO_NO_AUDIO=1 flock ~/.cache/rustudio/live-join.lock \
  timeout 240s gamescope --backend headless -- /tmp/assets-cloud-live-rustudio \
  --team-create 10000000003 100000000000004 --play --from-play \
  --luau server /tmp/assets-cloud-live-load.luau@3 \
  --screenshot /tmp/assets-cloud-live-load2.png --shot-after 30 \
  --log /tmp/assets-cloud-live-load2.output.log
```

Probe runs in `task.spawn` (the harness root itself cannot yield). Results:
`LoadAsset(80366567625947)` returned Model with one Decal, Texture
`http://www.roblox.com/asset/?id=103327827611569`;
`LoadAsset(94030309151493)` returned Model with 51 descendants (the local
FBX pack). Probe reached `AC_B7_LOAD_DONE`. Factory initialized normally;
**zero `Invalid file header` warnings**. Artifacts are
`/tmp/assets-cloud-live-load2.output.log`, `.png`, and `.view.png`.
No live place edits or live writes were performed.

## Capture follow-up: observed desktop Play results

An ordinary LocalScript in owned hidden offline Studio (slot 1, Play) returned:
`StartVideoCaptureAsync(callback)` -> `Enum.VideoCaptureStartedResult.NoDeviceSupport`
(no callback); `StopVideoCapture()` -> no values; `ReadCapturesFromGalleryAsync()`
-> the single value `Enum.ReadCapturesFromGalleryResult.NeedPermission`.
These desktop paths are implemented without inventing a recording, gallery
contents, or completion events. Argument-bearing permission prompts and valid
save/share prompts remain under investigation: the owned Studio exited during
the combined screenshot/gallery probe, and its replacement encountered
`MaxInstanceCount` (replacement closed; no other agent's Studio touched).
Observed invalid-content errors were `PromptSaveCapturesToGallery: Invalid content`
and `PromptShareCapture: Invalid capture Content: invalid`; these alone do not
establish the valid-content path, so no unconditional-error shim was added.

## Corrected Play capability and enabled-feature evidence

This section supersedes the binary-default gate assumptions in the earlier
“Follow-up group 5” section. Ordinary Script/LocalScript probes in the owned
offline Studio establish the *effective* settings, rather than assuming that
an executable's zero-initialized flag is its effective value:

- ML `CreateSessionAsync`: `MLService is not enabled`. Post-process methods
  instead fail immediately with `The current thread cannot call '<method>'
  (lacking capability InternalTest)`.
- RemoteCommandService's four methods fail capability `RemoteCommand` before
  reaching the executed-command context checks. AppHandle, ParticipantHandle,
  FeatureMatchingSession, video-gen, Generation Connect/Disconnect, RealtimeMedia
  calls and the three Capture InternalTest methods similarly enforce their
  declared InternalTest capability. No backend requests occur on these paths.
- RealtimeMedia construction itself fails `The current thread cannot create
  'RealtimeMedia' (lacking capability InternalTest)`. AppHandle,
  ParticipantHandle, ExecutedRemoteCommand, OpenCloudApiV1, Capture and
  VideoCapture are NotCreatable. FeatureMatchingService is absent from the
  running executable (`'FeatureMatchingService' is not a valid Service name`).
  A new FeatureMatchingTable has SchemaName `""`; GetSchema raises the verified
  disabled-table error.
- OpenCloudService.GetApiV1: `API is not enabled`; InvokeAsync is server-only,
  then rejects an unregistered version with `Unable to find version.` No keys,
  notifications, or live writes were used to discover these results.
- GenerationService *is enabled*. SegmentMeshAsync in Play fails
  `SegmentMeshAsync can only be called from Roblox Studio, not from a running
  experience.` LoadGeneratedMeshAsync on the server fails
  `loadGeneratedMeshAsync can currently only be called from the client`; with
  no generated replicated mesh on the client: `Unable to replicate mesh`.
  The old unconditional disabled GenerateModelAsync shim was removed; it
  would have claimed parity for an enabled remote-writing operation.
- SceneAnalysisService *is enabled*. The previous six unconditional disabled
  shims were removed. `GetInstanceCompositionAsync` now counts the actual live
  DOM using the native root-service whitelist, all 85 ordered IsA category
  rules, and the native Name/Size/Children tree ( / ;
  tables  and ). A focused test adds a Part, Light and
  Folder and verifies the measured +3 delta and category subtotals.
  `GetCollisionGeometryMemoryAsync` is absent in this Studio build and reading
  it raises the native invalid-member error; GetLatestVersionUriAsync is also
  absent from this shipped executable and documentation.

Additional write guards recovered without executing writes:
`CreateAssetAsync` and `CreateAssetVersionAsync` share , whose Play
refusal is `CreateAssetAsync can only be used in the Edit datamodel`.
`PromptImportAnimationClipFromVideoAsync` uses  and its effective
DFFlag is false (StudioAppSettings, sole place override 14883099012):
`AssetService::PromptImportAnimationClipFromVideoAsync feature not enabled.`
These Play refusals are implemented. The native Edit upload path is intentionally
not called. Source-based guards and observed runtime guards are distinguished
here; no successful live write was used as a test.

Validation: `cargo check` passed; the four focused assets_cloud tests passed
with `--test-threads=1` (includes the real Factory XML/binary regression).
An initial concurrent test run hit a Luau SIGILL; the serial run passed and is
the repeatable verification command. No full self-test was run.

## Capture status: recovered read-only polling route

`CheckUploadCaptureStatusAsync` (, response ) now makes
Studio's session-authenticated GET:
`https://apis.roblox.com/content-captures-api/v1/check-upload-status?token=...`
(client), or `check-upload-status-rcc` (server). No upload/start-upload call is
made. HTTP 202 returns `(UploadPending, 0)`. HTTP 200 reads the `result` object:
nonempty operationError -> `(UploadFailed,0)`; not approved ->
`(CaptureModerated,assetId)`; approved with ID zero -> `(UploadPending,0)`;
otherwise `(Success,assetId)`. The native parse/missing-result/network/status
errors are preserved. A focused decoder test covers these branches.

Signed-in read-only smoke succeeded: client polling the non-operation token
`invalid` returned HTTP 202 / `(UploadPending,0)`; server polling returned
`CheckUploadCaptureStatusAsync: Unexpected status code`. This differs from the
offline Studio's client invalid-token HTTP error, as expected for its offline
HTTP layer; the response mapping comes from the executable, not a guessed
unconditional invalid-token error. Both routes and the existing asset read
smoke passed (2 tests). `cargo check` passed. No capture was uploaded.

## Gallery prompts and screenshot event correction (owned Studio slot 6)

After other Studios closed, a new owned hidden Studio could start. An ordinary
LocalScript captured a real Studio screenshot (`rbxtemp://171`) and verified:

- `PromptSaveCapturesToGallery({id}, callback)` and `({ScreenshotCapture}, callback)`
  return no values, show no gallery prompt in Studio Play, and keep their callbacks
  pending. `PromptShareCapture(id,"",accepted,denied)` does the same. No save/share
  was accepted; no live write occurred. The hidden viewport was inspected after
  these calls and had no gallery UI. The callbacks remained pending after further
  tests. This is the default Studio frontend path now implemented.
- Empty save list: `PromptSaveCapturesToGallery: Invalid contents`.
  Unknown ID, or Content.fromObject(capture) in the list:
  `PromptSaveCapturesToGallery: Invalid content`.
- A newly taken ScreenshotCapture has **empty LocalId**, a real CaptureTime,
  and CaptureType Screenshot. Its content is available through the screenshot
  callback, not through an invented gallery LocalId. Content.fromObject accepts
  Capture instances, although they are not valid gallery content values.
- Neither CaptureScreenshot nor TakeScreenshotCaptureAsync fires CaptureBegan,
  CaptureEnded, CaptureSaved or UserCaptureSaved in this Studio build. The earlier
  rustudio-only probe that expected two began/ended events was incorrect. Those
  artificial emissions and real-event registrations have been removed; no
  screenshot event is fabricated to inflate the event count. User gallery saves
  and supported video capture remain the unverified triggering contexts.

Headless rustudio probe `/tmp/ac-capture-studio-mode.luau`, actual release run
`/tmp/ac-capture-studio-mode2.output.log`: ID prompt and object prompt assertions
passed; `AC_CAPTURE_STUDIO_MODE_OK 0 0` confirms zero public events and callbacks.
Its ImageLabel draws the captured rustudio viewport in
`/tmp/ac-capture-studio-mode2.view.png`. `cargo check` passed. This section
supersedes the earlier screenshot-event and LocalId claims.

## Script memory: actual Luau allocations

The enabled Studio service returned `ScriptMemory` with a byte Size and
Name/Size/Children hierarchy: root service (`Chat`, `Players`, `Workspace`),
`ModuleScripts` / `Scripts`, then full script path leaves, sorted by memory.
The root includes unattributed VM overhead, so its Size exceeds the child sum.

Rustudio now measures its own Luau VM using `lua_totalbytes` and assigns Luau
allocation categories to compiled Script/LocalScript/ModuleScript code,
coroutines and signal callbacks. Requiring a module switches categories and
restores the caller across yielding pcall execution. Source text/line numbers
and script environments are preserved. Core runtime edits are limited to
compilation, coroutine creation and the require wrapper. This supplies actual
allocator measurements, not a fabricated empty tree or Windows byte constants.
Luau exposes 256 categories; category zero remains unattributed VM overhead and
scripts beyond the first 255 remain included in the root total without invented
per-script attribution. Native-created callback wrapper overhead can likewise
remain unattributed.

Focused serial domain tests passed (4 tests), including holding a new 256 KiB
string and asserting the real VM's memory increase and its script leaf, plus a
required ModuleScript returning a 10 KiB string and its distinct module leaf.
The real-asset loading regression remains included. `cargo check` passed.

## Unparented instance composition

`GetUnparentedInstancesAsync` now reports the live retained orphan instances,
using the observed `UnparentedInstances` Name/Size/Children shape, grouped by
creating script full name and then class. Weak origin metadata is recorded by
Instance.new, Clone and the shared internal factory; it does not retain Lua
userdata, and dead refs are periodically removed. Descendants of unparented
models are included, and instances with a DataModel ancestor are excluded.
Internal objects without a known Lua allocation owner use an `Unknown` group.
The existing runtime cache supplies live internal instance references as well.

Focused serial tests passed (4): creating an orphan Model/Folder/IntValue adds
exactly three entries under its actual creator; parenting the Model into
Workspace removes all three from the report. `cargo check` passed. Shared edits
are only allocation-observer calls and the internal factory's borrow boundary.

## EditableImage maps into a real Decal

Owned Studio slot 6, ordinary LocalScript: `CreateDecalAsync` is enabled and
builds a local Decal, with no upload. Its dictionary keys are **TextureContent**,
NormalMapContent, MetalnessMapContent, RoughnessMapContent (not ColorMap).
Each must be Content.fromObject(EditableImage). The returned properties retain
that very EditableImage object. URI, none and string values fail
`AssetService:CreateDecalAsync: <key> value should be an EditableImage Content`.
No maps -> `AssetService:CreateDecalAsync: No Content Maps provided`.
Unknown keys warn `AssetService:CreateDecalAsync: Unknown content parameter: <key>`.
Native helper: ; English shipped docs confirm the map keys/types.
These paths are implemented using existing editable objects and real DOM content
properties, without changing CreateEditableImage* / CreateEditableMesh*.

`CreateTextureAsync({})` and a nonempty dictionary both return
`AssetService::CreateTextureAsync is not yet enabled` in the same Studio,
regardless of CreateDecalAsync being enabled. That separate gate is implemented.
Focused serial domain tests passed (4), including a real 2x2 EditableImage,
retained object identity for two maps, invalid URI, empty maps and the Texture
gate. `cargo check` passed. No live asset was created.

## Ephemeral DataModel content

Owned Studio slot 6 verified that CreateDataModelContentAsync accepts only
Content wrapping EditableImage / EditableMesh. None and URI input fail exactly
`The input content type is not supported by AssetService::CreateDataModelContentAsync yet!`.
An empty mesh fails `Error creating FileMeshData: Empty Result`. A valid image
returns `(Enum.CreateContentResult.Success, Content{SourceType=Opaque})`.
Shipped docs confirm the tuple, supported inputs and that options currently have
no public controls.

Implemented real immutable snapshots: images are PNG-encoded on a worker;
meshes reuse the existing editable mesh encoder. Returned Opaque Content has
SourceType Opaque, nil Uri/Object and native tostring. It can be decoded again
with CreateEditableImageAsync / CreateEditableMeshAsync, and it continues to
represent the snapshot after the original is edited. Buffers are owned by this
Play DataModel and released with its VM. No publish/upload endpoint is called.
Existing CreateEditable* implementations were left in place. Dictionary checks
and new local-asset helper errors return plain Lua strings with native text;
CreateDecal dictionaries are processed in native key order.

Serial focused tests passed: opaque image round-trip after changing the original
(red remains red), nonempty mesh round-trip (3 vertices / 1 face), native error
strings, opaque metadata, and scope cleanup. Six domain tests passed including
the separate realtime access test. `cargo check` and release build passed.
The allocation test now forces a collection through a test-only helper before
its before/after snapshot, avoiding unrelated incremental-GC reclamation in
its memory-growth assertion; production diagnostics do not force a collection.

## RealtimeMedia class access, including loaded files

Loading a local XML fixture through a new owned hidden Studio (slot 1, PID 1700)
creates the RealtimeMedia in the DataModel, bypassing Instance.new. Even the
plugin command bar cannot retrieve it with Workspace:FindFirstChild:
`The current thread cannot access 'RealtimeMedia' (lacking capability InternalTest)`.
LoadLocalAsset was separately rejected for lacking capability RobloxScript.
Thus normal Play scripts cannot obtain this object to inspect its connection
properties or subscribe to its four events; the boundary precedes those reads.

Rustudio now enforces that class boundary for loaded files, cloned/internal
objects and worker-loaded asset models, as well as the already observed
Instance.new creation boundary. It does not synthesize connection state, SDP,
messages, audio-input requests, or disconnected events. A focused loaded-fixture
Script test passes. All six serial domain tests and cargo check passed.
The only shared changes are the Lua instance exposure guard and initial-world
scan. Existing allocation observers supply subsequent object scans.

The offline Studio MCP then became unreachable (connection refused on 28100 and 28101),
and the owned launch PID had already exited. No other Studio or broker was
closed by this agent. Further native-only checks must wait for that external
service to return; rustudio's headless verification remains available.

## Final measured state after the priority fix and follow-up

Actual headless `--api-coverage /tmp/assets-cloud-after-final.json` completed
with the rebuilt release binary; log `/tmp/assets-cloud-after-final.log`.
For these assigned classes (135 functions, 93 properties, 12 events):
**functions 11 -> 108; properties 62 -> 67; verified event producers 0 -> 1**.
The baseline JSON predates per-class event reporting, so the event baseline
is established from source before 8ef8900: none of these 12 had a producer
in the domain/runtime/play/asset paths. AssetFetchFailed is now implemented.
Earlier intermediate counts in this document are superseded by this run.
Coverage records member lookup, not exhaustive behavioural equivalence.

The stricter, native-observed RealtimeMedia class-access boundary makes its
six methods unavailable to the probe. They are deliberately not counted as
real, even though their method-level capability errors are implemented.
The probe now protects fixture lookup with pcall, recording inaccessible
targets instead of aborting the entire coverage run. Abstract Capture's five
properties remain probe gaps, while actual ScreenshotCapture objects have
the measured metadata implemented.

Final checks: cargo check and release build passed; six focused domain tests
passed serially under headless gamescope; capture-status decoder test passed;
two explicitly selected read-only login-session smoke tests passed. No full
self-test was run. Own-frame screenshot/gallery probe passed with a drawable
content ID (`/tmp/ac-capture-studio-mode2.output.log`, `.png`, `.view.png`).

The priority LoadAsset fix was also repeated against a live place after all runtime
changes, using the documented flock/headless join and the same real-asset
assertions. `/tmp/assets-cloud-live-final.output.log` contains both LOAD_OK
markers, the expected Decal texture, LOAD_DONE, and Factory assets ready in
1.04 seconds; zero Invalid file header errors. Artifacts include `.png`,
`.view.png` and `.process.log`. This repeat additionally sets
`RUSTUDIO_LOCAL_DATASTORE=1`: other agents have since added live persistence,
and this confines game Play persistence to memory while retaining the real
signed-in Team Create join and asset reads. No live edits or writes.

### Remaining function gaps: attempted paths and concrete reasons

- AssetService ComposeDecalAsync: valid native ColorMap input succeeds, clears
  the public maps and produces a private TexturePackContent protected by
  RobloxEngine. Rustudio lacks that texture-pack compositor/renderer path;
  returning success after clearing maps would lose the image. GetCreatorAssetID:
  deprecated native bridge builds `GetCreatorAssetID?creationID=N`; its dynamic
  base is unresolved and a positive native call never completed. No guessed
  substitute endpoint. GetLatestVersionUriAsync: absent from this executable;
  native invalid-member behaviour remains a measured coverage gap.
- AssetService CreatePlaceAsync, SavePlaceAsync, PromptCreatePlatformContentAsync:
  unpublished/client guards were investigated, but valid published paths can
  create/save/publish remotely. No write was attempted, and an unconditional
  disabled error would misrepresent API-enabled Studio. Further native probing
  is additionally blocked by an offline Studio connection refusal.
- CaptureService PromptCaptureGalleryPermissionAsync: native valid prompt stayed
  pending without Studio gallery UI; no fabricated permission result.
  StartUploadCaptureAsync and UploadCaptureAsync: real upload writes, with no
  saved gallery capture available in the tested desktop Studio environment.
  Upload status polling is implemented separately and tested read-only.
- GenerationService GenerateMeshAsync and GenerateModelAsync: enabled native
  server paths create remote generation jobs/assets. Client guards and argument
  failures were observed; no remote generation submission was made.
- SceneAnalysisService GetAnimationMemoryAsync, GetAudioMemoryAsync and
  GetTriangleCompositionAsync: native trees were inspected and Rust caches/
  renderer sources searched. Actual allocation and per-DataModel/pass provenance
  are not exported to this domain. Disk sizes and aggregate draw counts would
  be false substitutes. GetCollisionGeometryMemoryAsync is absent from this
  executable and gives native invalid-member behaviour.
- FeatureMatchingService CreateTable: service absent in this running Studio
  build; ordinary GetService fails. No fabricated feature data.
- RealtimeMedia six methods: loaded-file and Instance.new probes both hit
  InternalTest before scripts can obtain the object. Exact class guards are
  implemented; no manufactured RTC connection/session.
- OpenCloudApiV1 CreateModel/CreateUserNotificationAsync: GetApiV1 is disabled
  and the handle is NotCreatable; notification creation also writes remotely.
  No API keys used. ExecutedRemoteCommand RunMoreCode/SendUpdate/Stop:
  NotCreatable executor handle, with RemoteCommand capability denied by the
  service in ordinary Play. No attached executor context to act on.

The 26 property gaps are Capture abstract fixture (5), PackageLink metadata not
present in the loaded document (5), import analysis data requiring an actual
editor import graph (13), and inaccessible RealtimeMedia (3). Stored metadata
is returned when present; cage validation and import geometry are not invented.
SurfaceSlot is stored and signals changes, but multi-slot rendering still needs
renderer integration.

The other 11 events were checked, not registered as empty promises: desktop
screenshot calls emitted none of the four Capture events in native Studio;
gallery save/video contexts were unavailable. Import StatusRemoved/StatusReported
need an import-analysis producer. RealtimeMedia's four events require the denied
internal class/session. ReceivedUpdate requires an attached remote executor.

Commits for this continuation, in order: dbb927b (priority LoadAsset fix),
6fabc96, 28e18e0, 94b4335, a354af5, 2b475a8, 4b6ee99, 31b89ac, 5a284a0,
3b64150; the final coverage-fixture protection and these notes are committed
separately. Earlier domain implementation commits: 8ef8900, c73431f, 2415fea,
a42ac7d, e078fa6, c0ecd05, e528dbb. The later native Play-capability findings
explicitly supersede incorrect disabled-service assumptions in c0ecd05.
