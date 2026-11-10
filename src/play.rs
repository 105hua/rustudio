//! Play (F5), Run (F8) and Stop (Shift+F5).
//!
//! Play runs two DataModels like a Roblox test session: the **server** (a
//! deep copy of the edit DataModel, running Scripts and physics) and one
//! **client** (a replicated copy without ServerStorage / ServerScriptService
//! contents, running LocalScripts). Server changes stream to the client
//! every frame; client changes stay local, except the character, which the
//! client owns (its movement is written to both sides). Remotes carry calls
//! across. Run (F8) is the server alone. Stop restores the edit DataModel.
//!
//! While a session runs, the editor shows one side (`showing`); the other
//! waits in `other_doc`. Each tick puts the server in front, works, and puts
//! the chosen side back.

use std::collections::{HashMap, HashSet};

use glam::{Mat3, Vec3};
use mlua::Value;
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{ustr, InstanceBuilder};

use crate::app::{Editor, Level};
use crate::camera::{Camera, CameraMode};
use crate::doc::{self, Document};
use crate::ltypes::LEnumItem;
use crate::math::Xform;
use crate::mech;
use crate::physics::{CharBody, Physics};
use crate::replicate::{self, RepMap};
use crate::rig::{self, Rig, RigType, State};
use crate::runtime::{RemoteKind, RepOp, RunMode, Runtime, Side, World};
use crate::scene::f32_of;

#[path = "play_game.rs"]
pub mod game;
#[path = "play_drag.rs"]
mod drag_input;

/// The player's character (refs are server-side).
pub struct Character {
    pub model: Ref,
    /// Accessories the rig's attachments were last worked out for.
    pub accessories_seen: usize,
    /// Humanoid.FloorMaterial as last written (Enum.Material value).
    pub floor_material: u32,
    pub humanoid: Ref,
    pub rig: Rig,
    pub rig_type: RigType,
    /// Body part touches are reported with this part as "other".
    pub touch_part: Ref,
    pub body: CharBody,
    /// HumanoidRootPart centre above the capsule centre.
    root_lift: f32,
    pub yaw: f32,
    /// Velocity last frame (studs/s): what AssemblyLinearVelocity reports.
    pub velocity: Vec3,
    pub dead: bool,
    touching: HashSet<Ref>,
    /// What it touched when the seats last looked (a Seat sits whoever
    /// starts touching it, not whoever stands on it).
    seat_seen: HashSet<Ref>,
    pub state: State,
    landed_for: f32,
    last_speed: f32,
    /// Where the controller last put the HumanoidRootPart (a script moving
    /// it elsewhere teleports the character).
    last_root: Option<Xform>,
    /// The Seat / VehicleSeat it sits in (server ref).
    pub seat: Option<Ref>,
    /// The client copy's Humanoid.Sit last frame (the client owns its
    /// character: a LocalScript setting Sit = false gets up).
    client_sit: bool,
    /// What Studio's Humanoid controller knows of the body (round 22).
    pub shape: crate::physics::HumShape,
    /// The last frame was a floating mode (swim, climb, mover, seat).
    hum_mode_left: bool,
    /// A wall the body was pushed into last frame (its outward normal):
    /// AutoRotate turns the body flush against it, as Studio's does.
    wall: Option<Vec3>,
    /// The root's forward tilt (radians, negative = looking down): a
    /// swimming Humanoid lies flat (Studio's root looks down while it swims).
    pitch: f32,
    /// Seconds before it may sit again after leaving a seat.
    seat_cooldown: f32,
    /// FallingDown / GettingUp / Ragdoll / Physics asked for by ChangeState.
    forced: Option<State>,
    /// Time in the forced state.
    forced_for: f32,
    /// Climbing / Swimming speed last reported (their events fire on change).
    last_move_speed: f32,
}

/// A change of seat this frame: Humanoid.Seated(active, seat).
struct SeatChange {
    active: bool,
    seat: Option<Ref>,
}

/// Which input decided the Play character's rig.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RigSource {
    /// The R15 / R6 picker in the test bar (an explicit choice wins).
    Picker,
    /// StarterPlayer.GameSettingsAvatar.
    Place,
    /// The signed-in person's avatar (playerAvatarType).
    Avatar,
    /// Nothing chose: rustudio's default.
    Default,
}

impl RigSource {
    pub fn label(self) -> &'static str {
        match self {
            RigSource::Picker => "picker",
            RigSource::Place => "place setting",
            RigSource::Avatar => "avatar",
            RigSource::Default => "default",
        }
    }
}

/// See `Editor::client_root_movers`.
#[derive(Default)]
struct RootMovers {
    plane: Option<Vec3>,
    /// A Vector-mode LinearVelocity strong enough to hold the character:
    /// it moves at this velocity, gravity included (Place C's ship).
    vector: Option<Vec3>,
    yaw: Option<f32>,
    /// AlignOrientation without rigidity eases toward `yaw` at this
    /// Responsiveness (per second); 0 = at once.
    yaw_rate: f32,
    vel_y: Option<f32>,
}

/// One side's runtime and (while not entered) its World.
pub struct SideState {
    pub runtime: Runtime,
    world: Option<World>,
}

pub struct PlaySession {
    pub mode: RunMode,
    saved_doc: Option<Document>,
    saved_cam: Camera,
    saved_sel: Vec<Ref>,
    server: SideState,
    client: Option<SideState>,
    /// The DataModel not shown in the editor right now.
    other_doc: Option<Document>,
    /// Which DataModel the editor shows (Explorer, Properties, viewport).
    pub showing: Side,
    pub map: RepMap,
    pub physics: Physics,
    pub character: Option<Character>,
    /// The Player instance (server ref).
    pub player: Ref,
    respawn_at: Option<f64>,
    pub time: f64,
    keys_prev: HashSet<String>,
    /// Keys whose InputBegan was game-processed (pressed into a TextBox):
    /// their InputEnded is too.
    keys_processed: HashSet<String>,
    /// The OS window had focus last frame (UserInputService.WindowFocused).
    window_focused: bool,
    /// Joystick devices seen (sorted names; slot i = Enum.UserInputType.Gamepad(i+1))
    /// and when they were last scanned.
    gamepads: Vec<String>,
    gamepad_scan_at: f64,
    /// When the instance collector last ran (`collect_garbage`), and parts the
    /// engine itself took out of the tree (fallen) for it to free.
    gc_at: f64,
    engine_orphans: Vec<Ref>,
    /// RUSTUDIO_MEMSTATS: when the last line was printed, and frames since.
    memstats_at: f64,
    memstats_frames: u32,
    memstats_worst: f32,
    /// Places the parts of assemblies only the client has.
    client_joints: crate::client_joints::ClientJoints,
    /// The client DataModel's own simulation (src/physics_client.rs).
    client_sim: crate::physics::client::ClientSim,
    /// Humanoid state transitions so far (shown in Output, read by the self-test).
    pub state_log: Vec<String>,
    /// Clicks, chat, tools, sounds and other in-game interaction.
    pub game: game::GameState,
    /// Mouse motion (pixels) and wheel notches over the game view since last frame.
    pub mouse_delta: glam::Vec2,
    pub mouse_moved: bool,
    pub wheel: f32,
    /// The client's UserInputService.MouseBehavior (0 Default, 1 LockCenter,
    /// 2 LockCurrentPosition) and MouseIconEnabled, read after its scripts ran.
    pub mouse_behavior: u32,
    pub mouse_icon: bool,
    /// The game's cursor image: UserInputService.MouseIcon, else the
    /// PlayerMouse's Icon (None = the system arrow).
    pub cursor_image: Option<String>,
    /// UserGameSettings.RotationType is CameraRelative (shift-lock, first person).
    pub camera_relative: bool,
}

impl PlaySession {
    /// The server side's collector counts while its runtime isn't entered:
    /// (unparented instances watched, instances freed so far, instances with
    /// a Lua slot). The self-test's `gc` step reads these.
    pub fn gc_counts(&self) -> (usize, u64, usize) {
        let (orphans, freed) = self
            .server
            .world
            .as_ref()
            .map(|w| (w.orphans.len(), w.freed))
            .unwrap_or_default();
        (orphans, freed, self.server.runtime.lua_slots())
    }

    /// The id of the DataModel not shown (the server's while the client is).
    pub fn other_doc_id(&self) -> Option<u64> {
        self.other_doc.as_ref().map(|d| d.id)
    }
}

/// KeyCode names for the keys the play session reports to scripts.
fn key_name(k: egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    Some(match k {
        A => "A",
        B => "B",
        C => "C",
        D => "D",
        E => "E",
        F => "F",
        G => "G",
        H => "H",
        I => "I",
        J => "J",
        K => "K",
        L => "L",
        M => "M",
        N => "N",
        O => "O",
        P => "P",
        Q => "Q",
        R => "R",
        S => "S",
        T => "T",
        U => "U",
        V => "V",
        W => "W",
        X => "X",
        Y => "Y",
        Z => "Z",
        Space => "Space",
        ArrowUp => "Up",
        ArrowDown => "Down",
        ArrowLeft => "Left",
        ArrowRight => "Right",
        Num0 => "Zero",
        Num1 => "One",
        Num2 => "Two",
        Num3 => "Three",
        Num4 => "Four",
        Num5 => "Five",
        Num6 => "Six",
        Num7 => "Seven",
        Num8 => "Eight",
        Num9 => "Nine",
        Enter => "Return",
        Tab => "Tab",
        Escape => "Escape",
        Backspace => "Backspace",
        Delete => "Delete",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        Comma => "Comma",
        Period => "Period",
        Minus => "Minus",
        Equals => "Equals",
        Semicolon => "Semicolon",
        Quote => "Quote",
        OpenBracket => "LeftBracket",
        CloseBracket => "RightBracket",
        Backslash => "BackSlash",
        Backtick => "Backquote",
        PageUp => "PageUp",
        PageDown => "PageDown",
        Home => "Home",
        End => "End",
        Insert => "Insert",
        _ => return None,
    })
}

fn new_side(mode: RunMode, side: Side, peer: bool) -> Result<SideState, String> {
    let runtime = Runtime::new()?;
    runtime
        .install_side()
        .map_err(|e| crate::runtime::describe_error(&e))?;
    let mut world = World::new(Document::empty(), mode);
    world.doc = None;
    world.side = side;
    world.has_peer = peer;
    Ok(SideState {
        runtime,
        world: Some(world),
    })
}

impl Editor {
    pub fn is_playing(&self) -> bool {
        self.play.is_some()
    }

    /// True while the editor shows the client of a Play session: mouse and
    /// keys go to the game's scripts.
    pub fn play_game_input(&self) -> bool {
        self.play.as_ref().is_some_and(|p| {
            p.mode == RunMode::Play && p.client.is_some() && p.showing == Side::Client
        })
    }

    /// True while a Play session drives a character.
    pub fn play_has_character(&self) -> bool {
        self.play.as_ref().is_some_and(|p| {
            p.mode == RunMode::Play && p.character.as_ref().is_some_and(|c| !c.dead)
        })
    }

    /// Puts the server DataModel in `self.doc` (the client one in `other_doc`).
    fn server_front(&mut self) {
        if let Some(s) = self.play.as_mut() {
            if s.showing == Side::Client {
                if let Some(other) = s.other_doc.as_mut() {
                    std::mem::swap(&mut self.doc, other);
                }
            }
        }
    }

    /// Undoes `server_front`.
    fn shown_front(&mut self) {
        self.server_front();
    }

    /// Switches the editor between the server's and the client's DataModel.
    pub fn show_play_side(&mut self, side: Side) {
        let Some(s) = self.play.as_mut() else { return };
        if s.showing == side || s.other_doc.is_none() {
            return;
        }
        if let Some(other) = s.other_doc.as_mut() {
            std::mem::swap(&mut self.doc, other);
        }
        s.showing = side;
        self.doc.selection.clear();
        self.scene = crate::scene::Scene::default();
        self.explorer.expand_defaults(&self.doc);
        self.drag = None;
    }

    /// Runs `f` with one side's runtime entered (its DataModel lent to Lua).
    /// `peer` is the other side's DataModel. Must be called with the server
    /// in front. Output is tagged with the side.
    fn with_side<R>(
        &mut self,
        side: Side,
        f: impl FnOnce(&Runtime, Option<&Document>, &mut RepMap) -> R,
    ) -> Option<R> {
        let s = self.play.as_mut()?;
        let (result, output) = match side {
            Side::Server => {
                let mut w = s.server.world.take()?;
                w.doc = Some(std::mem::replace(&mut self.doc, Document::empty()));
                s.server.runtime.enter(w);
                let r = f(&s.server.runtime, s.other_doc.as_ref(), &mut s.map);
                let mut w = s.server.runtime.leave();
                self.doc = w.doc.take().expect("server document returned");
                let out = std::mem::take(&mut w.output);
                s.server.world = Some(w);
                (r, out)
            }
            Side::Client => {
                let c = s.client.as_mut()?;
                let mut w = c.world.take()?;
                w.doc = Some(s.other_doc.take()?);
                c.runtime.enter(w);
                let r = f(&c.runtime, Some(&self.doc), &mut s.map);
                let mut w = c.runtime.leave();
                s.other_doc = w.doc.take();
                let out = std::mem::take(&mut w.output);
                c.world = Some(w);
                (r, out)
            }
        };
        let tag = side.label();
        // Run has only a server, and Studio still marks its lines Server
        // (the green bar).
        let in_session = self.play.is_some();
        for (level, text) in output {
            if in_session {
                self.push_log(level, format!("[{tag}] {text}"));
            } else {
                self.push_log(level, text);
            }
        }
        Some(result)
    }

    fn server_world(&mut self) -> Option<&mut World> {
        self.play.as_mut()?.server.world.as_mut()
    }

    fn client_world(&mut self) -> Option<&mut World> {
        self.play.as_mut()?.client.as_mut()?.world.as_mut()
    }

    fn record(&mut self, op: RepOp) {
        if let Some(w) = self.server_world() {
            w.record(op);
        }
    }

    pub fn start_play(&mut self, mode: RunMode) {
        if self.play.is_some() {
            return;
        }
        // Play on a place still loading waits for it (and so for its
        // PlaceId / GameId), as Studio's Play only runs a loaded DataModel.
        if self.defer_play_until_loaded(mode) {
            return;
        }
        // The local player is the signed-in account (identity and avatar),
        // as in Studio: a session check still running at Play is waited for.
        if mode == RunMode::Play {
            for (level, text) in self.account.settle_for_play(std::time::Duration::from_secs(20)) {
                self.push_log(level, text);
            }
        }
        let two = mode == RunMode::Play;
        // Cloud data stays in memory for any published place but the
        // scratch experience (domains::data::local_mode_for).
        let universe = self
            .live_team_create_ids()
            .map(|(u, _)| u)
            .or_else(|| self.doc.cloud.as_ref().map(|o| o.universe_id))
            .unwrap_or(0);
        crate::runtime::domains::data::PLAY_UNIVERSE
            .store(universe, std::sync::atomic::Ordering::Relaxed);
        if crate::runtime::domains::data::local_mode_for(universe) && universe != 0 {
            self.push_log(
                crate::app::Level::Warn,
                format!(
                    "DataStores, MemoryStores and Messaging stay in memory for this Play \
                     (universe {universe} is a published game; RUSTUDIO_LIVE_DATA=1 uses its real data)"
                ),
            );
        }
        let server = match new_side(mode, Side::Server, two) {
            Ok(s) => s,
            Err(e) => {
                self.error(format!("Could not start the Luau runtime: {e}"));
                return;
            }
        };
        let client = if two {
            match new_side(mode, Side::Client, true) {
                Ok(c) => Some(c),
                Err(e) => {
                    self.error(format!("Could not start the client runtime: {e}"));
                    return;
                }
            }
        } else {
            None
        };
        let label = if two { "Play" } else { "Run" };
        let mut server_doc = self
            .doc
            .clone_for_play(format!("{} ({label}: Server)", self.doc.title));
        // Roblox's default PlayerModule, PlayerScriptsLoader and
        // RbxCharacterSounds join StarterPlayerScripts, as Studio does.
        // Every Roblox DataModel has these services, even when the file
        // doesn't list them (a place saved with only a Workspace).
        // Workspace (with Terrain and a CurrentCamera) and the other services
        // exist before any script runs, as in every Roblox DataModel.
        server_doc.ensure_services();
        // Players starts empty, as in Studio's Play: a live Team Create
        // DataModel holds the collaborators' Player objects (a live Place A session:
        // Players.testuser1), and copied along they were a phantom first
        // player that Place A's match spawned and placed instead of the real one.
        if let Some(players) = server_doc.find_child(server_doc.dom.root_ref(), "Players") {
            let stale: Vec<_> = server_doc
                .dom
                .get_by_ref(players)
                .map(|i| i.children().to_vec())
                .unwrap_or_default()
                .into_iter()
                .filter(|c| server_doc.dom.get_by_ref(*c).is_some_and(|i| i.class == "Player"))
                .collect();
            for c in stale {
                server_doc.dom.destroy(c);
            }
        }
        // A place opened from Roblox plays with its real ids, as in Studio
        // (an unpublished one keeps 0: runtime_ext's DataModel defaults).
        if let Some(o) = server_doc.cloud.clone() {
            let root = server_doc.dom.root_ref();
            if let Some(i) = server_doc.dom.get_by_ref_mut(root) {
                use rbx_dom_weak::{types::Enum, ustr};
                i.properties.insert(ustr("PlaceId"), Variant::Int64(o.place_id as i64));
                i.properties.insert(ustr("GameId"), Variant::Int64(o.universe_id as i64));
                i.properties.insert(ustr("CreatorId"), Variant::Int64(o.creator_id as i64));
                i.properties.insert(
                    ustr("CreatorType"),
                    Variant::Enum(Enum::from_u32(o.creator_type.enum_value())),
                );
                i.properties.insert(ustr("PlaceVersion"), Variant::Int32(o.version as i32));
            }
        }
        // Direct live joins have no REST CloudOrigin. Their session still
        // supplies the published identity required by game initialization.
        if let Some((universe, place)) = self.live_team_create_ids() {
            let root = server_doc.dom.root_ref();
            if let Some(i) = server_doc.dom.get_by_ref_mut(root) {
                i.properties.insert(ustr("PlaceId"), Variant::Int64(place as i64));
                i.properties.insert(ustr("GameId"), Variant::Int64(universe as i64));
            }
        }
        let defaults = if two {
            crate::default_scripts::insert_player_scripts(&mut server_doc)
        } else {
            Vec::new()
        };
        let (client_doc, map) = if two {
            let (c, m) = replicate::initial_client(
                &server_doc,
                format!("{} ({label}: Client)", self.doc.title),
            );
            (Some(c), m)
        } else {
            (None, RepMap::default())
        };
        let saved_sel = std::mem::take(&mut self.doc.selection);
        let saved_doc = std::mem::replace(&mut self.doc, server_doc);
        let mut physics = Physics::new(&mut self.doc);
        // One collision-group table for the session: the server's scripts
        // set it, the client's queries and the physics read it.
        let mut server = server;
        let mut client = client;
        let groups = crate::collision_groups::shared();
        for w in [server.world.as_mut(), client.as_mut().and_then(|c| c.world.as_mut())]
            .into_iter()
            .flatten()
        {
            w.groups = groups.clone();
            w.avatar = self.account.avatar().cloned();
        }
        physics.groups = groups;
        self.play = Some(PlaySession {
            mode,
            saved_doc: Some(saved_doc),
            saved_cam: self.cam.clone(),
            saved_sel,
            server,
            client,
            other_doc: client_doc,
            showing: Side::Server,
            map,
            physics,
            character: None,
            player: Ref::none(),
            respawn_at: None,
            time: 0.0,
            keys_prev: HashSet::new(),
            keys_processed: HashSet::new(),
            window_focused: true,
            gamepads: Vec::new(),
            gamepad_scan_at: f64::NEG_INFINITY,
            gc_at: 0.0,
            engine_orphans: Vec::new(),
            memstats_at: 0.0,
            memstats_frames: 0,
            memstats_worst: 0.0,
            client_joints: Default::default(),
            client_sim: Default::default(),
            state_log: Vec::new(),
            game: Default::default(),
            mouse_delta: glam::Vec2::ZERO,
            mouse_moved: false,
            wheel: 0.0,
            mouse_behavior: 0,
            mouse_icon: true,
            cursor_image: None,
            camera_relative: false,
        });
        self.add_chat_stub();
        if !defaults.is_empty() {
            self.info(format!(
                "Default player scripts from Studio: {}",
                defaults.join(", ")
            ));
        }
        self.drag = None;
        self.scene = crate::scene::Scene::default();
        self.explorer.expand_defaults(&self.doc);
        self.info(match mode {
            RunMode::Play => "▶ Play: a server and a client are running; WASD + Space to move, Shift+F5 to stop",
            RunMode::Run => "▶ Run: server scripts and physics running (no client, no character), Shift+F5 to stop",
        });

        let bind = |rt: &Runtime, _: Option<&Document>, _: &mut RepMap| {
            if let Err(e) = rt.bind_globals() {
                if let Some(mut w) = rt.world_mut() {
                    w.output
                        .push((Level::Error, crate::runtime::describe_error(&e)));
                }
            }
        };
        self.with_side(Side::Server, bind);
        if two {
            self.with_side(Side::Client, bind);
        }
        self.with_side(Side::Server, |rt, _, _| rt.warm_queries());
        // Server scripts start first, as on a real server; the player joins
        // after (so Players:GetPlayers() is empty when they start).
        let server_scripts = self.server_scripts(None);
        self.with_side(Side::Server, |rt, _, _| {
            for r in server_scripts {
                rt.start_collected(r);
            }
            rt.dispatch();
        });
        if two {
            self.create_player();
            let player = self
                .play
                .as_ref()
                .map(|p| p.player)
                .unwrap_or_else(Ref::none);
            // Tool scripts run from the player's Backpack, and legacy
            // Scripts from its PlayerGui too (Place B Newer's StarterGui
            // "version" label: Studio shows "server version: 0").
            let backpack: Vec<Ref> = ["Backpack", "PlayerGui"]
                .iter()
                .filter_map(|c| self.doc.find_child(player, c))
                .flat_map(|f| self.server_scripts(Some(f)))
                .collect();
            let players = self.doc.find_child(self.doc.dom.root_ref(), "Players");
            self.with_side(Side::Server, |rt, _, _| {
                if let Some(p) = players {
                    rt.fire(p, "PlayerAdded", vec![rt.inst(player)]);
                }
                for r in backpack {
                    rt.start_collected(r);
                }
                rt.dispatch();
            });
            self.replicate_now();
            self.with_side(Side::Client, |rt, _, _| rt.warm_queries());
            self.setup_client_player();
            // Players.CharacterAutoLoads false: no character until a script
            // calls Player:LoadCharacter (the client's camera is the scripts').
            if self.character_auto_loads() {
                self.spawn_character();
            }
            // Studio shows the client's view while playing.
            if self.play_shows_client {
                self.show_play_side(Side::Client);
            }
        }
        self.start_autoplay_sounds();
    }

    /// Player.CharacterRemoving (on both sides) just before the old character
    /// goes, as Roblox does on respawn and LoadCharacter.
    fn fire_character_removing(&mut self, model: Ref) {
        let Some((player, cplayer, cmodel)) = self.play.as_ref().map(|s| {
            (
                s.player,
                s.map.s2c.get(&s.player).copied(),
                s.map.s2c.get(&model).copied(),
            )
        }) else {
            return;
        };
        if player.is_some() {
            self.with_side(Side::Server, |rt, _, _| {
                rt.fire(player, "CharacterRemoving", vec![rt.inst(model)]);
                rt.dispatch();
            });
        }
        if let (Some(cp), Some(cm)) = (cplayer, cmodel) {
            self.with_side(Side::Client, |rt, _, _| {
                rt.fire(cp, "CharacterRemoving", vec![rt.inst(cm)]);
                rt.dispatch();
            });
        }
    }

    /// Players.PlayerRemoving when Play stops (the player leaves the server),
    /// with one dispatch so handlers run before the session goes.
    fn fire_player_removing(&mut self) {
        let Some((players, player, cplayers, cplayer)) = self.play.as_ref().map(|s| {
            let root = self.doc.dom.root_ref();
            let players = self.doc.find_child(root, "Players");
            (
                players,
                s.player,
                players.and_then(|p| s.map.s2c.get(&p).copied()),
                s.map.s2c.get(&s.player).copied(),
            )
        }) else {
            return;
        };
        if let Some(ps) = players.filter(|_| player.is_some()) {
            self.with_side(Side::Server, |rt, _, _| {
                rt.fire(ps, "PlayerRemoving", vec![rt.inst(player)]);
                rt.dispatch();
            });
        }
        if let (Some(ps), Some(p)) = (cplayers, cplayer) {
            self.with_side(Side::Client, |rt, _, _| {
                rt.fire(ps, "PlayerRemoving", vec![rt.inst(p)]);
                rt.dispatch();
            });
        }
    }

    pub fn stop_play(&mut self) {
        // A game's orthographic camera ends with the session.
        self.cam.ortho = None;
        self.cam.stretch = 1.0;
        if self.play.is_some() {
            self.server_front();
            self.fire_player_removing();
        }
        crate::telemetry::flush();
        self.stop_all_sounds();
        self.cam.basis = None;
        let Some(mut s) = self.play.take() else {
            return;
        };
        if let Some(d) = s.saved_doc.take() {
            self.doc = d;
        }
        self.doc.selection = s.saved_sel.clone();
        self.cam = s.saved_cam.clone();
        self.scene = crate::scene::Scene::default();
        self.drag = None;
        self.explorer.expand_defaults(&self.doc);
        self.info(format!(
            "■ Stopped after {:.1} s; the edit DataModel is back as it was",
            s.time
        ));
    }

    /// Server Scripts to start: under Workspace and ServerScriptService
    /// (Legacy / Server RunContext), or within `under` only.
    fn server_scripts(&self, under: Option<Ref>) -> Vec<Ref> {
        let d = &self.doc;
        let root = d.dom.root_ref();
        let roots: Vec<Ref> = match under {
            Some(u) => vec![u],
            None => ["Workspace", "ServerScriptService"]
                .iter()
                .filter_map(|s| d.find_child(root, s))
                .collect(),
        };
        let mut out = Vec::new();
        for s in roots {
            for i in d.dom.descendants_of(s) {
                let client = crate::scene::enum_u32(d.prop(i.referent(), "RunContext")) == Some(2);
                if i.class == "Script" && !client && script_enabled(d, i.referent()) {
                    out.push(i.referent());
                }
            }
        }
        out
    }

    /// The local player's name: the signed-in account's, as in Studio; offline
    /// Studio's "Player" when signed out.
    pub(crate) fn player_name(&self) -> String {
        self.account.names().map_or_else(|| mech::PLAYER_NAME.to_string(), |n| n.0)
    }

    fn player_display_name(&self) -> String {
        self.account.names().map_or_else(|| mech::PLAYER_NAME.to_string(), |n| n.1)
    }

    /// Creates Players.Player1 on the server with Backpack and PlayerGui
    /// filled from StarterPack / StarterGui (both replicate to the client).
    fn create_player(&mut self) {
        let root = self.doc.dom.root_ref();
        let players = match self.doc.find_child(root, "Players") {
            Some(p) => p,
            None => self
                .doc
                .dom
                .insert(root, InstanceBuilder::new("Players").with_name("Players")),
        };
        let player = self.doc.dom.insert(
            players,
            InstanceBuilder::new("Player")
                .with_name(self.player_name())
                .with_property("DisplayName", self.player_display_name())
                .with_property("UserId", self.account.user_id().unwrap_or(1) as i64),
        );
        // Signed in: the account's AccountAge, verified badge and membership,
        // as a signed-in Studio's Play gives its local player (players_chat).
        if let Some(uid) = self.account.user_id() {
            let id = crate::runtime::domains::players_chat::account_identity(uid as i64, std::time::Duration::from_secs(5));
            if let (Some(id), Some(inst)) = (id, self.doc.dom.get_by_ref_mut(player)) {
                inst.properties.insert(ustr("AccountAge"), Variant::Int32(id.account_age));
                inst.properties.insert(ustr("HasVerifiedBadge"), Variant::Bool(id.verified));
                // Enum.MembershipType: None 0, Premium 4.
                inst.properties.insert(
                    ustr("MembershipType"),
                    Variant::Enum(rbx_dom_weak::types::Enum::from_u32(if id.premium { 4 } else { 0 })),
                );
                inst.properties.insert(ustr("CharacterAppearanceId"), Variant::Int64(uid as i64));
            }
        }
        // The Player takes StarterPlayer's camera and control settings (Roblox docs, StarterPlayer).
        if let Some(sp) = self.doc.find_child(root, "StarterPlayer") {
            for (from, to) in mech::STARTER_PLAYER_TO_PLAYER {
                if let Some(v) = self.doc.prop(sp, from) {
                    if let Some(inst) = self.doc.dom.get_by_ref_mut(player) {
                        inst.properties.insert(ustr(to), v);
                    }
                }
            }
        }
        let backpack = self.doc.dom.insert(
            player,
            InstanceBuilder::new("Backpack").with_name("Backpack"),
        );
        let gui = self.doc.dom.insert(
            player,
            InstanceBuilder::new("PlayerGui").with_name("PlayerGui"),
        );
        for (from, to) in [
            (self.doc.find_child(root, "StarterPack"), backpack),
            (self.doc.find_child(root, "StarterGui"), gui),
        ] {
            let Some(from) = from else { continue };
            for k in self
                .doc
                .dom
                .get_by_ref(from)
                .map(|i| i.children().to_vec())
                .unwrap_or_default()
            {
                let c = self.doc.dom.clone_within(k);
                self.doc.dom.transfer_within(c, to);
            }
        }
        // Teams: join the auto-assignable team with the fewest players.
        let team = self.doc.find_child(root, "Teams").and_then(|t| {
            let d = &self.doc;
            d.dom.get_by_ref(t).and_then(|i| {
                i.children().iter().copied().find(|c| {
                    d.class_of(*c).is_some_and(|k| k == "Team")
                        && !matches!(d.prop(*c, "AutoAssignable"), Some(Variant::Bool(false)))
                })
            })
        });
        if let Some(team) = team {
            let color = self.doc.prop(team, "TeamColor");
            if let Some(inst) = self.doc.dom.get_by_ref_mut(player) {
                inst.properties.insert(ustr("Team"), Variant::Ref(team));
                inst.properties
                    .insert(ustr("Neutral"), Variant::Bool(false));
                if let Some(c) = color {
                    inst.properties.insert(ustr("TeamColor"), c);
                }
            }
        }
        self.doc.mark_tree(player);
        if let Some(p) = self.play.as_mut() {
            p.player = player;
        }
        self.record(RepOp::Parent(player));
    }

    /// Client side of joining: LocalPlayer, PlayerScripts (client-only,
    /// filled from StarterPlayerScripts), then LocalScripts start.
    fn setup_client_player(&mut self) {
        let Some(s) = self.play.as_ref() else { return };
        let Some(cplayer) = s.map.s2c.get(&s.player).copied() else {
            return;
        };
        let mut local = Vec::new();
        if let Some(cdoc) = self.play.as_mut().and_then(|s| s.other_doc.as_mut()) {
            let root = cdoc.dom.root_ref();
            if let Some(players) = cdoc.find_child(root, "Players") {
                if let Some(i) = cdoc.dom.get_by_ref_mut(players) {
                    i.properties
                        .insert(ustr("LocalPlayer"), Variant::Ref(cplayer));
                }
                cdoc.mark_prop(players, ustr("LocalPlayer"));
            }
            // Every client has a Workspace.CurrentCamera (Roblox makes one if the place has none).
            if let Some(ws) = cdoc.workspace() {
                let has = matches!(cdoc.prop(ws, "CurrentCamera"), Some(Variant::Ref(c)) if cdoc.exists(c));
                if !has {
                    let cam = cdoc.find_child(ws, "Camera").unwrap_or_else(|| {
                        cdoc.dom.insert(
                            ws,
                            InstanceBuilder::new("Camera")
                                .with_name("Camera")
                                .with_property("FieldOfView", mech::FOV_DEG),
                        )
                    });
                    if let Some(i) = cdoc.dom.get_by_ref_mut(ws) {
                        i.properties
                            .insert(ustr("CurrentCamera"), Variant::Ref(cam));
                    }
                    cdoc.mark_tree(cam);
                }
                // A joining client's camera is Custom (the PlayerModule's
                // cameras drive it), whatever the edit camera was saved as.
                if let Some(Variant::Ref(cam)) = cdoc.prop(ws, "CurrentCamera") {
                    if let Some(i) = cdoc.dom.get_by_ref_mut(cam) {
                        i.properties.insert(
                            ustr("CameraType"),
                            Variant::Enum(rbx_dom_weak::types::Enum::from_u32(
                                mech::CAMERA_TYPE_CUSTOM,
                            )),
                        );
                    }
                    cdoc.mark_prop(cam, ustr("CameraType"));
                }
            }
            let scripts = cdoc.dom.insert(
                cplayer,
                InstanceBuilder::new("PlayerScripts").with_name("PlayerScripts"),
            );
            if let Some(sps) = cdoc
                .find_child(root, "StarterPlayer")
                .and_then(|sp| cdoc.find_child(sp, "StarterPlayerScripts"))
            {
                for k in cdoc
                    .dom
                    .get_by_ref(sps)
                    .map(|i| i.children().to_vec())
                    .unwrap_or_default()
                {
                    let c = cdoc.dom.clone_within(k);
                    cdoc.dom.transfer_within(c, scripts);
                }
            }
            cdoc.mark_tree(scripts);
            local = client_scripts(
                cdoc,
                &[Some(cplayer), cdoc.find_child(root, "ReplicatedFirst")],
            );
            // RunContext = Client Scripts run anywhere replicated.
            for i in cdoc.dom.descendants() {
                if i.class == "Script"
                    && crate::scene::enum_u32(cdoc.prop(i.referent(), "RunContext")) == Some(2)
                    && script_enabled(cdoc, i.referent())
                    && !local.contains(&i.referent())
                {
                    local.push(i.referent());
                }
            }
        }
        self.sync_view_size();
        self.with_side(Side::Client, |rt, _, _| {
            for r in local {
                rt.start_collected(r);
            }
            rt.dispatch();
        });
    }

    /// Frees what each side's scripts can no longer reach (Runtime::collect),
    /// twice a second, and drops the freed instances from the replication
    /// pairs. Roblox's Lua GC does the same; without it every tracer, hole
    /// and effect part a game ever made stayed in memory for the session.
    fn collect_garbage(&mut self, dt: f32) {
        let Some(s) = self.play.as_mut() else { return };
        s.memstats_frames += 1;
        s.memstats_worst = s.memstats_worst.max(dt);
        let pass = s.time - s.gc_at >= crate::mech::GC_INTERVAL;
        {
            let _sc = crate::bench::scope("p9_gc_step");
            for side in [Side::Server, Side::Client] {
                self.with_side(side, |rt, _, _| rt.gc_step(dt as f64));
            }
        }
        let Some(s) = self.play.as_mut() else { return };
        if !pass {
            return;
        }
        s.gc_at = s.time;
        let fallen = std::mem::take(&mut s.engine_orphans);
        let player = s.player;
        let model = s.character.as_ref().map(|c| c.model).unwrap_or_else(Ref::none);
        let due = crate::runtime::memstats_on() && s.time - s.memstats_at >= 5.0;
        let _sc = crate::bench::scope("p9_collect");
        let (server_freed, srv) = self
            .with_side(Side::Server, |rt, _, map| {
                for r in &fallen {
                    rt.note_orphan(*r);
                }
                let freed = rt.collect(
                    &|r| r == player || r == model || map.s2c.contains_key(&r),
                    crate::mech::GC_BUDGET,
                );
                (freed.len(), if due { rt.mem_stats() } else { String::new() })
            })
            .unwrap_or_default();
        let (client_freed, cli) = self
            .with_side(Side::Client, |rt, _, map| {
                let freed = rt.collect(&|r| map.c2s.contains_key(&r), crate::mech::GC_BUDGET);
                (freed.len(), if due { rt.mem_stats() } else { String::new() })
            })
            .unwrap_or_default();
        let Some(s) = self.play.as_mut() else { return };
        if !due {
            return;
        }
        let span = (s.time - s.memstats_at).max(1e-3);
        let fps = s.memstats_frames as f64 / span;
        let worst = s.memstats_worst * 1000.0;
        s.memstats_at = s.time;
        s.memstats_frames = 0;
        s.memstats_worst = 0.0;
        let rss = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|t| t.lines().find(|l| l.starts_with("VmRSS:")).map(|l| l.split_whitespace().nth(1).unwrap_or("0").to_string()))
            .unwrap_or_default();
        let time = s.time;
        let (rs2c, rc2s) = (s.map.s2c.len(), s.map.c2s.len());
        eprintln!(
            "MEMSTATS t={time:.0} rss_kb={rss} fps={fps:.1} worst_ms={worst:.0} gc_freed={server_freed}/{client_freed} rep={rs2c}/{rc2s} | server {srv} | client {cli}"
        );
    }

    /// Streams queued server changes to the client, within the frame's
    /// replication budget (`replicate::apply_budgeted`).
    fn replicate(&mut self) {
        self.replicate_with(true);
    }

    /// Applies every waiting server change now: engine steps that read the
    /// client copy right after (player join, character spawn).
    fn replicate_now(&mut self) {
        self.replicate_with(false);
    }

    fn replicate_with(&mut self, budgeted: bool) {
        let ops = match self.server_world() {
            Some(w) => std::mem::take(&mut w.rep),
            None => return,
        };
        let idle = self.play.as_ref().is_none_or(|p| p.client.is_none() || p.map.backlog() == 0);
        if ops.is_empty() && idle || self.play.as_ref().is_none_or(|p| p.client.is_none()) {
            return;
        }
        self.with_side(Side::Client, |rt, server, map| {
            if let Some(server) = server {
                if budgeted {
                    replicate::apply_budgeted(server, rt, map, ops);
                } else {
                    replicate::apply(server, rt, map, ops);
                }
            }
            let _sc = crate::bench::scope("rep_dispatch");
            rt.dispatch();
        });
    }

    /// Carries remote calls across: client → server first (so replies can
    /// come back in the same frame), then server → client.
    fn deliver_remotes(&mut self) {
        let Some(player) = self.play.as_ref().map(|p| p.player) else {
            return;
        };
        for _ in 0..2 {
            let to_server = self
                .client_world()
                .map(|w| std::mem::take(&mut w.outbox))
                .unwrap_or_default();
            crate::runtime::domains::services::perf::remotes(false, &to_server);
            if std::env::var_os("RUSTUDIO_GUI_DEBUG").is_some() {
                for m in &to_server {
                    let name = self
                        .play_other_doc()
                        .and_then(|d| d.dom.get_by_ref(m.inst).map(|i| i.name.clone()))
                        .unwrap_or_default();
                    let mapped = self
                        .play
                        .as_ref()
                        .is_some_and(|s| s.map.c2s.contains_key(&m.inst));
                    self.info(format!(
                        "[remote] client → server {name} ({:?}) mapped={mapped} args={:?}",
                        m.kind, m.args
                    ));
                }
            }
            if !to_server.is_empty() {
                self.with_side(Side::Server, |rt, _, map| {
                    let m: &RepMap = map;
                    let c2s = |c: Ref| m.c2s.get(&c).copied();
                    for msg in to_server {
                        if let RemoteKind::Reply(id, ok) = msg.kind {
                            rt.finish_invoke(id, ok, &msg.args, &c2s);
                            continue;
                        }
                        let Some(remote) = m.c2s.get(&msg.inst).copied() else {
                            continue;
                        };
                        match msg.kind {
                            RemoteKind::Event if crate::runtime::domains::web_social::is_engine_msg(&msg.args) => {
                                crate::runtime::domains::web_social::engine_message(rt,remote,player,&msg.args,&c2s)
                            }
                            RemoteKind::Event if crate::runtime::domains::gui_input::is_engine_msg(&msg.args) => {
                                crate::runtime::domains::gui_input::engine_message(rt, remote, player, &msg.args, &c2s)
                            }
                            RemoteKind::Event => rt.deliver_event(
                                remote,
                                "OnServerEvent",
                                Some(player),
                                &msg.args,
                                &c2s,
                            ),
                            RemoteKind::Invoke(id) => {
                                rt.serve_invoke(remote, player, id, &msg.args, &c2s)
                            }
                            RemoteKind::Reply(..) => {}
                        }
                    }
                    rt.dispatch();
                });
            }
            self.replicate();
            let to_client = self
                .server_world()
                .map(|w| std::mem::take(&mut w.outbox))
                .unwrap_or_default();
            // Behind the replication sent before them (Roblox keeps one order).
            let to_client = match self.play.as_mut() {
                Some(s) => s.map.gate_remotes(to_client),
                None => to_client,
            };
            crate::runtime::domains::services::perf::remotes(true, &to_client);
            if to_client.is_empty() {
                if self.client_world().is_none_or(|w| w.outbox.is_empty()) {
                    break;
                }
                continue;
            }
            self.with_side(Side::Client, |rt, _, map| {
                let m: &RepMap = map;
                let s2c = |s: Ref| m.s2c.get(&s).copied();
                for msg in to_client {
                    match msg.kind {
                        RemoteKind::Event if crate::runtime::domains::web_social::is_engine_msg(&msg.args) => {
                            if let Some(remote)=m.s2c.get(&msg.inst).copied() {
                                crate::runtime::domains::web_social::engine_message(rt,remote,Ref::none(),&msg.args,&s2c);
                            }
                        }
                        RemoteKind::Event => {
                            if let Some(remote) = m.s2c.get(&msg.inst).copied() {
                                rt.deliver_event(remote, "OnClientEvent", None, &msg.args, &s2c);
                            }
                        }
                        RemoteKind::Reply(id, ok) => rt.finish_invoke(id, ok, &msg.args, &s2c),
                        RemoteKind::Invoke(id) => {
                            if let Some(remote) = m.s2c.get(&msg.inst).copied() {
                                rt.serve_client_invoke(remote, id, &msg.args, &s2c);
                            }
                        }
                    }
                }
                rt.dispatch();
            });
        }
    }

    /// Where characters appear: the capsule centre above the first enabled SpawnLocation.
    fn character_spawn_point(&self) -> Vec3 {
        // Test Here: where the camera was when the test started.
        if let Some(p) = self.play_here {
            return p;
        }
        let d = &self.doc;
        // A team player spawns on a Neutral spawn or one in the team's colour.
        let player = self.play.as_ref().map(|p| p.player).filter(|p| p.is_some());
        let team_color = player
            .filter(|p| !matches!(d.prop(*p, "Neutral"), Some(Variant::Bool(true))))
            .and_then(|p| d.prop(p, "TeamColor"));
        let fits = |s: Ref| match &team_color {
            None => true,
            Some(c) => {
                !matches!(d.prop(s, "Neutral"), Some(Variant::Bool(false)))
                    || d.prop(s, "TeamColor").as_ref() == Some(c)
            }
        };
        if let Some(ws) = d.workspace() {
            for i in d.dom.descendants_of(ws) {
                if i.class == "SpawnLocation"
                    && !matches!(d.prop(i.referent(), "Enabled"), Some(Variant::Bool(false)))
                    && fits(i.referent())
                {
                    if let Some(xf) = crate::scene::xform_of(d, i.referent()) {
                        let size = crate::scene::vec3_of(d.prop(i.referent(), "Size"))
                            .unwrap_or(Vec3::ONE);
                        let top = xf.pos + Vec3::Y * (size.y * 0.5);
                        return top
                            + Vec3::Y
                                * (mech::CHARACTER_HALF_HEIGHT + mech::CHARACTER_RADIUS + 0.5);
                    }
                }
            }
        }
        Vec3::new(0.0, mech::NO_SPAWN_HEIGHT, 0.0)
    }

    /// The Humanoid's settings come from StarterPlayer (Character* properties).
    fn humanoid_builder(&self) -> InstanceBuilder {
        let d = &self.doc;
        let sp = d.find_child(d.dom.root_ref(), "StarterPlayer");
        let f =
            |name: &str, default: f32| sp.and_then(|s| f32_of(d.prop(s, name))).unwrap_or(default);
        let use_power = sp
            .map(|s| {
                matches!(
                    d.prop(s, "CharacterUseJumpPower"),
                    Some(Variant::Bool(true))
                )
            })
            .unwrap_or(false);
        InstanceBuilder::new("Humanoid")
            .with_name("Humanoid")
            .with_property("Health", 100.0f32)
            .with_property("MaxHealth", 100.0f32)
            .with_property("WalkSpeed", f("CharacterWalkSpeed", mech::WALK_SPEED))
            .with_property("JumpHeight", f("CharacterJumpHeight", mech::JUMP_HEIGHT))
            .with_property("JumpPower", f("CharacterJumpPower", mech::JUMP_POWER))
            .with_property("UseJumpPower", use_power)
            .with_property(
                "MaxSlopeAngle",
                f("CharacterMaxSlopeAngle", mech::MAX_SLOPE_DEG),
            )
            // Spawned characters carry an Animator (Animate finds it with
            // FindFirstChildOfClass).
            .with_child(InstanceBuilder::new("Animator").with_name("Animator"))
    }

    fn spawn_character(&mut self) {
        let Some(player) = self.play.as_ref().map(|p| p.player).filter(|p| p.is_some()) else {
            return;
        };
        let Some(ws) = self.doc.workspace() else {
            return;
        };
        let (rig_type, colors, scales) = self.character_look();
        let (_, source) = self.rig_choice();
        let rig_name = |r: RigType| if r == RigType::R15 { "R15" } else { "R6" };
        let mut why = format!(
            "Character: {} (from the {})",
            rig_name(rig_type),
            source.label()
        );
        if let (RigSource::Picker, Some(p)) = (source, self.place_rig()) {
            if p != rig_type {
                why.push_str(&format!("; the place asks for {}", rig_name(p)));
            }
        }
        self.info(why);
        let humanoid = self.humanoid_builder();
        let name = self.player_name();
        let m = rig::insert_character(
            &mut self.doc.dom,
            ws,
            &name,
            rig_type,
            &colors,
            &scales,
            humanoid,
        );
        self.dress_character(m);
        // Studio's avatar joint upgrade (StarterPlayer.AvatarJointUpgrade):
        // an R15 body's rig Motor6Ds become AnimationConstraints + ball sockets.
        if rig_type == RigType::R15 && rig::joint_upgrade(&self.doc) {
            rig::upgrade_rig(&mut self.doc.dom, m);
        }
        // StarterCharacterScripts go into the character.
        let root_ref = self.doc.dom.root_ref();
        if let Some(scs) = self
            .doc
            .find_child(root_ref, "StarterPlayer")
            .and_then(|sp| self.doc.find_child(sp, "StarterCharacterScripts"))
        {
            for k in self
                .doc
                .dom
                .get_by_ref(scs)
                .map(|i| i.children().to_vec())
                .unwrap_or_default()
            {
                let c = self.doc.dom.clone_within(k);
                self.doc.dom.transfer_within(c, m);
            }
        }
        // Roblox's Health and Animate, unless the place brought its own.
        crate::default_scripts::insert_character_scripts(&mut self.doc, m, rig_type);
        if let Some(inst) = self.doc.dom.get_by_ref_mut(player) {
            inst.properties.insert(ustr("Character"), Variant::Ref(m));
        }
        self.doc.mark_prop(player, ustr("Character"));
        self.doc.mark_tree(m);
        let humanoid = self
            .doc
            .dom
            .get_by_ref(m)
            .and_then(|i| {
                i.children()
                    .iter()
                    .copied()
                    .find(|c| self.doc.class_of(*c).is_some_and(|k| k == "Humanoid"))
            })
            .unwrap_or_else(Ref::none);
        // Which parts collide, as on Studio's spawned character (round 22,
        // movement_probe: R15 HumanoidRootPart, UpperTorso, LowerTorso, Head;
        // R6 Torso, Head; limbs and R6's root don't). rustudio's controller
        // moves the body itself; scripts read these.
        let body: Vec<(Ref, bool)> = self
            .doc
            .dom
            .get_by_ref(m)
            .map(|i| i.children().to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| {
                let i = self.doc.dom.get_by_ref(c)?;
                doc::is_a(&i.class, "BasePart").then(|| (c, studio_collides(&i.name, rig_type)))
            })
            .collect();
        for (c, on) in body {
            if let Some(i) = self.doc.dom.get_by_ref_mut(c) {
                i.properties.insert(ustr("CanCollide"), Variant::Bool(on));
            }
        }
        let rig = Rig::read(&self.doc, m);
        let touch_part = self
            .doc
            .find_named(
                m,
                if rig_type == RigType::R15 {
                    "LowerTorso"
                } else {
                    "Torso"
                },
            )
            .unwrap_or(rig.root);
        let start = self.character_spawn_point();
        let root_lift =
            rig::root_height(&self.doc, m)
                - (mech::CHARACTER_HALF_HEIGHT + mech::CHARACTER_RADIUS + mech::CHARACTER_HOVER);
        let ch = Character {
            model: m,
            accessories_seen: usize::MAX,
            floor_material: 0,
            humanoid,
            rig,
            rig_type,
            touch_part,
            body: CharBody {
                pos: start,
                vel_y: 0.0,
                grounded: false,
                ..Default::default()
            },
            root_lift,
            yaw: 0.0,
            velocity: Vec3::ZERO,
            dead: false,
            touching: HashSet::new(),
            seat_seen: HashSet::new(),
            state: State::Freefall,
            landed_for: 0.0,
            last_speed: -1.0,
            last_root: None,
            seat: None,
            client_sit: false,
            shape: hum_shape(&self.doc, m, root_lift),
            hum_mode_left: false,
            pitch: 0.0,
            wall: None,
            seat_cooldown: 0.0,
            forced: None,
            forced_for: 0.0,
            last_move_speed: -1.0,
        };
        let mut ch = ch;
        let pose = character_pose(&ch, &HashMap::new());
        // Where it stands now: a script moving it before the first frame
        // (PivotTo at spawn) is a teleport too.
        ch.last_root = pose.iter().find(|(r, _)| *r == ch.rig.root).map(|(_, x)| *x);
        rig::write_pose(&mut self.doc, &pose, None);
        let parts: Vec<Ref> = self
            .doc
            .dom
            .descendants_of(m)
            .filter(|i| doc::is_a(&i.class, "BasePart"))
            .map(|i| i.referent())
            .collect();
        if let Some(p) = self.play.as_mut() {
            p.physics.skip.extend(parts.iter().copied());
            p.physics.refresh(parts);
            p.character = Some(ch);
            p.respawn_at = None;
        }

        // Server: CharacterAdded, server Scripts inside the character.
        let server_scripts = self.server_scripts(Some(m));
        self.with_side(Side::Server, |rt, _, _| {
            rt.fire(player, "CharacterAdded", vec![rt.inst(m)]);
            for r in server_scripts {
                rt.start_collected(r);
            }
            rt.dispatch();
        });
        // Replicate the new character, then the client's CharacterAdded and LocalScripts.
        self.record(RepOp::Parent(m));
        self.record(RepOp::Prop(player, "Character".into()));
        self.replicate_now();
        let (cplayer, cmodel) = match self.play.as_ref() {
            Some(s) => (s.map.s2c.get(&player).copied(), s.map.s2c.get(&m).copied()),
            None => (None, None),
        };
        if let (Some(cp), Some(cm)) = (cplayer, cmodel) {
            let local = self
                .play
                .as_ref()
                .and_then(|s| s.other_doc.as_ref())
                .map(|cd| client_scripts(cd, &[Some(cm)]))
                .unwrap_or_default();
            // The client's camera follows the new character's Humanoid.
            let chum = self
                .play
                .as_ref()
                .and_then(|s| s.map.s2c.get(&humanoid).copied());
            let ccam = self
                .play
                .as_ref()
                .and_then(|s| s.other_doc.as_ref())
                .and_then(
                    |cd| match cd.workspace().and_then(|ws| cd.prop(ws, "CurrentCamera")) {
                        Some(Variant::Ref(c)) if cd.exists(c) => Some(c),
                        _ => None,
                    },
                );
            self.with_side(Side::Client, |rt, _, _| {
                if let (Some(cam), Some(h)) = (ccam, chum) {
                    rt.set(cam, "CameraSubject", rt.inst(h));
                }
                rt.fire(cp, "CharacterAdded", vec![rt.inst(cm)]);
                for r in local {
                    rt.start_collected(r);
                }
                rt.dispatch();
            });
        }
    }

    /// The place's avatar setting (StarterPlayer.GameSettingsAvatar: R6 /
    /// R15; PlayerChoice leaves it to the avatar).
    pub fn place_rig(&self) -> Option<RigType> {
        let root = self.doc.dom.root_ref();
        self.doc
            .find_child(root, "StarterPlayer")
            .and_then(|sp| crate::scene::enum_u32(self.doc.prop(sp, "GameSettingsAvatar")))
            .and_then(|v| match v {
                0 => Some(RigType::R6),
                1 => Some(RigType::R15),
                _ => None,
            })
    }

    /// Which rig the next character gets and which input decided it: the
    /// picker when it names one (an explicit choice wins), else the place's
    /// avatar setting, else the signed-in avatar, else the default (R15).
    pub fn rig_choice(&self) -> (RigType, RigSource) {
        if let Some(r) = self.play_rig {
            return (r, RigSource::Picker);
        }
        if let Some(r) = self.place_rig() {
            return (r, RigSource::Place);
        }
        if let Some(a) = self.account.avatar() {
            return (
                if a.r15 { RigType::R15 } else { RigType::R6 },
                RigSource::Avatar,
            );
        }
        let def = if mech::DEFAULT_RIG_R15 {
            RigType::R15
        } else {
            RigType::R6
        };
        (def, RigSource::Default)
    }

    /// Rig type, colours and scales for the next character: the rig from
    /// [`Self::rig_choice`]; colours and (R15) scales from the signed-in
    /// avatar when it has been fetched, else the default body's.
    fn character_look(&self) -> (RigType, rig::BodyColors, rig::Scales) {
        let (rig, _) = self.rig_choice();
        match self.account.avatar() {
            Some(a) => (
                rig,
                a.colors,
                if rig == RigType::R15 {
                    a.scales
                } else {
                    rig::Scales::default()
                },
            ),
            None => (rig, rig::BodyColors::default(), rig::Scales::default()),
        }
    }

    /// A property of the player's Humanoid as its owner (the client) has it.
    fn owned_humanoid_prop(&self, h: Ref, name: &str) -> Option<Variant> {
        let client = self
            .play
            .as_ref()
            .and_then(|s| s.other_doc.as_ref().zip(s.map.s2c.get(&h).copied()));
        match client {
            Some((cd, ch)) => cd.prop(ch, name),
            None => self.doc.prop(h, name),
        }
    }

    fn owned_humanoid_f32(&self, h: Ref, name: &str, default: f32) -> f32 {
        f32_of(self.owned_humanoid_prop(h, name)).unwrap_or(default)
    }

    fn humanoid_f32(&self, h: Ref, name: &str, default: f32) -> f32 {
        f32_of(self.doc.prop(h, name)).unwrap_or(default)
    }

    /// Death: joints break (BreakJointsOnDeath), the parts fall loose, and
    /// the character respawns after Players.RespawnTime.
    fn kill_character(&mut self) {
        let auto = self.character_auto_loads();
        let (model, parts) = {
            let Some(s) = self.play.as_mut() else { return };
            let Some(ch) = s.character.as_mut() else {
                return;
            };
            if ch.dead {
                return;
            }
            ch.dead = true;
            ch.state = State::Dead;
            s.respawn_at = auto.then(|| s.time + mech::RESPAWN_TIME as f64);
            let parts: Vec<Ref> = self
                .doc
                .dom
                .descendants_of(ch.model)
                .filter(|i| doc::is_a(&i.class, "BasePart"))
                .map(|i| i.referent())
                .collect();
            (ch.model, parts)
        };
        let motors: Vec<Ref> = self
            .doc
            .dom
            .descendants_of(model)
            .filter(|i| crate::rig::is_animated_joint(&i.class))
            .map(|i| i.referent())
            .collect();
        for m in &motors {
            self.doc.dom.transfer_within(*m, Ref::none());
            self.doc.mark_tree(*m);
        }
        // An upgraded R15 body's ball sockets are disabled with it (Studio).
        for b in crate::rig::ball_sockets(&self.doc, model) {
            if let Some(inst) = self.doc.dom.get_by_ref_mut(b) {
                inst.properties.insert(ustr("Enabled"), Variant::Bool(false));
            }
            self.doc.mark_prop(b, ustr("Enabled"));
            self.record(RepOp::Prop(b, "Enabled".into()));
        }
        for r in &parts {
            if let Some(inst) = self.doc.dom.get_by_ref_mut(*r) {
                inst.properties
                    .insert(ustr("CanCollide"), Variant::Bool(true));
            }
            self.doc.mark_prop(*r, ustr("CanCollide"));
        }
        if let Some(s) = self.play.as_mut() {
            s.physics.skip.retain(|x| !parts.contains(x));
            s.physics.refresh(parts.iter().copied());
        }
        for m in motors {
            self.record(RepOp::Parent(m));
        }
        if auto {
            self.warn(format!(
                "{} died; respawning in {:.0} s",
                self.player_name(),
                mech::RESPAWN_TIME
            ));
        } else {
            self.warn(format!(
                "{} died (CharacterAutoLoads is off: a script must call LoadCharacter)",
                self.player_name()
            ));
        }
    }

    fn respawn(&mut self) {
        if let Some(old) = self.play.as_ref().and_then(|s| s.character.as_ref()).map(|c| c.model) {
            self.fire_character_removing(old);
        }
        if let Some(old) = self.play.as_mut().and_then(|s| s.character.take()) {
            self.doc.dom.transfer_within(old.model, Ref::none());
            self.doc.mark_tree(old.model);
            self.record(RepOp::Parent(old.model));
        }
        self.spawn_character();
    }

    /// After the client's scripts ran: UserInputService.MouseBehavior /
    /// MouseIconEnabled (the view grabs the cursor) and UserGameSettings.RotationType.
    fn read_client_input_state(&mut self) {
        let Some(s) = self.play.as_mut() else { return };
        if let Some(cd) = s.other_doc.as_ref() {
            if let Some(u) = cd.find_child(cd.dom.root_ref(), "UserInputService") {
                let inst = cd.dom.get_by_ref(u);
                s.mouse_behavior = match inst.and_then(|i| i.properties.get(&ustr("MouseBehavior")))
                {
                    Some(Variant::Enum(e)) => e.to_u32(),
                    _ => 0,
                };
                s.mouse_icon = !matches!(
                    inst.and_then(|i| i.properties.get(&ustr("MouseIconEnabled"))),
                    Some(Variant::Bool(false))
                );
            }
            // The cursor image: UserInputService.MouseIcon wins over the
            // PlayerMouse's Icon (Roblox); "" means the system arrow.
            let content = |r: Ref, name: &str| match cd.prop(r, name) {
                Some(Variant::String(t)) => Some(t),
                Some(Variant::ContentId(c)) => Some(c.as_str().to_string()),
                Some(Variant::Content(c)) => c.as_uri().map(|u| u.to_string()),
                _ => None,
            }
            .filter(|t| !t.trim().is_empty());
            let uis_icon = cd
                .find_child(cd.dom.root_ref(), "UserInputService")
                .and_then(|u| content(u, "MouseIcon"));
            let mouse = s
                .client
                .as_ref()
                .and_then(|c| c.world.as_ref())
                .and_then(|w| w.input.mouse);
            s.cursor_image = uis_icon.or_else(|| mouse.and_then(|m| content(m, "Icon")));
        }
        if let Some(c) = s.client.as_ref() {
            let rot: Option<String> = c
                .runtime
                .lua
                .globals()
                .get::<mlua::Table>("__rs_user_game_settings")
                .ok()
                .and_then(|t| t.get::<Value>("RotationType").ok())
                .and_then(|v| match v {
                    Value::UserData(ud) => ud.borrow::<LEnumItem>().ok().map(|e| e.name.clone()),
                    _ => None,
                });
            s.camera_relative = rot.as_deref() == Some("CameraRelative");
        }
    }

    /// What the client's own constraints and scripts do to its HumanoidRootPart
    /// this frame: an enabled LinearVelocity in Plane mode sets the horizontal
    /// velocity (PlaneVelocity along its tangent axes, world-relative), a rigid
    /// one-attachment AlignOrientation sets the facing, and a script setting
    /// AssemblyLinearVelocity sets the vertical speed (Roblox physics does
    /// this with forces; rustudio's character is kinematic).
    fn client_root_movers(&mut self) -> RootMovers {
        let mut out = RootMovers::default();
        let Some(s) = self.play.as_mut() else {
            return out;
        };
        let Some(root) = s
            .character
            .as_ref()
            .and_then(|c| s.map.s2c.get(&c.rig.root).copied())
        else {
            return out;
        };
        if let Some(w) = s.client.as_mut().and_then(|c| c.world.as_mut()) {
            if let Some((_, v)) = w.velocity_requests.iter().rev().find(|(r, _)| *r == root) {
                out.vel_y = Some(v.y);
            }
            w.velocity_requests.retain(|(r, _)| *r != root);
        }
        let Some(cd) = s.other_doc.as_ref() else {
            return out;
        };
        let enabled = |r: Ref| !matches!(cd.prop(r, "Enabled"), Some(Variant::Bool(false)));
        let attached_here = |r: Ref| matches!(cd.prop(r, "Attachment0"), Some(Variant::Ref(a)) if cd.dom.get_by_ref(a).is_some_and(|i| i.parent() == root));
        let v3 = |v: Option<Variant>| match v {
            Some(Variant::Vector3(v)) => Some(Vec3::new(v.x, v.y, v.z)),
            _ => None,
        };
        for c in cd
            .dom
            .get_by_ref(root)
            .map(|i| i.children().to_vec())
            .unwrap_or_default()
        {
            let Some(class) = cd.class_of(c) else {
                continue;
            };
            if !enabled(c) || !attached_here(c) {
                continue;
            }
            match class.as_str() {
                // VelocityConstraintMode.Plane = 1, RelativeTo World = 2
                "LinearVelocity"
                    if crate::scene::enum_u32(cd.prop(c, "VelocityConstraintMode")) == Some(1)
                        && crate::scene::enum_u32(cd.prop(c, "RelativeTo")) == Some(2) =>
                {
                    let pv = match cd.prop(c, "PlaneVelocity") {
                        Some(Variant::Vector2(v)) => glam::Vec2::new(v.x, v.y),
                        _ => glam::Vec2::ZERO,
                    };
                    let a = v3(cd.prop(c, "PrimaryTangentAxis")).unwrap_or(Vec3::X);
                    let b = v3(cd.prop(c, "SecondaryTangentAxis")).unwrap_or(Vec3::Z);
                    let v = a * pv.x + b * pv.y;
                    out.plane = Some(Vec3::new(v.x, 0.0, v.z));
                }
                // VelocityConstraintMode.Vector = 2 (the default), RelativeTo
                // World (the default): with force enough to carry the
                // character (or no limit) it sets the whole velocity.
                "LinearVelocity"
                    if crate::scene::enum_u32(cd.prop(c, "VelocityConstraintMode")).unwrap_or(2) == 2
                        && crate::scene::enum_u32(cd.prop(c, "RelativeTo")).unwrap_or(2) == 2
                        && (matches!(cd.prop(c, "ForceLimitsEnabled"), Some(Variant::Bool(false)))
                            || crate::scene::f32_of(cd.prop(c, "MaxForce")).unwrap_or(1000.0)
                                >= crate::mech::MOVER_HOLDS_CHARACTER_FORCE) =>
                {
                    out.vector = Some(v3(cd.prop(c, "VectorVelocity")).unwrap_or(Vec3::ZERO));
                }
                // OrientationAlignmentMode.OneAttachment = 0
                "AlignOrientation" if crate::scene::enum_u32(cd.prop(c, "Mode")).unwrap_or(0) == 0 => {
                    if let Some(Variant::CFrame(cf)) = cd.prop(c, "CFrame") {
                        let look = -Xform::from_cframe(&cf).rot.z_axis;
                        if look.x.abs() + look.z.abs() > 1e-4 {
                            out.yaw = Some((-look.x).atan2(-look.z));
                        }
                    }
                    out.yaw_rate = if matches!(cd.prop(c, "RigidityEnabled"), Some(Variant::Bool(true))) {
                        0.0
                    } else {
                        crate::scene::f32_of(cd.prop(c, "Responsiveness")).unwrap_or(10.0)
                    };
                }
                _ => {}
            }
        }
        out
    }

    /// Each joint's Motor6D.Transform as its owner (the client) has it: what
    /// the Animator's tracks, or the game's own scripts, wrote this frame.
    /// Keyed by the server's motor; unset ones are left out.
    fn joint_transforms(&self) -> HashMap<Ref, Xform> {
        let mut out = HashMap::new();
        let Some(s) = self.play.as_ref() else {
            return out;
        };
        let Some(ch) = s.character.as_ref() else {
            return out;
        };
        for j in &ch.rig.joints {
            let client = s.other_doc.as_ref().zip(s.map.s2c.get(&j.motor).copied());
            let v = match client {
                Some((cd, m)) => cd
                    .dom
                    .get_by_ref(m)
                    .and_then(|i| i.properties.get(&ustr("Transform")).cloned()),
                None => self
                    .doc
                    .dom
                    .get_by_ref(j.motor)
                    .and_then(|i| i.properties.get(&ustr("Transform")).cloned()),
            };
            if let Some(Variant::CFrame(cf)) = v {
                out.insert(j.motor, Xform::from_cframe(&cf));
            }
        }
        out
    }

    /// The client Humanoid's MoveDirection (at most length 1; it only has a
    /// vertical part while swimming) and AutoRotate, for the server-side
    /// `humanoid`.
    fn client_move_request(&self, humanoid: Ref) -> (Vec3, bool) {
        let Some(s) = self.play.as_ref() else {
            return (Vec3::ZERO, true);
        };
        let Some((cd, h)) = s.other_doc.as_ref().zip(s.map.s2c.get(&humanoid).copied()) else {
            return (Vec3::ZERO, true);
        };
        let dir = match cd.prop(h, "MoveDirection") {
            Some(Variant::Vector3(v)) => Vec3::new(v.x, v.y, v.z),
            _ => Vec3::ZERO,
        };
        let dir = if dir.length() > 1.0 {
            dir.normalize()
        } else {
            dir
        };
        let auto = !matches!(cd.prop(h, "AutoRotate"), Some(Variant::Bool(false)));
        (if dir.is_finite() { dir } else { Vec3::ZERO }, auto)
    }

    /// Seats (Seat / VehicleSeat), as Roblox does them: touching an enabled,
    /// empty seat (or Seat:Sit) sits the humanoid — a SeatWeld from the seat
    /// to the HumanoidRootPart, Seat.Occupant, Humanoid.Sit and SeatPart —
    /// and jumping, Sit = false, a removed SeatWeld or a gone / disabled seat
    /// gets it up again. The welds and properties are made by the server's
    /// library (`__rs_seat` / `__rs_unseat`) so they replicate and fire
    /// Changed like a script's.
    fn update_seat(&mut self, humanoid: Ref, jump: bool, dt: f32) -> Option<SeatChange> {
        let (seat, touching, cooldown, root, dead, asked, seat_off, seen) = {
            let s = self.play.as_mut()?;
            let asked = s.game.sit_on.take();
            let off = s.game.states_off.contains("Seated");
            let ch = s.character.as_mut()?;
            ch.seat_cooldown = (ch.seat_cooldown - dt).max(0.0);
            let seen = std::mem::replace(&mut ch.seat_seen, ch.touching.clone());
            (
                ch.seat,
                ch.touching.clone(),
                ch.seat_cooldown,
                ch.rig.root,
                ch.dead,
                asked,
                off,
                seen,
            )
        };
        let usable = |doc: &Document, r: Ref| {
            doc.class_of(r)
                .is_some_and(|c| c == "Seat" || c == "VehicleSeat")
                && doc.dom.get_by_ref(r).is_some_and(|i| i.parent().is_some())
                && !matches!(doc.prop(r, "Disabled"), Some(Variant::Bool(true)))
        };
        // The client's Sit going from true to false (its LocalScript stood up).
        let client_sit = matches!(self.owned_humanoid_prop(humanoid, "Sit"), Some(Variant::Bool(true)));
        let client_stood = {
            let ch = self.play.as_mut().and_then(|s| s.character.as_mut());
            ch.map(|c| std::mem::replace(&mut c.client_sit, client_sit) && !client_sit).unwrap_or(false)
        };
        if let Some(seat) = seat {
            let gone = !usable(&self.doc, seat)
                || self.doc.find_named(seat, "SeatWeld").is_none()
                || client_stood
                || matches!(self.doc.prop(humanoid, "Sit"), Some(Variant::Bool(false)));
            if !(jump || gone || dead) {
                return None;
            }
            self.with_side(Side::Server, |rt, _, _| {
                rt.spawn_global("__rs_unseat", vec![rt.inst(humanoid), rt.inst(seat)]);
            });
            // Out of the seat: the capsule rises clear of the seat's top (it
            // sat sunk into it).
            let top = crate::scene::xform_of(&self.doc, seat).map(|xf| {
                let h = match self.doc.prop(seat, "Size") {
                    Some(Variant::Vector3(v)) => v.y,
                    _ => 1.0,
                };
                xf.pos.y + h / 2.0
            });
            if let Some(ch) = self.play.as_mut().and_then(|s| s.character.as_mut()) {
                ch.seat = None;
                ch.seat_cooldown = mech::SEAT_COOLDOWN_S;
                if let Some(top) = top {
                    let clear = top
                        + mech::CHARACTER_HALF_HEIGHT
                        + mech::CHARACTER_RADIUS
                        + mech::CHARACTER_HOVER;
                    ch.body.pos.y = ch.body.pos.y.max(clear);
                }
            }
            return Some(SeatChange {
                active: false,
                seat: None,
            });
        }
        if dead || seat_off {
            return None;
        }
        let empty = |doc: &Document, r: Ref| match doc.prop(r, "Occupant") {
            Some(Variant::Ref(o)) => o.is_none() || o == humanoid,
            _ => true,
        };
        // Only a touch that began (Studio: standing on a seat after jumping
        // off it doesn't sit again; round 22 movement_probe seat).
        let pick = asked.filter(|r| usable(&self.doc, *r)).or_else(|| {
            (cooldown <= 0.0)
                .then(|| {
                    touching
                        .iter()
                        .copied()
                        .filter(|r| !seen.contains(r))
                        .find(|r| usable(&self.doc, *r) && empty(&self.doc, *r))
                })
                .flatten()
        })?;
        self.with_side(Side::Server, |rt, _, _| {
            rt.spawn_global(
                "__rs_seat",
                vec![rt.inst(humanoid), rt.inst(pick), rt.inst(root)],
            );
        });
        if let Some(ch) = self.play.as_mut().and_then(|s| s.character.as_mut()) {
            ch.seat = Some(pick);
        }
        Some(SeatChange {
            active: true,
            seat: Some(pick),
        })
    }

    /// Players.CharacterAutoLoads (default true).
    fn character_auto_loads(&self) -> bool {
        let root = self.doc.dom.root_ref();
        !self.doc.find_child(root, "Players").is_some_and(|p| {
            matches!(
                self.doc.prop(p, "CharacterAutoLoads"),
                Some(Variant::Bool(false))
            )
        })
    }

    /// One frame of the play session.
    pub fn play_tick(&mut self, ctx: &egui::Context, dt: f32) {
        let Some(mode) = self.play.as_ref().map(|p| p.mode) else {
            return;
        };
        // Roblox steps the game by at most 1/15 s a frame: slower frames slow
        // time(), physics and animations, while waits keep wall time (World::realtime).
        let realtime = self.fixed_dt.is_none();
        let dt = dt.clamp(0.0, crate::mech::MAX_FRAME_DT);
        // The in-game menu's Leave stops the session; Reset Character kills
        // the character on the server (Humanoid.Health = 0, as Roblox does).
        if self.play.as_ref().is_some_and(|s| s.game.leave_request) {
            self.stop_play();
            return;
        }
        // StudioTestService:EndTest ends the session; the Studio services
        // read the editor's settings (runtime/domains/testing*.rs).
        {
            use crate::runtime::domains::testing as t;
            if let Some(value) = t::take_end_request() {
                self.stop_play();
                t::report_test_run(&value, &self.log);
                return;
            }
            t::set_editor_state(t::EditorState {
                snap_move: self.snap_move,
                move_inc: self.move_inc,
                snap_rotate: self.snap_rotate,
                rotate_inc: self.rotate_inc,
                local_space: self.local_space,
                ..Default::default()
            });
        }
        self.server_front();
        if let Some(h) = self.play.as_mut().and_then(|s| {
            std::mem::take(&mut s.game.reset_request)
                .then(|| s.character.as_ref().map(|c| c.humanoid))
                .flatten()
        }) {
            self.with_side(Side::Server, |rt, _, _| {
                rt.set(h, "Health", Value::Number(0.0));
            });
        }
        // The menu's Settings: UserGameSettings changes on the client.
        let menu_lua = self
            .play
            .as_mut()
            .map(|s| std::mem::take(&mut s.game.menu.lua))
            .unwrap_or_default();
        for code in menu_lua {
            if let Some(e) = self.run_luau_as(Side::Client, &code, crate::runtime::security::CORE) {
                self.warn(format!("Settings: {e}"));
            }
        }

        // Keys: character movement and UserInputService (client side).
        // A focused TextBox has the keys: the character doesn't hear them, but
        // UserInputService still does, with gameProcessedEvent true (chat typing).
        // The key positions are physical, like Roblox's KeyCodes: WASD moves
        // from the keys labelled ZQSD on AZERTY (src/keys.rs).
        let textbox = self.gui.focus.is_some();
        let blocked = ctx.egui_wants_keyboard_input()
            || self.play.as_ref().is_some_and(|s| s.game.menu_open);
        let typing = blocked || textbox;
        let raw_keys: HashSet<String> = if blocked {
            HashSet::new()
        } else {
            let mut k: HashSet<String> = self
                .phys
                .iter()
                .filter_map(|k| key_name(k))
                .map(String::from)
                .collect();
            // egui reports modifiers apart from keys (shift-lock is LeftShift).
            let mods = ctx.input(|i| i.modifiers);
            for (on, name) in [
                (mods.shift || self.harness.shift, "LeftShift"),
                (mods.ctrl, "LeftControl"),
                (mods.alt, "LeftAlt"),
            ] {
                if on {
                    k.insert(name.to_string());
                }
            }
            k
        };
        let keys: HashSet<String> = if typing { HashSet::new() } else { raw_keys.clone() };
        // (name, gameProcessedEvent)
        let (began, ended): (Vec<(String, bool)>, Vec<(String, bool)>) = {
            let s = self.play.as_mut().expect("playing");
            let mut b: Vec<(String, bool)> = raw_keys
                .difference(&s.keys_prev)
                .map(|k| (k.clone(), textbox))
                .collect();
            let mut e: Vec<String> = s.keys_prev.difference(&raw_keys).cloned().collect();
            // A tap that came and went between two frames: began and ended now
            // (processed when a TextBox had the keys as it was pressed, the
            // Enter that submits it included).
            let taps = std::mem::take(&mut s.game.key_taps);
            if !blocked {
                for (k, processed) in taps {
                    let Some(k) = key_name(k) else { continue };
                    let k = k.to_string();
                    if !raw_keys.contains(&k)
                        && !s.keys_prev.contains(&k)
                        && !b.iter().any(|(n, _)| *n == k)
                    {
                        b.push((k.clone(), processed));
                        e.push(k);
                    }
                }
            }
            for (k, p) in &b {
                if *p {
                    s.keys_processed.insert(k.clone());
                }
            }
            let e = e
                .into_iter()
                .map(|k| {
                    let p = s.keys_processed.remove(&k);
                    (k, p)
                })
                .collect();
            s.keys_prev = raw_keys;
            (b, e)
        };

        // 1. Server script changes from last frame reach physics.
        let sc = crate::bench::scope("p1_physics_sync");
        {
            let s = self.play.as_mut().expect("playing");
            let w = s.server.world.as_mut().expect("server world");
            let vel = std::mem::take(&mut w.velocity_requests);
            s.physics.sync(&mut self.doc);
            if let Some(c) = s.other_doc.as_mut() {
                s.physics.follow_client(&self.doc, c, &s.map.c2s);
            }
            for (r, v) in vel {
                s.physics.set_velocity(r, v);
            }
            s.time += dt as f64;
        }

        drop(sc);
        // Clicks, chat, tool keys and mouse buttons from the viewport.
        self.game_inputs();
        // 2. Character (owned by the client: written to both DataModels).
        let sc = crate::bench::scope("p2_character");
        let mut char_touches: Vec<Ref> = Vec::new();
        let mut state_change: Option<(State, State)> = None;
        let mut seat_change: Option<SeatChange> = None;
        let mut move_event: Option<(&'static str, f32)> = None;
        let mut running_speed: Option<f32> = None;
        let mut floor_changed: Option<u32> = None;
        let mut char_part = Ref::none();
        let mut humanoid_ref = Ref::none();
        if mode == RunMode::Play && self.play_has_character() {
            let (humanoid, touch_part) = {
                let ch = self
                    .play
                    .as_ref()
                    .and_then(|s| s.character.as_ref())
                    .expect("character");
                (ch.humanoid, ch.touch_part)
            };
            humanoid_ref = humanoid;
            char_part = touch_part;
            // A script moved the character (PivotTo, HumanoidRootPart.CFrame,
            // on either side): the controller carries on from there.
            if let Some(s) = self.play.as_mut() {
                if let Some(ch) = s.character.as_mut() {
                    if let Some(last) = ch.last_root {
                        let root = ch.rig.root;
                        let client = s.map.s2c.get(&root).and_then(|c| {
                            s.other_doc
                                .as_ref()
                                .and_then(|cd| crate::scene::xform_of(cd, *c))
                        });
                        let server = crate::scene::xform_of(&self.doc, root);
                        if let Some(xf) = [client, server]
                            .into_iter()
                            .flatten()
                            .find(|xf| (xf.pos - last.pos).length() > mech::TELEPORT_MIN_STUDS)
                        {
                            ch.body.pos = xf.pos - Vec3::Y * ch.root_lift;
                            ch.body.vel_y = 0.0;
                            let look = -xf.rot.z_axis;
                            ch.yaw = (-look.x).atan2(-look.z);
                        }
                    }
                }
            }
            // The client owns its character: its Humanoid's values count (a
            // LocalScript changing WalkSpeed moves the player at once).
            let speed = self.owned_humanoid_f32(humanoid, "WalkSpeed", mech::WALK_SPEED);
            let use_power = matches!(
                self.owned_humanoid_prop(humanoid, "UseJumpPower"),
                Some(Variant::Bool(true))
            );
            // Studio's Humanoid jump speed: JumpPower with
            // UseJumpPower, else √(2·g·JumpHeight) / 1.06 clamped to [0, 1000];
            // the jump then leaves at 1.06 × that (its inverse:
            // height = (1.06·v)² / 2g).
            let commanded = if use_power {
                self.owned_humanoid_f32(humanoid, "JumpPower", mech::JUMP_POWER)
            } else {
                let g = self
                    .play
                    .as_ref()
                    .map(|s| s.physics.gravity)
                    .unwrap_or(mech::GRAVITY);
                ((2.0 * g * self.owned_humanoid_f32(humanoid, "JumpHeight", mech::JUMP_HEIGHT))
                    .max(0.0)
                    .sqrt()
                    / mech::JUMP_TAKEOFF_FACTOR)
                    .clamp(0.0, mech::JUMP_SPEED_MAX)
            };
            let jump_v = commanded * mech::JUMP_TAKEOFF_FACTOR;
            // The client's Humanoid carries what its scripts asked for: the
            // ControlModule's Player:Move sets MoveDirection every frame and
            // Humanoid.Jump while Space is held (Roblox's own PlayerModule).
            let (move_3d, auto_rotate) = self.client_move_request(humanoid);
            // Walking uses the level part; swimming the whole of it.
            let mut dir = Vec3::new(move_3d.x, 0.0, move_3d.z);
            // Humanoid:MoveTo walks there until a movement key is pressed.
            let (target, jump_req, at) = self
                .play
                .as_mut()
                .map(|s| {
                    (
                        s.game.move_target,
                        std::mem::take(&mut s.game.jump),
                        s.character
                            .as_ref()
                            .map(|c| c.body.pos)
                            .unwrap_or(Vec3::ZERO),
                    )
                })
                .unwrap_or((None, false, Vec3::ZERO));
            if dir != Vec3::ZERO {
                if let Some(s) = self.play.as_mut() {
                    s.game.move_target = None;
                }
            } else if let Some(t) = target {
                let mut to = t - at;
                to.y = 0.0;
                if to.length() < mech::MOVE_TO_REACHED {
                    if let Some(s) = self.play.as_mut() {
                        s.game.move_target = None;
                    }
                    self.with_side(Side::Server, |rt, _, _| {
                        rt.fire(humanoid, "MoveToFinished", vec![Value::Boolean(true)])
                    });
                } else {
                    dir = to.normalize();
                }
            }
            // Humanoid:ChangeState from scripts: FallingDown / GettingUp /
            // Ragdoll / Physics hold the humanoid until they end (FallingDown
            // gets up by itself), Seated sits it where it is, Dead kills it,
            // the walking states end a forced one. Disabled states are ignored.
            let (forced_req, states_off) = {
                let s = self.play.as_mut().expect("playing");
                (s.game.forced_state.take(), s.game.states_off.clone())
            };
            let mut force_dead = false;
            if let Some(name) = forced_req.filter(|n| !states_off.contains(n)) {
                match State::from_name(&name) {
                    Some(State::Dead) => force_dead = true,
                    Some(State::Seated) => {
                        if let Some(inst) = self.doc.dom.get_by_ref_mut(humanoid) {
                            inst.properties.insert(ustr("Sit"), Variant::Bool(true));
                        }
                        self.doc.mark_prop(humanoid, ustr("Sit"));
                    }
                    other => {
                        let s = self.play.as_mut().expect("playing");
                        let ch = s.character.as_mut().expect("character");
                        match other {
                            Some(
                                st @ (State::FallingDown
                                | State::GettingUp
                                | State::Ragdoll
                                | State::Physics),
                            ) => {
                                ch.forced = Some(st);
                                ch.forced_for = 0.0;
                            }
                            Some(_) => ch.forced = None,
                            None => {}
                        }
                    }
                }
            }
            let downed = {
                let s = self.play.as_mut().expect("playing");
                let ch = s.character.as_mut().expect("character");
                ch.forced_for += dt;
                match ch.forced {
                    Some(State::FallingDown) if ch.forced_for >= mech::FALLING_DOWN_S => {
                        ch.forced = (!states_off.contains("GettingUp")).then_some(State::GettingUp);
                        ch.forced_for = 0.0;
                    }
                    Some(State::GettingUp) if ch.forced_for >= mech::GETTING_UP_S => {
                        ch.forced = None;
                    }
                    // Studio's Ragdoll: over after its 5 s timer,
                    // or as soon as the root is slow enough (|v| and |ω| < 1;
                    // the controller's root has no spin, so its velocity decides).
                    Some(State::Ragdoll)
                        if ch.state == State::Ragdoll
                            && (ch.forced_for >= mech::RAGDOLL_TIMER_S
                                || (jump_req && ch.forced_for >= mech::RAGDOLL_EARLY_EXIT_S)
                                || ch.velocity.length() < mech::RAGDOLL_SLOW_ENOUGH) =>
                    {
                        ch.forced = (!states_off.contains("GettingUp")).then_some(State::GettingUp);
                        ch.forced_for = 0.0;
                    }
                    _ => {}
                }
                ch.forced.is_some()
            };
            // Seats (jumping gets up).
            seat_change = self.update_seat(humanoid, jump_req && !downed, dt);
            let seat_root = self
                .play
                .as_ref()
                .and_then(|s| s.character.as_ref())
                .and_then(|c| c.seat)
                .and_then(|seat| {
                    // Where the SeatWeld holds the root: seat · C0 · C1⁻¹
                    //.
                    let xf = crate::scene::xform_of(&self.doc, seat)?;
                    let weld = self.doc.find_named(seat, "SeatWeld")?;
                    let cf = |k: &str| match self.doc.prop(weld, k) {
                        Some(Variant::CFrame(c)) => Xform::from_cframe(&c),
                        _ => Xform {
                            pos: Vec3::ZERO,
                            rot: glam::Mat3::IDENTITY,
                        },
                    };
                    use crate::ltypes::{cf_inv, cf_mul};
                    Some(cf_mul(&cf_mul(&xf, &cf("C0")), &cf_inv(&cf("C1"))))
                });
            // Sitting, PlatformStand or a forced state: no walking.
            let sit = matches!(self.doc.prop(humanoid, "Sit"), Some(Variant::Bool(true)));
            // PlatformStand from either side (the client owns the character;
            // Place C sets it on the server).
            let platform = matches!(self.doc.prop(humanoid, "PlatformStand"), Some(Variant::Bool(true)))
                || matches!(self.owned_humanoid_prop(humanoid, "PlatformStand"), Some(Variant::Bool(true)));
            let held = sit || platform || downed || seat_root.is_some();
            if held {
                dir = Vec3::ZERO;
            }
            let wants_jump = jump_req && !downed;
            if wants_jump && sit {
                if let Some(inst) = self.doc.dom.get_by_ref_mut(humanoid) {
                    inst.properties.insert(ustr("Sit"), Variant::Bool(false));
                }
                self.doc.mark_prop(humanoid, ustr("Sit"));
            }
            let left_seat = seat_change.as_ref().is_some_and(|c| !c.active);
            // Constraints the client put on the HumanoidRootPart (a game's own
            // movement, e.g. Place A's LinearVelocity "pmove" with WalkSpeed 0).
            let movers = self.client_root_movers();
            if let Some(s) = self.play.as_mut() {
                s.physics.sync_gravity(&self.doc);
            }
            let joint_xf = self.joint_transforms();
            let own: HashSet<Ref> = self
                .play
                .as_ref()
                .and_then(|s| s.character.as_ref())
                .map(|c| {
                    self.doc
                        .dom
                        .descendants_of(c.model)
                        .filter(|i| doc::is_a(&i.class, "BasePart"))
                        .map(|i| i.referent())
                        .collect()
                })
                .unwrap_or_default();
            // The character collides as its HumanoidRootPart's CollisionGroup
            // (a script on either side may have set it; the client owns it).
            let char_group = {
                let s = self.play.as_ref().expect("playing");
                let root = s.character.as_ref().map(|c| c.rig.root).unwrap_or_else(Ref::none);
                let from = |d: &Document, r: Ref| match d.prop(r, "CollisionGroup") {
                    Some(Variant::String(g)) if !g.is_empty() => Some(g),
                    _ => None,
                };
                s.map
                    .s2c
                    .get(&root)
                    .and_then(|c| s.other_doc.as_ref().and_then(|cd| from(cd, *c)))
                    .filter(|g| g != crate::collision_groups::DEFAULT)
                    .or_else(|| from(&self.doc, root))
                    .unwrap_or_else(|| crate::collision_groups::DEFAULT.to_string())
            };
            let s = self.play.as_mut().expect("playing");
            s.physics.char_group = char_group;
            let ch = s.character.as_mut().expect("character");
            // Accessories put on during Play (Humanoid:AddAccessory) attach
            // the way spawn-time ones do, by matching attachment names; parts
            // a game welds to the body go along with it (refresh_attached).
            let accessories = self
                .doc
                .dom
                .descendants_of(ch.model)
                .filter(|i| {
                    doc::is_a(&i.class, "Accoutrement")
                        || doc::is_a(&i.class, "BasePart")
                        || i.class == "WeldConstraint"
                        || i.class == "RigidConstraint"
                        || i.class == "Weld"
                })
                .count();
            if accessories != ch.accessories_seen {
                let carried = ch.rig.refresh_attached(&self.doc, ch.model);
                ch.accessories_seen = accessories;
                if !carried.is_empty() {
                    s.physics.skip.extend(carried.iter().copied());
                    s.physics.refresh(carried);
                }
            }
            if let Some(vy) = movers.vel_y {
                // The Humanoid controller (move_humanoid) integrates
                // body.vel and copies its y back to vel_y afterwards, so a
                // script's velocity has to land there too or it's lost
                // (Place A's own jump: Jumping disabled, root velocity set to the
                // take-off speed). Going up, the floor stops holding the root
                // until it falls again, as after a Humanoid jump.
                ch.body.vel_y = vy;
                ch.body.vel.y = vy;
                if vy > 0.0 {
                    ch.body.grounded = false;
                    ch.body.jump_rise = true;
                }
            }
            if let Some(y) = movers.yaw {
                if movers.yaw_rate > 0.0 {
                    let d = (y - ch.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                        - std::f32::consts::PI;
                    ch.yaw += d * (1.0 - (-movers.yaw_rate * dt).exp());
                } else {
                    ch.yaw = y;
                }
            }
            // Facing: with the camera (RotationType CameraRelative: shift-lock,
            // first person) or easing toward the walking direction.
            if auto_rotate {
                let target = if s.camera_relative {
                    let look = -self.cam.forward();
                    (look.x.abs() + look.z.abs() > 1e-4).then(|| look.x.atan2(look.z))
                } else if let Some(wn) = ch.wall.filter(|wn| dir.dot(*wn) < 0.0 && ch.body.grounded) {
                    // Pushed into a wall on a floor the body turns flush to it
                    // (Studio's contact torque beats AutoRotate: movement_probe
                    // wall_slide); in the air it keeps easing toward the walk
                    // (wall_jump_diag: −0.07 → −0.46 = the walk's yaw, round 25).
                    Some(wn.x.atan2(wn.z))
                } else {
                    (dir != Vec3::ZERO).then(|| (-dir.x).atan2(-dir.z))
                };
                if let Some(t) = target {
                    let mut d = t - ch.yaw;
                    d = (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                        - std::f32::consts::PI;
                    ch.yaw += if s.camera_relative {
                        d
                    } else {
                        d * (1.0 - (-mech::TURN_RATE * dt).exp())
                    };
                }
            }
            if left_seat && wants_jump {
                // Jumping out of a seat leaves from where it sat.
                ch.body.grounded = true;
            }
            // Swimming: the root in terrain water. Climbing: walking into a
            // TrussPart (and holding on to it while standing still).
            let in_water = seat_root.is_none()
                && !states_off.contains("Swimming")
                && s.physics.water_at(ch.body.pos) > mech::SWIM_ENTER_WATER;
            let facing = Vec3::new(-ch.yaw.sin(), 0.0, -ch.yaw.cos());
            // On a truss the root keeps facing it (climbing down moves away
            // from where it looks); otherwise the walk finds one.
            let climb_look = if ch.state == State::Climbing {
                facing
            } else {
                dir
            };
            let on_truss = seat_root.is_none()
                && !in_water
                && !held
                && !states_off.contains("Climbing")
                && climb_look != Vec3::ZERO
                && [-1.0f32, 0.0, 1.0].iter().any(|dy| {
                    s.physics
                        .part_ahead(
                            ch.body.pos + Vec3::Y * *dy,
                            climb_look,
                            mech::CHARACTER_RADIUS + mech::CLIMB_REACH,
                            &own,
                        )
                        .is_some_and(|p| self.doc.class_of(p).is_some_and(|c| c == "TrussPart"))
                });
            let was_grounded = ch.body.grounded;
            let mut walk = movers.plane.unwrap_or(dir * speed);
            let mut hum_jumped = false;
            let mut swimming_now = false;
            let mut swim_jumped = false;
            let mut hum_floor: Option<Option<Ref>> = None;
            let mut hum_velocity: Option<Vec3> = None;
            let mut climbing = false;
            if let Some(xf) = seat_root {
                ch.body.pos = xf.pos - Vec3::Y * ch.root_lift;
                ch.body.vel_y = 0.0;
                ch.body.grounded = true;
                let look = -xf.rot.z_axis;
                ch.yaw = (-look.x).atan2(-look.z);
                walk = Vec3::ZERO;
                char_touches = ch.touching.iter().copied().collect();
            } else if let Some(v) = movers.vector {
                // A LinearVelocity holds the root: its velocity, no gravity.
                ch.body.vel_y = v.y;
                walk = Vec3::new(v.x, 0.0, v.z);
                char_touches = s.physics.move_character_floating(&mut ch.body, walk, dt, &own);
            } else if in_water {
                // Studio's Swimming: a move whose level part is
                // over SWIM_FLAT_RATIO × its vertical swims level at the full
                // speed; a steeper one (the camera looking down or up) dives
                // or rises along it.
                let swim_move = if movers.plane.is_some() || dir != Vec3::new(move_3d.x, 0.0, move_3d.z) {
                    None
                } else {
                    let flat = Vec3::new(move_3d.x, 0.0, move_3d.z);
                    (move_3d.y.abs() * mech::SWIM_FLAT_RATIO >= flat.length() && move_3d.y.abs() > 1e-3)
                        .then_some(move_3d * speed)
                };
                if let Some(m) = swim_move {
                    walk = Vec3::new(m.x, 0.0, m.z);
                } else if walk.length() > 1e-4 {
                    // Level: the whole move's length along the level part.
                    let full = if movers.plane.is_some() { walk.length() } else { move_3d.length().max(dir.length()) * speed };
                    walk = walk.normalize() * full;
                }
                // The float fades out just under the surface (root centre).
                let root_y = ch.body.pos.y + ch.root_lift;
                let float = s
                    .physics
                    .water_top(ch.body.pos)
                    .map(|top| ((top - mech::SWIM_SURFACE_GAP - root_y) / mech::SWIM_SURFACE_FADE).clamp(0.0, 1.0))
                    .unwrap_or(0.0);
                // Studio's Swimming pulls the velocity toward
                // the move vector at SWIM_GAIN /s; Jump adds mass × 500 up,
                // which settles at 500 / 35 studs/s. (The idle float is the
                // water's buoyancy, still an estimate here.)
                // Jump while swimming: Studio jumps at full speed (Jumping,
                // then Swimming again a step later; held, it leaves the water).
                if jump_req {
                    ch.body.vel_y = jump_v;
                    swim_jumped = true;
                }
                let drag_k = mech::SWIM_GAIN
                    / (mech::SWIM_GAIN + mech::WATER_DRAG_PER_PART * ch.shape.parts as f32 / ch.shape.mass.max(1.0));
                let target = if jump_req {
                    jump_v
                } else if let Some(m) = swim_move {
                    m.y * drag_k
                } else if ch.rig_type == RigType::R6 {
                    mech::SWIM_FLOAT_SPEED_R6 * float
                } else {
                    mech::SWIM_FLOAT_SPEED * float
                };
                // The swim force is clamped per axis (..ce92, limits
                //): up at most SWIM_FORCE_UP_MAX, down
                // SWIM_FORCE_MAX (× the body scale³, 1 here), so a fall into
                // water is braked at 1000 / mass, not stopped at once.
                let dv = (target - ch.body.vel_y) * (1.0 - (-mech::SWIM_GAIN * dt).exp());
                let m = ch.shape.mass.max(1.0);
                ch.body.vel_y += if jump_req {
                    dv
                } else {
                    dv.clamp(-mech::SWIM_FORCE_MAX / m * dt, mech::SWIM_FORCE_UP_MAX / m * dt)
                };
                // Water drag against the swim force: per part over the mass
                // (R15 0.695 × the move speed, R6 0.829 in Studio).
                let drag = mech::WATER_DRAG_PER_PART * ch.shape.parts as f32 / ch.shape.mass.max(1.0);
                walk *= mech::SWIM_GAIN / (mech::SWIM_GAIN + drag);
                swimming_now = walk.length() > mech::CLIMB_MIN_INPUT;
                char_touches = s.physics.move_character_floating(&mut ch.body, walk, dt, &own);
            } else if on_truss && !wants_jump {
                // Studio's Climbing: straight up or down at 0.7 ×
                // the move speed — down when the move points away from where
                // the root faces (dot < −0.2) — a 0.01 creep under 0.1 of
                // input, and no sideways velocity.
                climbing = true;
                let input = walk.length();
                let away = input > 1e-4 && (walk / input).dot(facing) < mech::CLIMB_DOWN_DOT;
                let up = if input < mech::CLIMB_MIN_INPUT {
                    mech::CLIMB_CREEP
                } else {
                    input * mech::CLIMB_SPEED_FACTOR
                };
                ch.body.vel_y = if away { -up } else { up };
                walk = Vec3::ZERO;
                char_touches = s.physics.move_character_floating(&mut ch.body, walk, dt, &own);
            } else {
                if (on_truss || ch.state == State::Climbing) && wants_jump {
                    // Jumping off a truss: away from it.
                    ch.body.grounded = true;
                    walk = -facing * speed;
                }
                // Studio's Humanoid controller (round 22): floor rays, the hip
                // spring, the body box (physics::move_humanoid).
                if ch.hum_mode_left {
                    // Back from a floating mode: carry its vertical speed.
                    ch.body.vel.y = ch.body.vel_y;
                    ch.hum_mode_left = false;
                }
                // A Plane LinearVelocity holds the horizontal velocity itself.
                ch.body.walk_exact = movers.plane.is_some_and(|p| p == walk);
                let shape = ch.shape;
                let m = s.physics.move_humanoid(
                    &self.doc,
                    &mut ch.body,
                    &mut ch.yaw,
                    &shape,
                    walk,
                    wants_jump.then_some(jump_v),
                    !platform,
                    dt,
                    &own,
                );
                ch.body.vel_y = ch.body.vel.y;
                ch.wall = m.wall;
                char_touches = m.touched;
                // Still Jumping while the take-off speed is held against a wall.
                hum_jumped = m.jumped || m.jumping;
                hum_floor = Some(m.floor);
                hum_velocity = Some(m.velocity);
            }
            if hum_floor.is_none() {
                ch.hum_mode_left = true;
            }
            // Swimming along: the body lies flat, easing in; upright again
            // out of the water.
            let tilt = if swimming_now { -std::f32::consts::FRAC_PI_2 } else { 0.0 };
            ch.pitch += (tilt - ch.pitch) * (1.0 - (-mech::SWIM_TILT_RATE * dt).exp());
            let jumped = hum_jumped
                || swim_jumped
                || (wants_jump && was_grounded && !ch.body.grounded && hum_floor.is_none());
            // Humanoid.FloorMaterial: Air in the air, else the floor part's
            // Material; on terrain the voxel's material under the feet.
            let floor_part = match hum_floor {
                Some(f) => f,
                None => s.physics.floor_under(&ch.body, &own),
            };
            let floor = match floor_part {
                None => mech::MATERIAL_AIR,
                Some(p) if self.doc.class_of(p).is_some_and(|k| k == "Terrain") => s
                    .physics
                    .terrain_material_under(ch.body.pos)
                    .and_then(|m| {
                        crate::ltypes::LEnumItem::from_name("Material", crate::terrain::material_name(m))
                    })
                    .map(|e| e.value)
                    .unwrap_or(256),
                Some(p) => crate::scene::enum_u32(self.doc.prop(p, "Material")).unwrap_or(256),
            };
            // Seated, PlatformStanding, FallingDown…: Studio reads Air.
            let floor = if held { mech::MATERIAL_AIR } else { floor };
            if floor != ch.floor_material {
                ch.floor_material = floor;
                floor_changed = Some(floor);
            }
            ch.velocity = hum_velocity.unwrap_or(Vec3::new(walk.x, ch.body.vel_y, walk.z));
            let horizontal = Vec3::new(walk.x, 0.0, walk.z).length();
            // Humanoid states.
            let next = if seat_root.is_some() || (sit && ch.forced.is_none()) {
                State::Seated
            } else if let Some(f) = ch.forced {
                f
            } else if platform {
                State::PlatformStanding
            } else if in_water && !swim_jumped {
                State::Swimming
            } else if climbing {
                State::Climbing
            } else if !ch.body.grounded {
                // Studio's Jumping lasts one step, then Freefall (longer
                // against a wall: physics::move_humanoid's hold).
                if jumped {
                    State::Jumping
                } else {
                    State::Freefall
                }
            } else if matches!(ch.state, State::Freefall | State::Jumping) {
                ch.landed_for = 0.0;
                State::Landed
            } else if ch.state == State::Landed && ch.landed_for + dt * 0.5 < mech::LANDED_TO_RUNNING_S {
                ch.landed_for += dt;
                State::Landed
            } else {
                State::Running
            };
            if next != ch.state {
                state_change = Some((ch.state, next));
                s.state_log.push(next.enum_name().to_string());
                // Back on its feet: Running reports the speed again.
                // Entering Running (after landing, too) always reports it:
                // Animate goes back to idle on Running(0), else a character
                // that fell or jumped in place keeps its fall pose.
                if next == State::Running
                    || !matches!(ch.state, State::Running | State::Landed | State::Freefall | State::Jumping)
                {
                    ch.last_speed = -1.0;
                }
                ch.last_move_speed = -1.0;
                ch.state = next;
            }
            // Humanoid.Climbing(speed) / Swimming(speed) when the speed changes.
            if matches!(ch.state, State::Climbing | State::Swimming) {
                let sp = if ch.state == State::Climbing {
                    ch.body.vel_y.abs()
                } else {
                    Vec3::new(walk.x, ch.body.vel_y, walk.z).length()
                };
                if (sp - ch.last_move_speed).abs() > 0.1 {
                    move_event = Some((ch.state.enum_name(), sp));
                    ch.last_move_speed = sp;
                }
            }
            if ch.body.grounded
                && matches!(ch.state, State::Running | State::Landed)
                && (horizontal - ch.last_speed).abs() > 0.1
            {
                running_speed = Some(horizontal);
                ch.last_speed = horizontal;
            }
            let fell = ch.body.pos.y < mech::FALLEN_PARTS_HEIGHT || force_dead;
            let mut pose = character_pose(ch, &joint_xf);
            ch.last_root = pose
                .iter()
                .find(|(r, _)| *r == ch.rig.root)
                .map(|(_, x)| *x);
            // The equipped tool rides in the right hand.
            pose.extend(self.tool_pose(&pose));
            // Client first (it owns the character), then the server copy.
            if let Some(s) = self.play.as_mut() {
                if let Some(cdoc) = s.other_doc.as_mut() {
                    rig::write_pose(cdoc, &pose, Some(&s.map.s2c));
                }
            }
            rig::write_pose(&mut self.doc, &pose, None);
            if let Some(m) = floor_changed {
                let v = Variant::Enum(rbx_dom_weak::types::Enum::from_u32(m));
                if let Some(inst) = self.doc.dom.get_by_ref_mut(humanoid) {
                    inst.properties.insert(ustr("FloorMaterial"), v.clone());
                }
                self.doc.mark_prop(humanoid, ustr("FloorMaterial"));
                if let Some(s) = self.play.as_mut() {
                    if let (Some(c), Some(cdoc)) = (s.map.s2c.get(&humanoid).copied(), s.other_doc.as_mut()) {
                        if let Some(inst) = cdoc.dom.get_by_ref_mut(c) {
                            inst.properties.insert(ustr("FloorMaterial"), v);
                        }
                        cdoc.mark_prop(c, ustr("FloorMaterial"));
                    }
                }
            }
            if fell {
                if let Some(inst) = self.doc.dom.get_by_ref_mut(humanoid) {
                    inst.properties
                        .insert(ustr("Health"), Variant::Float32(0.0));
                }
                self.doc.mark_prop(humanoid, ustr("Health"));
                self.record(RepOp::Prop(humanoid, "Health".into()));
                self.kill_character();
            }
        }

        drop(sc);
        // 3. Physics (server-authoritative for everything not owned by the client).
        let sc = crate::bench::scope("p3_physics_step");
        let result = {
            let s = self.play.as_mut().expect("playing");
            s.physics.step(&mut self.doc, dt)
        };
        for (r, _) in &result.moved {
            self.record(RepOp::Moved(*r));
        }
        for r in &result.fallen {
            self.doc.dom.transfer_within(*r, Ref::none());
            self.doc.mark_tree(*r);
            self.record(RepOp::Parent(*r));
        }
        if let Some(s) = self.play.as_mut() {
            s.engine_orphans.extend(result.fallen.iter().copied());
        }

        drop(sc);
        // Touch events (CanTouch parts only).
        let sc = crate::bench::scope("p3b_touch_prep");
        let can_touch =
            |doc: &Document, r: Ref| !matches!(doc.prop(r, "CanTouch"), Some(Variant::Bool(false)));
        let part_touches: Vec<(Ref, Ref, bool)> = result
            .touches
            .iter()
            .copied()
            .filter(|(a, b, _)| can_touch(&self.doc, *a) && can_touch(&self.doc, *b))
            .collect();
        let (char_began, char_ended): (Vec<Ref>, Vec<Ref>) =
            match self.play.as_mut().and_then(|s| s.character.as_mut()) {
                Some(ch) if char_part.is_some() => {
                    let now: HashSet<Ref> = char_touches
                        .into_iter()
                        .filter(|r| can_touch(&self.doc, *r))
                        .collect();
                    let b = now.difference(&ch.touching).copied().collect();
                    let e = ch.touching.difference(&now).copied().collect();
                    ch.touching = now;
                    (b, e)
                }
                _ => (Vec::new(), Vec::new()),
            };
        let velocities = self
            .play
            .as_ref()
            .map(|s| s.physics.velocities())
            .unwrap_or_default();
        let time = self.play.as_ref().map(|s| s.time).unwrap_or(0.0);
        let state_names: HashMap<Ref, &'static str> = self
            .play
            .as_ref()
            .and_then(|s| s.character.as_ref())
            .map(|c| HashMap::from([(c.humanoid, c.state.enum_name())]))
            .unwrap_or_default();

        // 4. Server scripts.
        let fire_touches = |rt: &Runtime, map: Option<&HashMap<Ref, Ref>>| {
            let m = |r: Ref| match map {
                Some(m) => m.get(&r).copied(),
                None => Some(r),
            };
            for (a, b, started) in &part_touches {
                let ev = if *started { "Touched" } else { "TouchEnded" };
                // Studio fires the deprecated twins with them: StoppedTouching
                // with TouchEnded, LocalSimulationTouched with Touched where
                // the part is simulated (the server's parts).
                let twin = match (*started, map.is_none()) {
                    (true, true) => Some("LocalSimulationTouched"),
                    (false, _) => Some("StoppedTouching"),
                    _ => None,
                };
                if let (Some(a), Some(b)) = (m(*a), m(*b)) {
                    rt.fire(a, ev, vec![rt.inst(b)]);
                    rt.fire(b, ev, vec![rt.inst(a)]);
                    if *started {
                        crate::rig::fire_humanoid_touched(rt, a, b);
                    }
                    if let Some(t) = twin {
                        rt.fire(a, t, vec![rt.inst(b)]);
                        rt.fire(b, t, vec![rt.inst(a)]);
                    }
                }
            }
            if let Some(cp) = m(char_part) {
                for p in &char_began {
                    if let Some(p) = m(*p) {
                        rt.fire(p, "Touched", vec![rt.inst(cp)]);
                        rt.fire(cp, "Touched", vec![rt.inst(p)]);
                        crate::rig::fire_humanoid_touched(rt, cp, p);
                        // The character is simulated by its client.
                        if map.is_some() {
                            rt.fire(cp, "LocalSimulationTouched", vec![rt.inst(p)]);
                        }
                    }
                }
                for p in &char_ended {
                    if let Some(p) = m(*p) {
                        rt.fire(p, "TouchEnded", vec![rt.inst(cp)]);
                        rt.fire(p, "StoppedTouching", vec![rt.inst(cp)]);
                    }
                }
            }
        };
        let fire_humanoid = |rt: &Runtime, h: Option<Ref>, seat: Option<Ref>, client: bool| {
            let Some(h) = h else { return };
            if let Some((old, new)) = state_change {
                let item = |s: State| {
                    LEnumItem::from_name("HumanoidStateType", s.enum_name())
                        .and_then(|i| crate::ltypes::enum_value(&rt.lua, i).ok())
                        .unwrap_or(Value::Nil)
                };
                rt.fire(h, "StateChanged", vec![item(old), item(new)]);
                match new {
                    State::Jumping => rt.fire(h, "Jumping", vec![Value::Boolean(true)]),
                    State::Freefall => rt.fire(h, "FreeFalling", vec![Value::Boolean(true)]),
                    _ => {}
                }
                if old == State::Freefall {
                    rt.fire(h, "FreeFalling", vec![Value::Boolean(false)]);
                }
                // The (active) events of held states: true on entering,
                // false on leaving (Humanoid.FallingDown / GettingUp / Ragdoll).
                for (s, on) in [(old, false), (new, true)] {
                    if matches!(s, State::FallingDown | State::GettingUp | State::Ragdoll) {
                        rt.fire(h, s.enum_name(), vec![Value::Boolean(on)]);
                    }
                    // PlatformStanding reaches the client (its Animate) from
                    // the state; the side whose script set PlatformStand got it
                    // then (runtime_ext). Place C sets it on the server.
                    if client && s == State::PlatformStanding {
                        rt.fire(h, "PlatformStanding", vec![Value::Boolean(on)]);
                    }
                }
            }
            if let Some(c) = &seat_change {
                let part = if c.active {
                    seat.map(|s| rt.inst(s)).unwrap_or(Value::Nil)
                } else {
                    Value::Nil
                };
                rt.fire(h, "Seated", vec![Value::Boolean(c.active), part]);
            }
            if let Some((ev, sp)) = move_event {
                rt.fire(h, ev, vec![Value::Number(sp as f64)]);
            }
            if let Some(sp) = running_speed {
                rt.fire(h, "Running", vec![Value::Number(sp as f64)]);
            }
        };
        let seat_server = seat_change.as_ref().and_then(|c| c.seat);
        drop(sc);
        let sc = crate::bench::scope("p4_server_lua");
        let root = self.doc.dom.root_ref();
        let run_service = self.doc.find_child(root, "RunService");
        let humanoid_server = humanoid_ref.is_some().then_some(humanoid_ref);
        let server_out = self
            .with_side(Side::Server, |rt, _, _| {
                if let Some(mut w) = rt.world_mut() {
                    w.velocities = velocities.clone();
                    w.humanoid_states = state_names
                        .iter()
                        .map(|(k, v)| (*k, v.to_string()))
                        .collect();
                }
                fire_touches(rt, None);
                fire_humanoid(rt, humanoid_server, seat_server, false);
                if let Some(rs) = run_service {
                    // A server frame in Studio's order (Run mode probe):
                    // PreAnimation (then the Animator), Stepped,
                    // PreSimulation, PostSimulation, Heartbeat (then tweens).
                    rt.fire(rs, "PreAnimation", vec![Value::Number(dt as f64)]);
                    rt.fire(
                        rs,
                        "Stepped",
                        vec![Value::Number(time), Value::Number(dt as f64)],
                    );
                    rt.fire(rs, "PreSimulation", vec![Value::Number(dt as f64)]);
                    rt.fire(rs, "PostSimulation", vec![Value::Number(dt as f64)]);
                    rt.fire(rs, "Heartbeat", vec![Value::Number(dt as f64)]);
                }
                if let Some(mut w) = rt.world_mut() {
                    w.realtime = realtime;
                }
                rt.step(dt as f64);
                rt.world_mut()
                    .map(|mut w| {
                        (
                            std::mem::take(&mut w.died),
                            std::mem::take(&mut w.load_character),
                        )
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        self.game_requests(Side::Server);

        drop(sc);
        // 5. Replicate to the client, then client scripts.
        let sc = crate::bench::scope("p5_replicate");
        // The server's Animator track slots, then the frame's replication.
        self.with_side(Side::Server, |rt, _, _| crate::anim_replicate::publish(&rt.lua));
        if let Some(s) = self.play.as_mut() {
            s.map.begin_frame(dt as f64);
        }
        self.replicate();
        drop(sc);
        let sc = crate::bench::scope("p6_client_lua");
        if mode == RunMode::Play {
            let prep = crate::bench::scope("p6a_prep");
            // Server → client refs for just what this phase looks up (touches,
            // the character, humanoid states, moving parts): cloning the whole
            // map (Place B: 116k instances) was 0.75 ms a frame.
            let map_s2c: HashMap<Ref, Ref> = self
                .play
                .as_ref()
                .map(|s| {
                    let full = &s.map.s2c;
                    part_touches
                        .iter()
                        .flat_map(|(a, b, _)| [*a, *b])
                        .chain([char_part])
                        .chain(char_began.iter().copied())
                        .chain(char_ended.iter().copied())
                        .chain(humanoid_server)
                        .chain(seat_server)
                        .chain(velocities.keys().copied())
                        .chain(state_names.keys().copied())
                        .chain(s.character.as_ref().map(|c| c.rig.root))
                        .filter_map(|r| full.get(&r).map(|c| (r, *c)))
                        .collect()
                })
                .unwrap_or_default();
            let (crs, cuis) = match self.play.as_ref().and_then(|s| s.other_doc.as_ref()) {
                Some(cd) => {
                    let r = cd.dom.root_ref();
                    (
                        cd.find_child(r, "RunService"),
                        cd.find_child(r, "UserInputService"),
                    )
                }
                None => (None, None),
            };
            self.game_client_inputs();
            let humanoid_client = humanoid_server.and_then(|h| map_s2c.get(&h).copied());
            let client_states: HashMap<Ref, String> = state_names
                .iter()
                .filter_map(|(k, v)| map_s2c.get(k).map(|c| (*c, v.to_string())))
                .collect();
            let (gui_events, gui_layout, mouse_events, menu_events, menu_open) =
                match self.play.as_mut() {
                    Some(s) => (
                        std::mem::take(&mut s.game.gui_events),
                        std::mem::take(&mut s.game.gui_layout),
                        std::mem::take(&mut s.game.uis_mouse),
                        std::mem::take(&mut s.game.menu_events),
                        s.game.menu_open,
                    ),
                    None => Default::default(),
                };
            let gui_service = self
                .play
                .as_ref()
                .and_then(|s| s.other_doc.as_ref())
                .and_then(|cd| cd.find_child(cd.dom.root_ref(), "GuiService"));
            // Mouse over the game view: position (GUI coordinates, under the
            // top bar), motion and wheel since last frame.
            let (mouse_xy, mouse_delta, mouse_moved, wheel) = match self.play.as_mut() {
                Some(s) => {
                    let xy = s.game.last_pointer.map(|p| p.xy).unwrap_or([0.0, 0.0]);
                    (
                        xy,
                        std::mem::take(&mut s.mouse_delta),
                        std::mem::take(&mut s.mouse_moved),
                        std::mem::take(&mut s.wheel),
                    )
                }
                None => ([0.0, 0.0], glam::Vec2::ZERO, false, 0.0),
            };
            let (mx, my) = (
                mouse_xy[0] as f64,
                (mouse_xy[1] - mech::GUI_TOP_INSET) as f64,
            );
            // The OS window gained or lost focus since last frame.
            let focused_now = ctx.input(|i| i.focused);
            let window_focus_change = self.play.as_mut().and_then(|s| {
                (s.window_focused != focused_now).then(|| {
                    s.window_focused = focused_now;
                    focused_now
                })
            });
            // Gamepads plugged in or out (scanned about once a second):
            // UserInputService.GamepadConnected / GamepadDisconnected.
            let (pads_added, pads_removed, pads_now) = match self.play.as_mut() {
                Some(s) if s.time - s.gamepad_scan_at >= mech::GAMEPAD_SCAN_S as f64 => {
                    s.gamepad_scan_at = s.time;
                    let now = gamepad_devices();
                    let added: Vec<usize> = (s.gamepads.len()..now.len()).collect();
                    let removed: Vec<usize> = (now.len()..s.gamepads.len()).collect();
                    s.gamepads = now;
                    (added, removed, s.gamepads.len())
                }
                Some(s) => (Vec::new(), Vec::new(), s.gamepads.len()),
                None => (Vec::new(), Vec::new(), 0),
            };
            // Moving parts' velocities on the client, its own character's too.
            let client_velocities: HashMap<Ref, Vec3> = {
                let mut v: HashMap<Ref, Vec3> = velocities
                    .iter()
                    .filter_map(|(k, v)| map_s2c.get(k).map(|c| (*c, *v)))
                    .collect();
                if let Some((root, vel)) = self
                    .play
                    .as_ref()
                    .and_then(|s| s.character.as_ref())
                    .map(|c| (c.rig.root, c.velocity))
                {
                    if let Some(cr) = map_s2c.get(&root) {
                        v.insert(*cr, vel);
                    }
                }
                v
            };
            // What the client simulates itself (parts its scripts made or
            // cloned) reads the client world's motion, as in Studio.
            let client_own = self
                .play
                .as_ref()
                .map(|s| s.client_sim.own_motion(&s.map.c2s))
                .unwrap_or_default();
            let mut client_velocities = client_velocities;
            client_velocities.extend(client_own.iter().map(|(r, m)| (*r, m.0)));
            drop(prep);
            let lua_sc = crate::bench::scope("p6b_lua");
            let server_doc_id = self.doc.id;
            self.with_side(Side::Client, |rt, _, _| {
                if let Some(mut w) = rt.world_mut() {
                    w.velocities = client_velocities;
                    // Angular velocities / awake counts for the client's
                    // scripts (domains/world_part.rs reads them by document).
                    let _s = crate::bench::scope("p6b_mirror");
                    crate::physics::live::mirror(server_doc_id, w.d().id, &map_s2c);
                    crate::physics::live::overlay_spins(
                        w.d().id,
                        client_own.iter().map(|(r, m)| (*r, (m.1, m.2))),
                    );
                    drop(_s);
                    w.keys_down = keys.clone();
                    w.humanoid_states = client_states;
                    if !gui_layout.is_empty() {
                        w.input.gui_layout = gui_layout;
                    }
                    w.input.menu_open = menu_open;
                }
                // AbsoluteSize / AbsolutePosition / ViewportSize changes from
                // the new layout (a resized window refits screen-sized GUIs).
                {
                    let _s = crate::bench::scope("p6b_layout_changes");
                    let _ = crate::runtime::ext::fire_layout_changes(&rt.lua);
                }
                // GuiService.MenuOpened / MenuClosed (the Esc menu).
                if let Some(gs) = gui_service {
                    for open in &menu_events {
                        rt.fire(gs, if *open { "MenuOpened" } else { "MenuClosed" }, vec![]);
                    }
                }
                // Input reaches ContextActionService's bound actions, then
                // UserInputService (InputBegan / InputChanged / InputEnded,
                // PointerAction for the wheel), one InputObject per device.
                let ev = |state: &str,
                          uit: &str,
                          key: &str,
                          pos: (f64, f64, f64),
                          delta: (f64, f64),
                          gpe: bool| {
                    let s = |x: &str| Value::String(rt.lua.create_string(x).expect("lua string"));
                    rt.spawn_global(
                        "__rs_input_event",
                        vec![
                            s(state),
                            s(uit),
                            s(key),
                            Value::Number(pos.0),
                            Value::Number(pos.1),
                            Value::Number(pos.2),
                            Value::Number(delta.0),
                            Value::Number(delta.1),
                            Value::Boolean(gpe),
                        ],
                    );
                };
                rt.spawn_global("__rs_input_frame_end", vec![]);
                // A press reaches UserInputService.InputBegan before the GUI's
                // MouseButton1Down / Up / Click / Activated, even when press and
                // release land in the same frame (the harness's --click).
                let mouse_buttons = |begin: bool| {
                    for (button, down, processed) in &mouse_events {
                        if *down != begin {
                            continue;
                        }
                        let uit = if *button == 1 { "MouseButton1" } else { "MouseButton2" };
                        ev(
                            if *down { "Begin" } else { "End" },
                            uit,
                            "Unknown",
                            (mx, my, 0.0),
                            (0.0, 0.0),
                            *processed,
                        );
                    }
                };
                mouse_buttons(true);
                game::fire_gui_events(rt, gui_events, gui_service);
                if let Some(on) = window_focus_change {
                    rt.spawn_global("__rs_window_focus", vec![Value::Boolean(on)]);
                }
                if mouse_moved {
                    ev(
                        "Change",
                        "MouseMovement",
                        "Unknown",
                        (mx, my, 0.0),
                        (mouse_delta.x as f64, mouse_delta.y as f64),
                        false,
                    );
                }
                if wheel != 0.0 {
                    ev(
                        "Change",
                        "MouseWheel",
                        "Unknown",
                        (mx, my, wheel as f64),
                        (0.0, 0.0),
                        false,
                    );
                }
                // Button releases after the GUI's events (Up / Click / Activated).
                mouse_buttons(false);
                fire_touches(rt, Some(&map_s2c));
                fire_humanoid(rt, humanoid_client, seat_server.and_then(|s| map_s2c.get(&s).copied()), true);
                if let Some(mut w) = rt.world_mut() {
                    w.input.gamepads = pads_now;
                }
                if let Some(uis) = cuis {
                    let pad = |i: usize| {
                        LEnumItem::from_name("UserInputType", &format!("Gamepad{}", i + 1))
                            .and_then(|e| crate::ltypes::enum_value(&rt.lua, e).ok())
                            .unwrap_or(Value::Nil)
                    };
                    for i in &pads_removed {
                        rt.fire(uis, "GamepadDisconnected", vec![pad(*i)]);
                    }
                    for i in &pads_added {
                        rt.fire(uis, "GamepadConnected", vec![pad(*i)]);
                    }
                }
                for (list, state) in [(&began, "Begin"), (&ended, "End")] {
                    for (k, processed) in list {
                        ev(state, "Keyboard", k, (0.0, 0.0, 0.0), (0.0, 0.0), *processed);
                    }
                }
                if let Some(rs) = crs {
                    // A client frame in Roblox's order: PreRender (RenderStepped),
                    // PreAnimation, PreSimulation (Stepped), PostSimulation (Heartbeat).
                    rt.fire(rs, "RenderStepped", vec![Value::Number(dt as f64)]);
                    rt.fire(rs, "PreRender", vec![Value::Number(dt as f64)]);
                    rt.fire(rs, "PreAnimation", vec![Value::Number(dt as f64)]);
                    rt.fire(
                        rs,
                        "Stepped",
                        vec![Value::Number(time), Value::Number(dt as f64)],
                    );
                    rt.fire(rs, "PreSimulation", vec![Value::Number(dt as f64)]);
                    rt.fire(rs, "PostSimulation", vec![Value::Number(dt as f64)]);
                    rt.fire(rs, "Heartbeat", vec![Value::Number(dt as f64)]);
                }
                if let Some(mut w) = rt.world_mut() {
                    w.realtime = realtime;
                }
                let _s = crate::bench::scope("p6b_rt_step");
                rt.step(dt as f64);
            });
            drop(lua_sc);
            let req = crate::bench::scope("p6c_requests");
            self.game_requests(Side::Client);
            self.read_client_input_state();
            self.deliver_remotes();
            drop(req);
            // The client's own assemblies (its simulation step in Studio).
            let _cj = crate::bench::scope("p6d_client_joints");
            if let Some(s) = self.play.as_mut() {
                if let Some(cd) = s.other_doc.as_mut() {
                    s.client_joints.solve(cd, &s.map.c2s);
                }
            }
            drop(_cj);
            let _cp = crate::bench::scope("p6e_client_physics");
            if let Some(s) = self.play.as_mut() {
                // Velocities client scripts gave parts (the character's own
                // stay for client_root_movers).
                let own: HashSet<Ref> =
                    s.physics.skip.iter().filter_map(|r| s.map.s2c.get(r).copied()).collect();
                let vel: Vec<(Ref, glam::Vec3)> = match s.client.as_mut().and_then(|c| c.world.as_mut()) {
                    Some(mut w) => {
                        let (keep, take) = std::mem::take(&mut w.velocity_requests)
                            .into_iter()
                            .partition(|(r, _)| own.contains(r));
                        w.velocity_requests = keep;
                        take
                    }
                    None => Vec::new(),
                };
                if let Some(cd) = s.other_doc.as_mut() {
                    let out = s.client_sim.step(&s.physics, cd, &s.map.s2c, &s.map.c2s, &vel, dt);
                    for r in out.fallen {
                        cd.dom.transfer_within(r, Ref::none());
                        cd.mark_tree(r);
                    }
                    // Its step published the client world's stats under the
                    // client's document: the server's mirror goes back under
                    // them for scripts that run before the next frame's.
                    if s.client_sim.has_bodies() {
                        crate::physics::live::mirror(server_doc_id, cd.id, &s.map.s2c);
                        let own = s.client_sim.own_motion(&s.map.c2s);
                        crate::physics::live::overlay_spins(cd.id, own.into_iter().map(|(r, m)| (r, (m.1, m.2))));
                    }
                }
            }
        }
        drop(sc);

        self.collect_garbage(dt);

        // 6. Deaths and respawns.
        let (died, reload) = server_out;
        let my_humanoid = self
            .play
            .as_ref()
            .and_then(|s| s.character.as_ref())
            .map(|c| c.humanoid);
        if my_humanoid.is_some_and(|h| died.contains(&h)) {
            if let Some(s) = self.play.as_mut() {
                s.state_log.push("Dead".into());
            }
            self.kill_character();
        }
        let due = self
            .play
            .as_ref()
            .is_some_and(|s| s.respawn_at.is_some_and(|t| s.time >= t));
        if due || !reload.is_empty() {
            self.respawn();
        }
        self.replicate();

        self.game_tick_end();

        // 7. The view is the client's Workspace.CurrentCamera, whatever its
        // CameraType: Roblox's CameraModule (or the place's own camera
        // script) moves it. Looking at the server side frees the editor camera.
        let client_cam = (mode == RunMode::Play
            && self
                .play
                .as_ref()
                .is_some_and(|s| s.showing == Side::Client))
        .then(|| self.play.as_ref().and_then(|s| s.other_doc.as_ref()))
        .flatten()
        .and_then(|cd| {
            let ws = cd.workspace()?;
            let cam = match cd.prop(ws, "CurrentCamera") {
                Some(Variant::Ref(r)) if cd.exists(r) => r,
                _ => cd.find_child(ws, "Camera")?,
            };
            let xf = crate::scene::xform_of(cd, cam)?;
            let fov = f32_of(cd.prop(cam, "FieldOfView")).unwrap_or(mech::FOV_DEG);
            // ProjectionType Orthographic (domains/world_camera.rs).
            let ortho = (crate::scene::enum_u32(cd.prop(cam, "ProjectionType")) == Some(1)).then(|| {
                (
                    f32_of(cd.prop(cam, "OrthographicSize")).unwrap_or(300.0),
                    crate::scene::enum_u32(cd.prop(cam, "OrthographicMode")).unwrap_or(0),
                )
            });
            let stretch = f32_of(cd.prop(cam, "ViewStretch")).unwrap_or(1.0);
            Some((xf, fov, ortho, stretch))
        });
        if let Some((xf, fov, ortho, stretch)) = client_cam {
            self.cam.ortho = ortho;
            self.cam.stretch = stretch;
            let look = -xf.rot.z_axis;
            self.cam.pos = xf.pos;
            self.cam.set_rotation(xf.rot);
            self.cam.focus = xf.pos + look * 10.0;
            self.cam.fov_deg = fov;
            self.cam.mode = CameraMode::Fly;
        } else {
            self.cam.ortho = None;
            self.cam.stretch = 1.0;
        }
        self.shown_front();
    }

    /// Runs a Luau chunk on one side of the Play session (test harness);
    /// its prints go to Output like a script's. Returns an error message.
    pub fn run_luau(&mut self, side: Side, code: &str) -> Option<String> {
        self.run_luau_as(side, code, crate::runtime::security::COMMAND)
    }

    /// `run_luau` at identity `id` (the in-game menu runs as a CoreScript).
    pub fn run_luau_as(&mut self, side: Side, code: &str, id: u8) -> Option<String> {
        self.server_front();
        let r = self.with_side(side, |rt, _, _| {
            // The given identity, for this chunk and what it spawns.
            let out = crate::runtime::security::with_main_identity(&rt.lua, id, || {
                // Scripts' _G and shared, with every host global.
                let env = crate::globals_parity::harness_env(&rt.lua)?;
                rt.lua.load(code).set_name("=harness").set_environment(env).exec()
            })
            .err()
            .map(|e| crate::runtime::describe_error(&e));
            rt.dispatch();
            out
        });
        self.shown_front();
        r.flatten()
    }

    /// The other side's DataModel while playing (the one not shown).
    pub fn play_other_doc(&self) -> Option<&Document> {
        self.play.as_ref()?.other_doc.as_ref()
    }

    /// Status text for the toolbar / status bar.
    pub fn play_status(&self) -> Option<String> {
        let s = self.play.as_ref()?;
        let what = if s.mode == RunMode::Play {
            "Playing"
        } else {
            "Running"
        };
        let ch = match &s.character {
            Some(c) if c.dead => " · respawning".to_string(),
            Some(c) => {
                let h = if s.showing == Side::Server {
                    self.humanoid_f32(c.humanoid, "Health", 100.0)
                } else {
                    100.0
                };
                format!(" · {} · health {h:.0}", c.state.enum_name())
            }
            None => String::new(),
        };
        let side = if s.client.is_some() {
            format!(" · viewing {}", s.showing.label())
        } else {
            String::new()
        };
        Some(format!("{what} {:.1} s{ch}{side}", s.time))
    }
}

/// Gamepads plugged in: the kernel's joystick devices
/// (/dev/input/by-id/*-event-joystick), sorted so slots stay stable.
fn gamepad_devices() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/dev/input/by-id")
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.ends_with("-event-joystick"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.truncate(8); // Enum.UserInputType.Gamepad1..Gamepad8
    v
}

fn script_enabled(d: &Document, r: Ref) -> bool {
    !matches!(d.prop(r, "Disabled"), Some(Variant::Bool(true)))
        && !matches!(d.prop(r, "Enabled"), Some(Variant::Bool(false)))
}

/// LocalScripts (and RunContext=Client Scripts) under the given client instances.
fn client_scripts(cd: &Document, under: &[Option<Ref>]) -> Vec<Ref> {
    let mut out = Vec::new();
    for u in under.iter().flatten() {
        for i in cd.dom.descendants_of(*u) {
            let client_script = i.class == "Script"
                && crate::scene::enum_u32(cd.prop(i.referent(), "RunContext")) == Some(2);
            if (i.class == "LocalScript" || client_script) && script_enabled(cd, i.referent()) {
                out.push(i.referent());
            }
        }
    }
    out
}

/// The character's pose: capsule → HumanoidRootPart → Motor6D chain, each
/// joint turned by its Motor6D.Transform.
fn character_pose(ch: &Character, transforms: &HashMap<Ref, Xform>) -> Vec<(Ref, Xform)> {
    let rot = Mat3::from_rotation_y(ch.yaw) * Mat3::from_rotation_x(ch.pitch);
    let root = Xform {
        pos: ch.body.pos + Vec3::Y * ch.root_lift,
        rot,
    };
    ch.rig.solve(root, transforms)
}

/// The Humanoid controller's view of a character (round 22): its hip
/// (R6: the 2-stud legs), root size and the box its torso, head and
/// root fill in the root's frame, from the root's bottom up (Studio's limbs
/// don't collide; anything under the root's bottom is the floor rays' job).
fn hum_shape(doc: &Document, model: Ref, root_lift: f32) -> crate::physics::HumShape {
    let root = doc.find_named(model, "HumanoidRootPart");
    let root_xf = root.and_then(|r| crate::scene::xform_of(doc, r));
    let size = root.and_then(|r| crate::scene::vec3_of(doc.prop(r, "Size"))).unwrap_or(Vec3::new(2.0, 2.0, 1.0));
    let hip = rig::root_height(doc, model) - size.y * 0.5;
    let mut lo = -size * 0.5;
    let mut hi = size * 0.5;
    for inst in doc.dom.descendants_of(model) {
        if !doc::is_a(&inst.class, "BasePart") {
            continue;
        }
        let r = inst.referent();
        let s = crate::scene::vec3_of(doc.prop(r, "Size")).unwrap_or(Vec3::ONE);
        if !matches!(inst.name.as_str(), "UpperTorso" | "LowerTorso" | "Torso" | "Head") {
            continue;
        }
        let (Some(rx), Some(px)) = (root_xf, crate::scene::xform_of(doc, r)) else { continue };
        let inv = rx.rot.transpose();
        for i in 0..8 {
            let c = Vec3::new(
                if i & 1 == 0 { -s.x } else { s.x },
                if i & 2 == 0 { -s.y } else { s.y },
                if i & 4 == 0 { -s.z } else { s.z },
            ) * 0.5;
            let w = px.pos + px.rot * c;
            let l = inv * (w - rx.pos);
            lo = lo.min(l);
            hi = hi.max(l);
        }
    }
    // From the root's bottom up, whatever hangs lower.
    lo.y = -size.y * 0.5;
    let mut mass = 0.0f32;
    let mut parts = 0u32;
    for inst in doc.dom.descendants_of(model) {
        if doc::is_a(&inst.class, "BasePart") {
            parts += 1;
            if let Some(s) = crate::scene::vec3_of(doc.prop(inst.referent(), "Size")) {
                mass += s.x * s.y * s.z * mech::PART_DENSITY;
            }
        }
    }
    let r6 = doc.find_named(model, "UpperTorso").is_none() && doc.find_named(model, "Torso").is_some();
    crate::physics::HumShape { hip, root_size: size, box_lo: lo, box_hi: hi, root_lift, mass, parts, r6 }
}

/// Whether a character part collides on Studio's spawned character.
fn studio_collides(name: &str, rig: RigType) -> bool {
    match rig {
        RigType::R15 => matches!(name, "HumanoidRootPart" | "UpperTorso" | "LowerTorso" | "Head"),
        RigType::R6 => matches!(name, "Torso" | "Head"),
    }
}
