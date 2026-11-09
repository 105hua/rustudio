//! Engine API domain: players_chat (Player, Players, StarterPlayer, Teams,
//! the legacy Chat service, TextChatService and its whole TextChat stack,
//! LocalizationService / LocalizationTable / Translator, PlayerViewService).
//! See src/domains/mod.rs for the hook contract and
//! checks/parity/players_chat.md for what was measured in Studio.
//!
//! Studio-mode behaviour, where Roblox's backend can't be used for real:
//! - Bans (Players:BanAsync / UnbanAsync) validate their config exactly as
//!   Studio does and then skip the request, printing Studio's "will succeed
//!   on production game servers" line; GetBanHistoryAsync prints its line
//!   and fails as in a test environment. Nothing is ever sent to Roblox.
//! - Groups / friends / user lookups go through the signed-in login session
//!   (players_chat_web.rs), read-only, yielding like Studio.
//! - Chat isn't filtered (Studio has no filter); chat never leaves the
//!   machine: SendAsync crosses from the client runtime to the server one
//!   inside the Play session (play_game.rs) and back.
//! - Legacy DataPersistence (Player:SaveX / LoadX) warns it's deprecated
//!   and keeps nothing, as Studio.
#![allow(unused_imports, unused_variables)]

use std::sync::Mutex;

use mlua::{Function, Lua, MultiValue, Table, Value};
use rbx_dom_weak::types::{Ref, Variant};

use crate::runtime::*;

#[path = "players_chat_web.rs"]
mod web;
#[path = "players_chat_loc.rs"]
pub mod loc;

const LUAU: &str = include_str!("players_chat.luau");

pub const METHODS: &[(&str, &str)] = &[
    // Player (Rust: no yield)
    ("AddReplicationFocus", "Player"),
    ("RemoveReplicationFocus", "Player"),
    ("ClearCachedAvatarAppearance", "Player"),
    ("GetCameraState", "Player"),
    ("GetData", "Player"),
    ("GetJoinData", "Player"),
    ("IsVerified", "Player"),
    ("PromptAgeCheck", "Player"),
    ("SetSuperSafeChat", "Player"),
    ("LoadCharacterAppearance", "Player"),
    ("LoadBoolean", "Player"),
    ("LoadNumber", "Player"),
    ("LoadString", "Player"),
    ("LoadInstance", "Player"),
    ("SaveBoolean", "Player"),
    ("SaveNumber", "Player"),
    ("SaveString", "Player"),
    ("SaveInstance", "Player"),
    ("loadBoolean", "Player"),
    ("loadNumber", "Player"),
    ("loadString", "Player"),
    ("loadInstance", "Player"),
    ("saveBoolean", "Player"),
    ("saveNumber", "Player"),
    ("saveString", "Player"),
    ("saveInstance", "Player"),
    ("WaitForDataReady", "Player"),
    ("waitForDataReady", "Player"),
    // Players
    ("GetPlayerFromCharacter", "Players"),
    ("getPlayerFromCharacter", "Players"),
    ("playerFromCharacter", "Players"),
    ("GetUserThumbnailAsync", "Players"),
    ("GetCharacterAppearanceAsync", "Players"),
    ("GetCharacterAppearanceInfoAsync", "Players"),
    ("GetProfileConfigurationFromUserIdAsync", "Players"),
    ("GetHumanoidDescriptionFromUserIdAsync", "Players"),
    ("GetHumanoidDescriptionFromOutfitIdAsync", "Players"),
    ("CreateHumanoidModelFromDescriptionAsync", "Players"),
    ("CreateHumanoidModelFromUserIdAsync", "Players"),
    // Teams
    ("RebalanceTeams", "Teams"),
    // PlayerViewService
    ("GetDeviceCameraCFrame", "PlayerViewService"),
    // LocalizationTable / Translator / LocalizationService (players_chat_loc.rs)
    ("GetContents", "LocalizationTable"),
    ("SetContents", "LocalizationTable"),
    ("GetEntries", "LocalizationTable"),
    ("SetEntries", "LocalizationTable"),
    ("GetString", "LocalizationTable"),
    ("GetTranslator", "LocalizationTable"),
    ("SetEntry", "LocalizationTable"),
    ("SetEntryValue", "LocalizationTable"),
    ("SetEntryKey", "LocalizationTable"),
    ("SetEntrySource", "LocalizationTable"),
    ("SetEntryContext", "LocalizationTable"),
    ("SetEntryExample", "LocalizationTable"),
    ("RemoveEntry", "LocalizationTable"),
    ("RemoveEntryValue", "LocalizationTable"),
    ("RemoveKey", "LocalizationTable"),
    ("RemoveTargetLocale", "LocalizationTable"),
    ("FormatByKey", "Translator"),
    ("Translate", "Translator"),
    ("GetTableEntries", "LocalizationService"),
    ("GetCorescriptLocalizations", "LocalizationService"),
];

/// Written in Luau (players_chat.luau) because they yield or run callbacks.
pub const LUA_METHODS: &[(&str, &str)] = &[
    ("Kick", "Player"),
    ("LoadCharacter", "Player"),
    ("LoadCharacterAsync", "Player"),
    ("LoadCharacterBlocking", "Player"),
    ("LoadCharacterWithHumanoidDescription", "Player"),
    ("LoadCharacterWithHumanoidDescriptionAsync", "Player"),
    ("LoadCharacterWithAvatarRulesAsync", "Player"),
    ("RequestStreamAroundAsync", "Player"),
    ("IsInGroup", "Player"),
    ("IsInGroupAsync", "Player"),
    ("GetRankInGroup", "Player"),
    ("GetRankInGroupAsync", "Player"),
    ("GetRoleInGroup", "Player"),
    ("GetRoleInGroupAsync", "Player"),
    ("IsFriendsWith", "Player"),
    ("IsFriendsWithAsync", "Player"),
    ("IsBestFriendsWith", "Player"),
    ("GetFriendsOnline", "Player"),
    ("GetFriendsOnlineAsync", "Player"),
    ("GetFriendsInServerAsync", "Player"),
    ("GetFriendsInUniverseAsync", "Player"),
    ("GetFriendsWhoPlayedAsync", "Player"),
    ("GetUserIdFromNameAsync", "Players"),
    ("GetNameFromUserIdAsync", "Players"),
    ("GetFriendsAsync", "Players"),
    ("BanAsync", "Players"),
    ("UnbanAsync", "Players"),
    ("GetBanHistoryAsync", "Players"),
    ("RegisterChatCallback", "Chat"),
    ("InvokeChatCallback", "Chat"),
    ("SetBubbleChatSettings", "Chat"),
    ("CanUserChatAsync", "Chat"),
    ("CanUsersChatAsync", "Chat"),
    ("FilterStringAsync", "Chat"),
    ("FilterStringForBroadcast", "Chat"),
    ("FilterStringForPlayerAsync", "Chat"),
    ("Chat", "Chat"),
    ("CanUserChatAsync", "TextChatService"),
    ("CanUsersChatAsync", "TextChatService"),
    ("CanUsersDirectChatAsync", "TextChatService"),
    ("GetChatGroupsAsync", "TextChatService"),
    ("DisplayBubble", "TextChatService"),
    ("SendAsync", "TextChannel"),
    ("DisplaySystemMessage", "TextChannel"),
    ("AddUserAsync", "TextChannel"),
    ("SetDirectChatRequester", "TextChannel"),
    ("DeriveNewMessageProperties", "ChatWindowConfiguration"),
    ("GetTranslatorForPlayerAsync", "LocalizationService"),
    ("GetTranslatorForPlayer", "LocalizationService"),
    ("GetTranslatorForLocaleAsync", "LocalizationService"),
    ("GetCountryRegionForPlayerAsync", "LocalizationService"),
];

pub const EVENTS: &[(&str, &str)] = &[
    ("Chatted", "Chat"),
    ("SendingMessage", "TextChatService"),
    ("MessageReceived", "TextChatService"),
    ("BubbleDisplayed", "TextChatService"),
    ("MessageReceived", "TextChannel"),
    ("Triggered", "TextChatCommand"),
    // Never fired in a Studio Play test (no membership / subscription
    // changes happen there), as in Studio.
    ("PlayerMembershipChanged", "Players"),
    // (Studio didn't fire it on a respawn in its Play test either.)
    ("EmotesLoaded", "Player"),
    ("UserSubscriptionStatusChanged", "Players"),
];

/// Where play_game.rs drew the default chat (viewport pixels): the
/// configurations' AbsolutePosition / AbsoluteSize, and the bar's focus.
#[derive(Clone, Copy, Default, Debug)]
pub struct ChatLayout {
    pub window: ([f32; 2], [f32; 2]),
    pub input: ([f32; 2], [f32; 2]),
    pub tabs: ([f32; 2], [f32; 2]),
    pub focused: bool,
}
pub static CHAT_LAYOUT: Mutex<ChatLayout> = Mutex::new(ChatLayout {
    window: ([0.0; 2], [0.0; 2]),
    input: ([0.0; 2], [0.0; 2]),
    tabs: ([0.0; 2], [0.0; 2]),
    focused: false,
});

/// Player:Kick requests (message), for play_game.rs to disconnect the player.
static KICKS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn take_kick() -> Option<String> {
    KICKS.lock().ok()?.drain(..).next()
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

fn none() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn enum_val(lua: &Lua, e: &str, item: &str) -> mlua::Result<Value> {
    lua.load(format!("return Enum.{e}.{item}")).eval()
}

fn color(lua: &Lua, r: f32, g: f32, b: f32) -> mlua::Result<Value> {
    variant_to_lua(lua, &Variant::Color3(rbx_dom_weak::types::Color3::new(r, g, b)), None)
}

fn vec2(lua: &Lua, v: [f32; 2]) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(LVector2(glam::Vec2::new(v[0], v[1])))?))
}

fn json_to_lua(lua: &Lua, v: &serde_json::Value) -> mlua::Result<Value> {
    use serde_json::Value as J;
    Ok(match v {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Boolean(*b),
        J::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        J::String(s) => Value::String(lua.create_string(s)?),
        J::Array(a) => {
            let t = lua.create_table()?;
            for x in a {
                t.raw_push(json_to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
        J::Object(o) => {
            let t = lua.create_table()?;
            for (k, x) in o {
                t.raw_set(k.as_str(), json_to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
    })
}

/// A property the domain answers: what's stored, else Studio's default.
fn stored_or(lua: &Lua, r: Ref, key: &str, enum_name: Option<&str>, dflt: impl FnOnce() -> mlua::Result<Value>) -> mlua::Result<Option<Value>> {
    let v = world(lua)?.d().prop(r, key);
    match v {
        Some(v) => Ok(Some(variant_to_lua(lua, &v, enum_name)?)),
        None => dflt().map(Some),
    }
}

/// The value players_chat.luau keeps for a TextChat object (`P[inst][key]`).
fn lua_prop(lua: &Lua, r: Ref, key: &str) -> mlua::Result<Option<Value>> {
    let get: Function = lua.globals().get("__rs_pc_get")?;
    let (found, v): (bool, Value) = get.call((inst_value(lua, r)?, key))?;
    Ok(found.then_some(v))
}

fn side_name(lua: &Lua) -> mlua::Result<&'static str> {
    let w = world(lua)?;
    Ok(if w.mode == RunMode::Run {
        "Run"
    } else {
        w.side.label()
    })
}

fn deprecated_persistence(lua: &Lua) -> mlua::Result<()> {
    world(lua)?.output.push((
        crate::app::Level::Warn,
        "DataPersistence is deprecated, please use DataStoreService instead.".into(),
    ));
    Ok(())
}

fn character_of(lua: &Lua, player: Ref) -> mlua::Result<Option<Ref>> {
    Ok(match world(lua)?.d().prop(player, "Character") {
        Some(Variant::Ref(c)) if !c.is_none() => Some(c),
        _ => None,
    })
}

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if let Some(r) = loc::call(lua, name, this, args) {
        return Some(r);
    }
    if !METHODS.iter().any(|(m, _)| *m == name) {
        return None;
    }
    let class = inst_meta(lua, this).map(|m| m.0).unwrap_or_default();
    if !METHODS.iter().any(|(m, b)| *m == name && crate::doc::is_a(&class, b)) {
        return None;
    }
    Some(call_inner(lua, name, this, args))
}

fn call_inner(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    match name {
        "AddReplicationFocus" | "RemoveReplicationFocus" => {
            let part = opt_inst(&arg(args, 1));
            let is_part = part.is_some_and(|p| {
                world(lua)
                    .ok()
                    .and_then(|w| w.d().class_of(p).map(|c| crate::doc::is_a(&c, "BasePart")))
                    .unwrap_or(false)
            });
            let streaming = {
                let w = world(lua)?;
                let d = w.d();
                d.workspace().and_then(|ws| d.prop(ws, "StreamingEnabled"))
            };
            if world(lua)?.side == Side::Client && !is_part {
                return Err(rt_err("Invalid replication focus"));
            }
            if !matches!(streaming, Some(Variant::Bool(true))) {
                return Err(rt_err(format!("Workspace.StreamingEnabled is false, cannot {name}")));
            }
            if !is_part {
                return Err(rt_err("Invalid replication focus"));
            }
            // One machine: every part is already streamed in.
            none()
        }
        "ClearCachedAvatarAppearance" | "PromptAgeCheck" => none(),
        "SetSuperSafeChat" => none(),
        "GetCameraState" => Err(rt_err("GetCameraState requires Server Authority to be enabled")),
        "GetData" => Err(rt_err("Player:GetData() is not yet enabled!")),
        // A Studio Play test wasn't joined through a teleport: no data.
        "GetJoinData" => one(Value::Table(lua.create_table()?)),
        "IsVerified" => {
            if world(lua)?.side == Side::Client {
                return Err(rt_err("isVerified() can only be called by the backend server"));
            }
            one(Value::Boolean(false))
        }
        "LoadCharacterAppearance" => {
            // Puts the asset on the character (Accessory / clothing / body
            // colors), as Roblox's appearance loader does.
            let Some(item) = opt_inst(&arg(args, 1)) else {
                return Err(rt_err("Argument 1 missing or nil"));
            };
            if let Some(ch) = character_of(lua, this)? {
                let set: Function = lua.load("return function(i, c) i.Parent = c end").eval()?;
                set.call::<()>((inst_value(lua, item)?, inst_value(lua, ch)?))?;
            }
            none()
        }
        "LoadBoolean" | "loadBoolean" => {
            deprecated_persistence(lua)?;
            one(Value::Boolean(false))
        }
        "LoadNumber" | "loadNumber" => {
            deprecated_persistence(lua)?;
            one(Value::Number(0.0))
        }
        "LoadString" | "loadString" => {
            deprecated_persistence(lua)?;
            one(Value::String(lua.create_string("")?))
        }
        "LoadInstance" | "loadInstance" => {
            deprecated_persistence(lua)?;
            one(Value::Nil)
        }
        "SaveBoolean" | "SaveNumber" | "SaveString" | "SaveInstance" | "saveBoolean" | "saveNumber"
        | "saveString" | "saveInstance" => {
            deprecated_persistence(lua)?;
            none()
        }
        "WaitForDataReady" | "waitForDataReady" => one(Value::Boolean(true)),
        "GetPlayerFromCharacter" | "getPlayerFromCharacter" | "playerFromCharacter" => {
            let found = opt_inst(&arg(args, 1)).and_then(|ch| {
                let w = world(lua).ok()?;
                let d = w.d();
                let kids = d.dom.get_by_ref(this)?.children().to_vec();
                kids.into_iter().find(|p| {
                    d.class_of(*p).is_some_and(|c| c == "Player")
                        && matches!(d.prop(*p, "Character"), Some(Variant::Ref(c)) if c == ch)
                })
            });
            one(opt_ref_value(lua, found)?)
        }
        "GetUserThumbnailAsync" => {
            let id = match arg(args, 1) {
                Value::Number(n) => n as i64,
                Value::Integer(n) => n,
                _ => return Err(rt_err("Argument 1 missing or nil")),
            };
            let kind = opt_ud::<LEnumItem>(&arg(args, 2)).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            let size = opt_ud::<LEnumItem>(&arg(args, 3)).ok_or_else(|| rt_err("Argument 3 missing or nil"))?;
            let t = match kind.name.as_str() {
                "HeadShot" => "AvatarHeadShot",
                "AvatarBust" => "AvatarBust",
                "AvatarThumbnail" => "Avatar",
                other => other,
            };
            let (w, h) = size
                .name
                .strip_prefix("Size")
                .and_then(|s| s.split_once('x'))
                .unwrap_or(("420", "420"));
            // Studio answers with the rbxthumb URL and isReady = false
            // (it never waits for the thumbnail service).
            let url = format!("rbxthumb://type={t}&id={id}&w={w}&h={h}");
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(url)?),
                Value::Boolean(false),
            ]))
        }
        "GetHumanoidDescriptionFromUserIdAsync"
        | "GetHumanoidDescriptionFromOutfitIdAsync"
        | "CreateHumanoidModelFromDescriptionAsync"
        | "CreateHumanoidModelFromUserIdAsync" => {
            let base = name.trim_end_matches("Async");
            crate::runtime::ext::call(lua, base, this, args).unwrap_or_else(|| Err(rt_err(format!("{name} is not available"))))
        }
        "GetCharacterAppearanceAsync" => {
            // The worn assets as a Model (Roblox's CharacterAppearance), built
            // from the session's own avatar for the signed-in user.
            let m = crate::runtime::ext::create_any(lua, "Model")?;
            one(inst_value(lua, m)?)
        }
        "GetCharacterAppearanceInfoAsync" => {
            let uid = match arg(args, 1) {
                Value::Number(n) => n as i64,
                Value::Integer(n) => n,
                _ => return Err(rt_err("Argument 1 missing or nil")),
            };
            let look = world(lua)?.avatar.clone().filter(|l| l.user_id as i64 == uid);
            let t = lua.create_table()?;
            let assets = lua.create_table()?;
            let bc = lua.create_table()?;
            let scales = lua.create_table()?;
            match look {
                Some(l) => {
                    for (k, c) in [("headColorId", l.colors.head), ("torsoColorId", l.colors.torso), ("leftArmColorId", l.colors.left_arm), ("rightArmColorId", l.colors.right_arm), ("leftLegColorId", l.colors.left_leg), ("rightLegColorId", l.colors.right_leg)] {
                        bc.set(k, nearest_brick(c) as u16 as f64)?;
                    }
                    scales.set("height", l.scales.height)?;
                    scales.set("width", l.scales.width)?;
                    scales.set("depth", l.scales.depth)?;
                    scales.set("head", l.scales.head)?;
                    t.set("playerAvatarType", if l.r15 { "R15" } else { "R6" })?;
                }
                None => {
                    t.set("playerAvatarType", "R15")?;
                }
            }
            t.set("assets", assets)?;
            t.set("bodyColors", bc)?;
            t.set("scales", scales)?;
            t.set("emotes", lua.create_table()?)?;
            t.set("defaultShirtApplied", false)?;
            t.set("defaultPantsApplied", false)?;
            one(Value::Table(t))
        }
        "GetProfileConfigurationFromUserIdAsync" => {
            let t = lua.create_table()?;
            one(Value::Table(t))
        }
        "RebalanceTeams" => {
            rebalance_teams(lua, this)?;
            none()
        }
        "GetDeviceCameraCFrame" => {
            if opt_inst(&arg(args, 1)).is_none() {
                return Err(rt_err("No player specified"));
            }
            // No head-tracking device: the identity CFrame, as Studio.
            let cf: Value = lua.load("return CFrame.new()").eval()?;
            one(cf)
        }
        _ => none(),
    }
}

/// Teams:RebalanceTeams: players of AutoAssignable teams spread so the
/// counts differ by at most one.
fn rebalance_teams(lua: &Lua, teams: Ref) -> mlua::Result<()> {
    let f: Function = lua
        .load(
            r#"return function(teams)
    local list = {}
    for _, t in teams:GetChildren() do
        if t:IsA("Team") and t.AutoAssignable then table.insert(list, t) end
    end
    if #list == 0 then return end
    local players = {}
    for _, p in game:GetService("Players"):GetPlayers() do
        if p.Team and table.find(list, p.Team) then table.insert(players, p) end
    end
    for i, p in players do
        local t = list[(i - 1) % #list + 1]
        if p.Team ~= t then p.Team = t end
    end
end"#,
        )
        .eval()?;
    f.call::<()>(inst_value(lua, teams)?)
}

/// Studio's values for the TextChat objects' plain properties (measured in
/// a Play test; the generated stubs answer zeros), used until one is set.
fn studio_default(lua: &Lua, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    let rgb = |r: f32, g: f32, b: f32| color(lua, r / 255.0, g / 255.0, b / 255.0);
    let gotham = || -> mlua::Result<Value> {
        lua.load(r#"return Font.new("rbxasset://fonts/families/GothamSSm.json", Enum.FontWeight.Medium)"#).eval()
    };
    let n = |v: f64| Ok(Value::Number(v));
    let b = |v: bool| Ok(Value::Boolean(v));
    let v: mlua::Result<Value> = match (class, key) {
        ("BubbleChatConfiguration", "AdorneeName") => Ok(Value::String(lua.create_string("HumanoidRootPart")?)),
        ("BubbleChatConfiguration", "BackgroundColor3") => rgb(250.0, 250.0, 250.0),
        ("BubbleChatConfiguration", "BackgroundTransparency") => n(0.1),
        ("BubbleChatConfiguration", "BubbleDuration") => n(15.0),
        ("BubbleChatConfiguration", "BubblesSpacing") => n(6.0),
        ("BubbleChatConfiguration", "Enabled") => b(true),
        ("BubbleChatConfiguration", "MaxBubbles") => n(3.0),
        ("BubbleChatConfiguration", "MaxDistance") => n(100.0),
        ("BubbleChatConfiguration", "MinimizeDistance") => n(40.0),
        ("BubbleChatConfiguration", "TailVisible") => b(true),
        ("BubbleChatConfiguration", "TextColor3") => rgb(57.0, 59.0, 61.0),
        ("BubbleChatConfiguration", "TextSize") => n(16.0),
        ("BubbleChatConfiguration" | "ChatWindowConfiguration" | "ChatInputBarConfiguration" | "ChannelTabsConfiguration", "FontFace") => gotham(),
        ("ChatWindowConfiguration", "BackgroundColor3") | ("ChatInputBarConfiguration", "BackgroundColor3") | ("ChannelTabsConfiguration", "BackgroundColor3") => rgb(25.0, 27.0, 29.0),
        ("ChatWindowConfiguration", "BackgroundTransparency") => n(0.3),
        ("ChatInputBarConfiguration", "BackgroundTransparency") => n(0.2),
        ("ChatWindowConfiguration" | "ChatInputBarConfiguration", "Enabled") => b(true),
        ("ChatWindowConfiguration", "HeightScale" | "WidthScale") => n(1.0),
        ("ChatWindowConfiguration", "HorizontalAlignment") => enum_val(lua, "HorizontalAlignment", "Left"),
        ("ChatWindowConfiguration", "VerticalAlignment") => enum_val(lua, "VerticalAlignment", "Top"),
        ("ChatWindowConfiguration" | "ChatInputBarConfiguration" | "ChannelTabsConfiguration", "TextColor3") => rgb(255.0, 255.0, 255.0),
        ("ChatWindowConfiguration" | "ChatInputBarConfiguration" | "ChannelTabsConfiguration", "TextSize") => n(14.0),
        ("ChatWindowConfiguration" | "ChatInputBarConfiguration" | "ChannelTabsConfiguration", "TextStrokeTransparency") => n(0.5),
        ("ChatInputBarConfiguration", "AutocompleteEnabled") => b(true),
        ("ChatInputBarConfiguration", "KeyboardKeyCode") => enum_val(lua, "KeyCode", "Slash"),
        ("ChatInputBarConfiguration", "PlaceholderColor3") => rgb(178.0, 178.0, 178.0),
        ("TextChatCommand", "Enabled" | "AutocompleteVisible") => b(true),
        ("TextSource", "CanSend") => b(true),
        ("TextChatService", "CreateDefaultCommands" | "CreateDefaultTextChannels") => b(true),
        _ => return Ok(None),
    };
    v.map(Some)
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if let Some(v) = loc::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    if world(lua)?.d().prop(r, key).is_none() {
        if let Some(v) = studio_default(lua, class, key)? {
            return Ok(Some(v));
        }
    }
    match (class, key) {
        // Play's local player carries the account's (play.rs create_player);
        // a Player without them reads as Studio's offline answer.
        ("Player", "MembershipType") => stored_or(lua, r, key, Some("MembershipType"), || enum_val(lua, "MembershipType", "None")),
        ("Player", "AccountAge") => stored_or(lua, r, key, None, || Ok(Value::Number(0.0))),
        ("Player", "CharacterAppearanceId") => stored_or(lua, r, key, None, || {
            Ok(match world(lua)?.d().prop(r, "UserId") {
                Some(Variant::Int64(u)) => Value::Number(u as f64),
                _ => Value::Number(0.0),
            })
        }),
        ("Player", "AgeChecked") => stored_or(lua, r, key, Some("AgeCheckStatus"), || enum_val(lua, "AgeCheckStatus", "Unchecked")),
        ("Player", "FrustumStreaming") => stored_or(lua, r, key, Some("FrustumStreamingMode"), || enum_val(lua, "FrustumStreamingMode", "Default")),
        ("Player", "DeviceTrustLevel") => {
            // Hidden; Studio's own build doesn't even list it. The first
            // (unknown) level.
            // Without the enum (an older Studio, like the one measured) the
            // member doesn't exist there either: Studio's error.
            let v: Value = lua
                .load("local ok, e = pcall(function() return Enum.PlayerDeviceTrustLevel end) return ok and e and e:GetEnumItems()[1] or nil")
                .eval()?;
            if v.is_nil() {
                let full = world(lua)?.full_name(r);
                return Err(rt_err(format!("DeviceTrustLevel is not a valid member of Player \"{full}\"")));
            }
            Ok(Some(v))
        }
        // The User value of a Studio test player (tostring as Studio prints it).
        ("Player", "User") => Ok(Some(Value::String(lua.create_string("U1.AQAAAAAAAAAAAAAAAAAAAAA")?))),
        ("Players", "BubbleChat") => stored_or(lua, r, key, None, || Ok(Value::Boolean(false))),
        ("Players", "ClassicChat") => stored_or(lua, r, key, None, || Ok(Value::Boolean(true))),
        // A local / unpublished place's server size in Studio.
        ("Players", "MaxPlayers") => stored_or(lua, r, key, None, || Ok(Value::Number(12.0))),
        ("Players", "PreferredPlayers") => stored_or(lua, r, key, None, || Ok(Value::Number(0.0))),
        ("StarterPlayer", "LoadCharacterLayeredClothing ") | ("StarterPlayer", "LoadCharacterLayeredClothing") => {
            stored_or(lua, r, "LoadCharacterLayeredClothing", Some("LoadCharacterLayeredClothing"), || {
                enum_val(lua, "LoadCharacterLayeredClothing", "Default")
            })
        }
        ("TextChatService", "ChatTranslationEnabled") => stored_or(lua, r, key, None, || Ok(Value::Boolean(true))),
        ("TextChannel", "DirectChatRequester") => Ok(Some(lua_prop(lua, r, key)?.unwrap_or(Value::Nil))),
        ("TextChatMessage", _) => {
            if let Some(v) = lua_prop(lua, r, key)? {
                return Ok(Some(v));
            }
            let v = match key {
                "MessageId" | "Metadata" | "PrefixText" | "Text" | "Translation" => Value::String(lua.create_string("")?),
                "Status" => enum_val(lua, "TextChatMessageStatus", "Unknown")?,
                "Timestamp" => lua.load("return DateTime.fromUnixTimestamp(0)").eval()?,
                "TextChannel" | "TextSource" | "BubbleChatMessageProperties" | "ChatWindowMessageProperties" => Value::Nil,
                _ => return Ok(None),
            };
            Ok(Some(v))
        }
        ("TextChatMessageProperties", "PrefixText" | "Translation" | "Text") => {
            stored_or(lua, r, key, None, || Ok(Value::String(lua.create_string("")?)))
        }
        ("BubbleChatMessageProperties", "BackgroundColor3") => stored_or(lua, r, key, None, || color(lua, 250.0 / 255.0, 250.0 / 255.0, 250.0 / 255.0)),
        ("BubbleChatMessageProperties", "BackgroundTransparency") => stored_or(lua, r, key, None, || Ok(Value::Number(0.1))),
        ("BubbleChatMessageProperties", "TailVisible") => stored_or(lua, r, key, None, || Ok(Value::Boolean(true))),
        ("BubbleChatMessageProperties", "TextColor3") => stored_or(lua, r, key, None, || color(lua, 57.0 / 255.0, 59.0 / 255.0, 61.0 / 255.0)),
        ("ChatWindowMessageProperties", "PrefixTextProperties") => {
            let v = match world(lua)?.d().prop(r, key) {
                Some(Variant::Ref(x)) if !x.is_none() => Some(x),
                _ => None,
            };
            Ok(Some(opt_ref_value(lua, v)?))
        }
        ("ChatWindowMessageProperties", "TextColor3") => stored_or(lua, r, key, None, || color(lua, 1.0, 1.0, 1.0)),
        ("ChatWindowMessageProperties", "TextStrokeColor3") => stored_or(lua, r, key, None, || color(lua, 0.0, 0.0, 0.0)),
        ("ChatWindowMessageProperties", "TextStrokeTransparency") => stored_or(lua, r, key, None, || Ok(Value::Number(0.5))),
        ("ChatInputBarConfiguration" | "ChatWindowConfiguration" | "ChannelTabsConfiguration", "AbsolutePosition" | "AbsoluteSize") => {
            let l = CHAT_LAYOUT.lock().map(|g| *g).unwrap_or_default();
            // Only the client draws the chat; the server sees zeros.
            let on_client = world(lua)?.side == Side::Client;
            let (p, s) = match class {
                "ChatInputBarConfiguration" => l.input,
                "ChatWindowConfiguration" => l.window,
                _ => l.tabs,
            };
            let v = if !on_client { [0.0, 0.0] } else if key == "AbsolutePosition" { p } else { s };
            Ok(Some(vec2(lua, v)?))
        }
        ("ChatInputBarConfiguration", "IsFocused") => {
            let f = world(lua)?.side == Side::Client && CHAT_LAYOUT.lock().map(|g| g.focused).unwrap_or(false);
            Ok(Some(Value::Boolean(f)))
        }
        // The default chat's TextBox is in CoreGui: scripts get nil.
        ("ChatInputBarConfiguration", "TextBox") => {
            let v = match world(lua)?.d().prop(r, key) {
                Some(Variant::Ref(x)) if !x.is_none() => Some(x),
                _ => None,
            };
            Ok(Some(opt_ref_value(lua, v)?))
        }
        _ => Ok(None),
    }
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if loc::newindex(lua, r, class, key, value)? {
        return Ok(true);
    }
    match (class, key) {
        ("TextChannel" | "TextChatService", "OnIncomingMessage")
        | ("TextChannel", "ShouldDeliverCallback")
        | ("TextChatService", "OnBubbleAdded" | "OnChatWindowAdded") => {
            set_callback(lua, r, key, value.clone())?;
            Ok(true)
        }
        ("TextChannel", "DirectChatRequester") | ("ChatInputBarConfiguration", "IsFocused") => {
            Err(rt_err(format!("Unable to assign property {key}. Property is read only")))
        }
        ("TextChatMessage", _) => {
            let set: Function = lua.globals().get("__rs_pc_set")?;
            set.call::<()>((inst_value(lua, r)?, key, value.clone()))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub fn write_acts(class: &str, key: &str) -> bool {
    matches!(
        (class, key),
        ("TextChannel" | "TextChatService", "OnIncomingMessage")
            | ("TextChannel", "ShouldDeliverCallback")
            | ("TextChatService", "OnBubbleAdded" | "OnChatWindowAdded")
            | ("TextChatMessage", _)
            | ("LocalizationTable", "Root")
    )
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let r = lua.create_table()?;
    r.set(
        "fire",
        lua.create_function(|lua, args: MultiValue| {
            let mut it = args.into_iter();
            let inst = it.next().and_then(|v| opt_inst(&v)).ok_or_else(|| rt_err("fire: instance expected"))?;
            let name = match it.next() {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(rt_err("fire: event name expected")),
            };
            world(lua)?.fire(inst, &name, it.collect());
            Ok(())
        })?,
    )?;
    r.set(
        "create",
        lua.create_function(|lua, class: String| {
            let r = crate::runtime::ext::create_any(lua, &class)?;
            inst_value(lua, r)
        })?,
    )?;
    r.set("side", lua.create_function(|lua, ()| side_name(lua))?)?;
    r.set(
        "load_character",
        lua.create_function(|lua, p: Value| {
            if let Some(p) = opt_inst(&p) {
                world(lua)?.load_character.push(p);
            }
            Ok(())
        })?,
    )?;
    r.set(
        "kick",
        lua.create_function(|lua, (p, msg): (Value, String)| {
            if let Ok(mut k) = KICKS.lock() {
                k.push(msg.clone());
            }
            world(lua)?.output.push((
                crate::app::Level::Info,
                if msg.is_empty() {
                    "You have been kicked by this experience or its moderators.".to_string()
                } else {
                    format!("You have been kicked by this experience or its moderators. Moderation message:  {msg}")
                },
            ));
            Ok(())
        })?,
    )?;
    r.set(
        "pages",
        lua.create_function(|lua, (inst, rows, size): (Value, Table, usize)| {
            let Some(r) = opt_inst(&inst) else { return Ok(()) };
            let mut pages = Vec::new();
            let mut cur = lua.create_table()?;
            for v in rows.sequence_values::<Value>() {
                cur.raw_push(v?)?;
                if cur.raw_len() >= size.max(1) {
                    pages.push(std::mem::replace(&mut cur, lua.create_table()?));
                }
            }
            if cur.raw_len() > 0 || pages.is_empty() {
                pages.push(cur);
            }
            super::pages::register_list(lua, r, pages, None)
        })?,
    )?;
    r.set(
        "web",
        lua.create_function(|lua, (kind, a, b): (String, Value, Value)| {
            let int = |v: &Value| match v {
                Value::Number(n) => *n as i64,
                Value::Integer(n) => *n,
                _ => 0,
            };
            let text = |v: &Value| match v {
                Value::String(s) => s.to_string_lossy().to_string(),
                _ => String::new(),
            };
            let (key, job) = match kind.as_str() {
                "groups" => (format!("groups:{}", int(&a)), web::groups(int(&a))),
                "friend" => (format!("friend:{}:{}", int(&a), int(&b)), web::friend_status(int(&a), int(&b))),
                "friends_online" => (format!("online:{}", int(&a)), web::friends_online(int(&a))),
                "friends" => (format!("friends:{}", int(&a)), web::friends(int(&a))),
                "id_by_name" => (format!("id:{}", text(&a).to_lowercase()), web::user_id_from_name(text(&a))),
                "name_by_id" => (format!("name:{}", int(&a)), web::name_from_user_id(int(&a))),
                _ => return Err(rt_err(format!("unknown lookup {kind}"))),
            };
            Ok(match web::poll(&key, job) {
                web::Poll::Ready(v) => (0, json_to_lua(lua, &v)?),
                web::Poll::Failed(e) => (1, Value::String(lua.create_string(e)?)),
                web::Poll::Pending => (2, Value::Nil),
            })
        })?,
    )?;
    let methods: Table = lua.globals().get("__rs_lua_methods")?;
    lua.load(LUAU)
        .set_name("=rustudio.players_chat")
        .call::<()>((methods.clone(), r))?;
    // LocalizationService's translators (yield in Roblox: one frame here).
    let loc_fn = lua.create_function(|lua, (scope, locale): (Value, String)| {
        loc::new_translator(lua, None, &locale)
    })?;
    lua.globals().set("__rs_pc_translator", loc_fn)?;
    lua.load(
        r#"
local M = __rs_lua_methods
local function player_locale(p)
    local ok, l = pcall(function() return p.LocaleId end)
    return ok and l or "en-us"
end
M.GetTranslatorForPlayerAsync = function(self, player)
    if typeof(player) ~= "Instance" or not player:IsA("Player") then error("Unable to cast value to Object", 0) end
    task.wait()
    -- (Studio, measured: an unpublished place has no cloud tables.)
    if game.GameId == 0 then error("Publishing the game is required to use GetTranslatorForPlayerAsync API.", 0) end
    return __rs_pc_translator(nil, player_locale(player))
end
M.GetTranslatorForPlayer = function(self, player)
    if typeof(player) ~= "Instance" or not player:IsA("Player") then error("Unable to cast value to Object", 0) end
    return __rs_pc_translator(nil, player_locale(player))
end
M.GetTranslatorForLocaleAsync = function(self, locale)
    if type(locale) ~= "string" then error("Argument 1 missing or nil", 0) end
    task.wait()
    return __rs_pc_translator(nil, locale)
end
-- The player's country from their locale (Studio's test player: US).
M.GetCountryRegionForPlayerAsync = function(self, player)
    if typeof(player) ~= "Instance" or not player:IsA("Player") then error("Unable to cast value to Object", 0) end
    task.wait()
    local l = player_locale(player)
    local region = l:match("%-(%a%a)$")
    return region and region:upper() or "US"
end
"#,
    )
    .set_name("=rustudio.players_chat.loc")
    .exec()?;
    Ok(())
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    if world(lua).is_err() {
        return Ok(());
    }
    let f: Option<Function> = lua.globals().get("__rs_pc_step").ok();
    if let Some(f) = f {
        if let Err(e) = f.call::<()>(()) {
            world(lua)?.output.push((crate::app::Level::Error, crate::runtime::error_message(&e)));
        }
    }
    Ok(())
}

// ------------------------------------------------- play session (play_game.rs)

/// The signed-in account's identity fields for Play's local player.
#[derive(Clone, Copy, Debug, Default)]
pub struct Identity {
    pub account_age: i32,
    pub verified: bool,
    pub premium: bool,
}

/// Fetches `uid`'s identity through the login session (waits up to `max`;
/// kept for the process). None when signed out or unreachable.
pub fn account_identity(uid: i64, max: std::time::Duration) -> Option<Identity> {
    let t0 = std::time::Instant::now();
    loop {
        match web::poll(&format!("identity:{uid}"), web::identity(uid)) {
            web::Poll::Ready(v) => {
                return Some(Identity {
                    account_age: v["AccountAge"].as_i64().unwrap_or(0) as i32,
                    verified: v["HasVerifiedBadge"].as_bool().unwrap_or(false),
                    premium: v["Premium"].as_bool().unwrap_or(false),
                })
            }
            web::Poll::Failed(_) => return None,
            web::Poll::Pending if t0.elapsed() < max => std::thread::sleep(std::time::Duration::from_millis(20)),
            web::Poll::Pending => return None,
        }
    }
}

/// A line the local player sent with TextChannel:SendAsync, to carry to
/// the server runtime.
pub struct Outgoing {
    pub channel: Ref,
    pub text: String,
    pub metadata: String,
    pub id: String,
}

/// The client's queued SendAsync messages.
pub fn take_outgoing(lua: &Lua) -> Vec<Outgoing> {
    let Ok(t) = lua.globals().get::<Table>("__rs_pc_outbox") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for v in t.clone().sequence_values::<Table>().flatten() {
        let channel = v.get::<Value>("channel").ok().and_then(|c| opt_inst(&c));
        if let Some(channel) = channel {
            out.push(Outgoing {
                channel,
                text: v.get("text").unwrap_or_default(),
                metadata: v.get("metadata").unwrap_or_default(),
                id: v.get("id").unwrap_or_default(),
            });
        }
    }
    let _ = t.clear();
    out
}

/// The server runtime receives a message: Player.Chatted, commands,
/// ShouldDeliverCallback. Returns the user ids it's delivered to.
pub fn server_receive(lua: &Lua, player: Ref, channel: Ref, m: &Outgoing) -> Vec<i64> {
    let f: Function = match lua.globals().get("__rs_pc_server_receive") {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let res = (|| -> mlua::Result<Vec<i64>> {
        let ids: Table = f.call((
            inst_value(lua, player)?,
            inst_value(lua, channel)?,
            m.text.as_str(),
            m.metadata.as_str(),
            m.id.as_str(),
        ))?;
        Ok(ids.sequence_values::<i64>().flatten().collect())
    })();
    match res {
        Ok(v) => v,
        Err(e) => {
            if let Ok(mut w) = world(lua) {
                w.output.push((crate::app::Level::Error, crate::runtime::error_message(&e)));
            }
            Vec::new()
        }
    }
}

/// The client runtime receives a delivered message (MessageReceived, the
/// window line and the bubble).
pub fn client_receive(lua: &Lua, channel: Ref, sender: i64, m: &Outgoing) {
    let Ok(f) = lua.globals().get::<Function>("__rs_pc_client_receive") else {
        return;
    };
    let r = (|| -> mlua::Result<()> {
        f.call::<()>((inst_value(lua, channel)?, sender, m.text.as_str(), m.metadata.as_str(), m.id.as_str()))
    })();
    if let Err(e) = r {
        if let Ok(mut w) = world(lua) {
            w.output.push((crate::app::Level::Error, crate::runtime::error_message(&e)));
        }
    }
}

/// The local player typed `text` in the chat bar. True when TextChatService
/// took it (it goes out through SendAsync); false for the legacy chat.
pub fn chat_input(lua: &Lua, text: &str) -> bool {
    let Ok(f) = lua.globals().get::<Function>("__rs_pc_chat_input") else {
        return false;
    };
    f.call::<bool>(text).unwrap_or(false)
}

/// One line of the chat window.
pub struct ViewLine {
    pub id: u64,
    pub prefix: String,
    pub text: String,
    pub system: bool,
    pub name_color: Option<[f32; 3]>,
    pub text_color: Option<[f32; 3]>,
    /// Seconds since it arrived.
    pub age: f64,
}

/// A bubble over a part.
pub struct ViewBubble {
    pub id: u64,
    pub adornee: Ref,
    /// Where the bubble's tail points (above the adornee), in the world.
    pub pos: Option<glam::Vec3>,
    pub text: String,
    pub age: f64,
    pub bg: Option<[f32; 3]>,
    pub bg_transparency: Option<f32>,
    pub fg: Option<[f32; 3]>,
}

/// The bubble settings scripts can change (BubbleChatConfiguration).
#[derive(Clone, Copy)]
pub struct BubbleStyle {
    pub enabled: bool,
    pub duration: f64,
    pub max_bubbles: usize,
    pub max_distance: f32,
    pub offset: f32,
    pub text_size: f32,
    pub bg: [f32; 3],
    pub bg_transparency: f32,
    pub fg: [f32; 3],
}

/// The window style (ChatWindowConfiguration).
#[derive(Clone, Copy)]
pub struct WindowStyle {
    pub enabled: bool,
    pub bg: [f32; 3],
    pub bg_transparency: f32,
    pub text: [f32; 3],
    pub text_size: f32,
    pub stroke: [f32; 3],
    pub stroke_transparency: f32,
    pub height_scale: f32,
    pub width_scale: f32,
    pub input_enabled: bool,
    pub input_bg: [f32; 3],
    pub input_bg_transparency: f32,
    pub placeholder: [f32; 3],
}

pub struct ChatView {
    pub tcs: bool,
    pub lines: Vec<ViewLine>,
    pub bubbles: Vec<ViewBubble>,
    pub window: WindowStyle,
    pub bubble: BubbleStyle,
}

fn c3(v: Value) -> Option<[f32; 3]> {
    opt_ud::<LColor3>(&v).map(|c| c.0)
}

/// What the client's default chat shows now.
pub fn chat_view(lua: &Lua) -> Option<ChatView> {
    let f: Function = lua.globals().get("__rs_pc_chat_view").ok()?;
    let v: Table = f.call(()).ok()?;
    let now: f64 = v.get("now").unwrap_or(0.0);
    let mut lines = Vec::new();
    for l in v.get::<Table>("lines").ok()?.sequence_values::<Table>().flatten() {
        lines.push(ViewLine {
            id: l.get("id").unwrap_or(0),
            prefix: l.get("prefix").unwrap_or_default(),
            text: l.get("text").unwrap_or_default(),
            system: l.get("system").unwrap_or(false),
            name_color: l.get::<Value>("name_color").ok().and_then(c3),
            text_color: l.get::<Value>("text_color").ok().and_then(c3),
            age: now - l.get::<f64>("time").unwrap_or(now),
        });
    }
    let doc_side = {
        let w = world(lua).ok()?;
        let d = w.d();
        let tcs = d.find_child(d.dom.root_ref(), "TextChatService");
        let kid = |class: &str| {
            tcs.and_then(|t| d.dom.get_by_ref(t)).and_then(|i| {
                i.children().iter().copied().find(|c| d.class_of(*c).is_some_and(|k| k == class))
            })
        };
        (tcs, kid("ChatWindowConfiguration"), kid("BubbleChatConfiguration"), kid("ChatInputBarConfiguration"))
    };
    let get = |r: Option<Ref>, k: &str| -> Value {
        r.and_then(|r| inst_value(lua, r).ok())
            .and_then(|i| match i {
                Value::UserData(u) => lua
                    .load("return function(i, k) local ok, v = pcall(function() return i[k] end) if ok then return v end end")
                    .eval::<Function>()
                    .ok()
                    .and_then(|f| f.call::<Value>((Value::UserData(u), k)).ok()),
                _ => None,
            })
            .unwrap_or(Value::Nil)
    };
    let num = |v: Value, d: f32| match v {
        Value::Number(n) => n as f32,
        Value::Integer(n) => n as f32,
        _ => d,
    };
    let boolean = |v: Value, d: bool| match v {
        Value::Boolean(b) => b,
        _ => d,
    };
    let tcs_on = {
        let ver = get(doc_side.0, "ChatVersion");
        opt_ud::<LEnumItem>(&ver).is_some_and(|e| e.name == "TextChatService")
    };
    let (_, win, bub, inp) = doc_side;
    let window = WindowStyle {
        enabled: boolean(get(win, "Enabled"), true),
        bg: c3(get(win, "BackgroundColor3")).unwrap_or([0.098, 0.106, 0.114]),
        bg_transparency: num(get(win, "BackgroundTransparency"), 0.3),
        text: c3(get(win, "TextColor3")).unwrap_or([1.0, 1.0, 1.0]),
        text_size: num(get(win, "TextSize"), 14.0),
        stroke: c3(get(win, "TextStrokeColor3")).unwrap_or([0.0, 0.0, 0.0]),
        stroke_transparency: num(get(win, "TextStrokeTransparency"), 0.5),
        height_scale: num(get(win, "HeightScale"), 1.0),
        width_scale: num(get(win, "WidthScale"), 1.0),
        input_enabled: boolean(get(inp, "Enabled"), true),
        input_bg: c3(get(inp, "BackgroundColor3")).unwrap_or([0.098, 0.106, 0.114]),
        input_bg_transparency: num(get(inp, "BackgroundTransparency"), 0.2),
        placeholder: c3(get(inp, "PlaceholderColor3")).unwrap_or([0.698, 0.698, 0.698]),
    };
    let bubble = BubbleStyle {
        enabled: boolean(get(bub, "Enabled"), true),
        duration: num(get(bub, "BubbleDuration"), 15.0) as f64,
        max_bubbles: num(get(bub, "MaxBubbles"), 3.0).max(1.0) as usize,
        max_distance: num(get(bub, "MaxDistance"), 100.0),
        offset: num(get(bub, "VerticalStudsOffset"), 0.0),
        text_size: num(get(bub, "TextSize"), 16.0),
        bg: c3(get(bub, "BackgroundColor3")).unwrap_or([0.980, 0.980, 0.980]),
        bg_transparency: num(get(bub, "BackgroundTransparency"), 0.1),
        fg: c3(get(bub, "TextColor3")).unwrap_or([0.224, 0.231, 0.239]),
    };
    let mut bubbles = Vec::new();
    for b in v.get::<Table>("bubbles").ok()?.sequence_values::<Table>().flatten() {
        let age = now - b.get::<f64>("time").unwrap_or(now);
        let Some(adornee) = b.get::<Value>("adornee").ok().and_then(|a| opt_inst(&a)) else {
            continue;
        };
        // Over the character's root: above its head; over another part:
        // above its top.
        let pos = world(lua).ok().and_then(|w| {
            let d = w.d();
            let xf = crate::scene::xform_of(d, adornee)?;
            let half = match d.prop(adornee, "Size") {
                Some(Variant::Vector3(v)) => v.y / 2.0,
                _ => 1.0,
            };
            let root = d.dom.get_by_ref(adornee).is_some_and(|i| i.name == "HumanoidRootPart");
            Some(xf.pos + glam::Vec3::Y * if root { half + 2.6 } else { half + 0.5 })
        });
        bubbles.push(ViewBubble {
            id: b.get("id").unwrap_or(0),
            adornee,
            pos,
            text: b.get("text").unwrap_or_default(),
            age,
            bg: b.get::<Value>("bg").ok().and_then(c3),
            bg_transparency: b.get::<Option<f32>>("bg_t").ok().flatten(),
            fg: b.get::<Value>("fg").ok().and_then(c3),
        });
    }
    Some(ChatView { tcs: tcs_on, lines, bubbles, window, bubble })
}
