# Team Create

Current implementation (2026-11-05): direct QUIC joining, initial DataModel loading and cross-account object synchronization work. Protected script Source round trips and native atomic script commits passed in the user's authorized Place B Studio. The editor keeps private recoverable drafts and offers server comparison on conflicts. Live save/publish requests, collaborator lists, selection, idle presence and fly-to are integrated. Save/publish encoding is tested; this work did not publish the user's game. The phased plan below is historical.

How Roblox Studio finds, opens and runs a live Team Create (TC) session, and
what rustudio needs to do the same. Research of 2026-11-01 (first pass) and
2026-11-04 (this rewrite). Nothing here turned Team Create on or off, created
anything, or opened a live session.

Every fact is tagged:

- **[verified: live]** the live Roblox service answered it to rustudio's cookie session.
- **[verified: lua]** read out of the built-in plugin bytecode (constants, table
  templates, call order), citing `Plugin/Module` and the Lua line from the debug info.

- **[verified: offline-studio]** seen in an offline Studio's mock request log (`~/.local/share/rustudio/run/requests.log`, `bodies.log`)
  for the offline Studio, which is the same build.
- **[inferred]** follows from the above but was not observed directly.
- **[recalled, unverified]** public knowledge from memory, not checked here.

## Headline

1. **Listing, status and permission checks are plain REST that accept the
   `.ROBLOSECURITY` cookie.** Open Cloud is not involved.
2. **Opening a TC place is a game join.** Studio calls
   `POST gamejoin.roblox.com/v1/team-create` and gets back the same kind of
   join script a game client gets: RCC address and port, a client ticket, a
   session id, UDMUX endpoints, a token, and an RbxTransport (QUIC) token and port.
3. **The live session is a normal Roblox replicator connection to an RCC
   server running the place in "CloudEdit" mode**, over RakNet with Roblox's
   own encrypted handshake (`RbxOpenRequest1/2`, `SessionCrypto`), or over
   RbxTransport, which is QUIC (ngtcp2 + OpenSSL, WebTransport optional).
   Every collaborator is a `Player` on that server.
4. **The server saves, not the client.** The RCC autosaves every 240 s and on
   request (`TeamCreatePublishService`), and uploads the place itself.
5. **@testuser2 cannot join any of the user's places yet.** It is not a
   collaborator (`join` answers `403 RccForbidden`) and has no tenure.

Built-in plugins are Luau **bytecode**, not source.

---

## 1. Listing "Shared with me" and group experiences

The Start Page plugin (`BuiltInStandalonePlugins/StartPage.rbxm`) does this.

### Endpoint

```
GET https://apis.roblox.com/universes/v1/search
    ?creatorType=<User|Team|Group>&creatorTargetId=<id>
    &search=<text>&isArchived=false&isTemplates=false
    &sortParam=LastUpdated&sortOrder=Desc
    &pageSize=10&surface=StudioStartPage&PageIndex=0
    &needsAssetOptions=true&needsReachStatus=true
```

- URL: `StartPage.Src.Network.Urls` L17 `EXPERIENCES_SEARCH_URL = APIS_URL + 'universes/v1/search'` [verified: lua].
- Query builder: `StartPage.Src.Util.getQueryString` L18-48. Keys: `search`,
  `creatorType`, `creatorTargetId`, `isPublic`, `isArchived`, `sortParam`,
  `sortOrder`, `pageSize`, `pageIndex` [verified: lua]. `DiscoverExperiences`
  L123-128 appends `&surface=StudioStartPage&PageIndex=%d&PageSize=%d&needsAssetOptions=true&needsReachStatus=true` [verified: lua].
- Tab to query, from `StartPage.Src.Foundation.Components.ExperiencesPage` [verified: lua]:

  | Tab | Template | `creatorTargetId` |
  |---|---|---|
  | Owned (L201) | `{creatorType='User'}` | your user id (`StudioService:GetUserId()`) |
  | **Shared with me** (L320) | `{searchKey='Experiences', creatorType='Team'}` | **your user id** |
  | Group (L258) | `{creatorType='Group'}` | the group picked in the GroupDropdown |

  So **"Shared with me" is `creatorType=Team` plus your own user id.**
- The exact string Studio sent for Owned [verified: offline-studio]:
  `apis.roblox.com/universes/v1/search?isTemplates=false&sortOrder=Desc&search=&creatorTargetId=100000002&isArchived=false&sortParam=LastUpdated&pageSize=10&creatorType=User&surface=StudioStartPage&PageIndex=0&needsAssetOptions=true&needsReachStatus=true`.

### Response

- As @testuser2 [verified: live]:
  - Team: `200 {"data":[],"totalResults":0,"totalHits":0,"nextResultIndex":null}`.
  - User: the same.
  - With `creatorTargetId` set to another user: `401 {"code":"Unauthorized","message":"Authenticated user does not match target"}`.
    So you can only list your own Team and User experiences.
- Item shape, from `StartPage.Src.Models.UniverseModel` [verified: lua]:
  `id:number, name, description, rootPlaceId:number, privacyType:string,
  isFriendsOnly:bool, audiences:[number], creatorTargetId:number,
  creatorType:string, creatorName, created, updated, reachStatus{…}`.
  The envelope `SearchUniversesResponse` is `{data:[UniverseModel], totalResults:number}`.
  The live service also returns `totalHits` and `nextResultIndex`.
- No populated example exists yet: @testuser2 owns nothing and has
  nothing shared with it.

### Groups for the dropdown

`GET https://apis.roblox.com/creator-home-api/v1/groups?surface=StudioStartPage`
returns `200 {"groups":[]}` [verified: live; URL `Urls` L52, caller `DiscoverGroups`].
Each group then feeds `creatorType=Group&creatorTargetId=<groupId>`. The older
`develop.roblox.com/v1/user/groups/canmanage` (`GROUPS_SEARCH_URL`) also exists [verified: lua].

### Places in a universe

`GET develop.roblox.com/v2/universes/{id}/places?limit=50&SortOrder=Asc` [verified: offline-studio].

## 2. Is Team Create on, and turning it on or off

### Status

- `GET https://develop.roblox.com/v1/universes/multiget/teamcreate?ids=<u1>&ids=<u2>&`
  - URL `Urls` L115 `MULTI_TEAM_CREATE_STATUS` [verified: lua]. The exact request (trailing `&` included) [verified: offline-studio].
  - Studio reads `data[].isEnabled` into `IsTeamCreateEnabled` (`DiscoverTeamCreateStatus`) [verified: lua].
    Entries are probably `{id, isEnabled}` [inferred].
  - For a universe you cannot edit, the entry is dropped: as @testuser2
    on 100000004 it returned `200 {"data":[]}` [verified: live].
  - Native Studio uses the same path, `/v1/universes/multiget/teamcreate?{}`, batched by `MultiGetTeamCreateStatusesBatchSize` [verified: exe].
- Who is in a live session right now:
  `GET https://apis.roblox.com/teamcreate/v1/MultiGetUniversesActiveSessionMembers?maxUsers=10&ids=<u>…`
  returns `200 {"data":[{"id":100000004,"access":1,"activeUsers":[]}]}` [verified: live; request shape verified: offline-studio].
  `activeUsers` is a list of `ActiveUserModel` with a user id, used for the avatar row on the start page (`DiscoverTeamCreatePresence`) [verified: lua].
  What `access` means is unknown.
- Per place (native): `GET develop.roblox.com/v1/places/{placeId}/teamcreate/active_session/members`
  returns `403 "Not authorized to perform this action."` to a non-collaborator [verified: live; path verified: exe].

### Turning it on or off (documented only, never called)

- **Off, from the Start Page:** `ChangeTeamCreateEnabled` (L28-46) sends
  `POST develop.roblox.com/v1/universes/{universeId}/teamcreate`,
  `Content-Type: application/json`, body `{"isEnabled": false}`
  (`TEAMCREATE_CHANGE_ENABLED`, `Urls` L164) [verified: lua]. It is gated by
  `getFFlagStartPageAllowDisableTeamCreate`. Note that the plugin's Framework
  client calls `post`. Roblox's public API docs list it as
  `PATCH /v1/universes/{universeId}/teamcreate {"isEnabled":bool}` [recalled, unverified].
  rustudio should copy whatever a capture shows.
- **On, native:** the Collaborate widget → `RobloxTeamCreateWidget::dispatchTurnOn`.
  Log lines: "TurnOnTeamCreate Pressed", "Commit Finished - About to turn on TeamCreate",
  "Succesful Publish - Beginning Enable TeamCreate", "Enable TC failed: {}" [verified: exe].
  The Lua-callable entry points are `CollaboratorsService:ToggleTeamCreate(on)`,
  `StudioPublishService:PublishThenTurnOnTeamCreate()` and
  `SetTeamCreateOnPublishInfo(shouldTurnOnTcOnPublish, newPlaceName)` [verified: api-dump.json].
  So turning it on means: publish the place if needed, then the same
  `develop /v1/universes/{id}/teamcreate {"isEnabled":true}`, then reopen the
  place as a TC session [inferred from order of log strings].
- **Remove a member:** `DELETE develop.roblox.com/v1/universes/{id}/teamcreate/memberships`, body `{"userId": n}` (`DeleteUserFromTeamCreateUniverse`) [verified: lua].
  A GET on that path returns 404 [verified: live].

## 3. Opening a Team Create place

### Order of calls when you click a TC experience

1. **Safety and eligibility** (`DiscoverCreatorEligibilityAndCheckTCAndOpenPlace` L53-171) [verified: lua]:
   - `GET apis.roblox.com/content-safety/v1/places/{placeId}/safety-status`.
     `userPlayabilityRestrictions == RestrictedForAll` blocks the open.
   - `POST apis.roblox.com/experience-guidelines-service/v1beta1/multi-creator-eligibility` `{action:1, userIds, universeId}`.
   - `POST apis.roblox.com/team-create-service/v1/GetCanCollaborate`, header
     **`robloxctx-authenticated-userid: <your user id>`**, body `{"universeId": n}`.
     Reply `CanCollaborateResponse {CanCollaborate:bool, Error?:string, UserId?, RequiresTrustedConnection?:table, UniverseId?}`.
     `Error` is one of `Invalid, None, NotAgeVerified, OutsideAgeBucket, TooManyCollaborators, PCBlock, NotFound, OutsideOwnerAgeBucket` (`Models.CanCollaborateError`).
     Not called here: it is a new POST.

     The `robloxctx-*` header is the likely reason the old probe of
     `MultiGetCanCollaborate` / `hasTenureBulk` got
     `403 "Request Context Failure"` [inferred]. In Studio those two are made
     by `CollaboratorsService::serverMultiGetCanCollaborate` and
     `tenureEvictionCheck`, which run on **RCC**, and RCC has a request
     context that a user client lacks [verified: Studio's messages; inferred: the cause].
2. **Pre-warm** `POST gamejoin.roblox.com/v1/team-create-preemptive`
   `{"gameJoinAttemptId":"<GUID>","placeId":<id>}` (`PreLaunchTeamCreate` L25-46, flag `LuaStartPagePreLaunchTeamCreate`) [verified: lua].
   Offline Studio also sends it with `placeId:0` for "Server & Clients" tests [verified: offline-studio].
   Reply `TeamCreateResponse {status:number, message:string}`.
   It probably starts an RCC early. **It changes state, so do not call it.**
3. **Native open.** `PlaceManager` → `TeamCreateManager` → `CloudEditConnectionModel::connectToTeamCreateSession`.
   Progress markers, in order: `PreGameJoin`, `GetPlaceInfoSynchronous`,
   `Matchmaker`, `RCCConnection`, `RCCConnectionAccepted`, `DMInitialStream`
   (`StudioOpenPlace.Progress.TeamCreate.*.End`) [verified: exe].
   - **Matchmaker = `POST gamejoin.roblox.com/v1/team-create`**: path in the
     URL table, next to `/v1/teleport` [verified: exe]. Its
     generated client is `postV1TeamCreateAsync` in the
     `OpenApiGameJoinApiv1` package (host `gamejoin`) [verified: lua].
   - Request `TeamCreateRequest` [verified: lua, `Models.TeamCreateRequest`]:
     `gameJoinAttemptId:string (GUID), placeId:number`, and optionally
     `gamerTag, isPlayTogetherGame, browserTrackerId, isTeleport,
     isQueueAllowedOverride, isImmersiveAdsTeleport, channelName, joinOrigin, partyId`.
   - Like a normal join it is polled: timeout `FIntStudioTeamCreateGamejoinRequestTimeoutMs=10000`,
     `StudioTeamCreateGamejoinRequestRetryLimit`, `StudioTeamCreateRetryOnUnauthorized` [verified: exe + flag snapshot].
     The response is a `GameJoinResponse {jobId, status, statusData, joinScriptUrl, authenticationUrl, authenticationTicket, message, joinScript, queuePosition}` [verified: lua model; that `/v1/team-create` returns this type is inferred].
     As for game joins, `status` presumably means 0 or 1 = wait and poll,
     2 = ready [recalled, unverified]. Failure texts: `JoinErrorServerStartup`,
     `ServerFull`, `Moderated`, `Unauthorized`, `InvalidRequest`, `RccChannel`,
     `GamejoinGeneric` [verified: exe].
4. **Join payload.** The `TeamCreateJoinPayload` parser at about
   to reads these keys, in order [verified: exe]:
   `GameId, UniverseId, PlaceId, DataCenterId, MachineAddress, ServerPort,
   User…, ClientTicket, SessionId, ServerConnections[{Address,Port}],
   UdmuxEndpoints[{Address,Port}], DirectServerReturn, TokenValue,
   NetStackTokenValue, NetStackPort, UserName, IsUnknownOrUnder13`.
   It then logs `Joining game {} place {} at [{}]:{}`. The game-client parser
 reads the same set plus `GameFqdn`,
   `ClientPort`, `ClientPublicKeyData`, `EphemeralEarlyPubKey`.
   The camelCase `JoinInformation` model lists the full set, including
   `tokenGenAlgorithm, pepperId, clientPublicKeyData, ephemeralEarlyPubKey,
   rccVersion, sessionId` [verified: lua].
   - So what comes back is: **RCC IP and port, UDMUX relay endpoints, a RUPP
     token (`TokenValue`), an RbxTransport token and port
     (`NetStackTokenValue`/`NetStackPort`), a client ticket, and the server's
     ephemeral early public key.**
   - **There are no SessionCrypto keys in it.** Those are negotiated in the
     handshake.
5. `team-create-service/v1/collaboration/join` is the permission and tenure
   check (`CollaborationJoinResultModel`). RCC also uses it as a fallback
   ("Join fallback failed for user {}") [verified: exe]. It does not return an address.

## 4. The connection

### Transport choice

From the `NetworkClient` log lines [verified: exe]:

- `Transport selection: useRbxTransportEnabled=…, rbxTransportHasValidPort, …ValidAddress, …ValidPubKey, canUseRbxTransport, selectedTransport=…`.
  Either transport can be tried first, with the other as fallback
  ("Failed to connect … with RakNet, falling back to RbxTransport", and the reverse).
- The flag snapshot (`offline-studio/mock/StudioAppSettings.json`) has
  `FFlagUseRbxTransportClient3=True`,
  `FFlagEnableRbxTransportEphemeralEarlyPubKeyCloudEditClientLoading2=True` and
  `DFFlagRbxTransportSessionCrypto=True`.
  So TC is expected to use **RbxTransport** where the join payload carries
  `NetStackPort` and token [inferred].

### RakNet path

[verified: Studio's messages]

- Roblox's fork of RakNet. It does the stock offline handshake
  (`OpenRequest1` with MTU probing, `OpenReply1`, `OpenRequest2`, `OpenReply2`,
  then `ID_CONNECTION_REQUEST` and `ID_CONNECTION_REQUEST_ACCEPTED`,
  `ID_NEW_INCOMING_CONNECTION`), except that requests 1 and 2 are Roblox's
  `RbxOpenRequest1/2` and `RbxOpenReply1/2`.
- `RbxOpenRequest2` carries a sealed box encrypted to the server's early public key:
  "declared box size", "key exchange version incorrect", "desired encryption is unset",
  "unexpected aad size", "mac was previously used".
- The server key is distributed as a client-setting key ring:
  `{"applications":{"RakNetEarlyPublicKey":{"versions":[{"id":5,…}],"send":5,"revert":5}}}`.
  The per-session one is `RbxTransportEphemeralEarlyPublicKey`, which comes in
  the join reply (`ephemeralEarlyPubKey`). The client key is `clientPublicKeyData`.
- After the handshake every datagram is AEAD-encrypted. `SessionCrypto` and
  `RakNetMissingPingPong` count "Decryption failed".
- A connection without a SessionCrypto entry is dropped before a replicator
  exists: "ID_NEW_INCOMING_CONNECTION: SessionCrypto not found".
- `IncompatibleProtocolVersion` / "RbxOpenRequest1 Incompatible Protocol" mean
  the protocol byte must match.
- The exact primitives are not pinned down. The bundled OpenSSL has
  X25519 and ChaCha20-Poly1305/AES-GCM, and there is no libsodium string
  [verified: exe]. Which one SessionCrypto uses is [unknown].

### RbxTransport path

[verified: Studio's messages]

- QUIC through **ngtcp2** with OpenSSL TLS (`ngtcp2_crypto_ossl_configure_client_session`).
  It can also use WebTransport (`DebugRbxTransportClientUseWt`, "Initiating
  WebTransport session") and a post-quantum key share (`X25519MlKem768`).
- Separate channels carry reliable streams and unreliable datagrams
  (`RbxTransportChannelTracker` fragment and reassembly,
  `OpenUnreliableChannelControl`), plus ping and time-sync channels.
- It connects to the RCC address and `NetStackPort` directly, or through
  UDMUX with a RUPP token: "Client will connect to server {}|{}, udmux {}|{}",
  "RuppConfig = RCC {}:{}, DSR {}, Token type {}".
- `RbxTransportSessionCrypto` adds a Roblox layer on top of TLS.

### How TC differs from a game join

- The endpoint is `/v1/team-create`, not `/v1/join-game`.
- The DataModel is a `CloudEdit` model (`IsCloudEdit`, `FFlagSetIsCloudEditBeforeDMSetup`).
- Next-gen replication and server authority are off in CloudEdit
  (`FFlagNextGenReplicationDisableInCloudEdit`, `FFlagServerAuthorityDisableInCloudEdit`).
- A property filter applies (`CloudEditNetworkFilterObjectProperties`).
- Streaming uses infinite radius (`TeamCreateInfiniteRadiusAvoidsSubscriptionBitmasks`).
- The client is allowed to write everything: an editor, not a player.

[verified: exe/flags; "allowed to write everything" is inferred]

### Public reverse-engineering

Nothing is on disk locally (searched `~/code`, `~/.local/share`, `~/Downloads`).
From memory [recalled, unverified]:

- **Gskartwii's `roblox-dissector` ("Sala")**, Go, about 2017-2020. A
  Wireshark-style dissector and proxy for Roblox RakNet. It documented:
  - the offline magic `00 FF 00 FE FE FE FE FD FD FD FD 12 34 56 78`;
  - protocol byte 5;
  - Roblox packet ids `0x81 ID_SET_GLOBALS`, `0x82 ID_TEACH_DESCRIPTOR_DICTIONARIES`,
    `0x83 ID_DATA`, `0x84 ID_MARKER`, `0x85 ID_PHYSICS`, `0x86 ID_TOUCHES`,
    `0x8A ID_SUBMIT_TICKET`, `0x8D ID_CLUSTER`, `0x8F ID_PREFERRED_SPAWN_NAME`,
    `0x90 ID_PROTOCOL_SYNC`, `0x91 ID_SCHEMA_SYNC`, `0x92 ID_PLACEID_VERIFICATION`,
    `0x93 ID_DICTIONARY_FORMAT`, `0x97 ID_NEW_SCHEMA`;
  - `ID_DATA` sub-items `0x01 delete instance`, `0x02 new instance`,
    `0x03 property change`, `0x04 marker`, `0x05/06 ping`, `0x07 event invocation`,
    `0x0B join data`, `0x0D stream data`, `0x10 tag`, and others;
  - the schema (class, property and enum tables) sent at join, and the
    property value encodings.

  Since then Roblox has added encryption (`RbxOpenRequest`, SessionCrypto,
  about 2024), the QUIC transport (about 2025), JoinData v2 and
  instance-intent items. Old captures and code will not decode a current
  session without the keys.
- What current Studio confirms of that model:   `NewObjectItem`, `DeleteInstanceItem`, `DestroyInstanceItem`,   `ChangePropertyItem`, `ChangeAttributeItem`, `EventInvocationItem`,   `MarkerItem`, `PingItem`, `TagItem`, `JoinDataItem(V2)`, `ClusterItem`,   `SharedStringItem`, `StreamDataItem`, `InstanceIntentItem`,   `ResolveNewInstanceAtomicTreeItem`, `SharedReferencePropertyChangedItem`,   `StatsItem`. The vocabulary matches; the wire layout must be re-derived from the binary.

## 5. What flows during a session

| What | How | Evidence |
|---|---|---|
| Initial place | The JoinData stream from RCC (`tcinitialstreambytes`, `DMInitialStream`); shared strings plus instances, compressed | [verified: exe] |
| Instance create, change, delete | Replicator items in both directions (table above). Each Studio edit becomes a `ChangeProperty`, `NewObject` or `DeleteInstance` item. Undo stays local (`TeamCreateEnableUndoTime`) | items [verified: exe]; mapping [inferred] |
| Collaborators | Each collaborator is a `Player` on the RCC. `CollaboratorsService` holds `Collaborator` instances (`UserId, Username, CFrame` (camera), `CollaboratorColor3, CurDocGUID, CurScriptLineNumber, IsIdle, Status`) and fires `CollaboratorStatusUpdatedSignal` and `CollaboratorIdleUpdate` | [verified: api-dump.json] |
| Selection presence | `Player.CloudEditSelectionChanged`. The Explorer colours each editor's selection by `CollaboratorColor3` (`ExplorerPlugin.Src.Guest.createStudioCollaboratorSelectionWatcher`); there are also `SelectionHighlightManager` and `TeamCreate-SelectionChanged` | [verified: lua + exe] |
| Camera and jump-to | `Collaborator.CFrame`, `CollaboratorsService:RequestFlyToCollaborator`, `CloudEditCameraCoordinateFrame` | [verified: api-dump + exe] |
| Collaborative scripts (live) | `LiveScriptingService` on RCC with `LiveScriptingDMClient` in Studio. The algorithm is **Jupiter operational transform** (`JupiterOperation@LiveEditing`). Messages: BeginEdit, OpenedWithSource, SendOp, SendAtomicOp, AtomicResponse, Broadcast, EndEdit, KickFromEdit, FailedToOpen, carried on Roblox RemoteEvents (`$roblox/beginLiveScripting`, `DFIntWaitLiveScriptingRemoteEventPeriodMs`). Ops are strings; scripts are addressed by script GUID or URI (`ScriptRegistrationService:GetSourceContainerByScriptGuid`) | [verified: exe]; the op string format is [unknown] |
| Drafts (Collaborative Editing off) | `DraftsService`: local drafts, then `CommitEdits`, `UpdateToLatestVersion`, `ShowDiffsAgainstServer`; per-script editors through `GetEditors`. The switch is `VersionControlService.ScriptCollabEnabled` and `StudioData.EnableScriptCollabByDefaultOnLoad` | [verified: api-dump] |
| Who is editing a script | `Collaborator.CurDocGUID` and `CurScriptLineNumber`, `DraftsService.EditorsListChanged`, `ScriptCommitService` "onLiveScriptingEdit (author=…)" | [verified] |
| Locks | No instance locks were found. Conflicts surface as `TeamCreateConflict` with retry (`StudioTeamCreateConflictRetryCount/DelaySec`). Packages and scripts have read-only reasons (`ScriptDocument:AddReadOnlyReasonsAsync`) | [verified: exe/api]; "no locks" is [inferred] |
| Chat | `teamCreateChatPanel`, `CloudEditChatNotifications`. The transport is probably Roblox chat over the replicator | names [verified: exe]; transport [unknown] |
| Health | `TeamCreateServerPing` and `TeamCreateConnectionMonitor` heartbeats, `TeamCreateReplicatorDisconnectionMessage`, reconnect manager | [verified: exe] |
| Eviction | RCC rechecks tenure and permissions regularly (`CollaboratorsService::tenureEvictionCheck`, `hasTenureBulk`) and kicks with reasons such as "TeamCreate access is lost because trusted connections are required" | [verified: exe] |

## 6. Saving in Team Create

- **The server saves.** `DFIntServerConfigCloudEditSavePeriodSeconds=240`
  gives an autosave every 4 minutes (`TeamCreateAutoSave`). There is also a
  final save on shutdown with backoff (`TeamCreateFinalSaveRetry*`)
  [verified: flags + exe].
- **"Save to Roblox" in TC** is a request over the session:
  `TeamCreatePublishService` logs "TeamCreatePlaceUploadRequest from user: {}"
  and later "…Response to user". RCC serializes the place
  (`TeamCreatePlaceUpload`) and uploads it either to the "edge depot" or
  through the assets-upload backend, with headers `x-is-team-create` and
  `x-team-create-save-user-id`, then polls the operation
  (`OperationStatusPollingForTeamCreatePlaceMaxRetry`). The resulting version
  is credited to the user, or to "Studio" when no connection carries that user.
  Errors come back as `TeamCreatePublishService.TeamCreateErrorStatus(result, teamCreateSaveData)`
  [verified: exe + api-dump].
- **Publish while in TC** first checks for other active players
  (`LocalPlacePublishRequestMediator::checkActivePlayersInTeamCreateSession`) [verified: exe].
- **Local safety copies:** the client writes recovery files to a TeamCreate
  recovery folder (`TeamCreateSaver`, file-count limits) [verified: exe].
- **Consequence for rustudio:** in a TC session rustudio would not upload
  place files itself. It would ask the server and wait for the result. That
  also avoids the cookie publishing gap in `AUTH.md` [inferred].

## 7. Requirements, and what @testuser2 needs

- **The place must be published and its universe must have TC on.** TC is
  per universe, and turning it on runs "publish then turn on" [verified: §2].
- **The user needs Edit permission on the universe.** Live, as @testuser2 [verified: live]:
  ```
  POST team-create-service/v1/collaboration/hasTenure {"universeId":100000004}
    -> 200 {"HasTenure":false,"EstablishedAtMs":null}
  POST team-create-service/v1/collaboration/join      {"universeId":100000004}
    -> 403 {"Code":403,"Error":"RccForbidden",
            "Message":"User 10000000004 does not have edit permission on universe 100000004."}
  GET  develop.roblox.com/v2/universes/100000004/permissions -> 403 (code 15)
  GET  develop.roblox.com/v1/universes/100000004 -> 200 (owner Owner2, User, Private)
  ```
  On 2026-11-01 @Owner2 (the owner) got `HasTenure:true`
  (EstablishedAt 2019-11-24, the universe's creation) and `join Success:true`.
- **What @testuser2 needs** to join one of @Owner2's places:
  1. To be granted **Edit** on the universe by the owner. Manage
     Collaborators does this through `develop.roblox.com/v2/universes/{id}/permissions`
     and `/permissions_batched`, with subjects `User/Group/Roleset` and actions
     `Play/Edit/Admin` (`ManageCollaborators.Src.Controllers.GamePermissionsController`,
     `Networking.Requests.Constants`) [verified: lua; not called].
  2. For a user-owned experience, very likely to be a **Trusted Connection**
     (friend) of the owner. The join reply has `RequiresTrustedConnection`,
     and RCC evicts with "trusted connections are required" [verified: exe and
     live field name; the rule itself is inferred].
  3. To pass the age gates (`NotAgeVerified`, `OutsideAgeBucket`,
     `OutsideOwnerAgeBucket`, `PCBlock`) [verified: lua enum]. A new account
     with no age check may be blocked [inferred].
  4. **Tenure**, which today is false. Whether a successful `join` creates it
     (`EstablishedAtMs`) or it is earned another way is [unknown]. RCC rechecks
     tenure and evicts on failure.
  5. The owner to turn TC on for that universe (if it is off) and to have published it.

---

## Implementation plan for rustudio

Each phase can ship on its own and is useful alone.

### Phase 0: REST only, safe now

- "Shared with me" and Group tabs in Open from Roblox:
  `universes/v1/search` with `creatorType=Team` and your user id, and with
  `creatorType=Group` for each group from `creator-home-api/v1/groups`.
- A TC badge from `develop /v1/universes/multiget/teamcreate`, and live avatars
  from `teamcreate/v1/MultiGetUniversesActiveSessionMembers`.
- A "Can I collaborate?" check from `collaboration/hasTenure` and `join`,
  showing `RccForbidden` and age or trusted-connection errors in plain words.
- Until Phase 1 works, opening a TC place loads a read-only snapshot
  (assetdelivery, as Open from Roblox already does) and marks it
  "live session not supported yet".

### Phase 1: read-only live view

- Implement the matchmaker: `POST gamejoin /v1/team-create` with the CSRF
  dance, poll until ready, and parse the join script (§3.4).
- Bring up a transport. Pick one:
  - **RakNet + SessionCrypto**: smaller, but the handshake crypto is undocumented.
  - **RbxTransport (QUIC)**: standard QUIC via `quinn` or `quiche`, plus
    Roblox's channel framing and its own session-crypto layer.
- Decode the schema and JoinData, then build the DataModel from the stream.
  Keep pings and heartbeats alive. Send nothing that edits.
- Exit criterion: rustudio shows the live place and sees others' edits arrive.

### Phase 2: property and instance sync

- Encode rustudio edits as replicator items (`NewObject`, `ChangeProperty`,
  `DeleteInstance`, `ChangeAttribute`, reference fix-ups via
  `ResolveNewInstanceAtomicTree`).
- Apply remote items with undo-history isolation. Handle `TeamCreateConflict`.
- Save through `TeamCreatePublishService`, not uploads.

### Phase 3: collaborative scripts

- Drafts mode first (simpler): `DraftsService` semantics, committing the
  `Source` property.
- Then live editing: the Jupiter OT client over LiveScripting RemoteEvents
  (BeginEdit, SendOp, Broadcast).

### Phase 4: presence

- Publish your own camera `CFrame`, selection (`CloudEditSelectionChanged`),
  `CurDocGUID` and line number.
- Render `Collaborator` instances: coloured selection outlines, the
  collaborator list, fly-to, idle state.
- Chat.

### Open unknowns, and the experiment that settles each

| Unknown | Experiment |
|---|---|
| Exact `/v1/team-create` reply, status codes and polling cadence | With the user's OK, open a TC place in a offline Studio signed in to a real collaborator account, with `mockblox` passing gamejoin through and logging bodies. Or have the offline Studio's mock answer `/v1/team-create` and log what Studio sends and expects. |
| Whether the TC toggle is `POST` or `PATCH`, and its reply | Read the native `dispatchTurnOn` request builder . Or capture one toggle the user does by hand on a throwaway universe. |
| Fix for `GetCanCollaborate` / `MultiGetCanCollaborate` "Request Context Failure" | One `GetCanCollaborate` POST with `robloxctx-authenticated-userid`, which is read-only but needs the user's OK as a new POST. |
| SessionCrypto: algorithms, box format, key-exchange version, which side picks the encryption | Then confirm against a local RCC-less an offline Studio "Server & Clients" test, where Studio is both client and server, so both ends are observable offline. |
| RbxTransport channel framing and its SessionCrypto |  |
| Replicator item wire formats in this version | Run an offline Studio "Server & Clients" test with `DFLogRakNetPktTrace`/`DebugRakNetLogAllPackets`, diffing known edits against packet dumps. This is entirely offline and is the best first experiment for Phases 1 and 2. |
| How a CloudEdit server differs from a play server (property filter, write permissions) | Read `CloudEditPropertiesToNetworkFilter` and the `TeamCreateWorkspaceInstanceAccess` code. |
| Jupiter op string format |  |
| How tenure is earned; what `access` means in `ActiveSessionMembers` | After @Owner2 adds @testuser2 as an editor, rerun `hasTenure` and `join` as @testuser2 and watch `EstablishedAtMs`. |
| Whether a user-owned place needs a friend link for edit collaborators | The same step: try adding the account as a non-friend and see what Manage Collaborators says. |

## Do not

- Call `team-create-preemptive`, `team-create`, the teamcreate toggle,
  memberships DELETE or permissions writes without the user's go-ahead. Each
  one starts servers or changes access.
- Print or store the cookie. the probe scripts keep it inside curl's
  stdin config.
