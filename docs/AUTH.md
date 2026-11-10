# Account and sign-in

Decision taken 2026-11-01 by the person building this app:

> "It should be one login, OAuth can go. Cookies should be the source of truth
> for logged in profile."

This file replaces `LOGIN.md` (folded in below and deleted; its OAuth
sections were dead).

## The decision

- **One credential: the `.ROBLOSECURITY` cookie session.** It is the only thing
  rustudio uses to identify the signed-in person.
- **OAuth 2.0 / PKCE is removed** (round 10): no client id, no localhost
  callback listener, no token endpoint or refresh tokens, no scopes, no
  `OAuthConfig` / `DEFAULT_SCOPES` / `CALLBACK_PATH`, no OAuth panel or
  "test sign-in config". An old `account.toml` with an `[oauth]` table is
  rewritten without it, and old OAuth tokens are deleted from the keyring.
- **The profile is derived from the cookie, not stored independently.**
  Signed in means `users.roblox.com/v1/users/authenticated` answered 200 for
  the cookie this run; anything else is "signed out". The profile (id, name,
  displayName) and `avatar.roblox.com/v1/avatar` are cached for display
  only, in `~/.cache/rustudio/session.json`, under a one-way key of the
  cookie (SHA-256 with an app prefix), so a cookie change invalidates them.
- **API keys:** no longer a way to sign in. One remains only as the
  *publishing key*, because Roblox refuses the cookie for place uploads (see
  "Publishing" below). It goes once publishing over the cookie works, so
  that exactly one credential remains.

## Where the cookie lives

- Import source: `~/.config/rustudio/roblox-cookie`, mode `600`, one line
  (with or without the `.ROBLOSECURITY=` prefix). Set by hand; never written
  by the app. A file readable by others is refused.
- Runtime copy: the login keyring through the `keyring` crate (Secret
  Service on Linux, Credential Manager on Windows), service `rustudio`, user
  `roblox-cookie`. A session an older build stored with `secret-tool`
  (attributes `service rustudio key roblox-cookie`) is read once, moved into
  the keyring and removed. The app reads the keyring first and imports from
  the file when the keyring has none (once per file value:
  `roblox-cookie.imported` holds the one-way key of the value already
  imported). **Sign out** removes the keyring copy and stops the file from
  being imported again until **Try again**.
- Saved accounts (switching): service `rustudio`, user `roblox-account-<id>`
  (older builds used `service rustudio key roblox-account user <id>`, moved
  over the same way), one per account that signed in; switching copies one
  into the active slot after Roblox accepts it. `account.toml` keeps only
  id, name, display name.
- Never: the project folder, `account.toml`, any log, any screenshot, any
  `checks/` artifact, any error message. In code the value is a `Cookie`
  whose Debug / Display print `<redacted>`; it only goes into the `Cookie`
  header of requests to `*.roblox.com`. A rejected cookie is reported as
  "signed out (Roblox did not accept the saved session: HTTP 401)", never
  echoed. The self-test step "account-session" checks that no Output line or
  result contains it.

## Evidence: what the cookie reaches

Probed 2026-11-01 against the live cookie (account @Owner2, id
10000000003). `200` means the endpoint accepted the session.

| Endpoint | Result | Gives |
|---|---|---|
| `users.roblox.com/v1/users/authenticated` | 200 | `id`, `name`, `displayName` — the profile |
| `avatar.roblox.com/v1/avatar` | 200 | `playerAvatarType`, `scales`, `bodyColors` (BrickColor numbers), worn `assets` |
| `www.roblox.com/my/account/json` | 200 | legacy account fields (id, name, email state) |
| `develop.roblox.com/v1/user/universes?limit=50` | 200 | the experiences this account owns (one: "Owner2's Place", universe 100000004, root place 100000005, private) |
| `develop.roblox.com/v1/universes/{id}` | 200 | universe details |
| `assetdelivery.roblox.com/v2/assetId/{id}` | 200 | asset bytes: public ones anonymously; with the cookie also the account's own private place and assets served only to signed-in accounts (the current dynamic head 123480233606534 and its cage 76523574292282; catalog layered clothing such as 8516620262) |
| `apis.roblox.com/...` (Open Cloud) | 404 | **not cookie-authenticated** |
| `data.roblox.com/Data/Upload.ashx` (POST) | 403 + `x-csrf-token`, then 307 → `request-error?code=404` | see "Publishing" |
| `www.roblox.com/ide/publish/UploadExistingAsset` (POST) | 404 | see "Publishing" |

`limit` on `develop.roblox.com/v1/user/universes` only accepts `10, 25, 50,
100` — anything else is a 400. Cloudflare fronts these hosts: requests carry
a browser-like user agent, and redirects are reported rather than followed
(a cookie call sent to a login page is a refusal; a POST must not be
replayed elsewhere).

The account's `playerAvatarType` is **R6**, with non-default scales (height
0.9, width 0.7, head 0.95, depth 0.85), torso BrickColor 26 (Black) and the
rest 1003 (Really black): a real-world test of the R6 rig and, when a place
forces R15, of scale-aware body parts.

## CSRF

A state-changing POST is answered `403` with an `x-csrf-token` header;
rustudio repeats the same request with that value in `X-CSRF-TOKEN` and
keeps the token for the session (`Session::post_csrf`), by Studio's own rule
: only for methods other than GET, HEAD,
OPTIONS and TRACE, only when the new token is non-empty and differs from the
one sent, at most 3 times. Studio also treats `.ROBLOSECURITY`,
`.RBXIDCHECK` and `.RBXID` as secret cookie names; rustudio
never shows any of them. Verified on every
self-test run with a POST to `Upload.ashx?assetid=0` (no asset 0 exists, so
nothing can be written).

## Publishing

Tried end to end on 2026-11-01 without changing anything: the account's own
default place (100000005, Roblox's 2019 starter template, private, not
updated since 2019) was downloaded and its unchanged bytes uploaded again,
then downloaded once more and compared.

- `POST data.roblox.com/Data/Upload.ashx?assetid=100000005`: the CSRF
  token is accepted, then the endpoint answers `307` to
  `www.roblox.com/request-error?code=404` (also for `assetid=0`, and with
  `&type=Place`). The legacy upload endpoint no longer takes place uploads.
- `POST www.roblox.com/ide/publish/UploadExistingAsset?assetId=…&isAppCreation=true`   (the route Studio builds, strings `/ide/publish/UploadExistingAsset`   and `assetId=%s&isAppCreation=true`): `404 {"errors":[{"code":0,"message":""}]}`   for the account's own place as for asset 0. Studio authenticates that call   with its own sign-in, not the web cookie, and rustudio doesn't pretend to   be Studio.
- After both, the place's bytes were identical to before (367 380 bytes):
  nothing was written.

So publishing over the cookie doesn't work today. rustudio still tries the
session upload when no publishing key is saved (and reports Roblox's
answer), and publishes through Open Cloud Place Publishing
(`POST apis.roblox.com/universes/v1/{universeId}/places/{placeId}/versions`)
with the publishing key otherwise. The key UI goes when a cookie upload
path is proven.

## Code

| Where | What |
|---|---|
| `crates/rustudio-account/src/session.rs` | `Cookie` (redacted), keyring via `store.rs`, the import file, `Session` (validate, GET, `post_csrf`, avatar, owned universes, asset download, place upload), `ProfileCache` |
| `crates/rustudio-account/src/api.rs` | `Client::open` (cookie → validated session → cache), experiences, downloads, publishing; `public_asset` stays anonymous |
| `crates/rustudio-account/src/config.rs` | bookmarks and the signed-out flag only |
| `crates/rustudio-account/src/store.rs` | the system keyring: the publishing key, and the entries `session.rs` keeps the cookie in |
| `src/account_ui.rs` | Account panel (active account, saved accounts, switch, add by pasted cookie, sign out) and the Publish to Roblox window |
| `src/avatar.rs` | the avatar from the session's own description |
| `crates/rustudio-account/src/cloud.rs` | places on Roblox: lists, resolve, download, versions, Team Create status, cache, save with the version check |
| `crates/rustudio-account/src/mock.rs` | a local HTTP stand-in for Roblox's hosts (tests) |
| `src/cloud_ui.rs` | File > Open from Roblox, Save to Roblox / Publish, Experience Settings |
| `src/assets.rs` | downloads use the session; background loads fall back to it for login-only assets |

## Places on Roblox (round 27)

Probed 2026-11-04 with the session, read-only, first as @testuser2
(10000000004, owns nothing, no groups), then as @testuser1 (10000000001),
who collaborates on a live Place A session (universe 10000000003, place
100000000000004, owner Owner1 100000002, Team Create on) and is a Member of
the group Place B (12989002). No call here changed anything; the
POSTs are "get" calls. MECHANICS.md round 27 has how they are used.

| Endpoint | Result |
|---|---|
| `develop.roblox.com/v1/user/universes?limit=50` | 200, owned experiences (none for either account) |
| `apis.roblox.com/universes/v1/search?creatorType=Team&creatorTargetId=<me>&…&surface=StudioStartPage` | 200; Shared with Me (testuser1: a live Place A session, `isTeamCreateEnabled: true`). Without `creatorTargetId`, or someone else's: 401 "Authenticated user does not match target" |
| `…/universes/v1/search?creatorType=User&creatorTargetId=<me>` | 200 (Studio's Owned tab) |
| `groups.roblox.com/v2/users/{id}/groups/roles` | 200, groups and role names |
| `groups.roblox.com/v1/groups/{id}/membership` | 200, the role's permissions; `groupEconomyPermissions.manageGroupGames` = may edit the group's experiences (false for Member) |
| `apis.roblox.com/creator-home-api/v1/groups?surface=StudioStartPage` | 200, Studio's group menu (all groups, role name, no permissions) |
| `develop.roblox.com/v1/user/groups/canmanage` | 200 (empty for both) |
| `develop.roblox.com/v1/groups/{id}/universes` | 403 "You are not authorized for access." for a role that can't edit |
| `develop.roblox.com/v1/universes/{id}`, `…/multiget?ids=` | 200 for any universe: name, rootPlaceId, creatorType / creatorTargetId / creatorName |
| `develop.roblox.com/v1/universes/{id}/places` | 200 for any universe |
| `apis.roblox.com/universes/v1/places/{placeId}/universe` | 200 `{universeId}`, also for private places; an unknown place gives a null id |
| `games.roblox.com/v1/games/multiget-place-details?placeIds=` | 200 place name, also for private places |
| `develop.roblox.com/v2/places/{id}` | 403 unless the place is editable |
| `assetdelivery.roblox.com/v2/assetId/{placeId}` | 200; the reply header `roblox-assetversionnumber` is the newest version. No access: 200 with `errors[0]` 409 "User is not authorized to access Asset."; without the cookie: 401 "Authentication required" for any place |
| `…/v2/assetId/{placeId}/version/{n}` | 200 for any version of a readable place (Baseplate v1, v70) |
| `apis.roblox.com/place-version-history-api/v1/{placeId}/history?pageSize=` | 200 for an editor (a live Place A session: 50 rows, `version` as a string, `saveType` a number, `isPublished`); 403 otherwise |
| `develop.roblox.com/v1/assets/{id}/saved-versions` | 200 for an editor (`assetVersionNumber`, `created`, `isPublished`); 403 "…permissions to manage the asset" otherwise |
| `apis.roblox.com/creation-storage/v1/places/{id}/place-version-log` (in Studio) | 404 |
| `develop.roblox.com/v1/universes/multiget/teamcreate?ids=` | 200; `data[{id,isEnabled}]` only for universes the account can edit (a live Place A session: true), empty otherwise |
| `develop.roblox.com/v1/universes/{id}/teamcreate` | 200 `{isEnabled}` for an editor; 403 otherwise |
| `develop.roblox.com/v1/universes/{id}/permissions` | 200 `{canManage, canCloudEdit}` (a live Place A session true/true, Baseplate false/false) |
| POST `apis.roblox.com/team-create-service/v1/GetCanCollaborate` `{universeId}` | 200 after the CSRF 403: `CanCollaborate`, `Error` ("None", "NotAuthorized"), `HasTenure`, `UserCapabilities` |
| POST `…/collaboration/hasTenure` | 200 `{HasTenure:false}` for Baseplate |
| POST `…/collaboration/join` | 403 "RccForbidden: … does not have edit permission" for Baseplate (not used by rustudio) |
| `thumbnails.roblox.com/v1/games/icons?universeIds=…&size=150x150&format=Png` | 200, CDN PNG URLs (fetched without the cookie) |
| `develop.roblox.com/v1/gametemplates` | 200, Roblox's templates (Baseplate 28220420 / 95206881, by Templates 998796, uncopylocked) |

Saving a version is Open Cloud Place Publishing with the publishing key
(`POST apis.roblox.com/universes/v1/{u}/places/{p}/versions?versionType=Saved|Published`).
It was **not** called against Roblox: it is tested against a local mock
only. Still waiting on a live test the person approves: one Saved and one
Published version of a place they own, with a key, and the conflict check
against a real second save.

### Seeing your own games

The session is whatever cookie the import file / keyring hold. rustudio
lists the experiences of *that* account: owned ones, its groups' (when the
role has "Edit group experiences"), and ones it collaborates on. To see a
main account's games either sign rustudio in with that account's cookie, or
keep the alt and add it as a collaborator (Shared with Me; edit permission
for saving) or give its group role "Edit group experiences".

## Team Create (later; the reason any of this exists)

- **Known:** joining is web API — the Team Create status for a universe /
  place gives a session, then a WebSocket realtime connection carries it.
  It runs on the same cookie session.
- **Unknown, and must be captured before any promise:** the binary
  instance-delta protocol, presence, per-instance locking, version
  reconciliation, join / leave / reconnect, and edit permissions.
- **Blocker:** there is no real Roblox Studio or client on this machine to
  observe. `~/.local/share/rustudio/studio` is an offline mock that never talks
  to Roblox, so it cannot show a Team Create handshake. A real client has to
  be installed first, then the session captured.
- rustudio already has the supporting pieces — the rbxl DOM (`src/doc.rs`)
  and server-to-client replication (`src/replicate.rs`) — so this is
  protocol and session-lifecycle work, not a rewrite.

## Getting a cookie (not automated)

The cookie is put in the import file by hand. An in-app login was looked at
and set aside: a username/password POST to `auth.roblox.com/v2/login` meets
2FA and FunCaptcha challenges that can't be solved programmatically, and
Roblox's risk checks flag that request shape. If an in-app login is added
later, it should be a real browser view (WebKitGTK at `roblox.com/login`)
where the person types their own password and answers their own
challenges, with rustudio reading `.ROBLOSECURITY` from that view's cookie
store.

## Sequencing

1. Cookie store + validation + profile cache (the whole of "signed in"). Done.
2. Publishing over the cookie. Tried; refused by Roblox (above). The
   publishing key stays until a cookie path works.
3. OAuth removed. Done. The API key as a sign-in: removed; as a publishing
   key: waits on 2.
4. Team Create on the same session.

## Open questions

- Which upload path takes the cookie for a place version today, if any?
- Does `Upload.ashx` (if it returns) accept `.rbxlx` (XML) or only `.rbxl`?
- Cookie lifetime: how long a session survives without a page load, and
  whether `users.roblox.com/v1/users/authenticated` stays a safe liveness
  probe (it has on every run so far).
- Which client version does the Team Create realtime protocol expect?
