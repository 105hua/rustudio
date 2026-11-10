//! Engine members beyond the core Instance API, following the public Roblox
//! docs: legacy lowercase aliases, ClickDetector / GUI / Sound / Tool /
//! Mouse events, Sound playback requests, CollectionService tags, raycasts
//! and region queries, Humanoid helpers, Players / Teams / HttpService
//! helpers. Lua-side services (TweenService, Debris, DataStores…) live in
//! library.luau.

use mlua::ObjectLike;
use rbx_dom_weak::types::Tags;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundCmd {
    Play,
    Stop,
    Pause,
    Resume,
}

/// Things scripts asked the engine for this frame; the play session acts on them.
#[derive(Default)]
pub struct Requests {
    pub sounds: Vec<(Ref, SoundCmd)>,
    /// Humanoid:MoveTo / WalkToPoint: (humanoid, target).
    pub move_to: Vec<(Ref, Vec3)>,
    /// Humanoid.Jump = true / ChangeState(Jumping).
    pub jumps: Vec<Ref>,
    /// Humanoid:EquipTool / UnequipTools: (humanoid, tool or None).
    pub equip: Vec<(Ref, Option<Ref>)>,
    /// Explosions parented into Workspace.
    pub explosions: Vec<Ref>,
    /// Humanoid:ChangeState other than Jumping: (humanoid, state name).
    pub change_state: Vec<(Ref, String)>,
    /// Seat:Sit / VehicleSeat:Sit: (seat, humanoid).
    pub sits: Vec<(Ref, Ref)>,
    /// Humanoid:SetStateEnabled: (humanoid, state name, enabled).
    pub state_enabled: Vec<(Ref, String, bool)>,
}

/// What the play session tells scripts each frame.
#[derive(Default, Clone)]
pub struct Inputs {
    /// The in-game menu is open (GuiService.MenuIsOpen).
    pub menu_open: bool,
    /// Gamepads plugged in (slots Gamepad1.. in order).
    pub gamepads: usize,
    /// Point and part under the cursor (client).
    pub mouse_hit: Option<Xform>,
    pub mouse_target: Option<Ref>,
    pub mouse_ray: (Vec3, Vec3),
    pub mouse_xy: [f32; 2],
    pub view_size: [f32; 2],
    /// The Player's Mouse object, once a script asked for it.
    pub mouse: Option<Ref>,
    /// Sound playback position / length by Sound.
    pub sound_pos: HashMap<Ref, f64>,
    pub sound_listener: Option<(u32, Option<Xform>, Option<Ref>)>,
    pub sound_len: HashMap<Ref, f64>,
    pub sound_playing: HashSet<Ref>,
    /// Sounds paused (Pause, or Playing = false) and not stopped since.
    pub sound_paused: HashSet<Ref>,
    /// Where the GUI was last drawn (client): AbsolutePosition / Size / TextBounds.
    pub gui_layout: HashMap<Ref, crate::gui::GuiBox>,
}

/// Deprecated camelCase spellings (still used by old models) → current name.
pub fn canonical(key: &str) -> Option<&'static str> {
    Some(match key {
        "findFirstChild" => "FindFirstChild",
        "FindFirstChildOfClass" => return None,
        "getChildren" | "children" => "GetChildren",
        "GetChildren" => return None,
        "clone" => "Clone",
        "destroy" => "Destroy",
        "remove" => "Remove",
        "isA" => "IsA",
        "isDescendantOf" => "IsDescendantOf",
        "getFullName" => "GetFullName",
        "clearAllChildren" => "ClearAllChildren",
        "getPlayers" => "GetPlayers",
        "playerFromCharacter" | "getPlayerFromCharacter" | "GetPlayerFromCharacter" => {
            "GetPlayerFromCharacter"
        }
        "breakJoints" => "BreakJoints",
        "makeJoints" => "MakeJoints",
        "move" | "moveTo" => "MoveTo",
        "takeDamage" => "TakeDamage",
        "loadAnimation" => "LoadAnimation",
        "addItem" => "AddItem",
        "service" | "getService" => "GetService",
        "findPartOnRay" => "FindPartOnRay",
        "findPartsInRegion3" => "FindPartsInRegion3",
        "getMouse" => "GetMouse",
        "isKeyDown" => "IsKeyDown",
        "resize" => "Resize",
        "getMass" => "GetMass",
        "GetModelCFrame" => return None,
        "lerp" => "Lerp",
        _ => return None,
    })
}

/// Events beyond the core set (event_exists in runtime.rs).
pub fn event_exists(class: &str, name: &str) -> bool {
    let is = |b: &str| doc::is_a(class, b);
    match name {
        "MouseClick" | "RightMouseClick" | "MouseHoverEnter" | "MouseHoverLeave" => {
            class == "ClickDetector"
        }
        "Activated" => is("GuiButton") || class == "Tool",
        "MouseButton1Click" | "MouseButton1Down" | "MouseButton1Up" | "MouseButton2Click"
        | "MouseButton2Down" | "MouseButton2Up" => is("GuiButton"),
        "MouseEnter" | "MouseLeave" | "MouseMoved" | "MouseWheelForward" | "MouseWheelBackward"
        | "SelectionGained" | "SelectionLost" => is("GuiObject"),
        "TouchTap" => is("GuiObject") || class == "UserInputService",
        "InputBegan" | "InputEnded" | "InputChanged" => {
            is("GuiObject") || class == "UserInputService"
        }
        "FocusLost" | "Focused" | "ReturnPressedFromOnScreenKeyboard" => class == "TextBox",
        "Chatted" | "Idled" | "CharacterAppearanceLoaded" => class == "Player",
        "MoveToFinished" | "Seated" | "Climbing" | "GettingUp" | "PlatformStanding" | "Ragdoll"
        | "Strafing" | "Swimming" | "FallingDown" | "AnimationPlayed" | "Touched" => {
            class == "Humanoid" || (name == "Touched" && is("BasePart"))
        }
        "Ended" | "Played" | "Paused" | "Resumed" | "Stopped" | "DidLoop" | "Loaded" => {
            class == "Sound"
                || (matches!(name, "Stopped" | "Ended" | "DidLoop") && class == "AnimationTrack")
                || (name == "Loaded" && class == "DataModel")
        }
        "KeyframeReached" => class == "AnimationTrack",
        "Deactivated" | "Equipped" | "Unequipped" => class == "Tool",
        "Button1Down" | "Button1Up" | "Button2Down" | "Button2Up" | "Move" | "KeyDown"
        | "KeyUp" | "WheelForward" | "WheelBackward" | "Idle" => is("Mouse"),
        "JumpRequest"
        | "TextBoxFocused"
        | "TextBoxFocusReleased"
        | "WindowFocused"
        | "WindowFocusReleased"
        | "LastInputTypeChanged"
        | "GamepadConnected"
        | "GamepadDisconnected"
        | "DeviceRotationChanged" => class == "UserInputService",
        "PreferredInputChanged"
        | "PointerAction"
        | "TouchStarted"
        | "TouchMoved"
        | "TouchEnded"
        | "TouchTapInWorld"
        | "TouchPinch"
        | "TouchSwipe"
        | "TouchLongPress"
        | "TouchPan"
        | "TouchRotate" => class == "UserInputService",
        "MenuOpened" | "MenuClosed" => class == "GuiService",
        "UserCFrameChanged" | "UserCFrameEnabled" | "NavigationRequested" => class == "VRService",
        "StateEnabledChanged" => class == "Humanoid",
        "Hit" => class == "Explosion",
        "Completed" => is("TweenBase"),
        "PlayerAdded" | "PlayerRemoved" => class == "Team",
        "Triggered"
        | "TriggerEnded"
        | "PromptShown"
        | "PromptHidden"
        | "PromptButtonHoldBegan"
        | "PromptButtonHoldEnded" => class == "ProximityPrompt",
        "Error" => class == "ScriptContext",
        "Close" => class == "DataModel",
        "LightingChanged" => class == "Lighting",
        "PromptProductPurchaseFinished"
        | "PromptGamePassPurchaseFinished"
        | "PromptPurchaseFinished" => class == "MarketplaceService",
        "OnTeleport" => class == "Player",
        _ => name.starts_with("TagAdded:") || name.starts_with("TagRemoved:"),
    }
}

/// (method, class that has it), implemented in `call` below.
pub const METHODS: &[(&str, &str)] = &[
    ("Spherecast", "WorldRoot"),
    ("Blockcast", "WorldRoot"),
    ("Shapecast", "WorldRoot"),
    ("GetRenderCFrame", "BasePart"),
    ("GetClosestPointOnSurface", "BasePart"),
    ("GetJoints", "BasePart"),
    ("GetNoCollisionConstraints", "BasePart"),
    ("ApplyImpulseAtPosition", "BasePart"),
    ("GetSunDirection", "Lighting"),
    ("GetMoonDirection", "Lighting"),
    ("GetMoonPhase", "Lighting"),
    ("players", "Players"),
    ("GetTestArgs", "StudioTestService"),
    ("BreakJoints", "Model"),
    ("BreakJoints", "BasePart"),
    ("MakeJoints", "Model"),
    ("MakeJoints", "BasePart"),
    ("GetModelCFrame", "Model"),
    ("GetModelSize", "Model"),
    ("GetExtentsSize", "Model"),
    ("GetBoundingBox", "Model"),
    ("TranslateBy", "Model"),
    ("GetScale", "Model"),
    ("GetTouchingParts", "BasePart"),
    ("GetConnectedParts", "BasePart"),
    ("GetRootPart", "BasePart"),
    ("SetNetworkOwner", "BasePart"),
    ("SetNetworkOwnershipAuto", "BasePart"),
    ("GetNetworkOwner", "BasePart"),
    ("CanSetNetworkOwnership", "BasePart"),
    ("Resize", "BasePart"),
    ("ApplyAngularImpulse", "BasePart"),
    ("GetVelocityAtPosition", "BasePart"),
    ("CanCollideWith", "BasePart"),
    ("Raycast", "WorldRoot"),
    ("FindPartOnRay", "WorldRoot"),
    ("FindPartOnRayWithIgnoreList", "WorldRoot"),
    ("FindPartOnRayWithWhitelist", "WorldRoot"),
    ("FindPartsInRegion3", "WorldRoot"),
    ("FindPartsInRegion3WithIgnoreList", "WorldRoot"),
    ("FindPartsInRegion3WithWhiteList", "WorldRoot"),
    ("GetPartBoundsInBox", "WorldRoot"),
    ("GetPartBoundsInRadius", "WorldRoot"),
    ("GetPartsInPart", "WorldRoot"),
    ("GetServerTimeNow", "Workspace"),
    ("GetRealPhysicsFPS", "Workspace"),
    ("MoveTo", "Humanoid"),
    ("EquipTool", "Humanoid"),
    ("UnequipTools", "Humanoid"),
    ("ChangeState", "Humanoid"),
    ("GetAccessories", "Humanoid"),
    ("AddAccessory", "Humanoid"),
    ("RemoveAccessories", "Humanoid"),
    ("ApplyDescription", "Humanoid"),
    ("BuildRigFromAttachments", "Humanoid"),
    ("SetStateEnabled", "Humanoid"),
    ("GetStateEnabled", "Humanoid"),
    ("Sit", "Seat"),
    ("Sit", "VehicleSeat"),
    ("Play", "Sound"),
    ("Stop", "Sound"),
    ("Pause", "Sound"),
    ("Resume", "Sound"),
    ("PlayLocalSound", "SoundService"),
    ("SetListener", "SoundService"),
    ("GetListener", "SoundService"),
    ("Activate", "Tool"),
    ("Deactivate", "Tool"),
    ("AddTag", "Instance"),
    ("RemoveTag", "Instance"),
    ("HasTag", "Instance"),
    ("GetTags", "Instance"),
    ("AddTag", "CollectionService"),
    ("RemoveTag", "CollectionService"),
    ("HasTag", "CollectionService"),
    ("GetTags", "CollectionService"),
    ("GetTagged", "CollectionService"),
    ("GetAllTags", "CollectionService"),
    ("GetInstanceAddedSignal", "CollectionService"),
    ("GetInstanceRemovedSignal", "CollectionService"),
    ("GetTeams", "Teams"),
    ("GetPlayers", "Team"),
    ("GetPlayerByUserId", "Players"),
    ("GetMouse", "Player"),
    ("HasAppearanceLoaded", "Player"),
    ("DistanceFromCharacter", "Player"),
    ("GetNetworkPing", "Player"),
    ("ClearCharacterAppearance", "Player"),
    ("JSONEncode", "HttpService"),
    ("JSONDecode", "HttpService"),
    ("GenerateGUID", "HttpService"),
    ("UrlEncode", "HttpService"),
    ("GetMinutesAfterMidnight", "Lighting"),
    ("SetMinutesAfterMidnight", "Lighting"),
    ("GetMouseLocation", "UserInputService"),
    ("IsMouseButtonPressed", "UserInputService"),
    ("GetKeysPressed", "UserInputService"),
    ("IsLoaded", "DataModel"),
    ("BindToClose", "DataModel"),
    ("SetCoreGuiEnabled", "StarterGui"),
    ("GetCoreGuiEnabled", "StarterGui"),
    ("SetCore", "StarterGui"),
    ("GetCore", "StarterGui"),
    ("TeleportAsync", "TeleportService"),
    ("PreloadAsync", "ContentProvider"),
    ("RegisterKeyframeSequence", "KeyframeSequenceProvider"),
    ("RegisterActiveKeyframeSequence", "KeyframeSequenceProvider"),
    ("GetKeyframeSequenceAsync", "KeyframeSequenceProvider"),
    ("LoadAsset", "InsertService"),
    ("CreateMeshPartAsync", "AssetService"),
    ("CreateMeshPartAsync", "InsertService"),
    ("LoadAssetVersion", "InsertService"),
    ("RemoveDefaultLoadingScreen", "ReplicatedFirst"),
    ("GetHumanoidDescriptionFromUserId", "Players"),
    ("GetHumanoidDescriptionFromOutfitId", "Players"),
    ("CreateHumanoidModelFromDescription", "Players"),
    ("CreateHumanoidModelFromUserId", "Players"),
    ("GetLastInputType", "UserInputService"),
    ("GetConnectedGamepads", "UserInputService"),
    ("GetGamepadConnected", "UserInputService"),
    ("FillBlock", "Terrain"),
    ("FillBall", "Terrain"),
    ("FillCylinder", "Terrain"),
    ("FillWedge", "Terrain"),
    ("FillRegion", "Terrain"),
    ("Clear", "Terrain"),
    ("ReplaceMaterial", "Terrain"),
    ("WriteVoxels", "Terrain"),
    ("ReadVoxels", "Terrain"),
    ("WriteVoxelChannels", "Terrain"),
    ("ReadVoxelChannels", "Terrain"),
    ("CopyRegion", "Terrain"),
    ("PasteRegion", "Terrain"),
    ("CellCenterToWorld", "Terrain"),
    ("CellCornerToWorld", "Terrain"),
    ("WorldToCell", "Terrain"),
    ("WorldToCellPreferSolid", "Terrain"),
    ("WorldToCellPreferEmpty", "Terrain"),
    ("SetCell", "Terrain"),
    ("SetMaterialColor", "Terrain"),
    ("GetMaterialColor", "Terrain"),
];

/// Methods written in Lua (library.luau).
pub const LUA_METHODS: &[(&str, &str)] = &[
    ("PreloadAsync", "ContentProvider"),
    ("AdvanceToNextPageAsync", "Pages"),
    ("RegisterCollisionGroup", "WorldRoot"),
    ("UnregisterCollisionGroup", "WorldRoot"),
    ("RenameCollisionGroup", "WorldRoot"),
    ("CollisionGroupSetCollidable", "WorldRoot"),
    ("CollisionGroupsAreCollidable", "WorldRoot"),
    ("IsCollisionGroupRegistered", "WorldRoot"),
    ("GetRegisteredCollisionGroups", "WorldRoot"),
    ("GetMaxCollisionGroups", "WorldRoot"),
    ("GetMaxCollisionGroups", "PhysicsService"),
    ("GetCollisionGroupName", "PhysicsService"),
    ("CollisionGroupContainsPart", "PhysicsService"),
    ("TweenPosition", "GuiObject"),
    ("TweenSize", "GuiObject"),
    ("TweenSizeAndPosition", "GuiObject"),
    ("GetLimb", "Humanoid"),
    ("GetBodyPartR15", "Humanoid"),
    ("GetMoveVelocity", "Humanoid"),
    ("GetActor", "Instance"),
    ("BindToMessage", "Actor"),
    ("BindToMessageParallel", "Actor"),
    ("SendMessage", "Actor"),
    ("GetLastInputType", "UserInputService"),
    ("RegisterKeyframeSequence", "KeyframeSequenceProvider"),
    ("RegisterActiveKeyframeSequence", "KeyframeSequenceProvider"),
    ("Move", "Humanoid"),
    ("Move", "Player"),
    ("UnbindAllActions", "ContextActionService"),
    ("GetBoundActionInfo", "ContextActionService"),
    ("GetAllBoundActionInfo", "ContextActionService"),
    ("GetButton", "ContextActionService"),
    ("Emit", "ParticleEmitter"),
    ("Clear", "ParticleEmitter"),
    ("SetTitle", "ContextActionService"),
    ("SetDescription", "ContextActionService"),
    ("SetImage", "ContextActionService"),
    ("SetPosition", "ContextActionService"),
    ("BindActivate", "ContextActionService"),
    ("UnbindActivate", "ContextActionService"),
    ("GetMouseDelta", "UserInputService"),
    ("GetGamepadState", "UserInputService"),
    ("GetNavigationGamepads", "UserInputService"),
    ("GetSupportedGamepadKeyCodes", "UserInputService"),
    ("IsGamepadButtonDown", "UserInputService"),
    ("GamepadSupports", "UserInputService"),
    ("IsNavigationGamepad", "UserInputService"),
    ("GetStringForKeyCode", "UserInputService"),
    ("GetImageForKeyCode", "UserInputService"),
    ("GetDeviceRotation", "UserInputService"),
    ("GetDeviceGravity", "UserInputService"),
    ("GetUserCFrame", "VRService"),
    ("GetUserCFrameEnabled", "VRService"),
    ("RecenterUserHeadCFrame", "VRService"),
    ("RequestNavigation", "VRService"),
    ("IsTenFootInterface", "GuiService"),
    ("GetGuiInset", "GuiService"),
    ("GetEmotesMenuOpen", "GuiService"),
    ("GetInspectMenuEnabled", "GuiService"),
    ("RegisterTouchCameraMovementMode", "PlayerScripts"),
    ("RegisterComputerCameraMovementMode", "PlayerScripts"),
    ("RegisterTouchMovementMode", "PlayerScripts"),
    ("RegisterComputerMovementMode", "PlayerScripts"),
    ("ClearComputerCameraMovementModes", "PlayerScripts"),
    ("ClearComputerMovementModes", "PlayerScripts"),
    ("ClearTouchCameraMovementModes", "PlayerScripts"),
    ("ClearTouchMovementModes", "PlayerScripts"),
    ("ViewportPointToRay", "Camera"),
    ("ScreenPointToRay", "Camera"),
    ("WorldToViewportPoint", "Camera"),
    ("WorldToScreenPoint", "Camera"),
    ("GetRenderCFrame", "Camera"),
    ("GetPartsObscuringTarget", "Camera"),
    ("GetLargestCutoffDistance", "Camera"),
    ("GetRoll", "Camera"),
    ("SetRoll", "Camera"),
    ("ZoomToExtents", "Camera"),
    ("AddItem", "Debris"),
    ("Create", "TweenService"),
    ("GetValue", "TweenService"),
    ("Play", "Tween"),
    ("Pause", "Tween"),
    ("Cancel", "Tween"),
    ("LoadAnimation", "Humanoid"),
    ("LoadAnimation", "Animator"),
    ("LoadAnimation", "AnimationController"),
    ("GetPlayingAnimationTracks", "Humanoid"),
    ("GetPlayingAnimationTracks", "Animator"),
    ("Play", "AnimationTrack"),
    ("Stop", "AnimationTrack"),
    ("AdjustSpeed", "AnimationTrack"),
    ("AdjustWeight", "AnimationTrack"),
    ("GetMarkerReachedSignal", "AnimationTrack"),
    ("GetTimeOfKeyframe", "AnimationTrack"),
    ("GetDataStore", "DataStoreService"),
    ("GetGlobalDataStore", "DataStoreService"),
    ("GetOrderedDataStore", "DataStoreService"),
    ("RegisterCollisionGroup", "PhysicsService"),
    ("CreateCollisionGroup", "PhysicsService"),
    ("UnregisterCollisionGroup", "PhysicsService"),
    ("RemoveCollisionGroup", "PhysicsService"),
    ("RenameCollisionGroup", "PhysicsService"),
    ("CollisionGroupSetCollidable", "PhysicsService"),
    ("CollisionGroupsAreCollidable", "PhysicsService"),
    ("IsCollisionGroupRegistered", "PhysicsService"),
    ("GetRegisteredCollisionGroups", "PhysicsService"),
    ("GetCollisionGroups", "PhysicsService"),
    ("SetPartCollisionGroup", "PhysicsService"),
    ("GetCollisionGroupId", "PhysicsService"),
    ("BindAction", "ContextActionService"),
    ("BindActionAtPriority", "ContextActionService"),
    ("UnbindAction", "ContextActionService"),
    ("BindToRenderStep", "RunService"),
    ("UnbindFromRenderStep", "RunService"),
    ("BulkMoveTo", "WorldRoot"),
    ("IsUserFeatureEnabled", "UserSettings"),
    ("GetService", "UserSettings"),
    ("FindService", "UserSettings"),
    ("GetSharedTable", "SharedTableRegistry"),
    ("SetSharedTable", "SharedTableRegistry"),
    ("GetProductInfo", "MarketplaceService"),
    ("GetProductInfoAsync", "MarketplaceService"),
    // (LocalizationService / Translator: domains/players_chat.)
    ("FilterStringAsync", "TextService"),
    ("GetNonChatStringForUserAsync", "TextFilterResult"),
    ("GetNonChatStringForBroadcastAsync", "TextFilterResult"),
    ("GetChatForUserAsync", "TextFilterResult"),
];

fn none() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter([v]))
}

// ------------------------------------------------------------ terrain edits

fn num_arg(args: &MultiValue, i: usize) -> Option<f32> {
    match arg(args, i) {
        Value::Number(n) => Some(n as f32),
        Value::Integer(n) => Some(n as f32),
        _ => None,
    }
}

/// A terrain material argument: its SmoothGrid slot (Air 0, Water 1, …).
fn terrain_material(args: &MultiValue, i: usize, call: &str) -> mlua::Result<u8> {
    let e = opt_ud::<LEnumItem>(&arg(args, i))
        .ok_or_else(|| rt_err(&format!("Terrain:{call}: Enum.Material expected")))?;
    crate::terrain::material_index(&e.name)
        .ok_or_else(|| rt_err(&format!("Terrain:{call}: Enum.Material.{} is not a terrain material", e.name)))
}

/// FillBlock's distance to a box of half size `h` (local coordinates): the
/// largest per-axis distance, inside and out (so a voxel's share is the
/// smallest per-axis share). Studio fills a 2-stud cube's four voxels at
/// 0.254 each, which a Euclidean distance (0.146) doesn't give (round 21,
/// ReadVoxels in an offline Studio).
fn box_distance(p: Vec3, h: Vec3) -> f32 {
    (p.abs() - h).max_element()
}

/// Terrain editing (Roblox's Terrain:Fill…, Clear, ReplaceMaterial and the
/// material colours), round 19: Place C builds its planet surfaces with
/// FillBlock, Place B its water. Edits go to a decoded copy of the voxels
/// that `flush_terrain` writes back to SmoothGrid once a step.
fn terrain_call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    use crate::terrain::{self, Voxels};
    if name == "SetMaterialColor" || name == "GetMaterialColor" {
        let slot = terrain_material(args, 1, name)? as usize;
        let mut w = world(lua)?;
        let mut blob: Vec<u8> = match w.d().prop(this, "MaterialColors") {
            Some(Variant::BinaryString(b)) if AsRef::<[u8]>::as_ref(&b).len() >= 69 => {
                AsRef::<[u8]>::as_ref(&b).to_vec()
            }
            _ => (0..23u8).flat_map(terrain::material_color).collect(),
        };
        if name == "GetMaterialColor" {
            let c = &blob[slot * 3..slot * 3 + 3];
            drop(w);
            let v = LColor3([c[0], c[1], c[2]].map(|x| x as f32 / 255.0));
            return one(Value::UserData(lua.create_userdata(v)?));
        }
        let c = opt_ud::<LColor3>(&arg(args, 2))
            .ok_or_else(|| rt_err("Terrain:SetMaterialColor: Color3 expected"))?;
        blob[slot * 3..slot * 3 + 3].copy_from_slice(&c.0.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8));
        if let Some(i) = w.dm().dom.get_by_ref_mut(this) {
            i.properties.insert(
                ustr("MaterialColors"),
                Variant::BinaryString(blob.into()),
            );
        }
        w.dm().mark_prop(this, ustr("MaterialColors"));
        w.record(RepOp::Prop(this, "MaterialColors".into()));
        return none();
    }
    // Cell ↔ world (round 25): cells are 4-stud voxels from the origin.
    match name {
        "CellCenterToWorld" | "CellCornerToWorld" => {
            let c = Vec3::new(
                num_arg(args, 1).unwrap_or(0.0).floor(),
                num_arg(args, 2).unwrap_or(0.0).floor(),
                num_arg(args, 3).unwrap_or(0.0).floor(),
            );
            let half = if name == "CellCenterToWorld" { 0.5 } else { 0.0 };
            return one(vec3_value(lua, (c + half) * terrain::VOXEL)?);
        }
        "WorldToCell" | "WorldToCellPreferSolid" | "WorldToCellPreferEmpty" => {
            let p = vec3_arg(args, 1, &format!("Terrain:{name}"))?;
            return one(vec3_value(lua, (p / terrain::VOXEL).floor())?);
        }
        _ => {}
    }
    // The voxels being edited: this frame's copy, else the saved grid.
    let mut voxels = {
        let mut w = world(lua)?;
        match w.terrain_edit.take() {
            Some((r, v)) if r == this => v,
            _ => match Voxels::from_dom(&w.d().dom) {
                Some(Ok(v)) => v,
                _ => Voxels::empty(),
            },
        }
    };
    let corners = |xf: &Xform, h: Vec3| {
        let e = xf.rot.x_axis.abs() * h.x + xf.rot.y_axis.abs() * h.y + xf.rot.z_axis.abs() * h.z;
        ((xf.pos - e).to_array(), (xf.pos + e).to_array())
    };
    let cframe = |i: usize| {
        opt_ud::<LCFrame>(&arg(args, i))
            .map(|c| c.0)
            .ok_or_else(|| rt_err(&format!("Terrain:{name}: CFrame expected")))
    };
    let region = |i: usize| {
        opt_ud::<crate::ltypes::LRegion3>(&arg(args, i))
            .ok_or_else(|| rt_err(&format!("Terrain:{name}: Region3 expected")))
    };
    match name {
        "FillBlock" | "FillWedge" => {
            let xf = cframe(1)?;
            let h = vec3_arg(args, 2, name)?.abs() * 0.5;
            let mat = terrain_material(args, 3, name)?;
            let (lo, hi) = corners(&xf, h);
            let wedge = name == "FillWedge";
            voxels.fill(lo, hi, mat, |c| {
                let p = xf.rot.transpose() * (Vec3::from(c) - xf.pos);
                let d = box_distance(p, h);
                if wedge {
                    // A WedgePart: full height at the back (+Z), none at the front.
                    let n = Vec3::new(0.0, h.z, -h.y).normalize_or_zero();
                    d.max(n.dot(p))
                } else {
                    d
                }
            });
        }
        "FillBall" => {
            let c0 = vec3_arg(args, 1, name)?;
            let r = num_arg(args, 2).unwrap_or(0.0).abs();
            let mat = terrain_material(args, 3, name)?;
            voxels.fill((c0 - r).to_array(), (c0 + r).to_array(), mat, |c| {
                (Vec3::from(c) - c0).length() - r
            });
        }
        "FillCylinder" => {
            let xf = cframe(1)?;
            let height = num_arg(args, 2).unwrap_or(0.0).abs();
            let r = num_arg(args, 3).unwrap_or(0.0).abs();
            let mat = terrain_material(args, 4, name)?;
            let (lo, hi) = corners(&xf, Vec3::new(r, height * 0.5, r));
            voxels.fill(lo, hi, mat, |c| {
                let p = xf.rot.transpose() * (Vec3::from(c) - xf.pos);
                let q = glam::Vec2::new(glam::Vec2::new(p.x, p.z).length() - r, p.y.abs() - height * 0.5);
                q.max(glam::Vec2::ZERO).length() + q.max_element().min(0.0)
            });
        }
        "FillRegion" => {
            let rg = region(1)?;
            let mat = terrain_material(args, 3, name)?;
            let (lo, hi) = (rg.min, rg.max);
            // Whole voxels whose centres are inside the region.
            voxels.fill(lo.to_array(), hi.to_array(), mat, |c| {
                let c = Vec3::from(c);
                if c.cmpge(lo).all() && c.cmple(hi).all() {
                    -terrain::VOXEL
                } else {
                    terrain::VOXEL
                }
            });
        }
        "ReplaceMaterial" => {
            let rg = region(1)?;
            let from = terrain_material(args, 3, name)?;
            let to = terrain_material(args, 4, name)?;
            let a = (rg.min / terrain::VOXEL).round().as_ivec3().to_array();
            let b = (rg.max / terrain::VOXEL).round().as_ivec3().to_array();
            voxels.replace(a, b, from, to);
        }
        "Clear" => voxels.clear(),
        // CopyRegion(Region3int16 of cells, Max inclusive) → a TerrainRegion
        // (ExtentsMin / ExtentsMax, saved only, its cells in SmoothGrid at
        // their world cells); PasteRegion(region, corner cell,
        // pasteEmptyCells). Round 25; Max and the saved form measured in
        // round 26 (region_probe.luau: (10, 1, 20) .. (13, 2, 23) holds 4 × 2
        // × 4 cells, and saves those extents and world cells).
        "CopyRegion" => {
            let rg = opt_ud::<crate::ltypes::LRegion3int16>(&arg(args, 1))
                .ok_or_else(|| rt_err("Terrain:CopyRegion: Region3int16 expected"))?;
            let (lo, hi) = (rg.min.0.map(i32::from), rg.max.0.map(i32::from));
            let size = [0, 1, 2].map(|k| (hi[k] - lo[k] + 1).max(0));
            let region = crate::terrain_region::Region::copy(&voxels, lo, size);
            world(lua)?.terrain_edit = Some((this, voxels));
            let r = create_any(lua, "TerrainRegion")?;
            let v3 = |c: [i32; 3]| Variant::Vector3int16(rbx_dom_weak::types::Vector3int16::new(c[0] as i16, c[1] as i16, c[2] as i16));
            if let Some(i) = world(lua)?.dm().dom.get_by_ref_mut(r) {
                i.properties.insert(ustr("ExtentsMin"), v3(lo));
                i.properties.insert(ustr("ExtentsMax"), v3([0, 1, 2].map(|k| lo[k] + size[k] - 1)));
                i.properties.insert(ustr("SmoothGrid"), Variant::BinaryString(region.to_smooth_grid(lo).into()));
            }
            return one(inst_value(lua, r)?);
        }
        "PasteRegion" => {
            let src = opt_inst(&arg(args, 1))
                .filter(|r| inst_meta(lua, *r).is_ok_and(|m| m.0 == "TerrainRegion"))
                .ok_or_else(|| rt_err("Terrain:PasteRegion: region has to be a TerrainRegion"))?;
            let corner = opt_ud::<crate::ltypes::LVector3int16>(&arg(args, 2))
                .ok_or_else(|| rt_err("Terrain:PasteRegion: Vector3int16 expected"))?
                .0
                .map(i32::from);
            let paste_empty = matches!(arg(args, 3), Value::Boolean(true));
            let region = {
                let w = world(lua)?;
                let d = w.d();
                let ext = |k: &str| match d.prop(src, k) {
                    Some(Variant::Vector3int16(v)) => [v.x as i32, v.y as i32, v.z as i32],
                    _ => [0; 3],
                };
                let (min, max) = (ext("ExtentsMin"), ext("ExtentsMax"));
                let grid = match d.prop(src, "SmoothGrid") {
                    Some(Variant::BinaryString(b)) => AsRef::<[u8]>::as_ref(&b).to_vec(),
                    _ => Vec::new(),
                };
                crate::terrain_region::Region::from_smooth_grid(&grid, min, max)
            };
            match region {
                Ok(region) => region.paste(&mut voxels, corner, paste_empty),
                Err(e) => {
                    world(lua)?.terrain_edit = Some((this, voxels));
                    return Err(rt_err(format!("Terrain:PasteRegion: {e}")));
                }
            }
        }
        // ReadVoxelChannels(region, 4, {names}) → {Name = [x][y][z], Size}
        // and WriteVoxelChannels(region, 4, {Name = [x][y][z]}): SolidMaterial,
        // SolidOccupancy and LiquidOccupancy, the grid's two layers (a Water
        // voxel is Air with liquid; a part-full solid may hold shoreline water).
        "ReadVoxelChannels" | "WriteVoxelChannels" => {
            let rg = region(1)?;
            let res = num_arg(args, 2).unwrap_or(4.0);
            if (res - terrain::VOXEL).abs() > 1e-6 {
                return Err(rt_err("Terrain:".to_string() + name + ": resolution must be 4"));
            }
            let a = (rg.min / terrain::VOXEL).round().as_ivec3();
            let n = ((rg.max / terrain::VOXEL).round().as_ivec3() - a).max(glam::IVec3::ZERO);
            const CHANNELS: [&str; 3] = ["SolidMaterial", "SolidOccupancy", "LiquidOccupancy"];
            // (solid material, solid occupancy, liquid occupancy) of a cell.
            let read = |v: glam::IVec3| -> (u8, f32, f32) {
                let (m, o) = voxels.get(v.x, v.y, v.z);
                if m == terrain::WATER {
                    // LiquidOccupancy reads (byte + 1) / 255 where ReadVoxels
                    // reads (byte + 1) / 256 (round 26, region_probe.luau:
                    // WriteVoxels Water 0.1 reads 0.101562 and 0.101961;
                    // Place B's lake one step over rustudio's above half).
                    (0, 0.0, (o * 256.0 / 255.0).min(1.0))
                } else {
                    (m, o, voxels.water(v.x, v.y, v.z))
                }
            };
            if name == "ReadVoxelChannels" {
                let wanted: Vec<String> = match arg(args, 3) {
                    Value::Table(t) => t.sequence_values::<String>().flatten().collect(),
                    _ => return Err(rt_err("Terrain:ReadVoxelChannels: channel names expected")),
                };
                let out = lua.create_table()?;
                for ch in CHANNELS.iter().filter(|c| wanted.iter().any(|w| w == *c)) {
                    let t = lua.create_table()?;
                    for x in 0..n.x {
                        let tx = lua.create_table()?;
                        for y in 0..n.y {
                            let ty = lua.create_table()?;
                            for z in 0..n.z {
                                let (m, so, lo) = read(a + glam::IVec3::new(x, y, z));
                                let item = match *ch {
                                    "SolidMaterial" => LEnumItem::from_name("Material", terrain::material_name(m))
                                        .map(|e| crate::ltypes::enum_value(lua, e))
                                        .transpose()?
                                        .unwrap_or(Value::Nil),
                                    "SolidOccupancy" => Value::Number(so as f64),
                                    _ => Value::Number(lo as f64),
                                };
                                ty.raw_set(z + 1, item)?;
                            }
                            tx.raw_set(y + 1, ty)?;
                        }
                        t.raw_set(x + 1, tx)?;
                    }
                    out.raw_set(*ch, t)?;
                }
                out.raw_set("Size", vec3_value(lua, n.as_vec3())?)?;
                world(lua)?.terrain_edit = Some((this, voxels));
                return one(Value::Table(out));
            }
            let Value::Table(chans) = arg(args, 3) else {
                return Err(rt_err("Terrain:WriteVoxelChannels: channel table expected"));
            };
            let given: Vec<(usize, Table)> = CHANNELS
                .iter()
                .enumerate()
                .filter_map(|(i, c)| chans.get::<Option<Table>>(*c).ok().flatten().map(|t| (i, t)))
                .collect();
            let mut out = Vec::new();
            for x in 0..n.x {
                for y in 0..n.y {
                    for z in 0..n.z {
                        let v = a + glam::IVec3::new(x, y, z);
                        let (mut m, mut so, mut lq) = read(v);
                        for (i, t) in &given {
                            let cell: Value = t.get::<Option<Table>>(x + 1)?
                                .and_then(|tx| tx.get::<Option<Table>>(y + 1).ok().flatten())
                                .map(|ty| ty.get::<Value>(z + 1))
                                .transpose()?
                                .unwrap_or(Value::Nil);
                            match i {
                                0 => {
                                    m = opt_ud::<LEnumItem>(&cell)
                                        .and_then(|e| terrain::material_index(&e.name))
                                        .ok_or_else(|| rt_err("Terrain:WriteVoxelChannels: Enum.Material expected"))?;
                                    if m == terrain::WATER {
                                        m = 0;
                                    }
                                }
                                1 => so = num(&cell).unwrap_or(0.0).clamp(0.0, 1.0) as f32,
                                _ => lq = num(&cell).unwrap_or(0.0).clamp(0.0, 1.0) as f32,
                            }
                        }
                        let solid = terrain::occupancy_to_byte(so).filter(|_| m != 0);
                        let water = (lq > 0.0).then(|| ((lq * 256.0).round() as u16).clamp(1, 256));
                        let cell = match (solid, water) {
                            (Some(b), w) => (m, b, if b == 255 { 0 } else { w.unwrap_or(0) }),
                            (None, Some(_)) => match terrain::occupancy_to_byte(lq) {
                                Some(b) => (terrain::WATER, b, 0),
                                None => (0, 0, 0),
                            },
                            (None, None) => (0, 0, 0),
                        };
                        out.push((v.to_array(), cell));
                    }
                }
            }
            voxels.set_raw_many(out);
        }
        // WriteVoxels(region, 4, materials[x][y][z], occupancies[x][y][z]) and
        // ReadVoxels(region, 4) → (materials, occupancies), each with a Size:
        // the voxels whose cells lie in the (4-stud aligned) region.
        "WriteVoxels" | "ReadVoxels" => {
            let rg = region(1)?;
            let res = num_arg(args, 2).unwrap_or(4.0);
            if (res - terrain::VOXEL).abs() > 1e-6 {
                return Err(rt_err("Terrain:".to_string() + name + ": resolution must be 4"));
            }
            let a = (rg.min / terrain::VOXEL).round().as_ivec3();
            let b = (rg.max / terrain::VOXEL).round().as_ivec3();
            let n = (b - a).max(glam::IVec3::ZERO);
            if name == "ReadVoxels" {
                let mats = lua.create_table()?;
                let occs = lua.create_table()?;
                for x in 0..n.x {
                    let (mx, ox) = (lua.create_table()?, lua.create_table()?);
                    for y in 0..n.y {
                        let (my, oy) = (lua.create_table()?, lua.create_table()?);
                        for z in 0..n.z {
                            let v = a + glam::IVec3::new(x, y, z);
                            let (m, o) = voxels.get(v.x, v.y, v.z);
                            let w = voxels.water(v.x, v.y, v.z);
                            let (m, o) = if m == 0 && w > 0.0 { (terrain::WATER, w) } else { (m, o) };
                            let item = LEnumItem::from_name("Material", terrain::material_name(m))
                                .map(|e| crate::ltypes::enum_value(lua, e))
                                .transpose()?
                                .unwrap_or(Value::Nil);
                            my.raw_set(z + 1, item)?;
                            oy.raw_set(z + 1, o as f64)?;
                        }
                        mx.raw_set(y + 1, my)?;
                        ox.raw_set(y + 1, oy)?;
                    }
                    mats.raw_set(x + 1, mx)?;
                    occs.raw_set(x + 1, ox)?;
                }
                let size = LVector3(n.as_vec3()).into_value();
                mats.raw_set("Size", size.clone())?;
                occs.raw_set("Size", size)?;
                world(lua)?.terrain_edit = Some((this, voxels));
                return Ok(MultiValue::from_iter([Value::Table(mats), Value::Table(occs)]));
            }
            let (Value::Table(mats), Value::Table(occs)) = (arg(args, 3), arg(args, 4)) else {
                return Err(rt_err("Terrain:WriteVoxels: material and occupancy arrays expected"));
            };
            let mut out = Vec::new();
            for x in 0..n.x {
                let (mx, ox): (Table, Table) = (mats.get(x + 1)?, occs.get(x + 1)?);
                for y in 0..n.y {
                    let (my, oy): (Table, Table) = (mx.get(y + 1)?, ox.get(y + 1)?);
                    for z in 0..n.z {
                        let m = opt_ud::<LEnumItem>(&my.get::<Value>(z + 1)?)
                            .and_then(|e| terrain::material_index(&e.name))
                            .ok_or_else(|| rt_err("Terrain:WriteVoxels: Enum.Material expected"))?;
                        let o: f64 = oy.get::<Option<f64>>(z + 1)?.unwrap_or(0.0);
                        let v = a + glam::IVec3::new(x, y, z);
                        // WriteVoxels truncates to the byte (FillBlock rounds):
                        // 0.1 / 0.25 / 0.3 / 0.5 / 0.7 / 0.9 read back as
                        // 26 / 64 / 77 / 128 / 179 / 230 ÷ 256 in Studio (round
                        // 26, region_probe.luau), bytes floor(share × 255).
                        let o = (o.clamp(0.0, 1.0) * 255.0).floor() / 255.0;
                        out.push((v.to_array(), m, o as f32));
                    }
                }
            }
            voxels.set_many(out);
        }
        _ => {}
    }
    world(lua)?.terrain_edit = Some((this, voxels));
    none()
}

/// Writes scripts' terrain edits back to Terrain.SmoothGrid (and
/// replicates it), so the drawn terrain and the physics follow.
pub fn flush_terrain(lua: &Lua) {
    let Some(mut w) = lua.app_data_mut::<World>() else {
        return;
    };
    let Some((r, v)) = w.terrain_edit.take() else {
        return;
    };
    if w.d().dom.get_by_ref(r).is_none() {
        return;
    }
    let blob = v.encode();
    if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
        i.properties
            .insert(ustr("SmoothGrid"), Variant::BinaryString(blob.into()));
    }
    w.dm().mark_prop(r, ustr("SmoothGrid"));
    w.record(RepOp::Prop(r, "SmoothGrid".into()));
}

fn vec3_arg(args: &MultiValue, i: usize, what: &str) -> mlua::Result<Vec3> {
    opt_ud::<LVector3>(&arg(args, i))
        .map(|v| v.0)
        .ok_or_else(|| rt_err(format!("{what}: Vector3 expected")))
}

fn ud_field<T: Clone + 'static>(v: &Value, key: &str) -> Option<T> {
    match v {
        Value::UserData(ud) => ud.get::<Value>(key).ok().and_then(|x| opt_ud::<T>(&x)),
        Value::Table(t) => t.get::<Value>(key).ok().and_then(|x| opt_ud::<T>(&x)),
        _ => None,
    }
}

fn ud_value(v: &Value, key: &str) -> Value {
    match v {
        Value::UserData(ud) => ud.get::<Value>(key).unwrap_or(Value::Nil),
        Value::Table(t) => t.get::<Value>(key).unwrap_or(Value::Nil),
        _ => Value::Nil,
    }
}

fn refs_in(v: &Value) -> Vec<Ref> {
    match v {
        Value::Table(t) => t
            .sequence_values::<Value>()
            .flatten()
            .filter_map(|x| opt_inst(&x))
            .collect(),
        other => opt_inst(other).into_iter().collect(),
    }
}

fn vec3_value(lua: &Lua, v: Vec3) -> mlua::Result<Value> {
    Ok(LVector3(v).into_value())
}

fn cframe_value(lua: &Lua, xf: Xform) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(LCFrame(xf))?))
}

// ------------------------------------------------------------- world queries

/// A Workspace part for queries: (ref, CFrame, size, CanCollide, CanQuery).
#[derive(Clone, Copy)]
struct QPart {
    r: Ref,
    xf: Xform,
    size: Vec3,
    collide: bool,
    query: bool,
    /// A MeshPart whose CollisionFidelity isn't Box: rays test its mesh.
    mesh: bool,
    /// query_shapes::BOX / BALL / CYLINDER / WEDGE / CORNER_WEDGE: rays,
    /// sweeps and GetPartsInPart test the true shape.
    shape: u8,
}

/// CollectionService's tag → instances, kept up to date from the change
/// journal: a Tree mark re-reads only that subtree, a Tags change only that
/// instance (round 27 rebuilt the whole index on any Tree mark, so with
/// enemies and effects spawning every frame each GetTagged walked the whole
/// 65k-instance DataModel: ~2.5 ms a call on a live place).
#[derive(Default)]
struct TagCache {
    doc_id: u64,
    valid: bool,
    /// tag → (instances in tag-add order, membership). Removals only drop
    /// from the set; the order list is compacted when read.
    by_tag: HashMap<String, (Vec<Ref>, rustc_hash::FxHashSet<Ref>, bool)>,
    /// instance → its tags as indexed.
    of: rustc_hash::FxHashMap<Ref, Vec<String>>,
}

fn tags_in(v: Option<&Variant>) -> Vec<String> {
    match v {
        Some(Variant::Tags(t)) => t.iter().map(|s| s.to_string()).collect(),
        Some(Variant::BinaryString(b)) => Tags::decode(b.as_ref())
            .map(|t| t.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

impl TagCache {
    fn set(&mut self, r: Ref, tags: Vec<String>) {
        let old = self.of.remove(&r).unwrap_or_default();
        if old == tags {
            if !tags.is_empty() {
                self.of.insert(r, tags);
            }
            return;
        }
        for t in &old {
            if !tags.contains(t) {
                if let Some(e) = self.by_tag.get_mut(t) {
                    if e.1.remove(&r) {
                        e.2 = true;
                    }
                }
            }
        }
        for t in &tags {
            let e = self.by_tag.entry(t.clone()).or_default();
            if e.1.insert(r) {
                e.0.push(r);
            }
        }
        if !tags.is_empty() {
            self.of.insert(r, tags);
        }
    }
}

fn tagged(lua: &Lua, tag: &str) -> mlua::Result<Vec<Ref>> {
    if lua.app_data_ref::<TagCache>().is_none() {
        lua.set_app_data(TagCache::default());
    }
    let mut w = world(lua)?;
    let d = w.dm();
    let marks = d.marks_for(doc::Reader::Tags);
    let mut c = lua
        .app_data_mut::<TagCache>()
        .ok_or_else(|| rt_err("tag cache missing"))?;
    let key = ustr("Tags");
    let marks = if c.doc_id != d.id || !c.valid { doc::Marks::All } else { marks };
    match marks {
        doc::Marks::All => {
            c.by_tag.clear();
            c.of.clear();
            for i in d.dom.descendants() {
                let t = tags_in(i.properties.get(&key));
                if !t.is_empty() {
                    c.set(i.referent(), t);
                }
            }
            c.doc_id = d.id;
            c.valid = true;
        }
        doc::Marks::Some(m) => {
            for mk in m {
                match mk {
                    doc::Mark::Tree(r) => {
                        for x in d.subtree(r) {
                            let t = d
                                .dom
                                .get_by_ref(x)
                                .map(|i| tags_in(i.properties.get(&key)))
                                .unwrap_or_default();
                            if !t.is_empty() || c.of.contains_key(&x) {
                                c.set(x, t);
                            }
                        }
                    }
                    doc::Mark::Prop(r, n) if n.as_str() == "Tags" => {
                        let t = d
                            .dom
                            .get_by_ref(r)
                            .map(|i| tags_in(i.properties.get(&key)))
                            .unwrap_or_default();
                        c.set(r, t);
                    }
                    _ => {}
                }
            }
        }
    }
    let c = &mut *c;
    Ok(match c.by_tag.get_mut(tag) {
        Some(e) => {
            // Also drop instances freed before their subtree was re-read.
            let (list, set, _) = e;
            let of = &mut c.of;
            list.retain(|r| {
                let keep = set.contains(r) && d.dom.get_by_ref(*r).is_some();
                if !keep && set.remove(r) {
                    of.remove(r);
                }
                keep
            });
            e.2 = false;
            e.0.clone()
        }
        None => Vec::new(),
    })
}

/// Workspace parts for queries, kept up to date from the document's change
/// journal (only changed parts are re-read).
#[derive(Default)]
struct QueryCache {
    doc_id: u64,
    parts: std::rc::Rc<Vec<QPart>>,
    // (Fx hashing: Ref keys are hashed thousands of times a frame here.)
    index: rustc_hash::FxHashMap<Ref, usize>,
    grid: QueryGrid,
    /// Workspace.Terrain's voxels for rays (None = read them again).
    /// With its voxel bounds (studs), which every ray needs (round 20: the
    /// scan over every chunk per ray was 12% of Place B's Play frame).
    terrain: Option<Option<(Ref, std::rc::Rc<crate::terrain::Voxels>, Option<([f32; 3], [f32; 3])>)>>,
    /// The material of the last ray that hit terrain (for its RaycastResult).
    terrain_material: Option<u8>,
    /// Terrain triangles per chunk for rays (dropped with `terrain`).
    terrain_tris: HashMap<[i32; 3], std::rc::Rc<Vec<([Vec3; 3], [u8; 3])>>>,
}

/// A uniform grid over the query parts (by bounding box) so rays and box
/// queries only test parts near them. Parts spanning too many cells sit in
/// `big` and are always tested.
#[derive(Default)]
struct QueryGrid {
    cells: rustc_hash::FxHashMap<[i32; 3], Vec<Ref>>,
    /// Each part's cell range (inclusive).
    of: rustc_hash::FxHashMap<Ref, ([i32; 3], [i32; 3])>,
    big: rustc_hash::FxHashSet<Ref>,
}

impl QueryGrid {
    fn cell_range(lo: Vec3, hi: Vec3) -> ([i32; 3], [i32; 3]) {
        let c = mech::QUERY_GRID_CELL;
        let a = (lo / c).floor();
        let b = (hi / c).floor();
        (
            [a.x as i32, a.y as i32, a.z as i32],
            [b.x as i32, b.y as i32, b.z as i32],
        )
    }

    /// Cells in an inclusive range, in i64: a part far out (cell indices
    /// saturated at i32::MIN / MAX) wrapped the i32 width to a small count,
    /// and `insert` then walked billions of cells, growing `cells` by
    /// ~200 MB/s with the main thread stuck (a live place power-up weapon freeze).
    fn cells_in(a: [i32; 3], b: [i32; 3]) -> i64 {
        (0..3)
            .map(|i| b[i] as i64 - a[i] as i64 + 1)
            .fold(1i64, |n, w| if w <= 0 { 0 } else { n.saturating_mul(w) })
    }

    fn remove(&mut self, r: Ref) {
        self.big.remove(&r);
        if let Some((a, b)) = self.of.remove(&r) {
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        let k = [x, y, z];
                        if let Some(v) = self.cells.get_mut(&k) {
                            v.retain(|x| *x != r);
                            if v.is_empty() {
                                self.cells.remove(&k);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Removes many parts at once: each cell they were in is filtered once
    /// (removing them one by one filtered a shared cell once per part:
    /// quadratic when a 20k-part model left or arrived; round 2, bulk).
    fn remove_many(&mut self, rs: &rustc_hash::FxHashSet<Ref>) {
        let mut hit: rustc_hash::FxHashSet<[i32; 3]> = Default::default();
        for r in rs {
            self.big.remove(r);
            if let Some((a, b)) = self.of.remove(r) {
                for x in a[0]..=b[0] {
                    for y in a[1]..=b[1] {
                        for z in a[2]..=b[2] {
                            hit.insert([x, y, z]);
                        }
                    }
                }
            }
        }
        for k in hit {
            if let Some(v) = self.cells.get_mut(&k) {
                v.retain(|x| !rs.contains(x));
                if v.is_empty() {
                    self.cells.remove(&k);
                }
            }
        }
    }

    /// A part moved or resized: nothing to do while it stays in the same
    /// cells (most moving parts, most frames).
    fn update(&mut self, p: &QPart) {
        if let Some(range) = self.of.get(&p.r) {
            let (lo, hi) = part_aabb(p);
            if lo.is_finite() && hi.is_finite() && Self::cell_range(lo, hi) == *range {
                return;
            }
        }
        self.remove(p.r);
        self.insert(p);
    }

    fn insert(&mut self, p: &QPart) {
        let (lo, hi) = part_aabb(p);
        let (a, b) = Self::cell_range(lo, hi);
        let count = Self::cells_in(a, b);
        if !(lo.is_finite() && hi.is_finite()) || count <= 0 || count > mech::QUERY_GRID_MAX_CELLS as i64 {
            if crate::runtime::memstats_on() && count > 1 << 31 {
                eprintln!("MEMSTATS query part spans {count} cells: pos {:?} size {:?}", p.xf.pos, p.size);
            }
            self.big.insert(p.r);
            return;
        }
        for x in a[0]..=b[0] {
            for y in a[1]..=b[1] {
                for z in a[2]..=b[2] {
                    self.cells.entry([x, y, z]).or_default().push(p.r);
                }
            }
        }
        self.of.insert(p.r, (a, b));
    }

    fn rebuild(&mut self, parts: &[QPart]) {
        *self = QueryGrid::default();
        for p in parts {
            self.insert(p);
        }
    }

    /// Parts whose cells meet a box; None when the box covers too many cells
    /// (the caller scans everything instead).
    fn in_box(&self, lo: Vec3, hi: Vec3) -> Option<rustc_hash::FxHashSet<Ref>> {
        let (a, b) = Self::cell_range(lo, hi);
        let count = Self::cells_in(a, b);
        if count > mech::QUERY_GRID_SCAN_CELLS as i64 || count <= 0 {
            return None;
        }
        let mut out: rustc_hash::FxHashSet<Ref> = self.big.clone();
        for x in a[0]..=b[0] {
            for y in a[1]..=b[1] {
                for z in a[2]..=b[2] {
                    if let Some(v) = self.cells.get(&[x, y, z]) {
                        out.extend(v.iter().copied());
                    }
                }
            }
        }
        Some(out)
    }

    /// `in_box` for rays: the cells' parts without the always-tested big
    /// ones and without removing repeats (a ray testing a part twice gets
    /// the same nearest hit), into a reused buffer.
    fn in_box_into(&self, lo: Vec3, hi: Vec3, out: &mut Vec<Ref>) -> bool {
        let (a, b) = Self::cell_range(lo, hi);
        let count = Self::cells_in(a, b);
        if count > mech::QUERY_GRID_SCAN_CELLS as i64 || count <= 0 {
            return false;
        }
        for x in a[0]..=b[0] {
            for y in a[1]..=b[1] {
                for z in a[2]..=b[2] {
                    if let Some(v) = self.cells.get(&[x, y, z]) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        true
    }
}

/// Where a part's Size box sits: its CFrame, except an off-centre union
/// (GeometryService keeps the main part's CFrame; mesh_bvh::union_offset).
fn geometry_box(d: &Document, r: Ref, class: &str, xf: Xform, size: Vec3) -> Xform {
    let id = if doc::is_a(class, "PartOperation") {
        crate::scene::union_mesh(d, r).map(|m| m.id)
    } else if class == "MeshPart" {
        crate::scene::content_string(d.prop(r, "MeshId"))
    } else {
        None
    };
    match id {
        Some(id) => Xform { pos: xf.pos + xf.rot * (crate::mesh_bvh::union_offset(&id) * size), rot: xf.rot },
        None => xf,
    }
}

fn qpart(d: &Document, r: Ref) -> Option<QPart> {
    let class = d.class_of(r)?;
    if class == "Terrain" || !doc::is_a(&class, "BasePart") {
        return None;
    }
    let ws = d.workspace()?;
    if !d.is_ancestor(ws, r) {
        return None;
    }
    let xf = crate::scene::xform_of(d, r)?;
    let size = crate::scene::vec3_of(d.prop(r, "Size")).unwrap_or(Vec3::ONE);
    let xf = geometry_box(d, r, &class, xf, size);
    let collide = !matches!(d.prop(r, "CanCollide"), Some(Variant::Bool(false)));
    let query = !matches!(d.prop(r, "CanQuery"), Some(Variant::Bool(false)));
    // CollisionFidelity: Default 0, Hull 1, Box 2, PreciseConvexDecomposition 3.
    let mesh = (class == "MeshPart" || doc::is_a(&class, "PartOperation"))
        && crate::scene::enum_u32(d.prop(r, "CollisionFidelity")) != Some(2);
    let shape = crate::query_shapes::kind_of(d, r, &class);
    Some(QPart {
        r,
        xf,
        size,
        collide,
        query,
        mesh,
        shape,
    })
}

/// Builds the world-query cache now (Play start) rather than inside the
/// game frame of the first ray: Place B's 50k parts took 80–90 ms there.
pub fn warm_queries(lua: &Lua) {
    let _ = query_parts(lua);
}

fn query_parts(lua: &Lua) -> mlua::Result<std::rc::Rc<Vec<QPart>>> {
    if lua.app_data_ref::<QueryCache>().is_none() {
        lua.set_app_data(QueryCache::default());
    }
    let mut w = world(lua)?;
    let d = w.dm();
    let marks = d.marks_for(doc::Reader::Query);
    let mut c = lua
        .app_data_mut::<QueryCache>()
        .ok_or_else(|| rt_err("query cache missing"))?;
    let marks = if c.doc_id != d.id {
        doc::Marks::All
    } else {
        marks
    };
    match marks {
        doc::Marks::All => {
            let parts: Vec<QPart> = match d.workspace() {
                Some(ws) => d
                    .dom
                    .descendants_of(ws)
                    .filter_map(|i| qpart(d, i.referent()))
                    .collect(),
                None => Vec::new(),
            };
            c.index = parts.iter().enumerate().map(|(i, p)| (p.r, i)).collect();
            c.grid.rebuild(&parts);
            c.parts = std::rc::Rc::new(parts);
            c.doc_id = d.id;
            c.terrain = None;
        }
        doc::Marks::Some(m) if !m.is_empty() => {
            let mut touched: rustc_hash::FxHashSet<Ref> = Default::default();
            // Property changes of parts already listed: class and place in
            // the tree are as they were, only these values are read again.
            // Which of CFrame 1, Size 2, CanCollide 4, CanQuery 8 changed.
            let mut props: rustc_hash::FxHashMap<Ref, u8> = Default::default();
            for mark in m {
                match mark {
                    doc::Mark::Prop(r, name) => {
                        let bit = match name.as_str() {
                            "CFrame" => 1,
                            "Size" => 2,
                            "CanCollide" => 4,
                            "CanQuery" => 8,
                            _ => 0,
                        };
                        if bit != 0 {
                            *props.entry(r).or_default() |= bit;
                        }
                        if name.as_str() == "SmoothGrid" {
                            c.terrain = None;
                        }
                        // A part's Shape: read the part again in full.
                        if name.as_str() == "Shape" {
                            touched.insert(r);
                        }
                    }
                    doc::Mark::Tree(r) => {
                        if d.class_of(r).is_some_and(|k| k == "Terrain" || k == "Workspace") {
                            c.terrain = None;
                        }
                        touched.extend(d.subtree(r))
                    }
                }
            }
            let QueryCache {
                parts, index, grid, ..
            } = &mut *c;
            let parts = std::rc::Rc::make_mut(parts);
            for (r, bits) in props {
                if touched.contains(&r) {
                    continue;
                }
                let Some(&i) = index.get(&r) else {
                    // Not a query part (yet): checked in full.
                    touched.insert(r);
                    continue;
                };
                // The instance once, names interned once (stored values;
                // Document::prop for defaults).
                thread_local!(static KEYS: [rbx_dom_weak::Ustr; 4] =
                    ["CFrame", "Size", "CanCollide", "CanQuery"].map(rbx_dom_weak::ustr));
                let [k_cf, k_size, k_collide, k_query] = KEYS.with(|k| *k);
                let Some(inst) = d.dom.get_by_ref(r) else {
                    touched.insert(r);
                    continue;
                };
                let get = |k: rbx_dom_weak::Ustr| {
                    inst.properties.get(&k).cloned().or_else(|| d.prop(r, k.as_str()))
                };
                let p = &mut parts[i];
                if bits & 1 != 0 {
                    let Some(Variant::CFrame(cf)) = get(k_cf) else {
                        touched.insert(r);
                        continue;
                    };
                    p.xf = crate::math::Xform::from_cframe(&cf).orthonormalized();
                }
                if bits & 2 != 0 {
                    p.size = crate::scene::vec3_of(get(k_size)).unwrap_or(Vec3::ONE);
                }
                if bits & 4 != 0 {
                    p.collide = !matches!(get(k_collide), Some(Variant::Bool(false)));
                }
                if bits & 8 != 0 {
                    p.query = !matches!(get(k_query), Some(Variant::Bool(false)));
                }
                let p = *p;
                grid.update(&p);
            }
            // Out of the grid all at once, then back in (re-read) one by one.
            if touched.len() > 64 {
                grid.remove_many(&touched);
            }
            let bulk = touched.len() > 64;
            for r in touched {
                if !bulk {
                    grid.remove(r);
                }
                match (qpart(d, r), index.get(&r).copied()) {
                    (Some(p), Some(i)) => {
                        grid.insert(&p);
                        parts[i] = p;
                    }
                    (Some(p), None) => {
                        grid.insert(&p);
                        index.insert(r, parts.len());
                        parts.push(p);
                    }
                    (None, Some(i)) => {
                        parts.swap_remove(i);
                        index.remove(&r);
                        if let Some(moved) = parts.get(i) {
                            index.insert(moved.r, i);
                        }
                    }
                    (None, None) => {}
                }
            }
        }
        _ => {}
    }
    Ok(c.parts.clone())
}

/// Include / exclude filter by instance and descendants.
struct Filter {
    list: HashSet<Ref>,
    include: bool,
    respect_collide: bool,
    /// Collision groups that don't collide with the query's CollisionGroup
    /// (PhysicsService:CollisionGroupSetCollidable): their parts are passed.
    skip_groups: HashSet<String>,
}

impl Filter {
    fn none() -> Filter {
        Filter {
            list: HashSet::new(),
            include: false,
            respect_collide: false,
            skip_groups: HashSet::new(),
        }
    }

    fn passes(&self, d: &Document, r: Ref) -> bool {
        if !self.skip_groups.is_empty() {
            if let Some(Variant::String(g)) = d.prop(r, "CollisionGroup") {
                if self.skip_groups.contains(&g) {
                    return false;
                }
            }
        }
        if self.list.is_empty() {
            return !self.include;
        }
        let mut cur = r;
        let mut inside = false;
        while let Some(i) = d.dom.get_by_ref(cur) {
            if self.list.contains(&cur) {
                inside = true;
                break;
            }
            cur = i.parent();
        }
        inside == self.include
    }
}

/// A query's filter with its RaycastParams / OverlapParams CollisionGroup
/// applied (groups set not to collide with it are skipped).
fn params_filter_groups(lua: &Lua, v: &Value) -> Filter {
    let mut f = params_filter(v);
    let group = match ud_value(v, "CollisionGroup") {
        Value::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_default(),
        _ => String::new(),
    };
    let group = if group.is_empty() {
        "Default".to_string()
    } else {
        group
    };
    if let Ok(w) = world(lua) {
        if let Ok(g) = w.groups.lock() {
            f.skip_groups.extend(g.uncollidable_with(&group));
        }
    }
    f
}

fn params_filter(v: &Value) -> Filter {
    let list: HashSet<Ref> = refs_in(&ud_value(v, "FilterDescendantsInstances"))
        .into_iter()
        .collect();
    let include = opt_ud::<LEnumItem>(&ud_value(v, "FilterType"))
        .is_some_and(|e| matches!(e.name.as_str(), "Include" | "Whitelist"));
    let respect_collide = matches!(ud_value(v, "RespectCanCollide"), Value::Boolean(true));
    Filter {
        list,
        include,
        respect_collide,
        skip_groups: HashSet::new(),
    }
}

/// Closest hit along origin + dir (dir's length is the range): (part, point, normal, distance).
fn cast(
    lua: &Lua,
    origin: Vec3,
    dir: Vec3,
    filter: &Filter,
) -> mlua::Result<Option<(Ref, Vec3, Vec3, f32)>> {
    cast_swept(lua, origin, dir, filter, Vec3::ZERO, None)
}

/// A ray, or with `grow` > 0 a swept shape: each part's box grown by the
/// shape's half extents (a sphere's radius, a block's world half extents),
/// so the centre's ray against it is the sweep (Spherecast / Blockcast /
/// Shapecast). Parts the shape starts inside aren't hit, as in Roblox.
/// The grown box only narrows the parts down: `probe` (the swept shape's
/// query_shapes kind, size and turn) is then swept against each part's true
/// shape, and rays test balls / cylinders / wedges exactly too.
fn cast_swept(
    lua: &Lua,
    origin: Vec3,
    dir: Vec3,
    filter: &Filter,
    grow: Vec3,
    probe: Option<(u8, Vec3, glam::Mat3)>,
) -> mlua::Result<Option<(Ref, Vec3, Vec3, f32)>> {
    let swept = grow != Vec3::ZERO;
    let len = dir.length();
    if len < 1e-6 {
        return Ok(None);
    }
    let ray = crate::math::Ray {
        origin,
        dir: dir / len,
    };
    // (distance, part, the hit's world normal when not a box face, the
    // touching point when not on the ray)
    type Best = Option<(f32, QPart, Option<Vec3>, Option<Vec3>)>;
    let mut best: Best = None;
    let test = |p: &QPart, d: &Document, best: &mut Best| {
        if !p.query || (filter.respect_collide && !p.collide) {
            return;
        }
        // Boxes: entered from outside (a ray starting inside a part doesn't
        // hit it). MeshParts: their mesh's triangles, when the mesh is here.
        let mut normal: Option<Vec3> = None;
        let mut touch: Option<Vec3> = None;
        let t = if swept {
            let Some(t) = ray.enter_obb(&p.xf, p.size + grow * 2.0) else {
                return;
            };
            match probe {
                Some((kind, size, rot)) => {
                    if best.is_some_and(|(bt, ..)| t >= bt) {
                        return;
                    }
                    let start = crate::math::Xform { pos: ray.origin, rot };
                    let Some((t, at, n)) = crate::query_shapes::sweep_part(
                        &start, size, kind, ray.dir, len, &p.xf, p.size, p.shape,
                    ) else {
                        return;
                    };
                    normal = Some(n);
                    touch = Some(at);
                    t
                }
                None => t,
            }
        } else if p.shape != crate::query_shapes::BOX && !p.mesh {
            let Some((t, n)) = crate::query_shapes::ray_part(&p.xf, p.size, p.shape, ray.origin, ray.dir)
            else {
                return;
            };
            normal = Some(n);
            t
        } else if p.mesh {
            let Some(_) = ray.hit_obb(&p.xf, p.size) else {
                return;
            };
            // Borrowed when stored as MeshId; else the canonical lookup
            // (MeshContent and other stored names).
            let owned;
            let id = match d.dom.get_by_ref(p.r).and_then(|i| i.properties.get(&ustr("MeshId"))) {
                Some(Variant::ContentId(c)) => c.as_str(),
                _ => {
                    owned = match d.prop(p.r, "MeshId") {
                        Some(Variant::ContentId(c)) => c.as_str().to_string(),
                        // A union: its mesh (geometry_box already centred p.xf on it).
                        _ => crate::scene::union_mesh(d, p.r).map(|m| m.id).unwrap_or_default(),
                    };
                    owned.as_str()
                }
            };
            let _m = crate::bench::scope("ray_mesh");
            match (!id.is_empty())
                .then(|| crate::mesh_bvh::for_mesh(id))
                .flatten()
            {
                Some(bvh) => {
                    crate::bench::add("ray_mesh_tris", 0.0, bvh.tris.len() as u64);
                    let inv = p.xf.rot.transpose();
                    let size = p.size.max(Vec3::splat(1e-4));
                    let o = (inv * (ray.origin - p.xf.pos)) / size;
                    let dl = (inv * ray.dir) / size;
                    let Some((t, n)) = bvh.ray_normal(o, dl) else {
                        return;
                    };
                    // Unit-mesh normal to world: inverse-transpose of the scale.
                    normal = Some((p.xf.rot * (n / size)).normalize_or_zero());
                    t
                }
                None => {
                    let Some(t) = ray.enter_obb(&p.xf, p.size) else {
                        return;
                    };
                    t
                }
            }
        } else {
            let Some(t) = ray.enter_obb(&p.xf, p.size) else {
                return;
            };
            t
        };
        if t < 0.0 || t > len || best.is_some_and(|(bt, ..)| t >= bt) {
            return;
        }
        if filter.passes(d, p.r) {
            *best = Some((t, *p, normal, touch));
        }
    };
    // Along the ray a few grid cells at a time; stop once a hit is nearer
    // than the next stretch.
    let parts_scope = crate::bench::scope("ray_parts");
    {
        let _q = crate::bench::scope("ray_query_sync");
        query_parts(lua)?;
    }
    let step = mech::QUERY_GRID_CELL * 4.0;
    let mut t0 = 0.0f32;
    let mut scanned_all = false;
    let mut refs: Vec<Ref> = Vec::new();
    let mut big_done = false;
    while t0 < len && !scanned_all {
        let t1 = (t0 + step).min(len);
        let (a, b) = (ray.at(t0), ray.at(t1));
        let w = world(lua)?;
        let d = w.d();
        let c = lua.app_data_ref::<QueryCache>();
        refs.clear();
        let gridded = c.as_ref().is_some_and(|c| c.grid.in_box_into(a.min(b) - grow, a.max(b) + grow, &mut refs));
        match (gridded, c.as_ref()) {
            (true, Some(c)) => {
                // Parts too big for the grid: once per ray.
                if !big_done {
                    big_done = true;
                    refs.extend(c.grid.big.iter().copied());
                }
                for r in &refs {
                    if let Some(p) = c.index.get(r).and_then(|i| c.parts.get(*i)) {
                        test(p, d, &mut best);
                    }
                }
            }
            _ => {
                drop(c);
                let parts = query_parts_cached(lua)?;
                for p in parts.iter() {
                    test(p, d, &mut best);
                }
                scanned_all = true;
            }
        }
        if best.is_some_and(|(bt, ..)| bt <= t1) {
            break;
        }
        t0 = t1;
    }
    // Terrain (rays only; sweeps still see parts alone).
    if !swept {
        drop(parts_scope);
        let _t = crate::bench::scope("ray_terrain");
        let terrain = terrain_hit(lua, ray.origin, ray.dir, len)?;
        let passes = terrain.as_ref().is_some_and(|(r, ..)| {
            world(lua).is_ok_and(|w| filter.passes(w.d(), *r))
        });
        if let Some(mut c) = lua.app_data_mut::<QueryCache>() {
            c.terrain_material = None;
        }
        if let (true, Some((r, point, n, t, m))) = (passes, terrain) {
            if best.as_ref().is_none_or(|(bt, ..)| t < *bt) {
                if let Some(mut c) = lua.app_data_mut::<QueryCache>() {
                    c.terrain_material = Some(m);
                }
                return Ok(Some((r, point, n, t)));
            }
        }
    }
    Ok(best.map(|(t, p, shape_normal, touch)| {
        let point = ray.at(t);
        if let Some(n) = shape_normal {
            return (p.r, touch.unwrap_or(point), n, t);
        }
        // Normal of the box face that was hit (of the grown box for a sweep).
        let local = p.xf.rot.transpose() * (point - p.xf.pos);
        let rel = local / ((p.size + grow * 2.0) * 0.5).max(Vec3::splat(1e-4));
        let a = rel.abs();
        let n = if a.x >= a.y && a.x >= a.z {
            Vec3::X * rel.x.signum()
        } else if a.y >= a.z {
            Vec3::Y * rel.y.signum()
        } else {
            Vec3::Z * rel.z.signum()
        };
        let n = p.xf.rot * n;
        // A sweep reports where the shape touches the part: back from the
        // centre by the shape's extent along the normal.
        let touch = if swept {
            point - n * (n.abs().dot(grow))
        } else {
            point
        };
        (p.r, touch, n, t)
    }))
}

/// Parts a query has to look at: with an include list just the parts in it.
fn candidates(lua: &Lua, filter: &Filter) -> mlua::Result<std::rc::Rc<Vec<QPart>>> {
    if filter.include {
        // The listed parts as the (current) query cache holds them: every
        // BasePart under Workspace is in it.
        let all = query_parts(lua)?;
        let w = world(lua)?;
        let d = w.d();
        let c = lua.app_data_ref::<QueryCache>();
        let mut out = Vec::new();
        for root in &filter.list {
            for i in d.dom.descendants_of(*root) {
                let r = i.referent();
                match c.as_ref().and_then(|c| c.index.get(&r)).and_then(|k| all.get(*k)) {
                    Some(p) if p.r == r => out.push(*p),
                    _ => {
                        if let Some(p) = qpart(d, r) {
                            out.push(p);
                        }
                    }
                }
            }
        }
        return Ok(std::rc::Rc::new(out));
    }
    query_parts(lua)
}

fn part_aabb(p: &QPart) -> (Vec3, Vec3) {
    let h = p.size * 0.5;
    let r = p.xf.rot;
    let ext = r.x_axis.abs() * h.x + r.y_axis.abs() * h.y + r.z_axis.abs() * h.z;
    (p.xf.pos - ext, p.xf.pos + ext)
}

/// The query parts near a box (from the grid), or None when the box is too
/// big for the grid to help. The cache must be current (query_parts).
fn near_parts(lua: &Lua, lo: Vec3, hi: Vec3) -> mlua::Result<Option<Vec<QPart>>> {
    let Some(c) = lua.app_data_ref::<QueryCache>() else {
        return Ok(None);
    };
    let Some(refs) = c.grid.in_box(lo, hi) else {
        return Ok(None);
    };
    Ok(Some(
        refs.iter()
            .filter_map(|r| c.index.get(r).and_then(|i| c.parts.get(*i)).copied())
            .collect(),
    ))
}

/// The query parts as last updated (no journal read).
fn query_parts_cached(lua: &Lua) -> mlua::Result<std::rc::Rc<Vec<QPart>>> {
    Ok(lua
        .app_data_ref::<QueryCache>()
        .map(|c| c.parts.clone())
        .unwrap_or_default())
}

/// Parts whose bounds overlap an oriented box (as an AABB) or a sphere.
fn overlap(
    lua: &Lua,
    lo: Vec3,
    hi: Vec3,
    sphere: Option<(Vec3, f32)>,
    filter: &Filter,
    max: usize,
) -> mlua::Result<Vec<Ref>> {
    query_parts(lua)?;
    let parts = match near_parts(lua, lo, hi)? {
        Some(list) => std::rc::Rc::new(list),
        None => candidates(lua, filter)?,
    };
    let w = world(lua)?;
    let d = w.d();
    let mut out = Vec::new();
    for p in parts.iter() {
        if !p.query {
            continue;
        }
        let (plo, phi) = part_aabb(p);
        let hit = match sphere {
            Some((c, r)) => (c.clamp(plo, phi) - c).length() <= r,
            None => plo.cmple(hi).all() && phi.cmpge(lo).all(),
        };
        if hit && filter.passes(d, p.r) {
            out.push(p.r);
            if out.len() >= max {
                break;
            }
        }
    }
    Ok(out)
}

/// Terrain under a ray (WorldRoot:Raycast and the legacy FindPartOnRay
/// family): the surface where the voxels' occupancy, blended between voxel
/// centres, crosses one half (the mesher's surface), found in half-stud
/// steps and refined. Water isn't hit. Returns (terrain, point, normal,
/// distance, SmoothGrid material). Round 19: rays never saw terrain.
fn terrain_hit(
    lua: &Lua,
    origin: Vec3,
    dir: Vec3,
    len: f32,
) -> mlua::Result<Option<(Ref, Vec3, Vec3, f32, u8)>> {
    use crate::terrain::VOXEL;
    let cached = {
        let c = lua.app_data_ref::<QueryCache>();
        c.and_then(|c| c.terrain.clone())
    };
    let terrain = match cached {
        Some(t) => t,
        None => {
            let t = {
                let w = world(lua)?;
                let d = w.d();
                let r = d.workspace().and_then(|ws| d.find_child(ws, "Terrain"));
                match (r, crate::terrain::Voxels::from_dom(&d.dom)) {
                    (Some(r), Some(Ok(v))) if v.count() > 0 => {
                        let b = v.world_bounds();
                        Some((r, std::rc::Rc::new(v), b))
                    }
                    _ => None,
                }
            };
            if let Some(mut c) = lua.app_data_mut::<QueryCache>() {
                c.terrain = Some(t.clone());
                c.terrain_tris.clear();
            }
            t
        }
    };
    let Some((r, v, bounds)) = terrain else {
        return Ok(None);
    };
    let Some((lo, hi)) = bounds else {
        return Ok(None);
    };
    let (lo, hi) = (Vec3::from(lo) - VOXEL, Vec3::from(hi) + VOXEL);
    // The stretch of the ray inside the terrain's bounds.
    let (mut t0, mut t1) = (0.0f32, len);
    for k in 0..3 {
        if dir[k].abs() < 1e-9 {
            if origin[k] < lo[k] || origin[k] > hi[k] {
                return Ok(None);
            }
            continue;
        }
        let (a, b) = ((lo[k] - origin[k]) / dir[k], (hi[k] - origin[k]) / dir[k]);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    if t0 > t1 {
        return Ok(None);
    }
    // The drawn (and collided) surface itself: each chunk's triangles, made
    // once per terrain change, nearest front face along the ray (round 21;
    // before, rays marched a separate trilinear field).
    let dir_n = dir.normalize_or_zero();
    let mut best: Option<(f32, Vec3, u8)> = None;
    let chunk = crate::terrain::VOXEL * 32.0;
    let mut seen: HashSet<[i32; 3]> = HashSet::new();
    let mut t = t0;
    let step = chunk * 0.5;
    loop {
        let seg_end = (t + step).min(t1);
        let (a, b) = (origin + dir_n * t, origin + dir_n * seg_end);
        // A chunk's surface reaches one voxel past its corner.
        let lo = ((a.min(b) - VOXEL) / chunk).floor().as_ivec3();
        let hi = ((a.max(b) + VOXEL) / chunk).floor().as_ivec3();
        for x in lo.x..=hi.x {
            for y in lo.y..=hi.y {
                for z in lo.z..=hi.z {
                    let key = [x, y, z];
                    if !seen.insert(key) {
                        continue;
                    }
                    let tris = terrain_chunk_tris(lua, &v, key);
                    for tri in tris.iter() {
                        if let Some((d, w)) = ray_triangle(origin, dir_n, &tri.0) {
                            if d >= t0 && d <= t1 && best.is_none_or(|(bt, _, _)| d < bt) {
                                let n = (tri.0[1] - tri.0[0]).cross(tri.0[2] - tri.0[0]).normalize_or_zero();
                                let k = if w.x >= w.y && w.x >= w.z { 0 } else if w.y >= w.z { 1 } else { 2 };
                                best = Some((d, n, tri.1[k]));
                            }
                        }
                    }
                }
            }
        }
        if best.is_some_and(|(bt, _, _)| bt <= seg_end) || seg_end >= t1 {
            break;
        }
        t = seg_end;
    }
    Ok(best.map(|(d, n, m)| (r, origin + dir_n * d, n, d, m)))
}

/// One chunk's terrain triangles (positions, corner materials) for rays,
/// cached until the terrain changes.
fn terrain_chunk_tris(
    lua: &Lua,
    v: &crate::terrain::Voxels,
    key: [i32; 3],
) -> std::rc::Rc<Vec<([Vec3; 3], [u8; 3])>> {
    if let Some(t) = lua.app_data_ref::<QueryCache>().and_then(|c| c.terrain_tris.get(&key).cloned()) {
        return t;
    }
    let verts = crate::terrain::chunk_triangles(v, key, &|_| false);
    let tris: Vec<([Vec3; 3], [u8; 3])> = verts
        .chunks_exact(3)
        .map(|t| {
            let mats = [0, 1, 2].map(|i| t[0].palette[i] / 3);
            ([Vec3::from(t[0].pos), Vec3::from(t[1].pos), Vec3::from(t[2].pos)], mats)
        })
        .collect();
    let tris = std::rc::Rc::new(tris);
    if let Some(mut c) = lua.app_data_mut::<QueryCache>() {
        c.terrain_tris.insert(key, tris.clone());
    }
    tris
}

/// Ray / triangle (Möller–Trumbore), front faces only (a ray starting
/// inside terrain doesn't hit it on the way out, as in Roblox): distance and
/// barycentric weights of the corners.
fn ray_triangle(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<(f32, Vec3)> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    // Counter-clockwise from outside: the ray must come at the front.
    if det < 1e-8 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let w = d.dot(q) * inv;
    if w < 0.0 || u + w > 1.0 {
        return None;
    }
    let dist = e2.dot(q) * inv;
    (dist >= 0.0).then_some((dist, Vec3::new(1.0 - u - w, u, w)))
}

fn raycast_result(lua: &Lua, hit: (Ref, Vec3, Vec3, f32)) -> mlua::Result<Value> {
    let terrain_mat = lua
        .app_data_ref::<QueryCache>()
        .and_then(|c| c.terrain_material)
        .filter(|_| world(lua).is_ok_and(|w| w.d().class_of(hit.0).is_some_and(|k| k == "Terrain")));
    let (material, name) = {
        let w = world(lua)?;
        let d = w.d();
        let material = match terrain_mat {
            Some(m) => LEnumItem::from_name("Material", crate::terrain::material_name(m)),
            None => match d.prop(hit.0, "Material") {
                Some(Variant::Enum(e)) => LEnumItem::from_value("Material", e.to_u32()),
                _ => LEnumItem::from_name("Material", "Plastic"),
            },
        };
        let name = d.dom.get_by_ref(hit.0).map(|i| i.name.clone()).unwrap_or_default();
        (material, name)
    };
    let r = crate::ltypes::LRaycastResult {
        instance: inst_value(lua, hit.0)?,
        name,
        position: hit.1,
        normal: hit.2,
        distance: hit.3,
        material,
    };
    Ok(Value::UserData(lua.create_userdata(r)?))
}

/// Region3 → its world AABB.
fn region_box(v: &Value) -> Option<(Vec3, Vec3)> {
    let cf = ud_field::<LCFrame>(v, "CFrame")?;
    let size = ud_field::<LVector3>(v, "Size")?;
    let h = size.0 * 0.5;
    Some((cf.0.pos - h, cf.0.pos + h))
}

// -------------------------------------------------------------------- tags

fn tags_of(lua: &Lua, r: Ref) -> mlua::Result<Vec<String>> {
    Ok(
        match world(lua)?
            .d()
            .dom
            .get_by_ref(r)
            .and_then(|i| i.properties.get(&ustr("Tags")).cloned())
        {
            Some(Variant::Tags(t)) => t.iter().map(|s| s.to_string()).collect(),
            Some(Variant::BinaryString(b)) => Tags::decode(b.as_ref())
                .map(|t| t.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        },
    )
}

fn set_tags(lua: &Lua, r: Ref, tags: Vec<String>) -> mlua::Result<()> {
    let mut w = world(lua)?;
    if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
        i.properties
            .insert(ustr("Tags"), Variant::Tags(Tags::from(tags)));
    }
    w.dm().mark_prop(r, ustr("Tags"));
    Ok(())
}

fn in_data_model(lua: &Lua, r: Ref) -> mlua::Result<bool> {
    let w = world(lua)?;
    let d = w.d();
    let root = d.dom.root_ref();
    Ok(r == root || d.is_ancestor(root, r))
}

fn add_tag(lua: &Lua, r: Ref, tag: &str) -> mlua::Result<()> {
    let mut tags = tags_of(lua, r)?;
    if tags.iter().any(|t| t == tag) {
        return Ok(());
    }
    tags.push(tag.to_string());
    set_tags(lua, r, tags)?;
    // InstanceAdded only fires for instances in the DataModel; one tagged
    // while out of it fires when it is parented in (set_parent).
    if !in_data_model(lua, r)? {
        return Ok(());
    }
    let cs = get_service(lua, "CollectionService", true)?;
    if let Some(cs) = cs {
        let v = inst_value(lua, r)?;
        world(lua)?.fire(cs, &format!("TagAdded:{tag}"), vec![v]);
    }
    Ok(())
}

fn remove_tag(lua: &Lua, r: Ref, tag: &str) -> mlua::Result<()> {
    let tags = tags_of(lua, r)?;
    if !tags.iter().any(|t| t == tag) {
        return Ok(());
    }
    set_tags(lua, r, tags.into_iter().filter(|t| t != tag).collect())?;
    if !in_data_model(lua, r)? {
        return Ok(());
    }
    if let Some(cs) = get_service(lua, "CollectionService", true)? {
        let v = inst_value(lua, r)?;
        world(lua)?.fire(cs, &format!("TagRemoved:{tag}"), vec![v]);
    }
    Ok(())
}

// --------------------------------------------------------------------- json

fn to_json(v: &Value, depth: usize) -> mlua::Result<serde_json::Value> {
    if depth > 64 {
        return Err(rt_err("Can't convert to JSON: nesting too deep"));
    }
    Ok(match v {
        Value::Nil => serde_json::Value::Null,
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Integer(i) => serde_json::json!(*i),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                serde_json::json!(*n as i64)
            } else {
                serde_json::json!(*n)
            }
        }
        Value::String(s) => serde_json::Value::String(s.to_str()?.to_string()),
        Value::Table(t) => {
            let n = t.raw_len();
            if n > 0 {
                let mut arr = Vec::with_capacity(n);
                for i in 1..=n {
                    arr.push(to_json(&t.raw_get::<Value>(i)?, depth + 1)?);
                }
                serde_json::Value::Array(arr)
            } else {
                let mut map = serde_json::Map::new();
                for pair in t.pairs::<Value, Value>() {
                    let (k, v) = pair?;
                    let key = match &k {
                        Value::String(s) => s.to_str()?.to_string(),
                        Value::Number(n) => n.to_string(),
                        Value::Integer(i) => i.to_string(),
                        _ => continue,
                    };
                    map.insert(key, to_json(&v, depth + 1)?);
                }
                // An empty table is "[]" in Roblox's JSONEncode.
                if map.is_empty() {
                    serde_json::Value::Array(Vec::new())
                } else {
                    serde_json::Value::Object(map)
                }
            }
        }
        _ => {
            return Err(rt_err(format!(
                "Can't convert to JSON: {} is not supported",
                v.type_name()
            )))
        }
    })
}

fn from_json(lua: &Lua, v: &serde_json::Value) -> mlua::Result<Value> {
    Ok(match v {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::String(lua.create_string(s)?),
        serde_json::Value::Array(a) => {
            let t = lua.create_table()?;
            for (i, x) in a.iter().enumerate() {
                t.raw_set(i + 1, from_json(lua, x)?)?;
            }
            Value::Table(t)
        }
        serde_json::Value::Object(m) => {
            let t = lua.create_table()?;
            for (k, x) in m {
                t.raw_set(k.as_str(), from_json(lua, x)?)?;
            }
            Value::Table(t)
        }
    })
}

fn guid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for (i, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_usize(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

// ------------------------------------------------------------------ helpers

fn joint_class(class: &str) -> bool {
    matches!(
        class,
        "Weld"
            | "ManualWeld"
            | "Snap"
            | "Glue"
            | "ManualGlue"
            | "Motor"
            | "Motor6D"
            | "WeldConstraint"
            | "VelocityMotor"
            | "Rotate"
            | "RotateP"
            | "RotateV"
    )
}

/// BreakJoints: destroys rigid joints inside `r` and those touching its parts.
fn break_joints(lua: &Lua, r: Ref) -> mlua::Result<()> {
    let joints: Vec<Ref> = {
        let w = world(lua)?;
        let d = w.d();
        let parts: HashSet<Ref> = d
            .dom
            .descendants_of(r)
            .filter(|i| doc::is_a(&i.class, "BasePart"))
            .map(|i| i.referent())
            .collect();
        let mut js: Vec<Ref> = d
            .dom
            .descendants_of(r)
            .filter(|i| joint_class(&i.class))
            .map(|i| i.referent())
            .collect();
        // Joints elsewhere that hold one of these parts.
        if let Some(ws) = d.workspace() {
            for i in d.dom.descendants_of(ws) {
                if joint_class(&i.class) && !js.contains(&i.referent()) {
                    let touches = ["Part0", "Part1"].iter().any(|p| matches!(i.properties.get(&ustr(p)), Some(Variant::Ref(x)) if parts.contains(x)));
                    if touches {
                        js.push(i.referent());
                    }
                }
            }
        }
        js
    };
    for j in joints {
        destroy_inst(lua, j)?;
    }
    Ok(())
}

fn model_bounds(lua: &Lua, r: Ref) -> mlua::Result<(Vec3, Vec3)> {
    let w = world(lua)?;
    let d = w.d();
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for i in d.dom.descendants_of(r) {
        if !doc::is_a(&i.class, "BasePart") {
            continue;
        }
        let Some(xf) = crate::scene::xform_of(d, i.referent()) else {
            continue;
        };
        let size = crate::scene::vec3_of(d.prop(i.referent(), "Size")).unwrap_or(Vec3::ONE);
        let xf = geometry_box(d, i.referent(), &i.class, xf, size);
        let (a, b) = part_aabb(&QPart {
            r: i.referent(),
            xf,
            size,
            collide: true,
            query: true,
            mesh: false,
            shape: 0,
        });
        lo = lo.min(a);
        hi = hi.max(b);
    }
    if !lo.is_finite() {
        return Ok((Vec3::ZERO, Vec3::ZERO));
    }
    Ok((lo, hi))
}

fn character_root(d: &Document, player: Ref) -> Option<Vec3> {
    let ch = match d.prop(player, "Character") {
        Some(Variant::Ref(c)) if c.is_some() => c,
        _ => return None,
    };
    let root = d
        .find_named(ch, "HumanoidRootPart")
        .or_else(|| d.find_named(ch, "Torso"))?;
    crate::scene::xform_of(d, root).map(|x| x.pos)
}

fn sound_key_of(r: Ref) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    r.hash(&mut h);
    h.finish() % 1_000_000_000
}

/// A parentless instance of any class (Mouse, Tween, AnimationTrack…).
pub fn create_any(lua: &Lua, class: &str) -> mlua::Result<Ref> {
    let mut w = world(lua)?;
    let r=w.dm().dom.insert(Ref::none(), InstanceBuilder::new(class).with_name(class));
    drop(w);
    super::domains::assets_cloud::scene::track_instance(lua,r)?;
    Ok(r)
}

/// Sound playback requests (also used when a script sets Sound.Playing).
pub fn sound(lua: &Lua, r: Ref, cmd: SoundCmd) -> mlua::Result<()> {
    let playing = matches!(cmd, SoundCmd::Play | SoundCmd::Resume);
    {
        let mut w = world(lua)?;
        if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
            i.properties.insert(ustr("Playing"), Variant::Bool(playing));
        }
        w.dm().mark_prop(r, ustr("Playing"));
        w.record(RepOp::Prop(r, "Playing".into()));
        w.requests.sounds.push((r, cmd));
        if playing {
            w.input.sound_playing.insert(r);
        } else {
            w.input.sound_playing.remove(&r);
        }
        if cmd == SoundCmd::Pause {
            w.input.sound_paused.insert(r);
        } else {
            w.input.sound_paused.remove(&r);
        }
    }
    let ev = match cmd {
        SoundCmd::Play => "Played",
        SoundCmd::Stop => "Stopped",
        SoundCmd::Pause => "Paused",
        SoundCmd::Resume => "Resumed",
    };
    let sid = match world(lua)?.d().prop(r, "SoundId") {
        Some(Variant::ContentId(c)) => c.as_str().to_string(),
        Some(Variant::Content(c)) => c.as_uri().unwrap_or("").to_string(),
        _ => String::new(),
    };
    let s = Value::String(lua.create_string(&sid)?);
    world(lua)?.fire(r, ev, vec![s]);
    Ok(())
}

/// InsertService:LoadAsset: a Model holding the asset's instances, from the
/// local asset store (the offline Studio's ~/.local/share/rustudio/assets) or the asset cache.
fn load_asset(lua: &Lua, id: u64) -> mlua::Result<Ref> {
    let path = crate::assets::local_asset_path(id);
    if let Some(bytes) = path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .filter(|b| crate::fbx::is_fbx(b))
    {
        return load_fbx(lua, id, &bytes);
    }
    // Signed in, a model not on disk is downloaded (LoadAsset yields in
    // Roblox; here it waits for the download), once per id per run.
    // A session check still running at Play start is waited for (a server
    // script's LoadAsset in its first frame), then the model is downloaded
    // with the session; Roblox's refusal is kept for the message.
    static FAILED: std::sync::Mutex<Option<HashMap<u64, String>>> = std::sync::Mutex::new(None);
    let mut why = String::from("signed out");
    let mut fetched = || -> Option<Vec<u8>> {
        if !crate::assets::wait_for_session(std::time::Duration::from_secs(20)) {
            return None;
        }
        let mut f = FAILED.lock().ok()?;
        let map = f.get_or_insert_with(Default::default);
        if let Some(e) = map.get(&id) {
            why = e.clone();
            return None;
        }
        match crate::assets::remote_cached(id) {
            Ok(b) => Some((*b).clone()),
            Err(e) => {
                why = e.clone();
                map.insert(id, e);
                None
            }
        }
    };
    // The local store / upload ledger may hold an upload's *source* (a
    // decal's .png): only a model counts, else Roblox's own asset (for a
    // decal, the Model holding its Decal) is fetched.
    let is_model = |b: &Vec<u8>| b.starts_with(b"<roblox") || b.starts_with(b"<?xml");
    let bytes = path
        .and_then(|p| std::fs::read(p).ok())
        .filter(is_model)
        .or_else(|| std::fs::read(crate::assets::cache_dir().join(format!("assets/{id}_vlatest.bin"))).ok().filter(is_model))
        .or_else(|| crate::runtime::outside_budget(lua, &mut fetched))
        .filter(|b| {
            let ok = is_model(b);
            if !ok {
                why = "Roblox's copy isn't a model".into();
            }
            ok
        })
        .ok_or_else(|| rt_err(format!("Asset {id} isn't available: not in the local asset store, upload ledger or cache, and Roblox refused it to this session ({why})")))?;
    let mut tmp = Document::decode(&bytes)
        .map_err(|e| rt_err(format!("Asset {id} couldn't be read: {e}")))?;
    let mut w = world(lua)?;
    let model = w.dm().dom.insert(
        Ref::none(),
        InstanceBuilder::new("Model").with_name("Model"),
    );
    for c in tmp.root().children().to_vec() {
        tmp.transfer(c, &mut w.dm().dom, model);
    }
    Ok(model)
}

/// An FBX upload, imported the way Roblox imports FBX models (see fbx.rs):
/// a Model of MeshParts (empty nodes become Models), each re-centred on its
/// own bounds. Meshes and embedded textures get `rbxtemp://rustudio/fbx/…`
/// ids, so every copy of the parts (clones, the client's) draws them.
pub fn load_fbx(lua: &Lua, id: u64, bytes: &[u8]) -> mlua::Result<Ref> {
    use rbx_dom_weak::types::{CFrame, Color3, Matrix3};
    // Parse once per asset; later loads reuse the meshes.
    static IMPORTED: std::sync::Mutex<
        Option<HashMap<u64, std::sync::Arc<Vec<crate::fbx::ImportNode>>>>,
    > = std::sync::Mutex::new(None);
    let nodes = {
        let mut g = IMPORTED.lock().map_err(|_| rt_err("FBX import busy"))?;
        let map = g.get_or_insert_with(HashMap::new);
        match map.get(&id) {
            Some(n) => n.clone(),
            None => {
                let n =
                    std::sync::Arc::new(crate::fbx::import(bytes).map_err(|e| {
                        rt_err(format!("Asset {id} (FBX) couldn't be imported: {e}"))
                    })?);
                for (i, node) in n.iter().enumerate() {
                    if let Some(g) = &node.mesh {
                        crate::assets::register_generated(
                            &format!("rbxtemp://rustudio/fbx/{id}/{i}"),
                            crate::fbx::mesh_v2(g),
                        );
                    }
                    if let Some((_, tex)) = &node.texture {
                        crate::assets::register_generated(
                            &format!("rbxtemp://rustudio/fbx/{id}/{i}.tex"),
                            tex.clone(),
                        );
                    }
                }
                map.insert(id, n.clone());
                n
            }
        }
    };
    let mut w = world(lua)?;
    let dom = &mut w.dm().dom;
    let model = dom.insert(
        Ref::none(),
        InstanceBuilder::new("Model").with_name("Model"),
    );
    let mut made: Vec<Ref> = Vec::with_capacity(nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        let parent = n.parent.and_then(|p| made.get(p).copied()).unwrap_or(model);
        let r = match &n.mesh {
            Some(g) => {
                let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for p in &g.positions {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
                let size = (hi - lo).max(Vec3::splat(0.001));
                let centre = (hi + lo) * 0.5;
                let color = n
                    .color
                    .unwrap_or([163.0 / 255.0, 162.0 / 255.0, 165.0 / 255.0]);
                let mut b = InstanceBuilder::new("MeshPart")
                    .with_name(n.name.clone())
                    .with_property(
                        "MeshId",
                        Variant::ContentId(format!("rbxtemp://rustudio/fbx/{id}/{i}").into()),
                    )
                    .with_property("Size", Vector3::new(size.x, size.y, size.z))
                    .with_property("InitialSize", Vector3::new(size.x, size.y, size.z))
                    .with_property("MeshSize", Vector3::new(size.x, size.y, size.z))
                    .with_property(
                        "CFrame",
                        CFrame::new(
                            Vector3::new(centre.x, centre.y, centre.z),
                            Matrix3::identity(),
                        ),
                    )
                    .with_property("Color", Color3::new(color[0], color[1], color[2]))
                    .with_property("Anchored", true);
                if n.texture.is_some() {
                    b = b.with_property(
                        "TextureID",
                        Variant::ContentId(format!("rbxtemp://rustudio/fbx/{id}/{i}.tex").into()),
                    );
                }
                dom.insert(parent, b)
            }
            None => dom.insert(
                parent,
                InstanceBuilder::new("Model").with_name(n.name.clone()),
            ),
        };
        made.push(r);
    }
    Ok(model)
}

/// A GUI object's absolute position and size (simple UDim2 layout from the
/// screen down; UIListLayout and friends aren't applied).
/// A GuiBase2d's AbsolutePosition / AbsoluteSize / AbsoluteRotation.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AbsBox {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub rot: f32,
}

/// As drawn last frame; before that (or when hidden), from the UDim2s.
fn abs_now(w: &World, r: Ref) -> AbsBox {
    match w.input.gui_layout.get(&r) {
        Some(b) => AbsBox { pos: b.pos, size: b.size, rot: b.rot },
        None => {
            let (pos, size) = gui_rect(w, r, 0);
            AbsBox { pos, size, rot: 0.0 }
        }
    }
}

/// Fires Changed / GetPropertyChangedSignal for AbsolutePosition,
/// AbsoluteSize and AbsoluteRotation of every listened GuiBase2d whose box
/// moved since scripts last saw it, and for Camera.ViewportSize, after a new
/// GUI layout or view size. Roblox fires these as the screen resizes; layouts
/// that fit themselves to the screen (Place A's 720p HUD canvas: a UIScale set
/// from ScreenGui.AbsoluteSize) stayed at the size the window had when they
/// were made.
pub fn fire_layout_changes(lua: &Lua) -> mlua::Result<()> {
    let mut fires: Vec<(Ref, &'static str)> = Vec::new();
    {
        let mut w = world(lua)?;
        let view = w.input.view_size;
        let mut all = false;
        if view[0] > 0.0 && w.viewport_seen != Some(view) {
            let had = w.viewport_seen.replace(view).is_some();
            let d = w.d();
            let cam = d.workspace().and_then(|ws| match d.prop(ws, "CurrentCamera") {
                Some(Variant::Ref(c)) if d.exists(c) => Some(c),
                _ => None,
            });
            if let (true, Some(c)) = (had, cam) {
                fires.push((c, "ViewportSize"));
            }
            // Every box scaled from the screen may have moved.
            all = true;
        }
        // Only what can have moved is looked at (Studio recomputes a GUI
        // object's box when its layout is invalidated, not every frame): the
        // objects drawn this frame or last (their box is the drawn one),
        // those newly listened to, and for the rest (hidden GUIs, whose box
        // comes from the UDim2s in `gui_rect`) the subtrees whose Size /
        // Position / AnchorPoint / canvas or place in the tree changed, from
        // the document's journal. Checking every listened instance each
        // frame was 2.5 ms in Place E (6 800 listened, 2 250 of
        // them hidden GUI objects, each a walk up to its ScreenGui).
        let layout_props = LAYOUT_PROPS.get_or_init(|| {
            ["Size", "Position", "AnchorPoint", "IgnoreGuiInset", "CanvasSize"].map(rbx_dom_weak::ustr)
        });
        let mut check: Vec<Ref> = Vec::new();
        match w.dm().marks_for(doc::Reader::Layout) {
            doc::Marks::All => all = true,
            doc::Marks::Some(marks) => {
                let mut roots: Vec<Ref> = Vec::new();
                for m in marks {
                    match m {
                        doc::Mark::Prop(r, name) if layout_props.contains(&name) => roots.push(r),
                        doc::Mark::Tree(r) => roots.push(r),
                        _ => {}
                    }
                }
                let mut seen_root: HashSet<Ref> = HashSet::new();
                roots.retain(|r| seen_root.insert(*r));
                let w = &*w;
                let d = w.d();
                for root in roots {
                    if d.dom.get_by_ref(root).is_none() {
                        continue;
                    }
                    check.extend(
                        d.dom
                            .descendants_of(root)
                            .map(|i| i.referent())
                            .filter(|r| w.listening.contains(r)),
                    );
                }
            }
        }
        let fresh = std::mem::take(&mut w.listen_new);
        if all {
            check = w.listening.iter().copied().collect();
        } else {
            check.extend(fresh);
            check.extend(w.input.gui_layout.keys().copied().filter(|r| w.listening.contains(r)));
            // Drawn last frame, not this one: back to its UDim2 box.
            check.extend(w.abs_laid.iter().copied().filter(|r| !w.input.gui_layout.contains_key(r)));
            let mut seen: HashSet<Ref> = HashSet::with_capacity(check.len());
            check.retain(|r| seen.insert(*r));
        }
        let listened: Vec<Ref> = check
            .into_iter()
            .filter(|r| w.listening.contains(r))
            .filter(|r| w.d().class_of(*r).is_some_and(|c| doc::is_a(&c, "GuiBase2d")))
            .collect();
        let laid: HashSet<Ref> = {
            let w = &*w;
            if all {
                listened.iter().copied().filter(|r| w.input.gui_layout.contains_key(r)).collect()
            } else {
                w.input.gui_layout.keys().copied().filter(|r| w.listening.contains(r)).collect()
            }
        };
        w.abs_laid = laid;
        for r in listened {
            let now = abs_now(&w, r);
            match w.abs_seen.insert(r, now) {
                Some(old) if old != now => {
                    if old.pos != now.pos {
                        fires.push((r, "AbsolutePosition"));
                    }
                    if old.size != now.size {
                        fires.push((r, "AbsoluteSize"));
                    }
                    if old.rot != now.rot {
                        fires.push((r, "AbsoluteRotation"));
                    }
                }
                _ => {}
            }
        }
        // (nothing kept for instances gone or no longer listened to)
        if w.abs_seen.len() > w.listening.len() {
            let World { abs_seen, listening, .. } = &mut *w;
            abs_seen.retain(|r, _| listening.contains(r));
        }
        for (r, name) in &fires {
            let pname = Value::String(lua.create_string(name)?);
            w.fire(*r, "Changed", vec![pname]);
            w.fire(*r, &format!("Changed:{name}"), vec![]);
        }
    }
    Ok(())
}

static LAYOUT_PROPS: std::sync::OnceLock<[rbx_dom_weak::Ustr; 5]> = std::sync::OnceLock::new();

fn gui_rect(w: &World, r: Ref, depth: usize) -> ([f32; 2], [f32; 2]) {
    let d = w.d();
    let screen = if w.input.view_size[0] > 0.0 {
        w.input.view_size
    } else {
        [1265.0, 836.0]
    };
    let Some(i) = d.dom.get_by_ref(r) else {
        return ([0.0; 2], screen);
    };
    if depth > 64 || !doc::is_a(&i.class, "GuiObject") {
        // The GUI's own canvas: a ScreenGui below the top bar inset (unless it
        // ignores it), a SurfaceGui its CanvasSize.
        let inset = mech::GUI_TOP_INSET;
        return match i.class.as_str() {
            "ScreenGui" if matches!(d.prop(r, "IgnoreGuiInset"), Some(Variant::Bool(true))) => {
                ([0.0, -inset], screen)
            }
            "ScreenGui" => ([0.0, 0.0], [screen[0], screen[1] - inset]),
            "SurfaceGui" => match d.prop(r, "CanvasSize") {
                Some(Variant::Vector2(v)) => ([0.0, 0.0], [v.x, v.y]),
                _ => ([0.0, 0.0], [800.0, 600.0]),
            },
            _ => ([0.0, 0.0], screen),
        };
    }
    let (ppos, psize) = gui_rect(w, i.parent(), depth + 1);
    let udim2 = |name: &str| match d.prop(r, name) {
        Some(Variant::UDim2(u)) => [u.x.scale, u.x.offset as f32, u.y.scale, u.y.offset as f32],
        _ => [0.0; 4],
    };
    let s = udim2("Size");
    let p = udim2("Position");
    let size = [s[0] * psize[0] + s[1], s[2] * psize[1] + s[3]];
    let anchor = match d.prop(r, "AnchorPoint") {
        Some(Variant::Vector2(v)) => [v.x, v.y],
        _ => [0.0, 0.0],
    };
    let pos = [
        ppos[0] + p[0] * psize[0] + p[1] - anchor[0] * size[0],
        ppos[1] + p[2] * psize[1] + p[3] - anchor[1] * size[1],
    ];
    (pos, size)
}

/// Read-side overrides: Mouse, Sound playback state, GUI layout, camera viewport.
/// A stored value, else `default` (for engine-owned properties the
/// reflection database has no default for).
fn stored_or(
    lua: &Lua,
    r: Ref,
    key: &str,
    default: impl FnOnce(&Lua) -> mlua::Result<Value>,
) -> mlua::Result<Option<Value>> {
    let v = world(lua)?
        .d()
        .dom
        .get_by_ref(r)
        .and_then(|i| i.properties.get(&rbx_dom_weak::ustr(key)).cloned());
    match v {
        Some(v) => {
            let enum_name = match key {
                "MouseBehavior" => Some("MouseBehavior"),
                "FloorMaterial" => Some("Material"),
                "CreatorType" => Some("CreatorType"),
                _ => None,
            };
            Ok(Some(variant_to_lua(lua, &v, enum_name)?))
        }
        None => Ok(Some(default(lua)?)),
    }
}

fn enum_item(lua: &Lua, en: &str, item: &str) -> mlua::Result<Value> {
    match LEnumItem::from_name(en, item) {
        Some(i) => crate::ltypes::enum_value(lua, i),
        None => Ok(Value::Nil),
    }
}

/// Client device and engine state the default player scripts read: a
/// desktop with keyboard and mouse, no touch, gamepad or VR (Studio's Play).
fn device_prop(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    let b = |v: bool| Ok(Some(Value::Boolean(v)));
    match (class, key) {
        (
            "UserInputService",
            "TouchEnabled"
            | "GamepadEnabled"
            | "VREnabled"
            | "AccelerometerEnabled"
            | "GyroscopeEnabled"
            | "OnScreenKeyboardVisible"
            | "ModalEnabled",
        ) => b(false),
        ("UserInputService", "KeyboardEnabled" | "MouseEnabled") => b(true),
        ("UserInputService", "PreferredInput") => {
            Ok(Some(enum_item(lua, "PreferredInput", "KeyboardAndMouse")?))
        }
        ("UserInputService", "MouseBehavior") => {
            stored_or(lua, r, key, |l| enum_item(l, "MouseBehavior", "Default"))
        }
        ("UserInputService", "MouseIconEnabled") => {
            stored_or(lua, r, key, |_| Ok(Value::Boolean(true)))
        }
        ("UserInputService", "MouseDeltaSensitivity") => {
            stored_or(lua, r, key, |_| Ok(Value::Number(1.0)))
        }
        ("UserInputService", "MouseIcon") => {
            stored_or(lua, r, key, |l| Ok(Value::String(l.create_string("")?)))
        }
        ("Lighting", "ClockTime") => Ok(Some(Value::Number(clock_hours(lua, r)? as f64))),
        ("GuiService", "IsModalDialog" | "IsWindows") => b(false),
        ("GuiService", "TouchControlsEnabled") => {
            stored_or(lua, r, key, |_| Ok(Value::Boolean(true)))
        }
        (
            "VRService",
            "VREnabled"
            | "ThirdPersonFollowCamEnabled"
            | "AvatarGestures"
            | "AutomaticScaling"
            | "FadeOutViewOnCollision",
        ) => b(false),
        ("VRService", "VRDeviceName") => Ok(Some(Value::String(lua.create_string("")?))),
        ("Camera", "NearPlaneZ") => Ok(Some(Value::Number(-0.1))),
        // Round 25: the cell box CopyRegion(MaxExtents) copies everything in,
        // and a TerrainRegion's size from its extents.
        ("Terrain", "MaxExtents") => {
            let v = |c: i16| crate::ltypes::LVector3int16([c; 3]);
            let r = crate::ltypes::LRegion3int16 { min: v(-32000), max: v(32000) };
            Ok(Some(Value::UserData(lua.create_userdata(r)?)))
        }
        ("Terrain" | "TerrainRegion", "IsSmooth") => b(true),
        // Saved, not scriptable: Studio says "ExtentsMin is not a valid
        // member of TerrainRegion" (round 26, region_probe.luau).
        ("TerrainRegion", "ExtentsMin" | "ExtentsMax") => {
            let full = world(lua)?.full_name(r);
            Err(rt_err(format!("{key} is not a valid member of TerrainRegion \"{full}\"")))
        }
        ("TerrainRegion", "SizeInCells") => {
            let w = world(lua)?;
            let ext = |k: &str| match w.d().prop(r, k) {
                Some(Variant::Vector3int16(v)) => Vec3::new(v.x as f32, v.y as f32, v.z as f32),
                _ => Vec3::ZERO,
            };
            // (Max inclusive; a TerrainRegion with no extents holds nothing)
            let has = w.d().prop(r, "ExtentsMax").is_some();
            let size = if has { (ext("ExtentsMax") - ext("ExtentsMin") + Vec3::ONE).max(Vec3::ZERO) } else { Vec3::ZERO };
            drop(w);
            Ok(Some(vec3_value(lua, size)?))
        }
        ("Camera", "HeadScale") => stored_or(lua, r, key, |_| Ok(Value::Number(1.0))),
        ("Camera", "HeadLocked") => stored_or(lua, r, key, |_| Ok(Value::Boolean(true))),
        ("Humanoid", "MoveDirection") => stored_or(lua, r, key, |l| {
            Ok(LVector3(Vec3::ZERO).into_value())
        }),
        // RootPart and the deprecated Torso share one getter in Studio
        // the rig's HumanoidRootPart, else
        // UpperTorso (R15) / Torso (R6). Old free-model NPCs test
        // `Humanoid.Torso ~= nil` to see they're alive (Place B's ZombieScript).
        ("Humanoid", "RootPart" | "Torso") => {
            let root = {
                let w = world(lua)?;
                let d = w.d();
                let parent = d
                    .dom
                    .get_by_ref(r)
                    .map(|i| i.parent())
                    .unwrap_or_else(Ref::none);
                // RigType: R6 0, R15 1.
                let r15 = crate::scene::enum_u32(d.prop(r, "RigType")).unwrap_or(0) != 0;
                let fallback = if r15 { mech::ROOT_PART_FALLBACK_R15 } else { mech::ROOT_PART_FALLBACK_R6 };
                let part_named = |name: &str| {
                    d.dom.get_by_ref(parent).and_then(|p| {
                        p.children().iter().copied().find(|c| {
                            d.dom
                                .get_by_ref(*c)
                                .is_some_and(|i| i.name == name && doc::is_a(&i.class, "BasePart"))
                        })
                    })
                };
                part_named("HumanoidRootPart").or_else(|| part_named(fallback))
            };
            Ok(Some(match root {
                Some(x) => inst_value(lua, x)?,
                None => Value::Nil,
            }))
        }
        (_, "Mass" | "AssemblyMass") if doc::is_a(class, "BasePart") => {
            // Plastic density × volume; a character's assembly is its whole
            // model (less Massless parts), other parts count alone.
            let w = world(lua)?;
            let d = w.d();
            let mass = |p: Ref| {
                let s = crate::scene::vec3_of(d.prop(p, "Size")).unwrap_or(Vec3::ONE);
                s.x * s.y * s.z * mech::PART_DENSITY
            };
            let mut m = mass(r);
            if key == "AssemblyMass" {
                let model = d
                    .dom
                    .get_by_ref(r)
                    .map(|i| i.parent())
                    .unwrap_or_else(Ref::none);
                let humanoid_model = d.dom.get_by_ref(model).is_some_and(|i| {
                    i.children()
                        .iter()
                        .any(|c| d.class_of(*c).is_some_and(|k| k == "Humanoid"))
                });
                if humanoid_model {
                    m = d
                        .dom
                        .descendants_of(model)
                        .filter(|i| {
                            doc::is_a(&i.class, "BasePart")
                                && !matches!(
                                    d.prop(i.referent(), "Massless"),
                                    Some(Variant::Bool(true))
                                )
                        })
                        .map(|i| mass(i.referent()))
                        .sum();
                }
            }
            Ok(Some(Value::Number(m as f64)))
        }
        // An unpublished place opened from a file, as in Studio: no place,
        // universe or creator ids (0) and version 0.
        ("DataModel", "PlaceId" | "GameId" | "CreatorId" | "PlaceVersion") => {
            stored_or(lua, r, key, |_| Ok(Value::Number(0.0)))
        }
        ("Humanoid", "CameraOffset") => stored_or(lua, r, key, |l| {
            Ok(LVector3(Vec3::ZERO).into_value())
        }),
        ("Humanoid", "SeatPart") => stored_or(lua, r, key, |_| Ok(Value::Nil)),
        ("Humanoid", "FloorMaterial") => {
            stored_or(lua, r, key, |l| enum_item(l, "Material", "Plastic"))
        }
        ("Humanoid", "EvaluateStateMachine") => {
            stored_or(lua, r, key, |_| Ok(Value::Boolean(true)))
        }
        _ => Ok(None),
    }
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if class.starts_with("Editable") {
        if let Some(v) = super::editable::prop_get(lua, r, class, key)? {
            return Ok(Some(v));
        }
    }
    if let Some(v) = device_prop(lua, r, class, key)? {
        return Ok(Some(v));
    }
    if matches!(
        key,
        "AbsoluteSize" | "AbsolutePosition" | "AbsoluteRotation"
    ) && doc::is_a(class, "GuiBase2d")
    {
        // As drawn last frame; before that (or when hidden), from the UDim2s.
        let b = {
            let mut w = world(lua)?;
            let b = abs_now(&w, r);
            // (what the script saw: a later layout that differs fires Changed)
            w.abs_seen.insert(r, b);
            b
        };
        return Ok(Some(match key {
            "AbsoluteSize" => Value::UserData(
                lua.create_userdata(LVector2(glam::Vec2::new(b.size[0], b.size[1])))?,
            ),
            "AbsolutePosition" => Value::UserData(
                lua.create_userdata(LVector2(glam::Vec2::new(b.pos[0], b.pos[1])))?,
            ),
            _ => Value::Number(b.rot as f64),
        }));
    }
    // UIListLayout / UIGridLayout / UITableLayout.AbsoluteContentSize: the
    // extent of the visible GuiObjects it lays out (its siblings), as drawn
    // last frame (before that, from their UDim2s).
    if key == "AbsoluteContentSize" && doc::is_a(class, "UIGridStyleLayout") {
        let w = world(lua)?;
        let d = w.d();
        let parent = d.dom.get_by_ref(r).map(|i| i.parent()).unwrap_or_else(Ref::none);
        let kids: Vec<Ref> = d
            .dom
            .get_by_ref(parent)
            .map(|p| p.children().to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|k| {
                d.class_of(*k).is_some_and(|c| doc::is_a(&c, "GuiObject"))
                    && !matches!(d.prop(*k, "Visible"), Some(Variant::Bool(false)))
            })
            .collect();
        let (mut mn, mut mx) = ([f32::MAX; 2], [f32::MIN; 2]);
        for k in &kids {
            let (pos, size) = match w.input.gui_layout.get(k) {
                Some(b) => (b.pos, b.size),
                None => gui_rect(&w, *k, 0),
            };
            for a in 0..2 {
                mn[a] = mn[a].min(pos[a]);
                mx[a] = mx[a].max(pos[a] + size[a]);
            }
        }
        let size = if kids.is_empty() {
            glam::Vec2::ZERO
        } else {
            glam::Vec2::new(mx[0] - mn[0], mx[1] - mn[1])
        };
        drop(w);
        return Ok(Some(Value::UserData(lua.create_userdata(LVector2(size))?)));
    }
    if key == "TextBounds" && matches!(class, "TextLabel" | "TextButton" | "TextBox") {
        if let Some(t) = world(lua)?.input.gui_layout.get(&r).and_then(|b| b.text) {
            return Ok(Some(Value::UserData(
                lua.create_userdata(LVector2(glam::Vec2::new(t[0], t[1])))?,
            )));
        }
        // Not drawn yet: estimate from the text length and TextSize.
        let w = world(lua)?;
        let d = w.d();
        let text = match d.prop(r, "Text") {
            Some(Variant::String(s)) => s,
            _ => String::new(),
        };
        let size = crate::scene::f32_of(d.prop(r, "TextSize")).unwrap_or(14.0);
        let width = text.chars().count() as f32 * size * mech::TEXT_WIDTH_PER_SIZE;
        return Ok(Some(Value::UserData(lua.create_userdata(LVector2(
            glam::Vec2::new(width, if text.is_empty() { 0.0 } else { size }),
        ))?)));
    }
    if key == "LocalTransparencyModifier" && doc::is_a(class, "BasePart") {
        // Client-only (never saved): 0 until a script sets it.
        let v = crate::scene::f32_of(
            world(lua)?
                .d()
                .dom
                .get_by_ref(r)
                .and_then(|i| i.properties.get(&rbx_dom_weak::ustr(key)).cloned()),
        )
        .unwrap_or(0.0);
        return Ok(Some(Value::Number(v as f64)));
    }
    if class == "AnimationTrack"
        && matches!(
            key,
            "Speed"
                | "IsPlaying"
                | "Length"
                | "TimePosition"
                | "WeightCurrent"
                | "WeightTarget"
                | "Looped"
                | "Priority"
                | "Animation"
        )
    {
        // The library's track state (an animation that couldn't load has Length 0, as in Roblox).
        let get: mlua::Function = lua.globals().get("__rs_track_get")?;
        let v: Value = get.call((inst_value(lua, r)?, key))?;
        if !v.is_nil() {
            return Ok(Some(v));
        }
    }
    if class == "Camera" && key == "ViewportSize" {
        let v = world(lua)?.input.view_size;
        let v = if v[0] > 0.0 { v } else { [1265.0, 836.0] };
        return Ok(Some(Value::UserData(
            lua.create_userdata(LVector2(glam::Vec2::new(v[0], v[1])))?,
        )));
    }
    if doc::is_a(class, "Mouse") {
        let inp = world(lua)?.input.clone();
        let hit = inp.mouse_hit.unwrap_or(Xform {
            pos: inp.mouse_ray.0 + inp.mouse_ray.1 * 1000.0,
            rot: glam::Mat3::IDENTITY,
        });
        return Ok(Some(match key {
            "Hit" | "hit" => cframe_value(lua, hit)?,
            // The face of Target the mouse is over (the hit's nearest face).
            "TargetSurface" => {
                let face = inp.mouse_target.and_then(|t| {
                    let w = world(lua).ok()?;
                    let d = w.d();
                    let txf = crate::scene::xform_of(d, t)?;
                    let size = crate::scene::vec3_of(d.prop(t, "Size")).unwrap_or(Vec3::ONE);
                    let rel = (txf.rot.transpose() * (hit.pos - txf.pos))
                        / (size * 0.5).max(Vec3::splat(1e-4));
                    let a = rel.abs();
                    Some(if a.x >= a.y && a.x >= a.z {
                        if rel.x > 0.0 { "Right" } else { "Left" }
                    } else if a.y >= a.z {
                        if rel.y > 0.0 { "Top" } else { "Bottom" }
                    } else if rel.z > 0.0 {
                        "Back"
                    } else {
                        "Front"
                    })
                });
                enum_item(lua, "NormalId", face.unwrap_or("Top"))?
            }
            "Target" | "target" => match inp.mouse_target {
                Some(t) => inst_value(lua, t)?,
                None => Value::Nil,
            },
            "Origin" => cframe_value(
                lua,
                Xform {
                    pos: inp.mouse_ray.0,
                    rot: glam::Mat3::IDENTITY,
                },
            )?,
            "UnitRay" => {
                let t = lua.create_table()?;
                t.set("Origin", vec3_value(lua, inp.mouse_ray.0)?)?;
                t.set("Direction", vec3_value(lua, inp.mouse_ray.1)?)?;
                Value::Table(t)
            }
            "X" => Value::Number(inp.mouse_xy[0] as f64),
            "Y" => Value::Number(inp.mouse_xy[1] as f64),
            "ViewSizeX" => Value::Number(inp.view_size[0] as f64),
            "ViewSizeY" => Value::Number(inp.view_size[1] as f64),
            _ => return Ok(None),
        }));
    }
    // Run-time properties with Roblox's documented values (round 12, from the
    // API-gap telemetry): what a game script reads in a Studio session.
    if class == "Tween" && matches!(key, "Instance" | "TweenInfo" | "PlaybackState") {
        let get: mlua::Function = lua.globals().get("__rs_tween_get")?;
        let v: Value = get.call((inst_value(lua, r)?, key))?;
        return Ok(Some(v));
    }
    if class == "Player" {
        let name = world(lua)?
            .d()
            .dom
            .get_by_ref(r)
            .map(|i| i.name.clone())
            .unwrap_or_default();
        let v = match key {
            // An offline Studio session: no account age, nobody followed,
            // en-us; the display name is the name unless one was set.
            "AccountAge" | "FollowUserId" | "DataComplexity" => Some(Value::Number(0.0)),
            "DisplayName" => {
                return stored_or(lua, r, key, |l| Ok(Value::String(l.create_string(&name)?)))
            }
            "MembershipType" => {
                return stored_or(lua, r, key, |l| enum_item(l, "MembershipType", "None"))
            }
            "LocaleId" => Some(Value::String(lua.create_string("en-us")?)),
            "PartyId" => Some(Value::String(lua.create_string("")?)),
            "CanLoadCharacterAppearance" => {
                return stored_or(lua, r, key, |_| Ok(Value::Boolean(true)))
            }
            "HasVerifiedBadge" | "GameplayPaused" | "DataReady" | "HasRobloxSubscription" => {
                return stored_or(lua, r, key, |_| Ok(Value::Boolean(false)))
            }
            "userId" => return index_alias(lua, r, "UserId"),
            _ => None,
        };
        if v.is_some() {
            return Ok(v);
        }
    }
    if class == "Players" {
        match key {
            "NumPlayers" | "numPlayers" => {
                let n = {
                    let w = world(lua)?;
                    let d = w.d();
                    d.dom
                        .get_by_ref(r)
                        .map(|i| {
                            i.children()
                                .iter()
                                .filter(|c| d.class_of(**c).is_some_and(|k| k == "Player"))
                                .count()
                        })
                        .unwrap_or(0)
                };
                return Ok(Some(Value::Number(n as f64)));
            }
            "localPlayer" => return index_alias(lua, r, "LocalPlayer"),
            _ => {}
        }
    }
    if class == "DataModel" {
        let v = match key {
            // A Studio test session: no job id, no private server.
            "JobId" | "PrivateServerId" | "VIPServerId" => {
                Some(Value::String(lua.create_string("")?))
            }
            "PrivateServerOwnerId" | "VIPServerOwnerId" => Some(Value::Number(0.0)),
            "CreatorType" => {
                return stored_or(lua, r, key, |l| enum_item(l, "CreatorType", "User"))
            }
            "Genre" => return stored_or(lua, r, key, |l| enum_item(l, "Genre", "All")),
            "MatchmakingType" => Some(enum_item(lua, "MatchmakingType", "Default")?),
            _ => None,
        };
        if v.is_some() {
            return Ok(v);
        }
    }
    if class == "Humanoid" && matches!(key, "Sit" | "PlatformStand") {
        return stored_or(lua, r, key, |_| Ok(Value::Boolean(false)));
    }
    if class == "Humanoid" && matches!(key, "WalkToPoint" | "TargetPoint") {
        return stored_or(lua, r, key, |l| {
            Ok(LVector3(Vec3::ZERO).into_value())
        });
    }
    if class == "Humanoid" && key == "WalkToPart" {
        return stored_or(lua, r, key, |_| Ok(Value::Nil));
    }
    if class == "Model" && key == "WorldPivot" {
        let xf = model_pivot(&*world(lua)?, r);
        return Ok(Some(cframe_value(lua, xf)?));
    }
    if class == "Attachment" && matches!(key, "Rotation" | "WorldRotation") {
        // Older names of Orientation / WorldOrientation.
        let canon = if key == "Rotation" { "Orientation" } else { "WorldOrientation" };
        return index_alias(lua, r, canon);
    }
    if doc::is_a(class, "BasePart")
        && matches!(
            key,
            "ExtentsCFrame" | "ExtentsSize" | "CenterOfMass" | "AssemblyCenterOfMass"
        )
    {
        // One part: its own box; its centre of mass is its centre.
        let xf = part_xform(lua, r)?;
        return Ok(Some(match key {
            "ExtentsSize" => vec3_value(lua, part_size(lua, r)?)?,
            "CenterOfMass" | "AssemblyCenterOfMass" => vec3_value(lua, xf.pos)?,
            _ => cframe_value(lua, xf)?,
        }));
    }
    if class == "Sound" {
        let w = world(lua)?;
        return Ok(match key {
            "TimePosition" => Some(Value::Number(
                w.input.sound_pos.get(&r).copied().unwrap_or(0.0),
            )),
            "TimeLength" => Some(Value::Number(
                w.input.sound_len.get(&r).copied().unwrap_or(0.0),
            )),
            "IsPlaying" => Some(Value::Boolean(w.input.sound_playing.contains(&r))),
            // Studio's IsPaused getter is
            // == 0`, the same byte Playing / IsPlaying read
            // a
            // sound that never played or has ended reads IsPaused = true.
            "IsPaused" => Some(Value::Boolean(!w.input.sound_playing.contains(&r))),
            "IsLoaded" => Some(Value::Boolean(true)),
            _ => None,
        });
    }
    Ok(None)
}

/// A property under its canonical name, through the normal index.
fn index_alias(lua: &Lua, r: Ref, key: &str) -> mlua::Result<Option<Value>> {
    let v = match inst_value(lua, r)? {
        Value::UserData(u) => u.get::<Value>(key)?,
        _ => Value::Nil,
    };
    Ok(Some(v))
}

/// Writes newindex acts on and then still stores (returns false for).
pub fn write_acts(class: &str, key: &str) -> bool {
    matches!(
        (class, key),
        ("Humanoid", "Jump")
            | ("Humanoid", "WalkToPoint")
            | ("Humanoid", "PlatformStand")
            | ("Model", "WorldPivot")
    )
}

/// Lighting's time of day as hours from "HH:MM:SS" (Lighting.TimeOfDay).
fn parse_time_of_day(s: &str) -> Option<f32> {
    let mut p = s.split(':').map(|x| x.trim().parse::<f32>().ok());
    let h = p.next()??;
    let m = p.next().flatten().unwrap_or(0.0);
    let sec = p.next().flatten().unwrap_or(0.0);
    Some(h + m / 60.0 + sec / 3600.0)
}

/// Lighting.ClockTime and TimeOfDay are one value in Roblox (whole seconds
/// in [0, 24], mech::clock_time): writing either sets both.
fn set_clock(lua: &Lua, r: Ref, hours: f32) -> mlua::Result<()> {
    let t = mech::clock_time(hours);
    let secs = (t * 3600.0).round() as u32;
    let tod = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    store_prop(lua, r, "ClockTime", Variant::Float32(t))?;
    store_prop(lua, r, "TimeOfDay", Variant::String(tod))
}

/// Lighting's hours now: ClockTime, else the saved TimeOfDay, else 14.
fn clock_hours(lua: &Lua, r: Ref) -> mlua::Result<f32> {
    let w = world(lua)?;
    let d = w.d();
    Ok(crate::scene::f32_of(d.prop(r, "ClockTime"))
        .or_else(|| match d.prop(r, "TimeOfDay") {
            Some(Variant::String(s)) => parse_time_of_day(&s),
            _ => None,
        })
        .unwrap_or(mech::CLOCK_TIME))
}

/// Write-side hooks: returns true when handled here.
pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    // Model.WorldPivot (Roblox): with a PrimaryPart, its PivotOffset becomes
    // PrimaryPart.CFrame⁻¹ · pivot; without one, the model keeps the pivot
    // itself (WorldPivotData). Nothing moves. (model_pivot reads both.)
    if key == "WorldPivot" && doc::is_a(class, "Model") {
        let pivot = opt_ud::<LCFrame>(value)
            .ok_or_else(|| rt_err("Unable to assign property WorldPivot. CFrame expected"))?
            .0;
        let primary = match world(lua)?.d().prop(r, "PrimaryPart") {
            Some(Variant::Ref(pp)) if pp.is_some() => Some(pp),
            _ => None,
        };
        let primary_cf = primary.and_then(|pp| match world(lua).ok()?.d().prop(pp, "CFrame") {
            Some(Variant::CFrame(cf)) => Some((pp, Xform::from_cframe(&cf))),
            _ => None,
        });
        match primary_cf {
            Some((pp, cf)) => {
                let off = crate::ltypes::cf_mul(&crate::ltypes::cf_inv(&cf), &pivot);
                super::newindex(
                    lua,
                    pp,
                    "PivotOffset",
                    Value::UserData(lua.create_userdata(LCFrame(off))?),
                )?;
            }
            None => store_prop(
                lua,
                r,
                "WorldPivotData",
                Variant::OptionalCFrame(Some(pivot.to_cframe())),
            )?,
        }
        // Studio's setter then signals WorldPivot itself changed.
        let mut w = world(lua)?;
        w.fire(r, "Changed", vec![Value::String(lua.create_string("WorldPivot")?)]);
        w.fire(r, "Changed:WorldPivot", vec![]);
        return Ok(true);
    }
    match (class, key) {
        ("Lighting", "ClockTime") => {
            let h = num(value).ok_or_else(|| rt_err("ClockTime: number expected"))?;
            set_clock(lua, r, h as f32)?;
            Ok(true)
        }
        ("Lighting", "TimeOfDay") => {
            let h = match value {
                Value::String(s) => parse_time_of_day(&s.to_str()?),
                _ => None,
            }
            .ok_or_else(|| rt_err("TimeOfDay: \"HH:MM:SS\" expected"))?;
            set_clock(lua, r, h)?;
            Ok(true)
        }
        ("Sound", "Playing") => {
            // Idempotent, unlike Play(): setting the value it already has
            // does nothing; false pauses (TimePosition is kept), true goes
            // on from there (Roblox Sound.Playing).
            let on = matches!(value, Value::Boolean(true));
            let (playing, paused) = {
                let w = world(lua)?;
                (
                    w.input.sound_playing.contains(&r),
                    w.input.sound_paused.contains(&r),
                )
            };
            let cmd = match (on, playing, paused) {
                (true, true, _) | (false, false, _) => return Ok(true),
                (true, false, true) => SoundCmd::Resume,
                (true, false, false) => SoundCmd::Play,
                (false, true, _) => SoundCmd::Pause,
            };
            sound(lua, r, cmd)?;
            Ok(true)
        }
        ("Humanoid", "PlatformStand") => {
            // Humanoid.PlatformStanding(active) when it changes (Roblox).
            let on = matches!(value, Value::Boolean(true));
            let was = matches!(world(lua)?.d().prop(r, "PlatformStand"), Some(Variant::Bool(true)));
            if on != was {
                world(lua)?.fire(r, "PlatformStanding", vec![Value::Boolean(on)]);
            }
            Ok(false)
        }
        ("Humanoid", "Jump") if matches!(value, Value::Boolean(true)) => {
            world(lua)?.requests.jumps.push(r);
            Ok(false)
        }
        ("AnimationTrack", "Looped" | "Priority" | "TimePosition") => {
            let set: mlua::Function = lua.globals().get("__rs_track_set")?;
            set.call::<bool>((inst_value(lua, r)?, key, value.clone()))
        }
        ("Humanoid", "WalkToPoint") => {
            if let Some(p) = opt_ud::<LVector3>(value) {
                world(lua)?.requests.move_to.push((r, p.0));
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Engine methods from METHODS; None when `name` isn't one of them.
pub fn call(
    lua: &Lua,
    name: &str,
    this: Ref,
    args: &MultiValue,
) -> Option<mlua::Result<MultiValue>> {
    let mask = match super::inst_class(lua, this) {
        Ok((class, _)) => super::domains::call_claims(class.as_str(), name, false, None),
        Err(_) => u16::MAX,
    };
    Some(call_inner(lua, name, this, args, mask))
}

/// `call` asking only the domains in `domains` (the method's route,
/// runtime_routes.rs).
pub fn call_routed(lua: &Lua, name: &str, this: Ref, args: &MultiValue, domains: u16) -> mlua::Result<MultiValue> {
    call_inner(lua, name, this, args, domains)
}

fn call_inner(lua: &Lua, name: &str, this: Ref, args: &MultiValue, domains: u16) -> mlua::Result<MultiValue> {
    // EditableMesh / EditableImage and AssetService's Editable* calls (editable.rs).
    if let Some(r) = super::editable::call(lua, name, this, args) {
        return r;
    }
    if let Some(r) = super::domains::call_masked(lua, name, this, args, domains) {
        return r;
    }
    match name {
        "BreakJoints" => {
            break_joints(lua, this)?;
            none()
        }
        "MakeJoints"
        | "SetNetworkOwner"
        | "SetNetworkOwnershipAuto"
        | "ApplyAngularImpulse"
        | "ApplyDescription"
        | "ClearCharacterAppearance"
        | "BindToClose"
        | "SetCore" => none(),
        "GetModelCFrame" => {
            let xf = model_pivot(&*world(lua)?, this);
            one(cframe_value(lua, xf)?)
        }
        "GetModelSize" | "GetExtentsSize" => {
            let (lo, hi) = model_bounds(lua, this)?;
            one(vec3_value(lua, hi - lo)?)
        }
        "GetBoundingBox" => {
            let (lo, hi) = model_bounds(lua, this)?;
            let rot = model_pivot(&*world(lua)?, this).rot;
            Ok(MultiValue::from_iter([
                cframe_value(
                    lua,
                    Xform {
                        pos: (lo + hi) * 0.5,
                        rot,
                    },
                )?,
                vec3_value(lua, hi - lo)?,
            ]))
        }
        "TranslateBy" => {
            let v = vec3_arg(args, 1, "TranslateBy")?;
            let p = model_pivot(&*world(lua)?, this);
            pivot_to(
                lua,
                this,
                Xform {
                    pos: p.pos + v,
                    rot: p.rot,
                },
            )?;
            none()
        }
        "GetScale" => one(Value::Number(1.0)),
        "GetTouchingParts" => one(Value::Table(list(lua, Vec::new())?)),
        "GetConnectedParts" | "GetRootPart" => {
            if name == "GetRootPart" {
                one(inst_value(lua, this)?)
            } else {
                one(Value::Table(list(lua, vec![this])?))
            }
        }
        "GetNetworkOwner" => one(Value::Nil),
        "CanSetNetworkOwnership" => Ok(MultiValue::from_iter([Value::Boolean(true), Value::Nil])),
        "Resize" | "CanCollideWith" => one(Value::Boolean(true)),
        "GetVelocityAtPosition" => {
            let v = world(lua)?
                .velocities
                .get(&this)
                .copied()
                .unwrap_or(Vec3::ZERO);
            one(vec3_value(lua, v)?)
        }
        "Raycast" => {
            let origin = vec3_arg(args, 1, "Raycast")?;
            let dir = vec3_arg(args, 2, "Raycast")?;
            let filter = match arg(args, 3) {
                Value::Nil => Filter::none(),
                p => params_filter_groups(lua, &p),
            };
            match cast(lua, origin, dir, &filter)? {
                Some(hit) => one(raycast_result(lua, hit)?),
                None => one(Value::Nil),
            }
        }
        // WorldRoot:Spherecast(position, radius, direction, params),
        // Blockcast(cframe, size, direction, params), Shapecast(part,
        // direction, params): the shape swept along `direction`.
        "Spherecast" | "Blockcast" | "Shapecast" => {
            let probe: Option<(u8, Vec3, glam::Mat3)>;
            let (origin, grow, dir, params) = match name {
                // Studio's checks (Spherecast, Blockcast
                // Shapecast), with its messages:
                // finite inputs, radius ≥ 0, sizes up to FInt
                // ShapecastMaxSize (512; a sphere's diameter), distance up to
                // ShapecastMaxDistance (1024).
                "Spherecast" => {
                    let o = vec3_arg(args, 1, name)?;
                    if !o.is_finite() {
                        return Err(rt_err("Invalid position. Must contain finite values."));
                    }
                    let r = num(&arg(args, 2)).unwrap_or(0.0) as f32;
                    if !r.is_finite() {
                        return Err(rt_err("Invalid Radius. Must contain finite values."));
                    }
                    if r < 0.0 {
                        return Err(rt_err(format!(
                            "Invalid Radius. Must be greater than or equal to 0, got {r:.6}."
                        )));
                    }
                    if r + r > mech::SHAPECAST_MAX_SIZE {
                        return Err(rt_err(format!(
                            "Attempt to spherecast with radius {r:.6}. The maximum radius is {:.6}.",
                            mech::SHAPECAST_MAX_SIZE * 0.5
                        )));
                    }
                    probe = Some((crate::query_shapes::BALL, Vec3::splat(r + r), glam::Mat3::IDENTITY));
                    (o, Vec3::splat(r), vec3_arg(args, 3, name)?, arg(args, 4))
                }
                "Blockcast" => {
                    let cf = opt_ud::<LCFrame>(&arg(args, 1))
                        .ok_or_else(|| rt_err("Blockcast: CFrame expected"))?;
                    let side = vec3_arg(args, 2, name)?;
                    if side.max_element() > mech::SHAPECAST_MAX_SIZE {
                        return Err(rt_err(format!(
                            "Attempt to blockcast with side length {:.6}. The maximum side length is {:.6}.",
                            side.max_element(),
                            mech::SHAPECAST_MAX_SIZE
                        )));
                    }
                    let h = side * 0.5;
                    let r = cf.0.rot;
                    let ext = r.x_axis.abs() * h.x + r.y_axis.abs() * h.y + r.z_axis.abs() * h.z;
                    probe = Some((crate::query_shapes::BOX, side, r));
                    (cf.0.pos, ext, vec3_arg(args, 3, name)?, arg(args, 4))
                }
                _ => {
                    let p = opt_inst(&arg(args, 1))
                        .ok_or_else(|| rt_err("Shapecast: BasePart expected"))?;
                    if world(lua)?.d().class_of(p).is_some_and(|c| c == "Terrain") {
                        return Err(rt_err("Shapecasted part cannot be Terrain."));
                    }
                    let xf = part_xform(lua, p)?;
                    let size = part_size(lua, p)?;
                    if size.max_element() > mech::SHAPECAST_MAX_SIZE {
                        return Err(rt_err(format!(
                            "Attempt to shapecast with side length {:.6}. The maximum side length is {:.6}.",
                            size.max_element(),
                            mech::SHAPECAST_MAX_SIZE
                        )));
                    }
                    let kind = {
                        let w = world(lua)?;
                        let d = w.d();
                        let class = d.class_of(p).map(|c| c.to_string()).unwrap_or_default();
                        crate::query_shapes::kind_of(d, p, &class)
                    };
                    probe = Some((kind, size, xf.rot));
                    let ball = kind == crate::query_shapes::BALL;
                    let ext = if ball {
                        Vec3::splat(size.min_element() * 0.5)
                    } else {
                        let h = size * 0.5;
                        let r = xf.rot;
                        r.x_axis.abs() * h.x + r.y_axis.abs() * h.y + r.z_axis.abs() * h.z
                    };
                    (xf.pos, ext, vec3_arg(args, 2, name)?, arg(args, 3))
                }
            };
            let mut filter = match params {
                Value::Nil => Filter::none(),
                p => params_filter_groups(lua, &p),
            };
            if name == "Shapecast" {
                if let Some(p) = opt_inst(&arg(args, 1)) {
                    if !filter.include {
                        filter.list.insert(p);
                    }
                }
            }
            if !dir.is_finite() {
                return Err(rt_err("Invalid direction. Must contain finite values."));
            }
            if dir.length() > mech::SHAPECAST_MAX_DISTANCE {
                return Err(rt_err(format!(
                    "Attempt to shapecast with distance {:.6}. The maximum distance is {}.",
                    dir.length(),
                    mech::SHAPECAST_MAX_DISTANCE as i32
                )));
            }
            match cast_swept(lua, origin, dir, &filter, grow, probe)? {
                Some(hit) => one(raycast_result(lua, hit)?),
                None => one(Value::Nil),
            }
        }
        // BasePart helpers (Roblox docs), for one part at a time.
        "GetRenderCFrame" => one(cframe_value(lua, part_xform(lua, this)?)?),
        "GetClosestPointOnSurface" => {
            // The closest point on (or in) the part's box to `position`.
            let p = vec3_arg(args, 1, name)?;
            let xf = part_xform(lua, this)?;
            let h = part_size(lua, this)? * 0.5;
            let local = (xf.rot.transpose() * (p - xf.pos)).clamp(-h, h);
            one(vec3_value(lua, xf.pos + xf.rot * local)?)
        }
        "GetJoints" | "GetNoCollisionConstraints" => {
            // Joints / NoCollisionConstraints anywhere that name this part.
            let want_nc = name == "GetNoCollisionConstraints";
            let found: Vec<Ref> = {
                let w = world(lua)?;
                let d = w.d();
                d.dom
                    .descendants()
                    .filter(|i| {
                        let is_joint = doc::is_a(&i.class, "JointInstance")
                            || matches!(i.class.as_str(), "WeldConstraint" | "NoCollisionConstraint")
                            || doc::is_a(&i.class, "Constraint");
                        let ok = if want_nc {
                            i.class == "NoCollisionConstraint"
                        } else {
                            is_joint && i.class != "NoCollisionConstraint"
                        };
                        ok && ["Part0", "Part1"].iter().any(|k| {
                            matches!(i.properties.get(&ustr(k)), Some(Variant::Ref(x)) if *x == this)
                        })
                    })
                    .map(|i| i.referent())
                    .collect()
            };
            one(Value::Table(list(lua, found)?))
        }
        "ApplyImpulseAtPosition" => {
            // The linear part of the impulse (no angular response yet).
            let imp = vec3_arg(args, 1, name)?;
            let mass = num(&index_alias(lua, this, "AssemblyMass")?.unwrap_or(Value::Nil))
                .unwrap_or(1.0)
                .max(1e-4) as f32;
            let v = world(lua)?.velocities.get(&this).copied().unwrap_or(Vec3::ZERO);
            world(lua)?.velocity_requests.push((this, v + imp / mass));
            none()
        }
        // Lighting:GetSunDirection / GetMoonDirection (unit vectors toward
        // them, from ClockTime and GeographicLatitude) and GetMoonPhase.
        "GetSunDirection" | "GetMoonDirection" => {
            let (clock, lat) = {
                let w = world(lua)?;
                let d = w.d();
                (
                    crate::scene::f32_of(d.prop(this, "ClockTime")).unwrap_or(14.0),
                    crate::scene::f32_of(d.prop(this, "GeographicLatitude")).unwrap_or(41.733),
                )
            };
            let s = crate::lighting::sun_direction(clock, lat);
            one(vec3_value(lua, if name == "GetSunDirection" { s } else { -s })?)
        }
        "GetMoonPhase" => one(Value::Number(mech::MOON_PHASE as f64)),
        "players" => {
            let kids: Vec<Ref> = {
                let all = children_of(lua, this)?;
                let w = world(lua)?;
                all.into_iter()
                    .filter(|c| w.d().class_of(*c).is_some_and(|k| k == "Player"))
                    .collect()
            };
            one(Value::Table(list(lua, kids)?))
        }
        "FindPartOnRay" | "FindPartOnRayWithIgnoreList" | "FindPartOnRayWithWhitelist" => {
            let ray = arg(args, 1);
            let origin = ud_field::<LVector3>(&ray, "Origin")
                .map(|v| v.0)
                .ok_or_else(|| rt_err(format!("{name}: Ray expected")))?;
            let dir = ud_field::<LVector3>(&ray, "Direction")
                .map(|v| v.0)
                .unwrap_or(Vec3::ZERO);
            let list: HashSet<Ref> = refs_in(&arg(args, 2)).into_iter().collect();
            let filter = Filter {
                list,
                include: name == "FindPartOnRayWithWhitelist",
                respect_collide: false,
                skip_groups: HashSet::new(),
            };
            match cast(lua, origin, dir, &filter)? {
                Some((r, p, n, _)) => {
                    let terrain = lua
                        .app_data_ref::<QueryCache>()
                        .and_then(|c| c.terrain_material)
                        .filter(|_| world(lua).is_ok_and(|w| w.d().class_of(r).is_some_and(|k| k == "Terrain")));
                    let mat = match (terrain, world(lua)?.d().prop(r, "Material")) {
                        (Some(m), _) => LEnumItem::from_name("Material", crate::terrain::material_name(m)),
                        (None, Some(Variant::Enum(e))) => LEnumItem::from_value("Material", e.to_u32()),
                        _ => None,
                    };
                    let mat = match mat {
                        Some(m) => crate::ltypes::enum_value(lua, m)?,
                        None => Value::Nil,
                    };
                    Ok(MultiValue::from_iter([
                        inst_value(lua, r)?,
                        vec3_value(lua, p)?,
                        vec3_value(lua, n)?,
                        mat,
                    ]))
                }
                None => Ok(MultiValue::from_iter([
                    Value::Nil,
                    vec3_value(lua, origin + dir)?,
                    vec3_value(lua, Vec3::ZERO)?,
                ])),
            }
        }
        "FindPartsInRegion3"
        | "FindPartsInRegion3WithIgnoreList"
        | "FindPartsInRegion3WithWhiteList" => {
            let (lo, hi) = region_box(&arg(args, 1))
                .ok_or_else(|| rt_err(format!("{name}: Region3 expected")))?;
            let listed: HashSet<Ref> = refs_in(&arg(args, 2)).into_iter().collect();
            let filter = Filter {
                list: listed,
                include: name.ends_with("WhiteList"),
                respect_collide: false,
                skip_groups: HashSet::new(),
            };
            let max = num(&arg(args, 3)).map(|n| n as usize).unwrap_or(20);
            let parts = overlap(lua, lo, hi, None, &filter, max)?;
            one(Value::Table(list(lua, parts)?))
        }
        "GetPartBoundsInBox" => {
            let cf = opt_ud::<LCFrame>(&arg(args, 1))
                .ok_or_else(|| rt_err("GetPartBoundsInBox: CFrame expected"))?;
            let size = vec3_arg(args, 2, "GetPartBoundsInBox")?;
            let h = size * 0.5;
            let r = cf.0.rot;
            let ext = r.x_axis.abs() * h.x + r.y_axis.abs() * h.y + r.z_axis.abs() * h.z;
            let filter = match arg(args, 3) {
                Value::Nil => Filter::none(),
                p => params_filter_groups(lua, &p),
            };
            let max = num(&ud_value(&arg(args, 3), "MaxParts"))
                .filter(|n| *n > 0.0)
                .map(|n| n as usize)
                .unwrap_or(usize::MAX);
            let parts = overlap(lua, cf.0.pos - ext, cf.0.pos + ext, None, &filter, max)?;
            one(Value::Table(list(lua, parts)?))
        }
        "GetPartBoundsInRadius" => {
            let c = vec3_arg(args, 1, "GetPartBoundsInRadius")?;
            let r = num(&arg(args, 2)).unwrap_or(0.0) as f32;
            let filter = match arg(args, 3) {
                Value::Nil => Filter::none(),
                p => params_filter_groups(lua, &p),
            };
            let parts = overlap(
                lua,
                c - Vec3::splat(r),
                c + Vec3::splat(r),
                Some((c, r)),
                &filter,
                usize::MAX,
            )?;
            one(Value::Table(list(lua, parts)?))
        }
        "GetPartsInPart" => {
            let p = opt_inst(&arg(args, 1))
                .ok_or_else(|| rt_err("GetPartsInPart: BasePart expected"))?;
            let xf = part_xform(lua, p)?;
            let size = part_size(lua, p)?;
            let probe_kind = {
                let w = world(lua)?;
                let d = w.d();
                let class = d.class_of(p).map(|c| c.to_string()).unwrap_or_default();
                crate::query_shapes::kind_of(d, p, &class)
            };
            let (lo, hi) = part_aabb(&QPart {
                r: p,
                xf,
                size,
                collide: true,
                query: true,
                mesh: false,
                shape: 0,
            });
            let mut filter = match arg(args, 2) {
                Value::Nil => Filter::none(),
                v => params_filter(&v),
            };
            if !filter.include {
                filter.list.insert(p);
            }
            let max = num(&ud_value(&arg(args, 2), "MaxParts"))
                .filter(|n| *n > 0.0)
                .map(|n| n as usize)
                .unwrap_or(usize::MAX);
            // Bounds first, then the real shapes (GetPartsInPart is exact in
            // Roblox: a turned brush's or a terrain chunk's box covers far more
            // than the part).
            let near = overlap(lua, lo, hi, None, &filter, usize::MAX)?;
            let mut parts = Vec::new();
            for r in near {
                if parts.len() >= max {
                    break;
                }
                let q = lua.app_data_ref::<QueryCache>().and_then(|c| {
                    c.index.get(&r).and_then(|i| c.parts.get(*i)).copied()
                });
                let Some(q) = q else { continue };
                let exact = if q.mesh {
                    let id = match world(lua)?.d().prop(r, "MeshId") {
                        Some(Variant::ContentId(c)) => c.as_str().to_string(),
                        _ => crate::scene::union_mesh(world(lua)?.d(), r).map(|m| m.id).unwrap_or_default(),
                    };
                    match (!id.is_empty()).then(|| crate::mesh_bvh::for_mesh(&id)).flatten() {
                        Some(bvh) => crate::query_shapes::obb_mesh_bvh(&xf, size, &q.xf, q.size, &bvh),
                        None => crate::query_shapes::obb_obb(&xf, size, &q.xf, q.size),
                    }
                } else {
                    crate::query_shapes::part_part(&xf, size, probe_kind, &q.xf, q.size, q.shape)
                };
                if exact {
                    parts.push(r);
                }
            }
            one(Value::Table(list(lua, parts)?))
        }
        "GetServerTimeNow" => one(Value::Number(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
        )),
        "GetRealPhysicsFPS" => one(Value::Number(mech::PHYSICS_HZ as f64)),
        "MoveTo" => {
            let target = vec3_arg(args, 1, "MoveTo")?;
            world(lua)?.requests.move_to.push((this, target));
            none()
        }
        "EquipTool" => {
            let tool = opt_inst(&arg(args, 1)).ok_or_else(|| rt_err("EquipTool: Tool expected"))?;
            world(lua)?.requests.equip.push((this, Some(tool)));
            none()
        }
        "UnequipTools" => {
            world(lua)?.requests.equip.push((this, None));
            none()
        }
        "ChangeState" => {
            if let Some(e) = opt_ud::<LEnumItem>(&arg(args, 1)) {
                let mut w = world(lua)?;
                if e.name == "Jumping" {
                    w.requests.jumps.push(this);
                } else {
                    w.requests.change_state.push((this, e.name.to_string()));
                }
            }
            none()
        }
        // Humanoid:SetStateEnabled / GetStateEnabled: src/domains/anim.rs.
        // Seat:Sit(humanoid): the humanoid sits in this seat.
        "Sit" => {
            let h = opt_inst(&arg(args, 1)).ok_or_else(|| rt_err("Sit: Humanoid expected"))?;
            world(lua)?.requests.sits.push((this, h));
            none()
        }
        "HasAppearanceLoaded" | "IsLoaded" => one(Value::Boolean(true)),
        "SetCoreGuiEnabled" | "GetCoreGuiEnabled" => {
            // StarterGui::setCoreGuiEnabled / getCoreGuiEnabled
            // client only; All (4) sets every type 0..9; one
            // type is set, then All becomes `enabled` only if every other
            // type agrees with it (else false). Get reads the type's own
            // entry, All included.
            if world(lua)?.side != Side::Client {
                return Err(rt_err(format!(
                    "StarterGui:{name} must be called from a local script."
                )));
            }
            // The item, its name or its value, as Roblox's enum arguments
            // (an admin script's console: `GetCoreGuiEnabled("Chat")`).
            let ty = match arg(args, 1) {
                Value::Nil => return Err(rt_err(format!("{name}: Enum.CoreGuiType expected"))),
                v => crate::ltypes::enum_arg(Some(&v), "CoreGuiType", "All")?,
            };
            let label = |v: u32| LEnumItem::from_value("CoreGuiType", v).map(|e| e.name.to_string());
            let mut w = world(lua)?;
            let ty_name = ty.name.to_string();
            if name == "GetCoreGuiEnabled" {
                return one(Value::Boolean(!w.core_gui_off.contains(&ty_name)));
            }
            let on = matches!(arg(args, 2), Value::Boolean(true));
            let set = |w: &mut World, t: String| {
                if on {
                    w.core_gui_off.remove(&t);
                } else {
                    w.core_gui_off.insert(t);
                }
            };
            if ty.value == mech::CORE_GUI_ALL {
                for v in 0..mech::CORE_GUI_TYPE_COUNT {
                    if let Some(t) = label(v) {
                        set(&mut w, t);
                    }
                }
            } else {
                set(&mut w, ty_name);
                let agree = (0..mech::CORE_GUI_TYPE_COUNT)
                    .filter(|v| *v != mech::CORE_GUI_ALL)
                    .filter_map(label)
                    .all(|t| w.core_gui_off.contains(&t) != on);
                if let Some(all) = label(mech::CORE_GUI_ALL) {
                    if agree && on {
                        w.core_gui_off.remove(&all);
                    } else {
                        w.core_gui_off.insert(all);
                    }
                }
            }
            none()
        }
        "GetAccessories" => {
            let parent = inst_meta(lua, this)?.2;
            let acc: Vec<Ref> = {
                let w = world(lua)?;
                let d = w.d();
                d.dom
                    .get_by_ref(parent)
                    .map(|i| {
                        i.children()
                            .iter()
                            .copied()
                            .filter(|c| {
                                d.class_of(*c)
                                    .is_some_and(|k| doc::is_a(&k, "Accoutrement"))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            one(Value::Table(list(lua, acc)?))
        }
        "AddAccessory" => {
            let acc = opt_inst(&arg(args, 1))
                .ok_or_else(|| rt_err("AddAccessory: Accessory expected"))?;
            let parent = inst_meta(lua, this)?.2;
            set_parent(lua, acc, parent)?;
            {
                // Welded to the matching body attachment, as Roblox does.
                let mut w = world(lua)?;
                let d = w.dm();
                crate::avatar::weld_accessory(&mut d.dom, parent, acc);
                d.mark_tree(acc);
            }
            none()
        }
        "RemoveAccessories" => {
            let parent = inst_meta(lua, this)?.2;
            let acc: Vec<Ref> = {
                let w = world(lua)?;
                let d = w.d();
                d.dom
                    .get_by_ref(parent)
                    .map(|i| {
                        i.children()
                            .iter()
                            .copied()
                            .filter(|c| {
                                d.class_of(*c)
                                    .is_some_and(|k| doc::is_a(&k, "Accoutrement"))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            for a in acc {
                destroy_inst(lua, a)?;
            }
            none()
        }
        "Play" | "Resume" | "Stop" | "Pause" => {
            let cmd = match name {
                "Play" => SoundCmd::Play,
                "Resume" => SoundCmd::Resume,
                "Stop" => SoundCmd::Stop,
                _ => SoundCmd::Pause,
            };
            sound(lua, this, cmd)?;
            none()
        }
        "SetListener" => {
            let mode = opt_ud::<LEnumItem>(&arg(args, 1))
                .filter(|e| e.enum_type == "ListenerType")
                .ok_or_else(|| rt_err("SetListener: ListenerType expected"))?
                .value;
            let cf = opt_ud::<LCFrame>(&arg(args, 2)).map(|x| x.0);
            let part = opt_inst(&arg(args, 2));
            if mode > 3 || (mode == 1 && cf.is_none()) || (mode >= 2 && part.is_none()) {
                return Err(rt_err("SetListener: invalid listener"));
            }
            world(lua)?.input.sound_listener = Some((mode, cf, part));
            none()
        }
        "GetListener" => {
            let (mode, cf, part) = world(lua)?.input.sound_listener.unwrap_or((0, None, None));
            let ty = LEnumItem::from_value("ListenerType", mode)
                .ok_or_else(|| rt_err("ListenerType unavailable"))?;
            let listener = if let Some(cf) = cf {
                Value::UserData(lua.create_userdata(LCFrame(cf))?)
            } else if let Some(part) = part {
                inst_value(lua, part)?
            } else {
                Value::Nil
            };
            Ok(MultiValue::from_vec(vec![
                Value::UserData(lua.create_userdata(ty)?),
                listener,
            ]))
        }
        "PlayLocalSound" => {
            let s =
                opt_inst(&arg(args, 1)).ok_or_else(|| rt_err("PlayLocalSound: Sound expected"))?;
            sound(lua, s, SoundCmd::Play)?;
            none()
        }
        "Activate" | "Deactivate" => {
            let ev = if name == "Activate" {
                "Activated"
            } else {
                "Deactivated"
            };
            world(lua)?.fire(this, ev, vec![]);
            none()
        }
        "AddTag" | "RemoveTag" | "HasTag" | "GetTags" => {
            // Instance:AddTag("t") or CollectionService:AddTag(inst, "t").
            let on_service = inst_meta(lua, this)?.0 == "CollectionService";
            let (target, tag_at) = if on_service {
                (
                    opt_inst(&arg(args, 1))
                        .ok_or_else(|| rt_err(format!("{name}: Instance expected")))?,
                    2,
                )
            } else {
                (this, 1)
            };
            match name {
                "GetTags" => {
                    let tags = tags_of(lua, target)?;
                    one(Value::Table(lua.create_sequence_from(tags)?))
                }
                _ => {
                    let tag = arg_str(args, tag_at, "string")?;
                    match name {
                        "AddTag" => add_tag(lua, target, &tag).and_then(|_| none()),
                        "RemoveTag" => remove_tag(lua, target, &tag).and_then(|_| none()),
                        _ => one(Value::Boolean(
                            tags_of(lua, target)?.iter().any(|t| *t == tag),
                        )),
                    }
                }
            }
        }
        "GetTagged" => {
            let tag = arg_str(args, 1, "string")?;
            let found = tagged(lua, &tag)?;
            one(Value::Table(list(lua, found)?))
        }
        "GetAllTags" => {
            let mut all: Vec<String> = {
                let w = world(lua)?;
                let d = w.d();
                d.dom
                    .descendants()
                    .filter_map(|i| match i.properties.get(&ustr("Tags")) {
                        Some(Variant::Tags(t)) => {
                            Some(t.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                        }
                        _ => None,
                    })
                    .flatten()
                    .collect()
            };
            all.sort();
            all.dedup();
            one(Value::Table(lua.create_sequence_from(all)?))
        }
        "GetInstanceAddedSignal" | "GetInstanceRemovedSignal" => {
            let tag = arg_str(args, 1, "string")?;
            let ev = if name == "GetInstanceAddedSignal" {
                format!("TagAdded:{tag}")
            } else {
                format!("TagRemoved:{tag}")
            };
            one(signal(lua, this, &ev)?)
        }
        "GetTeams" => {
            let kids: Vec<Ref> = {
                let w = world(lua)?;
                let d = w.d();
                d.dom
                    .get_by_ref(this)
                    .map(|i| {
                        i.children()
                            .iter()
                            .copied()
                            .filter(|c| d.class_of(*c).is_some_and(|k| k == "Team"))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            one(Value::Table(list(lua, kids)?))
        }
        "GetPlayers" => {
            // Team:GetPlayers(): players whose Team is this one.
            let found: Vec<Ref> = {
                let w = world(lua)?;
                let d = w.d();
                let players = d.find_child(d.dom.root_ref(), "Players");
                players
                    .and_then(|p| d.dom.get_by_ref(p))
                    .map(|i| i.children().iter().copied().filter(|p| matches!(d.prop(*p, "Team"), Some(Variant::Ref(t)) if t == this)).collect())
                    .unwrap_or_default()
            };
            one(Value::Table(list(lua, found)?))
        }
        "GetPlayerByUserId" => {
            let id = num(&arg(args, 1)).unwrap_or(-1.0) as i64;
            let found = {
                let w = world(lua)?;
                let d = w.d();
                d.dom.get_by_ref(this).and_then(|i| {
                    i.children().iter().copied().find(
                        |p| matches!(d.prop(*p, "UserId"), Some(Variant::Int64(u)) if u == id),
                    )
                })
            };
            one(opt_ref_value(lua, found)?)
        }
        "GetMouse" => {
            let existing = world(lua)?.input.mouse;
            let m = match existing {
                Some(m) if world(lua)?.d().exists(m) => m,
                _ => {
                    let m = create_any(lua, "PlayerMouse")?;
                    world(lua)?.input.mouse = Some(m);
                    m
                }
            };
            one(inst_value(lua, m)?)
        }
        "GetNetworkPing" => one(Value::Number(0.0)),
        "DistanceFromCharacter" => {
            let p = vec3_arg(args, 1, "DistanceFromCharacter")?;
            let at = character_root(world(lua)?.d(), this);
            one(Value::Number(
                at.map(|a| (a - p).length() as f64).unwrap_or(0.0),
            ))
        }
        "JSONEncode" => {
            let j = to_json(&arg(args, 1), 0)?;
            one(Value::String(lua.create_string(
                serde_json::to_string(&j).map_err(|e| rt_err(e.to_string()))?,
            )?))
        }
        "JSONDecode" => {
            let s = arg_str(args, 1, "string")?;
            let j: serde_json::Value =
                serde_json::from_str(&s).map_err(|e| rt_err(format!("Can't parse JSON: {e}")))?;
            one(from_json(lua, &j)?)
        }
        "GenerateGUID" => {
            let g = guid();
            let wrap = !matches!(arg(args, 1), Value::Boolean(false));
            one(Value::String(lua.create_string(if wrap {
                g
            } else {
                g.trim_matches(['{', '}']).to_string()
            })?))
        }
        "UrlEncode" => {
            let s = arg_str(args, 1, "string")?;
            let enc: String = s
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            one(Value::String(lua.create_string(enc)?))
        }
        "GetMinutesAfterMidnight" => one(Value::Number(clock_hours(lua, this)? as f64 * 60.0)),
        "SetMinutesAfterMidnight" => {
            let m = num(&arg(args, 1)).ok_or_else(|| rt_err("SetMinutesAfterMidnight: number expected"))?;
            set_clock(lua, this, m as f32 / 60.0)?;
            none()
        }
        "GetMouseLocation" => {
            let xy = world(lua)?.input.mouse_xy;
            one(Value::UserData(
                lua.create_userdata(LVector2(glam::Vec2::new(xy[0], xy[1])))?,
            ))
        }
        "IsMouseButtonPressed" => one(Value::Boolean(false)),
        "GetKeysPressed" => one(Value::Table(lua.create_table()?)),
        "GetCore" => one(Value::Nil),
        "TeleportAsync" => {
            world(lua)?.output.push((
                Level::Warn,
                format!("{name}: teleporting to other places isn't available in rustudio"),
            ));
            none()
        }
        "RemoveDefaultLoadingScreen" => none(),
        "RegisterKeyframeSequence" | "RegisterActiveKeyframeSequence" => {
            // A temporary content id for the sequence (Studio hands out a hash id).
            let seq = opt_inst(&arg(args, 1))
                .ok_or_else(|| rt_err(format!("{name}: KeyframeSequence expected")))?;
            let id = format!("rbxtemp://{}", sound_key_of(seq));
            one(Value::String(lua.create_string(id)?))
        }
        "GetKeyframeSequenceAsync" => {
            let r = create_any(lua, "KeyframeSequence")?;
            one(inst_value(lua, r)?)
        }
        "CreateMeshPartAsync" => {
            // A MeshPart showing the mesh, sized to the mesh's own bounds.
            // A Content: a URI, or an EditableMesh (editable.rs snapshots it
            // as a generated mesh, sized to its bounds).
            let mut editable_size = None;
            let content = match arg(args, 1) {
                Value::String(s) => s.to_str()?.to_string(),
                Value::Number(n) => format!("rbxassetid://{}", n as u64),
                v => match opt_ud::<super::editable::LContent>(&v) {
                    Some(c) => match super::editable::mesh_part_source(&c.0) {
                        Some(r) => {
                            let (id, size) = r?;
                            editable_size = Some(size);
                            id
                        }
                        None => c.0.as_uri().map(str::to_string).ok_or_else(|| rt_err("CreateMeshPartAsync: empty Content"))?,
                    },
                    None => return Err(rt_err("CreateMeshPartAsync: mesh id expected")),
                },
            };
            let size = editable_size.unwrap_or_else(|| {
                crate::assets::asset_ref(&content)
                    .and_then(|a| crate::assets::local_asset_path(a.id))
                    .and_then(|p| std::fs::read(p).ok())
                    .and_then(|b| crate::mesh::parse(&b).ok())
                    .map(|m| (m.max - m.min).max(Vec3::splat(0.05)))
                    .unwrap_or(Vec3::ONE)
            });
            // AssetService takes an options table (CollisionFidelity,
            // RenderFidelity); InsertService the two enums in turn (API dump).
            let opts = arg(args, 2);
            let fidelity = |key: &str| -> Option<u32> {
                opt_ud::<LEnumItem>(&ud_value(&opts, key)).map(|e| e.value)
            };
            let (collision, render) = match opt_ud::<LEnumItem>(&opts) {
                Some(c) => (Some(c.value), opt_ud::<LEnumItem>(&arg(args, 3)).map(|e| e.value)),
                None => (fidelity("CollisionFidelity"), fidelity("RenderFidelity")),
            };
            let r = create_any(lua, "MeshPart")?;
            {
                let mut w = world(lua)?;
                if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
                    i.properties
                        .insert(ustr("MeshId"), Variant::ContentId(content.clone().into()));
                    i.properties.insert(
                        ustr("Size"),
                        Variant::Vector3(Vector3::new(size.x, size.y, size.z)),
                    );
                    i.properties.insert(
                        ustr("MeshSize"),
                        Variant::Vector3(Vector3::new(size.x, size.y, size.z)),
                    );
                    if let Some(c) = collision {
                        i.properties.insert(
                            ustr("CollisionFidelity"),
                            Variant::Enum(rbx_dom_weak::types::Enum::from_u32(c)),
                        );
                    }
                    if let Some(c) = render {
                        i.properties.insert(
                            ustr("RenderFidelity"),
                            Variant::Enum(rbx_dom_weak::types::Enum::from_u32(c)),
                        );
                    }
                }
            }
            one(inst_value(lua, r)?)
        }
        "GetHumanoidDescriptionFromUserId" | "GetHumanoidDescriptionFromOutfitId" => {
            let r = create_any(lua, "HumanoidDescription")?;
            // The signed-in person's own user id: their avatar, as a
            // signed-in Studio fetches it (body colours, scales; the model
            // made from it is dressed). Others / offline: a blank description
            // (black body), as the offline Studio answers.
            let uid = num(&arg(args, 1)).unwrap_or(-1.0);
            if name == "GetHumanoidDescriptionFromUserId" {
                let mut w = world(lua)?;
                if let Some(look) = w.avatar.clone().filter(|l| l.user_id as f64 == uid) {
                    let c = look.colors;
                    let d = w.dm();
                    if let Some(i) = d.dom.get_by_ref_mut(r) {
                        let col = |v: [u8; 3]| Variant::Color3(rbx_types::Color3::new(v[0] as f32 / 255.0, v[1] as f32 / 255.0, v[2] as f32 / 255.0));
                        for (k, v) in [("HeadColor", c.head), ("TorsoColor", c.torso), ("LeftArmColor", c.left_arm),
                            ("RightArmColor", c.right_arm), ("LeftLegColor", c.left_leg), ("RightLegColor", c.right_leg)] {
                            i.properties.insert(ustr(k), col(v));
                        }
                        let sc = look.scales;
                        for (k, v) in [("HeightScale", sc.height), ("WidthScale", sc.width), ("DepthScale", sc.depth), ("HeadScale", sc.head)] {
                            i.properties.insert(ustr(k), Variant::Float32(v));
                        }
                    }
                    w.avatar_descs.insert(r);
                }
            }
            one(inst_value(lua, r)?)
        }
        "CreateHumanoidModelFromDescription" | "CreateHumanoidModelFromUserId" => {
            let r15 = match arg(args, 2) {
                v @ Value::UserData(_) => opt_ud::<LEnumItem>(&v).is_none_or(|e| e.name != "R6"),
                _ => true,
            };
            let rig = if r15 {
                crate::rig::RigType::R15
            } else {
                crate::rig::RigType::R6
            };
            // The description's body colours and scales. A new
            // HumanoidDescription's colours are black: Studio's offline
            // lobby avatar (Place A, from Instance.new("HumanoidDescription")) has
            // every body part Color (0, 0, 0) (tools/fidelity place_a capture).
            let desc = if name == "CreateHumanoidModelFromDescription" {
                opt_inst(&arg(args, 1))
            } else {
                None
            };
            let (colors, scales) = {
                let w = world(lua)?;
                let d = w.d();
                let color = |k: &str| match desc.and_then(|r| d.prop(r, k)) {
                    Some(Variant::Color3(c)) => [c.r, c.g, c.b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8),
                    Some(Variant::Color3uint8(c)) => [c.r, c.g, c.b],
                    _ => [0, 0, 0],
                };
                let scale = |k: &str| match desc.and_then(|r| d.prop(r, k)) {
                    Some(Variant::Float32(v)) => v,
                    Some(Variant::Float64(v)) => v as f32,
                    _ => 1.0,
                };
                (
                    crate::rig::BodyColors {
                        head: color("HeadColor"),
                        torso: color("TorsoColor"),
                        left_arm: color("LeftArmColor"),
                        right_arm: color("RightArmColor"),
                        left_leg: color("LeftLegColor"),
                        right_leg: color("RightLegColor"),
                    },
                    crate::rig::Scales {
                        height: scale("HeightScale"),
                        width: scale("WidthScale"),
                        depth: scale("DepthScale"),
                        head: scale("HeadScale"),
                    },
                )
            };
            let (colors, scales) = if desc.is_some() {
                (colors, scales)
            } else {
                (Default::default(), Default::default())
            };
            // Roblox's rig has its Animator under the Humanoid already
            // (Place D's second admin script preview: humanoid:WaitForChild("Animator")
            // warned of an infinite yield).
            let humanoid = InstanceBuilder::new("Humanoid")
                .with_name("Humanoid")
                .with_child(InstanceBuilder::new("Animator").with_name("Animator"));
            let r = crate::rig::insert_character(
                &mut world(lua)?.dm().dom,
                Ref::none(),
                "Player",
                rig,
                &colors,
                &scales,
                humanoid,
            );
            {
                let mut w = world(lua)?;
                if let Some(look) = desc.filter(|d| w.avatar_descs.contains(d)).and(w.avatar.clone()) {
                    crate::avatar::dress(w.dm(), r, &look);
                }
            }
            one(inst_value(lua, r)?)
        }
        "GetLastInputType" => match LEnumItem::from_name("UserInputType", "Keyboard") {
            Some(i) => one(crate::ltypes::enum_value(lua, i)?),
            None => one(Value::Nil),
        },
        // The pads the session sees (play.rs gamepad_devices).
        "GetConnectedGamepads" | "GetNavigationGamepads" => {
            let n = world(lua)?.input.gamepads;
            let t = lua.create_table()?;
            for i in 0..n {
                if let Some(e) = LEnumItem::from_name("UserInputType", &format!("Gamepad{}", i + 1)) {
                    t.set(i + 1, crate::ltypes::enum_value(lua, e)?)?;
                }
            }
            one(Value::Table(t))
        }
        "GetGamepadConnected" => {
            let n = world(lua)?.input.gamepads;
            let slot = opt_ud::<LEnumItem>(&arg(args, 1))
                .and_then(|e| e.name.strip_prefix("Gamepad").and_then(|k| k.parse::<usize>().ok()));
            one(Value::Boolean(slot.is_some_and(|k| k >= 1 && k <= n)))
        }
        "FillBlock" | "FillBall" | "FillCylinder" | "FillWedge" | "FillRegion" | "Clear"
        | "ReplaceMaterial" | "SetMaterialColor" | "GetMaterialColor" | "WriteVoxels" | "ReadVoxels"
        | "WriteVoxelChannels" | "ReadVoxelChannels" | "CopyRegion" | "PasteRegion" | "CellCenterToWorld"
        | "CellCornerToWorld" | "WorldToCell" | "WorldToCellPreferSolid" | "WorldToCellPreferEmpty"
            if inst_meta(lua, this)?.0 == "Terrain" =>
        {
            terrain_call(lua, name, this, args)
        }
        "SetCell" => {
            // (no behaviour: counted against scripting_api's coverage)
            crate::telemetry::gap(lua, "call-unimplemented", "Terrain", name);
            let first = !world(lua)?
                .output
                .iter()
                .any(|(_, t)| t.starts_with("Terrain editing"));
            if first {
                world(lua)?.output.push((Level::Warn, format!("Terrain editing from scripts (Terrain:{name}) isn't supported in rustudio yet; the terrain stays as saved")));
            }
            none()
        }
        // StudioTestService:GetTestArgs: the arguments of a test run started
        // with ExecuteRunModeAsync / ExecutePlayModeAsync; a plain Play has
        // none (nil), which is what Place A's DevTest and Progression check for.
        "GetTestArgs" => none(),
        _ => {
            let class = inst_meta(lua, this)?.0;
            super::stubs::unimplemented_call(lua, this, &class, name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A part far out saturates its cell indices; the width must not wrap
    /// to a small count (that walked billions of cells: a live place freeze).
    #[test]
    fn query_grid_far_part_goes_big() {
        let (a, b) = QueryGrid::cell_range(Vec3::splat(-4.0e10), Vec3::splat(1.0));
        assert_eq!(a[0], i32::MIN);
        // the old i32 width: wraps instead of being huge
        assert!(b[0].wrapping_sub(a[0]).wrapping_add(1) <= mech::QUERY_GRID_MAX_CELLS as i32);
        assert!(QueryGrid::cells_in(a, b) > mech::QUERY_GRID_MAX_CELLS as i64);
        let (a, b) = QueryGrid::cell_range(Vec3::splat(-4.0e10), Vec3::splat(4.0e10));
        assert!(QueryGrid::cells_in(a, b) > mech::QUERY_GRID_MAX_CELLS as i64);
        assert_eq!(QueryGrid::cells_in([0, 0, 0], [1, 1, 1]), 8);
    }

    fn run(src: &str) -> (Vec<String>, crate::terrain::Voxels) {
        let mut d = Document::new_place();
        let sss = d.find_child(d.dom.root_ref(), "ServerScriptService").unwrap();
        let s = d.dom.insert(sss, InstanceBuilder::new("Script").with_name("T").with_property("Source", src.to_string()));
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        for _ in 0..3 {
            rt.step(1.0 / 60.0);
        }
        let w = rt.leave();
        let v = match crate::terrain::Voxels::from_dom(&w.d().dom) {
            Some(Ok(v)) => v,
            _ => crate::terrain::Voxels::empty(),
        };
        (w.output.into_iter().map(|(_, t)| t).collect(), v)
    }

    /// Round 25: CopyRegion / PasteRegion, ReadVoxelChannels /
    /// WriteVoxelChannels and the cell conversions.
    #[test]
    fn terrain_regions_and_channels() {
        let (out, v) = run(r#"
local t = workspace.Terrain
t:FillBlock(CFrame.new(6, 6, 6), Vector3.new(8, 8, 8), Enum.Material.Rock)
local r = t:CopyRegion(Region3int16.new(Vector3int16.new(0, 0, 0), Vector3int16.new(3, 3, 3)))
print(r.ClassName, tostring(r.SizeInCells), r.IsSmooth)
t:PasteRegion(r, Vector3int16.new(10, 0, 0), true)
local _, a = t:ReadVoxels(Region3.new(Vector3.new(0, 0, 0), Vector3.new(16, 16, 16)), 4)
local m, b = t:ReadVoxels(Region3.new(Vector3.new(40, 0, 0), Vector3.new(56, 16, 16)), 4)
local same, rock = true, 0
for x = 1, 4 do for y = 1, 4 do for z = 1, 4 do
	same = same and a[x][y][z] == b[x][y][z]
	if m[x][y][z] == Enum.Material.Rock then rock += 1 end
end end end
print("paste", same, rock)
t:WriteVoxelChannels(Region3.new(Vector3.new(0, 40, 0), Vector3.new(8, 44, 4)), 4, {
	SolidMaterial = {{{Enum.Material.Air}}, {{Enum.Material.Sand}}},
	SolidOccupancy = {{{0}}, {{0.5}}},
	LiquidOccupancy = {{{1}}, {{0.5}}},
})
local ch = t:ReadVoxelChannels(Region3.new(Vector3.new(0, 40, 0), Vector3.new(8, 44, 4)), 4, {"SolidMaterial", "LiquidOccupancy"})
print("channels", ch.SolidMaterial[1][1][1].Name, ch.SolidMaterial[2][1][1].Name, ch.LiquidOccupancy[1][1][1], ch.SolidOccupancy == nil, tostring(ch.Size))
print("cells", tostring(t:CellCenterToWorld(1, 2, 3)), tostring(t:CellCornerToWorld(1, 2, 3)), tostring(t:WorldToCell(Vector3.new(5, -1, 9))))
local empty = t:CopyRegion(Region3int16.new(Vector3int16.new(100, 100, 100), Vector3int16.new(101, 101, 101)))
t:PasteRegion(empty, Vector3int16.new(10, 0, 0), false)
t:PasteRegion(empty, Vector3int16.new(0, 0, 0), true)
local all = t:CopyRegion(t.MaxExtents)
print("max", tostring(all.SizeInCells))
"#);
        let text = out.join("\n");
        assert!(text.contains("TerrainRegion 4, 4, 4 true"), "{text}");
        assert!(text.contains("paste true 27"), "{text}");
        assert!(text.contains("channels Air Sand 1 true 2, 1, 1"), "{text}");
        assert!(text.contains("cells 6, 10, 14 4, 8, 12 1, -1, 2"), "{text}");
        assert!(text.contains("max 64001, 64001, 64001"), "{text}");
        let rock = crate::terrain::material_index("Rock").unwrap();
        // pasteEmptyCells cleared the original's 2³ corner; the copy stayed.
        assert_eq!(v.get(0, 0, 0).0, 0);
        assert_eq!(v.get(2, 2, 2).0, rock);
        assert_eq!(v.get(11, 1, 1).0, rock);
        assert_eq!(v.get(0, 10, 0).0, crate::terrain::WATER);
        assert!((v.water(1, 10, 0) - 0.5).abs() < 0.01, "shoreline water {}", v.water(1, 10, 0));
    }
}
