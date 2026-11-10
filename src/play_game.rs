//! In-game interaction during Play, following the public Roblox docs:
//! ClickDetectors (MouseClick, RightMouseClick, hover, MaxActivationDistance),
//! the player's Mouse, chat (Player.Chatted), Tools and the hotbar,
//! Explosions, Sounds (real audio), Humanoid:MoveTo for NPCs, and a stand-in
//! for the chat system's ChatServiceRunner.

use std::sync::Arc;

use egui::{Color32, Pos2, Rect, Stroke};

use super::*;
use crate::audio::Audio;
use crate::runtime::ext::SoundCmd;
use crate::scene::vec3_of;
use crate::theme;

#[path = "game_menu.rs"]
pub mod game_menu;

/// What's under the cursor (refs belong to the DataModel shown when picked).
#[derive(Debug, Clone, Copy)]
pub struct Pointer {
    pub origin: Vec3,
    pub dir: Vec3,
    pub hit: Option<(Ref, Vec3)>,
    pub xy: [f32; 2],
    pub view: [f32; 2],
    pub side: Side,
}

/// Input the viewport collected for the next play tick.
#[derive(Default)]
pub struct PlayInput {
    pub pointer: Option<Pointer>,
    /// (part clicked, mouse button 1 or 2).
    pub clicks: Vec<(Ref, u8)>,
    /// (mouse button, pressed).
    pub buttons: Vec<(u8, bool)>,
    pub chat: Vec<String>,
    /// Hotbar slots toggled (0-based).
    pub slots: Vec<usize>,
}

struct Equipped {
    tool: Ref,
    /// Each tool part relative to the handle.
    offsets: Vec<(Ref, Xform)>,
    client_told: bool,
}

struct Walk {
    target: Vec3,
    since: f64,
}

struct Track {
    server: Option<Ref>,
    client: Option<Ref>,
    /// Sound id still downloading.
    pending: Option<String>,
    /// The SoundId it last started with.
    id: Option<String>,
}

pub struct Flash {
    pub pos: Vec3,
    pub radius: f32,
    pub born: f64,
}

#[derive(Default)]
pub struct GameState {
    pub input: PlayInput,
    pub(crate) last_pointer: Option<Pointer>,
    hover_detector: Option<Ref>,
    /// Whether the cursor is over something clickable (hand cursor).
    pub clickable: bool,
    equipped: Option<Equipped>,
    /// Tools on the hotbar (server refs), in slot order.
    pub slots: Vec<Ref>,
    walks: HashMap<Ref, Walk>,
    /// Humanoid:MoveTo target for the player's own character.
    pub move_target: Option<Vec3>,
    pub jump: bool,
    /// Humanoid:ChangeState for the player's own character (state name).
    pub forced_state: Option<String>,
    /// Seat:Sit asked for the player's own character (server seat).
    pub sit_on: Option<Ref>,
    /// States its Humanoid:SetStateEnabled turned off.
    pub states_off: HashSet<String>,
    audio: Option<Audio>,
    tracks: HashMap<u64, Track>,
    pub flashes: Vec<Flash>,
    pub chat_open: bool,
    /// Frames the playtest view had chat available but closed: where the
    /// old "Press / to chat" hint was drawn (the self-test checks none is).
    pub chat_ready_frames: u64,
    pub chat_text: String,
    /// (line, time) shown over the viewport.
    pub chat_log: Vec<(String, f64)>,
    /// Player:Kick's message: the player was disconnected (Studio's
    /// "Disconnected" dialog shows over the game).
    pub kicked: Option<String>,
    /// Play time of the player's last input, and of the last Player.Idled.
    pub last_activity: f64,
    idled_at: f64,
    warned: HashSet<String>,
    /// Audio::play starts per sound key (RUSTUDIO_AUDIO_LOG measurement).
    pub audio_starts: HashMap<u64, u32>,
    /// The same per SoundId, and how often each id was fetched (self-test).
    pub audio_starts_by_id: HashMap<String, u32>,
    pub sound_fetches: HashMap<String, u32>,
    /// Sound ids that came back unavailable: not fetched again this session.
    sound_unavailable: HashSet<String>,
    /// GUI events from the viewport, fired on the client next tick.
    pub gui_events: Vec<crate::gui::GuiEvent>,
    /// Where the client's GUI objects were drawn (AbsolutePosition / AbsoluteSize).
    pub gui_layout: HashMap<Ref, crate::gui::GuiBox>,
    /// Mouse buttons for UserInputService: (button, down, gameProcessedEvent).
    pub uis_mouse: Vec<(u8, bool, bool)>,
    /// The in-game menu (Esc) is open: the game gets no keys.
    pub menu_open: bool,
    /// Menu opened (true) / closed (false) since the last client tick
    /// (GuiService.MenuOpened / MenuClosed).
    pub menu_events: Vec<bool>,
    /// The menu's pages, tween and settings (game_menu.rs).
    pub menu: game_menu::MenuState,
    /// The menu's Reset Character / Leave were chosen.
    pub reset_request: bool,
    pub leave_request: bool,
    /// Keys pressed since the last tick (a same-frame tap included), and
    /// whether a focused TextBox had the keys then (gameProcessedEvent).
    pub key_taps: Vec<(egui::Key, bool)>,
}

impl GameState {
    /// View > Mute: silences the sounds playing now (later ones read the flag).
    pub fn set_muted(&mut self, on: bool) {
        if let Some(a) = self.audio.as_mut() {
            a.set_muted(on);
        }
    }

    /// Whether a Sound with this SoundId is still being played (self-test).
    pub fn is_sound_live(&self, id: &str) -> bool {
        self.tracks.iter().any(|(k, t)| {
            t.id.as_deref() == Some(id) && self.audio.as_ref().is_some_and(|a| a.is_playing(*k))
        })
    }
}

/// An InputObject for a mouse button (1, 2; 0 = movement) at a GUI position.
pub fn mouse_input_object(
    lua: &mlua::Lua,
    button: u8,
    began: bool,
    at: Option<(f32, f32)>,
) -> mlua::Result<Value> {
    let t = lua.create_table()?;
    let kind = match button {
        1 => "MouseButton1",
        2 => "MouseButton2",
        // The wheel, forward / backward (Position.Z +1 / -1).
        3 | 4 => "MouseWheel",
        _ => "MouseMovement",
    };
    for (prop, en, item) in [
        ("UserInputType", "UserInputType", kind),
        (
            "UserInputState",
            "UserInputState",
            if matches!(button, 0 | 3 | 4) {
                "Change"
            } else if began {
                "Begin"
            } else {
                "End"
            },
        ),
        ("KeyCode", "KeyCode", "Unknown"),
    ] {
        if let Some(k) = LEnumItem::from_name(en, item) {
            t.set(prop, crate::ltypes::enum_value(lua, k)?)?;
        }
    }
    let (x, y) = at.unwrap_or((0.0, 0.0));
    let z = match button {
        3 => 1.0,
        4 => -1.0,
        _ => 0.0,
    };
    t.set(
        "Position",
        crate::ltypes::LVector3(Vec3::new(x, y, z)).into_value(),
    )?;
    t.set(
        "Delta",
        crate::ltypes::LVector3(Vec3::ZERO).into_value(),
    )?;
    // As a real InputObject (a userdata), like UserInputService's.
    match lua.globals().get::<mlua::Function>("__rs_new_input_object") {
        Ok(f) => f.call(t),
        Err(_) => Ok(Value::Table(t)),
    }
}

/// Fires the viewport's GUI events on the client: clicks, hover, text,
/// selection and scrolling.
pub fn fire_gui_events(rt: &Runtime, events: Vec<crate::gui::GuiEvent>, gui_service: Option<Ref>) {
    use crate::gui::{GuiArg, GuiEvent};
    let lua = &rt.lua;
    let debug = std::env::var_os("RUSTUDIO_GUI_DEBUG").is_some();
    for ev in events {
        if debug
            && !matches!(
                &ev,
                GuiEvent::Fire(_, "MouseMoved" | "MouseEnter" | "MouseLeave", _)
            )
        {
            if let Some(mut w) = rt.world_mut() {
                let line = match &ev {
                    GuiEvent::Fire(r, name, _) => format!("[gui] {name} on {}", w.full_name(*r)),
                    other => format!("[gui] {other:?}"),
                };
                w.output.push((Level::Info, line));
            }
        }
        match ev {
            GuiEvent::Fire(r, name, args) => {
                let vals: Vec<Value> = args
                    .iter()
                    .filter_map(|a| match a {
                        GuiArg::Bool(b) => Some(Value::Boolean(*b)),
                        GuiArg::Num(n) => Some(Value::Number(*n)),
                        GuiArg::Input {
                            button,
                            began,
                            x,
                            y,
                        } => mouse_input_object(lua, *button, *began, Some((*x, *y))).ok(),
                    })
                    .collect();
                rt.fire(r, name, vals);
                // UserInputService.TextBoxFocused / TextBoxFocusReleased(textBox)
                // go with a TextBox's Focused / FocusLost.
                let uis_ev = match name {
                    "Focused" => Some("TextBoxFocused"),
                    "FocusLost" => Some("TextBoxFocusReleased"),
                    _ => None,
                };
                if let Some(ev) = uis_ev {
                    let uis = rt.world_mut().and_then(|w| {
                        let d = w.d();
                        d.find_child(d.dom.root_ref(), "UserInputService")
                    });
                    if let Some(u) = uis {
                        rt.fire(u, ev, vec![rt.inst(r)]);
                    }
                }
            }
            GuiEvent::SetText(r, text) => {
                if let Ok(s) = lua.create_string(&text) {
                    rt.set(r, "Text", Value::String(s));
                }
            }
            GuiEvent::Select(sel) => {
                if let Some(gs) = gui_service {
                    rt.set(
                        gs,
                        "SelectedObject",
                        sel.map(|s| rt.inst(s)).unwrap_or(Value::Nil),
                    );
                }
            }
            GuiEvent::Scroll(r, d) => {
                let (cur, max) = rt
                    .world_mut()
                    .map(|w| {
                        let doc = w.d();
                        let cp = match doc.prop(r, "CanvasPosition") {
                            Some(Variant::Vector2(v)) => [v.x, v.y],
                            _ => [0.0, 0.0],
                        };
                        let laid = w.input.gui_layout.get(&r);
                        let size = laid.map(|b| b.size).unwrap_or([0.0, 0.0]);
                        // The drawn canvas (AutomaticCanvasSize included).
                        let cs = match (laid.and_then(|b| b.canvas), doc.prop(r, "CanvasSize")) {
                            (Some(c), _) => c,
                            (None, Some(Variant::UDim2(u))) => [
                                u.x.scale * size[0] + u.x.offset as f32,
                                u.y.scale * size[1] + u.y.offset as f32,
                            ],
                            _ => size,
                        };
                        (cp, [(cs[0] - size[0]).max(0.0), (cs[1] - size[1]).max(0.0)])
                    })
                    .unwrap_or(([0.0, 0.0], [0.0, 0.0]));
                let step = mech::GUI_SCROLL_PX_PER_POINT;
                let next = glam::Vec2::new(
                    (cur[0] + d[0] * step).clamp(0.0, max[0]),
                    (cur[1] + d[1] * step).clamp(0.0, max[1]),
                );
                if let Ok(v) = lua.create_userdata(crate::ltypes::LVector2(next)) {
                    rt.set(r, "CanvasPosition", Value::UserData(v));
                }
            }
        }
    }
    rt.dispatch();
}

fn sound_key(r: Ref) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    r.hash(&mut h);
    h.finish()
}

fn f32_prop(d: &Document, r: Ref, name: &str, default: f32) -> f32 {
    f32_of(d.prop(r, name)).unwrap_or(default)
}

/// Source of the stand-in ChatService module (rustudio's own code).
const CHAT_SERVICE_STUB: &str = r#"-- rustudio stand-in for the chat system's ChatService module.
-- The calls exist so scripts that wait for it keep running; messages go to Output.
local Players = game:GetService("Players")
local ChatService = {}
local speakers, channels = {}, {}
local added, removed = Instance.new("BindableEvent"), Instance.new("BindableEvent")
ChatService.SpeakerAdded = added.Event
ChatService.SpeakerRemoved = removed.Event
local function channel(name)
    local c = channels[name]
    if c then return c end
    c = { Name = name }
    function c:SendSystemMessage(msg) print("[" .. name .. "] " .. tostring(msg)) end
    function c:KickSpeaker() end
    function c:MuteSpeaker() end
    function c:RegisterProcessCommandsFunction() end
    function c:RegisterFilterMessageFunction() end
    channels[name] = c
    return c
end
function ChatService:GetChannel(name) return channel(name) end
function ChatService:AddChannel(name) return channel(name) end
function ChatService:RemoveChannel(name) channels[name] = nil end
function ChatService:GetSpeaker(name) return speakers[name] end
function ChatService:AddSpeaker(name)
    local s = { Name = name }
    function s:SayMessage(msg) print("[Chat] " .. name .. ": " .. tostring(msg)) end
    function s:JoinChannel(c) return channel(c) end
    function s:LeaveChannel() end
    function s:SetExtraData() end
    function s:GetExtraData() return nil end
    function s:SendSystemMessage(msg) print("[System] " .. tostring(msg)) end
    function s:GetPlayer() return Players:FindFirstChild(name) end
    speakers[name] = s
    added:Fire(name)
    return s
end
function ChatService:RemoveSpeaker(name) speakers[name] = nil; removed:Fire(name) end
function ChatService:GetSpeakerList()
    local t = {}
    for n in pairs(speakers) do table.insert(t, n) end
    return t
end
function ChatService:RegisterProcessCommandsFunction() end
function ChatService:UnregisterProcessCommandsFunction() end
function ChatService:RegisterFilterMessageFunction() end
function ChatService:UnregisterFilterMessageFunction() end
function ChatService:InternalApplyRobloxFilter(_, msg) return msg end
return ChatService
"#;

impl Editor {
    /// Adds ServerScriptService.ChatServiceRunner (with a ChatService module)
    /// when the place has none, so WaitForChild("ChatServiceRunner") returns.
    pub(super) fn add_chat_stub(&mut self) {
        let root = self.doc.dom.root_ref();
        let Some(sss) = self.doc.find_child(root, "ServerScriptService") else {
            return;
        };
        if self.doc.find_child(sss, "ChatServiceRunner").is_some()
            || self.doc.find_named(sss, "ChatServiceRunner").is_some()
        {
            return;
        }
        let runner = InstanceBuilder::new("Script")
            .with_name("ChatServiceRunner")
            .with_property("Disabled", true)
            .with_property("Enabled", false)
            .with_child(
                InstanceBuilder::new("ModuleScript")
                    .with_name("ChatService")
                    .with_property("Source", CHAT_SERVICE_STUB.to_string()),
            );
        let r = self.doc.dom.insert(sss, runner);
        self.doc.mark_tree(r);
    }

    /// TextChannel:SendAsync messages: from the client runtime to the
    /// server's (Chatted, ShouldDeliverCallback) and back to the client
    /// (MessageReceived, the window and the bubble). Player:Kick too.
    fn pump_text_chat(&mut self) {
        use crate::runtime::domains::players_chat as pc;
        let Some(player) = self.play.as_ref().map(|p| p.player) else {
            return;
        };
        let out = self
            .with_side(Side::Client, |rt, _, _| pc::take_outgoing(&rt.lua))
            .unwrap_or_default();
        let uid = match self.doc.prop(player, "UserId") {
            Some(Variant::Int64(u)) => u,
            _ => 0,
        };
        for m in out {
            let Some(server_ch) = self.server_of(m.channel) else {
                continue;
            };
            let to = self
                .with_side(Side::Server, |rt, _, _| {
                    let ids = pc::server_receive(&rt.lua, player, server_ch, &m);
                    rt.dispatch();
                    ids
                })
                .unwrap_or_default();
            if to.contains(&uid) {
                if let Some(ch) = self.client_of(server_ch) {
                    self.with_side(Side::Client, |rt, _, _| {
                        pc::client_receive(&rt.lua, ch, uid, &m);
                        rt.dispatch();
                    });
                }
            }
        }
        if let Some(msg) = pc::take_kick() {
            if let Some(s) = self.play.as_mut() {
                if s.game.kicked.is_none() {
                    s.game.kicked = Some(msg.clone());
                    // The client left: the server sees the player go, as in
                    // Studio: the kick logged, CharacterRemoving, then
                    // PlayerRemoving(player, Enum.PlayerExitReason.CreatorKick).
                    self.error(format!("Server Kick Message: {msg}; Metadata:"));
                    let model = self.play.as_ref().and_then(|s| s.character.as_ref()).map(|c| c.model);
                    if let Some(m) = model {
                        self.fire_character_removing(m);
                    }
                    let players = self.doc.find_child(self.doc.dom.root_ref(), "Players");
                    if let Some(ps) = players {
                        self.with_side(Side::Server, |rt, _, _| {
                            let reason = rt
                                .lua
                                .load("return Enum.PlayerExitReason.CreatorKick")
                                .eval::<Value>()
                                .unwrap_or(Value::Nil);
                            rt.fire(ps, "PlayerRemoving", vec![rt.inst(player), reason]);
                            rt.dispatch();
                        });
                    }
                }
            }
        }
    }

    /// Player.Idled(seconds idle): two minutes after the player's last
    /// input, then every 30 seconds while still idle (both sides).
    fn fire_idled(&mut self, now: f64) {
        let Some((player, last, at)) = self.play.as_ref().map(|s| (s.player, s.game.last_activity, s.game.idled_at)) else {
            return;
        };
        let idle = now - last;
        let due = if at <= last { idle >= 120.0 } else { now - at >= 30.0 };
        if !due {
            return;
        }
        if let Some(s) = self.play.as_mut() {
            s.game.idled_at = now;
        }
        self.with_side(Side::Server, |rt, _, _| rt.fire(player, "Idled", vec![Value::Number(idle)]));
        if let Some(cp) = self.client_of(player) {
            self.with_side(Side::Client, |rt, _, _| rt.fire(cp, "Idled", vec![Value::Number(idle)]));
        }
    }

    fn client_of(&self, server: Ref) -> Option<Ref> {
        self.play.as_ref()?.map.s2c.get(&server).copied()
    }

    fn server_of(&self, client: Ref) -> Option<Ref> {
        self.play.as_ref()?.map.c2s.get(&client).copied()
    }

    fn to_server(&self, side: Side, r: Ref) -> Option<Ref> {
        match side {
            Side::Server => Some(r),
            Side::Client => self.server_of(r),
        }
    }

    /// Fires `event(player)` on the server instance and its client copy.
    fn fire_with_player(&mut self, server_target: Ref, event: &str) {
        let Some(player) = self.play.as_ref().map(|p| p.player) else {
            return;
        };
        let ev = event.to_string();
        self.with_side(Side::Server, |rt, _, _| {
            rt.fire(server_target, &ev, vec![rt.inst(player)])
        });
        if let (Some(c), Some(cp)) = (self.client_of(server_target), self.client_of(player)) {
            self.with_side(Side::Client, |rt, _, _| rt.fire(c, &ev, vec![rt.inst(cp)]));
        }
    }

    /// Fires `event()` on the server instance and its client copy.
    fn fire_both(&mut self, server_target: Ref, event: &str) {
        let ev = event.to_string();
        self.with_side(Side::Server, |rt, _, _| rt.fire(server_target, &ev, vec![]));
        if let Some(c) = self.client_of(server_target) {
            self.with_side(Side::Client, |rt, _, _| rt.fire(c, &ev, vec![]));
        }
    }

    /// The ClickDetector a click on `part` reaches: one in the part or in a
    /// Model / folder above it (below Workspace).
    fn click_detector_for(&self, part: Ref) -> Option<Ref> {
        let d = &self.doc;
        let ws = d.workspace()?;
        let mut cur = part;
        while cur.is_some() && cur != ws {
            let inst = d.dom.get_by_ref(cur)?;
            if let Some(c) = inst
                .children()
                .iter()
                .copied()
                .find(|c| d.class_of(*c).is_some_and(|k| k == "ClickDetector"))
            {
                return Some(c);
            }
            cur = inst.parent();
        }
        None
    }

    fn character_pos(&self) -> Option<Vec3> {
        self.play
            .as_ref()?
            .character
            .as_ref()
            .filter(|c| !c.dead)
            .map(|c| c.body.pos)
    }

    fn in_reach(&self, detector: Ref, part: Ref) -> bool {
        let max = f32_prop(
            &self.doc,
            detector,
            "MaxActivationDistance",
            mech::CLICK_DISTANCE,
        );
        let Some(at) = self.character_pos() else {
            return true;
        };
        let Some(xf) = crate::scene::xform_of(&self.doc, part) else {
            return false;
        };
        (xf.pos - at).length() <= max
    }

    /// The input the viewport queued, before the server's scripts run.
    pub(super) fn game_inputs(&mut self) {
        let Some(s) = self.play.as_mut() else { return };
        let input = std::mem::take(&mut s.game.input);
        if input.pointer.is_some() {
            s.game.last_pointer = input.pointer;
        }
        let player = s.player;
        let now = s.time;
        let me = self.player_name();

        for msg in input.chat {
            // TextChatService chat: the line goes out through the target
            // channel's SendAsync (players_chat.luau), which fires Chatted.
            let tcs = self
                .with_side(Side::Client, |rt, _, _| crate::runtime::domains::players_chat::chat_input(&rt.lua, &msg))
                .unwrap_or(false);
            if tcs {
                continue;
            }
            let line = format!("{me}: {msg}");
            self.info(format!("[Chat] {line}"));
            if let Some(s) = self.play.as_mut() {
                s.game.chat_log.push((line, now));
            }
            let m = msg.clone();
            self.with_side(Side::Server, |rt, _, _| {
                if let Ok(text) = rt.lua.create_string(&m) {
                    rt.fire(player, "Chatted", vec![Value::String(text), Value::Nil]);
                }
            });
            if let Some(cp) = self.client_of(player) {
                self.with_side(Side::Client, |rt, _, _| {
                    if let Ok(text) = rt.lua.create_string(&msg) {
                        rt.fire(cp, "Chatted", vec![Value::String(text), Value::Nil]);
                    }
                });
            }
        }

        self.pump_text_chat();
        self.fire_idled(now);

        // Hovering a ClickDetector.
        let pointer = self.play.as_ref().and_then(|s| s.game.last_pointer);
        let hovered = pointer
            .and_then(|p| p.hit.and_then(|(r, _)| self.to_server(p.side, r)))
            .and_then(|part| {
                self.click_detector_for(part)
                    .filter(|d| self.in_reach(*d, part))
            });
        let before = self.play.as_ref().and_then(|s| s.game.hover_detector);
        if hovered != before {
            if let Some(old) = before.filter(|d| self.doc.exists(*d)) {
                self.fire_with_player(old, "MouseHoverLeave");
            }
            if let Some(new) = hovered {
                self.fire_with_player(new, "MouseHoverEnter");
            }
        }
        if let Some(s) = self.play.as_mut() {
            s.game.hover_detector = hovered;
            s.game.clickable = hovered.is_some();
        }

        // Clicks on ClickDetectors (fired on both sides with the player).
        let side = pointer.map(|p| p.side).unwrap_or(Side::Server);
        for (r, button) in input.clicks {
            let Some(part) = self.to_server(side, r) else {
                continue;
            };
            if let Some(det) = self
                .click_detector_for(part)
                .filter(|d| self.in_reach(*d, part))
            {
                self.fire_with_player(
                    det,
                    if button == 1 {
                        "MouseClick"
                    } else {
                        "RightMouseClick"
                    },
                );
            }
        }

        // DragDetector drags (play_drag.rs).
        self.drag_detectors(&input.buttons);

        for slot in input.slots {
            let tool = self
                .play
                .as_ref()
                .and_then(|s| s.game.slots.get(slot).copied());
            let equipped = self
                .play
                .as_ref()
                .and_then(|s| s.game.equipped.as_ref().map(|e| e.tool));
            match tool {
                Some(t) if Some(t) == equipped => self.unequip_tool(),
                Some(t) => self.equip_tool(t),
                None => {}
            }
        }

        // Mouse buttons: the equipped tool activates; the Mouse object fires.
        let equipped = self
            .play
            .as_ref()
            .and_then(|s| s.game.equipped.as_ref().map(|e| e.tool));
        let mouse = self
            .play
            .as_ref()
            .and_then(|s| s.client.as_ref())
            .and_then(|c| c.world.as_ref())
            .and_then(|w| w.input.mouse);
        for (button, down) in input.buttons {
            if button == 1 {
                if let Some(t) = equipped {
                    self.fire_both(t, if down { "Activated" } else { "Deactivated" });
                }
            }
            if let Some(m) = mouse {
                let ev = format!("Button{button}{}", if down { "Down" } else { "Up" });
                self.with_side(Side::Client, |rt, _, _| rt.fire(m, &ev, vec![]));
            }
        }
    }

    /// Mouse state for client scripts, before they run.
    /// The client's Camera.ViewportSize: the 3D view's size, known before the
    /// mouse ever moves over it.
    pub(super) fn sync_view_size(&mut self) {
        let r = self.viewport_rect;
        if r.is_positive() {
            if let Some(w) = self
                .play
                .as_mut()
                .and_then(|s| s.client.as_mut())
                .and_then(|c| c.world.as_mut())
            {
                w.input.view_size = [r.width(), r.height()];
            }
        }
    }

    pub(super) fn game_client_inputs(&mut self) {
        self.sync_view_size();
        let Some(p) = self.play.as_ref().and_then(|s| s.game.last_pointer) else {
            return;
        };
        let hit = p.hit.and_then(|(r, pos)| {
            let client = match p.side {
                Side::Client => Some(r),
                Side::Server => self.client_of(r),
            };
            Some((client?, pos))
        });
        // Tell the client tool it was equipped once it has replicated.
        let pending = self.play.as_ref().and_then(|s| {
            s.game
                .equipped
                .as_ref()
                .filter(|e| !e.client_told)
                .map(|e| e.tool)
        });
        let client_tool = pending.and_then(|t| self.client_of(t));
        let Some(s) = self.play.as_mut() else { return };
        let Some(w) = s.client.as_mut().and_then(|c| c.world.as_mut()) else {
            return;
        };
        w.input.mouse_ray = (p.origin, p.dir);
        w.input.mouse_xy = p.xy;
        w.input.view_size = p.view;
        w.input.mouse_target = hit.map(|h| h.0);
        w.input.mouse_hit = Some(Xform {
            pos: hit.map(|h| h.1).unwrap_or(p.origin + p.dir * 1000.0),
            rot: crate::ltypes::look_at(p.origin, p.origin + p.dir, Vec3::Y).rot,
        });
        let mouse = w.input.mouse;
        if let Some(ct) = client_tool {
            if let Some(e) = s.game.equipped.as_mut() {
                e.client_told = true;
            }
            self.with_side(Side::Client, |rt, _, _| {
                let m = mouse.map(|m| rt.inst(m)).unwrap_or(Value::Nil);
                rt.fire(ct, "Equipped", vec![m]);
            });
        }
    }

    /// Acts on what scripts on one side asked for this frame.
    pub(super) fn game_requests(&mut self, side: Side) {
        let reqs = {
            let Some(s) = self.play.as_mut() else { return };
            let w = match side {
                Side::Server => s.server.world.as_mut(),
                Side::Client => s.client.as_mut().and_then(|c| c.world.as_mut()),
            };
            match w {
                Some(w) => std::mem::take(&mut w.requests),
                None => return,
            }
        };
        let mine_server = self
            .play
            .as_ref()
            .and_then(|s| s.character.as_ref().map(|c| c.humanoid));
        let mine = |h: Ref, ed: &Editor| -> bool {
            match side {
                Side::Server => Some(h) == mine_server,
                Side::Client => mine_server.and_then(|m| ed.client_of(m)) == Some(h),
            }
        };
        for (r, cmd) in reqs.sounds {
            let (server, client) = match side {
                Side::Server => (Some(r), self.client_of(r)),
                Side::Client => (self.server_of(r), Some(r)),
            };
            self.sound_command(server, client, cmd);
        }
        for (h, target) in reqs.move_to {
            if mine(h, self) {
                if let Some(s) = self.play.as_mut() {
                    s.game.move_target = Some(target);
                }
            } else if side == Side::Server {
                let now = self.play.as_ref().map(|s| s.time).unwrap_or(0.0);
                if let Some(s) = self.play.as_mut() {
                    s.game.walks.insert(h, Walk { target, since: now });
                }
            }
        }
        for h in reqs.jumps {
            if mine(h, self) {
                if let Some(s) = self.play.as_mut() {
                    s.game.jump = true;
                }
            } else if side == Side::Server {
                if let Some(root) = self.npc_root(h) {
                    let power = f32_prop(&self.doc, h, "JumpPower", mech::JUMP_POWER);
                    if let Some(s) = self.play.as_mut() {
                        let v = s
                            .physics
                            .velocities()
                            .get(&root)
                            .copied()
                            .unwrap_or(Vec3::ZERO);
                        s.physics.set_velocity(root, Vec3::new(v.x, power, v.z));
                    }
                }
            }
        }
        for (h, tool) in reqs.equip {
            if !mine(h, self) {
                continue;
            }
            let tool = match (side, tool) {
                (Side::Client, Some(t)) => self.server_of(t),
                (_, t) => t,
            };
            match tool {
                Some(t) => self.equip_tool(t),
                None => self.unequip_tool(),
            }
        }
        for e in reqs.explosions {
            if side == Side::Server {
                self.explode(e);
            }
        }
        for (h, state) in reqs.change_state {
            if mine(h, self) {
                if let Some(s) = self.play.as_mut() {
                    s.game.forced_state = Some(state);
                }
            }
        }
        for (seat, h) in reqs.sits {
            if mine(h, self) {
                let seat = match side {
                    Side::Client => self.server_of(seat),
                    Side::Server => Some(seat),
                };
                if let Some(s) = self.play.as_mut() {
                    s.game.sit_on = seat;
                }
            }
        }
        for (h, state, on) in reqs.state_enabled {
            if mine(h, self) {
                if let Some(s) = self.play.as_mut() {
                    if on {
                        s.game.states_off.remove(&state);
                    } else {
                        s.game.states_off.insert(state);
                    }
                }
            }
        }
    }

    /// The part an NPC Humanoid moves with.
    fn npc_root(&self, humanoid: Ref) -> Option<Ref> {
        let d = &self.doc;
        let model = d.dom.get_by_ref(humanoid)?.parent();
        if let Some(Variant::Ref(r)) = d
            .prop(humanoid, "RootPart")
            .or_else(|| d.prop(model, "PrimaryPart"))
        {
            if r.is_some() && d.exists(r) {
                return Some(r);
            }
        }
        d.find_named(model, "HumanoidRootPart")
            .or_else(|| d.find_named(model, "Torso"))
    }

    // ------------------------------------------------------------- tools

    fn backpack(&self) -> Option<Ref> {
        let p = self.play.as_ref()?.player;
        self.doc.find_child(p, "Backpack")
    }

    fn character_model(&self) -> Option<Ref> {
        self.play
            .as_ref()?
            .character
            .as_ref()
            .filter(|c| !c.dead)
            .map(|c| c.model)
    }

    fn equip_tool(&mut self, tool: Ref) {
        let Some(model) = self.character_model() else {
            return;
        };
        if !self
            .doc
            .class_of(tool)
            .is_some_and(|c| doc::is_a(&c, "BackpackItem"))
        {
            return;
        }
        self.unequip_tool();
        self.with_side(Side::Server, |rt, _, _| rt.apply_parent(tool, model));
        let handle = self.doc.find_child(tool, "Handle").filter(|h| {
            self.doc
                .class_of(*h)
                .is_some_and(|c| doc::is_a(&c, "BasePart"))
        });
        let parts: Vec<Ref> = self
            .doc
            .dom
            .descendants_of(tool)
            .filter(|i| doc::is_a(&i.class, "BasePart"))
            .map(|i| i.referent())
            .collect();
        let base = handle.and_then(|h| crate::scene::xform_of(&self.doc, h));
        let offsets: Vec<(Ref, Xform)> = match base {
            Some(b) => parts
                .iter()
                .filter_map(|p| {
                    crate::scene::xform_of(&self.doc, *p)
                        .map(|x| (*p, crate::ltypes::cf_mul(&crate::ltypes::cf_inv(&b), &x)))
                })
                .collect(),
            None => Vec::new(),
        };
        if let Some(s) = self.play.as_mut() {
            s.physics.skip.extend(parts.iter().copied());
            s.physics.refresh(parts);
            s.game.equipped = Some(Equipped {
                tool,
                offsets,
                client_told: false,
            });
        }
        self.with_side(Side::Server, |rt, _, _| {
            rt.fire(tool, "Equipped", vec![Value::Nil])
        });
    }

    fn unequip_tool(&mut self) {
        let Some(e) = self.play.as_mut().and_then(|s| s.game.equipped.take()) else {
            return;
        };
        let parts: Vec<Ref> = e.offsets.iter().map(|(r, _)| *r).collect();
        if let Some(s) = self.play.as_mut() {
            s.physics.skip.retain(|r| !parts.contains(r));
            s.physics.refresh(parts);
        }
        if let Some(bp) = self.backpack() {
            if self.doc.exists(e.tool) {
                self.with_side(Side::Server, |rt, _, _| rt.apply_parent(e.tool, bp));
            }
        }
        self.fire_both(e.tool, "Unequipped");
    }

    /// Where the equipped tool's parts go, given this frame's body pose.
    pub(super) fn tool_pose(&self, pose: &[(Ref, Xform)]) -> Vec<(Ref, Xform)> {
        let Some(s) = self.play.as_ref() else {
            return Vec::new();
        };
        let (Some(e), Some(ch)) = (s.game.equipped.as_ref(), s.character.as_ref()) else {
            return Vec::new();
        };
        let d = &self.doc;
        let hand = d
            .find_named(ch.model, "RightHand")
            .or_else(|| d.find_named(ch.model, "Right Arm"));
        let Some(hand) = hand else { return Vec::new() };
        let Some(hand_xf) = pose.iter().find(|(r, _)| *r == hand).map(|(_, x)| *x) else {
            return Vec::new();
        };
        // RightGrip: hand * grip attachment (turned to point forward) * Tool.Grip⁻¹.
        let att =
            d.find_named(hand, "RightGripAttachment")
                .and_then(|a| match d.prop(a, "CFrame") {
                    Some(Variant::CFrame(cf)) => Some(Xform::from_cframe(&cf)),
                    _ => None,
                });
        let mut c0 = att.unwrap_or(Xform {
            pos: Vec3::new(0.0, -1.0, 0.0),
            rot: Mat3::IDENTITY,
        });
        if c0.rot.abs_diff_eq(Mat3::IDENTITY, 1e-4) {
            c0.rot = Mat3::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        }
        let grip = match d.prop(e.tool, "Grip") {
            Some(Variant::CFrame(cf)) => Xform::from_cframe(&cf),
            _ => Xform {
                pos: Vec3::ZERO,
                rot: Mat3::IDENTITY,
            },
        };
        use crate::ltypes::{cf_inv, cf_mul};
        let handle_xf = cf_mul(&cf_mul(&hand_xf, &c0), &cf_inv(&grip));
        e.offsets
            .iter()
            .map(|(r, off)| (*r, cf_mul(&handle_xf, off)))
            .collect()
    }

    /// Keeps the hotbar list in step with the Backpack and the equipped tool.
    fn update_slots(&mut self) {
        let mut present: Vec<Ref> = Vec::new();
        if let Some(bp) = self.backpack() {
            if let Some(i) = self.doc.dom.get_by_ref(bp) {
                present.extend(i.children().iter().copied().filter(|c| {
                    self.doc
                        .class_of(*c)
                        .is_some_and(|k| doc::is_a(&k, "BackpackItem"))
                }));
            }
        }
        if let Some(e) = self.play.as_ref().and_then(|s| s.game.equipped.as_ref()) {
            if self.doc.exists(e.tool) {
                present.push(e.tool);
            }
        }
        let Some(s) = self.play.as_mut() else { return };
        s.game.slots.retain(|t| present.contains(t));
        for t in present {
            if !s.game.slots.contains(&t) {
                s.game.slots.push(t);
            }
        }
    }

    // --------------------------------------------------------- explosions

    fn explode(&mut self, e: Ref) {
        let d = &self.doc;
        let pos = vec3_of(d.prop(e, "Position")).unwrap_or(Vec3::ZERO);
        let radius = f32_prop(d, e, "BlastRadius", 4.0).max(0.0);
        let pressure = f32_prop(d, e, "BlastPressure", 500_000.0).max(0.0);
        let joint_radius =
            radius * f32_prop(d, e, "DestroyJointRadiusPercent", 1.0).clamp(0.0, 1.0);
        let Some(ws) = d.workspace() else { return };
        let mut hits: Vec<(Ref, f32, Vec3)> = Vec::new();
        for i in d.dom.descendants_of(ws) {
            if i.class == "Terrain" || !doc::is_a(&i.class, "BasePart") {
                continue;
            }
            let r = i.referent();
            let Some(xf) = crate::scene::xform_of(d, r) else {
                continue;
            };
            let size = vec3_of(d.prop(r, "Size")).unwrap_or(Vec3::ONE);
            let dist = ((xf.pos - pos).length() - size.length() * 0.5).max(0.0);
            if dist <= radius {
                hits.push((r, dist, xf.pos));
            }
        }
        let now = self.play.as_ref().map(|s| s.time).unwrap_or(0.0);
        if let Some(s) = self.play.as_mut() {
            s.game.flashes.push(Flash {
                pos,
                radius,
                born: now,
            });
        }
        // Hit events, a push outward, broken joints and dead humanoids.
        let mut humanoids: HashSet<Ref> = HashSet::new();
        let mut break_parts: HashSet<Ref> = HashSet::new();
        for (r, dist, at) in &hits {
            let (r, dist, at) = (*r, *dist, *at);
            self.with_side(Side::Server, |rt, _, _| {
                rt.fire(e, "Hit", vec![rt.inst(r), Value::Number(dist as f64)])
            });
            let anchored = matches!(self.doc.prop(r, "Anchored"), Some(Variant::Bool(true)));
            if !anchored {
                let size = vec3_of(self.doc.prop(r, "Size")).unwrap_or(Vec3::ONE);
                let mass = (size.x * size.y * size.z * mech::PART_DENSITY).max(0.1);
                let dir = (at - pos).normalize_or(Vec3::Y);
                let dv = (pressure * mech::EXPLOSION_PUSH / mass).min(mech::EXPLOSION_MAX_SPEED);
                if let Some(s) = self.play.as_mut() {
                    let v = s
                        .physics
                        .velocities()
                        .get(&r)
                        .copied()
                        .unwrap_or(Vec3::ZERO);
                    s.physics.set_velocity(r, v + dir * dv);
                }
            }
            if dist <= joint_radius {
                break_parts.insert(r);
                if let Some(model) = self.doc.dom.get_by_ref(r).map(|i| i.parent()) {
                    if let Some(h) = self.doc.find_child(model, "Humanoid") {
                        humanoids.insert(h);
                    }
                }
            }
        }
        let joints: Vec<Ref> = self
            .doc
            .dom
            .descendants_of(ws)
            .filter(|i| matches!(i.class.as_str(), "Weld" | "ManualWeld" | "Snap" | "Glue" | "WeldConstraint" | "Motor6D" | "Motor"))
            .filter(|i| ["Part0", "Part1"].iter().any(|p| matches!(i.properties.get(&ustr(p)), Some(Variant::Ref(x)) if break_parts.contains(x))))
            .map(|i| i.referent())
            .collect();
        let mine = self
            .play
            .as_ref()
            .and_then(|s| s.character.as_ref().map(|c| c.humanoid));
        self.with_side(Side::Server, |rt, _, _| {
            for j in &joints {
                rt.apply_parent(*j, Ref::none());
            }
            for h in &humanoids {
                rt.set(*h, "Health", Value::Number(0.0));
            }
        });
        if mine.is_some_and(|m| humanoids.contains(&m)) {
            self.kill_character();
        }
    }

    // ------------------------------------------------------------- sounds

    fn sound_bytes(&mut self, id: &str) -> crate::assets::Fetch {
        use crate::assets::Fetch;
        let dir = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        if let Some(path) = crate::assets::resolve(id, dir.as_deref()) {
            return match std::fs::read(&path) {
                Ok(b) => Fetch::Ready(Arc::new(b)),
                Err(e) => Fetch::Unavailable(format!("{}: {e}", path.display())),
            };
        }
        match crate::assets::asset_ref(id) {
            Some(a) => self.assets.fetch(a),
            None => Fetch::Unavailable("no SoundId".into()),
        }
    }

    /// The DataModel that has `r` (server in front during play_tick).
    fn sound_doc(&self, server: Option<Ref>, client: Option<Ref>) -> Option<(&Document, Ref)> {
        if let (Some(c), Some(cd)) = (
            client,
            self.play.as_ref().and_then(|s| s.other_doc.as_ref()),
        ) {
            if cd.exists(c) {
                return Some((cd, c));
            }
        }
        server
            .filter(|r| self.doc.exists(*r))
            .map(|r| (&self.doc, r))
    }

    fn sound_command(&mut self, server: Option<Ref>, client: Option<Ref>, cmd: SoundCmd) {
        let Some(key_ref) = client.or(server) else {
            return;
        };
        let key = sound_key(key_ref);
        if let Some(s) = self.play.as_mut() {
            s.game.tracks.entry(key).or_insert(Track {
                server,
                client,
                pending: None,
                id: None,
            });
        }
        match cmd {
            SoundCmd::Play => self.start_sound(key),
            SoundCmd::Stop | SoundCmd::Pause | SoundCmd::Resume => {
                if let Some(a) = self.play.as_mut().and_then(|s| s.game.audio.as_mut()) {
                    match cmd {
                        SoundCmd::Stop => a.stop(key),
                        SoundCmd::Pause => a.pause(key),
                        _ => a.resume(key),
                    }
                }
                if cmd == SoundCmd::Stop {
                    if let Some(t) = self.play.as_mut().and_then(|s| s.game.tracks.get_mut(&key)) {
                        t.pending = None;
                    }
                }
            }
        }
    }

    fn audio_listener(&self) -> Option<(u32, Option<crate::math::Xform>, Option<Ref>)> {
        let s = self.play.as_ref()?;
        s.client
            .as_ref()
            .and_then(|c| c.world.as_ref())
            .or(s.server.world.as_ref())
            .and_then(|w| w.input.sound_listener)
    }
    fn start_sound(&mut self, key: u64) {
        let Some((server, client)) = self
            .play
            .as_ref()
            .and_then(|s| s.game.tracks.get(&key))
            .map(|t| (t.server, t.client))
        else {
            return;
        };
        let Some((id, volume, speed, looped, start, region)) =
            self.sound_doc(server, client).map(|(d, r)| {
                let range = |k: &str| match d.prop(r, k) {
                    Some(Variant::NumberRange(n)) => Some((n.min as f64, n.max as f64)),
                    _ => None,
                };
                // Sound.PlaybackRegionsEnabled: play PlaybackRegion, loop LoopRegion.
                let region = if matches!(
                    d.prop(r, "PlaybackRegionsEnabled"),
                    Some(Variant::Bool(true))
                ) {
                    crate::audio::Region {
                        play: range("PlaybackRegion"),
                        looped: range("LoopRegion"),
                    }
                } else {
                    crate::audio::Region::default()
                };
                let id = match d.prop(r, "SoundId") {
                    Some(Variant::ContentId(c)) => c.as_str().to_string(),
                    Some(Variant::Content(c)) => c.as_uri().unwrap_or("").to_string(),
                    Some(Variant::String(s)) => s,
                    _ => String::new(),
                };
                (
                    id,
                    f32_prop(d, r, "Volume", 0.5),
                    f32_prop(d, r, "PlaybackSpeed", 1.0),
                    matches!(d.prop(r, "Looped"), Some(Variant::Bool(true))),
                    f32_prop(d, r, "TimePosition", 0.0) as f64,
                    region,
                )
            })
        else {
            return;
        };
        if id.trim().is_empty() {
            return;
        }
        if self
            .play
            .as_ref()
            .is_some_and(|s| s.game.sound_unavailable.contains(&id))
        {
            return;
        }
        if let Some(s) = self.play.as_mut() {
            *s.game.sound_fetches.entry(id.clone()).or_insert(0) += 1;
        }
        match self.sound_bytes(&id) {
            crate::assets::Fetch::Ready(bytes) => {
                let graph = self.sound_doc(server, client).map(|(d, r)| {
                    crate::audio_scene::settings(
                        d,
                        r,
                        self.cam.pos,
                        self.cam.right(),
                        self.audio_listener(),
                    )
                });
                let len = Audio::length(&bytes);
                let Some(s) = self.play.as_mut() else { return };
                let audio = s.game.audio.get_or_insert_with(Audio::new);
                if let Some((groups, settings)) = graph {
                    audio.configure(key, groups, settings);
                }
                if let Err(e) = audio.play(key, bytes, volume, speed, looped, start, region) {
                    if s.game.warned.insert(id.clone()) {
                        self.warn(format!("Sound {id} can't play: {e}"));
                    }
                    return;
                }
                if let Some(t) = self.play.as_mut().and_then(|s| s.game.tracks.get_mut(&key)) {
                    t.pending = None;
                    t.id = Some(id.clone());
                }
                self.log_audio_start(key, &id);
                if let Some(len) = len {
                    self.set_sound_info(server, client, |inp, r| {
                        inp.sound_len.insert(r, len);
                    });
                }
            }
            crate::assets::Fetch::Pending => {
                if let Some(t) = self.play.as_mut().and_then(|s| s.game.tracks.get_mut(&key)) {
                    t.pending = Some(id);
                }
            }
            crate::assets::Fetch::Unavailable(why) => {
                // Given up on: no per-frame retry, one warning, the game goes on.
                if let Some(s) = self.play.as_mut() {
                    if let Some(t) = s.game.tracks.get_mut(&key) {
                        t.pending = None;
                    }
                    s.game.sound_unavailable.insert(id.clone());
                }
                let first = self
                    .play
                    .as_mut()
                    .is_some_and(|s| s.game.warned.insert(id.clone()));
                if first {
                    self.warn(format!("Sound {id} isn't available: {why}"));
                }
            }
        }
    }

    /// RUSTUDIO_AUDIO_LOG=FILE: one line per Audio::play start (key, sound id,
    /// frame, how many times this key has started), to tell a caller that
    /// re-triggers from a mixer that repeats.
    fn log_audio_start(&mut self, key: u64, id: &str) {
        let frame = self.frame_no;
        let Some(s) = self.play.as_mut() else { return };
        *s.game.audio_starts_by_id.entry(id.to_string()).or_insert(0) += 1;
        let n = s.game.audio_starts.entry(key).or_insert(0);
        *n += 1;
        let n = *n;
        let live = s.game.audio.as_ref().map_or(0, |a| a.playing_count());
        if let Some(path) = std::env::var_os("RUSTUDIO_AUDIO_LOG") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "start key={key:016x} id={id} frame={frame} t={:.3} count={n} live={live}", s.time);
            }
        }
    }

    /// Updates the scripts' view of a sound on both sides.
    fn set_sound_info(
        &mut self,
        server: Option<Ref>,
        client: Option<Ref>,
        f: impl Fn(&mut crate::runtime::ext::Inputs, Ref),
    ) {
        let Some(s) = self.play.as_mut() else { return };
        if let (Some(r), Some(w)) = (server, s.server.world.as_mut()) {
            f(&mut w.input, r);
        }
        if let (Some(r), Some(w)) = (client, s.client.as_mut().and_then(|c| c.world.as_mut())) {
            f(&mut w.input, r);
        }
    }

    /// Per frame: NPC walking, sounds (positions, 3D volume, Ended), the hotbar.
    pub(super) fn game_tick_end(&mut self) {
        let now = self.play.as_ref().map(|s| s.time).unwrap_or(0.0);

        // NPC Humanoid:MoveTo.
        let walks: Vec<(Ref, Vec3, f64)> = self
            .play
            .as_ref()
            .map(|s| {
                s.game
                    .walks
                    .iter()
                    .map(|(h, w)| (*h, w.target, w.since))
                    .collect()
            })
            .unwrap_or_default();
        let mut finished: Vec<(Ref, bool)> = Vec::new();
        for (h, target, since) in walks {
            let Some(root) = self.npc_root(h) else {
                finished.push((h, false));
                continue;
            };
            let Some(pos) = crate::scene::xform_of(&self.doc, root).map(|x| x.pos) else {
                continue;
            };
            let mut to = target - pos;
            to.y = 0.0;
            let speed = f32_prop(&self.doc, h, "WalkSpeed", mech::WALK_SPEED);
            let reached = to.length() < mech::MOVE_TO_REACHED;
            let timed_out = now - since > mech::MOVE_TO_TIMEOUT_S as f64;
            let Some(s) = self.play.as_mut() else { break };
            let v = s
                .physics
                .velocities()
                .get(&root)
                .copied()
                .unwrap_or(Vec3::ZERO);
            if reached || timed_out {
                s.physics.set_velocity(root, Vec3::new(0.0, v.y, 0.0));
                finished.push((h, reached));
            } else {
                let dir = to.normalize();
                s.physics
                    .set_velocity(root, Vec3::new(dir.x * speed, v.y, dir.z * speed));
            }
        }
        for (h, reached) in finished {
            if let Some(s) = self.play.as_mut() {
                s.game.walks.remove(&h);
            }
            self.with_side(Side::Server, |rt, _, _| {
                rt.fire(h, "MoveToFinished", vec![Value::Boolean(reached)])
            });
        }

        // A Sound taken out of the game (Destroy, Debris, Parent = nil)
        // stops, as in Roblox: Place A removes each SoundBank clip with Debris.
        let gone: Vec<u64> = self
            .play
            .as_ref()
            .map(|s| {
                let alive = |d: Option<&Document>, r: Option<Ref>| {
                    d.zip(r)
                        .is_some_and(|(d, r)| d.exists(r) && d.is_ancestor(d.dom.root_ref(), r))
                };
                s.game
                    .tracks
                    .iter()
                    .filter(|(_, t)| {
                        !alive(Some(&self.doc), t.server) && !alive(s.other_doc.as_ref(), t.client)
                    })
                    .map(|(k, _)| *k)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(s) = self.play.as_mut() {
            for k in gone {
                s.game.tracks.remove(&k);
                if let Some(a) = s.game.audio.as_mut() {
                    a.stop(k);
                }
            }
        }

        // Sounds still downloading.
        let pending: Vec<u64> = self
            .play
            .as_ref()
            .map(|s| {
                s.game
                    .tracks
                    .iter()
                    .filter(|(_, t)| t.pending.is_some())
                    .map(|(k, _)| *k)
                    .collect()
            })
            .unwrap_or_default();
        for k in pending {
            self.start_sound(k);
        }

        // Ended / DidLoop, playback position and 3D volume.
        let cam = self.cam.pos;
        let (done, looped) = match self.play.as_mut().and_then(|s| s.game.audio.as_mut()) {
            Some(a) => (a.poll_finished(), a.looped_since_poll()),
            None => (Vec::new(), Vec::new()),
        };
        for k in done {
            let Some((server, client)) = self
                .play
                .as_ref()
                .and_then(|s| s.game.tracks.get(&k))
                .map(|t| (t.server, t.client))
            else {
                continue;
            };
            self.set_sound_info(server, client, |inp, r| {
                inp.sound_playing.remove(&r);
                inp.sound_paused.remove(&r);
            });
            if let Some(r) = server.filter(|r| self.doc.exists(*r)) {
                if let Some(i) = self.doc.dom.get_by_ref_mut(r) {
                    i.properties.insert(ustr("Playing"), Variant::Bool(false));
                }
                self.with_side(Side::Server, |rt, _, _| rt.fire(r, "Ended", vec![]));
            }
            if let Some(c) = client {
                self.with_side(Side::Client, |rt, _, _| rt.fire(c, "Ended", vec![]));
            }
        }
        for k in looped {
            let Some((server, client)) = self
                .play
                .as_ref()
                .and_then(|s| s.game.tracks.get(&k))
                .map(|t| (t.server, t.client))
            else {
                continue;
            };
            if let Some(r) = server {
                self.with_side(Side::Server, |rt, _, _| rt.fire(r, "DidLoop", vec![]));
            }
            if let Some(c) = client {
                self.with_side(Side::Client, |rt, _, _| rt.fire(c, "DidLoop", vec![]));
            }
        }
        let tracks: Vec<(u64, Option<Ref>, Option<Ref>)> = self
            .play
            .as_ref()
            .map(|s| {
                s.game
                    .tracks
                    .iter()
                    .map(|(k, t)| (*k, t.server, t.client))
                    .collect()
            })
            .unwrap_or_default();
        for (k, server, client) in tracks {
            let playing = self
                .play
                .as_ref()
                .and_then(|s| s.game.audio.as_ref())
                .is_some_and(|a| a.is_playing(k));
            if !playing {
                continue;
            }
            let pos = self
                .play
                .as_ref()
                .and_then(|s| s.game.audio.as_ref())
                .map(|a| a.position(k))
                .unwrap_or(0.0);
            self.set_sound_info(server, client, |inp, r| {
                inp.sound_pos.insert(r, pos);
            });
            let settings = self.sound_doc(server, client).map(|(d, r)| {
                (
                    f32_prop(d, r, "Volume", 0.5).clamp(0., 10.),
                    f32_prop(d, r, "PlaybackSpeed", 1.),
                    matches!(d.prop(r, "Looped"), Some(Variant::Bool(true))),
                    crate::audio_scene::settings(d, r, cam, self.cam.right(), self.audio_listener()),
                )
            });
            if let (Some((vol, speed, looped, (groups, settings))), Some(a)) = (
                settings,
                self.play.as_mut().and_then(|s| s.game.audio.as_mut()),
            ) {
                a.configure(k, groups, settings);
                a.set_volume(k, vol);
                a.set_speed(k, speed);
                a.set_looped(k, looped);
            }
        }

        if let Some(s) = self.play.as_mut() {
            s.game
                .flashes
                .retain(|f| now - f.born < mech::EXPLOSION_FLASH_S as f64);
            s.game
                .chat_log
                .retain(|(_, t)| now - t < mech::CHAT_SHOW_S as f64);
        }
        self.update_slots();
    }

    /// Sounds saved with Playing = true start when the game does. Not the ones
    /// a script already started during its first run (their Play() sets
    /// Playing = true too and is queued): that started every such sound twice.
    pub(super) fn start_autoplay_sounds(&mut self) {
        let queued: HashSet<Ref> = self
            .play
            .as_ref()
            .map(|s| {
                let server = s.server.world.as_ref().map(|w| &w.requests.sounds);
                let client = s
                    .client
                    .as_ref()
                    .and_then(|c| c.world.as_ref())
                    .map(|w| &w.requests.sounds);
                server
                    .into_iter()
                    .chain(client)
                    .flatten()
                    .map(|(r, _)| *r)
                    .collect()
            })
            .unwrap_or_default();
        let sounds: Vec<Ref> = self
            .doc
            .dom
            .descendants()
            .filter(|i| {
                i.class == "Sound"
                    && matches!(
                        i.properties.get(&ustr("Playing")),
                        Some(Variant::Bool(true))
                    )
            })
            .map(|i| i.referent())
            .filter(|r| !queued.contains(r))
            .collect();
        for r in sounds {
            if self
                .doc
                .workspace()
                .is_some_and(|ws| self.doc.is_ancestor(ws, r))
                || self
                    .doc
                    .find_child(self.doc.dom.root_ref(), "SoundService")
                    .is_some_and(|s| self.doc.is_ancestor(s, r))
            {
                let client = self.client_of(r);
                self.sound_command(Some(r), client, SoundCmd::Play);
                self.set_sound_info(Some(r), client, |inp, x| {
                    inp.sound_playing.insert(x);
                });
            }
        }
    }

    pub(super) fn stop_all_sounds(&mut self) {
        if let Some(a) = self.play.as_mut().and_then(|s| s.game.audio.as_mut()) {
            a.stop_all();
        }
    }

    // ------------------------------------------------------- viewport side

    /// During Play with a character: collects mouse / key input for the game
    /// and draws the hotbar, chat and explosion flashes. Returns true when
    /// clicks belong to the game (no selecting).
    pub fn game_viewport(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        resp: &egui::Response,
        gui_took: bool,
    ) -> bool {
        let Some(side) = self
            .play
            .as_ref()
            .filter(|p| p.mode == RunMode::Play)
            .map(|p| p.showing)
        else {
            return false;
        };
        let ctx = ui.ctx().clone();
        let pointer = resp.hover_pos().map(|p| {
            let ray = self.cam.ray(rect, p);
            let hit = self.pick_any(&ray).map(|(r, t)| (r, ray.at(t)));
            Pointer {
                origin: ray.origin,
                dir: ray.dir,
                hit,
                xy: [p.x - rect.min.x, p.y - rect.min.y],
                view: [rect.width(), rect.height()],
                side,
            }
        });
        let mut clicks = Vec::new();
        // A click the GUI took doesn't reach the world (ClickDetectors, Mouse, tools).
        if let Some((r, _)) = pointer.and_then(|p| p.hit).filter(|_| !gui_took) {
            if resp.clicked_by(egui::PointerButton::Primary) {
                clicks.push((r, 1));
            }
            if resp.clicked_by(egui::PointerButton::Secondary) {
                clicks.push((r, 2));
            }
        }
        // Any key, click or mouse move is activity (Player.Idled).
        if ctx.input(|i| !i.events.is_empty() || i.pointer.delta() != egui::Vec2::ZERO) {
            if let Some(s) = self.play.as_mut() {
                s.game.last_activity = s.time;
            }
        }
        let mut buttons = Vec::new();
        let mut uis = Vec::new();
        if resp.hovered() {
            ctx.input(|i| {
                for e in &i.events {
                    if let egui::Event::PointerButton {
                        button, pressed, ..
                    } = e
                    {
                        let b = match button {
                            egui::PointerButton::Primary => 1,
                            egui::PointerButton::Secondary => 2,
                            _ => continue,
                        };
                        // UserInputService hears every click (gameProcessedEvent when the GUI took it).
                        uis.push((b, *pressed, gui_took));
                        if !gui_took {
                            buttons.push((b, *pressed));
                        }
                    }
                }
            });
        }
        // Number keys pick hotbar slots; "/" opens chat.
        let mut slots = Vec::new();
        let mut open_chat = false;
        if !ctx.egui_wants_keyboard_input() {
            use egui::Key::*;
            ctx.input(|i| {
                for (n, k) in [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9]
                    .iter()
                    .enumerate()
                {
                    if i.key_pressed(*k) {
                        slots.push(n);
                    }
                }
                if i.key_pressed(Slash) {
                    open_chat = true;
                }
            });
        }
        // StarterGui:SetCoreGuiEnabled(Chat, false) hides Roblox's chat (Place A's
        // menu does; Studio shows no chat there).
        let chat_on = self
            .play
            .as_ref()
            .and_then(|s| s.client.as_ref())
            .and_then(|c| c.world.as_ref())
            .is_none_or(|w| !w.core_gui_off.contains("Chat"));
        open_chat &= chat_on;
        // The open menu has the mouse and keys: none of it reaches the game
        // (SettingsHub's Modal shield and RbxSettingsHubStopCharacter).
        let menu = self.play.as_ref().is_some_and(|s| s.game.menu_open);
        if menu {
            clicks.clear();
            buttons.clear();
            slots.clear();
            open_chat = false;
            // A button let go while it's open still ends what its press began
            // (UserInputService.InputEnded, gameProcessedEvent true): a camera
            // pan or a held drag can't stay stuck holding the mouse after Resume.
            uis = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, .. } => Some((1, false, true)),
                        egui::Event::PointerButton { button: egui::PointerButton::Secondary, pressed: false, .. } => Some((2, false, true)),
                        _ => None,
                    })
                    .collect()
            });
        }
        let painter = ui.painter_at(rect);
        let now = self.play.as_ref().map(|s| s.time).unwrap_or(0.0);

        // Explosion flashes.
        let flashes: Vec<(Vec3, f32, f64)> = self
            .play
            .as_ref()
            .map(|s| {
                s.game
                    .flashes
                    .iter()
                    .map(|f| (f.pos, f.radius, f.born))
                    .collect()
            })
            .unwrap_or_default();
        for (pos, radius, born) in flashes {
            let age = ((now - born) as f32 / mech::EXPLOSION_FLASH_S).clamp(0.0, 1.0);
            if let (Some(c), Some(edge)) = (
                self.cam.project(rect, pos),
                self.cam
                    .project(rect, pos + self.cam.right() * radius.max(1.0) * (0.6 + age)),
            ) {
                let a = ((1.0 - age) * 200.0) as u8;
                painter.circle_filled(
                    c,
                    (edge - c).length(),
                    Color32::from_rgba_unmultiplied(255, 150, 40, a / 2),
                );
                painter.circle_stroke(
                    c,
                    (edge - c).length(),
                    Stroke::new(3.0, Color32::from_rgba_unmultiplied(255, 220, 120, a)),
                );
            }
        }

        // Hotbar.
        let slot_tools: Vec<Ref> = self
            .play
            .as_ref()
            .map(|s| s.game.slots.clone())
            .unwrap_or_default();
        let equipped = self
            .play
            .as_ref()
            .and_then(|s| s.game.equipped.as_ref().map(|e| e.tool));
        if !slot_tools.is_empty() {
            let w = 64.0;
            let total = slot_tools.len() as f32 * (w + 6.0) - 6.0;
            let mut x = rect.center().x - total / 2.0;
            let y = rect.max.y - w - 14.0;
            for (n, t) in slot_tools.iter().enumerate() {
                let r = Rect::from_min_size(Pos2::new(x, y), egui::vec2(w, w));
                let on = Some(*t) == equipped;
                painter.rect_filled(
                    r,
                    2.0,
                    Color32::from_rgba_unmultiplied(20, 22, 28, if on { 230 } else { 170 }),
                );
                painter.rect_stroke(
                    r,
                    2.0,
                    Stroke::new(
                        if on { 2.0 } else { 1.0 },
                        if on {
                            theme::ACCENT
                        } else {
                            Color32::from_gray(90)
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    r.min + egui::vec2(5.0, 3.0),
                    egui::Align2::LEFT_TOP,
                    format!("{}", n + 1),
                    egui::FontId::proportional(11.0),
                    Color32::from_gray(170),
                );
                let name = self
                    .doc
                    .dom
                    .get_by_ref(*t)
                    .map(|i| i.name.clone())
                    .unwrap_or_default();
                painter.text(
                    r.center() + egui::vec2(0.0, 6.0),
                    egui::Align2::CENTER_CENTER,
                    name,
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
                let click = ui.interact(r, ui.id().with(("slot", n)), egui::Sense::click());
                if click.clicked() {
                    slots.push(n);
                }
                x += w + 6.0;
            }
        }

        // Chat: TextChatService's default chat (window, input bar, bubbles,
        // drawn as Studio's ExpChat lays them out), else the legacy lines.
        // Bubbles (Chat:Chat, so Dialog speech too) show either way: Studio's
        // ExpChat bubble chat runs whatever the place's ChatVersion, and
        // SetCoreGuiEnabled(Chat, false) hides only the window and bar.
        // (with_side needs the server's DataModel in front.)
        self.server_front();
        let view = self
            .with_side(Side::Client, |rt, _, _| {
                crate::runtime::domains::players_chat::chat_view(&rt.lua)
            })
            .flatten();
        self.shown_front();
        let chat_open_now = self.play.as_ref().is_some_and(|s| s.game.chat_open);
        let input_rect = match &view {
            Some(v) if v.tcs && chat_on => draw_text_chat(&painter, rect, v, chat_open_now, &self.cam),
            _ => {
                draw_legacy_chat(self, &painter, rect, chat_on);
                if let Some(v) = &view {
                    draw_chat_bubbles(&painter, rect, v, &self.cam);
                }
                None
            }
        };
        if let Some(s) = self.play.as_ref().and_then(|s| s.game.kicked.clone()) {
            draw_kicked(&painter, rect, &s);
        }
        self.game_menu(ui, rect);
        let mut sent: Option<String> = None;
        if let Some(s) = self.play.as_mut() {
            if open_chat {
                s.game.chat_open = true;
            }
            let box_rect = input_rect.unwrap_or(Rect::from_min_size(
                Pos2::new(rect.min.x + 10.0, rect.max.y - 38.0),
                egui::vec2(320.0f32.min(rect.width() - 20.0), 26.0),
            ));
            if s.game.chat_open {
                let hint = if input_rect.is_some() {
                    "To chat click here or press \"/\" key"
                } else {
                    "Say something (Enter to send, Esc to close)"
                };
                let mut edit = egui::TextEdit::singleline(&mut s.game.chat_text)
                    .hint_text(hint)
                    .id(egui::Id::new("play_chat"));
                if input_rect.is_some() {
                    edit = edit.frame(egui::Frame::NONE).text_color(Color32::WHITE);
                }
                let r = ui.put(box_rect, edit);
                if open_chat {
                    r.request_focus();
                }
                let (enter, esc) = ctx.input(|i| {
                    (
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::Escape),
                    )
                });
                if r.lost_focus() || esc {
                    if enter && !s.game.chat_text.trim().is_empty() {
                        sent = Some(std::mem::take(&mut s.game.chat_text));
                    }
                    s.game.chat_open = false;
                }
            } else if chat_on {
                // Studio's playtest shows no "/ to chat" hint (the Studio
                // an offline Studio runs); "/" still opens the chat box.
                s.game.chat_ready_frames += 1;
            }
            if let Ok(mut l) = crate::runtime::domains::players_chat::CHAT_LAYOUT.lock() {
                l.focused = s.game.chat_open;
            }
            if !menu {
                s.game.input.pointer = pointer;
            }
            s.game.input.clicks.extend(clicks);
            s.game.input.buttons.extend(buttons);
            s.game.uis_mouse.extend(uis);
            s.game.input.slots.extend(slots);
            if let Some(m) = sent {
                s.game.input.chat.push(m);
            }
            if s.game.clickable && !menu {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        true
    }

    /// The nearest part along a ray (any part, locked or not).
    fn pick_any(&self, ray: &crate::math::Ray) -> Option<(Ref, f32)> {
        // The player's own character and tool don't block the mouse.
        let own: HashSet<Ref> = self
            .play
            .as_ref()
            .map(|s| match s.showing {
                Side::Server => s.physics.skip.clone(),
                Side::Client => s
                    .physics
                    .skip
                    .iter()
                    .filter_map(|r| s.map.s2c.get(r).copied())
                    .collect(),
            })
            .unwrap_or_default();
        let mut best: Option<(f32, Ref)> = None;
        for (sp, b) in self.scene.parts.iter().zip(&self.scene.bounds) {
            let oc = b.truncate() - ray.origin;
            let along = oc.dot(ray.dir);
            if oc.length_squared() - along * along > b.w * b.w || along < -b.w {
                continue;
            }
            if own.contains(&sp.r) {
                continue;
            }
            if let Some(t) = ray.hit_obb(&sp.xf, sp.size) {
                if t > 0.0 && best.is_none_or(|(bt, _)| t < bt) {
                    best = Some((t, sp.r));
                }
            }
        }
        best.map(|(t, r)| (r, t))
    }
}

/// Roblox's default chat name colours (the legacy chat's and ExpChat's
/// `GetNameValue` hash over the name).
fn chat_name_color(name: &str) -> Color32 {
    const COLORS: [[u8; 3]; 8] = [
        [253, 41, 67],
        [1, 162, 255],
        [2, 184, 87],
        [107, 50, 124],
        [218, 133, 65],
        [245, 205, 48],
        [232, 186, 200],
        [215, 197, 154],
    ];
    let b = name.as_bytes();
    let n = b.len() as i64;
    let mut value: i64 = 0;
    for (i, c) in b.iter().enumerate() {
        let mut rev = n - i as i64;
        if n % 2 == 1 {
            rev -= 1;
        }
        let v = *c as i64;
        value += if rev % 4 >= 2 { -v } else { v };
    }
    let c = COLORS[value.rem_euclid(8) as usize];
    Color32::from_rgb(c[0], c[1], c[2])
}

fn c32(c: [f32; 3], alpha: f32) -> Color32 {
    let u = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(u(c[0]), u(c[1]), u(c[2]), u(alpha))
}

/// The TextChatService window (top left under the top bar, Studio's
/// ChatWindowConfiguration 8,12 344.8x154.75), its input bar while open,
/// and the bubbles over characters. Returns where the input box goes.
fn draw_text_chat(
    painter: &egui::Painter,
    rect: Rect,
    v: &crate::runtime::domains::players_chat::ChatView,
    open: bool,
    cam: &crate::camera::Camera,
) -> Option<Rect> {
    use egui::text::{LayoutJob, TextFormat};
    // ExpChat's container starts under the 58 px top bar.
    const INSET: f32 = 58.0;
    const TEXT_FADE_S: f64 = 30.0;
    let w = &v.window;
    let win = Rect::from_min_size(
        Pos2::new(rect.min.x + 8.0, rect.min.y + INSET + 12.0),
        egui::vec2(344.8 * w.width_scale, 154.75 * w.height_scale),
    );
    let bar = Rect::from_min_size(
        Pos2::new(rect.min.x + 16.0, win.max.y),
        egui::vec2(win.width() - 16.0, 43.0),
    );
    if let Ok(mut l) = crate::runtime::domains::players_chat::CHAT_LAYOUT.lock() {
        l.window = ([win.min.x - rect.min.x, win.min.y - rect.min.y - INSET], [win.width(), win.height()]);
        l.input = ([bar.min.x - rect.min.x, bar.min.y - rect.min.y - INSET], [bar.width(), bar.height()]);
        l.tabs = ([0.0, 0.0], [0.0, 0.0]);
    }
    if w.enabled {
        if open {
            painter.rect_filled(win, 8.0, c32(w.bg, 1.0 - w.bg_transparency));
        }
        let newest = v.lines.iter().map(|l| l.age).fold(f64::INFINITY, f64::min);
        let fade = if open { 1.0 } else { (1.0 - (newest - TEXT_FADE_S).max(0.0)).clamp(0.0, 1.0) as f32 };
        if fade > 0.0 {
            let font = egui::FontId::proportional(w.text_size);
            let wrap = win.width() - 16.0;
            let galleys: Vec<_> = v
                .lines
                .iter()
                .map(|l| {
                    let mut job = LayoutJob::default();
                    job.wrap.max_width = wrap;
                    let text_c = c32(l.text_color.unwrap_or(w.text), fade);
                    if !l.prefix.is_empty() {
                        let name = l.prefix.trim_end_matches(':');
                        let name = name.rsplit(' ').next().unwrap_or(name);
                        let nc = match l.name_color {
                            Some(c) => c32(c, fade),
                            None if l.system => text_c,
                            None => chat_name_color(name).gamma_multiply(fade),
                        };
                        job.append(&l.prefix, 0.0, TextFormat { font_id: font.clone(), color: nc, ..Default::default() });
                        job.append(" ", 0.0, TextFormat { font_id: font.clone(), color: text_c, ..Default::default() });
                    }
                    job.append(&l.text, 0.0, TextFormat { font_id: font.clone(), color: text_c, ..Default::default() });
                    painter.layout_job(job)
                })
                .collect();
            // Newest at the bottom; the oldest scroll off the top.
            let room = win.height() - 16.0;
            let mut used = 0.0;
            let mut first = galleys.len();
            for g in galleys.iter().rev() {
                if used + g.size().y > room {
                    break;
                }
                used += g.size().y + 2.0;
                first -= 1;
            }
            let mut y = win.min.y + 8.0;
            let stroke = c32(w.stroke, (1.0 - w.stroke_transparency) * fade);
            for g in &galleys[first..] {
                let p = Pos2::new(win.min.x + 8.0, y);
                painter.galley(p + egui::vec2(1.0, 1.0), g.clone(), stroke);
                painter.galley(p, g.clone(), Color32::WHITE);
                y += g.size().y + 2.0;
            }
        }
    }
    let input = if open && w.input_enabled {
        painter.rect_filled(bar, 8.0, c32(w.input_bg, 1.0 - w.input_bg_transparency));
        Some(bar.shrink2(egui::vec2(12.0, 10.0)))
    } else {
        None
    };
    draw_chat_bubbles(painter, rect, v, cam);
    input
}

/// Bubble chat over its adornees: the newest MaxBubbles per adornee, stacked
/// upwards (Chat:Chat and TextChatService bubbles).
fn draw_chat_bubbles(
    painter: &egui::Painter,
    rect: Rect,
    v: &crate::runtime::domains::players_chat::ChatView,
    cam: &crate::camera::Camera,
) {
    let b = &v.bubble;
    if b.enabled {
        let mut per: std::collections::HashMap<Ref, usize> = Default::default();
        let mut list: Vec<_> = v.bubbles.iter().filter(|x| x.pos.is_some()).collect();
        list.sort_by(|x, y| y.id.cmp(&x.id));
        let mut stack: std::collections::HashMap<Ref, f32> = Default::default();
        for x in list {
            let n = per.entry(x.adornee).or_insert(0);
            *n += 1;
            if *n > b.max_bubbles {
                continue;
            }
            let Some(at) = x.pos else { continue };
            if (at - cam.pos).length() > b.max_distance {
                continue;
            }
            let Some(anchor) = cam.project(rect, at + Vec3::Y * b.offset) else {
                continue;
            };
            let font = egui::FontId::proportional(b.text_size);
            let fg = c32(x.fg.unwrap_or(b.fg), 1.0);
            let g = painter.layout(x.text.clone(), font, fg, 260.0);
            let size = g.size() + egui::vec2(24.0, 16.0);
            let lift = stack.entry(x.adornee).or_insert(0.0);
            let bottom = anchor.y - 10.0 - *lift;
            let r = Rect::from_min_size(Pos2::new(anchor.x - size.x / 2.0, bottom - size.y), size);
            let bg = c32(x.bg.unwrap_or(b.bg), 1.0 - x.bg_transparency.unwrap_or(b.bg_transparency));
            painter.rect_filled(r, 12.0, bg);
            if *lift == 0.0 {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(anchor.x - 7.0, r.max.y),
                        Pos2::new(anchor.x + 7.0, r.max.y),
                        Pos2::new(anchor.x, r.max.y + 7.0),
                    ],
                    bg,
                    Stroke::NONE,
                ));
            }
            painter.galley(r.min + egui::vec2(12.0, 8.0), g, fg);
            *lift += size.y + 6.0;
        }
    }
}

/// The legacy chat (LegacyChatService): recent lines bottom left.
fn draw_legacy_chat(ed: &Editor, painter: &egui::Painter, rect: Rect, chat_on: bool) {
    let lines: Vec<String> = ed
        .play
        .as_ref()
        .filter(|_| chat_on)
        .map(|s| s.game.chat_log.iter().rev().take(6).rev().map(|(l, _)| l.clone()).collect())
        .unwrap_or_default();
    let base = Pos2::new(rect.min.x + 12.0, rect.max.y - 44.0);
    for (i, l) in lines.iter().rev().enumerate() {
        let p = base - egui::vec2(0.0, 18.0 * (i as f32 + 1.0));
        painter.text(p + egui::vec2(1.0, 1.0), egui::Align2::LEFT_BOTTOM, l, egui::FontId::proportional(13.0), Color32::from_black_alpha(180));
        painter.text(p, egui::Align2::LEFT_BOTTOM, l, egui::FontId::proportional(13.0), Color32::WHITE);
    }
}

/// Studio's "Disconnected" dialog after Player:Kick.
fn draw_kicked(painter: &egui::Painter, rect: Rect, msg: &str) {
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(110));
    let body = if msg.is_empty() {
        "You have been kicked by this experience or its moderators.".to_string()
    } else {
        format!("You have been kicked by this experience or its moderators. Moderation message:\n\n{msg}")
    };
    let g = painter.layout(body, egui::FontId::proportional(15.0), Color32::from_gray(220), 310.0);
    let size = egui::vec2(350.0, g.size().y + 100.0);
    let r = Rect::from_center_size(rect.center(), size);
    painter.rect_filled(r, 8.0, Color32::from_rgb(35, 37, 39));
    painter.text(r.min + egui::vec2(20.0, 18.0), egui::Align2::LEFT_TOP, "Disconnected", egui::FontId::proportional(18.0), Color32::WHITE);
    painter.galley(r.min + egui::vec2(20.0, 50.0), g, Color32::WHITE);
}
