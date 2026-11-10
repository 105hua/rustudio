# players_chat parity notes (2026-11-07)

Domain code: `src/domains/players_chat.rs` (hub, Player / Players / Teams /
PlayerViewService, TextChat properties, the play-session bridge),
`players_chat.luau` (everything that yields or runs callbacks: TextChat
message flow, group / friend / user lookups, bans, LoadCharacter, Kick, the
legacy Chat service), `players_chat_web.rs` (login-session lookups),
`players_chat_loc.rs` (LocalizationTable / Translator). Chat UI and the
SendAsync bridge between the client and server runtimes: `src/play_game.rs`.

Probes: `checks/parity/players_chat/{server,client}.luau` with
`checks/parity/players_chat/tcs.rbxlx` (ChatVersion = TextChatService):

    gamescope --backend headless -- env RUSTUDIO_NO_AUDIO=1 ./target/release/rustudio \
      checks/parity/players_chat/tcs.rbxlx --play --screenshot /tmp/pc.png --shot-after 14 \
      --luau server checks/parity/players_chat/server.luau@3 \
      --luau client checks/parity/players_chat/client.luau@4 --log /tmp/pc.log

## Measured in Studio (offline Studio slot of our own, Play test, signed-in mock)

Player
- `GetJoinData()` → `{}` (no teleport). `IsVerified()` server → false;
  client → error `isVerified() can only be called by the backend server`.
- `GetCameraState()` → `GetCameraState requires Server Authority to be enabled`.
- `GetData()` → `Player:GetData() is not yet enabled!`
- `AddReplicationFocus` / `RemoveReplicationFocus` / `RequestStreamAroundAsync`
  with StreamingEnabled off → `Workspace.StreamingEnabled is false, cannot X`;
  a client with a non-part → `Invalid replication focus`.
- DataPersistence (`LoadBoolean/Number/String/Instance`, `Save*`): warning
  `DataPersistence is deprecated, please use DataStoreService instead.`
  per call, Load gives false / 0 / "" / nil, nothing is kept.
  `WaitForDataReady()` → true.
- `LoadCharacter*` on the client → `LoadCharacter can only be called by the
  backend server`.
- `GetFriendsInServerAsync` → `Not Implemented`.
- `Kick(msg)`: Studio shows the "Disconnected" dialog: `You have been kicked
  by this experience or its moderators. Moderation message:` + blank line +
  message. A LocalScript kicking another player: `Cannot kick a non-local
  Player from a LocalScript` (Studio's message).
- Properties: AgeChecked = Unchecked, FrustumStreaming = Default,
  User prints `U1.AQAAAAAAAAAAAAAAAAAAAAA`. DeviceTrustLevel isn't a member
  in the installed Studio build (hidden in the dump).
- Events on the server: PlayerAdded, CharacterAdded, then
  CharacterAppearanceLoaded; Chatted fires on the server for TextChatService
  messages too (also for command text such as `/hey there`).

Players
- BubbleChat false, ClassicChat true, MaxPlayers 12, PreferredPlayers 0.
- `BanAsync` / `UnbanAsync`: config checks in this order with these texts
  (Studio's messages): `Config must contain field UserIds with type array`,
  `… Duration with type number`, `… DisplayReason with type string`,
  `… PrivateReason with type string`, `Duration must be a positive number or
  -1`, `UserIds exceeded the maximum length of {}`, `Expected type number in
  UserIds at index {}`, `Invalid UserId: {}. All UserIds must be positive`;
  Unban: `… ApplyToUniverse with type boolean`. Client: `BanAsync() can only
  be called on the server`. A valid call returns without kicking anyone and
  prints `{}Async will succeed on production game servers. Skipping request
  in test environment...`. `GetBanHistoryAsync` prints `GetBanHistoryAsync
  will succeed on production game servers. Request will fail in test
  environment.` and errors (the offline Studio's mock gave HTTP 503; rustudio gives
  HTTP 403 (Forbidden) — the real code isn't known).
- `GetNameFromUserIdAsync` failure text names the other function:
  `Players:GetUserIdFromNameAsync() failed: Unknown user` (Studio's bug, kept).
- `GetUserThumbnailAsync(1, HeadShot, Size420x420)` →
  `rbxthumb://type=AvatarHeadShot&id=1&w=420&h=420`, false.
- `CreateHumanoidModelFromDescriptionAsync` → a Model named after the
  player... (rustudio: the existing CreateHumanoidModelFromDescription).

Legacy Chat
- `InvokeChatCallback` calls the registered callback and returns its result;
  none registered → nothing. `SetBubbleChatSettings(5)` → ExpChat error
  `Bad settings object passed to Chat:SetBubbleChatSettings:\ninside
  consolidated BubbleChat table expected, got number`.
- `FilterStringForBroadcast` from a LocalScript warns the deprecation line
  and returns the text unfiltered (Studio doesn't filter).

TextChatService (ChatVersion = TextChatService)
- The server creates `TextChannels/{RBXSystem, RBXGeneral}` and
  `TextChatCommands/{RBXTeamCommand /team /t, RBXClearCommand /clear /cls,
  RBXMuteCommand /mute /m, RBXUnmuteCommand /unmute /um, RBXHelpCommand
  /help /?, RBXEmoteCommand /emote /e, RBXVersionCommand /version /v,
  RBXConsoleCommand /console, RBXWhisperCommand /whisper /w}`; a TextSource
  per player in each channel (CanSend true in RBXGeneral, false in
  RBXSystem). The client sets ChatInputBarConfiguration.TargetTextChannel to
  RBXGeneral.
- `SendAsync(text, meta)` (client only, else `TextChannel:SendAsync can only
  be called on the client.`; outside the service: `SendAsync error:
  TextChannel not a descendant of TextChatService`). Order seen:
  OnIncomingMessage (Status Sending) → TextChatService.SendingMessage →
  [command Triggered] → server Player.Chatted → OnIncomingMessage (Success)
  → TextChatService.MessageReceived → TextChannel.MessageReceived. The
  OnIncomingMessage result's PrefixText replaces the message's. MessageId is
  `{UserId}-{GUID}`; PrefixText `DisplayName:`; Timestamp a DateTime;
  Metadata as given. A command message is still delivered.
- `DisplaySystemMessage(text, meta)` (client only): MessageId `0-{GUID}`,
  no TextSource, PrefixText "", Success, MessageReceived on both.
- `DisplayBubble(part, text)`: nil → `partOrCharacter must be non-nil`, not
  in Workspace → `partOrCharacter must be in the Workspace`; fires
  BubbleDisplayed(part, message). Bubbles for player messages show over the
  head (AdorneeName HumanoidRootPart) but no BubbleDisplayed fired in Studio.
- `AddUserAsync(existing)` → (TextSource, false). `SetDirectChatRequester`
  with sources present → `TextChannel:SetDirectChatRequester: Cannot update
  direct chat requester when TextSources are already present`.
- Default UI (Studio, 862×677 viewport): window AbsolutePosition 8,12 size
  344.8×154.75 (below the 58 px top bar), input bar 16,166.75 size
  328.8×43; window bg (25,27,29) α0.3, Gotham SSm Medium 14, stroke α0.5;
  bar bg α0.2, placeholder (178,178,178); bubbles bg (250,250,250) α0.1,
  text (57,59,61), size 16, MaxBubbles 3, BubbleDuration 15, MaxDistance
  100; names coloured with the classic name-hash palette ("Player" → blue).
  ChannelTabs disabled (0,0 / 0,0).
- `ChatWindowConfiguration:DeriveNewMessageProperties()` copies the window's
  TextColor3 (1,1,1), TextStrokeColor3 (0,0,0), TextStrokeTransparency 0.5,
  FontFace; PrefixTextProperties nil.
- `PlayerViewService:GetDeviceCameraCFrame()` → `No player specified`;
  with a player → identity.

Localization (Studio)
- See the error texts and orders in players_chat_loc.rs' tests:
  `LocalizationTable does not contain key "zzz"`, `LocalizationTable entry
  with key "k" does not contain locale "fr"`, `Key nope not found for locale
  es`, `JSON parse error: Invalid value.`, `entries was not an array.`,
  `Entry at index 1 had invalid Key/Source/Context: Entry at index 1 "Key"
  was not a string.`; FormatByKey: `Found parameter token, but had no
  parameters`, `Parameter index "0" was out of range of empty array`,
  `Expected array table, but table contained named arguments.`
- `GetContents` order: keyed entries, then by source / context; fields
  key, context, examples, source, values (empty ones left out).
- `es-mx` falls back to `es`. Translate(workspace, "Play") picked the entry
  whose context lies under game.Workspace over the context-less one.
- `GetTranslatorForPlayerAsync` in an unpublished place → `Publishing the game
  is required to use GetTranslatorForPlayerAsync API.`;
  GetTranslatorForPlayer / GetTranslatorForLocaleAsync work.
  `GetCountryRegionForPlayerAsync` → "US". RobloxLocaleId / SystemLocaleId
  en-us. Translators are named "Instance", unparented.

## Web endpoints (login session only; reads)

From Studio strings: `/v1/users/%s/friends/online?origin= getFriendsOnline`, `v1/users/%s/friends/statuses`, `/v1/users/%s/friends/find`, `/v1/usernames/users`, `v1/users/%lld`, groups roles. rustudio uses friends.roblox.com, users.roblox.com and groups.roblox.com (`/v2/users/{id}/groups/roles`). FriendPages: friends/find ids + POST users.roblox.com/v1/users for names + friends/online for IsOnline, 50 a page (through the data domain's register_pages).

## Local player identity (2026-11-08)

Measured in the user's signed-in online Studio (`studio-mcp-call`, an
unpublished local Baseplate, Play; values not recorded, only types and
relations). Server and client Player agree on every field.

| Field | Type | Signed-in Studio |
|---|---|---|
| UserId | number (int > 0) | the signed-in account's id (not 1) |
| Name / DisplayName | string | the account's username / display name (may differ) |
| MembershipType | EnumItem | the account's (None or Premium) |
| AccountAge | number (int ≥ 0) | whole days since the account was created |
| HasVerifiedBadge | boolean | = UserService:GetUserInfosByUserIdsAsync's HasVerifiedBadge |
| CharacterAppearanceId | number | = UserId |
| LocaleId | string | "en-us" (Studio's locale) |
| FollowUserId | number | 0 |
| OsPlatform | — | error `The current thread cannot read 'OsPlatform' (lacking capability RobloxScript)` |

Relations that hold in Studio: character model Name = Player.Name;
GetNameFromUserIdAsync(UserId) = Name; GetUserIdFromNameAsync(Name) = UserId;
UserService's Username / DisplayName match; the character's Humanoid has a
HumanoidDescription child whose body colours / clothing match
GetHumanoidDescriptionFromUserIdAsync(UserId); VoiceChatService:
IsVoiceEnabledForUserIdAsync(own id) → boolean on the client AND the server;
the server with a user not in the game → `VoiceChatService:
IsVoiceEnabledForUserIdAsync() the player must be connected to the current
server`; the client with another user → `… cannot access eligibility info for
other users`. game.CreatorId isn't the player (unpublished place).

an offline Studio (offline, fake login): Name / DisplayName "Player", UserId and
AccountAge from its mock account, MembershipType None, CharacterAppearanceId =
UserId, a HumanoidDescription child, GetUserIdFromNameAsync(Name) = UserId.

rustudio now: Play waits (≤ 20 s) for a session check still running and then
the avatar (`AccountUi::settle_for_play`), since Studio only plays signed in
and its first character already wears the avatar. Signed in, the local
Player gets the rustudio login session's UserId / Name / DisplayName and,
through the session, AccountAge (users.roblox.com/v1/users/{id} `created`),
HasVerifiedBadge (same) and MembershipType (premiumfeatures
`/v1/users/{id}/validate-membership`), CharacterAppearanceId = UserId; the
character's Humanoid gets the account's HumanoidDescription (players_chat.luau,
before CharacterAppearanceLoaded). Signed out (RUSTUDIO_NO_SESSION=1 / no
session) keeps the offline identity: "Player" / "Player", UserId 1,
AccountAge 0, None — the offline Studio's 100000002 / 3000 are its mock account's.
Consumers that agree with it: GetUserIdFromNameAsync / GetNameFromUserIdAsync
(in-game players first, then the session), VoiceChatService's own-user check
(media_misc.rs: server branch now as Studio), the avatar (loaded for the
session's user), Team Create (its own Player is the join identity's, the same
session user; Play replaces it with this one).

Kick, measured in an offline Studio: the server logs `Server Kick Message: {msg};
Metadata:`, fires CharacterRemoving, then PlayerRemoving(player,
Enum.PlayerExitReason.CreatorKick). rustudio does the same.
