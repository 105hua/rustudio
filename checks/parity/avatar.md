# Avatar domain: parity notes (2026-11-08)

Code: `src/domains/avatar.rs` (dispatch, install, GetAccessoryType, pages
hooks) with `avatar_props.rs`, `avatar_desc.rs`, `avatar_rig.rs`,
`avatar_face.rs`, `avatar_web.rs` and the Luau files `avatar_aes.luau`
(AvatarEditorService and AvatarCreationService), `avatar_apply.luau`
(ApplyDescription, GetAppliedInstance, Wrap* methods) and
`avatar_rules.luau` (Studio's GetValidationRules table).

Ground truth: a offline Studio 0.744 (own instance, closed after), the strings and CoreScripts of `Studio` (`ExtraContent/scripts/CoreScripts/Modules/AvatarEditorPrompts`), and live read-only calls through the login session (the ignored test `probe_avatar_endpoint` in `crates/rustudio-account/src/avatar.rs`).

## Coverage (`--api-coverage`, game-script members of this domain's classes)

| | before | after |
|---|---|---|
| functions | 0 / 79 | 79 / 79 |
| properties | 240 / 346 | 346 / 346 |
| events in `events_to_check` really fired | 0 / 12 | 9 / 12 |

Also fired now: `Humanoid.ApplyDescriptionFinished` (it's on anim's list;
anim agreed that ApplyDescription* is mine).

## Endpoints (all through the login session, never a key)

The paths are the ones Studio uses, next to the `AvatarEditorService_*` names:

| Method | Request |
|---|---|
| GetInventoryAsync | `GET avatar.roblox.com/v1/avatar-inventory?pageLimit=25&sortOption=1&availabilityStatus=0&itemCategories[i].itemType=1&itemCategories[i].itemSubType=<AvatarAssetType>` + `&pageToken=` |
| GetOutfitsAsync | `GET avatar.roblox.com/v2/avatar/users/<uid>/outfits?itemsPerPage=50&isEditable=<true/false/empty>[&outfitType=X]` + `&paginationToken=` |
| GetOutfitDetailsAsync | `GET avatar.roblox.com/v3/outfits/<id>/details` |
| GetAvatarRulesAsync, Conform/CheckApplyDefaultClothing | `GET avatar.roblox.com/v1/avatar-rules` |
| GetHeadShapesAsync | `GET avatar.roblox.com/v1/avatar-inventory?itemCategories[0].itemSubType=1&itemCategories[0].itemType=headshape&sortOption=1&pageLimit=120&pageToken=0` |
| GetItemDetailsAsync | `GET catalog.roblox.com/v1/catalog/items/<id>/details?itemType=Asset/Bundle` |
| GetBatchItemDetailsAsync | `POST catalog.roblox.com/v1/catalog/items/details` (read-only lookup, CSRF) |
| GetFavoriteAsync | `GET catalog.roblox.com/v1/favorites/users/<uid>/(assets/bundles)/<id>/favorite` (`null` = not favourited) |
| GetRecommendedAssets/BundlesAsync | `GET catalog.roblox.com/v2/recommendations/(assets?assetTypeId=&assetId=/bundles?bundleId=)&numItems=50&details=true` |
| GetBundlesByAssetIdAsync | `GET catalog.roblox.com/v2/assets/<id>/bundles?limit=&sortOrder=Asc` (the live API answers 403 "User is unauthorized" with a plain session, so the method errors `HTTP 403 (Forbidden)`, as Studio would with the same answer) |
| SearchCatalogAsync | `GET catalog.roblox.com/v2/search/items/details?model.keyword=…&model.sortType=…` (Studio's `model.*` parameter names) |

Failures read `HTTP <code> (<reason>)`, Studio's text. Signed out, a request
fails with `HTTP 401 (Unauthorized)`. Replies reach scripts with keys
capitalised (camelCase → PascalCase). GetOutfitDetails turns `bodyColor3s`
into `BodyColors` with Color3 values.

## Prompts

In Studio, Prompt* fires an internal event (OpenPromptSaveAvatar …). The
AvatarEditorPrompts CoreScript shows a dialog, its buttons call
Perform*/Signal*PermissionDenied, and the engine writes to the account and
fires Prompt*Completed. rustudio has no CoreScripts, so `avatar_aes.luau`
builds the dialog itself as a ScreenGui "AvatarEditorPrompts" in the
LocalPlayer's PlayerGui (the only GUI rustudio draws; Studio uses CoreGui).
Cancel gives PermissionDenied and Confirm gives Success. Confirm never writes to the
account. Studio's checks are kept: "X can only be called on the client" and
"X can not be called again while currently in progress". GetInventory and
GetOutfits need PromptAllowInventoryReadAccess first ("Permission must be
obtained …"). Allow grants read access for the session. Tests answer a
dialog with `__rs_avatar_prompt_answer(ok, text)`.

The AvatarCreationService prompts show the same kind of dialog. Confirm
reports `UploadFailed` with Studio's "Unable to Publish Assets" message,
because rustudio never publishes or buys anything. Cancel reports
`PermissionDenied`.

## Behaviour checked against Studio

- HumanoidDescription: the accessories are one ordered list, and the rigid
  string props are views of it. Writing one moves that type to the end.
  SetAccessories warns and changes nothing on a bad entry ("invalid asset
  id", "invalid accessory type", "missing Order"). Layered entries carry
  Order and Puffiness (default 1). The list is kept in AccessoryBlob, so
  Clone keeps it. Emotes and equipped emotes fire EmotesChanged and
  EquippedEmotesChanged with the new table on every change, including
  AddEmote of an id that's already there. RemoveEmote doesn't unequip.
- Rig descriptions: joint names and labels, GetJointFromName with part
  names, and the warning text. Finger controls are clamped to [0, 1], and
  without the finger's joints Studio warns "missing some data for finger N"
  (N 0-based).
- Avatar*Rules defaults (BodyRules ranges etc.). ClassicPants/Shirt/
  TShirtOverride and BodyPartsCollidableOnDeath are flag-gated off in Studio
  0.744 ("not a valid member"). They are in the API dump, so here they
  behave like their siblings.
- FaceControls: values are clamped to [0, 1], Clone copies them and Changed
  fires.
- GetAccessoryType: checked item by item.
- AvatarCreationService errors (token checks, "Unable to find id.",
  "Humanoid not found in model.", "Incorrect number of arguments, expected
  2/3.").
- WrapDeformer, WrapTarget and WrapTextureTransfer precondition errors.
- GetAppliedInstance errors.
- Engine errors carry no script position.

## ApplyDescription (Humanoid)

ApplyDescription, ApplyDescriptionAsync, ApplyDescriptionReset and
ApplyDescriptionResetAsync rebuild the character:

- body colours (BodyColors and the parts)
- R15 scale values and real rescaling of parts, attachments, joint offsets
  and HipHeight
- Shirt, Pants and Shirt Graphic (templates read from the downloaded assets)
- the face decal
- accessories, downloaded with the session through InsertService:LoadAsset
  and welded on matching attachments; their placement offsets are applied
- the Animate script's animation ids

The applied copy becomes the Humanoid's HumanoidDescription child (anim's
GetAppliedDescription clones it) and ApplyDescriptionFinished fires.
ApplyDescription changes only what differs from the description applied
before. Reset removes everything the description doesn't list. Errors
follow Studio: "can only be called by the backend server", "got nil
humanoidDescription", "same asset used more than once". This was verified
in a signed-in headless Play test.

## Skinned meshes (renderer) and dynamic heads

`src/mesh.rs` reads skinning data: the v4/v5 bones, name table, subsets and
per-vertex envelopes, and the v6/v7 `SKINNING` chunk. `src/skin.rs` deforms
a MeshPart on the CPU whenever a Bone under it or its FaceControls changes
(scene deps):

- A mesh bone with a Bone instance of the same name follows that Bone
  (Bone.CFrame × Transform up the chain).
- A mesh bone without one keeps its bind frame relative to its parent, times
  the FACS offset from the part's FaceControls.
- Dynamic heads come without Bone instances, so this second case is how
  FaceControls moves the face. The head mesh names its controls with codes
  (`c_JD`, `l_EC` …), mapped to the 50 properties in `avatar_face.rs`.

Each mesh vertex is skinned once and copied to its corners, then
re-uploaded in place (`Renderer::set_mesh`). Parts with no Bones and no
FaceControls never take this path.

Heads come in Studio's MeshPart-head format: a `POST
assetdelivery.roblox.com/v1/assets/batch` with `assetFormat:
avatar_meshpart_head` (the string is Studio's). Plain asset delivery gives
the legacy SpecialMesh head.

Measured on a headless 1265×900 Play, 40 dynamic heads (8,778 corners
each), 20 s `--bench`. The machine was shared with 11 other agents (load
average 86–92), so the absolute numbers are inflated:

| | avg frame |
|---|---|
| heads static | 9.29 ms (skin path 0.03 ms) |
| FACS animated every frame, first version (per corner, all columns) | 40.7 ms (`sd_skin` 19.9) |
| FACS animated, per vertex, live columns only | 18.8 ms (`sd_skin` 7.4: deform 3.3, upload 2.3, matrices 1.6) |

## ApplyDescription body parts and scaling

- Body parts (Torso, the arms, the legs, Head): the asset's
  `R15ArtistIntent`, `R15Fixed` or `R15` folder is used, and each part goes
  through anim's `Humanoid:ReplaceBodyPartR15`. This was verified with the
  Woman bundle (86499666 etc.) and the 10725626210 dynamic head. Accessories welded to a
  swapped-out part attach again.
- Proportion/BodyType: measured in Studio 0.744 with
  CreateHumanoidModelFromDescription. Each part's factor relative to Classic
  is `lerp(1, lerp(Normal, Slender, Proportion), BodyType)` (tables in
  `avatar_apply.luau`), divided by the factor of the part's own
  AvatarPartScaleType, times Width/Height/Depth (HeadScale for the head).
  Sizes come from OriginalSize and attachments from OriginalPosition.
  Joints are rebuilt on the rig attachments. HipHeight is the leg's attachment chain
  (root centre to sole, less half the root): BodyType 1 gives 2.951786,
  Studio's value exactly. An unscaled default rig reads 2.19 by that chain,
  while Studio's default model keeps the 2.0 stored in its file.
- Downloads yield: a worker fetches the asset while the script waits with
  task.wait. Before, it blocked the script like InsertService:LoadAsset.

## What's left, and why

- **AvatarCreationService moderation events** (AvatarAssetModerationCompleted,
  AvatarModerationCompleted, AvatarOutfitModerationCompleted): Roblox fires
  them after a publish passes moderation. rustudio never publishes (it must
  not write to the account), so they never fire. That is also what Studio
  does when nothing is published.
- **Avatar generation** (GenerateAvatarAsync, GenerateAvatar2DPreviewAsync,
  LoadGeneratedAvatarAsync, LoadAvatar2DPreviewAsync,
  RequestAvatarGenerationSessionAsync, AutoSetupAvatarAsync): these use a
  Roblox generation backend tied to a published experience's universe and
  create assets. Studio's argument errors are reproduced; after that,
  rustudio refuses the session ("not available for this experience") instead
  of calling the backend.
- **GetBatchTokenDetailsAsync**: Studio's token validation is reproduced. The
  lookup itself (`universes/{u}/avatar-creation-tokens/{t}`, host not
  confirmed) needs a published universe, so it fails as Studio does on a
  failed lookup ("Failed Unexpectedly").
- **ValidateUGC\*Async**: these check against GetValidationRules (attachments,
  size bounds, layered-only types, scales). Studio's full UGC validation
  library (triangle counts, cage distance, texture sizes) isn't reproduced.
- **CreateTextureIn(Cage|Target)SpaceAsync**: once both meshes load, they
  copy the image. That is right for Roblox's standard cages, which share the
  render mesh's UV layout, but other UV layouts aren't reprojected.
- **Skinning on the GPU**: skinning runs on the CPU. It costs about
  0.2 ms per animating 8.8k-corner head under heavy load and nothing for
  still parts. A GPU skinning pass would remove the re-upload.
- **Uncertain shapes**: InventoryPages entries are {AssetId, AssetType (name),   Name, Created}. GetItemDetails keeps the catalog's numeric AssetType.
- **Shared-runtime notes**: events fire through World::fire, so they are
  delivered deferred, while Studio 0.744 delivered the HumanoidDescription
  events immediately in the edit-mode command bar.
