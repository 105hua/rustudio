# Data domain parity (2026-11-07)

Current status (2026-11-08): login-session DataStore, OrderedDataStore, Studio-scope MemoryStore and CSM Messaging backends have passed real headless rustudio Play round-trips against the explicitly authorized scratch universe 10000000007. Local MemoryStore quotas are implemented and tested. Credentials are never logged. Earlier sections below record superseded findings chronologically; final sections contain the current validation and remaining limitations.

## Direct Studio observations

Created hidden an offline Studio slot 2 against `/tmp/zdead/empty.rbxlx`
(PlaceId 0), used only command-bar probes, and closed that owned instance.

- DataStoreService:GetDataStore and ListDataStoresAsync throw exactly
  `You must publish this place to the web to access DataStore.`
- MemoryStoreService:GetQueue/GetSortedMap/GetHashMap throw exactly
  `You must publish this place to use MemoryStoreService.`
- GetDistributedCounter throws `MemoryStoreDistributedCounter is not enabled.`
- QueueService:GetStandardQueue throws `QueueService is not enabled.`
- MessagingService:SubscribeAsync in edit mode throws
  `MessagingService can only be executed by game server`. This was **not** a
  server Play probe, so it does not establish the Play-mode error.
- SetMetadata and GetMetadata on DataStoreSetOptions and
  DataStoreIncrementOptions deep-copy nested tables in both directions.
  Initial metadata is an empty dictionary. SetMetadata returns no values.
  Scalars throw `Unable to cast double/string/bool to Dictionary`;
  an array throws `Unable to cast Array to Dictionary`.
- DataStoreOptions:SetExperimentalFeatures accepts an unknown dictionary
  (`{foo=true}`), returns no values, and rejects scalar arguments with the
  same cast errors. No known experimental feature was established.

A bounded `strings` inspection of the Studio executable found:
`/v2/persistence/{}/datastores/objects/object`,
`/v2/persistence/{}/datastores`, `isStudioAccessToApisAllowed`,
`StudioAccessToApisNotAllowed`,
`403: Cannot write to DataStore from studio if API access is not enabled.`,
and `502: API Services rejected request with error. Error code: 8 Reason: The request rate exceeds the allowed maximum for the key.`
These are static string observations, **not** verified request schemas or
endpoint hostnames. The bounded request-log search found no persistence
traffic. No endpoint protocol is inferred from these strings.

## Changes

- Option dictionary methods and isolated nested metadata copies.
- Generic finite Pages registration in `data::register_pages(lua, ref, pages)`
  for other domains. Empty results normalize to one empty page, current-page
  reads copy records, and advancing past the last page errors. The Luau
  AdvanceToNextPageAsync wrapper yields with task.wait before advancing.
  This is a finite fetched-result helper, not an HTTP cursor implementation.
  Unregistered pages are deliberately not reported as successful empty results.
- Moved legacy in-memory DataStores from library.luau into data_local.rs.
  They now require explicit `RUSTUDIO_LOCAL_DATASTORE=1`, including when
  running unpublished test places. Normal unpublished mode keeps the observed
  Studio error. Published places report the explicitly rustudio-specific
  `rustudio: session DataStore backend is not implemented` error rather than
  silently accepting writes into memory. This error is **not** Studio parity.
  Local mode remains a development helper, not a claim of DataStore parity.

## Remaining

All authenticated backend work remains: access-flag fetch, session-authorized
request headers and hosts, JSON/form envelopes, budgets and throttle queues,
limits, optimistic UpdateAsync retry semantics, versions, listing cursors,
MemoryStore expiry/invisibility, Messaging subscriptions, PlayerData,
Config snapshots, and QueueService. The Studio probes above do not establish
these protocols or Play-mode disabled-service behavior. Implementing fabricated
responses or guessed live write requests would violate the parity/safety rules.
No account module was added because no authenticated protocol was verified.

Pages cursor/streaming fetch integration remains with each backend producer;
no producer currently calls register_pages in this patch. Properties on
DataStore record objects remain unimplemented because no real records exist.
Events remain unimplemented; no fake firing was registered.

## Measurement and validation

The assigned gap file starts with 71 missing functions, 27 missing properties,
and 4 events to check. The API dump declares 74 game-callable functions, 29
readable properties, and 4 events across the assigned classes. Seven formerly
missing function implementations were added (five option methods, two Pages
methods). IsFinished now works on registered real Pages but remains a gap for
unregistered probe objects. Thus declaration-based functions are 3 -> 10 / 74;
properties remain 2 -> 2 / 29 in the generic coverage probe; events 0 -> 0 / 4
behaviorally verified. Full release coverage should be run by the lead after
merging the shared checkout; these are scoped source counts, not a fresh release
coverage run.

Focused tests cover nested copy isolation, scalar/array dictionary rejection,
finite page exhaustion, unpublished denial, published backend refusal, and
explicit local write/read/remove. No full self-test was run.

Per-class changed function counts (declaring class only):

| Class | Before | After |
|---|---:|---:|
| DataStoreOptions | 0/1 | 1/1 |
| DataStoreSetOptions | 0/2 | 2/2 |
| DataStoreIncrementOptions | 0/2 | 2/2 |
| Pages | 0/2 | 2/2 |

All other assigned function counts are unchanged. Pages.IsFinished has actual
behavior only for registered fetched pages and is not credited as a generic
probe coverage gain.

Additional static observations: `102: Key name exceeds the 50 character limit.`
and `Key name exceeds the 50 character limit.` are present, along with budget
flag names DataStoreFixedRequestLimit, DataStorePerPlayerRequestLimit,
DataStoreOrderedSetFixedRequestLimit, DataStoreOrderedGetFixedRequestLimit,
DataStoreListFixedRequestLimit, DataStoreGetVersionFixedRequestLimit and their
PerPlayer counterparts. Their numeric defaults and refill semantics remain
unverified. Strings `memory-store.queue:add`, `memory-store.queue:dequeue`,
`memory-store.queue:discard`, `memory-store.sorted-map:read/write`,
`memory-store.hash-map:read/write`, and `universe-messaging-service:publish`
look like permission scopes; they do not establish HTTP routes.

Validation result: initial cargo check passed before the pagination/local
migration. Final cargo test was retried after more than a minute and remains
blocked by another domain's in-progress `avatar::LUA_METHODS` reference in
src/domains/mod.rs. No other agent's files were repaired. To validate the owned
code despite that blocker, a temporary /tmp rustc test harness imports the
actual data.rs unchanged and links the existing mlua/rbx_dom_weak libraries;
only World/class lookup and Instance conversion are shimmed. Its three focused
unit tests all pass under RUSTUDIO_NO_AUDIO=1 gamescope --backend headless.
This validates the helper and local-mode behavior, not runtime dispatch or
scheduler integration. A final integrated cargo check/test remains required.

## Follow-up step 1: protocol investigation and concrete blocker

Integrated cargo check and all three data unit tests now pass after the avatar
owner repaired its constant. The temporary harness is no longer the only check.

The actual login session validates, but `Session::my_universe_ids()` returns **zero owned universes**. Thus the authorized read-only probe could not obtain any owned published universe to read/capture a successful persistence response or an API-access denial. No other person's universe was substituted. the offline Studio's request log has no captured DataStore traffic.

Recovered actual function entries and formats:

- obtains a universe configuration URL via, using
  `/v1/universes/%s/configuration`; reads the explicit boolean
  `isStudioAccessToApisAllowed` and treats missing/nonboolean as false.
  Added a session-only, mockable access-check module in rustudio-account.
- Persistence-v2 builders: version-at-time query
  `datastore={}&objectKey={}&versionCurrentTime={}`;
 increment suffix `/increment` with
  `datastore={}&objectKey={}&incrementBy={}{} [optional &lockId]`;
 `/versions` query sortOrder/startTime/endTime/maxItemsToReturn;
 `/lock` query datastore/objectKey/lockId/ttl;
 object query datastore/objectKey/optional lockId;
 `/unlock` query datastore/objectKey/lockId.
- Classic engine builders use
  `%s%s?scope=%s&key=%s&target=%s` and
  `%s%s?key=%s&scope=%s&target=%s&usn=%s`; sorted
  uses `%ssorted/list?scope=%s&key=%s&pageSize=%i&direction=%s`.
 performs URL escaping conditional on engine context.

These are distinct code paths. The response schema, current Play-test path,
write verb/body and optimistic concurrency token contract are still unresolved.
A real backend implementation is blocked on those contracts, not on compilation.
No guessed live write request or synthetic successful HTTP backend was added.

Safe unpublished server Run probes also established rate setter validation:
GetAsync and SetIncrementAsync max base 60 / per-player 40; GetSortedAsync,
ListAsync, GetVersionAsync, RemoveVersionAsync max 5/2;
SetIncrementSortedAsync max 30/5. OnUpdate and UpdateAsync reject rate-setting
with `SetRateLimitForRequestType: Does not apply to DataStoreRequestType.NAME.`
Modern Standard*/Ordered* per-player maxima are 200; their base maxima and
actual token caps/refill mapping are not yet recovered. Budgets sampled at
one instant were 580 (standard/read), 110 (ordered write/remove), 34 (list and
version), 15 (OnUpdate), and UpdateAsync 580. These are observations at an
unknown session age, not hardcoded defaults or implemented budget behavior.

## Follow-up step 2: MemoryStore

Added actual local implementations for all 21 MemoryStore methods, enabled only
with RUSTUDIO_LOCAL_DATASTORE=1. Handles are real engine Instances. Separate
namespaces exist for queues, sorted maps, hash maps and counters. Map operations
copy values, enforce expiry, and retry update transforms when the old entry
changed during the callback. Sorted maps order numeric sort keys before string
sort keys before absent sort keys, break ties by key, support exclusive bounds,
and reverse order for descending requests. Hash-map listing uses shared real
Pages registration. Queues implement descending priority, FIFO ties, expiry,
per-handle invisibility timeout, all-or-nothing reads, waiting, batched receipts,
and stale receipt protection. Counters expire and permit signed increments in
explicit local mode; the real Studio feature remains disabled.

Normal unpublished constructors throw the exact directly observed Studio
message; GetDistributedCounter throws `MemoryStoreDistributedCounter is not
enabled.`. Publication is checked before queue timeout validation, as confirmed
by GetQueue('x', -1) on the unpublished server. Published backend calls remain
blocked by the missing authorized published target and unresolved wire contract
from step 1. The code explicitly errors instead of silently using local storage.
No transport or live write has been added.

The executable supplies these exact local validation errors:
`InvalidExpirationTime: The field 'expiration' time must be between 0 and 3,888,000.`,
`InvalidRequest: Unable to convert value to json.`, and
`InvalidRequest: Unable to convert sortKey to a valid number or string.`
Other local validation messages and data size/rate quotas are not yet verified
against a published backend and are not claimed as complete Studio parity.

Ordering corroboration (official primary documentation):
https://create.roblox.com/docs/reference/engine/classes/MemoryStoreSortedMap/GetRangeAsync
and https://create.roblox.com/docs/reference/engine/classes/MemoryStoreQueue .
The local emulator follows numeric/string/absent sort-key precedence and
queue descending priority/FIFO ties described there.

Three additional focused tests pass in the actual-source temporary harness:
unpublished constructor errors; map expiry/order/exclusive bounds/deep copies/
concurrent update retry/cancel; queue ordering/all-or-nothing/invisibility/
stale receipts/expiry; and counter expiry. Integrated cargo check is being
retried while another agent adds its referenced world_constraints.rs module.

## Follow-up step 3: MessagingService

Implemented explicit local session delivery with actual engine BindableEvents
and RBXScriptConnections. Publish yields, sends `{Data=..., Sent=os.time()}`,
and each subscriber gets a separate copied payload. Connections disconnect
normally and publishing with no listeners is valid. No live publish transport
exists and no live message was sent.

Unpublished server Run directly confirms topic empty/80-character validation,
1025-byte string rejection (`MessagingService:PublishAsync(): Cannot publish,
data exceeded 1kb`), invalid UTF-8/function rejection (`MessagingService:
PublishAsync(): Cannot publish message, can only accept valid UTF-8 characters.`),
and scalar callback cast errors. 1022/1023/1024-byte string probes passed
validation and yielded indefinitely rather than producing a success. The same
happened to a valid subscription. Server/edit checking preserves the directly
observed edit error `MessagingService can only be executed by game server`.

That function creates and connects a persistent CSM client (calls, ), rather than a simple stateless REST publish. Neither the offline runtime probes nor an account with zero owned universes can establish successful subscription/authentication/renewal and message wire frames. Published CSM support is genuinely blocked on that missing contract; normal valid calls currently produce an explicitly rustudio-specific unimplemented transport error after yielding. This is not claimed as Studio backend parity.

Local unit coverage includes multiple listeners, disconnects, per-listener and
caller payload isolation, timestamp presence, 1024/1025-byte boundary, unsupported
values/UTF-8, topic validation, and the nonserver error.

## Follow-up step 4: Config, PlayerData, QueueService and events

Added ConfigService testing overrides and explicit local ConfigSnapshot objects.
Overrides are copied and compared by value. Changed overrides mark existing
snapshots Outdated and enqueue the actual UpdateAvailable engine event. Snapshot
values stay stable until Refresh; Refresh clears Outdated and fires each changed
key's actual RBXScriptSignal with its new value. Error uses the real
ConfigSnapshotErrorState.None enum. Unsupported override types log an error and
return normally, matching the Studio function-value probe. Testing overrides
are scoped to the current runtime and never sent to Roblox. Normal mode
GetConfigAsync/GetConfigForPlayerAsync yield and throw the observed
`ConfigService: Failed to load config`; a missing player errors before loading.
Studio gives the exact nonserver message
`ConfigService is only available in the game server.`

Official primary sources corroborating override/refresh/event behavior:
https://create.roblox.com/docs/reference/engine/classes/ConfigSnapshot and
https://create.roblox.com/docs/production/configs .

PlayerDataService:GetRecordConfig (including the omitted/default record name)
throws the exact server-observed
`PlayerDataService:GetRecordConfig(string? recordName) is not yet enabled!`.
QueueService:GetStandardQueue throws `QueueService is not enabled.`; missing
name throws `Argument 1 missing or nil` before the feature check. Both gates
are now implemented rather than successful stubs.

Genuine feature blockers: the actual Studio runtime refuses Instance.new for
PlayerData, PlayerDataRecord, PlayerDataRecordConfig and StandardQueue. The only
record acquisition service is disabled. Player:GetData is also disabled (its
matching error is implemented by the players_chat owner). Therefore no actual
PlayerData/record/config object or StandardQueue can be obtained to observe
value/flush/release/default behavior or Changed/Loaded/Flushed event order. The
three record events are not faked or credited as firing. Their payload shapes
are known from api-dump.json, but implementing a made-up state machine would
violate parity. ConfigSnapshot.UpdateAvailable is implemented and fires in the
explicit local mode; backend-driven updates remain blocked by the missing
published config target/transport.

An additional actual-source local test verifies snapshot copy isolation,
Outdated/version behavior, UpdateAvailable firing, delayed key signals on
Refresh, idempotent refresh/override, clearing values, unsupported-value logging,
and the exact disabled-service and missing-argument errors. Eight focused data
unit tests now pass in the temporary harness. Integrated checks continue to
retry as shared world modules are completed.

## Final follow-up status

Commits in requested order: session access check/protocol investigation
`1b01fdf`; MemoryStore `472a018`; Messaging `098eb03`; Config and disabled
service gates `3965674`. The original groundwork is `ab41085`.

The login has zero personal universes **and zero saved universe/place
bookmarks**. Both were checked without logging credentials. No foreign universe
was substituted. Studio CSM connections never completed in the offline Run.
The owned hidden Studio was stopped and closed; the REA target was closed.

Further static recovery: selects the `gamepersistence` host;  selects `develop`. The persistence-v2 path is therefore on the former host, not apis.roblox.com. No speculative request was sent. The budget getter is actually; the first string hit was only its telemetry counter name. Its assembly returns clamped nonnegative integer token counts, UpdateAsync takes the minimum of read/write pools, OnUpdate calls , and modern request types have distinct pools at offsets 0x218..0x234. No sampled number was turned into a fake budget.

Current scoped source counts (not a fresh release coverage run): functions
**3 -> 42 / 74** with real implementations (32 added methods use explicit
local mode for their successful path; disabled gates still work normally).
Generic probe properties remain **2 -> 2 / 29** because it creates unregistered
objects; real producers additionally implement Pages.IsFinished and
ConfigSnapshot.Error/Outdated (**5 / 29** on produced objects). Verified events
**0 -> 1 / 4**, with UpdateAvailable firing in explicit local mode. These counts
do not imply authenticated backend parity.

| Class | Functions before -> now |
|---|---|
| DataStoreService | 3 -> 3 / 6 |
| GlobalDataStore | 0 -> 0 / 7 |
| DataStore | 0 -> 0 / 5 |
| OrderedDataStore | 0 -> 0 / 1 |
| DataStoreKeyInfo | 0 -> 0 / 2 |
| DataStoreOptions | 0 -> 1 / 1 |
| DataStoreSetOptions | 0 -> 2 / 2 |
| DataStoreIncrementOptions | 0 -> 2 / 2 |
| Pages | 0 -> 2 / 2 |
| MemoryStoreService | 0 -> 4 / 4 |
| MemoryStoreQueue | 0 -> 4 / 4 |
| MemoryStoreSortedMap | 0 -> 6 / 6 |
| MemoryStoreHashMap | 0 -> 5 / 5 |
| MemoryStoreDistributedCounter | 0 -> 2 / 2 |
| MessagingService | 0 -> 2 / 2 |
| PlayerDataService | 0 -> 1 / 1 |
| PlayerData | 0 -> 0 / 2 |
| PlayerDataRecord | 0 -> 0 / 7 |
| PlayerDataRecordConfig | 0 -> 0 / 2 |
| ConfigService | 0 -> 4 / 4 |
| ConfigSnapshot | 0 -> 3 / 3 |
| QueueService | 0 -> 1 / 1 |
| StandardQueue | 0 -> 0 / 3 |

Validation: the final shared checkout passed cargo check after the owners added
their world modules. The full focused data test invocation now executes eight
passing tests in the actual compiled application crate. A ninth test exercises
real runtime dispatch, scheduler yielding, Instance handles, Pages and event
connections together. It currently fails before entering any data code because
Runtime::new installs avatar_aes.luau, whose line 19 indexes `game` before
bind_globals defines it (`rustudio.avatar:19: attempt to index nil with
'GetService'`). No avatar file was changed. This integration test remains as a
repro for that independent startup defect; it must be rerun after its owner
fixes initialization order. The three account access-check mock tests pass.

Remaining 32 declared function gaps consist of DataStore backend/budgets/
versioning/metadata (18), PlayerData/record/config (11), and StandardQueue (3).
Remaining runtime record properties depend on those unresolved real record
producers. Backend MemoryStore, Messaging and Config success paths also remain
blocked despite their implemented explicit local paths. MemoryStore local size
and rate quotas are incomplete. The backend wire contracts and disabled feature
rollout are the concrete blockers; no credential or write approval is requested.

## Scratch authorization follow-up (2026-11-08): real persistence

The lead supplied and authorized writes solely to private scratch universe
10000000007 / root place 10000000000008, owned by the login account 10000000001.
The earlier zero-universe blocker is resolved. No place version was published.
All live writes in this follow-up target scratch-prefixed stores in that
universe; every other universe remains untouched.

Captured actual Studio Play traffic after SetPlaceId/SetUniverseId in an owned
hidden local Studio. an offline Studio is offline; a temporary, scratch-only route mirrors
the access flag that was independently verified true live. A temporary filesystem
relay used the existing login Session for scratch persistence calls (no cookie
or token was forwarded/logged). Other hosts/universes were not changed. Request
captures contain method/path, noncredential headers and scratch test values.

Actual required transport is gamepersistence.roblox.com, with Studio user agent,
Roblox-Place-Id, Roblox-Universe-Id, Requester=Client, PlayerCount, and
rbx-use-universe-rate-limiting=True. The same session request without context
gets 403/Not allowed; with context it succeeds. POST values are raw JSON with
Content-MD5 (base64 MD5), Roblox-Object-Userids and Roblox-Object-Attributes.
Update uses quoted If-Match versions, or If-None-Match: * for missing keys;
412 conflicts cause a new read and callback retry. Ordered updates use legacy
&usn= compare-and-swap; 409 conflicts retry. No Open Cloud credential is used.

Implemented real standard Get/Set/Update/Increment/Remove; version get, get at
time, list and remove; key/store listing with opaque lazy cursors; legacy
ordered Get/Set/Update/Increment/Remove and sorted listing. Standard object paths
are /v2/persistence/{universe}/datastores/objects/object; /increment and /versions
are suffixes, and listing uses /datastores or /datastores/objects. Object keys
include the scope prefix (global/key). Ordered paths are /v1/persistence/sorted,
/increment, /remove and /list, selecting the universe via context headers.
Legacy increment replies {value:"43",usn:"43"}; sorted listing replies entries
with target/value/usn and lastEvaluatedKey. Standard listing keys are **strings**,
not row objects. Empty pages can have a nonempty continuation cursor.

Standard GET/DELETE/increment return raw JSON plus Roblox-Object-Version-Id,
Roblox-Object-Created-Time, Roblox-Object-Version-Created-Time,
Roblox-Object-Attributes and Roblox-Object-Userids headers. Set replies with
version/deleted/contentLength/createdTime/objectCreatedTime JSON. These produce
real DataStoreKeyInfo/Key/Info/ObjectVersionInfo Instances and populated readonly
properties. API error codes/reasons are mapped to Studio's 502 message, missing
keys/versions return nil, and retryable failures honor AutomaticRetry with the
observed five-retry limit and Retry-After. Explicit unpublished mode still errors;
explicit local mode remains isolated. Access is checked over the login session
before writes. The harness RUSTUDIO_PLACE_CONTEXT now resolves its universe and
binds the real Play DataModel ids before scripts start, without publishing.

Studio GetFVariable and controlled budget probes establish legacy initial pools
100 (read/write), 50 (ordered write), 10 (sorted/list/version/remove-version),
base/per-player rates 60/40, 30/5 and 5/2 respectively, a one-second job,
100-times-rate cap multiplier, and 30-entry throttle queues. Actual requests
charge legacy plus modern pools; versions use the version pool plus standard
read, lists use list plus standard list. Update budget is min(read,write).
OnUpdate budget is (30+5*players)*30/60 minus refetch work (none yet here).
Rates can be reduced, modern limits cap at base 10000/per-player 200, and
repeated changes warn. Implemented single-precision token arithmetic, refill,
rate overrides, request charging and yielding queues with Studio's overflow
messages. Source: getter assembly and direct before/after API probes.

Validation now includes a **real rustudio release Play run** under gamescope
headless, with RUSTUDIO_PLACE_CONTEXT=10000000000008. The harness script runs
inside task.spawn because yielding APIs need a scheduler coroutine. Its output
contains `SCRATCH ROUNDTRIP PASSED rustudio_rt_1794126895`. It exercised Set,
cached and uncached Get, conditional Update, metadata/userIds, current-version
Get, GetVersionAtTime, ListVersions, ListKeys across empty cursor pages,
ListDataStores, Increment, Remove, RemoveVersion and OrderedDataStore sorted
round trips. Evidence: /tmp/rustudio-data-play.log and
/tmp/rustudio-data-roundtrip.png. The earlier unit fixture incorrectly expected
an overwritten version to remain available; the live backend returned code 12.
The fixture now uses versions returned by the actual listing/current record.

Measured release --api-coverage currently reports functions 1469/1626,
properties 2727/2802 and events 335/410 globally (other agents contribute).
Scoped data functions are 58/74 versus 3/74 initially. The generic probe still
constructs unpopulated record objects, so it reports 2/29 data properties despite
real produced records exercising the additional implemented properties. No fake
values were added to improve that metric. Scoped event implementation remains
ConfigSnapshot.UpdateAvailable; the disabled PlayerData events remain blocked.

The focused conditional-write mock now passes: a competing first creation causes
412, the callback runs again against the replacement value, the retry sends a
quoted If-Match and preserves userIds/metadata, and the resulting value is cached.
The original ten focused runtime/local/budget tests and the three account access
mock tests pass. Measured queue messages and validation were corrected from
actual Studio probes (including `101: Key name can't be empty.`, exact 104/105
messages, and separate DataStore name/scope messages). Metadata empty dictionaries
are encoded as objects, not arrays. Record property bags only expose declared
cursor properties on KeyPages/ListingPages.

## Scratch UMS + CSM transport (2026-11-08, phase 2)

Authorization: all probes below use universe 10000000007 / place 10000000000008 only. No place version was published. Studio's UMS scope is isolated from production (`/ums-service/studio/`). Captured native calls through our own an offline Studio slot 6; temporary routes were explicitly scoped to that universe. Cookie values are never captured.

MemoryStore calls use login-session context headers and `https://apis.roblox.com/ums-service/studio/{hash-map|sorted-map|queue}/v1/`. Map item GET/POST/DELETE uses universeId/name/key query fields. POST JSON uses data, expires (milliseconds), optional sortKey; update adds expect (USN), clientApiName HashMap.Update / SortedMap.Update. A 409 reply contains the new data/USN for the next transform. Controlled offline conflicts confirmed 101 callback invocations without failure; no invented fixed retry cap. Hash listing scans /keys with count/cursor/clientApiName, then POSTs /values for returned keys. Empty nonterminal pages are legitimate. Sorted /item/range POST uses ascending/count/lowerBound/upperBound objects. Queue /add JSON uses data/priority/expires; /read uses count/allOrNothing/waitTimeout/invisibilityTimeout query milliseconds; /remove uses receipt id. Size routes return {size}. Native SetAsync returns false for creation (201), true for replacement (200). UMS error codes 0..20 were measured through an offline response fixture and are mapped explicitly; server correlationId is appended as Studio's `RequestId:`. Live validation confirmed empty/129-byte keys and >32KB value errors. DistributedCounter is disabled in this Studio build, with its observed gate retained.

The headless rustudio Play probe passed hash Set/Get/Update/List/Remove, sorted Set/Get/Update/Size/Range/Remove, and queue Add/Read/Size/Remove against the real scratch UMS backend (`/tmp/rustudio-memory-play.log`: MEMORY SCRATCH ROUNDTRIP PASSED). The first combined run used a binary predating CSM integration; subsequent startup testing caught an eager HttpService lookup. All CSM service acquisition is now lazy so domain installation can precede `game` binding.

Messaging Studio is real, not session-local: WebSocket `wss://csm.roblox.com/v1/router/?id=&universeId=...`, cookie session, `X-Roblox-ChannelType: Test`, place/universe context. Native accessKey and Roblox-Job-Signature are empty in this path. SignalR handshake is {protocol:"json",version:1} plus record separator 0x1e. Invocations are type 1, target Subscribe / Publish / Unsubscribe, monotonically increasing string invocationId. Subscribe arguments [topic,0]; Publish [topic,JSON-string-envelope,0], envelope Data/Sent/TraceId/TracingEnabled. Completions are type 3, result status Ok; deliveries are target Message with [topic,envelope]. No API/Open Cloud keys. A focused account test connected, subscribed, published scratch-only, and received the exact message back from the real backend. Engine integration uses a background socket worker, scheduler polling and real RBXScriptConnections through BindableEvent delivery. Disconnecting the final connection unsubscribes. Local mode retains isolated delivery.

Quota references: https://create.roblox.com/docs/cloud-services/memory-stores (current official limits). Native flags measured: MemoryStoreLocalThrottlingInitialBudget=30, MaxBudget=120, PerPlayerPerMinuteBudget=120, MemoryStoresUseLocalThrottlerSeconds=120, MemoryStoreEnableStudioStartupLocalThrottlingForTest=false. Game-level documented limits are 64KB + 1.2KB per user, 1000 + 120 per concurrent user request units/minute; sorted/queue 1M items/100MB; item values 32KB. These guide the explicit local emulator, not a replacement for actual backend enforcement.

Final phase-2 validation: lazy CSM acquisition passed `cargo test --release data_runtime_integration` (1 passed). Headless real Play `/tmp/rustudio-memory-play3.log` logged both MEMORY SCRATCH ROUNDTRIP PASSED and MESSAGING SCRATCH ROUNDTRIP PASSED. Temporary scratch route fixtures and relay were removed/stopped; own Studio slot 6 closed without saving.

## Explicit local MemoryStore quotas (phase 3)

Local mode now enforces the measured 32,768 serialized-JSON-byte item limit, shared 65,536-byte base universe capacity plus the documented population allowance, storage recovery on expiration/removal/shrinking overwrites, the eight-day population peak window, and 1000+120/player/minute request units with refill. Update consumes read+write units; sorted ranges and queue reads charge returned-item units, waiting queue reads charge every two seconds, and hash pages charge scanning plus returned items. Generic Pages registration accepts an optional producer callback charged before advancing, so failed quota checks preserve the current page. Creation/replacement SetAsync booleans now match measured 201/200 native behavior. Invalid key/name validation matches observed backend errors.

Measured live scratch boundary: JSON string length 32,766 (32,768 encoded bytes) succeeded; length 32,767 failed code 4. Two hash values of encoded length 32,762 succeeded; a third failed code 5, `Memory usage of 98286 bytes would exceed the quota of 65536 bytes`. Hash memory accounting excludes key bytes in this observation. Test keys were explicitly removed after capture. Sorted-map writes did not synchronously reject the same third value; backend accounting propagation/partition behavior is opaque, so the local documented global-capacity enforcement is deliberately conservative rather than claiming exact server implementation equivalence.

Focused current-source MemoryStore tests pass 5/5 (headless standalone test harness linked to the same mlua); full data-domain Runtime tests follow. The local implementation models documented quota amounts with a token bucket. Roblox does not expose its exact distributed counter/window, partition assignments or per-partition admission algorithm; those remain delegated to the real service in published mode. The documented per-partition ~30,000/min and hot-hash-key approximate limits are not invented as fixed local rules. The 1M-item / 100MB structure guard is present but is normally dominated by the smaller universe capacity.

## Final DataStore callback/batch closure

After quota work, the full focused data suite passed 13/13. With callback/batch coverage added it passed 14/14 (two explicit live-write tests ignored by default).

Native standard DataStore BatchGetAsync throws the exact JSON error string with errorType BatchGetUnsupportedOperation and errorMessage/callbackMessage BatchGetAsync is not supported., even for empty/nonstring key arrays. Native ordered BatchGetAsync accepts at most 100 string keys and POSTs /v1/persistence/sorted/multi-get. Request JSON keys contains records with key=store name, scope and target. Reply entries contains scope/target/key/value/usn; numeric value is a string. The API result maps each returned target to a record with numeric value; missing keys are omitted. Empty, too-many and nonstring-key errors were measured. A real scratch POST confirmed the schema. These POSTs are reads, charged atomically to read pools by requested key count.

OnUpdate returns an actual RBXScriptConnection. A controlled native offline fixture asynchronously delivered initial value 10, local SetAsync value 11 before SetAsync resumed, externally changed value 12 on the next fetch, and nil on RemoveAsync. Native ordered polling uses multi-get; standard polling uses persistence-v2 GET. The measured fetch interval is 30 seconds. OnUpdate budget dropped 12 to 11 on registering one new watched key and stayed 11 immediately after disconnect. Watched-key accounting is now included. The mock-protocol test covers initial fetch, update/removal delivery, disconnect, batch schema/results, read charging and the disabled standard-batch gate. Both methods also exist in explicit local mode. All temporary native fixture routes were removed, and own Studio slot 7 closed without saving.

Remaining: 14 functions on PlayerData/record/config and StandardQueue are unreachable through measured disabled native acquisition gates. The generic coverage probe constructs unregistered synthetic record instances, so it undercounts produced DataStore record properties. Config backend refresh needs a published config and unresolved transport; observed failure and local testing overrides are available. CSM live connection, delivery, disconnect and heartbeats are implemented; reconnect/resubscription after transport loss and exact non-Ok CSM retry policy remain unverified. Real services enforce their distributed quotas; exact internal windows, partition assignment and admission algorithms are not exposed by the engine artifact or HTTP contract.

## Final measured API coverage

Headless release --api-coverage: scoped functions 3 → 60/74, generic-probe properties 2 → 2/29, measured events 2/4. The starting coverage JSON does not include per-class event results, so an event before-count cannot be reconstructed from it. Current events include generic Changed plus ConfigSnapshot.UpdateAvailable; Loaded/Flushed remain inaccessible. Produced objects exercise 17/29 implemented properties in live/local tests, but no fake defaults were added for synthetic coverage objects.

| Class | Functions before → after / total | Probe properties before → after / total | Measured events / total |
|---|---|---|---|
| DataStoreService | 3 → 6 / 6 | 0 → 0 / 0 | 0 / 0 |
| GlobalDataStore | 0 → 7 / 7 | 0 → 0 / 0 | 0 / 0 |
| DataStore | 0 → 5 / 5 | 0 → 0 / 0 | 0 / 0 |
| OrderedDataStore | 0 → 1 / 1 | 0 → 0 / 0 | 0 / 0 |
| DataStoreInfo | 0 → 0 / 0 | 0 → 0 / 3 | 0 / 0 |
| DataStoreKey | 0 → 0 / 0 | 0 → 0 / 1 | 0 / 0 |
| DataStoreKeyInfo | 0 → 2 / 2 | 0 → 0 / 3 | 0 / 0 |
| DataStoreKeyPages | 0 → 0 / 0 | 0 → 0 / 1 | 0 / 0 |
| DataStoreListingPages | 0 → 0 / 0 | 0 → 0 / 1 | 0 / 0 |
| DataStoreObjectVersionInfo | 0 → 0 / 0 | 0 → 0 / 3 | 0 / 0 |
| DataStoreOptions | 0 → 1 / 1 | 1 → 1 / 1 | 0 / 0 |
| DataStoreSetOptions | 0 → 2 / 2 | 0 → 0 / 0 | 0 / 0 |
| DataStoreIncrementOptions | 0 → 2 / 2 | 0 → 0 / 0 | 0 / 0 |
| Pages | 0 → 2 / 2 | 0 → 0 / 1 | 0 / 0 |
| MemoryStoreService | 0 → 4 / 4 | 0 → 0 / 0 | 0 / 0 |
| MemoryStoreQueue | 0 → 4 / 4 | 0 → 0 / 0 | 0 / 0 |
| MemoryStoreSortedMap | 0 → 6 / 6 | 0 → 0 / 0 | 0 / 0 |
| MemoryStoreHashMap | 0 → 5 / 5 | 0 → 0 / 0 | 0 / 0 |
| MemoryStoreDistributedCounter | 0 → 2 / 2 | 0 → 0 / 0 | 0 / 0 |
| MessagingService | 0 → 2 / 2 | 0 → 0 / 0 | 0 / 0 |
| PlayerDataService | 0 → 1 / 1 | 1 → 1 / 1 | 0 / 0 |
| PlayerData | 0 → 0 / 2 | 0 → 0 / 0 | 0 / 0 |
| PlayerDataRecord | 0 → 0 / 7 | 0 → 0 / 11 | 1 / 3 |
| PlayerDataRecordConfig | 0 → 0 / 2 | 0 → 0 / 1 | 0 / 0 |
| ConfigService | 0 → 4 / 4 | 0 → 0 / 0 | 0 / 0 |
| ConfigSnapshot | 0 → 3 / 3 | 0 → 0 / 2 | 1 / 1 |
| QueueService | 0 → 1 / 1 | 0 → 0 / 0 | 0 / 0 |
| StandardQueue | 0 → 0 / 3 | 0 → 0 / 0 | 0 / 0 |

Final release Play validation: /tmp/rustudio-data-final-play.log logged SCRATCH ROUNDTRIP PASSED rustudio_rt_1794130271 with initial/update/removal OnUpdate assertions, ordered batch reads, ordered CAS update, metadata/userIds, versions/time lookup, listings and removals. Combined UMS/CSM Play previously passed in /tmp/rustudio-memory-play3.log. The focused data suite passed 14/14; three account access mocks and the real scratch CSM transport test also passed. Final cargo check passed after a transient independently owned runtime.rs parse error was fixed by its owner. No live target except the authorized scratch universe was written, and no place was published.
