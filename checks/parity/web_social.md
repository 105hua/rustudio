# web_social parity evidence (2026-11-08, final handoff)

Scope: native Studio Play behavior. Live probes use login-session reads only;
notification POSTs are tested against localhost mocks. No purchase,
award, teleport, invitation, message, moderation or backend write was performed.
Baseline direct members: functions 13/127, properties 14/24; 18 events require
firing audit (registration alone is not proof).

## Sources

an offline Studio exe and packed ExtraContent/models/DataModelPatch/DataModelPatch.rbxm. PurchasePrompt source is packed here, not absent. Main evidence: Utils.isMockingPurchases, Network.getBalanceInfo, Thunks.completePurchase, Thunks.completeRequest, Services.Network, resolvePromptState and English locale.

Online signed-in Vinegar Studio 0.746.0.7461269, unpublished Place1 (PlaceId=0,
GameId=0). Only this unpublished window targeted; published Team Create window
excluded by wrapper guard. Temporary /tmp wrapper copy only waits longer for
Place1 discovery and prints structured results; published guard unchanged.
Actual read probes use temporary Play scripts/_G results; no Edit changes saved.
Owned offline hidden an offline Studio slot 2 /tmp/web-social-probe.rbxlx supplies isolated
fallback and callback-registration probes. Do not manipulate other slots.

## Login-session routes

All requests in crates/rustudio-account/src/web_social.rs use Session, never
Open Cloud. Explicit read-route enum; no arbitrary authenticated URL input.
POST batch reads use the existing session CSRF handling.

* economy /v2/assets/{id}/details (product metadata)
* apis /game-passes/v1/game-passes:batchGet (POST, Price view)
* apis /developer-products/v1/developer-products/batch?productIds=... (GET, comma-separated IDs)
* catalog /v1/bundles/{id}/details (PurchasePrompt Network)
* apis /game-passes/v1/game-passes:batchGetOwnership (read-only POST)
* inventory /v1/users/{user}/items/asset|Bundle/{id}/is-owned
* badges /v1/badges/{id}; /v1/users/{user}/badges/{badge}/awarded-date;
  /v1/users/{user}/badges/awarded-dates?badgeIds=...
* groups /v0/groups/{id}; /v0/users/{user}/groups;
  /v0/groups/{id}/relationships?relationshipType=Ally|Enemy&page=N;
  /v2/groups/{group}/users?userIds={user}
* users /v1/users (batch POST)
* apis /player-policy-service/v1/player-policy-client
* apis /virtual-events/v1/universes/{universe}/virtual-events;
  /virtual-events/v1/virtual-events/{id}
* apis /v1/subscriptions/{id}/product-info;
  /v1/subscriptions/{id}/status?userId=...
* apis /developer-products/v2/universes/{universe}/developerproducts?limit=50&cursor=...

Native builders: groups, badge,
inventory, users, gamepass ownership,
policy, developer products
, virtual events.
Upcoming event parser filters ended/cancelled/moderated/unpublished,
converts ISO timestamps to UTC date tables, and maps enum status/RSVP.

Real login-session reads succeeded for Marketplace, Badge, Group, User, Policy,
Social upcoming events, gamepass ownership and scratch universe products
(universe 10000000007 / place 10000000000008). Subscription EXP-1 is a negative
404 fixture only; no authorized valid subscription fixture exists. Real session
calls are in the explicitly ignored account live_reads test. Mock route tests
cover each implemented route, cookie forwarding, malformed JSON, 429 and cache.
Product/gamepass/subscription metadata cache is session-scoped, 60 seconds;
asset default cache is zero. Native defaults/1778/0ee0 and installed
Studio settings corroborate this. Other service throttles/cache parity remains
under investigation. Failed HTTP responses are not cached.

Online Studio confirmed GetProductInfo shape, ownership booleans, badge info
including AwardCount/PastDayAwardCount/Rarity, group info/group lists,
GetRolesInGroupAsync Roles plus IsMember, user info PascalCase fields, policy
PascalCase keys, friend page shapes, and local TextFilter result instances.
GetDeveloperProductsAsync in unpublished Place1 returns HTTP 404 (Not Found).
Commerce invalid product "0" returns HTTP 400. Online policy contains 13 keys
and filters return unchanged local text, SourceLanguage en-us, empty translations.

## Studio purchase flow

Five prompts (asset, pass, product, bundle, experience subscription) use local
mock purchases. Only metadata/ownership reads exist; no purchase HTTP route.
CoreScripts mock balance = 2147483647; disclaimer is “This is a test purchase.
Your account will not be charged.” Native completion hook selects event per
kind; product carries user ID, other ordinary prompts player. Closing completes
once; client completion relays to server with token/player validation. Product
mock confirmation creates receipt before final completion, callbacks process it,
then one-second loading/success screen requires closing. Gamepass/bundle mock
ownership updates before finished event. Pure mock tests exercise these paths;
no real/native purchase was accepted. UI screenshot verified headlessly with
mock metadata at /tmp/web-social-prompt-fixture.png.

ProcessReceipt server-only; BindReceiptHandler supports filters/catch-all,
prioritizes specific IDs, disconnect/rebind and Enum.ReceiptDecision validation.
Native registry; installed fallback/transfer/product
flags enabled. Duplicate-handler/catch-all/filter errors measured in offline
Studio. Handler returns RBXScriptConnection-shaped userdata with Connected.

Online prompt probe currently requests Kitty Ears 1374269 (for sale, 95 Robux),
cancellation only. CoreGui PurchasePrompt container exists but initial MCP
requests displayed no contents or finished events; normal LocalScript retry
under investigation. This is not a verified cancellation firing claim yet.

## Other observed local behavior

Teleport variant state copies containers; missing nil arguments reject; fresh
arrival data/gui nil. No valid-target teleport attempted: online Studio's
EnableStudioTeleports flag is true, so it might leave the local test.
Native old Studio failure handler raises
“Cannot Teleport in the Roblox Studio.” Invalid server place ID: “Invalid
placeId.” Client side/server-only checks measured separately.

Points balances zero; awardable 2147483647; AwardPoints returns (user,0,amount),
no PointsAwarded even with active player and nonzero amount in offline Studio.
Matchmaking InitializeServerAttributesForStudio copies dictionary; getter/setter
return success/error tuples; missing key does not become initialized by Set.
AdGui/Portal Status Inactive, placement reward strings empty and Content.none;
PortalInvalidReason not an available property in measured Studio version.
AdReward is opaque value userdata with equality by product ID. Negative IDs
accepted by constructor. ShowVideoAd warns “Sorry, AdService is currently off.”
and fires VideoAdClosed(false). Client availability returns InternalError.
Commerce browser/eligibility and Moderation violation APIs report disabled errors.
AwardBadge/AwardBadgeAsync return false. IsLegal true; IsDisabled warns and true.
GetPassOwnershipAsync and CanUsersTransferRobuxAsync report not-enabled errors.

## Validation and outstanding work

Account mock tests: 3 passed; explicit real read-only test passed. Domain tests:
8 passed (teleport isolation, local state, wrappers, social conversion, receipt
handlers, cancel deduplication, peer sender validation/event order). cargo check
passes with existing warnings; recheck after subsequent changes. Headless release
prompt fixture rendered and never confirmed. Coverage must be rerun; method
registration does not establish whole-domain parity.

Still actively investigating remaining Marketplace/Social/Recommendation/Ad/
Moderation/Commerce/notification/teleport APIs, all 18 event firing conditions,
error wrappers, subscriptions, lazy page behavior, service throttles and other
users' policy context. No claim that all 127 functions are done.

## Additional read verification and Studio defaults

Corrected subscription host/path from the native URL:
`https://apis.roblox.com/v1/subscriptions/...` (there is no `/subscriptions`
prefix). Login-session EXP-1 details returned NeverSubscribed and status returned
false/false. Product info returned subscriptionProductInfo:null, now converted
to native `Product info not found for subscription: EXP-1` rather than a raw
wrapper table. Non-null product parser traced, details parser at
. Converts enums, DateTime milliseconds, price-discount records and
PascalCase keys. Metadata/details caching defaults to 60 seconds.

Payment history is a native local Studio mock: EXP-0 relative
three Paid cycles, EXP-1 fixed two Paid cycles, EXP-2 fixed Refunded/Paid/Paid,
EXP-3 two relative Paid cycles; other valid EXP IDs warn and return an empty
array. Measured exact dates in owned offline server Play. It makes no web call.

CanViewBrandProjectAsync calls apis
`/brand-platform/v1/brand-projects/{id}/decision/studio`, parser
reads isProjectAvailable. Real session read returned false for id 0. Price levels
 calls apis `/developer-products/v1/universes/{universe}/price-levels`
with repeated userIds query parameters; real session returned price level 1000.
GetUserBadgesAsync parser reads the same awarded-dates response,
returns {BadgeId,AwardedDate=DateTime}, skips malformed records, maximum 100 IDs.
New conversion test covers badge statistics/dates, group membership, subscription
info/details/status/history and price levels. Domain focused tests now 9 passed;
account mock tests 3 passed and the explicit live read test passed again.

Studio-only capability checks: CanSendGameInviteAsync is client-only, requires
LocalPlayer and returns true even for recipient 1/-1. CanSendCallInviteAsync
returns true on both server and local client (client requires LocalPlayer).
CanPromptOptInAsync returns true only locally, with exact server-only error.
ShowRewardedVideoAdAsync in offline server Play returned ShowAdResult.InternalError;
client rejects with its server-only error. No valid ad was requested online.

Vinegar Place1 was stopped without saving. Fresh Play + normal LocalScript retry
also requested the asset prompt successfully but produced no prompt controls or
finished events. A screen_capture only captures Edit viewport, so it cannot prove
Play GUI state. Prompt cancellation/event order is still source-derived, not a
native firing observation. No completion hook invoked or purchase accepted.

Latest release coverage retry failed during unrelated data-domain initialization:
`rustudio.data.memory:106: Expected <eof>, got 'end'`. No coverage file was emitted.
The focused domain tests pass; retry coverage after a later shared release build.

## Native batch reads, pages, and teleport failure (continued)

Current Studio 0.746 enables the request orchestrators: GamePass metadata is
POST apis `/game-passes/v1/game-passes:batchGet` with gamePassIds/passView=Price
. Developer-product metadata is GET
apis `/developer-products/v1/developer-products/batch?productIds=...`
. Public ProductId 1286521615 succeeds; the different
DeveloperProductId is not the metadata identifier. Login-session mock/live tests
pass after route correction. Native GamePass and Bundle return shapes are
normalized separately. Asset cache remains zero, Product/GamePass cache60s,
ownership86400s. Inter-request batch coalescing/in-flight deduplication remains
unimplemented and is not claimed as cache/throttling parity.

Developer-product/group relationship pages now fetch the first page only and
advance on demand, retaining the last successful page after errors. Native
GetPlayersByPartyId is a local Player.PartyId filter, not an HTTP
party lookup. UUID argument validation and server-only behavior are implemented.
Recommendation-item read uses native POST `/recommendation-api/v1/recommendation-item/get`
; real public unknown item0 returns400.
Player-place read uses `/reserved-servers-service/get-player-place-instance`
; scratch-context real read403, not a fabricated Studio fallback.

Native offline legacy teleport probes fire TeleportInitFailed before raising
`Cannot Teleport in the Roblox Studio.` Options contain copied data/instance/
access-code values. The four legacy methods now implement that local failure,
including server-to-local-client handoff. Headless client order and server handoff
probes pass. TeleportAsync is still a legacy runtime stub: its separate native
manager returned an empty error offline and cannot safely be inferred from the
legacy path. No valid live teleport was attempted. ReserveServer methods POST
create reservations and were not called live under the read-only restriction.

BindReviewableContentEventProcessor now stores priority/callback connections;
duplicate priorities are valid and Disconnect removes only its own registration.
Native CreateReviewableContentKey accepts ScreenshotCapture/VideoCapture, not
EditableImage. A real Place1 Edit capture proved the key is padded Base64 of
compact JSON in order id/contentType/contentLength, uppercase random UUID, and
fresh ID on repeated calls. Example capture731751bytes image/png; no upload.
The local implementation resolves CaptureService generated bytes and retains
content by key, without moderation HTTP. Key encoding has an exact native fixture
test. Capture object typeof is still owned by assets_cloud (native Object versus
its current Instance representation), so that upstream type gap is not fixed here.

Latest focused tests16 passed, account mocks3 passed plus explicit live read test.
Release coverage before the last three local members:79/127 functions,21/24
properties,7/18 events recognized; recognized events are not firing proof.
PortalInvalidReason is absent in measured Studio0.746 and now errors as native.
PointsAwarded and fresh LocalPlayerArrivedFromTeleport did not fire in native
Studio. Five generic purchase completion flows are source-derived and tested
locally; native Place1 did not display purchase controls, so cancellation order
is not independently measured. Premium/Roblox-subscription/bulk/social/commerce/
notification prompt events still need implementation/firing verification.
The owned offline an offline Studio instance and MCP disappeared externally; online Place1
remains usable. Play is stopped, no save/publish performed.

## Additional native checks (online Place1)

Notification CoreScript modal appeared in normal Client Play: title `Turn on
Notifications`, body `Receive updates on activity while you're away`, buttons
Cancel/Yes. Clicked ONLY Cancel through Studio mouse input. OptInPromptClosed
fired once with zero arguments, matching NotificationsUpsell setup bytecode's
InvokeOptInPromptClosed(). No opt-in accepted; Play stopped without save.
This is a native firing observation, not a completed rustudio implementation.

GetRobloxSubscriptionDetailsAsync traced entry -> product-list builder
 -> first productKeys[1].id -> details request with RBP-prefix
 -> result parser. Product type RobloxPlus, includePurchased
true. Result IsSubscribed excludes NeverSubscribed/Expired; IsOriginExperience
false until positive robloxSubscriptionStateDetails.startTimestampMs, then true
and StartTime DateTime. Login-session product-list call returned403, matching
native online Place1's SubscriptionRequest::listAvailableSubscriptionProductsAsync
HTTP403. Mock route and two-stage conversion tests pass (17domain,3account),
explicit live read test passes. No subscription mutation endpoint added.

Headless actual rendered TakeScreenshotCaptureAsync -> Content.fromObject ->
CreateReviewableContentKey twice passed WS_CAPTURE_KEY_PASS. No upload.
Coverage now emits successfully again after shared rebuild; latest result is
/tmp/web-social-after6.json (counts pending domain-only aggregation).

Commerce metadata: GET apis
`/commerce/v1/purchasing/in-experience/commerce-products/{id}/product-info`;
identifier validator accepts optional COM- prefix and1..20digits.
Parser returns IsForSale/Item/Benefits; item parser
Name/Description/IconImageAssetId/DisplayPrice/IsPurchasable, benefit parser
 Name/Description/Id/Enum.BenefitType. Native Place1 Edit COM-0
HTTP400(Bad Request), login-session same route400. Success-schema mock tested;
no real commerce purchase. Focused tests18 passed, account3+live1 passed.

Ad campaign eligibility GET apis
`/brand-platform/v1/immersive-branded-ads/campaigns/{campaign}/eligibility`,
HTTP/status parser, JSON parser. Validates isEligible bool,
advertiserName string, immersiveBrandedAdPlacementIds string array. Public result
IsEligible only; successful local responses cached by player/campaign. Errors
remain Request failed as native Studio for unknown campaign0; live metadata-only
request tested, no ad displayed. Domain19 tests, account3+live1 pass.

## Explicit event firing audit (all18, still in progress)

| Event | Current evidence / remaining gap |
|---|---|
| PromptPurchaseFinished | Implemented. Native asset1818 not-for-sale X returned `(player,1818,false)`; mock cancellation/confirmation and peer dedup tested. |
| PromptGamePassPurchaseFinished | Implemented. Native pass1 not-for-sale X returned `(player,1,false)`; ownership-before-event source path tested in mock. |
| PromptProductPurchaseFinished | Implemented. Native product1286521615 test prompt X returned `(userId,id,false)`; receipt-before-completion path tested only in mock, never accepted live. |
| PromptBundlePurchaseFinished | Implemented. Native bundle1 not-for-sale X returned `(player,1,false)`; source controller mock completion/cancel tested. |
| PromptSubscriptionPurchaseFinished | Implemented. Native EXP-1 generic error X returned `(player,"EXP-1",false)`; subscription source flow tested with mock details. |
| PromptBulkPurchaseFinished | Open. Native three-item summary X yielded Aborted, RobuxSpent0, all SystemError; invalid quote yielded Error/empty Items. Quote helper exists, full order/confirmation path still requires native fulfillment contract. No real order submitted. |
| PromptPremiumPurchaseFinished | Registered, deliberately dormant with current online Premium-to-Plus routing. Native Premium X emitted only the RobloxSubscription event. |
| PromptRobloxSubscriptionPurchaseFinished | Implemented. Native Premium/direct Plus X returned `(player,false)`; mock confirmation/state handoff and physical headless X tested. |
| CallInviteStateChanged | Open. Native Studio CanSendCallInviteAsync returns true (both server and local client); no native call/device state transition exercised. No call/invitation sent. |
| GameInvitePromptClosed | Implemented. Native duplicate-open/cancel audit: exactly two arguments `(player,{})`; physical headless X plus server replication tested. No recipient activated live. |
| PhoneBookPromptClosed | Registered, dormant in current desktop Studio: native PromptPhoneBook returned nil/no picker/no event. Device contacts flow is outside Linux target. |
| ShareSheetClosed | Left. Link preparation returns result/link; shipped OpenShareSheetWithLink separately invokes clipboard/shareUrl platform transport. Both share flags are enabled in initialized native Client; platform completion/event transport remains unported. No external share attempted. |
| PointsAwarded | Registered, dormant: native Studio AwardPoints returns tuple without firing. Source-compatible Studio balances tested. |
| PromptCommerceProductPurchaseFinished | Open. Native COM-1 generic prepare failure returned nil/no event; CoreScript classified-error dismissal and browser-close completion are traced but not implemented. Native public event is exactly `(player,productId)`, no boolean. |
| OptInPromptClosed | Implemented. Native three requests then Cancel produced exactly one event, zero args; physical headless Cancel and mock backend failure/confirmation ordering tested. |
| LocalPlayerArrivedFromTeleport | Open. Native fresh Play never fires and has nil arrival state. Actual teleport prohibited; modern manager/arrival handoff remains unimplemented. |
| TeleportInitFailed | Implemented. Native legacy error event precedes thrown Studio error; headless client/server-handoff ordering and TeleportOptions payload tested. |
| VideoAdClosed | Implemented. Native obsolete ShowVideoAd fires false; local runtime verified, no real ad displayed. |

Latest complete coverage /tmp/web-social-after7.json:85/127 functions,21/24
properties,7/18 registered emitting events. Four remaining TextService layout
methods belong to gui_input. Registration count deliberately not described as
18-event behavioral completion. TeleportAsync remains a pre-existing false
coverage positive and must be corrected separately; the native manager performs
reservation/teleport backend work before legacy client Studio rejection, unlike
the verified four legacy methods. No valid live TeleportAsync/ReserveServer call.
Native Premium/RobloxPlus/Bulk probes left no prompt to cancel; Play stopped.

UserService GetUserInfosByUserIdsAsync explicitly serializes
excludeBannedUsers=true (store boolean1 at field offset0x40), corrected helper
from false. ID list accepts negative numeric IDs; native empty list and{-1}
return empty arrays, nonnumeric exact `UserService: invalid userId(s) given.
Please make sure they are of number type`. Helper now keeps signed IDs.
Native transport error wrapper is `UserService: Request Failed because %s`.

Further safe native reads: BadgeInfo0/-1 -> Invalid badgeId; positive1HTTP404
-> badgeId is invalid or does not exist. Native callback specially
maps only404, leaving other transport errors intact. GroupInfo0 rejects with
GroupService:GetGroupInfoAsync() argument is not a valid groupId; fixed before
HTTP. UserHasBadge(1,0) and UserOwnsGamePass(1,0) bothfalse.

Global-user lookup is not a plain users GET: traced
->. Native request is POST
/private-users-service/v1beta1/getOrCreatePrivateUserId. It can allocate a
private identity. Not added to the read-only request enum and never called live.
This blocks safe live parity verification/implementation as a read-only lookup
until a nonallocating native route or local Studio-mode behavior is established.

Legacy policy route remains apis/player-policy-service/v1/player-policy-client.
Alternate is gameinternationalization.roblox.com
/v1/player-policies-client. Which feature-flag branch
Studio0.746 actively selects still needs runtime flag evidence; not silently
switched based on a dead-string match.

Recommendation GenerateItemListAsync: validation; URL
POST apis/recommendation-api/v1/recommendation/generate; body.
Body experienceId/configName/locationId, paginationToken{count,cursor?}, optional
boostCustomTags, realtimeRenderContext array{key,value} (public CustomContexts).
Returns RecommendationPages with lazy subsequent calls. Native parser
reads items, responseToken.cursor, tracingId; shared item parser,
TracingId overwritten per response. Exact native unknown-key error measured.
Native Place1 config0/location0/UserId1 returns Failed to fetch recommendation
items - HTTP400(Bad Request); cookie scratch read config0 same400. No configuration
was created, so positive live fixture remains unavailable. Mock two-page test
covers context preservation, count, cursor, per-page tracing and required-context
errors. Domain20/account3/explicit live1 passed. Current policy migration flag
was read through settings():GetFFlag and is true; selected apis route retained.

Native GetFVariable settings: ad campaignId max36bytes, cache max50 campaigns,
placement IDs max110, HTTP timeout3000ms and client response timeout6000ms.
Added measured size checks/cache capacity. Product/GamePass metadata60s,
asset0s, ownership86400s; subscription details/status60s, product-inner60s,
outer SubscriptionIdProductInfo lifetime600s, subscription-products3600s.
Two-layer subscription caching applicability remains under investigation; not
blindly switched from60 to600. RankProducts and RecommendTopProducts enabled,
60s caches, 10/minute and5/minute respectively.

Further native boundary reads: UserHasBadgeAsync negative user/badge and
UserOwnsGamePassAsync negative user/pass bothfalse; early exits now precede
unsigned HTTP argument conversion. CheckUserBadges accepts negative badge IDs;
helper preserves signed values. GetGroupsAsync(-1) returns{}, helper now accepts
signed user ID. GetUserBadgesAsync(-1,...) instead returns GetUserBadgesAsync:
User is invalid or does not exist. These methods deliberately differ.

Marketplace ranking read trace (implementation next): rank POST body
{universeId,productIdentifiers:[{id,type:Product|GamePass}]}; recommend body
 {universeId,productInfoTypes:[Product|GamePass]}. URLs native
 rank, sibling recommend, apis/product-recommender/v1/virtual-products:
rank|recommend. HTTP response parser uses rankedItems array containing
productIdentifier{id,type}, optional versionTag. Final native result
fetches metadata and returns array{ProductIdentifier={Id,InfoType=enum},ProductInfo}.
It is NOT an array of raw IDs. Native observed numeric InfoType is invalid;
Rank index errors are1-based, Recommend index errors0-based. Limit50, rank cache
true, fallbackfalse, both60s; rank quota10/minute/recommend5/minute. Empty rank
argument exact must have at least one ProductIdentifier; Edit recommendedProduct
request with valid type reaches backend and400 with task wait prefix.

Marketplace RankProductsAsync/RecommendTopProductsAsync implemented from the
above trace. Login-session scratch-context rank returned200 with
rankedItems[{productIdentifier:{id:1286521615,type:Product},score:...}]; top
recommendations returned200/empty. No purchase. Success conversion, cached
copy isolation, 10/minute quota, expiry, numeric/wrong enum and unsupported
InfoType errors tested. Added own Rust enum-type helper because LEnum wrappers
do not compare by value; no shared ltypes edits. Headless actual Client run
WS_MARKET_REC_rank_PASS and WS_MARKET_REC_recommend_PASS reproduced native
unpublished-context task wait HTTP400 errors. Focused domain21/account3/live1 pass.

HTTP cache now has a per-service-VM lifetime token in the account helper; cookie
alone no longer lets ownership/product entries carry across fresh Play sessions.
Outstanding worker threads retain that token until done; final release clears
its entries. Mock test verifies same-cookie VM scopes are separate and each
scope reuses its successful response. Ranking/status cache TTL60s matches native
settings. Scope0 remains available for explicit standalone account read probes.
Latest release coverage emits /tmp/web-social-after10.json; expected function
count88/127, properties21/24, emitting-event registrations7/18.

Important teleport follow-up: online Studio GetFVariable EnableStudioTeleports
is true. Studio has a StudioTeleportsManager and the alternate message
Teleporting while using Roblox Studio is not permitted. A flag value alone does
not prove which delegate is active; tracing its Studio-specific hook is pending.
No valid online teleport was attempted. Existing legacy failure evidence is from
the owned offline Studio, not proof of every online/beta Studio configuration.
Do not replace the unknown TeleportAsync behavior with a guessed constant error.

Teleport deeper trace: shared manager and validation have separate server/context checks and can reach HTTP via. Only Studio itself has the manager. The alternate Teleporting while using Roblox Studio... text is an indexed script-message table entry, not an established active teleport rejection path; bounded direct-reference search found no use of its index. It must not be used as a guessed constant. No online teleport or global flag setting changed (the Studio process also owns the user's published window).

Self-view implemented from native engine and bundled
Chrome/Integrations/SelfieView. Native methods dispatch protected SelfViewHidden
and SelfViewVisible(position) signals without client/server restriction or HTTP.
Headless mode has no camera window; internal signals remain real. Online Place1
Edit/Client/Server calls returned zero values, Show default LastPosition, numeric
0..4 and named positions accepted; invalid number/bool/string/wrong enum errors
measured and matched. Protected signals are not part of the18 game events.
Focused23 tests pass with --test-threads=1, including real runtime signal firing.
Parallel tests intermittently SIGILL with the new full Runtime fixture; serialized
run passes. Place1 Play stopped after probes; no camera toggled or backend write.

Moderation follow-up: native CreateReviewableContentAsync consults
ModerationServiceEnabledUniverseIds via BEFORE server validation.
Universe0 bypasses allowlist; scratch10000000007 absent from online allowlist.
Enabled path POSTs user/cloud/v2/universes/{}/reviewable-content with uploadUriSpec.
InternalRequestReviewableContentReviewAsync POSTs same root plus
/{id}:requestReview, without that outer feature gate. Both are live mutation
paths, not safe metadata reads, and neither was invoked. Their enabled-path
implementation remains open; no disabled-only registration added for coverage.

GetPartyAsync added after correcting an earlier inference about its output.
 constructs dictionaries, not private User SDK objects: UserId,
PlaceId, JobId, optional PrivateServerId and ReservedServerAccessCode. Parser
 consumes universeMembers, ignores invalid user/place/job entries,
and keeps latestGroupUpJoinTimeMs for descending ordering.
Callback overlays matching connected Player.PartyId and removes
stale backend entries for connected players no longer in this party. Local
ReservedServerAccessCode comes from the top-level response. No identity write.
Read URL: apis/group-up/v1/get-group-up-universe-members?groupUpId=
...&universeId=...&placeId=...&instanceId=... (escaped values), no result cache.
Online PartyAsyncThrottleCountPerMinute=120; uses that
base budget, no per-player increment, default60-second window.
Online Place1 Server invalid UUID gives Received invalid party id.; all-zero
UUID reaches read request and returns exact Request failed: %s (literal percent
placeholder). Rustudio scratch login read returns403. Mock success tests cover
records, optional keys, stale local members, order, copy isolation and quota.
Domain24/account3 tests pass; live_party_read passes. Headless actual Server
WS_PARTY_READ_PASS matches native failure. Place1 stopped after probes.
Self-view release/headless actual Client WS_SELF_VIEW_PASS also confirmed.

Pending cached metadata/ownership reads now share the same request within the
login-session+VM scope. Completion publishes independent JSON copies to current
waiters; failures are not retained for later calls. A dropped request wakes its
waiters with an interrupted error. Mock eight simultaneous Product reads yield
one HTTP call; another VM scope yields its own call. Account4 tests pass, cargo
check passes. Different-ID native batch aggregation remains separate work.

Different-ID request batching now follows the native template scheduler.
Online settings: GamePassOwnershipBatching, GamePassInfoBatching and
ProductInfoBatching MaxSize50, ShortWindow40ms, LongWindow200ms,
ActivityCooldown1000ms, MaxTotalCallablesInflight5000. The named
SubsequentRequestCooldownMs10 settings exist but no active read of that raw
value was established; no invented10ms inter-batch sleep added.

Native product batch: +50/max, +58/short, +60/long,
+68/activity; getters/1440/1430/1420. Base scheduler
uses short40 AFTER >=1s idle, long200 during activity, subtracting elapsed time
from the first queued call. Enqueue schedules one pending batch,
without an early-full-queue dispatch. Flush takes at most50 and
records dispatch time. Completion clears busy and
schedules the next batch, so requests remain queued during the HTTP call.
No assumption that long means idle or that50 triggers immediate send.

Developer product URL joins IDs with comma and productIds={}
, not repeated query parameters. Game-pass info batches gamePassIds
with passView Price; ownership batches user/pass identifier pairs. Queue keys
include cookie fingerprint, VM scope, origin and request kind; each VM final
release clears its idle queues. Result rows are narrowed to each requested ID;
missing metadata is not cached (native completion missing-item branch bypasses
cache insertion). Followers receive independent result copies.

Account6 focused mock tests pass, including eight same-ID requests/one HTTP,
three separate kinds, per-ID selection,51 requests/max50 batches, and omitted
item retries. Explicit live_metadata_batch reads two public Doors product IDs
from its public product list and successfully fetches both metadata records in
one login-session batch. Domain24 focused tests pass, cargo check passes.
No purchase or backend mutation.

Notification eligibility corrected for published context. Native
CanPromptOptInAsync only returns true locally when GameId0.
Otherwise URL GETs apis/notifications/v1/
can-prompt-experience-notification-opt-in?universeId=..., no cache. Parser
 reads boolean canPrompt. Error callback matches
429 and resumes false,
otherwise prefixes ExperienceNotificationService request failed: . Empty,
malformed JSON and missing-key messages copied from parser/error helper
. Read-only scratch login probe returns {canPrompt:true}.
Domain25/account6 focused tests pass. No notification follow or prompt POST
was performed. Headless WS_BATCH_METADATA_PASS also passed, fetching Product
and GamePass records through the new batch queue.

PromptOptIn investigation (not implemented yet): checks local script, directly requests CoreScript UI at GameId0, otherwise POSTs apis/notifications/v1/prompt-experience-notification-opt-in {universeId}. Callback requires canPrompt:true; failure helper logs an error and fires OptInPromptClosed even when no dialog was shown. CoreScript NotificationsUpsell/Requests/FollowingsPostFollow POSTs JSONEncode({}) (which native Studio encodes as []) to followings.roblox.com/v1/users/{localUserId}/universes/{universeId} only after Yes and only for nonzero universe, then closes immediately before HTTP completion/toast. Native/online test clicked Cancel only. ModalRenderController.UpdateRoot invokes the old CloseCallback only when the modal key changes; another UENUpsell replaces its parameters without firing closed. English Heading Turn on Notifications; unpublished body Receive updates on activity while you're away; published body Receive notifications from {experienceName} while you're away?

Notification prompt implemented in web_social_prompts.rs. Online Place1
Client3 requests kept closed=0; explicit mouse Cancel produced closed=1 with
zero arguments. Play stopped. Rustudio headless actual Client UI rendered the
source English strings; physical harness click on Cancel emitted exactly one
WS_OPT_IN_CANCEL_PASS. ScreenGui survives respawn, duplicate UENUpsell requests
replace without false close, background dismissal and explicit Cancel/Yes close
once. Published flow posts the source prompt request, requires canPrompt true,
fetches/caches the experience name promise, and starts the follow POST only on
Yes. It closes before the follow-result toast; no double close on HTTP completion.

NotificationAction is separate from the Read allowlist. It contains only these
two source POSTs, no arbitrary URL or invitation/purchase operation; Follow
also checks the local user matches the login session. All POST verification is
localhost MockServer only, including Cookie/CSRF transport, numeric universeId
body and the actual empty-array [] follow body (verified by native JSONEncode).
No live follow or prompt POST has been performed. Read-only eligibility/name
remain in Read. Native engine error helper logs then fires closed without
showing a dialog; an empty success response takes both empty-response and
parse-failure branches, causing two closed events. That source-derived edge
case is copied/tested, rather than deduplicating independent native failures.
26 domain /7 account focused tests pass; release/headless cancel passed.

Published dialog-name read follows GameDetailRodux.GetGameNameAndDescription:
games/v1/games?universeIds={id}, cached per universe promise in the VM, including
failed lookups. Scratch login read returned a string name. The actual Native
JSONEncode({}) probe returned [] at PlaceId0/GameId0; the follow POST mock checks
that exact byte body. Three Native UENUpsell requests then explicit Cancel
produced closed1,args0; headless mouse input repeated the same lifecycle.
Latest /tmp/web-social-after13.json records92/127 functions,21/24 properties,
8/18 registered game events; protected CoreScript signals are excluded.

Online-client flag correction: NotificationsUpsellDisableBackgroundDismiss is
true in the Client DataModel (Edit cannot read every Lua-defined flag). The
notification backdrop captures input without closing; explicit Cancel closes.
Headless physical background click followed by Cancel passed
WS_OPT_IN_BACKGROUND_IGNORED / WS_OPT_IN_CANCEL_PASS. PointsAwarded is now
connectable, but deliberately has no Studio emission: native AwardPoints returns
(userId,0,amount) and does not emit; a full Runtime test checks this behavior.
This is not an additional positive event-firing proof.

The Vinegar installation differs from the older offline Studio artifact: online packed
DataModelPatch sha256 86d439e98dc7030b9722e95fdbf784032e20ab84878c70dbebce1190cd8039e1
versus an offline Studio b01a0afc6a861a58d8811dce91fab592f0063238b1faa2c81b771e5c4f9b2e54.
Extracted 45,690 online scripts to /tmp/web-social-online-scripts. Its purchase
connector routes Premium directly to RobloxSubscriptionPurchaseApp; the older
RoutePremiumToBlackbirdUpsell flag branch is gone. Therefore absence of that
flag in the Client does not imply the connector is unloaded. Continue prompt
investigation against the online package. Actual native Client binding and
Studio mock-subscription flags are both true. Native Play stopped after reads.

Online purchase initialization/cancellation correction: requests made before the
CoreScript UI mounted were previously recorded as nil/no-controls. Retrying
once initialized produced FoundationOverlay dialogs. Native Premium now routes
through the Roblox Plus sheet, and clicking its X fired only
PromptRobloxSubscriptionPurchaseFinished(player,false). Direct Plus request and
X repeated the same result. Never activated Subscribe/Try it for free/Buy.
Native Client RobloxSubscriptionProductsDirectFetch=true,
RobloxPlusAmpSubscriptionCheck=false, FixSignalFinishedOnError=true;
RobloxSubscriptionPurchasePollingTimeoutSeconds=8 (source default4 overridden).
The source mock confirm hook is SignalMockPurchaseRobloxSubscription: native
 sends a server signal; receiver branch sets
HasRobloxSubscription via. Implemented local
property replication and source event-before-polling ordering, no purchase HTTP.
Current UI uses a simplified engine-owned sheet; layout/icons/discount feature
copy are not pixel-identical to Foundation. Product prices/offers come from the
active client endpoint, not a hardcoded account offer.

CoreScript direct metadata GET is apis/subscriptions/v2/products?
productType=RobloxPlus&referrerId=0. Read-only login probe returned one product
with eligibleOffers/localizedPrice/productKey/productTypeDetails/periodType.
The engine RCC lookup remains separate (v1/subscriptions/rcc/products).
Network.createPaymentSession POSTs apis/payments-gateway/v1/payment-sessions
{entryPoint:"PromptSubscriptionPurchase"} concurrently with metadata; it catches
all failures and resolves an empty session ID. This initialization POST is
implemented and tested only against localhost (503 ignored with successful
product metadata); no live POST was executed by rustudio/tool HTTP. Native
Studio itself opened the explicitly authorized prompt. Mock confirmation does
not use any real payment/purchase endpoint. Source-derived mock tests cover
Premium routing, Plus false/true event shapes, no receipt, empty-product error,
server-local completion and duplicate dismissal.27 domain/8 account tests pass.
Actual headless source UI WS_PLUS_UI_READY / WS_PLUS_CANCEL_PASS passed via
physical X click with mock metadata; no live session POST during that run.
Native pass1 not-for-sale error displayed the exact copied message and X
produced(player,1,false). Native public Product1286521615 displayed 1 REVIVE,
40 Robux and the test-purchase disclaimer; X produced(userId,1286521615,false).
Updated generic Error/Buy item title, Buy label, X control, error disclaimer
visibility. Confirmation remains mock-only. Native Play stopped after probes.

ReserveServer / ReserveServerAsync now use native reservation operation
, yielding two strings in source order: ReservedServerAccessCode,
ReservedServerGameCode (the latter is the public PrivateServerId result).
POST apis/reserved-servers-service/create?placeId={signedInt64}, empty body,
ApplicationUrlEncoded. Native content-type enum2 confirmed; parser
requires both uppercase response keys and reproduces all six empty/parse/missing/
wrong-type messages. Client guard is the exact deprecated-name message even for
Async: TeleportService:ReserveServer can only be called by the server.
HTTP failure callback passes through HTTP errors without a new prefix.
No cache/reuse of reservation responses. Tests use localhost only, including
successful tuple, server/client guard, 429, empty/malformed/missing/wrong-type
responses and an actual empty POST body. No real reservation or teleport was
performed.28 domain/9 account focused tests pass. This operation is separate
from the Read allowlist. Modern teleport manager remains under investigation.

Bulk source investigation advanced: native PromptBulkPurchase is server-only;
client calls warn "Can only be called on the server." and return without an
event. C++ validates max20/empty before side/guest guards. Server
caller quotes a Studio-only order through POST
apis/marketplace-ua/game-engine/v1/prepare-studio-bulk-purchase-prompt
. Body purchaserUserId, items[{id,type}],
optional purchaseOptions; offer flags can add applyOffers/unselectedOffers.
Parser requires numeric purchaserRobuxBalance/orderTotalRobux,
receives items/order/discountInformation, and fills absent batchPurchaseAuthToken
with studio-bulk-purchase-{UUID}. Prepare helper is now implemented,
with localhost-only POST tests for success/400/429/invalid JSON/schema. No direct
live helper POST was run. Native authorized Studio itself opened the prompt.

Online native server order with asset1818 failed HTTP400: no GUI, one event (player,Enum.MarketplaceBulkPurchasePromptStatus.Error,{RobuxSpent=0,Items={}}), plus output "MarketplaceService.PromptBulkPurchase: HTTP 400 (Bad Request)". A subsequent valid public avatar order (63690008,48474313,1029025) opened Summary. Only X was clicked. It fired Aborted, RobuxSpent0, three items with string IDs, Enum.MarketplaceProductType.AvatarAsset and **SystemError for every status**, including already-owned items. Native Play stopped after cancellation. Do not translate cancellation into AlreadyOwned/Success: selectResultItems uses SystemError until there are purchase-result statuses; selectRobuxSpent sums only items with Success. Draft UI/controller remains /tmp/web-social-bulk-source.lua, not registered as implemented until its confirmation path is traced. No such POST or native confirmation was executed. Investigate the mock-token/server contract before integrating this path; never use an API key or create a production purchase order. Generic test purchases and Plus remain local mock paths.10 account tests pass.

Additional online AdService probes: only invisible/unparented Folder/TextButton,
placement0. No ad rendered or impression produced. RegisterAdOpportunityAsync
failed "Placement validation request failed."; RegisterDisclosureButton failed
"Missing associated campaign eligibility status for placement \"0\". Make sure
GetCampaignEligibilityAsync for your campaign succeeded and the user is eligible";
RegisterImpressionSource rejected Folder with "Instance must be a Part";
UnregisterAdOpportunity returned no values. Native Play stopped. Source SDK
eligibility lookup also distinguishes cached ineligible campaign with
"User is ineligible to see this ad campaign: \"{}\"". SDK impression registration
 requires actual Part with Shape.Block, enforces registration limit
and placement-specific duplicate ownership. Unregister removes all
outer opportunity records for the Instance; it is a different registry from
SDK disclosure/impression registrations. Do not conflate them. Placement-list
URL is GET apis/ads-placement-service/v1/getAdPlacements?universeId={id}.
This source trace is in progress, not yet registered as implemented.

Ad opportunity registration/unregistration implemented separately from disclosure/
impression SDK state. Read-only scratch getAdPlacements returned {placements:{}}.
Parser recognizes PLACEMENT_TYPE_REWARDED_VIDEO and
PLACEMENT_TYPE_PLAY_WITH_REWARD with defaultPlacementID/customPlacementIDs.
RegisterAdOpportunityAsync only selects rewarded-video placements: no matching
type => "No rewarded video placements available for this universe."; unmatched
explicit ID => "Placement ID is invalid."; failed HTTP/schema => "Placement
validation request failed.". Shared pending/failed result is retained per
universe in the VM, matching the shared native future. Native tracker
 validates max count before duplicate detection, then actual GuiButton
and ScreenGui ancestry. Stores the instance/selected placement;
repeat registration does not replace its placement. Unregister is
local, any side, and clears all matching opportunity records. No ad display is
requested by these methods. Native tracked limit AdOpportunityTrackedInstancesMax
was read from signed-in Edit settings:1000, matching binary default.
Focused mock test covers cache/errors, selection, null/type/ancestry, cap/dedupe,
registration/removal and destroyed-target cleanup.29 domain/10 account tests pass.
Disclosure-button and impression-source SDK operations remain under investigation.

Actual headless client + server API probes passed
WS_AD_OPPORTUNITY_CLIENT_PASS / WS_AD_OPPORTUNITY_SERVER_PASS, using only
metadata GET and unregister (no real ad registration/impression/display).

Online social remeasurement after UI initialization: PromptGameInvite(LocalPlayer)
opened Invite Friends in unpublished Place1. Only title-bar CloseButton clicked;
GameInvitePromptClosed had exactly(player,{}) with two args, empty array. Three
requests in one batch generated two immediate close events for duplicates,
then one more when X dismissed the still-open original. Server also received
all three events, each two args. No recipient buttons were activated; no invite
or message was sent. PromptPhoneBook requires the second string tag argument;
with empty tag returned nil, no phone-book UI or close event on this desktop.
Native Play stopped after the probes.

GameInvite CoreScript same bytes in online/offline bundle. ModalManager.closeModal
and duplicate-open branch invoke InvokeGameInvitePromptClosed(LocalPlayer,{})
unconditionally for developer triggers; they do not return sent recipients.
Frontend permission GET is apis/notifications/v1/can-prompt-experience-invite?
PlaceId={place}&Trigger={trigger}[&RecipientUserId={recipient}]. Wrapper reads
canPrompt/canCustomize, failures ->false,false. Login GET scratch10000000000008
returned both true; place0 HTTP400 (the default native Studio dialog nevertheless
opens through its fallback/legacy path). Request/mock test added. The send
request source is PostSendExperienceInvite: POST apis/notifications/v1/
send-experience-invite with recipientUserId/placeId/trigger and optional
inviteMessageId/launchData/idempotencyKey. No send POST has been executed.
Its Luau type-info bytes 08 00 00 05 06 0f 03 03 03 83 83 prove recipient,
place,trigger are strings and the two optional fields optional strings (rather
than guessing JSON numeric IDs). Actual Client LuaAppExperienceJoinIdempotencyKey
was true. At PlaceId0 the provider warns "Game Invite failed to send. Cannot
send invite to unpublished Place." before the HTTP call; this branch was
statically read, never activated through a recipient button in Native Studio.
Controller port still in progress; do not count GameInvite event as implemented
from native evidence alone. Current release /tmp/web-social-after15.json has
98/127 functions,21/24 properties,11/18 registered game events; 9 have emitting
implementations, PointsAwarded/Premium are intentionally dormant in current
Studio behavior. Four TextService layout gaps remain owned by gui_input.

PromptGameInvite now uses a separate modal controller and engine transport.
Native current default (customization2 disabled) opens a searchable friend roster;
duplicate requests emit(player,{}) without replacing the first modal. Explicit
Close emits once and replicates to the server; sent IDs are not returned.
Unpublished recipient activation warns/fails before HTTP. Published requests use
source string fields and fresh idempotency GUID, pending/success dedupe and
retryable failure without dismissing the dialog. Development exercises the send
POST only through localhost mocks. No invitation was sent to live Roblox.
30 focused domain tests and11 account tests pass; cargo check/release pass.
Headless actual API/physical CloseButton test passes WS_INVITE_DUPLICATE_PASS,
WS_INVITE_CANCEL_PASS, WS_INVITE_SERVER_REPLICATION_PASS. Visual shell uses a
simpler native rustudio layout; user-facing friend data comes from FriendPages.
Initial headless clicks used pre-layout bounds and missed; final viewport click
264,90 hit only Close. Current native desktop PromptPhoneBook empty tag returns
nil/no close signal. LinkSharing variants both reject Client with
"SocialService:PromptLinkSharing can only be called from script with Server RunContext".

Desktop PromptPhoneBook now matches measured signed-in Studio return/guards:
requires a string tag, a Player, and on Client the LocalPlayer. Valid empty tag
returns no values and does not emit PhoneBookPromptClosed. Native C++ guards at
 additionally have a user calling-support predicate; current account
passes it. Mobile contacts picker behavior is outside the Linux build. The
connectable desktop completion signal is intentionally dormant. Unit assertions
cover the no-event result and missing-tag error;30 focused domain tests pass.
Native corrected feedback({}) returned nil with no feedback CoreGui nodes.
Do not infer a working feedback dialog from that result: native callback
 enters an SDK/web-view path, still being traced. Link server calls
for both aliases returned HTTP403 in unpublished Place1; client guard is exact.
No feedback was submitted, no sharing or invitation action was completed.

Recommendation mutation source reconstructed: RegisterItemAsync,
; UpdateItemAsync;
RemoveItemAsync. Fixed login-session POST routes:
/recommendation-api/v1/rcc/recommendation-item/batch and /update; deletion
uses /recommendation-api/v1/rcc/recommendation-item/delete on Server and
/recommendation-api/v1/recommendation-item/delete on Client. Register body
experienceId/items, creator user ID/type User, ReferenceId, optional enum names,
integer Duration, attribute-table array, CustomTags string array. Update supports
creator User only as does. Register source validates unknown
keys against exact six-key list; Current online Studio confirmed
all six malformed-request error texts before HTTP, including allowed-key order.
Response reads results[1].itemId/referenceId, returns ItemId/ReferenceId;
empty results produces empty strings, malformed/missing items reject. Failure
callbacks preserve HTTP detail in source "Failed to ... - {}" forms.
These three transports were executed exclusively against localhost mocks,
covering all four routes, schema failures, empty results and HTTP429; no live
recommendation was created/updated/deleted.31 focused domain/12 account tests
pass, cargo check passes. Native Play stopped after pre-HTTP guard probes.

PromptLinkSharing/Async reconstructed rather than hardcoding Place1's HTTP403.
Both APIs share native entry, server-only guard, source request builder
 and response. POST apis/deeplinks/v2/rcc/expiring uses
CreatorUserId and optional capitalized FallbackLinkId/ExpirationSeconds/
PreviewTitle/PreviewDescription/PreviewAssetId; LaunchData becomes CustomData.
Nonempty response.link returns(Enum.PromptLinkSharingResult.Success,link).
Oversized LaunchData logs source message and returns(InvalidLaunchData,"")
without HTTP. Native Edit settings confirmed EnableLinkSharing=true and
LaunchDataLength=200. Malformed/empty link yields exact network-error text;
non2xx preserves HTTP status. This engine path prepares a link; it does not
open a platform share sheet or emit ShareSheetClosed. No link-preparation POST
was executed live from rustudio/tools. Localhost tests cover body, response
schema/status; source controller tests cover both aliases, guards and size
result.32 domain/13 account tests and cargo check pass. Headless actual client
recommendation guards also passed WS_RECOMMENDATION_CLIENT_GUARDS_PASS.

Ad SDK registry reconstruction retained privately, NOT exposed/counting the two
remaining methods yet. Current online Server rejects both with "<method> may
only be called on the client". Client checks null/GuiButton, exact Part class,
Shape.Block, then cached campaign placement; all six measured errors matched.
Private source reconstruction maps campaign placement IDs, rejects ineligible,
implements512 disclosure/128 impression caps (native defaults/
), duplicate-before-cap semantics, placement ownership, ancestry/
destroy cleanup, and disclosure protected callback data. Focused test covers
these, including callback fields and nil inputObject under current flag.
33 domain tests pass. GetCampaignEligibilityAsync now hands its full cached
payload to this private map; no additional HTTP. Remaining consumers still need
porting: CoreScript disclosure menu/report flow and SDK render/viewability
tracking. Do not register the methods merely for these validators/registries.
Native probes created only invisible/unparented objects with invalid placement0;
no ad displayed or impression tracked. Play stopped.

Commerce source facts: CoreScript generic PrepareCommerceProductPurchase failure
uses a networking toast then CommercePurchaseEnded WITHOUT signaling public
completion. Classified failures use an error dialog; its dismissal invokes
SignalPromptCommerceProductPurchaseFinished(product,false). Native COM-1 after
UI initialization returned nil/no commerce dialog/no completion event, consistent
with generic branch; a toast was not captured before timeout. Engine signal
 always fires two-argument(user,product) completion; didTryPurchase
only controls optional checkout-status polling. Never append a boolean to that
public event. Prepare uses empty body/prompt-purchase endpoint and
response requires nonempty productDetailPageUrl or invalidReason;
opens native purchase browser then returns CheckoutSessionId. Its browser and
status paths are still not implemented/countable. No checkout accepted.

PromptCancelSubscription implemented and online CancelSubscriptionApp. Prompt guard uses common regex ^[a-zA-Z]{1,5}-\d{1,19}$, total <=25, distinguishes missing/invalid IDs; updated shared subscription validator to accept native RBP/other valid prefixes. Server hands prompt to targeted client; duplicate active prompts are ignored. WillRenew/PaymentPending request uses NextRenewTime local-date format; Never/ Expired show Can't Cancel; WillNotRenew shows Already Canceled; failed details show Error. English strings obtained read-only from native CoreScriptLocalization translator. Native EXP-0 showed exact error body; only OK was dismissed.

Protected PerformCancelSubscription does NOT call a cancellation
endpoint: writes local details SubscribedWillNotRenew, ExpireTime=now,
ExpirationDetails.Reason=SubscriberCancelled, and writes local status
(true,false), then success. Port does the same, no cancellation POST exists.
Both local caches have100-entry limit and60s lifetime (native fields
/1198 and/11b0). Success toast onDismissed and Expiring OK
signal MarketplaceService's protected UserSubscriptionStatusChanged(id), not
Players' public event. only fires the local service signal.
Source UIBlox Toast has no duration supplied here; do not invent an automatic
status event/timed close. Error/Unsubscribed/Don't Cancel dismiss with no signal.
Buttons delay input0.75s as source. UI shell is simplified; localized English
strings and controller phases/callback order follow source.

34 domain tests and cargo check/release pass. Actual headless source error UI
with physical OK passed WS_CANCEL_SUB_ERROR_DISMISS_PASS; first click missed
because pre-layout bounds shifted, corrected viewport y390. Cancellation mock
confirmation only ran in a standalone mock Lua VM; no native cancellation was
confirmed and no live subscription changed. Separate batch of read-only native
subscription queries left Server MCP waiting; after stopping Play the sole
owned query client PID2477917 was terminated. No results inferred from it.
Online asset1818 not-for-sale error X now measured PromptPurchaseFinished exactly
(player,1818,false); fixes earlier pre-initialization no-controls observation.

Native post-initialization completion audit extended without accepting anything:
asset1818 and bundle1 each showed not-for-sale Error; X produced exactly
PromptPurchaseFinished(player,1818,false) and
PromptBundlePurchaseFinished(player,1,false). EXP-1 showed generic purchase
failure Error; X produced PromptSubscriptionPurchaseFinished(player,"EXP-1",false).
This closes the earlier uninitialized-UI measurement gaps for these events.
Native Play stopped after all three. Artifacts /tmp/web-social-event-*-finished.json.

TeleportAsyncResult properties now read actual engine instance string storage,
with empty constructor defaults rather than stub nil. Constru
initializes its std::string fields empty; class descriptor and
PrivateServerId reflection binding establish the string property.
35 focused tests pass, including an internally created result (game code cannot
create this NotCreatable class), stored mock server/code strings, empty defaults,
and read-only write rejection. No actual teleport or result was created in live
Studio. Modern teleport success remains unverified under the no-teleport rule.

UserService GetUserFromGlobalUserIdAsync now follows, worker
, callback, and parser.
Fixed POST private-users-service/v1beta1/getOrCreatePrivateUserId body uses numeric
global_user_id/domain_id/domain_type=1; response privateUserId must be positive.
It is deliberately outside the read allowlist: **no live get-or-create request
was made**, because it may allocate a private identity. Localhost fixture verifies
request fields, uint64 preservation, missing/invalid ID, and HTTP errors. Worker
uses a five-second total deadline and login cookie/CSRF handshake. Native error
messages wrap Request failed because, User not found, server/universe/ID guards.
Success cache is per VM/global user indefinitely; no in-flight dedupe. Current
GetUserFromGlobalUserIdThrottlePerMinute=100 measured read-only in online Edit,
with UserServiceGetUserFromGlobalUserId/EnableUserType both true. Counter window
resets strictly after60 seconds, native default; custom hidden per-player overrides
are not configurable here. Mock service checks guards, cache, and throttle reset.

User datatype implemented from pure native constructor probes and shipped docs:
User.fromId one or three arguments; fromString, Id/DomainId/DomainType read-only,
ToString/metatostring, equality. Encoding U1.+unpadded URL-base64 of domain type
byte, big-endian uint64 domain, big-endian signed64 identity; padded input accepts
and canonicalizes. Native explicit domains EXPERIENCE=1/OAUTH=3, no numeric enum
coercion. No identity was allocated to measure this.37 domain tests and focused
account fixture pass. No live identity allocation or published-place edit.

GroupService PromptJoinAsync now ports online Groups.Common.fetchGroupDetails,
fetchGroupActions, postJoinGroup and useGroupJoinPromptConnections. Native
 guards groupId/client/one active request; owns pending
completion. Read-only POST apis/profile-platform-api/v1/profiles/get with
profileType=Community and profileId=number, components header/actions/locked/
deactivated; action refresh requests only Actions. Online flag
AddCommunityDeactivatedToJoinPrompt=true. Full localhost body tests pass; a real
login-session metadata read returns CommunityProfileHeader and JoinCommunity.

Native Place1 UI for group1200769 showed Join Official Group of Roblox, By Games,
2.8K Members, Join Community, Not Now. ONLY Not Now clicked: yielding API returned
Enum.GroupMembershipStatus.None. No group membership changed, Play stopped.
Source cancellation returns AlreadyMember/JoinRequestPending/None from first
profile action. Skeleton opens before HTTP; cancellation suppresses late response;
network/component/deactivated/locked failures reject with source errors and toast.
Join activation waits source ACCEPT_DELAY_SECONDS=2, posts groups/v1/groups/{id}/
users then refetches Actions; Joined/Pending only after backend membership proof.
Mutation helper and confirmation/controller verification are **localhost/mock
only**. Native JSONEncode({}) measured [] and fixed join request uses [] likewise.
Source English strings extracted from packed Common.Locales.en-us (app package
strings are absent from CoreScriptLocalization). UI shell simplified; metadata,
controller phases/results and guards are source-derived.38 domain tests,16 account
fixtures+8 ignored tests and cargo check/release pass. Physical headless Not Now
passed WS_GROUP_CANCEL_PASS, including actual login-session metadata and English
UI assertions; no Join button activated. No new event among the18 game events.

Coverage before this group chunk (/tmp/web-social-after18.json):107/127 functions,
23/24 properties,13/18 registered events. UserService exact invalid-ID/unpublished-
universe/client-side errors corroborated online after implementation, matching
; Play stopped. Native PromptRobuxTransferAsync feature is enabled
on current online Studio (invalid receiver errors before context), unlike old
unreleased assumptions: do not implement it as always disabled.

InternalRequestReviewableContentReviewAsync implemented
and completion. No feature allowlist on this internal action (unlike
creation). Server-only; strict string ReviewableContentId field required (empty
string accepted by native dictionary extraction). Fixed login-session POST to
apis/user/cloud/v2/universes/{universe}/reviewable-content/{URL-encoded-id}:
requestReview?idempotencyKey.key=GUID, application/json body{}. Only HTTP200
succeeds, ignoring its body and returning nil.404 yields exact ReviewableContent
with Id \"id\" does not exist. Others use HTTP status N not OK: message, with
<no returned error string>/<unparseable response body> source fallbacks from
. Transport failures wrap HTTP failure:. No review submitted live.
Localhost verifies route/body/encoding,200/201/204/404/429/503 and parse failures;
mock runtime verifies guard ordering, strict ID, ignored unknown fields, empty ID,
return nil. Current API requires Consequences+InternalTest capabilities as in dump.
Focused tests and cargo check pass. Native Edit empty-config guards were probed
without any content or review ID, so no moderation action could be submitted.

Correction to private ad-disclosure reconstruction: inputObject is **not nil**. Direct assembly and  both load R9 from the saved native param4 (InputObject); the callee forwards R8/R9 to node. Private registry now forwards GuiButton.Activated's actual InputObject to the protected callback; mock test asserts identity. This matches packed DisclosureButtonListenerComponent's required InputObject validation. Methods remain private/not counted until consumers are ported.

Further mutation-boundary tracing: PromptRobuxTransferAsync validates then calls
provider BEFORE opening its client flow. URL uses
/transfer/v1/robux-transfer/experience/{universe}/initiate-transfer with POST JSON
senderId/recipientId/sendAmount/UniverseId. This is an initiation action, not a
read-only eligibility request. No valid native call was made. A transient Play
session used while inspecting the call boundary was stopped without invoking it.
Never infer this method is disabled from old flags or initiate a live transfer
as a verification probe. CurrencyTransfers flow modules are extracted for porting.

CreateReviewableContentAsync follow-up: source accepts absent/non-array Content
as an empty item list; even an empty server config could create an object. Do
NOT probe empty config on Server. Existing Native Edit probe fails server guard
first and was safe. Create response requires path string with a final
nonempty segment; empty/non-string path and malformed JSON have distinct errors.
Media branch checks returned content.items array and matching length,
image/video.uploadUri, connected source Player, then sends that player native
ContentKeyId/UploadUri upload requests and waits. It is not a server media upload.
 send the RPC and arms its timeout. This client
handoff remains to be ported before exposing content creation. No create/upload
was performed live.

Latest release coverage /tmp/web-social-after19.json:109/127 functions,23/24
properties,13/18 registered game events.39 focused domain tests pass.10 registered
events have emitting implementations;3 are source/measured dormant Studio paths.
Five events remain open as listed in the updated18-row audit above. Registration
is not firing proof. Current online Native CanUsersTransferRobuxAsync tested with
a real Player and pure User.fromId(1) returns exactly CanUsersTransferRobuxAsync
is not enabled, confirming that existing guard separately from the enabled
PromptRobuxTransferAsync flag. Play stopped after this read-only eligibility probe.

Event-audit correction: CanSendCallInviteAsync returns true in native Studio,
including Server, as already recorded in the earlier capability probe. The
updated table briefly misstated false; corrected from that evidence. A positive
capability result is not proof that CallInviteStateChanged fires. No call or
invitation was sent.

Current event-read discrepancy investigation: Native Edit accepts numeric1 and
string"1"/"0001" for GetExperienceEventAsync/GetEventRsvpStatusAsync; +1, leading
space, suffix, overflow decimal are invalid. GetExperienceEventAsync(1) resolves
nil on404 while GetEventRsvpStatusAsync(1) rejects Error getting event details:
HTTP404(Not Found). GetUpcomingExperienceEventsAsync resolves an array in Edit.
Source flag ExperienceEventsAllowEditMode controls this extra Edit allowance;
 accepts Client or Edit (do not misidentify IsEdit as Server).
PromptRsvpToEventAsync remains Client-only and its guard messages deliberately
use the GetEventRsvpStatusAsync name. Fixes and deferred response parser tracing
are in progress before the new RSVP controller is exposed.

Event-read fixes completed from deferred callbacks and
RsvpInfo parser. Decimal IDs now coerce number/string, reject nil/cast
failures, zero/overflow beyond signed64, and preserve leading zero IDs in errors.
Read context permits Client or Edit (ExperienceEventsAllowEditMode); prompt stays
Client-only. GetExperienceEventAsync resolves nil on404, invalid response format,
or foreign/missing universe; other HTTP failures preserve HttpError N. RSVP HTTP
failures use Error getting event details: HTTP..., and malformed replies use
Error parsing event details for eventId ID: Invalid response format. Required
RSVP fields include integer universeId, string userRsvpStatus and valid UTC
start/end dates. Existing universe guard retained. Focused mock/native input
matrix and40 domain tests pass. No change to coverage count; existing behavior
was corrected instead of claiming new names. Native current Edit calls were
read-only and returned the recorded404/nil/empty-array outcomes.

Recommendation logging resolved with both artifacts, rather than assuming a backend route. Old native LogAction/Impression/Preference  and online equivalents all build EventStream payloads and destroy them WITHOUT dispatch. Online initializes callback fields zero; destru only destroys callbacks/ fields. Old versions likewise. Their field helpers only validate/append typed payload fields. Compare protected RSVP completion, which DOES call EventStream dispatch  after filling fields; its dispatch routine invokes configured sinks.

Port now exposes LogActionEvent/LogImpressionEvent/LogPreferenceEvent with native
argument conversions, client context, item/tracing/target guards, lowercase UUID
validation, permitted details and typed fields; valid Studio calls discard the
constructed payload. No invented telemetry route, queue or HTTP. Fixture observer
exists only for tests. Native invalid-only probes confirmed enum casts accept
valid numeric/string names, invalid enum errors preserve source type, both action
and impression share the misleading LogImpressionEvent context error, UUID
uppercase/braces/compact forms reject, and details type errors name double/string/
bool. Preference feature RecommendationLogPreferenceEventEnabled=true measured
online. Native empty target errors before constructing an event. All native
probes rejected before payload creation; no valid telemetry operation invoked.
Play stopped after the invalid-input batch.

Action details allow Weight,ReactionType,DestinationPlaceId,CommentText; impression
allows Duration,ItemPosition,Weight,DepartureIntent. Typed numeric fields preserve
fractional weight/duration and truncate integer positions/place IDs; string and
typed DepartureIntent fields follow native checks, wrong known-field types omit.
Unknown keys reject with source allowed-key order. Preference optional empty item/
tracing fields omit and target need not be UUID. Focused test verifies constructed
payloads, all major rejection paths, nil return and no production observer.

41 focused domain tests and cargo check pass after recommendation logging.

PromptRsvpToEventAsync implemented from entry, Info callback
, pending request and protected completion,
plus online VirtualEvents.CoreUI.PromptConnections/RsvpPrompt. Client-only,
uses native GetEventRsvpStatusAsync-named input/context errors, then metadata
HTTP/parse failures, current-universe and started-before-ended checks. Pending
map rejects duplicate prompts after status resolution. CoreScript fetches full
event details again before opening modal; cancellation uses that fresh RSVP
status (not the first read), mapping going/notGoing/other to enum.

Notify Me/Unfollow Event confirms through fixed login-session POST
apis/virtual-events/v1/virtual-events/{id}/rsvps body{rsvpStatus:going|notGoing}.
UI closes immediately while update is pending. Success returns requested enum;
Going shows source English reminder toast; update/detail failure rejects exact
SocialService:PromptRsvpToEventCompleted: Error completing RSVP prompt. and warns
source prefix. Actual update transport and confirmation tests are localhost/mock
only; no RSVP changed live. Default acceptance delay3seconds traced from packed
UIBlox.Core.Button.GenericButton.defaultProps.delayInputSeconds (no RSVP override).
Date text follows source formatDateLocalized's year-sensitive ll/ddd,MMM DD+LT.
UI shell is simplified, with source title/description/status/actions/date.

42 focused domain tests and18 account fixtures(+8 ignored) pass, release builds.
Physical headless source fixture X passes WS_RSVP_CANCEL_PASS and
WS_RSVP_NO_UPDATE_PASS; no confirmation activated and fixture forbids writes.
Native Studio verified stopped/Edit after the probes. A native future event in
the current unpublished universe is unavailable, so positive modal verification
uses the shipped CoreScript and mock event instead of creating an event.

Feedback/shop follow-up (online0.746, unpublished Place1 only): fixed metadata
GETs now in the login-session helper, with exact-route/session mock coverage:
- apis/player-generated-reviews-service/v1/channels/experience-discovery-page/assets/{placeOrResolvedRootPlaceId}
- apis/creator-communication/v1beta1/creator-communication-api/universes/{universeId}/ticket-creation-eligibility
- apis/marketplace-widgets/v1/widgets/shop-widgets?requestId={guid}&universeId={id}
Native feedback and player-support branch;
shop packed InExperienceShop.Api.fetchShopItemIds, buildShopWidgetsUrl.
Explicit read-only session smoke for scratch place10000000000008/universe10000000007
returned feedback capabilities allfalse, support isEligiblefalse with
TICKET_CREATION_ELIGIBILITY_FAILURE_REASON_NOT_RECENTLY_VISITED, and shop Robux
category metadata. No feedback, ticket, purchase or other write was submitted.
18 account tests pass (+9 explicitly ignored live tests); cargo check passes.

Corrected earlier inference: feedback's nil return does NOT mean no dialog.
Native Client PromptFeedbackSubmissionAsync({}) completed nil in0.083seconds
while a generic ScreenGui (no Feedback in instance path) displayed:
Feedback Unavailable / You've reached the feedback limit for now. /
Thanks for sharing your thoughts. Please try again later. / OK and X.
Only OK dismissed. Eligibility callback and network failure
signal unavailable and complete nil immediately, before UI dismissal. Success
capability keys can_submit_review/canSubmitReview, can_submit_comment/
canSubmitComment, can_submit_category_options/canSubmitCategoryOptions, prefer
snake spelling, optionally inside capabilities. Parser accepts bool,
nonzero integer/double, and case-insensitive string true/false/1/0. Invalid JSON
or unknown fields default reviewfalse/commenttrue/categoryfalse. Native reason
also depends on Player under13 state, which this
port does not yet receive from its player initialization. Do not silently infer
it from the public user profile. Native HTTP failure reason is RateLimited.

Current native FastFlags: PlayerFeedbackPromptEnabled=true,
EnablePlayerSupportPrompt=false, SkipShopWidgetsCallInUnpublishedStudio=true,
HideShopMenuOnFailure=false. PlayerFeedbackPromptWatchdogSeconds=30 confirmed.
Thus current feedback options' FeedbackType is ignored by the native entry.
Native submit accepts10..1000 bytes; CoreScript constants impose
10..500 and2seconds acceptance delay. SignalFeedbackSubmissionPermissionDenied
clears in-flight state and completes nil; POST success/failure/
 also both complete nil. Submission targets
.../assets/{id}/reviews with category_type InExperience, category_option_ids[],
comment string. It was traced statically only; never invoked live. UI/controller
and local player age state remain to be connected before registering the API.

Native OpenShop(LocalPlayer) returned nil and opened the Chrome window
WindowHost:in_experience_shop, title Shop, search/ellipsis/X controls, no items
in unpublished Place1 (fetchShopItemIds skips its request there). This is an
actual frontend consumer, not a nil Studio no-op. OpenShop engine validator
 warns for null/guest/nonlocal players; dispatches the
local protected OpenShopRequested signal or sends it to the server's target.
No shop buttons or purchase actions activated. Missing port still includes
shop frontend/catalog interactions and internal Robux product purchase handoff;
adding only the protected signal would falsely claim implementation.
Play stopped and Edit confirmed in /tmp/web-social-feedback-shop-final-state.json.

Feedback controller is now installed as PromptFeedbackSubmissionAsync. Native
entry, eligibility conversion and CoreScript dialog lifecycle are implemented:
root-place lookup with positive first-result fallback; one pending API request;
duplicate calls complete nil; unavailable UI queues independently after nil
completion; eligible UI Cancel clears the pending call; text Submit10..500bytes
and2seconds input delay; native30second watchdog releases the call without
closing a stale UI; stale text submissions after timeout send nothing. Current
EnablePlayerSupportPrompt=false means options.FeedbackType is ignored. Native
CoreScript createApp supplies enableInputDelayed=true in its table constant.
Native reflection probes confirmed bool/double/string/Array/Function/Vector3
Dictionary cast error spelling. No new public event: feedback signals/hooks are
RobloxScript-protected and not part of the18 game events.

Eligibility JSON supports snake/camel keys, numeric and string boolean values,
malformed-response defaults, native Under13/RateLimited classification. HTTP
failures display RateLimited and complete nil. Invalid JSON on HTTP success is
kept separate from network failure for the under13 classification. The private
Player Under13 field defaults false and an engine-supplied bool is preserved;
current login Profile does not populate that field. Session age propagation is
still a known limitation for accounts under13. UI is a simplified English shell,
with shipped title, intro, footer, placeholder and unavailable text, text entry,
Submit/OK and X. Native viewport/source handles styles, focus and animation more
fully than this shell. API has no invented purchase/report completion event.

Native root success worker parses data[1].rootPlaceId, falls back to
current place, then requests eligibility. It does not persist that result into
the DataModel or the service; do not add an invented response cache. Submission
worker uses service+378 if positive, otherwise current PlaceId. This
entry initializes+378 tozero and the traced eligibility/root callbacks do not
set it, so the port retains the current-place submission fallback separately
from the eligibility target. No live submission was used to test this branch.

A separate fixed submission helper posts category_type InExperience,
category_option_ids[],comment; JSON escaping and HTTP429 are verified only on
localhost. Success and failure both release the API with nil, as native POST
completion do. All feedback development confirmations use
mock transport. Native online measurement only dismissed the unavailable OK.
Headless public-API login-read fixture displayed the same unavailable text,
completed nil before dismissal, and passed WS_FEEDBACK_DISMISS_PASS after the
physical dismissal run (/tmp/web-social-feedback-ui-debug.log). No Submit was
activated in the native or headless app.44 focused domain tests pass with
--test-threads=1;19 account fixtures pass (+9 ignored). One parallel domain run
hit SIGILL; the serial run passes. cargo check/release build pass.

Bulk fulfillment transport is isolated in the account helper, with an explicit
Studio test-token gate. posts the quoted order to
apis/cloud/v2/avatar-marketplace-orders?idempotencyKey.key={GUID}; native
 supplies studio-bulk-purchase-{GUID} if the quote lacks its token.
The port refuses non-Studio authorization before any HTTP.20 account fixtures
pass, including localhost-only body/status/rejection tests; no live order sent.
This does not complete PromptBulkPurchase: its UI/state/result mapping and
Robux browser upsell are still unported. Backend quote items use
purchaseableState/Purchaseable; shipped useBulkPurchaseRequestListenerEffect
normalizes these to purchasableState/Purchasable at the UI boundary. The draft
failed to model that boundary.
Do not reuse the draft's guessed success calculation. Native results map the
first fulfillment group's lineItems in order onto Purchasable items only;
RobuxSpent sums successful item priceRobux, not orderTotalRobux.

Experience-details investigation: Native online Client call for scratch
universe10000000007 opened InExperienceDetailsPromptApp with the error
This experience cannot be visited at this time. Only its X was clicked; the
waiting API returned Enum.PromptExperienceDetailsResult.PromptClosed. Duplicate
calls for0,-1,and scratch ID while pending all rejected with
TeleportService::PromptExperienceDetailsAsync: already has prompt in progress
for this Player. Native CoreScript InExperienceDetailsPromptApp.onJoin first
calls protected completion(TeleportAttempted), then calls legacy Teleport(root),
then closes UI. That ordering means a thrown Studio teleport error prevents its
final UI close; do not wrap this in pcall/finally and silently change behavior.
No join/teleport action was invoked. Play stopped after the native dialog cancel.

PromptExperienceDetailsAsync now installed plus shipped
InExperienceDetailsPromptApp/container/DataHydration. Player argument, client
context, LocalPlayer, pending-per-player errors match source/native measurements;
there is no invented positive-universe validation. It yields until a completion.
X returns PromptClosed. Join first completes TeleportAttempted, then invokes the
already implemented legacy Teleport; a thrown Studio error leaves UI open, as
source onJoin does (no finally-close). This branch is verified only with a mock
teleport, never by accepting a native Join action.

Four login-session metadata requests, exact-route mock coverage and real read
smokes: games/v1/games?universeIds={id}, games/v1/games/multiget-playability-status,
games/v1/games/votes, and read-only POST
apis/experience-guidelines-api/experience-guidelines/get-age-recommendation with
universeId string (the source app converts its ID to a string before hydration).
Current native Client LuaAppThrottleRefreshPlayabilityIntervalSeconds=30,
LuaAppDefaultDataTtlSec=3600, LuaAppDefaultFailedDataTtlSec=60 measured after UI
initialization. This overrides the shipped throttle default1800. Info/playability
cache30s, age/votes3600s, failures60s; pending lookups share their result. Each
Play VM has independent storage. Native games info trims names; vote percentage
uses floor, not round. UI shell shows title/creator/maturity/rating/description,
source unavailable/network/status text, Join/Unavailable and X. It omits native
image loading, animation, rich age/parental/paywall action subdialogs and layout.
Those action subdialogs remain a fidelity limitation beyond ordinary Join and
unavailable presentation; no external settings/purchase flow was exercised.

Real scratch metadata reports PlaceHasNoPublishedVersion despite its allocated
universe/place IDs. It is not a currently playable published-place fixture. No
place was published/saved or changed to obtain a positive join. Native main
account scratch displayed the error-only sheet; DOORS2440500124 displayed its
name, LSPLASH, Maturity:Mild,93% and an Unavailable action. Both native X cancels
returned PromptClosed; no join invoked. Headless current-session scratch showed
its metadata and unavailable reason; physical X passed WS_DETAILS_METADATA_PASS,
WS_DETAILS_CLOSED_PASS and WS_DETAILS_DISMISS_PASS.46 focused domain tests and
20 account fixtures(+10 ignored) pass, check/release pass. Native Play stopped.

Flag measurement correction: Lua-defined flags are not necessarily registered
in the Edit DataModel. GetFastFlag there reported undefined for ShareSheet flags;
that is NOT false. In initialized native Client EnableShareSheetForBuild=true,
EnableLinkSharingEvent=true and LuaAppInExperienceDetailsPrompt3=true. Therefore
ShareSheetClosed still requires a real frontend completion path; do not register
it as a dormant/off-feature signal on the basis of Edit's undefined result.

Shop investigation removed one suspected dependency: shipped PurchasePrompt.
launchRobuxPackagePurchase line292 returns nil immediately if RunService:IsStudio(),
before payment-session redemption or a native/webview purchase. Its Robux tile
activation has no cash purchase in Studio. Source-only evidence; no native shop
item or purchase action clicked. OpenShop still needs its catalog/controller port.
Latest coverage /tmp/web-social-after21.json recognizes115/127functions,23/24
properties,13/18events. One of those function registrations is the pre-existing
TeleportAsync stub; these totals are not a claim of complete behavioral parity.

Additional native Edit argument probes corrected PromptExperienceDetailsAsync's
binding-layer errors: bool/number/string player arguments throw Unable to cast
value to Object; a Folder throws Argument 1 is invalid: expected 'Player'
instance type. Source engine null-player text is not the public binding's error.
OpenShop is different: primitive args have Object cast errors, but Folder casts
to its null-player branch, logs the null warning and returns nil. No UI/action
opened by these invalid-argument probes. Focused details tests/check pass.

OpenShop now has its actual frontend consumer. Entry validation follows
: null/wrong-class instance logs player is null and returns nil;
primitive arguments throw Unable to cast value to Object; guests/nonlocal
players warn and return. Server calls route only to the requested local player
through the existing engine message transport; client repeats do not recreate an
already open window. No public completion event is invented.

Current initialized Native Client flags measured: EnableShopPrefetch=true,
ShopPrefetchProductInfo=true, ShopPrefetchDelayMs=0, ShopPrefetchMaxRetries=0,
ShopPrefetchRetryBackoffMs=1000, ShopItemsPerSection=6, ShopMaxSearchResults=50,
ShopProductsUseDisplayName=true, SkipShopWidgetsCallInUnpublishedStudio=true,
HideShopMenuOnFailure=false, BlockRobuxTileClickInShop=true, ShopUseFuzzySearch=false,
EnableRobuxInExperienceShop3=true, EnableBonusJwtInShop=true. GetFastFlag of
EnableRobuxInExperienceShop without its3 suffix is undefined, not false.
Native Play stopped after flag reads, no shop item activated.

Client prefetch/cached catalog implemented through fixed ShopWidgets read;
unpublished universe0 skips HTTP and yields the same empty Shop window.
Catalog product hydration uses existing GetProductInfoAsync (GamePass/Product)
and UserOwnsGamePassAsync reads; response order survives asynchronous metadata
completion, product IDs normalize from ProductId, display names take precedence,
Robux currency formats to two decimals. Category selection, six items per home
section, case-insensitive substring search,250ms input debounce,50-result cap and
search-only dedup follow current source flags/constants. Unsupported categories
without supported items are dropped. Cached loaded catalog survives closes;
failed initial retrieval can retry on a later open. Game-pass completion verifies
ownership after the source250ms initial polling delay before marking Owned.

Virtual item actions hand off to existing Studio test purchase prompts. Robux
tile activation is deliberately inert: current BlockRobuxTileClickInShop=true;
independently PurchasePrompt.launchRobuxPackagePurchase line292 IsStudio returns
nil before any payment/bonus POST. This is a source-supported Studio path, not a
missing browser substitute. No cash/browser purchase launched during development.
UI is a simplified native-GUI shell rather than the full Chrome integration;
images, promotions/bonus presentation, item detail sheets, responsive layout and
long-running ownership polling retain visual/fidelity differences. No fake
purchase success is synthesized by the shop.

48 focused domain tests, cargo check and release build pass. Actual headless
server OpenShop reached its target client's empty unpublished window, and
physical X passed WS_SHOP_SERVER_RETURN_PASS, WS_SHOP_EMPTY_WINDOW_PASS and
WS_SHOP_DISMISS_PASS. Mock catalog tests cover ordering, metadata cache, search,
owned refresh, virtual-purchase handoff and zero Robux purchase actions. No native
or headless purchase accepted. Latest coverage /tmp/web-social-after22.json:
116/127functions,23/24properties,13/18events recognized, including the previously
noted TeleportAsync false positive. Remaining missing functions are not all in
this domain: four TextService layout methods belong to gui_input.

Bulk result mapping now has a private source-backed module, deliberately not
registered as a public method/event until its controller is complete. Tests
cover positional mapping onto only purchasable items, SUCCEEDED precedence over
errorReason, all11 error reasons, first fulfillment group only, missing/truncated
response groups, zero spending on abort, per-item successful spending, and
Completed for mixed results versus Error for a common failure. Native quote
spelling is preserved and normalized at the CoreScript boundary. Empty statuses
have no common status, so current CoreScripts select PartialSuccess, not Success.
Error-dialog dismissal signals3; partial-success dismissal signals1.
Additional source correction: requires PurchaseOptions arrays
to contain exactly1 entry (including rejecting an empty array); its exact error
is Input contained a badly formed line item. Item N PurchaseOptions must
contain exactly 1 entry, but X were provided. Missing/non-array options are
ignored. This is static evidence, not a live fulfillment observation.

## Final handoff — lead time limit

Work stopped at the lead's explicit wrap-up request; the entries below are left,
not queued work. All owned Rust edits are committed. The last bulk chunk is a
private result-mapping module with two passing focused tests; it registers no
unfinished API or event. The domain had48 passing serial tests before this chunk;
the two additional bulk tests pass. Account suite:20 pass,10 intentionally ignored
live fixtures. No additional live action, prompt, dialog or probe was started.
The final safety state check confirmed unpublished Place1 is stopped in Edit.

Final requested verification: cargo check passed once; cargo build --release
passed; the release binary ran --api-coverage exactly once under
RUSTUDIO_NO_AUDIO=1 gamescope --backend headless. Logs:
/tmp/web-social-wrap-check.log, /tmp/web-social-wrap-release.log,
/tmp/web-social-final-coverage.log; result /tmp/web-social-final-coverage.json.
Other agents' dirty files were left untouched. No new live write was performed.

| Scoped metric | Before | Final |
|---|---:|---:|
| Function coverage |13/127|116/127|
| Property coverage |14/24|23/24|
| Required events registered |3/18|13/18|

Baseline event3 is reconstructed from the original runtime_ext event table
(Purchase/GamePass/Product); coverage-start.json predates per-class event output.
Event audit covers all18:10 have implemented emitting paths,3 match measured
Studio-dormant behavior (Premium,PhoneBook,Points),5 remain open. Registration is
not a claim that all18 fire. Function coverage retains the pre-existing
runtime_ext TeleportAsync warning/nil stub: it is NOT completed work. Excluding
that known false positive gives12→115 functions. Four missing TextService
layout functions are assigned to gui_input, not this domain.

Done (one line per implementation group):
- Login-session service reads: native endpoint builders, returned schemas, batching/cache/error handling, mock HTTP contracts and live read-only service fixtures are implemented and recorded above.
- Studio purchases: five ordinary prompt controllers, Plus/Premium routing, receipt handlers, source-derived receipt/event ordering, cancellation and peer completion transport are implemented; native verification only cancels.
- Social prompts: invite cancellation/replication, desktop phone-book result, RSVP, feedback eligibility/dialog flow and link-preparation tuples are implemented with action tests restricted to mocks.
- Shop and details: catalog metadata/read caches, target-player shop routing and experience-details metadata/dialog completion are implemented with headless physical dismissal checks.
- Other services: Studio badge/points/ad failures, local filtering, recommendation reads/mutations/log guards, matchmaking state, private User values and internal moderation review transport are implemented as documented above.
- Teleport: legacy Studio error/event order, settings/options, result getters, reserve/read transport and details prompt are implemented; modern pipeline is excluded below.

Left (one line each, including known fidelity limits):
- MarketplaceService.PromptBulkPurchase / PromptBulkPurchaseFinished: quote and Studio-authorized fulfillment helpers plus private result mapping are implemented; full controller, native binding/options, browser upsell and peer event transport remain unported at cutoff.
- MarketplaceService.PromptRobuxTransferAsync: native initiation POST and guards traced; transfer request parsing, client UI and completion flow remain unported; valid live initiation was prohibited.
- TeleportService.TeleportAsync: pre-existing false-positive stub remains; native modern manager performs preflight/reservation before Studio failure, and that pipeline has not been ported.
- TeleportService.TeleportPartyAsync / LocalPlayerArrivedFromTeleport: modern manager and arrival handoff remain unported; actual live teleport verification was prohibited.
- AdService.RegisterDisclosureButton: private registration/InputObject guards are implemented, but disclosure-menu/report consumer remains unported, so public API is not registered.
- AdService.RegisterImpressionSource: private registry/ancestry guards exist, but native renderer/viewability/impression consumer remains unported; no real impression was generated.
- CommerceService.PromptCommerceProductPurchase / PromptCommerceProductPurchaseFinished: preparation and CoreScript classifications traced; browser host/checkout-close completion remains unported; no checkout was accepted.
- ModerationService.CreateReviewableContentAsync: request/response and per-player upload RPC traced; client capture lookup, presigned upload, timeout and completion pipeline remain unported; no object or media was created/uploaded live.
- SocialService.CallInviteStateChanged: native capability is true, but platform call lifecycle/event bridge remains unported; no call or invitation was sent.
- SocialService.ShareSheetClosed: enabled native share listener uses ExternalContentSharingProtocol clipboard/shareUrl; platform completion/event bridge remains unported, so signal is not falsely registered.
- AdPortal.PortalInvalidReason: field is absent from measured native0.746; retained as a version mismatch rather than inventing a value.
- TextService.GetTextSize / GetFamilyInfoAsync / GetTextBoundsAsync / GetTextSizeOffsetAsync: four layout members belong to gui_input; no web_social implementation was added.
- Feedback fidelity: private Under13 metadata is not populated by current login Profile; disabled PlayerSupport UI branch remains unported; ordinary review flow is implemented.
- Details fidelity: rich parental/age/paywall subdialogs and image/layout presentation remain unported; ordinary unavailable/Join completion ordering is implemented.
- Shop fidelity: promotions/bonus presentation, item detail sheets, images, responsive layout and prolonged ownership polling remain simplified; current Studio Robux activation returns before purchase transport.
- Subscription live validation: no authorized valid subscription fixture was available; negative live fixture plus native source and localhost positive contracts were used.
- Service throttling/cache completeness: native product/gamepass ownership batching/defaults are covered; remaining per-service rate-limit/cache fidelity is not exhaustively established.
- Moderation event processor: local binding/disconnection is implemented; backend ingestion into reviewable-content processors remains unported.

Domain implementation commit range begins59a239e (plus account helpere88e95e)
and ends32a7585 before this documentation-only handoff commit. Use path-filtered
git log for the48 domain commits; shared history also contains other agents' work.
