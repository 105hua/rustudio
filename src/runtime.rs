//! The Luau runtime: runs Script / LocalScript / ModuleScript sources against
//! the play DataModel. Behaviour follows Roblox's public engine reference
//! (create.roblox.com/docs/reference/engine); estimates are listed in
//! docs/MECHANICS.md.
//!
//! The play `Document` lives inside [`World`] (Lua app data) while Lua runs
//! and is handed back to the editor between frames. Scripts run as Luau
//! coroutines; `task.wait`, `Signal:Wait` and `WaitForChild` yield the running
//! coroutine and the scheduler here resumes it later.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use glam::Vec3;
use mlua::thread::ThreadStatus;
use mlua::{
    AnyUserData, FromLua, Function, Lua, MetaMethod, MultiValue, Table, Thread, UserData,
    UserDataFields, UserDataMethods, Value, Variadic,
};
use rbx_dom_weak::types::{
    Attributes, Color3, Color3uint8, Content, ContentId, Enum, Ref, UDim, UDim2, Variant,
    VariantType, Vector2, Vector3,
};
use rbx_dom_weak::{ustr, InstanceBuilder};
use rbx_reflection::{DataType, PropertyKind, PropertyTag, Scriptability};

use crate::app::Level;
use crate::doc::{self, Document};
use crate::ltypes::*;
use crate::math::Xform;
use crate::mech;

#[path = "runtime_signals.rs"]
pub mod signal_timing;

#[path = "runtime_ext.rs"]
pub mod ext;
#[path = "stubs.rs"]
pub mod stubs;
#[path = "editable.rs"]
pub mod editable;
#[path = "domains/mod.rs"]
pub mod domains;
#[path = "security.rs"]
pub mod security;
#[path = "runtime_routes.rs"]
mod routes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// F5: player, character, LocalScripts.
    Play,
    /// F8: server scripts and physics only.
    Run,
}

/// Which DataModel a runtime drives in a Play session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Server,
    Client,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::Server => "Server",
            Side::Client => "Client",
        }
    }
}

/// A server-side change the client copy must receive.
#[derive(Debug, Clone, PartialEq)]
pub enum RepOp {
    Prop(Ref, String),
    Name(Ref),
    Parent(Ref),
    /// Physics moved a part: its CFrame is copied without Changed events
    /// (Roblox doesn't fire Changed for physics movement).
    Moved(Ref),
    /// Instance:Destroy() of the instance and its subtree, sent once (Studio's
    /// DestroyInstance item); the client destroys its copy.
    Destroy(Ref),
}

/// A Lua value that can cross between the server and client VMs (functions
/// and coroutines can't, as in Roblox).
#[derive(Debug, Clone)]
pub enum Xfer {
    Nil,
    Bool(bool),
    Num(f64),
    Str(Vec<u8>),
    Table(Vec<(Xfer, Xfer)>),
    Inst(Ref),
    V3(Vec3),
    V2(glam::Vec2),
    CF(Xform),
    C3([f32; 3]),
    Brick(rbx_dom_weak::types::BrickColor),
    UDim(LUDim),
    UDim2(LUDim2),
    EnumItem(LEnumItem),
    /// A Luau buffer (copied, as Roblox copies buffers across remotes).
    Buffer(Vec<u8>),
}

#[derive(Debug, Clone)]
pub enum RemoteKind {
    /// FireServer (→ OnServerEvent) or FireClient/FireAllClients (→ OnClientEvent).
    Event,
    /// InvokeServer with a reply id.
    Invoke(u64),
    /// The server's answer to an invoke: (id, ok).
    Reply(u64, bool),
}

/// A remote call waiting to be delivered to the other side.
#[derive(Debug, Clone)]
pub struct RemoteMsg {
    /// The remote instance, as a ref in the sender's DataModel.
    pub inst: Ref,
    pub kind: RemoteKind,
    pub args: Vec<Xfer>,
}

/// Everything Lua callbacks touch, stored as Lua app data.
pub struct World {
    pub doc: Option<Document>,
    pub mode: RunMode,
    pub side: Side,
    /// True when a separate client exists (Play); remotes are delivered.
    pub has_peer: bool,
    /// Server changes waiting to replicate to the client.
    pub rep: Vec<RepOp>,
    /// Remote calls waiting to cross to the other side.
    pub outbox: Vec<RemoteMsg>,
    pub time: f64,
    /// Waits and delays also come due by wall time, as Roblox's scheduler
    /// counts them: Roblox caps the game's frame step at 1/15 s (time(),
    /// animations) but not waits (Studio 0.746 at 4 fps: delay(1) fired at
    /// time() +0.27). Play sets it; off for fixed-step (deterministic) runs.
    pub realtime: bool,
    /// Signal fires waiting to be dispatched: (instance, event, args, last
    /// connection id at fire time, deferred re-entrancy depth). Handlers
    /// connected later don't get it.
    events: Vec<(Ref, String, Vec<Value>, u64, u32)>,
    /// Fires whose handlers run before the triggering engine call returns
    /// (Workspace.SignalBehavior Immediate): see `signal_timing`.
    now: Vec<(Ref, String, Vec<Value>, u64)>,
    /// The place's Workspace.SignalBehavior, read when the world is made.
    pub signal_mode: signal_timing::SignalMode,
    /// Re-entrancy depth fires made now get (set while a deferred event's
    /// handlers run).
    defer_depth: u32,
    /// Id of the newest connection / waiter.
    conn_seq: u64,
    /// Parts whose physics-relevant properties changed.
    pub dirty: HashSet<Ref>,
    /// Instances were reparented, created or destroyed.
    pub structure_changed: bool,
    pub destroyed: HashSet<Ref>,
    pub output: Vec<(Level, String)>,
    /// Velocities read from physics (AssemblyLinearVelocity).
    pub velocities: HashMap<Ref, Vec3>,
    /// Velocities scripts asked for.
    pub velocity_requests: Vec<(Ref, Vec3)>,
    /// Humanoids that died this frame.
    pub died: Vec<Ref>,
    /// Players that asked for a new character (Player:LoadCharacter).
    pub load_character: Vec<Ref>,
    /// KeyCode names held down (UserInputService:IsKeyDown).
    pub keys_down: HashSet<String>,
    /// Enum.CoreGuiType names turned off by StarterGui:SetCoreGuiEnabled.
    pub core_gui_off: HashSet<String>,
    /// Current Enum.HumanoidStateType name per Humanoid (Humanoid:GetState).
    pub humanoid_states: HashMap<Ref, String>,
    /// Humanoid:SetStateEnabled(state, false) per Humanoid (state names).
    pub states_disabled: HashMap<Ref, HashSet<String>>,
    /// PhysicsService collision groups: one table per Play session, shared
    /// by the server and client runtimes and the physics (replicated in Roblox).
    pub groups: crate::collision_groups::Shared,
    /// The signed-in person's avatar (Players:GetHumanoidDescriptionFromUserId
    /// for their user id, as a signed-in Studio answers), and the
    /// HumanoidDescriptions made from it (CreateHumanoidModelFromDescription
    /// dresses those models with its clothing, face and accessories).
    pub avatar: Option<crate::avatar::AvatarLook>,
    pub avatar_descs: HashSet<Ref>,
    /// Instances with at least one connection or waiter (fires on others are dropped early).
    pub listening: rustc_hash::FxHashSet<Ref>,
    /// Native class filters on game.DescendantAdded / DescendantRemoving
    /// for engine domains (`class_signals`): (classes, class -> matches).
    pub class_watches: Vec<(Vec<String>, HashMap<rbx_dom_weak::Ustr, bool>)>,
    /// Engine requests scripts made this frame, handled by the play session.
    pub requests: ext::Requests,
    /// What the play session tells scripts about the mouse, sounds, etc.
    pub input: ext::Inputs,
    /// Scripts that have run on this side (each starts once).
    pub started: HashSet<Ref>,
    /// Unparented instances the collector (`Runtime::collect`) looks at:
    /// freed with their subtree once no Lua value of any of them is left.
    pub orphans: HashSet<Ref>,
    /// Instances freed so far (`collect`), for RUSTUDIO_MEMSTATS.
    pub freed: u64,
    /// Scripts parented (or enabled) where Roblox runs them, started at the next dispatch.
    pub to_start: Vec<Ref>,
    /// Terrain edited by scripts this frame (Terrain:Fill…): its voxels,
    /// written back to SmoothGrid once per step (runtime_ext::flush_terrain).
    pub terrain_edit: Option<(Ref, crate::terrain::Voxels)>,
    /// GuiBase2d AbsolutePosition / AbsoluteSize / AbsoluteRotation as
    /// scripts last saw them (read, or told by a change), and the Camera's
    /// ViewportSize: what `ext::fire_layout_changes` compares the new layout
    /// with.
    pub abs_seen: HashMap<Ref, ext::AbsBox>,
    pub viewport_seen: Option<[f32; 2]>,
    /// Instances that joined `listening` since `ext::fire_layout_changes`
    /// last ran, and the listened GUI objects that were drawn then.
    pub listen_new: Vec<Ref>,
    pub abs_laid: HashSet<Ref>,
    /// The collector (`Runtime::collect`): how many unparented instances
    /// were left watched after its last pass, and whether that pass saw new
    /// ones or freed any (`Runtime::gc_step` works faster then).
    pub orphans_seen: usize,
    pub gc_busy: bool,
}

impl World {
    /// Whether a script at its current place runs on this side, as Roblox
    /// decides: Scripts (Legacy / Server) under Workspace or
    /// ServerScriptService on the server; LocalScripts under the player's
    /// PlayerGui, Backpack, PlayerScripts, Character or ReplicatedFirst, and
    /// RunContext Client Scripts anywhere, on the client.
    pub fn runs_here(&self, r: Ref) -> bool {
        let d = self.d();
        let Some(i) = d.dom.get_by_ref(r) else {
            return false;
        };
        let enabled = !matches!(d.prop(r, "Disabled"), Some(Variant::Bool(true)))
            && !matches!(d.prop(r, "Enabled"), Some(Variant::Bool(false)));
        if !enabled || self.mode != RunMode::Play && self.side == Side::Client {
            return false;
        }
        let ctx = crate::scene::enum_u32(d.prop(r, "RunContext")).unwrap_or(0);
        let root = d.dom.root_ref();
        let under = |class: &str| {
            d.find_child(root, class)
                .is_some_and(|s| d.is_ancestor(s, r))
        };
        match (self.side, i.class.as_str()) {
            // Legacy Scripts also run in a player's Backpack and PlayerGui
            // (tools; StarterGui's server Scripts, as in Studio).
            (Side::Server, "Script") => {
                ctx != 2
                    && (under("Workspace")
                        || under("ServerScriptService")
                        || d.find_child(root, "Players").is_some_and(|ps| {
                            d.dom.get_by_ref(ps).is_some_and(|pi| {
                                pi.children().iter().any(|p| {
                                    ["Backpack", "PlayerGui"].iter().any(|c| {
                                        d.find_child(*p, c).is_some_and(|f| d.is_ancestor(f, r))
                                    })
                                })
                            })
                        }))
            }
            (Side::Client, "Script") => {
                ctx == 2 && !under("ServerScriptService") && !under("ServerStorage")
            }
            (Side::Client, "LocalScript") => {
                let lp =
                    d.find_child(root, "Players")
                        .and_then(|p| match d.prop(p, "LocalPlayer") {
                            Some(Variant::Ref(lp)) if d.exists(lp) => Some(lp),
                            _ => None,
                        });
                let in_player = lp.is_some_and(|lp| {
                    ["PlayerGui", "Backpack", "PlayerScripts"]
                        .iter()
                        .any(|c| d.find_child(lp, c).is_some_and(|f| d.is_ancestor(f, r)))
                });
                let in_character = lp
                    .and_then(|lp| match d.prop(lp, "Character") {
                        Some(Variant::Ref(c)) if d.exists(c) => Some(c),
                        _ => None,
                    })
                    .is_some_and(|c| d.is_ancestor(c, r));
                in_player || in_character || under("ReplicatedFirst")
            }
            _ => false,
        }
    }

    /// Queues scripts at and under `r` that now run here.
    fn queue_scripts(&mut self, r: Ref) {
        let refs: Vec<Ref> = std::iter::once(r)
            .chain(self.d().dom.descendants_of(r).map(|i| i.referent()))
            .collect();
        for s in refs {
            if !self.started.contains(&s)
                && self
                    .d()
                    .class_of(s)
                    .is_some_and(|c| c == "Script" || c == "LocalScript")
                && self.runs_here(s)
            {
                self.to_start.push(s);
            }
        }
    }
}

struct Timer {
    at: f64,
    started: f64,
    /// Wall-clock start: a wait returns the larger of the simulated and the
    /// real elapsed time, so it agrees with os.clock across frame hitches
    /// (Play caps dt at mech::MAX_FRAME_DT, so time() lags the wall clock then).
    wall: Instant,
    thread: Thread,
    args: Option<Vec<Value>>,
}

#[derive(Default)]
struct Sched {
    timers: Vec<Timer>,
    deferred: VecDeque<(Thread, Vec<Value>)>,
}

struct Conn {
    id: u64,
    func: Function,
    once: bool,
    /// The connecting thread's identity: handlers run at it (security.rs).
    ident: u8,
}

#[derive(Default)]
struct Signals {
    conns: HashMap<(Ref, String), Vec<Conn>>,
    waiters: HashMap<(Ref, String), Vec<(u64, Thread)>>,
    next_id: u64,
    /// The signal names each instance has (or had) connections under in
    /// `conns`, so Destroy drops an instance's connections without a walk
    /// of every connection there is.
    conn_names: HashMap<Ref, Vec<String>>,
}

#[derive(Default)]
struct Cache {
    /// Instance → its slot in `weak`. The userdata lives in `weak`, a Lua
    /// table with weak values, so an instance's Lua value is collected once
    /// no script can reach it (a strong map here kept every instance ever
    /// touched alive: the power-up weapon's tracers grew RSS without bound).
    slots: rustc_hash::FxHashMap<Ref, i64>,
    next_slot: i64,
    weak: Option<Table>,
    /// Instances given a new Lua value since the last `collect`.
    fresh: Vec<Ref>,
    /// "slot:event" → the event's state table, weak-valued (see `signal`).
    sigs: Option<Table>,
    /// The prelude's Signal methods (every event object's).
    sig_methods: Option<Table>,
    /// Method functions by descriptor (base class, name), as Studio's
    /// bridge caches them: `Part.IsA == game.IsA`.
    methods: HashMap<(&'static str, &'static str), Function>,
    /// Rust methods that run as Lua instead (media's *Async ones yield a
    /// frame after answering): name → the Lua function, called with self.
    pub overrides: HashMap<&'static str, Function>,
    callbacks: HashMap<(Ref, String), Function>,
}

struct Budget {
    start: Option<Instant>,
}

/// Runs blocking work (a download a Roblox call would yield for, e.g.
/// InsertService:LoadAsset) outside the running script's timeout budget.
pub(crate) fn outside_budget<T>(lua: &Lua, f: impl FnOnce() -> T) -> T {
    let t0 = Instant::now();
    let r = f();
    if let Some(mut b) = lua.app_data_mut::<Budget>() {
        if let Some(start) = b.start.as_mut() {
            *start += t0.elapsed();
        }
    }
    r
}

pub struct Runtime {
    pub lua: Lua,
    shared: Table,
    /// This VM's lua_Callbacks (the watchdog arms its interrupt).
    callbacks: usize,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        interrupt_watchdog::unregister(self.callbacks);
    }
}

/// Luau calls the interrupt callback at every call and loop edge while one is
/// set. The timeout check needs it only now and then, so the callback stays
/// unset and a watchdog thread sets it every WATCHDOG_MS; the callback unsets
/// itself when it runs. (Before, mlua's interrupt wrapper ran on every edge:
/// a large share of tight Luau loops.) Luau supports setting it from another
/// thread; Roblox arms its script timeout the same way.
mod interrupt_watchdog {
    use std::sync::Mutex;
    use mlua::ffi;
    type Proc = unsafe extern "C-unwind" fn(*mut ffi::lua_State, std::ffi::c_int);
    const WATCHDOG_MS: u64 = 100;
    static VMS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());
    static STARTED: std::sync::Once = std::sync::Once::new();

    /// Takes the installed interrupt off `cb` and arms it periodically.
    pub fn register(cb: *mut ffi::lua_Callbacks) {
        let proc = unsafe { (*cb).interrupt.take() };
        let Some(proc) = proc else { return };
        VMS.lock().unwrap().push((cb as usize, proc as usize));
        STARTED.call_once(|| {
            std::thread::Builder::new()
                .name("lua-watchdog".into())
                .spawn(|| loop {
                    std::thread::sleep(std::time::Duration::from_millis(WATCHDOG_MS));
                    for &(cb, proc) in VMS.lock().unwrap().iter() {
                        let cb = cb as *mut ffi::lua_Callbacks;
                        // SAFETY: registered VMs are alive until unregister,
                        // which takes this lock first.
                        unsafe {
                            std::ptr::write_volatile(
                                &mut (*cb).interrupt,
                                Some(std::mem::transmute::<usize, Proc>(proc)),
                            )
                        };
                    }
                })
                .ok();
        });
    }

    pub fn disarm(cb: usize) {
        if cb != 0 {
            let cb = cb as *mut ffi::lua_Callbacks;
            unsafe { std::ptr::write_volatile(&mut (*cb).interrupt, None) };
        }
    }

    pub fn unregister(cb: usize) {
        VMS.lock().unwrap().retain(|e| e.0 != cb);
    }
}

// ------------------------------------------------------------- utilities

pub(crate) fn world(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, World>> {
    lua.app_data_mut::<World>()
        .ok_or_else(|| rt_err("the play session has ended"))
}

fn cache(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, Cache>> {
    lua.app_data_mut::<Cache>()
        .ok_or_else(|| rt_err("runtime cache missing"))
}

fn sched(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, Sched>> {
    lua.app_data_mut::<Sched>()
        .ok_or_else(|| rt_err("scheduler missing"))
}

fn signals(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, Signals>> {
    lua.app_data_mut::<Signals>()
        .ok_or_else(|| rt_err("signals missing"))
}

impl World {
    pub fn new(doc: Document, mode: RunMode) -> Self {
        World {
            signal_mode: signal_timing::SignalMode::of(&doc),
            doc: Some(doc),
            mode,
            side: Side::Server,
            has_peer: false,
            rep: Vec::new(),
            outbox: Vec::new(),
            time: 0.0,
            realtime: false,
            events: Vec::new(),
            now: Vec::new(),
            defer_depth: 0,
            conn_seq: 0,
            dirty: HashSet::new(),
            structure_changed: false,
            destroyed: HashSet::new(),
            output: Vec::new(),
            velocities: HashMap::new(),
            velocity_requests: Vec::new(),
            died: Vec::new(),
            load_character: Vec::new(),
            keys_down: HashSet::new(),
            core_gui_off: HashSet::new(),
            humanoid_states: HashMap::new(),
            states_disabled: HashMap::new(),
            groups: crate::collision_groups::shared(),
            avatar: None,
            avatar_descs: HashSet::new(),
            listening: Default::default(),
            class_watches: Vec::new(),
            abs_seen: HashMap::new(),
            viewport_seen: None,
            listen_new: Vec::new(),
            abs_laid: HashSet::new(),
            orphans_seen: 0,
            gc_busy: true,
            requests: Default::default(),
            input: Default::default(),
            started: HashSet::new(),
            orphans: HashSet::new(),
            freed: 0,
            to_start: Vec::new(),
            terrain_edit: None,
        }
    }

    pub fn d(&self) -> &Document {
        self.doc
            .as_ref()
            .expect("play document present while Lua runs")
    }

    pub fn dm(&mut self) -> &mut Document {
        self.doc
            .as_mut()
            .expect("play document present while Lua runs")
    }

    /// Notes a server change for replication (clients don't replicate back).
    pub fn record(&mut self, op: RepOp) {
        if self.side == Side::Server && self.has_peer {
            self.rep.push(op);
        }
    }

    pub fn fire(&mut self, r: Ref, name: &str, args: Vec<Value>) {
        crate::telemetry::fired(name);
        if name == "Changed" || name.starts_with("Tag") {
            domains::services::on_fire(self, r, name, &args);
        }
        // Nobody connected to anything on this instance: nothing to queue.
        if !self.listening.contains(&r) {
            return;
        }
        if self.signal_mode.defers(name, &args) {
            self.events.push((r, name.to_string(), args, self.conn_seq, self.defer_depth));
        } else {
            self.now.push((r, name.to_string(), args, self.conn_seq));
        }
    }

    /// Queues a native frame hook right behind the frame event `after`, in
    /// the same queue (immediate or deferred) whether or not anything
    /// listens; `signal_timing` runs it at that point.
    pub fn queue_hook(&mut self, r: Ref, after: &str, hook: &str, dt: Value) {
        if self.signal_mode.defers(after, &[]) {
            self.events.push((r, hook.to_string(), vec![dt], self.conn_seq, self.defer_depth));
        } else {
            self.now.push((r, hook.to_string(), vec![dt], self.conn_seq));
        }
    }

    pub fn full_name(&self, r: Ref) -> String {
        let d = self.d();
        let root = d.dom.root_ref();
        let mut parts = Vec::new();
        let mut cur = r;
        while let Some(i) = d.dom.get_by_ref(cur) {
            if cur == root {
                break;
            }
            parts.push(i.name.clone());
            cur = i.parent();
            if cur.is_none() {
                break;
            }
        }
        parts.reverse();
        parts.join(".")
    }
}

/// RUSTUDIO_MEMSTATS=1: a line of memory / instance / frame counters every
/// 5 s of Play on stderr (src/play.rs collect_garbage).
pub fn memstats_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUSTUDIO_MEMSTATS").is_some())
}

/// The Lua value for an instance (one userdata per instance, so `==` and
/// table keys work as in Roblox).
pub fn inst_value(lua: &Lua, r: Ref) -> mlua::Result<Value> {
    if r.is_none() {
        return Ok(Value::Nil);
    }
    if domains::assets_cloud::scene::access_blocked(lua,r){
        return Err(rt_err("The current thread cannot access 'RealtimeMedia' (lacking capability InternalTest)"));
    }
    let mut c = cache(lua)?;
    let slot = match c.slots.get(&r) {
        Some(s) => *s,
        None => {
            c.next_slot += 1;
            let s = c.next_slot;
            c.slots.insert(r, s);
            s
        }
    };
    let Some(weak) = c.weak.as_ref() else {
        return new_inst_ud(lua, r).map(Value::UserData);
    };
    if let Value::UserData(u) = weak.raw_get::<Value>(slot)? {
        return Ok(Value::UserData(u));
    }
    let u = new_inst_ud(lua, r)?;
    weak.raw_set(slot, u.clone())?;
    // A new Lua value (Instance.new, Clone, or one Lua let go of before): the
    // collector looks at it if it's unparented then.
    c.fresh.push(r);
    Ok(Value::UserData(u))
}

/// An Instance's userdata, or an `LObj` for engine objects that aren't
/// Instances (Logger, EditableImage, EditableMesh…: Studio's typeof says
/// "Object" for them).
fn new_inst_ud(lua: &Lua, r: Ref) -> mlua::Result<AnyUserData> {
    let object = lua
        .app_data_ref::<World>()
        .and_then(|w| w.doc.as_ref().and_then(|d| d.class_of(r)))
        .is_some_and(|class| !is_instance_class(&class));
    if object {
        lua.create_userdata(LObj(r))
    } else {
        lua.create_userdata(LInst(r))
    }
}

/// Whether Studio's API has `class` under Instance (unknown classes are).
fn is_instance_class(class: &str) -> bool {
    thread_local! {
        static CACHE: std::cell::RefCell<HashMap<String, bool>> = Default::default();
    }
    if let Some(v) = CACHE.with(|c| c.borrow().get(class).copied()) {
        return v;
    }
    let v = match crate::api::get() {
        Some(api) if api.classes.contains_key(class) => api.is_a(class, "Instance"),
        _ => true,
    };
    CACHE.with(|c| c.borrow_mut().insert(class.to_string(), v));
    v
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LInst(pub Ref);

/// A non-Instance engine object: an `LInst` in all but its typeof.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LObj(pub Ref);

impl FromLua for LInst {
    fn from_lua(v: Value, _: &Lua) -> mlua::Result<Self> {
        opt_inst(&v)
            .map(LInst)
            .ok_or_else(|| rt_err(format!("Instance expected, got {}", v.type_name())))
    }
}

fn opt_inst(v: &Value) -> Option<Ref> {
    match v {
        Value::UserData(u) => match u.borrow::<LInst>() {
            Ok(r) => Some(r.0),
            Err(_) => u.borrow::<LObj>().ok().map(|r| r.0),
        },
        _ => None,
    }
}

fn opt_ud<T: Clone + 'static>(v: &Value) -> Option<T> {
    if let Some(x) = crate::ltypes::vec_any::<T>(v) {
        return Some(x);
    }
    match v {
        Value::UserData(u) => u.borrow::<T>().ok().map(|r| (*r).clone()),
        _ => None,
    }
}

macro_rules! inst_userdata {
    ($ty:ty, $type_name:literal) => {
impl UserData for $ty {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", $type_name);
        // The bridge's C functions: unnamed,
        // shared by every Instance; the metatable is locked (lock_bridge).
        f.add_meta_field_with("__namecall", |lua| {
            lua.named_registry_value::<Function>("rs_inst_namecall")
        });
        f.add_meta_field_with("__index", |lua| {
            lua.named_registry_value::<Function>("rs_inst_index")
        });
        f.add_meta_field_with("__newindex", |lua| {
            lua.named_registry_value::<Function>("rs_inst_newindex")
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |lua, this, ()| {
            Ok(world(lua)?
                .d()
                .dom
                .get_by_ref(this.0)
                .map(|i| i.name.clone())
                .unwrap_or_default())
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: Value| {
            Ok(opt_inst(&o) == Some(this.0))
        });
    }
}
    };
}
inst_userdata!(LInst, "Instance");
inst_userdata!(LObj, "Object");

/// An `RBXScriptConnection`.
#[derive(Debug, Clone)]
struct LConn {
    key: (Ref, String),
    id: u64,
}

impl UserData for LConn {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "RBXScriptConnection");
        // C functions, as Studio's: self is checked first
        // on reads, the key first on writes.
        f.add_meta_field_with("__index", |lua| {
            crate::cmeta::cached(lua, "rs_conn_index", |lua| {
                let inner = lua.create_function(|lua, (this, key): (AnyUserData, String)| {
                    let this = this.borrow::<LConn>()?.clone();
                    match key.as_str() {
                        "Connected" => Ok(Value::Boolean(
                            signals(lua)?
                                .conns
                                .get(&this.key)
                                .is_some_and(|v| v.iter().any(|c| c.id == this.id)),
                        )),
                        // (`disconnect` is the deprecated spelling Animate
                        // still uses.) A new function each read, as Studio's.
                        "Disconnect" | "disconnect" => {
                            let check = lua.create_function(|lua, this: AnyUserData| {
                                let this = this.borrow::<LConn>()?.clone();
                                if let Some(v) = signals(lua)?.conns.get_mut(&this.key) {
                                    v.retain(|c| c.id != this.id);
                                }
                                Ok(true)
                            })?;
                            Ok(Value::Function(crate::cmeta::method(
                                lua,
                                &key,
                                Some(check),
                                None,
                                Some("RBXScriptConnection"),
                            )?))
                        }
                        _ => Err(rt_err(format!("{key} is not a valid member of RBXScriptConnection"))),
                    }
                })?;
                crate::cmeta::wrap_meta(lua, inner, false, &CONN_INDEX)
            })
        });
        f.add_meta_field_with("__newindex", |lua| {
            crate::cmeta::cached(lua, "rs_conn_newindex", |lua| {
                let inner = lua.create_function(|_, (_, key): (Value, String)| {
                    Err::<(), _>(rt_err(format!("{key} is not a valid member of RBXScriptConnection")))
                })?;
                crate::cmeta::wrap_meta(lua, inner, false, &CONN_NEWINDEX)
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok("Connection"));
    }
}

const CONN_INDEX: crate::cmeta::Spec =
    crate::cmeta::Spec { self_types: &[c"RBXScriptConnection"], key_first: false };
const CONN_NEWINDEX: crate::cmeta::Spec =
    crate::cmeta::Spec { self_types: &[c"RBXScriptConnection"], key_first: true };

// ------------------------------------------------------- class knowledge

/// Events the runtime knows (the reflection database lists only properties).
fn event_exists(class: &str, name: &str) -> bool {
    let is = |b: &str| doc::is_a(class, b);
    match name {
        "Changed" | "ChildAdded" | "ChildRemoved" | "DescendantAdded" | "DescendantRemoving"
        | "AncestryChanged" | "Destroying" | "AttributeChanged" => true,
        "Touched" | "TouchEnded" => is("BasePart"),
        "Heartbeat" | "Stepped" | "RenderStepped" | "PreSimulation" | "PostSimulation"
        | "PreRender" | "PreAnimation" => class == "RunService",
        "PlayerAdded" | "PlayerRemoving" => class == "Players",
        "CharacterAdded" | "CharacterRemoving" => class == "Player",
        "Died" | "HealthChanged" | "Jumping" | "StateChanged" | "Running" | "FreeFalling" => {
            class == "Humanoid"
        }
        "Event" => class == "BindableEvent",
        "OnServerEvent" | "OnClientEvent" => is("BaseRemoteEvent"),
        "InputBegan" | "InputEnded" => class == "UserInputService",
        _ => false,
    }
}

/// (method, class that has it), implemented in Rust.
const METHODS: &[(&str, &str)] = &[
    ("FindFirstChild", "Instance"),
    ("FindFirstChildOfClass", "Instance"),
    ("FindFirstChildWhichIsA", "Instance"),
    ("FindFirstAncestor", "Instance"),
    ("FindFirstAncestorOfClass", "Instance"),
    ("FindFirstAncestorWhichIsA", "Instance"),
    ("FindFirstDescendant", "Instance"),
    ("GetChildren", "Instance"),
    ("GetDescendants", "Instance"),
    ("IsA", "Instance"),
    ("IsDescendantOf", "Instance"),
    ("IsAncestorOf", "Instance"),
    ("Clone", "Instance"),
    ("Destroy", "Instance"),
    ("Remove", "Instance"),
    ("ClearAllChildren", "Instance"),
    ("GetFullName", "Instance"),
    ("GetAxis", "Attachment"),
    ("GetSecondaryAxis", "Attachment"),
    ("GetAttribute", "Instance"),
    ("SetAttribute", "Instance"),
    ("GetAttributes", "Instance"),
    ("GetPropertyChangedSignal", "Instance"),
    ("GetAttributeChangedSignal", "Instance"),
    ("GetService", "DataModel"),
    ("FindService", "DataModel"),
    ("GetPivot", "PVInstance"),
    ("PivotTo", "PVInstance"),
    ("SetPrimaryPartCFrame", "Model"),
    ("GetPrimaryPartCFrame", "Model"),
    ("MoveTo", "Model"),
    ("GetMass", "BasePart"),
    ("ApplyImpulse", "BasePart"),
    ("TakeDamage", "Humanoid"),
    ("GetState", "Humanoid"),
    ("Fire", "BindableEvent"),
    ("FireServer", "BaseRemoteEvent"),
    ("FireClient", "BaseRemoteEvent"),
    ("FireAllClients", "BaseRemoteEvent"),
    ("GetPlayers", "Players"),
    ("GetPlayerFromCharacter", "Players"),
    ("LoadCharacter", "Player"),
    ("IsClient", "RunService"),
    ("IsServer", "RunService"),
    ("IsStudio", "RunService"),
    ("IsRunning", "RunService"),
    ("IsRunMode", "RunService"),
    ("IsEdit", "RunService"),
    ("IsKeyDown", "UserInputService"),
];

/// Methods written in Lua because they yield (see PRELUDE).
const LUA_METHODS: &[(&str, &str)] = &[
    ("WaitForChild", "Instance"),
    ("Invoke", "BindableFunction"),
    ("InvokeServer", "RemoteFunction"),
    ("InvokeClient", "RemoteFunction"),
];

/// Property lookup through the class chain; aliases resolve to their
/// canonical property. Returns (canonical name, type, read-only).
fn prop_desc(class: &str, name: &str) -> Option<(&'static str, DataType<'static>, bool)> {
    let db = doc::db();
    let c = db.classes.get(class)?;
    for cls in db.superclasses_iter(c) {
        if let Some(p) = cls.properties.get(name) {
            if let PropertyKind::Alias { alias_for } = &p.kind {
                return prop_desc(class, alias_for);
            }
            let read_only = p.tags.contains(&PropertyTag::ReadOnly)
                || matches!(p.scriptability, Scriptability::Read | Scriptability::None);
            return Some((p.name, p.data_type.clone(), read_only));
        }
    }
    None
}

fn physics_prop(name: &str) -> bool {
    matches!(
        name,
        "CFrame" | "Size" | "Anchored" | "CanCollide" | "Shape" | "Massless" | "CanTouch"
    )
}

// ------------------------------------------------------- value conversion

pub fn variant_to_lua(lua: &Lua, v: &Variant, enum_name: Option<&str>) -> mlua::Result<Value> {
    let u = |x| -> mlua::Result<Value> { Ok(Value::UserData(x)) };
    Ok(match v {
        Variant::Bool(b) => Value::Boolean(*b),
        Variant::Int32(i) => Value::Number(*i as f64),
        Variant::Int64(i) => Value::Number(*i as f64),
        Variant::Float32(f) => Value::Number(*f as f64),
        Variant::Float64(f) => Value::Number(*f),
        Variant::String(s) => Value::String(lua.create_string(s)?),
        // rbx-dom reads string attributes (and some string properties) from
        // binary places as BinaryString: to scripts they are strings.
        Variant::BinaryString(b) => Value::String(lua.create_string(AsRef::<[u8]>::as_ref(b))?),
        Variant::ContentId(c) => Value::String(lua.create_string(c.as_str())?),
        Variant::Content(c) => editable::content_value(lua, c)?,
        Variant::Vector3(v) => LVector3(Vec3::new(v.x, v.y, v.z)).into_value(),
        Variant::Vector2(v) => u(lua.create_userdata(LVector2(glam::Vec2::new(v.x, v.y)))?)?,
        Variant::CFrame(cf) => u(lua.create_userdata(LCFrame(Xform::from_cframe(cf)))?)?,
        Variant::Color3(c) => u(lua.create_userdata(LColor3([c.r, c.g, c.b]))?)?,
        Variant::Color3uint8(c) => u(lua.create_userdata(LColor3([
            c.r as f32 / 255.0,
            c.g as f32 / 255.0,
            c.b as f32 / 255.0,
        ]))?)?,
        Variant::BrickColor(b) => u(lua.create_userdata(LBrickColor(*b))?)?,
        Variant::UDim(x) => u(lua.create_userdata(LUDim {
            scale: x.scale,
            offset: x.offset,
        })?)?,
        Variant::UDim2(x) => u(lua.create_userdata(LUDim2 {
            x: LUDim {
                scale: x.x.scale,
                offset: x.x.offset,
            },
            y: LUDim {
                scale: x.y.scale,
                offset: x.y.offset,
            },
        })?)?,
        Variant::Enum(e) => match enum_name.and_then(|n| LEnumItem::from_value(n, e.to_u32())) {
            Some(item) => crate::ltypes::enum_value(lua, item)?,
            None => Value::Number(e.to_u32() as f64),
        },
        Variant::EnumItem(e) => match LEnumItem::from_value(&e.ty, e.value) {
            Some(item) => crate::ltypes::enum_value(lua, item)?,
            None => Value::Number(e.value as f64),
        },
        Variant::Ref(r) => inst_value(lua, *r)?,
        Variant::Font(f) => Value::UserData(lua.create_userdata(LFont(f.clone()))?),
        other => crate::ltypes::variant_to_datatype(lua, other)?.unwrap_or(Value::Nil),
    })
}

fn bad_assign(prop: &str, expected: &str, got: &Value) -> mlua::Error {
    rt_err(format!(
        "Unable to assign property {prop}. {expected} expected, got {}",
        crate::cmeta::type_name(got)
    ))
}

/// Converts a Lua value to a property's type. `existing` keeps the stored
/// representation (Color3uint8 vs Color3).
fn lua_to_variant(
    prop: &str,
    v: &Value,
    ty: VariantType,
    enum_name: Option<&str>,
    existing: Option<&Variant>,
) -> mlua::Result<Variant> {
    // A numeric string converts, as Luau's tonumber (Place B's
    // `Code.Value = "042917"` on an IntValue; Studio takes it).
    let number = || {
        num(v)
            .or_else(|| match v {
                Value::String(s) => s.to_str().ok().and_then(|s| s.trim().parse::<f64>().ok()),
                _ => None,
            })
            .ok_or_else(|| bad_assign(prop, "number", v))
    };
    Ok(match ty {
        // Any value converts by Luau truthiness, as in Roblox (`Visible = 0`
        // shows the frame): Place D's second admin script binds a number state to
        // Visible, and Studio takes it (round 26).
        VariantType::Bool => Variant::Bool(!matches!(v, Value::Nil | Value::Boolean(false))),
        VariantType::Int32 => Variant::Int32(number()? as i32),
        VariantType::Int64 => Variant::Int64(number()? as i64),
        VariantType::Float32 => Variant::Float32(number()? as f32),
        VariantType::Float64 => Variant::Float64(number()?),
        VariantType::String => match v {
            Value::String(s) => Variant::String(s.to_str()?.to_string()),
            Value::Number(n) => Variant::String(n.to_string()),
            Value::Integer(n) => Variant::String(n.to_string()),
            _ => return Err(bad_assign(prop, "string", v)),
        },
        VariantType::ContentId => match v {
            Value::String(s) => Variant::ContentId(ContentId::from(s.to_str()?.to_string())),
            Value::UserData(_) if opt_ud::<editable::LContent>(v).is_some() => Variant::ContentId(ContentId::from(
                opt_ud::<editable::LContent>(v).and_then(|c| c.0.as_uri().map(str::to_string)).unwrap_or_default(),
            )),
            _ => return Err(bad_assign(prop, "string", v)),
        },
        VariantType::Content => match v {
            Value::String(s) => Variant::Content(Content::from(s.to_str()?.to_string())),
            Value::Nil => Variant::Content(Content::none()),
            Value::UserData(_) if opt_ud::<editable::LContent>(v).is_some() => {
                opt_ud::<editable::LContent>(v).map(|c| c.variant()).unwrap_or(Variant::Content(Content::none()))
            }
            _ => return Err(bad_assign(prop, "string", v)),
        },
        VariantType::Vector3 => {
            let p = opt_ud::<LVector3>(v).ok_or_else(|| bad_assign(prop, "Vector3", v))?;
            Variant::Vector3(Vector3::new(p.0.x, p.0.y, p.0.z))
        }
        VariantType::Vector2 => {
            let p = opt_ud::<LVector2>(v).ok_or_else(|| bad_assign(prop, "Vector2", v))?;
            Variant::Vector2(Vector2::new(p.0.x, p.0.y))
        }
        VariantType::CFrame => {
            let c = opt_ud::<LCFrame>(v).ok_or_else(|| bad_assign(prop, "CFrame", v))?;
            Variant::CFrame(c.0.orthonormalized().to_cframe())
        }
        VariantType::Color3 | VariantType::Color3uint8 => {
            let c = opt_ud::<LColor3>(v).ok_or_else(|| bad_assign(prop, "Color3", v))?;
            let c3 = Color3::new(c.0[0], c.0[1], c.0[2]);
            if ty == VariantType::Color3uint8 || matches!(existing, Some(Variant::Color3uint8(_))) {
                Variant::Color3uint8(Color3uint8::from(c3))
            } else {
                Variant::Color3(c3)
            }
        }
        VariantType::BrickColor => Variant::BrickColor(
            opt_ud::<LBrickColor>(v)
                .ok_or_else(|| bad_assign(prop, "BrickColor", v))?
                .0,
        ),
        VariantType::UDim => {
            let x = opt_ud::<LUDim>(v).ok_or_else(|| bad_assign(prop, "UDim", v))?;
            Variant::UDim(UDim::new(x.scale, x.offset))
        }
        VariantType::UDim2 => {
            let x = opt_ud::<LUDim2>(v).ok_or_else(|| bad_assign(prop, "UDim2", v))?;
            Variant::UDim2(UDim2::new(
                UDim::new(x.x.scale, x.x.offset),
                UDim::new(x.y.scale, x.y.offset),
            ))
        }
        VariantType::Enum => {
            let ename = enum_name.unwrap_or("");
            let value = match v {
                Value::UserData(_) => {
                    let item =
                        opt_ud::<LEnumItem>(v).ok_or_else(|| bad_assign(prop, "EnumItem", v))?;
                    if !ename.is_empty() && item.enum_type != ename {
                        return Err(rt_err(format!(
                            "Unable to assign property {prop}. Enum.{ename} expected, got Enum.{}",
                            item.enum_type
                        )));
                    }
                    item.value
                }
                Value::Number(n) => *n as u32,
                // A whole number reaches here as an integer (`TopSurface = 0`).
                Value::Integer(n) => *n as u32,
                Value::String(s) => LEnumItem::from_name(ename, &s.to_str()?)
                    .map(|i| i.value)
                    .ok_or_else(|| bad_assign(prop, "EnumItem", v))?,
                _ => return Err(bad_assign(prop, "EnumItem", v)),
            };
            Variant::Enum(Enum::from_u32(value))
        }
        VariantType::Font => Variant::Font(
            opt_ud::<LFont>(v)
                .ok_or_else(|| bad_assign(prop, "Font", v))?
                .0,
        ),
        VariantType::Ref => match v {
            Value::Nil => Variant::Ref(Ref::none()),
            // Luau's own argument error, at the assigning line (`p.Parent
            // = 3`, Studio probes 4): the value is the metamethod's #3.
            _ => Variant::Ref(opt_inst(v).ok_or_else(|| {
                crate::cmeta::located(format!(
                    "invalid argument #3 (Instance expected, got {})",
                    crate::cmeta::type_name(v)
                ))
            })?),
        },
        other => match crate::ltypes::datatype_to_variant(v) {
            Some(var) => var,
            None if matches!(v, Value::Nil) => {
                return Err(bad_assign(prop, &format!("{other:?}"), v))
            }
            None => {
                return Err(rt_err(format!(
                "Unable to assign property {prop}: {other:?} values can't be set from scripts yet"
            )))
            }
        },
    })
}

// ------------------------------------------------------------ index / set

fn inst_meta(lua: &Lua, r: Ref) -> mlua::Result<(String, String, Ref)> {
    let w = world(lua)?;
    let i = w
        .d()
        .dom
        .get_by_ref(r)
        .ok_or_else(|| rt_err("attempt to use an Instance that no longer exists"))?;
    Ok((i.class.to_string(), i.name.clone(), i.parent()))
}

/// An instance's class and parent, without copying its name.
fn inst_class(lua: &Lua, r: Ref) -> mlua::Result<(rbx_dom_weak::Ustr, Ref)> {
    let w = world(lua)?;
    let i = w
        .d()
        .dom
        .get_by_ref(r)
        .ok_or_else(|| rt_err("attempt to use an Instance that no longer exists"))?;
    Ok((i.class, i.parent()))
}

/// What `inst.key` is for a class, as far as it depends only on the class:
/// worked out once per (class, key) instead of scanning the method tables
/// (with an IsA per entry) on every property read.
#[derive(Clone, Copy)]
enum Disp {
    Callback,
    /// (name, base class): a method written in Lua.
    LuaMethod(&'static str, &'static str),
    /// (name, base class): a method implemented in Rust.
    Method(&'static str, &'static str),
    Event,
    Other,
}

fn disp(class: rbx_dom_weak::Ustr, key: &str) -> Disp {
    thread_local! {
        static CACHE: std::cell::RefCell<rustc_hash::FxHashMap<rbx_dom_weak::Ustr, rustc_hash::FxHashMap<Box<str>, Disp>>> =
            Default::default();
    }
    if let Some(d) = CACHE.with(|c| c.borrow().get(&class).and_then(|m| m.get(key).copied())) {
        return d;
    }
    let d = (|| {
        if crate::api::get()
            .and_then(|a| a.lookup(&class, key))
            .is_some_and(|(_, m)| m.kind == crate::api::Kind::Callback)
        {
            return Disp::Callback;
        }
        // Old scripts use deprecated spellings (findFirstChild, clone, remove…).
        let method = ext::canonical(key).unwrap_or(key);
        for (m, base) in LUA_METHODS.iter().chain(ext::LUA_METHODS).chain(domains::LUA_METHODS.iter()) {
            if *m == method && doc::is_a(&class, base) {
                return Disp::LuaMethod(m, base);
            }
        }
        for (m, base) in METHODS.iter().chain(ext::METHODS).chain(editable::METHODS).chain(domains::METHODS.iter()) {
            if *m == method && doc::is_a(&class, base) {
                return Disp::Method(m, base);
            }
        }
        if event_exists(&class, key) || ext::event_exists(&class, key) || domains::event_exists(&class, key) {
            return Disp::Event;
        }
        Disp::Other
    })();
    CACHE.with(|c| c.borrow_mut().entry(class).or_default().insert(key.into(), d));
    d
}

/// Whether `name` is a core and/or an extension method of `class` (cached).
fn method_owner(class: rbx_dom_weak::Ustr, name: &str) -> (bool, bool) {
    thread_local! {
        static CACHE: std::cell::RefCell<rustc_hash::FxHashMap<rbx_dom_weak::Ustr, rustc_hash::FxHashMap<Box<str>, (bool, bool)>>> =
            Default::default();
    }
    if let Some(v) = CACHE.with(|c| c.borrow().get(&class).and_then(|m| m.get(name).copied())) {
        return v;
    }
    let core = METHODS.iter().any(|(m, b)| *m == name && doc::is_a(&class, b));
    let extended = ext::METHODS.iter().chain(editable::METHODS).chain(domains::METHODS.iter()).any(|(m, b)| *m == name && doc::is_a(&class, b));
    CACHE.with(|c| c.borrow_mut().entry(class).or_default().insert(name.into(), (core, extended)));
    (core, extended)
}

/// The self of a member function call: an Instance that has the method
/// `name` of `base`, else Studio's `Expected ':' not '.' calling member
/// function X` (no position), checked before any argument.
fn member_self(lua: &Lua, args: &MultiValue, base: &str, name: &str) -> mlua::Result<Ref> {
    let expected = || rt_err(format!("Expected ':' not '.' calling member function {name}"));
    let this = opt_inst(&arg(args, 0)).ok_or_else(expected)?;
    let class = inst_class(lua, this)?.0;
    if !doc::is_a(&class, base) {
        return Err(expected());
    }
    Ok(this)
}

/// Argument checks a Lua-written method makes before it runs, as Studio's
/// reflected arguments (WaitForChild(nil): `Argument 1 missing or nil`).
fn lua_method_args(name: &str, args: &MultiValue) -> mlua::Result<()> {
    if name == "WaitForChild" && arg(args, 1).is_nil() {
        return Err(rt_err("Argument 1 missing or nil"));
    }
    Ok(())
}

/// A method's function, one per descriptor (base, name) per VM as in
/// Studio: a C function named after the method (src/cmeta.rs).
fn method_fn(lua: &Lua, name: &'static str, base: &'static str) -> mlua::Result<Function> {
    if let Some(f) = cache(lua)?.methods.get(&(base, name)) {
        return Ok(f.clone());
    }
    let over = cache(lua)?.overrides.get(name).cloned();
    if let Some(imp) = over {
        let check = lua.create_function(move |lua, args: MultiValue| {
            checked(lua, member_self(lua, &args, base, name).map(|_| MultiValue::new()))
        })?;
        let f = crate::cmeta::method(lua, name, Some(check), Some(imp), None)?;
        cache(lua)?.methods.insert((base, name), f.clone());
        return Ok(f);
    }
    let raw = lua.create_function(move |lua, args: MultiValue| {
        crate::bench::site(lua, "call", name);
        let r = crate::bench::api_timed("call", name, || {
            checked(lua, member_self(lua, &args, base, name).and_then(|_| call_method(lua, name, args)))
        });
        signal_timing::flush_immediate(lua);
        r
    })?;
    let f = crate::cmeta::method(lua, name, Some(raw), None, None)?;
    cache(lua)?.methods.insert((base, name), f.clone());
    Ok(f)
}

/// A Lua-written method's function: a named C function that checks self
/// and the arguments, then calls the Lua implementation (which may yield)
/// through a continuation.
fn lua_method_fn(lua: &Lua, name: &'static str, base: &'static str) -> mlua::Result<Function> {
    if let Some(f) = cache(lua)?.methods.get(&(base, name)) {
        return Ok(f.clone());
    }
    let check = lua.create_function(move |lua, args: MultiValue| {
        checked(
            lua,
            member_self(lua, &args, base, name)
                .and_then(|_| lua_method_args(name, &args))
                .map(|_| MultiValue::new()),
        )
    })?;
    let imp: Option<Function> = lua_methods(lua)?.get(name)?;
    let f = crate::cmeta::method(lua, name, Some(check), imp, None)?;
    cache(lua)?.methods.insert((base, name), f.clone());
    Ok(f)
}

/// The prelude's `__rs_lua_methods` (kept in the registry: inside a
/// callback, `lua.globals()` can be a script's environment).
fn lua_methods(lua: &Lua) -> mlua::Result<Table> {
    lua.named_registry_value("rs_lua_methods")
}

/// `X is not a valid member of Class "Full.Name"` (the DataModel by its
/// name: `… of DataModel "Game"`).
fn not_member(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Error {
    let full = match world(lua) {
        Ok(w) => {
            let full = w.full_name(r);
            if full.is_empty() {
                w.d().dom.get_by_ref(r).map(|i| i.name.clone()).unwrap_or_default()
            } else {
                full
            }
        }
        Err(_) => String::new(),
    };
    rt_err(format!("{key} is not a valid member of {class} \"{full}\""))
}

/// `inst:name(...)` (the Instance `__namecall`, src/cmeta.rs): the
/// method's results, or the Lua function to call for a Lua-written one.
/// Only methods are callable: properties, events, children and unknown
/// names are `not a valid member`.
fn namecall(lua: &Lua, name: &str, atom: i32, args: MultiValue) -> mlua::Result<MultiValue> {
    let this = opt_inst(&arg(&args, 0)).ok_or_else(|| rt_err("Instance expected"))?;
    let class_u = inst_class(lua, this)?.0;
    crate::telemetry::touch(class_u.as_str(), name);
    let rt = routes::route(class_u, name, atom);
    // Capability gates (security.rs) answer for restricted members first,
    // as they do for reads.
    if rt.restricted {
        match security::index_hook(lua, this, name) {
            Ok(None) => {}
            Ok(Some(Value::Function(f))) => return Ok(MultiValue::from_iter([Value::Function(f)])),
            Ok(Some(_)) => return checked(lua, Err(not_member(lua, this, class_u.as_str(), name))),
            Err(e) => return checked(lua, Err(e)),
        }
    }
    match rt.disp {
        Disp::Method(m, _) => {
            if rt.overridable {
                if let Some(f) = cache(lua)?.overrides.get(m) {
                    return Ok(MultiValue::from_iter([Value::Function(f.clone())]));
                }
            }
            crate::bench::site(lua, "call", m);
            let r = crate::bench::api_timed("call", m, || {
                checked(lua, call_method_routed(lua, m, args, this, class_u, &rt.method))
            });
            signal_timing::flush_immediate(lua);
            r
        }
        Disp::LuaMethod(m, _) => {
            if let Err(e) = lua_method_args(m, &args) {
                return checked(lua, Err(e));
            }
            let f: Option<Function> = lua_methods(lua)?.get(m)?;
            match f {
                Some(f) => Ok(MultiValue::from_iter([Value::Function(f)])),
                None => checked(lua, Ok(MultiValue::new())),
            }
        }
        Disp::Callback | Disp::Event => checked(lua, Err(not_member(lua, this, class_u.as_str(), name))),
        Disp::Other => {
            if !matches!(name, "Name" | "name" | "ClassName" | "className" | "Parent" | "parent") {
                // A member only index knows (stubs, gated members): its
                // function, or its failure (a capability refusal).
                match index(lua, this, name) {
                    Ok(Value::Function(f)) => return Ok(MultiValue::from_iter([Value::Function(f)])),
                    Err(e) => return checked(lua, Err(e)),
                    Ok(_) => {}
                }
            }
            checked(lua, Err(not_member(lua, this, class_u.as_str(), name)))
        }
    }
}

/// The cause of an error, without location or traceback.
pub fn error_message(e: &mlua::Error) -> String {
    let m = raw_error_message(e);
    match m.strip_prefix(crate::cmeta::LOCATED) {
        Some(rest) => rest.to_string(),
        None => m,
    }
}

/// `error_message`, keeping cmeta's LOCATED mark for the C bridge.
fn raw_error_message(e: &mlua::Error) -> String {
    match e {
        mlua::Error::RuntimeError(m) => m.lines().next().unwrap_or("").to_string(),
        mlua::Error::SyntaxError { message, .. } => {
            message.lines().next().unwrap_or("").to_string()
        }
        mlua::Error::CallbackError { cause, .. } => raw_error_message(cause),
        mlua::Error::WithContext { cause, .. } => raw_error_message(cause),
        other => other.to_string().lines().next().unwrap_or("").to_string(),
    }
}

/// `(true, results...)` or `(false, message)` for the Lua shims to rethrow.
/// `checked` for one result, without building a MultiValue (the bridge's
/// hot paths: Instance __index / __newindex).
fn checked1(lua: &Lua, r: mlua::Result<Value>) -> mlua::Result<(bool, Value)> {
    Ok(match r {
        Ok(v) => (true, v),
        Err(e) => (false, Value::String(lua.create_string(raw_error_message(&e))?)),
    })
}

fn checked(lua: &Lua, r: mlua::Result<MultiValue>) -> mlua::Result<MultiValue> {
    let mut out = MultiValue::new();
    match r {
        Ok(vals) => {
            out.push_back(Value::Boolean(true));
            out.extend(vals);
        }
        Err(e) => {
            out.push_back(Value::Boolean(false));
            out.push_back(Value::String(lua.create_string(raw_error_message(&e))?));
        }
    }
    Ok(out)
}

/// For engine domains that keep up with a few classes in the DataModel:
/// signals on `game` that fire like DescendantAdded / DescendantRemoving,
/// but only for instances that are one of `classes` (IsA), decided here
/// instead of in a Lua handler per instance (a 20k-part model arriving ran
/// four such handlers 20k times each: most of a client bulk reparent).
pub fn class_signals(lua: &Lua, classes: Vec<String>) -> mlua::Result<(Value, Value)> {
    let (root, id) = {
        let mut w = world(lua)?;
        w.class_watches.push((classes, HashMap::new()));
        (w.d().dom.root_ref(), w.class_watches.len() - 1)
    };
    Ok((
        signal(lua, root, &format!("ClassAdded:{id}"))?,
        signal(lua, root, &format!("ClassRemoving:{id}"))?,
    ))
}

/// Whether `r` has a connection or a waiter on `event` (true when the
/// signal tables are busy: then the fire is made and sorted out later).
fn hears(lua: &Lua, r: Ref, event: &str) -> bool {
    let Ok(Some(s)) = lua.try_app_data_ref::<Signals>() else {
        return true;
    };
    let key = (r, event.to_string());
    s.conns.get(&key).is_some_and(|v| !v.is_empty()) || s.waiters.get(&key).is_some_and(|v| !v.is_empty())
}

/// The class watches (`class_signals`) that instance `d` is one of.
fn class_watch_hits(w: &mut World, d: Ref) -> Vec<usize> {
    let Some(class) = w.d().class_of(d) else { return Vec::new() };
    let mut out = Vec::new();
    for (i, (classes, cache)) in w.class_watches.iter_mut().enumerate() {
        let hit = *cache
            .entry(class)
            .or_insert_with(|| classes.iter().any(|c| doc::is_a(&class, c)));
        if hit {
            out.push(i);
        }
    }
    out
}

fn signal(lua: &Lua, r: Ref, name: &str) -> mlua::Result<Value> {
    // A new signal object on every read, as Studio's (Studio probes
    // RBXScriptSignal.values: two reads are `==` but not rawequal). They
    // share one state table per (instance, event) while Lua holds any of
    // them, kept in a weak-valued table: the state holds its instance, so a
    // strong cache kept every instance whose events were ever read alive
    // for good. Connections live in `Signals`.
    let (key, sigs, methods) = {
        let c = cache(lua)?;
        let slot = c.slots.get(&r).copied().unwrap_or_default();
        (format!("{slot}:{name}"), c.sigs.clone(), c.sig_methods.clone())
    };
    let Some(sigs) = sigs else {
        let make: Function = lua.globals().get("__rs_new_signal")?;
        return make.call((inst_value(lua, r)?, name));
    };
    use crate::ltypes::LSignal;
    if let (Value::Table(data), Some(methods)) = (sigs.raw_get::<Value>(key.as_str())?, methods) {
        return Ok(Value::UserData(lua.create_userdata(LSignal::new(data, methods))?));
    }
    let make: Function = lua.globals().get("__rs_new_signal")?;
    let t: AnyUserData = make.call((inst_value(lua, r)?, name))?;
    {
        let s = t.borrow::<LSignal>()?;
        sigs.raw_set(key, s.data().clone())?;
        let methods = s.methods().clone();
        cache(lua)?.sig_methods = Some(methods);
    }
    Ok(Value::UserData(t))
}

/// `inst.key` from a script (the Instance `__index`): the capability
/// gates first (security.rs), for members a rule names, then `index`.
fn index_entry(lua: &Lua, r: Ref, key: &str, atom: i32) -> mlua::Result<Value> {
    let (class_u, parent) = inst_class(lua, r)?;
    let rt = routes::route(class_u, key, atom);
    if rt.restricted {
        if let Some(v) = security::index_hook(lua, r, key)? {
            return Ok(v);
        }
    }
    index_routed(lua, r, key, class_u, parent, rt)
}

/// `index_entry` for the C `__index` (runtime_routes.rs `fast_index`):
/// a plain member (name, class name, parent, a stored reflected property
/// no handler computes) is read in one borrow. Also says whether the read
/// was quiet (no handler ran: no signal flush needed).
/// A fast read's answer: a value, or an instance (pushed from the
/// instance cache without an mlua value when it has one).
pub(crate) enum Fv {
    V(Value),
    Inst(Ref),
    /// A new array of these instances (GetChildren).
    Insts(Vec<Ref>),
    /// A string, pushed as is.
    Str(String),
    Static(&'static str),
}

fn index_entry_fast(lua: &Lua, r: Ref, key: &str, atom: i32) -> (mlua::Result<Fv>, bool) {
    enum Got {
        Name,
        Class,
        Parent(Ref),
        Stored(Variant, Option<&'static str>),
        Default(Variant, Option<&'static str>, rbx_dom_weak::Ustr),
        Full(Ref),
    }
    let got = (|| -> mlua::Result<(rbx_dom_weak::Ustr, routes::Route, Got)> {
        let w = world(lua)?;
        let i = w
            .d()
            .dom
            .get_by_ref(r)
            .ok_or_else(|| rt_err("attempt to use an Instance that no longer exists"))?;
        let class_u = i.class;
        let rt = routes::route(class_u, key, atom);
        let got = if rt.child_only {
            // `inst.Child` (Studio: FindFirstChild after the
            // member lookup misses): a stored non-reflected property of that
            // name still wins, as in prop_get_plain; big child lists use
            // the child index (find_named_child).
            let stored = rbx_dom_weak::Ustr::from_existing(key).is_some_and(|k| i.properties.contains_key(&k));
            if stored || i.children().len() >= CHILD_INDEX_MIN {
                Got::Full(i.parent())
            } else {
                let d = w.d();
                match i.children().iter().copied().find(|c| d.dom.get_by_ref(*c).is_some_and(|c| c.name == key)) {
                    Some(c) => Got::Parent(c),
                    None => Got::Full(i.parent()),
                }
            }
        } else if !rt.plain {
            Got::Full(i.parent())
        } else {
            match rt.special {
                routes::Special::Name => Got::Name,
                routes::Special::ClassName => Got::Class,
                routes::Special::Parent => Got::Parent(i.parent()),
                _ => match rt.prop {
                    Some(p) => match i.properties.get(&p.canon) {
                        // An unset object property yields to a same-named
                        // child (prop_get_plain): the full path.
                        Some(Variant::Ref(x)) if x.is_none() => Got::Full(i.parent()),
                        Some(Variant::Ref(x)) => Got::Parent(*x),
                        Some(v) => Got::Stored(v.clone(), p.enum_name),
                        // The class default (prop_get_plain's answer when it
                        // isn't an object property, which may yield to a
                        // child of the same name).
                        None => match w.d().prop_key(r, p.canon) {
                            Some(v) if !matches!(v, Variant::Ref(_)) => Got::Default(v, p.enum_name, p.canon),
                            _ => Got::Full(i.parent()),
                        },
                    },
                    None => Got::Full(i.parent()),
                },
            }
        };
        Ok((class_u, rt, got))
    })();
    let (class_u, rt, got) = match got {
        Ok(x) => x,
        Err(e) => return (Err(e), false),
    };
    let quiet = rt.quiet;
    let v = match got {
        Got::Full(parent) => {
            if rt.restricted {
                match security::index_hook(lua, r, key) {
                    Ok(Some(v)) => return (Ok(Fv::V(v)), false),
                    Ok(None) => {}
                    Err(e) => return (Err(e), false),
                }
            }
            return (index_routed(lua, r, key, class_u, parent, rt).map(Fv::V), quiet);
        }
        Got::Name => {
            crate::telemetry::touch(class_u.as_str(), key);
            let name = world(lua).map(|w| w.d().dom.get_by_ref(r).map(|i| i.name.clone()).unwrap_or_default());
            return (name.map(Fv::Str), quiet);
        }
        Got::Class => {
            crate::telemetry::touch(class_u.as_str(), key);
            return (Ok(Fv::Static(class_u.as_str())), quiet);
        }
        Got::Stored(Variant::String(s), _) => {
            crate::telemetry::touch(class_u.as_str(), key);
            return (Ok(Fv::Str(s)), quiet);
        }
        Got::Parent(p) => {
            crate::telemetry::touch(class_u.as_str(), key);
            return (Ok(Fv::Inst(p)), quiet);
        }
        Got::Stored(v, en) => {
            crate::telemetry::touch(class_u.as_str(), key);
            variant_to_lua(lua, &v, en)
        }
        Got::Default(v, en, canon) => {
            crate::telemetry::touch(class_u.as_str(), key);
            crate::telemetry::default_read(lua, class_u.as_str(), canon.as_str());
            variant_to_lua(lua, &v, en)
        }
    };
    (v.map(Fv::V), quiet)
}

/// The cached userdata of instance `r` (the instance cache's weak table
/// slot), if it has one and access isn't blocked (else None: `inst_value`).
pub(crate) fn cached_inst_slot(lua: &Lua, r: Ref) -> Option<i64> {
    if domains::assets_cloud::scene::access_blocked(lua, r) {
        return None;
    }
    lua.app_data_ref::<Cache>()?.slots.get(&r).copied()
}

/// An argument of a native method, read off the Lua stack.
pub(crate) enum NArg<'a> {
    Nil,
    Bool(bool),
    Str(&'a str),
    Inst(Ref),
    /// Anything else (the general dispatcher converts it).
    Other,
}

/// Studio-style typed thunks for the hottest core methods, called from the
/// C `__namecall` with their arguments straight off the stack
/// (runtime_routes.rs `fast_namecall`): only when the method's route shows
/// the core implementation answers (no domain, no override, no gate) and
/// the arguments are the plain types it takes; else None, and the general
/// dispatcher runs. Answers as `call_method` would, and whether it was
/// quiet (nothing fired).
pub(crate) fn native_call(lua: &Lua, r: Ref, name: &str, atom: i32, args: &[NArg]) -> Option<mlua::Result<(Fv, bool)>> {
    let class_u = {
        let w = world(lua).ok()?;
        w.d().dom.get_by_ref(r)?.class
    };
    let rt = routes::route(class_u, name, atom);
    let Disp::Method(m, _) = rt.disp else { return None };
    if rt.restricted || rt.overridable || !rt.method.core || rt.method.domains != 0 {
        return None;
    }
    let class = class_u.as_str();
    let str1 = || match args.first() {
        Some(NArg::Str(s)) => Some(*s),
        _ => None,
    };
    let out = match m {
        "IsA" => {
            let c = str1()?;
            Ok((Fv::V(Value::Boolean(doc::is_a(class, c))), true))
        }
        "FindFirstChild" => {
            let n = str1()?;
            let rec = match args.get(1) {
                None | Some(NArg::Nil) => false,
                Some(NArg::Bool(b)) => *b,
                _ => return None,
            };
            let found = if rec {
                find_child(lua, r, |i| i.name == n, true)
            } else {
                find_named_child(lua, r, n)
            };
            found.map(|f| (Fv::Inst(f.unwrap_or_else(Ref::none)), true))
        }
        "FindFirstChildOfClass" => {
            let c = str1()?;
            find_child(lua, r, |i| i.class == c, false).map(|f| (Fv::Inst(f.unwrap_or_else(Ref::none)), true))
        }
        "GetAttribute" => {
            let n = str1()?;
            static ATTRS: std::sync::LazyLock<rbx_dom_weak::Ustr> = std::sync::LazyLock::new(|| ustr("Attributes"));
            let v = {
                let w = world(lua).ok()?;
                match w.d().dom.get_by_ref(r).and_then(|i| i.properties.get(&*ATTRS)) {
                    Some(Variant::Attributes(a)) => a.get(n).cloned(),
                    _ => None,
                }
            };
            match v {
                Some(v) => variant_to_lua(lua, &v, None).map(|v| (Fv::V(v), true)),
                None => Ok((Fv::V(Value::Nil), true)),
            }
        }
        "GetChildren" if args.is_empty() || matches!(args, [NArg::Nil]) => {
            children_of(lua, r).map(|k| (Fv::Insts(k), true))
        }
        "IsDescendantOf" => {
            let other = match args.first() {
                None | Some(NArg::Nil) => None,
                Some(NArg::Inst(o)) => Some(*o),
                _ => return None,
            };
            let w = world(lua).ok()?;
            Ok((Fv::V(Value::Boolean(other.is_some_and(|o| w.d().is_ancestor(o, r)))), true))
        }
        "GetService" | "FindService" => {
            let c = str1()?;
            if security::service_hidden(lua, c) {
                Ok((Fv::V(Value::Nil), true))
            } else {
                get_service(lua, c, m == "GetService").map(|f| (Fv::Inst(f.unwrap_or_else(Ref::none)), false))
            }
        }
        _ => return None,
    };
    crate::telemetry::touch(class, name);
    Some(out)
}

/// `inst.Parent = p` from the C `__newindex` when no domain or
/// runtime_ext takes Parent for the class (else None: the full path):
/// assigning the parent it has is a no-op, answered in one borrow.
/// Returns whether the write was quiet (nothing changed or fired).
fn parent_write(lua: &Lua, r: Ref, key: &str, atom: i32, p: Ref) -> Option<mlua::Result<bool>> {
    let (class_u, same) = {
        let w = world(lua).ok()?;
        let i = w.d().dom.get_by_ref(r)?;
        (i.class, i.parent() == p)
    };
    let rt = routes::route(class_u, key, atom);
    if !rt.plain_parent_write {
        return None;
    }
    crate::telemetry::touch(class_u.as_str(), key);
    // Domains whose newindex names Parent (testing's debugger objects) are
    // asked first, as in `newindex_routed`.
    if rt.ni_domains != 0 {
        let class = class_u.as_str();
        let taken = inst_value(lua, p)
            .and_then(|v| domains::newindex_masked(lua, r, class, key, &v, rt.ni_domains));
        match taken {
            Ok(false) => {}
            Ok(true) => return Some(Ok(false)),
            Err(e) => return Some(Err(e)),
        }
    }
    if same {
        return Some(Ok(true));
    }
    Some(set_parent(lua, r, p).map(|_| false))
}

/// `inst.key = value` from a script (the Instance `__newindex`): the
/// capability gates for members a rule names, then `newindex`.
fn newindex_entry(lua: &Lua, r: Ref, key: &str, value: Value, atom: i32) -> mlua::Result<()> {
    let (class_u, _) = inst_class(lua, r)?;
    let rt = routes::route(class_u, key, atom);
    if rt.restricted {
        security::newindex_hook(lua, r, key)?;
    }
    newindex_routed(lua, r, key, value, class_u, rt)
}

fn index(lua: &Lua, r: Ref, key: &str) -> mlua::Result<Value> {
    let (class_u, parent) = inst_class(lua, r)?;
    index_routed(lua, r, key, class_u, parent, routes::route(class_u, key, -1))
}

fn index_routed(
    lua: &Lua,
    r: Ref,
    key: &str,
    class_u: rbx_dom_weak::Ustr,
    parent: Ref,
    rt: routes::Route,
) -> mlua::Result<Value> {
    let class: &str = class_u.as_str();
    crate::telemetry::touch(class, key);
    match rt.special {
        routes::Special::None => {}
        routes::Special::Name => {
            let w = world(lua)?;
            let name = w.d().dom.get_by_ref(r).map(|i| i.name.as_str()).unwrap_or("");
            return Ok(Value::String(lua.create_string(name)?));
        }
        routes::Special::ClassName => return Ok(Value::String(lua.create_string(class)?)),
        routes::Special::Parent => return inst_value(lua, parent),
        routes::Special::ScriptEnabled => return Ok(Value::Boolean(script_enabled_here(lua, r)?)),
    }
    match rt.disp {
        // Callbacks can only be set: Studio's __index (Luau bridge,
        //) refuses to read one, set or not.
        Disp::Callback => {
            return Err(rt_err(format!(
                "{key} is a callback member of {class}; you can only set the callback value, get is not available"
            )));
        }
        Disp::LuaMethod(method, base) => return Ok(Value::Function(lua_method_fn(lua, method, base)?)),
        Disp::Method(method, base) => return Ok(Value::Function(method_fn(lua, method, base)?)),
        Disp::Event => {
            crate::telemetry::event_used(lua, class, key);
            return signal(lua, r, key);
        }
        Disp::Other => {}
    }
    // Only the handlers that can answer this member (runtime_routes.rs).
    if rt.domains != 0 || domains::route_check_on() {
        if let Some(v) = domains::prop_get_masked(lua, r, class, key, rt.domains)? {
            return Ok(v);
        }
    }
    if rt.ext {
        if let Some(v) = ext::prop_get(lua, r, class, key)? {
            return Ok(v);
        }
    } else if domains::route_check_on() {
        if let Ok(Some(_)) | Err(_) = ext::prop_get(lua, r, class, key) {
            eprintln!("ROUTE MISS ext answers {class}.{key}");
        }
    }
    if let Some(v) = prop_get_routed(lua, r, class, key, rt.core_special, rt.prop)? {
        return Ok(v);
    }
    // Old lowercase spellings (sound:play, player:isFriendsWith) reach the
    // real capitalised member before any stub of the deprecated name.
    if let Some(cap) = capitalized(key).filter(|c| real_member(&class, c)) {
        return index(lua, r, &cap);
    }
    // A member rustudio has no behaviour for yet still exists (src/stubs.rs);
    // members come before children, as in Roblox.
    if let Some(v) = stubs::index(lua, r, &class, key)? {
        return Ok(v);
    }
    if let Some(c) = find_named_child(lua, r, key)? {
        return inst_value(lua, c);
    }
    // Old scripts use lowercase names (script.parent, game.workspace).
    if let Some(cap) = capitalized(key) {
        if let Ok(v) = index(lua, r, &cap) {
            return Ok(v);
        }
    }
    crate::telemetry::gap(lua, "missing", &class, key);
    Err(not_member(lua, r, &class, key))
}

/// Whether rustudio itself implements `name` on `class` (a method, an event
/// or a reflected property), as opposed to answering it with a stub.
fn real_member(class: &str, name: &str) -> bool {
    LUA_METHODS
        .iter()
        .chain(ext::LUA_METHODS)
        .chain(domains::LUA_METHODS.iter())
        .chain(METHODS.iter())
        .chain(ext::METHODS)
        .chain(editable::METHODS).chain(domains::METHODS.iter())
        .any(|(m, base)| *m == name && doc::is_a(class, base))
        || event_exists(class, name)
        || ext::event_exists(class, name)
        || domains::event_exists(class, name)
        || prop_desc(class, name).is_some()
}

/// "parent" → "Parent" for keys starting lowercase.
fn capitalized(key: &str) -> Option<String> {
    let mut c = key.chars();
    let first = c.next()?;
    first
        .is_ascii_lowercase()
        .then(|| first.to_ascii_uppercase().to_string() + c.as_str())
}

fn part_xform(lua: &Lua, r: Ref) -> mlua::Result<Xform> {
    Ok(match world(lua)?.d().prop(r, "CFrame") {
        Some(Variant::CFrame(cf)) => Xform::from_cframe(&cf),
        _ => Xform {
            pos: Vec3::ZERO,
            rot: glam::Mat3::IDENTITY,
        },
    })
}

/// An Attachment's frame in its part (CFrame) and the part's frame in the
/// world (identity when it isn't on a part yet, as in Roblox).
fn attachment_frames(lua: &Lua, r: Ref) -> mlua::Result<(Xform, Xform)> {
    let w = world(lua)?;
    let d = w.d();
    let local = match d.prop(r, "CFrame") {
        Some(Variant::CFrame(cf)) => Xform::from_cframe(&cf),
        _ => Xform {
            pos: Vec3::ZERO,
            rot: glam::Mat3::IDENTITY,
        },
    };
    let part = crate::attachment::parent(d, r, true).unwrap_or(Xform {
        pos: Vec3::ZERO, rot: glam::Mat3::IDENTITY,
    });
    Ok((local, part))
}

fn compose(a: &Xform, b: &Xform) -> Xform {
    Xform {
        pos: a.apply(b.pos),
        rot: a.rot * b.rot,
    }
}

fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    let rt = routes::route(ustr(class), key, -1);
    prop_get_routed(lua, r, class, key, rt.core_special, rt.prop)
}

/// The core read: computed frames and the like (`special`), else the
/// stored or default value of the reflected property `prop`.
fn prop_get_routed(
    lua: &Lua,
    r: Ref,
    class: &str,
    key: &str,
    special: bool,
    prop: Option<routes::PropInfo>,
) -> mlua::Result<Option<Value>> {
    if special {
        if let Some(v) = prop_get_special(lua, r, class, key)? {
            return Ok(Some(v));
        }
    }
    prop_get_plain(lua, r, class, key, prop)
}

fn prop_get_special(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if class == "Bone" && matches!(key, "TransformedCFrame" | "TransformedWorldCFrame") {
        let d = world(lua)?;
        let xf = if key == "TransformedCFrame" { crate::attachment::local(d.d(), r, true) }
            else { crate::attachment::world(d.d(), r, true).unwrap_or(crate::attachment::local(d.d(), r, true)) };
        return Ok(Some(Value::UserData(lua.create_userdata(LCFrame(xf))?)));
    }
    if doc::is_a(class, "Attachment")
        && matches!(
            key,
            "WorldCFrame" | "WorldPosition" | "WorldOrientation" | "Position" | "Orientation"
            | "Axis" | "SecondaryAxis" | "WorldAxis" | "WorldSecondaryAxis"
        )
    {
        let (local, part) = attachment_frames(lua, r)?;
        let worldf = compose(&part, &local);
        let v = match key {
            "WorldCFrame" => {
                return Ok(Some(Value::UserData(
                    lua.create_userdata(crate::ltypes::LCFrame(worldf))?,
                )))
            }
            "WorldPosition" => worldf.pos,
            "WorldOrientation" => worldf.orientation_deg(),
            "Position" => local.pos,
            "Axis" => local.rot.x_axis,
            "SecondaryAxis" => local.rot.y_axis,
            "WorldAxis" => worldf.rot.x_axis,
            "WorldSecondaryAxis" => worldf.rot.y_axis,
            _ => local.orientation_deg(),
        };
        return Ok(Some(LVector3(v).into_value()));
    }
    if doc::is_a(class, "BasePart") {
        match key {
            "Position" | "Orientation" | "Rotation" => {
                let xf = part_xform(lua, r)?;
                let v = if key == "Position" {
                    xf.pos
                } else {
                    xf.orientation_deg()
                };
                return Ok(Some(LVector3(v).into_value()));
            }
            "BrickColor" => {
                let rgb = match world(lua)?.d().prop(r, "Color") {
                    Some(Variant::Color3uint8(c)) => [c.r, c.g, c.b],
                    Some(Variant::Color3(c)) => {
                        let c = Color3uint8::from(c);
                        [c.r, c.g, c.b]
                    }
                    _ => [163, 162, 165],
                };
                return Ok(Some(Value::UserData(
                    lua.create_userdata(LBrickColor(nearest_brick(rgb)))?,
                )));
            }
            "AssemblyLinearVelocity" | "Velocity" => {
                let v = world(lua)?
                    .velocities
                    .get(&r)
                    .copied()
                    .unwrap_or(Vec3::ZERO);
                return Ok(Some(LVector3(v).into_value()));
            }
            _ => {}
        }
    }
    Ok(None)
}

fn prop_get_plain(
    lua: &Lua,
    r: Ref,
    class: &str,
    key: &str,
    prop: Option<routes::PropInfo>,
) -> mlua::Result<Option<Value>> {
    // (set_callback keeps only these)
    if matches!(key, "OnInvoke" | "OnServerInvoke" | "OnClientInvoke") {
        if let Some(f) = cache(lua)?.callbacks.get(&(r, key.to_string())) {
            return Ok(Some(Value::Function(f.clone())));
        }
    }
    let (enum_name, value, defaulted) = match prop {
        Some(p) => {
            let (stored, v) = {
                let w = world(lua)?;
                let d = w.d();
                match d.dom.get_by_ref(r).and_then(|i| i.properties.get(&p.canon)) {
                    Some(v) => (true, Some(v.clone())),
                    None => (false, d.prop_key(r, p.canon)),
                }
            };
            (p.enum_name, v, (!stored).then_some(p.canon.as_str()))
        }
        None => {
            let v = world(lua)?
                .d()
                .dom
                .get_by_ref(r)
                // (A name never interned is no instance's property: don't
                // intern every child name a script indexes.)
                .and_then(|i| i.properties.get(&rbx_dom_weak::Ustr::from_existing(key)?).cloned());
            match v {
                Some(v) => (None, Some(v), None),
                None => return Ok(None),
            }
        }
    };
    // An unset object property (game.Workspace, workspace.Terrain, …) yields
    // to a child of the same name, which is what those properties point at.
    if matches!(value, None | Some(Variant::Ref(_)))
        && value
            .as_ref()
            .is_none_or(|v| matches!(v, Variant::Ref(x) if x.is_none()))
    {
        let child = {
            let w = world(lua)?;
            let d = w.d();
            let cap = capitalized(key);
            d.dom.get_by_ref(r).and_then(|i| {
                i.children().iter().copied().find(|c| {
                    d.dom
                        .get_by_ref(*c)
                        .is_some_and(|ci| ci.name == key || Some(&ci.name) == cap.as_ref())
                })
            })
        };
        if let Some(c) = child {
            return Ok(Some(inst_value(lua, c)?));
        }
    }
    // Answered from the reflection default: a gap if Roblox computes it.
    if let Some(canon) = defaulted {
        crate::telemetry::default_read(lua, class, canon);
    }
    match value {
        Some(v) => Ok(Some(variant_to_lua(lua, &v, enum_name)?)),
        // Known to the reflection database but with no default there: the
        // stub's typed default (false, 0, Enum.X.Default…) rather than nil.
        // (An unset object property is nil in Roblox too: not a stub.)
        None if stubs::typed_default(class, key) => match stubs::default_value(lua, r, class, key)? {
            Some(v) => Ok(Some(v)),
            None => Ok(Some(Value::Nil)),
        },
        None => Ok(Some(Value::Nil)),
    }
}

fn set_parent(lua: &Lua, r: Ref, new_parent: Ref) -> mlua::Result<()> {
    let (old, descendants) = {
        let mut w = world(lua)?;
        let old = w
            .d()
            .dom
            .get_by_ref(r)
            .map(|i| i.parent())
            .unwrap_or_else(Ref::none);
        // Assigning the existing parent is a no-op, including nil after
        // Destroy(). Index cleanup repeats Parent=nil on stripped book parts.
        if old == new_parent {
            return Ok(());
        }
        if w.destroyed.contains(&r) {
            let name = w
                .d()
                .dom
                .get_by_ref(r)
                .map(|i| i.name.clone())
                .unwrap_or_default();
            return Err(rt_err(format!(
                "The Parent property of {name} is locked, current parent: NULL, new parent {}",
                w.full_name(new_parent)
            )));
        }
        if r == w.d().dom.root_ref() {
            return Err(rt_err("The Parent property of game is locked"));
        }
        if new_parent.is_some() {
            if new_parent == r || w.d().is_ancestor(r, new_parent) {
                return Err(rt_err(
                    "Attempt to set parent of an Instance to itself or one of its descendants",
                ));
            }
            if w.d().dom.get_by_ref(new_parent).is_none() {
                return Err(rt_err("The new parent no longer exists"));
            }
        }
        w.dm().dom.transfer_within(r, new_parent);
        if new_parent.is_none() {
            w.orphans.insert(r);
        }
        w.record(RepOp::Parent(r));
        w.dm().mark_tree(r);
        // An Explosion going into the Workspace goes off.
        if w.d().class_of(r).is_some_and(|c| c == "Explosion")
            && w.d().workspace().is_some_and(|ws| w.d().is_ancestor(ws, r))
        {
            w.requests.explosions.push(r);
        }
        w.structure_changed = true;
        // Scripts arriving where they run start (next dispatch), as in Roblox.
        if new_parent.is_some() {
            w.queue_scripts(r);
        }
        let descendants: Vec<Ref> = w.d().dom.descendants_of(r).map(|i| i.referent()).collect();
        for d in &descendants {
            w.dirty.insert(*d);
        }
        (old, descendants)
    };
    let me = inst_value(lua, r)?;
    let pname = Value::String(lua.create_string("Parent")?);
    // The ancestor chains the subtree left and joined, and whether each
    // reaches the DataModel: tagged instances entering or leaving it fire
    // CollectionService's InstanceAdded / InstanceRemoved signals (Roblox
    // does this for parenting, cloning in and replication, not only AddTag).
    let chain = |w: &World, from: Ref| -> (Vec<Ref>, bool) {
        let d = w.d();
        let root = d.dom.root_ref();
        let mut out = Vec::new();
        let mut cur = from;
        while cur.is_some() {
            out.push(cur);
            if cur == root {
                return (out, true);
            }
            cur = d.dom.get_by_ref(cur).map(|i| i.parent()).unwrap_or_else(Ref::none);
        }
        (out, false)
    };
    let (old_chain, was_in) = chain(&*world(lua)?, old);
    let (new_chain, now_in) = chain(&*world(lua)?, new_parent);
    // Tagged instances in the moved subtree, when it crossed the DataModel's edge.
    let tagged_moved: Vec<(Ref, Vec<String>)> = if was_in != now_in {
        let w = world(lua)?;
        let key = ustr("Tags");
        descendants
            .iter()
            .filter_map(|d| match w.d().dom.get_by_ref(*d)?.properties.get(&key) {
                Some(Variant::Tags(t)) if t.iter().next().is_some() => {
                    Some((*d, t.iter().map(|s| s.to_string()).collect()))
                }
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    let cs = if tagged_moved.is_empty() {
        None
    } else {
        get_service(lua, "CollectionService", false)?
    };
    // DescendantAdded / DescendantRemoving fire on every ancestor for the
    // moved instance and each of its descendants, as in Roblox. Values are
    // only made for ancestors someone listens to (maps move thousands).
    let mut values: HashMap<Ref, Value> = HashMap::new();
    values.insert(r, me.clone());
    let mut value_of = |d: Ref| -> mlua::Result<Value> {
        if let Some(v) = values.get(&d) {
            return Ok(v.clone());
        }
        let v = inst_value(lua, d)?;
        values.insert(d, v.clone());
        Ok(v)
    };
    // Only ancestors with a connection or waiter on that very event: `game`
    // always has some (engine domains), and a 20k-part model made 20k
    // DescendantRemoving fires there for nobody.
    let listening = |chain: &[Ref], event: &str| -> mlua::Result<Vec<Ref>> {
        let w = world(lua)?;
        Ok(chain
            .iter()
            .copied()
            .filter(|a| w.listening.contains(a) && hears(lua, *a, event))
            .collect())
    };
    let old_listen = listening(&old_chain, "DescendantRemoving")?;
    let new_listen = listening(&new_chain, "DescendantAdded")?;
    let mut fires: Vec<(Ref, &'static str, Value)> = Vec::new();
    for a in &old_listen {
        for d in &descendants {
            fires.push((*a, "DescendantRemoving", value_of(*d)?));
        }
    }
    let mut add_fires: Vec<(Ref, &'static str, Value)> = Vec::new();
    for a in &new_listen {
        for d in &descendants {
            add_fires.push((*a, "DescendantAdded", value_of(*d)?));
        }
    }
    let mut tag_fires: Vec<(Ref, String, Value)> = Vec::new();
    if let Some(cs) = cs {
        let ev = if now_in { "TagAdded:" } else { "TagRemoved:" };
        for (d, tags) in &tagged_moved {
            for t in tags {
                tag_fires.push((cs, format!("{ev}{t}"), value_of(*d)?));
            }
        }
    }
    // Class watches (`class_signals`): DescendantRemoving / Added on game
    // for the instances of the watched classes only.
    let mut watch_out: Vec<(Ref, String, Value)> = Vec::new();
    let mut watch_in: Vec<(Ref, String, Value)> = Vec::new();
    let watched = !world(lua)?.class_watches.is_empty();
    if watched && (was_in || now_in) {
        let root = world(lua)?.d().dom.root_ref();
        for d in &descendants {
            let hits = class_watch_hits(&mut *world(lua)?, *d);
            for i in hits {
                if was_in {
                    watch_out.push((root, format!("ClassRemoving:{i}"), value_of(*d)?));
                }
                if now_in {
                    watch_in.push((root, format!("ClassAdded:{i}"), value_of(*d)?));
                }
            }
        }
    }
    let mut w = world(lua)?;
    if old.is_some() {
        w.fire(old, "ChildRemoved", vec![me.clone()]);
        for (a, name, v) in fires {
            w.fire(a, name, vec![v]);
        }
        for (a, name, v) in watch_out {
            w.fire(a, &name, vec![v]);
        }
    }
    if new_parent.is_some() {
        w.fire(new_parent, "ChildAdded", vec![me.clone()]);
        for (a, name, v) in add_fires {
            w.fire(a, name, vec![v]);
        }
        for (a, name, v) in watch_in {
            w.fire(a, &name, vec![v]);
        }
    }
    for (cs, name, v) in tag_fires {
        w.fire(cs, &name, vec![v]);
    }
    w.fire(r, "Changed", vec![pname]);
    w.fire(r, "Changed:Parent", vec![]);
    for d in descendants {
        w.fire(d, "AncestryChanged", vec![]);
    }
    Ok(())
}

fn store_prop(lua: &Lua, r: Ref, name: &str, v: Variant) -> mlua::Result<()> {
    store_prop_key(lua, r, ustr(name), v)
}

/// `store_prop` by an interned name.
fn store_prop_key(lua: &Lua, r: Ref, key: rbx_dom_weak::Ustr, v: Variant) -> mlua::Result<()> {
    let name = key.as_str();
    let mut w = world(lua)?;
    let changed = {
        let Some(inst) = w.dm().dom.get_by_ref_mut(r) else {
            return Err(rt_err("the Instance no longer exists"));
        };
        let old = inst.properties.insert(key, v.clone());
        // A legacy asset id and its Content twin are one property in Roblox
        // (Texture / TextureContent …): keep the twin in step, or readers that
        // take the twin first see a stale value or the class default (every
        // Place A Vfx emitter, made with Instance.new and then given Texture, drew
        // the default sparkle from TextureContent's default).
        if let Some(twin) = doc::content_twin(name) {
            let uri = crate::scene::content_string(Some(v.clone())).unwrap_or_default();
            let c = if uri.is_empty() {
                rbx_dom_weak::types::Content::none()
            } else {
                rbx_dom_weak::types::Content::from_uri(uri)
            };
            inst.properties.insert(ustr(twin), Variant::Content(c));
        } else if name.ends_with("Content") {
            for legacy in doc::content_legacy_names(name) {
                inst.properties.remove(&ustr(legacy));
            }
        }
        old.as_ref() != Some(&v)
    };
    if changed {
        w.dm().mark_prop(r, key);
        w.record(RepOp::Prop(r, name.to_string()));
        let class = w.d().class_of(r);
        if physics_prop(name) && class.is_some_and(|c| doc::is_a(&c, "BasePart")) {
            w.dirty.insert(r);
        }
        // Nobody listening to this instance or the DataModel (services'
        // on_fire), telemetry off: both fires below would do nothing.
        let root = w.d().dom.root_ref();
        if !w.listening.contains(&r) && !w.listening.contains(&root) && !crate::telemetry::on() {
            return Ok(());
        }
        // Changed's argument: the property name, or a ValueBase's new value.
        let value_changed = name == "Value" && class.is_some_and(|c| doc::is_a(&c, "ValueBase"));
        drop(w);
        let pname = if value_changed {
            variant_to_lua(lua, &v, None)?
        } else {
            Value::String(lua.create_string(name)?)
        };
        let mut w = world(lua)?;
        w.fire(r, "Changed", vec![pname]);
        w.fire(r, &format!("Changed:{name}"), vec![]);
    }
    Ok(())
}

fn set_callback(lua: &Lua, r: Ref, key: &str, value: Value) -> mlua::Result<()> {
    let mut c = cache(lua)?;
    match value {
        Value::Function(f) => {
            c.callbacks.insert((r, key.to_string()), f);
        }
        _ => {
            c.callbacks.retain(|k, _| !(k.0 == r && k.1 == key));
        }
    }
    Ok(())
}

fn newindex(lua: &Lua, r: Ref, key: &str, value: Value) -> mlua::Result<()> {
    let (class_u, _) = inst_class(lua, r)?;
    newindex_routed(lua, r, key, value, class_u, routes::route(class_u, key, -1))
}

/// `inst.key = value` with the member's route (runtime_routes.rs): only
/// the domains that can take the write are asked.
fn newindex_routed(
    lua: &Lua,
    r: Ref,
    key: &str,
    value: Value,
    class_u: rbx_dom_weak::Ustr,
    rt: routes::Route,
) -> mlua::Result<()> {
    let class: &str = class_u.as_str();
    crate::telemetry::touch(class, key);
    if (rt.ni_domains != 0 || domains::route_check_on())
        && domains::newindex_masked(lua, r, class, key, &value, rt.ni_domains)?
    {
        return Ok(());
    }
    if (rt.ni_ext || domains::route_check_on()) && ext::newindex(lua, r, class, key, &value)? {
        if !rt.ni_ext {
            eprintln!("ROUTE MISS ext takes {class}.{key} =");
        }
        return Ok(());
    }
    // The old FontSize enum ("Size28", Enum.FontSize.Size28) is TextSize in
    // Roblox: setting one sets the other.
    if key == "FontSize" && matches!(class, "TextLabel" | "TextButton" | "TextBox") {
        let name = match &value {
            Value::String(s) => s.to_str()?.to_string(),
            v => opt_ud::<LEnumItem>(v)
                .map(|e| e.name)
                .ok_or_else(|| bad_assign("FontSize", "EnumItem", v))?,
        };
        let px = name
            .strip_prefix("Size")
            .and_then(|n| n.parse::<f64>().ok())
            .ok_or_else(|| rt_err(&format!("Invalid value \"{name}\" for enum FontSize")))?;
        return newindex(lua, r, "TextSize", Value::Number(px));
    }
    match key {
        "Name" => {
            let s = match &value {
                Value::String(s) => s.to_str()?.to_string(),
                Value::Number(n) => n.to_string(),
                _ => return Err(bad_assign("Name", "string", &value)),
            };
            let pname = Value::String(lua.create_string("Name")?);
            let mut w = world(lua)?;
            if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
                i.name = s;
            }
            w.dm().mark_prop(r, ustr("Name"));
            w.fire(r, "Changed", vec![pname]);
            w.fire(r, "Changed:Name", vec![]);
            w.record(RepOp::Name(r));
            return Ok(());
        }
        "Parent" => {
            let p = match &value {
                Value::Nil => Ref::none(),
                // Luau's argument error at the assigning line.
                v => opt_inst(v).ok_or_else(|| {
                    crate::cmeta::located(format!(
                        "invalid argument #3 (Instance expected, got {})",
                        crate::cmeta::type_name(v)
                    ))
                })?,
            };
            return set_parent(lua, r, p);
        }
        "ClassName" => {
            return Err(rt_err(
                "Unable to assign property ClassName. Property is read only",
            ))
        }
        "OnInvoke" | "OnServerInvoke" | "OnClientInvoke" => {
            return set_callback(lua, r, key, value)
        }
        // Script.Enabled is Disabled turned round: places save Disabled, and
        // that is what decides whether it runs. Place B Newer's
        // MusicSystemInitializer parents ModernMusicGui and then sets its
        // Handler's Enabled (saved Disabled); rustudio kept that in a stub
        // and the Handler never ran.
        "Enabled" if matches!(class, "Script" | "LocalScript") => {
            let on = !matches!(value, Value::Nil | Value::Boolean(false));
            store_prop(lua, r, "Disabled", Variant::Bool(!on))?;
            {
                let mut w = world(lua)?;
                if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
                    i.properties.remove(&ustr("Enabled"));
                }
                w.fire(r, "Changed", vec![Value::String(lua.create_string("Enabled")?)]);
                w.fire(r, "Changed:Enabled", vec![]);
            }
            if on {
                start_if_enabled(lua, r)?;
            }
            return Ok(());
        }
        _ => {}
    }
    // Attachment world placement: stored as CFrame relative to the part (or
    // as given while it isn't on a part, like Roblox).
    if rt.set_attachment {
        let (local, part) = attachment_frames(lua, r)?;
        let worldf = compose(&part, &local);
        let new_local = match key {
            "WorldCFrame" => {
                let cf =
                    opt_ud::<LCFrame>(&value).ok_or_else(|| bad_assign(key, "CFrame", &value))?;
                compose(&part.inverse(), &cf.0)
            }
            _ => {
                let v = opt_ud::<LVector3>(&value)
                    .ok_or_else(|| bad_assign(key, "Vector3", &value))?
                    .0;
                match key {
                    "WorldPosition" => compose(
                        &part.inverse(),
                        &Xform {
                            pos: v,
                            rot: worldf.rot,
                        },
                    ),
                    "WorldOrientation" => {
                        compose(&part.inverse(), &Xform::from_orientation_deg(worldf.pos, v))
                    }
                    "Position" => Xform {
                        pos: v,
                        rot: local.rot,
                    },
                    _ => Xform::from_orientation_deg(local.pos, v),
                }
            }
        };
        store_prop(lua, r, "CFrame", Variant::CFrame(new_local.to_cframe()))?;
        let pname = Value::String(lua.create_string(key)?);
        let mut w = world(lua)?;
        w.fire(r, "Changed", vec![pname]);
        w.fire(r, &format!("Changed:{key}"), vec![]);
        return Ok(());
    }
    if rt.set_part {
        match key {
            "Position" | "Orientation" | "Rotation" => {
                let v =
                    opt_ud::<LVector3>(&value).ok_or_else(|| bad_assign(key, "Vector3", &value))?;
                let xf = part_xform(lua, r)?;
                let nx = if key == "Position" {
                    Xform {
                        pos: v.0,
                        rot: xf.rot,
                    }
                } else {
                    Xform::from_orientation_deg(xf.pos, v.0)
                };
                store_prop(lua, r, "CFrame", Variant::CFrame(nx.to_cframe()))?;
                let pname = Value::String(lua.create_string(key)?);
                let mut w = world(lua)?;
                w.fire(r, "Changed", vec![pname]);
                w.fire(r, &format!("Changed:{key}"), vec![]);
                return Ok(());
            }
            "BrickColor" => {
                let b = opt_ud::<LBrickColor>(&value)
                    .ok_or_else(|| bad_assign(key, "BrickColor", &value))?;
                return store_prop(lua, r, "Color", Variant::Color3uint8(b.0.to_color3uint8()));
            }
            "AssemblyLinearVelocity" | "Velocity" => {
                let v =
                    opt_ud::<LVector3>(&value).ok_or_else(|| bad_assign(key, "Vector3", &value))?;
                world(lua)?.velocity_requests.push((r, v.0));
                return Ok(());
            }
            _ => {}
        }
    }
    let Some(info) = rt.set else {
        if let Some(cap) = capitalized(key)
            .filter(|c| prop_desc(class, c).is_some() || c == "Parent" || c == "Name")
        {
            return newindex(lua, r, &cap, value);
        }
        if stubs::assign(lua, r, class, key, &value)? {
            return Ok(());
        }
        crate::telemetry::gap(lua, "missing", class, key);
        let full = world(lua)?.full_name(r);
        return Err(rt_err(format!(
            "{key} is not a valid member of {class} \"{full}\""
        )));
    };
    let canon = info.canon.as_str();
    if info.read_only {
        return Err(rt_err(format!(
            "Unable to assign property {key}. Property is read only"
        )));
    }
    if !info.acts {
        crate::telemetry::plain_write(lua, class, canon);
    }
    let Some((ty, enum_name)) = info.ty else {
        return Err(rt_err(format!("Unable to assign property {key}")));
    };
    // (Only a Color3 conversion looks at what is stored now.)
    let existing = if matches!(ty, VariantType::Color3 | VariantType::Color3uint8) {
        world(lua)?.d().prop_key(r, info.canon)
    } else {
        None
    };
    let mut v = lua_to_variant(key, &value, ty, enum_name, existing.as_ref())?;
    // Studio's own setters clamp these (mech::setter_clamp, with addresses).
    if let Variant::Float32(f) = v {
        if let Some(c) = mech::setter_clamp(class, canon, f) {
            v = Variant::Float32(c);
        }
    }
    if class == "Humanoid" && canon == "Health" {
        let max = match world(lua)?.d().prop(r, "MaxHealth") {
            Some(Variant::Float32(m)) => m,
            _ => 100.0,
        };
        let old = match existing {
            Some(Variant::Float32(h)) => h,
            _ => 100.0,
        };
        let new = match v {
            Variant::Float32(h) => h.clamp(0.0, max),
            _ => old,
        };
        store_prop(lua, r, canon, Variant::Float32(new))?;
        let mut w = world(lua)?;
        w.fire(r, "HealthChanged", vec![Value::Number(new as f64)]);
        if new <= 0.0 && old > 0.0 {
            w.fire(r, "Died", vec![]);
            w.died.push(r);
        }
        return Ok(());
    }
    let enabling = canon == "Disabled"
        && matches!(v, Variant::Bool(false))
        && matches!(class, "Script" | "LocalScript");
    store_prop_key(lua, r, info.canon, v)?;
    if enabling {
        start_if_enabled(lua, r)?;
    }
    Ok(())
}

/// Whether a script is enabled: not Disabled (and no saved Enabled false).
fn script_enabled_here(lua: &Lua, r: Ref) -> mlua::Result<bool> {
    let w = world(lua)?;
    let d = w.d();
    Ok(!matches!(d.prop(r, "Disabled"), Some(Variant::Bool(true)))
        && !matches!(d.prop(r, "Enabled"), Some(Variant::Bool(false))))
}

/// A script that hasn't run starts (next dispatch) when it's enabled where
/// it runs, as in Roblox.
fn start_if_enabled(lua: &Lua, r: Ref) -> mlua::Result<()> {
    let mut w = world(lua)?;
    if !w.started.contains(&r) && w.runs_here(r) {
        w.to_start.push(r);
    }
    Ok(())
}

// ---------------------------------------------------------------- methods

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

fn arg_str(args: &MultiValue, i: usize, what: &str) -> mlua::Result<String> {
    match arg(args, i) {
        // Studio's reflected string argument converts what it can
        // (GetService(true) is `'true' is not a valid Service name`,
        // GetService({}) `'{}' …`); only nil is missing.
        Value::String(s) => Ok(s.to_str()?.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Integer(n) => Ok(n.to_string()),
        Value::Boolean(b) => Ok(b.to_string()),
        Value::Nil => Err(rt_err(format!("Argument {i} missing or nil"))),
        Value::Table(t) if t.is_empty() => Ok("{}".into()),
        v => {
            let _ = what;
            Err(rt_err(format!("Unable to cast {} to string", crate::cmeta::type_name(&v))))
        }
    }
}

fn children_of(lua: &Lua, r: Ref) -> mlua::Result<Vec<Ref>> {
    Ok(world(lua)?
        .d()
        .dom
        .get_by_ref(r)
        .map(|i| i.children().to_vec())
        .unwrap_or_default())
}

fn list(lua: &Lua, refs: Vec<Ref>) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(refs.len(), 0)?;
    for (i, r) in refs.into_iter().enumerate() {
        t.set(i + 1, inst_value(lua, r)?)?;
    }
    Ok(t)
}

fn find_child(
    lua: &Lua,
    r: Ref,
    pred: impl Fn(&rbx_dom_weak::Instance) -> bool,
    recursive: bool,
) -> mlua::Result<Option<Ref>> {
    let w = world(lua)?;
    let d = w.d();
    Ok(if recursive {
        d.dom
            .descendants_of(r)
            .filter(|i| i.referent() != r)
            .find(|i| pred(i))
            .map(|i| i.referent())
    } else {
        d.dom.get_by_ref(r).and_then(|i| {
            i.children()
                .iter()
                .copied()
                .find(|c| d.dom.get_by_ref(*c).is_some_and(&pred))
        })
    })
}

/// Children by name for parents with many children (Place B's Workspace:
/// FindFirstChild scanned thousands per call). Kept from the document's
/// journal: a child added, moved in or renamed is added to its parent's
/// lists; one that left is filtered out when looked up.
/// Open debug.profilebegin markers per thread (by pointer), for the current
/// resumption only: Studio's profiler stack doesn't survive a yield.
#[derive(Default)]
struct ProfileDepth(HashMap<usize, u32>);

#[derive(Default)]
struct ChildIndex {
    doc_id: u64,
    /// Parent → name → children that had that name there (maybe stale).
    by_parent: HashMap<Ref, HashMap<String, Vec<Ref>>>,
}

/// Parents with fewer children are just scanned.
const CHILD_INDEX_MIN: usize = 64;

/// `parent:FindFirstChild(name)`: the first child with that name.
fn find_named_child(lua: &Lua, r: Ref, name: &str) -> mlua::Result<Option<Ref>> {
    let w = world(lua)?;
    let d = w.d();
    let Some(i) = d.dom.get_by_ref(r) else {
        return Ok(None);
    };
    if i.children().len() < CHILD_INDEX_MIN {
        return Ok(i
            .children()
            .iter()
            .copied()
            .find(|c| d.dom.get_by_ref(*c).is_some_and(|c| c.name == name)));
    }
    drop(w);
    let mut w = world(lua)?;
    let marks = w.dm().marks_for(doc::Reader::Children);
    let d = w.d();
    let mut ix = lua
        .app_data_mut::<ChildIndex>()
        .ok_or_else(|| rt_err("child index missing"))?;
    match marks {
        doc::Marks::Some(m) if ix.doc_id == d.id => {
            for mark in m {
                let c = match mark {
                    doc::Mark::Tree(c) => c,
                    doc::Mark::Prop(c, n) if n == "Name" => c,
                    _ => continue,
                };
                let Some(ci) = d.dom.get_by_ref(c) else { continue };
                if let Some(names) = ix.by_parent.get_mut(&ci.parent()) {
                    let list = names.entry(ci.name.clone()).or_default();
                    if !list.contains(&c) {
                        list.push(c);
                    }
                }
            }
        }
        doc::Marks::Some(_) => {}
        doc::Marks::All => ix.by_parent.clear(),
    }
    if ix.doc_id != d.id {
        ix.doc_id = d.id;
        ix.by_parent.clear();
    }
    let Some(i) = d.dom.get_by_ref(r) else {
        return Ok(None);
    };
    let names = ix.by_parent.entry(r).or_insert_with(|| {
        let mut m: HashMap<String, Vec<Ref>> = HashMap::with_capacity(i.children().len());
        for c in i.children() {
            if let Some(ci) = d.dom.get_by_ref(*c) {
                m.entry(ci.name.clone()).or_default().push(*c);
            }
        }
        m
    });
    let Some(list) = names.get_mut(name) else {
        return Ok(None);
    };
    // Still here under that name; with several, the first in child order.
    list.retain(|c| d.dom.get_by_ref(*c).is_some_and(|ci| ci.parent() == r && ci.name == name));
    Ok(match list.len() {
        0 => None,
        1 => Some(list[0]),
        _ => i.children().iter().copied().find(|c| list.contains(c)),
    })
}

fn find_ancestor(
    lua: &Lua,
    r: Ref,
    pred: impl Fn(&rbx_dom_weak::Instance) -> bool,
) -> mlua::Result<Option<Ref>> {
    let w = world(lua)?;
    let d = w.d();
    let mut cur = d
        .dom
        .get_by_ref(r)
        .map(|i| i.parent())
        .unwrap_or_else(Ref::none);
    while let Some(i) = d.dom.get_by_ref(cur) {
        if pred(i) {
            return Ok(Some(cur));
        }
        cur = i.parent();
    }
    Ok(None)
}

fn opt_ref_value(lua: &Lua, r: Option<Ref>) -> mlua::Result<Value> {
    match r {
        Some(r) => inst_value(lua, r),
        None => Ok(Value::Nil),
    }
}

fn get_service(lua: &Lua, class: &str, create: bool) -> mlua::Result<Option<Ref>> {
    // Studio-only services: GetService / FindService give game scripts nil
    // (measured in Studio, checks/parity/testing.md).
    if matches!(
        class,
        "DebuggerManager" | "StudioService" | "DraggerService" | "LiveSyncService" | "TestCoordinatorService"
    ) {
        return Ok(None);
    }
    let mut w = world(lua)?;
    let root = w.d().dom.root_ref();
    if let Some(r) = w.d().find_child(root, class) {
        return Ok(Some(r));
    }
    // Studio 0.744's classes (its API dump): a name it doesn't have is not a
    // valid Service name (FeatureMatchingService, measured), whatever the
    // reflection database (another version) says.
    let api_class = crate::api::get().map(|a| a.classes.get(class));
    if matches!(api_class, Some(None)) {
        return Err(rt_err(format!("'{class}' is not a valid Service name")));
    }
    if !doc::is_service(class) {
        // Any other class answers nil (Studio probes 6, every library row:
        // AnimationTrack, WorldRoot, Terrain, RealtimeMedia…); one that can
        // be created is created first, so Player wants WritePlayer
        // (library.Player.Move).
        let known = doc::db().classes.contains_key(class) || matches!(api_class, Some(Some(_)));
        if !known {
            return Err(rt_err(format!("'{class}' is not a valid Service name")));
        }
        if doc::is_script_creatable(class) {
            drop(w);
            security::check_create(lua, class)?;
        }
        return Ok(None);
    }
    if !create {
        return Ok(None);
    }
    let r = w
        .dm()
        .dom
        .insert(root, InstanceBuilder::new(class).with_name(class));
    w.dm().mark_tree(r);
    w.structure_changed = true;
    Ok(Some(r))
}

/// A model's pivot: its PrimaryPart, else the centre of its parts.
/// When this process started (os.clock's origin, as in Studio).
pub fn process_start() -> Instant {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *START.get_or_init(Instant::now)
}

fn model_pivot(w: &World, r: Ref) -> Xform {
    // Roblox: the PrimaryPart's pivot (CFrame · PivotOffset), else the
    // model's own WorldPivotData, else the centre of its parts' bounds.
    let d = w.d();
    if let Some(Variant::Ref(pp)) = d.prop(r, "PrimaryPart") {
        if let Some(Variant::CFrame(cf)) = d.prop(pp, "CFrame") {
            let off = match d.prop(pp, "PivotOffset") {
                Some(Variant::CFrame(o)) => Xform::from_cframe(&o),
                _ => Xform {
                    pos: Vec3::ZERO,
                    rot: glam::Mat3::IDENTITY,
                },
            };
            return compose(&Xform::from_cframe(&cf), &off);
        }
    }
    if let Some(Variant::OptionalCFrame(Some(cf))) = d
        .dom
        .get_by_ref(r)
        .and_then(|i| i.properties.get(&ustr("WorldPivotData")).cloned())
    {
        return Xform::from_cframe(&cf);
    }
    // Studio's fallback: the centre
    // of the parts' boxes (each part's half size through |R|), measured in
    // the world frame when there is no PrimaryPart (identity rotation).
    let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut any = false;
    for i in d.dom.descendants_of(r) {
        if doc::is_a(&i.class, "BasePart") {
            if let Some(Variant::CFrame(cf)) = d.prop(i.referent(), "CFrame") {
                let xf = Xform::from_cframe(&cf);
                let half = match d.prop(i.referent(), "Size") {
                    Some(Variant::Vector3(s)) => Vec3::new(s.x, s.y, s.z) * 0.5,
                    _ => Vec3::ZERO,
                };
                let m = xf.rot;
                let ext = Vec3::new(
                    m.x_axis.x.abs() * half.x + m.y_axis.x.abs() * half.y + m.z_axis.x.abs() * half.z,
                    m.x_axis.y.abs() * half.x + m.y_axis.y.abs() * half.y + m.z_axis.y.abs() * half.z,
                    m.x_axis.z.abs() * half.x + m.y_axis.z.abs() * half.y + m.z_axis.z.abs() * half.z,
                );
                mn = mn.min(xf.pos - ext);
                mx = mx.max(xf.pos + ext);
                any = true;
            }
        }
    }
    Xform {
        pos: if any { (mn + mx) * 0.5 } else { Vec3::ZERO },
        rot: glam::Mat3::IDENTITY,
    }
}

fn pivot_to(lua: &Lua, r: Ref, target: Xform) -> mlua::Result<()> {
    let (class, _, _) = inst_meta(lua, r)?;
    if doc::is_a(&class, "BasePart") {
        return store_prop(lua, r, "CFrame", Variant::CFrame(target.to_cframe()));
    }
    let (pivot, parts) = {
        let w = world(lua)?;
        let pivot = model_pivot(&w, r);
        let parts: Vec<(Ref, Xform)> = w
            .d()
            .dom
            .descendants_of(r)
            .filter(|i| doc::is_a(&i.class, "BasePart"))
            .filter_map(|i| match w.d().prop(i.referent(), "CFrame") {
                Some(Variant::CFrame(cf)) => Some((i.referent(), Xform::from_cframe(&cf))),
                _ => None,
            })
            .collect();
        (pivot, parts)
    };
    let delta = cf_mul(&target, &cf_inv(&pivot));
    for (p, xf) in parts {
        store_prop(
            lua,
            p,
            "CFrame",
            Variant::CFrame(cf_mul(&delta, &xf).orthonormalized().to_cframe()),
        )?;
    }
    Ok(())
}

/// Instance:Destroy(), as Studio's `Instance::destroy`: the
/// instance's Parent goes nil (and locks), then each descendant's in turn,
/// a parent before its children, then the connections go. Studio walks the
/// tree one `setParent(nil)` at a time; once the root is out of the
/// DataModel each of those only notifies the (destroyed) parent, so here
/// the subtree is cut loose in one pass and the per-descendant events are
/// made only for instances something listens to (round 2, bulk: a 20k-part
/// model took 111-117 ms, Studio 23).
fn destroy_inst(lua: &Lua, r: Ref) -> mlua::Result<()> {
    if r == world(lua)?.d().dom.root_ref() {
        return Err(rt_err("game cannot be destroyed"));
    }
    // Destroying what is already destroyed (a descendant of a destroyed
    // instance, as Place D's preview strip loop over GetDescendants does) is a
    // no-op in Roblox, not the locked-Parent error.
    if world(lua)?.destroyed.contains(&r) {
        return Ok(());
    }
    {
        let mut w = world(lua)?;
        w.fire(r, "Destroying", vec![]);
        // One replicated op for the subtree: Studio's server sends one
        // DestroyInstance item and the client runs Destroy on its copy
        // (DeserializedDestroyInstanceItem),
        // not an unparent per descendant. Recorded before the Parent op so
        // the client still has the pair when it gets here.
        w.record(RepOp::Destroy(r));
    }
    set_parent(lua, r, Ref::none())?;
    let order = detach_descendants(lua, r)?;
    // Connections go only when something in the subtree has any (a walk of
    // every connection in the DataModel otherwise: 125 us per Destroy in
    // Place E, 1.6 ms in a live Place A match, whose effect parts and
    // tracers have none).
    let listened: Vec<Ref> = {
        let mut w = world(lua)?;
        w.destroyed.insert(r);
        w.destroyed.extend(order.iter().map(|x| x.0));
        std::iter::once(r)
            .chain(order.iter().map(|x| x.0))
            .filter(|x| w.listening.contains(x))
            .collect()
    };
    if !listened.is_empty() {
        let mut s = signals(lua)?;
        let Signals { conns, conn_names, .. } = &mut *s;
        for x in &listened {
            let Some(names) = conn_names.get_mut(x) else { continue };
            for name in names.drain(..).collect::<Vec<_>>() {
                if name == "Destroying" {
                    names.push(name);
                } else {
                    conns.remove(&(*x, name));
                }
            }
            if names.is_empty() {
                conn_names.remove(x);
            }
        }
    }
    Ok(())
}

/// Cuts every descendant of the (already unparented) `r` loose, as
/// Studio's destroy does one `setParent(nil)` at a time, parent before
/// children: each one's ChildRemoved / DescendantRemoving go to its old
/// parent only (already out of the tree), its Changed("Parent") and the
/// AncestryChanged of it and everything under it to themselves. Fires are
/// made only where someone listens (or, for Changed, game.ItemChanged
/// does). Returns the descendants in pre-order with their old parents.
fn detach_descendants(lua: &Lua, r: Ref) -> mlua::Result<Vec<(Ref, Ref)>> {
    let (order, listens, item_changed) = {
        let mut w = world(lua)?;
        let order = w.dm().dom.detach_descendants(r);
        for (d, _) in &order {
            w.orphans.insert(*d);
            // Each one is a root of its own now: readers (scene, physics,
            // GUI) see it leave through its own mark.
            w.dm().mark_tree(*d);
        }
        if !order.is_empty() {
            w.structure_changed = true;
        }
        let listens: Vec<bool> = order
            .iter()
            .map(|(d, p)| w.listening.contains(d) || w.listening.contains(p))
            .collect();
        let item_changed = w.listening.contains(&w.d().dom.root_ref());
        (order, listens, item_changed)
    };
    if !item_changed && !listens.iter().any(|x| *x) {
        return Ok(order);
    }
    // Pre-order: each subtree is a contiguous run order[i..end[i]].
    let n = order.len();
    let mut end = vec![n; n];
    {
        let mut stack: Vec<usize> = Vec::new();
        for (i, (_, p)) in order.iter().enumerate() {
            while let Some(&top) = stack.last() {
                if order[top].0 == *p {
                    break;
                }
                end[top] = i;
                stack.pop();
            }
            stack.push(i);
        }
    }
    // Listened instances before index i, to skip quiet subtrees fast.
    let mut heard = vec![0usize; n + 1];
    for i in 0..n {
        heard[i + 1] = heard[i] + usize::from(world(lua)?.listening.contains(&order[i].0));
    }
    let pname = Value::String(lua.create_string("Parent")?);
    let mut values: HashMap<Ref, Value> = HashMap::new();
    let mut value_of = |d: Ref| -> mlua::Result<Value> {
        if let Some(v) = values.get(&d) {
            return Ok(v.clone());
        }
        let v = inst_value(lua, d)?;
        values.insert(d, v.clone());
        Ok(v)
    };
    for i in 0..n {
        let (d, p) = order[i];
        if !listens[i] && !item_changed && heard[end[i]] == heard[i] {
            continue;
        }
        let (p_hears, d_hears) = {
            let w = world(lua)?;
            (w.listening.contains(&p), w.listening.contains(&d))
        };
        if p_hears {
            let me = value_of(d)?;
            let subtree: Vec<Value> = (i..end[i]).map(|j| value_of(order[j].0)).collect::<mlua::Result<_>>()?;
            let mut w = world(lua)?;
            w.fire(p, "ChildRemoved", vec![me]);
            for v in subtree {
                w.fire(p, "DescendantRemoving", vec![v]);
            }
        }
        let mut w = world(lua)?;
        if d_hears || item_changed {
            w.fire(d, "Changed", vec![pname.clone()]);
            w.fire(d, "Changed:Parent", vec![]);
        }
        if heard[end[i]] > heard[i] {
            for j in i..end[i] {
                let e = order[j].0;
                if w.listening.contains(&e) {
                    w.fire(e, "AncestryChanged", vec![]);
                }
            }
        }
    }
    Ok(order)
}

fn part_size(lua: &Lua, r: Ref) -> mlua::Result<Vec3> {
    Ok(match world(lua)?.d().prop(r, "Size") {
        Some(Variant::Vector3(v)) => Vec3::new(v.x, v.y, v.z),
        _ => Vec3::ONE,
    })
}

/// Studio's attribute-name check: not empty, at most 100
/// bytes (0x64), and only ASCII letters, digits, '_', '-', '.' and '/'.
fn check_attribute_name(n: &str) -> mlua::Result<()> {
    if n.is_empty() {
        return Err(rt_err("Attribute name cannot be empty"));
    }
    if n.len() > mech::ATTRIBUTE_NAME_MAX {
        return Err(rt_err(format!(
            "Attribute name exceeds {} character limit",
            mech::ATTRIBUTE_NAME_MAX
        )));
    }
    if let Some(c) = n
        .bytes()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'/')))
    {
        return Err(rt_err(format!(
            "Attribute name contains illegal character '{}'",
            c as char
        )));
    }
    Ok(())
}

fn attribute_value(v: &Value) -> mlua::Result<Option<Variant>> {
    Ok(match v {
        Value::Nil => None,
        Value::Boolean(b) => Some(Variant::Bool(*b)),
        Value::Number(x) => Some(Variant::Float64(*x)),
        Value::Integer(i) => Some(Variant::Float64(*i as f64)),
        Value::String(s) => Some(Variant::String(s.to_str()?.to_string())),
        _ => {
            if let Some(e) = opt_ud::<LEnumItem>(v) {
                Some(Variant::EnumItem(rbx_dom_weak::types::EnumItem {
                    ty: e.enum_type.clone(),
                    value: e.value,
                }))
            } else if let Some(p) = opt_ud::<LVector2>(v) {
                Some(Variant::Vector2(Vector2::new(p.0.x, p.0.y)))
            } else if let Some(b) = opt_ud::<LBrickColor>(v) {
                Some(Variant::BrickColor(b.0))
            } else if let Some(u) = opt_ud::<LUDim>(v) {
                Some(Variant::UDim(UDim::new(u.scale, u.offset)))
            } else if let Some(u) = opt_ud::<LUDim2>(v) {
                Some(Variant::UDim2(UDim2::new(
                    UDim::new(u.x.scale, u.x.offset),
                    UDim::new(u.y.scale, u.y.offset),
                )))
            } else if let Some(f) = opt_ud::<LFont>(v) {
                Some(Variant::Font(f.0))
            } else if let Some(x) = crate::ltypes::datatype_to_variant(v) {
                Some(x)
            } else if let Some(p) = opt_ud::<LVector3>(v) {
                Some(Variant::Vector3(Vector3::new(p.0.x, p.0.y, p.0.z)))
            } else if let Some(c) = opt_ud::<LColor3>(v) {
                Some(Variant::Color3(Color3::new(c.0[0], c.0[1], c.0[2])))
            } else if let Some(c) = opt_ud::<LCFrame>(v) {
                Some(Variant::CFrame(c.0.to_cframe()))
            } else {
                return Err(rt_err(format!(
                    "{} is not a supported attribute type",
                    v.type_name()
                )));
            }
        }
    })
}

fn call_method(lua: &Lua, name: &str, args: MultiValue) -> mlua::Result<MultiValue> {
    let this = opt_inst(&arg(&args, 0)).ok_or_else(|| {
        rt_err(format!(
            "Expected ':' not '.' calling member function {name}"
        ))
    })?;
    let class_u = inst_class(lua, this)?.0;
    let m = routes::method_route(class_u, name);
    call_method_routed(lua, name, args, this, class_u, &m)
}

/// `call_method` with the method's route worked out (runtime_routes.rs).
fn call_method_routed(
    lua: &Lua,
    name: &str,
    args: MultiValue,
    this: Ref,
    class_u: rbx_dom_weak::Ustr,
    m: &routes::MethodRoute,
) -> mlua::Result<MultiValue> {
    // Same name, different class (Team:GetPlayers, Humanoid:MoveTo…): the extension handles it.
    let (core, extended) = (m.core, m.extended);
    let class: &str = class_u.as_str();
    // A method taken from one class and called on an instance that hasn't
    // it fails as in Roblox (an admin script's anti-kick check calls
    // `Player.Kick(workspace, …)` and expects exactly this message).
    if !core && !extended {
        return Err(rt_err(format!(
            "Expected ':' not '.' calling member function {name}"
        )));
    }
    // Domains (src/domains) may take over a core method too.
    if core && (m.domains != 0 || domains::route_check_on()) {
        if let Some(r) = domains::call_masked_check(lua, name, this, &args, m.domains, m.shadowed) {
            return r;
        }
    }
    if !core && extended {
        return Some(ext::call_routed(lua, name, this, &args, m.domains))
            .unwrap_or_else(|| stubs::unimplemented_call(lua, this, class, name));
    }
    let one = |v: Value| -> mlua::Result<MultiValue> { Ok(MultiValue::from_iter([v])) };
    let none = || -> mlua::Result<MultiValue> { Ok(MultiValue::new()) };
    match name {
        "GetAxis" | "GetSecondaryAxis" => {
            let key = if name == "GetAxis" { "Axis" } else { "SecondaryAxis" };
            one(prop_get(lua, this, &class, key)?.unwrap_or(Value::Nil))
        }
        "FindFirstChild" => {
            let n = arg_str(&args, 1, "string")?;
            let rec = matches!(arg(&args, 2), Value::Boolean(true));
            let r = if rec {
                find_child(lua, this, |i| i.name == n, rec)?
            } else {
                find_named_child(lua, this, &n)?
            };
            one(opt_ref_value(lua, r)?)
        }
        "FindFirstChildOfClass" => {
            let c = arg_str(&args, 1, "string")?;
            let r = find_child(lua, this, |i| i.class == c.as_str(), false)?;
            one(opt_ref_value(lua, r)?)
        }
        "FindFirstChildWhichIsA" => {
            let c = arg_str(&args, 1, "string")?;
            let rec = matches!(arg(&args, 2), Value::Boolean(true));
            let r = find_child(lua, this, |i| doc::is_a(&i.class, &c), rec)?;
            one(opt_ref_value(lua, r)?)
        }
        "FindFirstDescendant" => {
            let n = arg_str(&args, 1, "string")?;
            let r = find_child(lua, this, |i| i.name == n, true)?;
            one(opt_ref_value(lua, r)?)
        }
        "FindFirstAncestor" => {
            let n = arg_str(&args, 1, "string")?;
            let r = find_ancestor(lua, this, |i| i.name == n)?;
            one(opt_ref_value(lua, r)?)
        }
        "FindFirstAncestorOfClass" => {
            let c = arg_str(&args, 1, "string")?;
            let r = find_ancestor(lua, this, |i| i.class == c.as_str())?;
            one(opt_ref_value(lua, r)?)
        }
        "FindFirstAncestorWhichIsA" => {
            let c = arg_str(&args, 1, "string")?;
            let r = find_ancestor(lua, this, |i| doc::is_a(&i.class, &c))?;
            one(opt_ref_value(lua, r)?)
        }
        "GetChildren" => {
            let kids = children_of(lua, this)?;
            one(Value::Table(list(lua, kids)?))
        }
        "GetDescendants" => {
            let refs: Vec<Ref> = world(lua)?
                .d()
                .dom
                .descendants_of(this)
                .map(|i| i.referent())
                .filter(|r| *r != this)
                .collect();
            one(Value::Table(list(lua, refs)?))
        }
        "IsA" => {
            let c = arg_str(&args, 1, "string")?;
            one(Value::Boolean(doc::is_a(&class, &c)))
        }
        "IsDescendantOf" => {
            let other = opt_inst(&arg(&args, 1));
            let w = world(lua)?;
            one(Value::Boolean(
                other.is_some_and(|o| w.d().is_ancestor(o, this)),
            ))
        }
        "IsAncestorOf" => {
            let other = opt_inst(&arg(&args, 1));
            let w = world(lua)?;
            one(Value::Boolean(
                other.is_some_and(|o| w.d().is_ancestor(this, o)),
            ))
        }
        "Clone" => {
            if matches!(
                world(lua)?.d().prop(this, "Archivable"),
                Some(Variant::Bool(false))
            ) {
                return one(Value::Nil);
            }
            // clone_within leaves the copy unparented, like Instance:Clone().
            let r = world(lua)?.dm().dom.clone_within(this);
            domains::assets_cloud::scene::track_instance(lua,r)?;
            // Descendants with Archivable = false aren't copied (Roblox).
            {
                let mut w = world(lua)?;
                let skip: Vec<Ref> = w
                    .d()
                    .dom
                    .descendants_of(r)
                    .filter(|i| {
                        i.referent() != r
                            && matches!(
                                i.properties.get(&ustr("Archivable")),
                                Some(Variant::Bool(false))
                            )
                    })
                    .map(|i| i.referent())
                    .collect();
                for x in skip {
                    if w.d().dom.get_by_ref(x).is_some() {
                        w.dm().dom.destroy(x);
                    }
                }
                w.structure_changed = true;
            }
            one(inst_value(lua, r)?)
        }
        "Destroy" => {
            destroy_inst(lua, this)?;
            none()
        }
        "Remove" => {
            set_parent(lua, this, Ref::none())?;
            none()
        }
        "ClearAllChildren" => {
            // Studio's Instance::clearAllChildren destroys the
            // last child until none is left: ChildRemoved comes newest first,
            // and each child leaves from the end of the list (no shift).
            for c in children_of(lua, this)?.into_iter().rev() {
                destroy_inst(lua, c)?;
            }
            none()
        }
        "GetFullName" => {
            let s = world(lua)?.full_name(this);
            one(Value::String(lua.create_string(&s)?))
        }
        "GetAttribute" => {
            let n = arg_str(&args, 1, "string")?;
            // Borrowed: only the one value is cloned (round 27: cloning the
            // whole map per call made GetAttribute 3.5 µs in Place A's nav build).
            let v = {
                let w = world(lua)?;
                let d = w.d();
                match d.dom.get_by_ref(this).and_then(|i| i.properties.get(&ustr("Attributes"))) {
                    Some(Variant::Attributes(a)) => a.get(n.as_str()).cloned(),
                    _ => None,
                }
            };
            one(match v {
                Some(v) => variant_to_lua(lua, &v, None)?,
                None => Value::Nil,
            })
        }
        "GetAttributes" => {
            let t = lua.create_table()?;
            if let Some(Variant::Attributes(a)) = world(lua)?.d().prop(this, "Attributes") {
                for (k, v) in a.iter() {
                    t.set(k.as_str(), variant_to_lua(lua, v, None)?)?;
                }
            }
            one(Value::Table(t))
        }
        "SetAttribute" => {
            let n = arg_str(&args, 1, "string")?;
            check_attribute_name(&n)?;
            let var = attribute_value(&arg(&args, 2))?;
            let old = match world(lua)?.d().prop(this, "Attributes") {
                Some(Variant::Attributes(a)) => a,
                _ => Attributes::new(),
            };
            // The value it already has: nothing changes and nothing fires
            // (Roblox fires AttributeChanged only on a change).
            let current = old.iter().find(|(k, _)| k.as_str() == n).map(|(_, v)| v);
            if current == var.as_ref() {
                return none();
            }
            // Rebuild without the key, then add the new value (nil clears it).
            let mut attrs = Attributes::new();
            for (k, v) in old.iter() {
                if k.as_str() != n {
                    attrs.insert(k.clone(), v.clone());
                }
            }
            if let Some(var) = var {
                attrs.insert(n.clone(), var);
            }
            store_prop(lua, this, "Attributes", Variant::Attributes(attrs))?;
            let an = Value::String(lua.create_string(&n)?);
            let mut w = world(lua)?;
            w.fire(this, "AttributeChanged", vec![an]);
            w.fire(this, &format!("AttributeChanged:{n}"), vec![]);
            none()
        }
        "GetPropertyChangedSignal" => {
            let p = arg_str(&args, 1, "string")?;
            one(signal(lua, this, &format!("Changed:{p}"))?)
        }
        "GetAttributeChangedSignal" => {
            let p = arg_str(&args, 1, "string")?;
            one(signal(lua, this, &format!("AttributeChanged:{p}"))?)
        }
        "GetService" | "FindService" => {
            let c = arg_str(&args, 1, "string")?;
            if security::service_hidden(lua, &c) {
                return one(Value::Nil);
            }
            let r = get_service(lua, &c, name == "GetService")?;
            one(opt_ref_value(lua, r)?)
        }
        "GetPivot" | "GetPrimaryPartCFrame" => {
            let (class, _, _) = inst_meta(lua, this)?;
            let xf = if doc::is_a(&class, "BasePart") {
                part_xform(lua, this)?
            } else {
                model_pivot(&*world(lua)?, this)
            };
            one(Value::UserData(lua.create_userdata(LCFrame(xf))?))
        }
        "PivotTo" | "SetPrimaryPartCFrame" => {
            let cf = opt_ud::<LCFrame>(&arg(&args, 1)).ok_or_else(|| rt_err("CFrame expected"))?;
            pivot_to(lua, this, cf.0)?;
            none()
        }
        "MoveTo" => {
            let v = opt_ud::<LVector3>(&arg(&args, 1)).ok_or_else(|| rt_err("Vector3 expected"))?;
            let pivot = model_pivot(&*world(lua)?, this);
            pivot_to(
                lua,
                this,
                Xform {
                    pos: v.0,
                    rot: pivot.rot,
                },
            )?;
            none()
        }
        "GetMass" => {
            let s = part_size(lua, this)?;
            one(Value::Number((s.x * s.y * s.z * mech::PART_DENSITY) as f64))
        }
        "ApplyImpulse" => {
            let imp =
                opt_ud::<LVector3>(&arg(&args, 1)).ok_or_else(|| rt_err("Vector3 expected"))?;
            let s = part_size(lua, this)?;
            let mass = (s.x * s.y * s.z * mech::PART_DENSITY).max(1e-4);
            let mut w = world(lua)?;
            let v = w.velocities.get(&this).copied().unwrap_or(Vec3::ZERO);
            w.velocity_requests.push((this, v + imp.0 / mass));
            none()
        }
        "GetState" => {
            let name = world(lua)?
                .humanoid_states
                .get(&this)
                .cloned()
                .unwrap_or_else(|| "Running".to_string());
            match LEnumItem::from_name("HumanoidStateType", &name) {
                Some(item) => one(crate::ltypes::enum_value(lua, item)?),
                None => one(Value::Nil),
            }
        }
        "TakeDamage" => {
            let dmg = num(&arg(&args, 1)).unwrap_or(0.0);
            let shielded = {
                let w = world(lua)?;
                let d = w.d();
                d.dom
                    .get_by_ref(this)
                    .map(|i| i.parent())
                    .is_some_and(|p| d.find_child(p, "ForceField").is_some())
            };
            if !shielded {
                let h = match world(lua)?.d().prop(this, "Health") {
                    Some(Variant::Float32(h)) => h as f64,
                    _ => 100.0,
                };
                newindex(lua, this, "Health", Value::Number(h - dmg))?;
            }
            none()
        }
        "Fire" => {
            let rest: Vec<Value> = args.iter().skip(1).cloned().collect();
            world(lua)?.fire(this, "Event", rest);
            none()
        }
        "FireServer" => {
            if world(lua)?.side != Side::Client {
                return Err(rt_err("FireServer can only be called from the client"));
            }
            let payload: Vec<Xfer> = args.iter().skip(1).map(|v| export(v, 0)).collect();
            let mut w = world(lua)?;
            if w.has_peer {
                w.outbox.push(RemoteMsg {
                    inst: this,
                    kind: RemoteKind::Event,
                    args: payload,
                });
            }
            none()
        }
        "FireClient" | "FireAllClients" => {
            if world(lua)?.side != Side::Server {
                return Err(rt_err(format!("{name} can only be called from the server")));
            }
            if name == "FireClient" && opt_inst(&arg(&args, 1)).is_none() {
                return Err(rt_err("FireClient: player argument must be a Player"));
            }
            // One client: FireClient(player, …) and FireAllClients(…) both reach it.
            let skip = if name == "FireClient" { 2 } else { 1 };
            let payload: Vec<Xfer> = args.iter().skip(skip).map(|v| export(v, 0)).collect();
            let mut w = world(lua)?;
            if w.has_peer {
                w.outbox.push(RemoteMsg {
                    inst: this,
                    kind: RemoteKind::Event,
                    args: payload,
                });
            }
            none()
        }
        "GetPlayers" => {
            let kids: Vec<Ref> = {
                let w = world(lua)?;
                let d = w.d();
                d.dom
                    .get_by_ref(this)
                    .map(|i| {
                        i.children()
                            .iter()
                            .copied()
                            .filter(|c| d.class_of(*c).is_some_and(|k| k == "Player"))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            one(Value::Table(list(lua, kids)?))
        }
        "GetPlayerFromCharacter" => {
            let ch = opt_inst(&arg(&args, 1));
            let found = {
                let w = world(lua)?;
                let d = w.d();
                ch.and_then(|ch| {
                    d.dom.get_by_ref(this)?.children().iter().copied().find(
                        |p| matches!(d.prop(*p, "Character"), Some(Variant::Ref(c)) if c == ch),
                    )
                })
            };
            one(opt_ref_value(lua, found)?)
        }
        "LoadCharacter" => {
            world(lua)?.load_character.push(this);
            none()
        }
        "IsClient" => one(Value::Boolean(world(lua)?.side == Side::Client)),
        "IsServer" => one(Value::Boolean(world(lua)?.side == Side::Server)),
        "IsStudio" | "IsRunning" => one(Value::Boolean(true)),
        "IsRunMode" => one(Value::Boolean(world(lua)?.mode == RunMode::Run)),
        "IsEdit" => one(Value::Boolean(false)),
        "IsKeyDown" => {
            let k = opt_ud::<LEnumItem>(&arg(&args, 1))
                .ok_or_else(|| rt_err("Enum.KeyCode expected"))?;
            one(Value::Boolean(world(lua)?.keys_down.contains(&k.name)))
        }
        _ => ext::call_routed(lua, name, this, &args, m.domains),
    }
}

// ---------------------------------------------------------------- prelude

/// Lua side of the runtime: everything that yields, plus the signal type.
const PRELUDE: &str = r##"
local rs_wait, rs_wait_signal, rs_output, rs_connect, rs_load_module, rs_index, rs_newindex, WAIT_MIN = ...
local yield, running = coroutine.yield, coroutine.running

-- Engine calls return (ok, ...) and errors are rethrown here without a
-- position, like Roblox's own: pcall gets "X is not a valid member of …"
-- alone (Studio's Place B Newer warns "DataStore unavailable: You must
-- publish …"), and Output puts the script and line in the stack after it.
local function check(ok, ...)
    if ok then return ... end
    error((...), 0)
end
local function inst_index(self, key) return check(rs_index(self, key)) end
local function inst_newindex(self, key, value) check(rs_newindex(self, key, value)) end
local function wrap(f) return function(...) return check(f(...)) end end

-- A signal is a userdata over { __inst, __name } (ltypes LSignal), its
-- methods these.
local Signal = {}
-- Events whose Connect / Wait want a capability (API dump Security /
-- Capabilities, checked for the calling thread: src/security.rs).
local rs_identity, sec_event, sec_names = __rs_identity, __rs_sec_event, __rs_sec_event_names
local function gate(self, verb)
    if sec_names[self.__name] then
        local ok, e = sec_event(self.__inst, self.__name, verb)
        if not ok then error(e, 0) end
    end
end
-- Studio: a non-function still answers a (dead) Connection; the failure
-- goes to Output as an error, not to the caller.
local function noop() end
local function connect(self, fn, once)
    gate(self, "connect")
    if type(fn) ~= "function" then
        rs_output("error", "Attempt to connect failed: Passed value is not a function")
        local c = rs_connect(self.__inst, self.__name, noop, once)
        c:Disconnect()
        return c
    end
    return rs_connect(self.__inst, self.__name, fn, once)
end
function Signal:Connect(fn) return connect(self, fn, false) end
function Signal:Once(fn) return connect(self, fn, true) end
Signal.connect = Signal.Connect
-- The calling script (level 3: past this function and the named C method
-- function, cmeta) must sit under an Actor; checked before anything else.
local getfenv, rawget_, typeof_ = getfenv, rawget, typeof
local function under_actor(level)
    local ok, env = pcall(getfenv, level + 1)
    local s = ok and type(env) == "table" and env.script
    if typeof_(s) ~= "Instance" then return false end
    local ok2, a = pcall(function() return s:FindFirstAncestorWhichIsA("Actor") end)
    return ok2 and a ~= nil
end
function Signal:ConnectParallel(fn)
    if not under_actor(2) then error("Scripts that connect in parallel must be rooted under an Actor.", 0) end
    return connect(self, fn, false)
end
function Signal:Wait()
    gate(self, "wait")
    rs_wait_signal(self.__inst, self.__name, running())
    return yield()
end
Signal.wait = Signal.Wait
function __rs_new_signal(inst, name)
    return __rs_make_signal({ __inst = inst, __name = name }, Signal)
end

function task.wait(t)
    rs_wait(running(), tonumber(t) or 0)
    return yield()
end
function wait(t)
    t = math.max(tonumber(t) or 0, WAIT_MIN)
    rs_wait(running(), t)
    local elapsed = yield()
    return elapsed, time()
end

local function fmt(...)
    local n = select("#", ...)
    local parts = table.create(n)
    for i = 1, n do parts[i] = tostring((select(i, ...))) end
    return table.concat(parts, " ")
end
function print(...)
    local src, line = debug.info(2, "sl")
    rs_output("print", fmt(...), src, line)
end
function warn(...)
    local src, line = debug.info(2, "sl")
    rs_output("warn", fmt(...), src, line)
end

-- One module cache per identity: a command-bar require (identity 1) must
-- not hand game scripts its result, and Studio runs the module again.
local loaded_by, loading_by = {}, {}
local NIL_RESULT = newproxy(false) -- a module that returned nil, cached
local FAILED = newproxy(false) -- a module whose code raised
-- Require by string (Luau's require-by-string, as Studio's own messages
-- read): "./X" is a sibling of the requiring script, "../X" one level up,
-- "@self/X" its child, "@game/Service/X" from the DataModel (estimated); ".." inside a
-- path goes to the parent. Children are found, not waited for.
local function require_path(path, ctx)
    -- Studio's require raises without a position.
    local function fail(why) error(string.format('error requiring "%s": %s', path, why), 0) end
    if typeof(ctx) ~= "Instance" then fail("could not get requiring context") end
    local node, rest
    if path:sub(1, 2) == "./" then
        node, rest = ctx.Parent, path:sub(3)
    elseif path:sub(1, 3) == "../" then
        node, rest = ctx.Parent and ctx.Parent.Parent, path:sub(4)
    elseif path:sub(1, 1) == "@" then
        local alias
        alias, rest = path:match("^@([^/]*)/?(.*)$")
        if alias == "self" then node = ctx
        elseif alias == "game" then node = game
        else fail('could not resolve alias "' .. tostring(alias) .. '"') end
    else
        fail("require path must start with a valid prefix: ./, ../, or @")
    end
    if not node then fail("could not get parent of requiring context") end
    for comp in rest:gmatch("[^/]+") do
        if comp == ".." then
            node = node.Parent
            if not node then fail('could not get parent of component "' .. comp .. '"') end
        elseif comp ~= "." then
            local c = node:FindFirstChild(comp)
            if not c then fail('could not resolve child component "' .. comp .. '"') end
            node = c
        end
    end
    return node
end
local function require_impl(m, script)
    if type(m) == "string" then
        -- The requiring script: the calling function's environment's `script`.
        m = require_path(m, script or __rs_caller_script())
    end
    -- false is asset id 0 (measured); the client may not load assets.
    if type(m) == "boolean" then m = 0 end
    if type(m) == "number" and game:GetService("RunService"):IsClient() then
        error(string.format("require(assetId) cannot be called from a client.  assetId = %d", m), 0)
    end
    if type(m) == "number" then
        -- An asset id: the published model's MainModule, else Studio's
        -- error (offline Studio prints the same for every id).
        local ok, model = pcall(function()
            return game:GetService("InsertService"):LoadAsset(m)
        end)
        local main = ok and model and model:FindFirstChild("MainModule", true)
        if not (main and main:IsA("ModuleScript")) then
            error(string.format('Downloading asset failed for asset id %d.  Is the asset id correct and is the asset type "Model"?', m), 0)
        end
        m = main
    end
    if typeof(m) ~= "Instance" or not m:IsA("ModuleScript") then
        error("Attempted to call require with invalid argument(s).", 0)
    end
    local ident = rs_identity()
    local loaded, loading = loaded_by[ident], loading_by[ident]
    if not loaded then
        loaded, loading = {}, {}
        loaded_by[ident], loading_by[ident] = loaded, loading
    end
    local v = loaded[m]
    if v == FAILED then error("Requested module experienced an error while loading", 0) end
    if v ~= nil then return (if v == NIL_RESULT then nil else v) end
    if loading[m] then
        -- Another script is still loading it (the module yielded): wait for
        -- that, like Roblox. The same thread asking again is a real cycle.
        if loading[m] == running() then error("Requested module was required recursively", 0) end
        while loading[m] do task.wait() end
        v = loaded[m]
        if v ~= nil and v ~= FAILED then return (if v == NIL_RESULT then nil else v) end
        error("Requested module experienced an error while loading", 0)
    end
    loading[m] = running()
    local f = rs_load_module(m)
    local memoryCategory = __ac_scene_mem_begin(m)
    local res = table.pack(pcall(f))
    __ac_scene_mem_restore(memoryCategory)
    loading[m] = nil
    local ok, r = res[1], res[2]
    if not ok then
        -- The module's own error goes to Output; the requirer gets Studio's
        -- generic message, and so does every later require of it.
        loaded[m] = FAILED
        rs_output("error", tostring(r))
        error("Requested module experienced an error while loading", 0)
    end
    -- One value, which may be nil (a types-only module ends `return nil`);
    -- no return at all, or two values, is the error.
    if res.n ~= 2 then error("Module code did not return exactly one value", 0) end
    loaded[m] = if r == nil then NIL_RESULT else r
    return r
end
-- One require for every script, as Studio's: the requiring script for a
-- path is read from the calling function's environment by
-- __rs_caller_script (src/globals_parity.rs), not getfenv, which would clear
-- the caller's safeenv flag and with it Luau's fast paths.
function require(m) return require_impl(m, nil) end

__rs_lua_methods = {}
-- A Lua-written method called on the wrong thing fails as Roblox's own do
-- (an admin script: `RemoteFunction.InvokeServer(workspace)` must say exactly this).
-- typeof only: a script's proxy userdata must not see an __index.
function __rs_member_check(self, class, name)
    if typeof(self) ~= "Instance" or not self:IsA(class) then
        error("Expected ':' not '.' calling member function " .. name, 0)
    end
end
local member = __rs_member_check
function __rs_lua_methods.WaitForChild(self, name, timeout)
    local c = self:FindFirstChild(name)
    if c then return c end
    -- Studio names the instance as it was at the call (Place B's
    -- CharacterAdded handler: 'Player:WaitForChild("Torso")', unparented).
    local start, warned, full = time(), false, self:GetFullName()
    -- Studio's warning names the script line that waits, not this prelude.
    local lvl, src, line = 2, nil, nil
    repeat
        src, line = debug.info(lvl, "sl")
        lvl += 1
    until src == nil or (src ~= "[C]" and not string.find(src, "rustudio.", 1, true))
    while true do
        task.wait()
        c = self:FindFirstChild(name)
        if c then return c end
        if timeout and time() - start >= timeout then return nil end
        if not timeout and not warned and time() - start >= 5 then
            warned = true
            rs_output("warn", "Infinite yield possible on '" .. full .. ":WaitForChild(\"" .. tostring(name) .. "\")'", src, line)
        end
    end
end
function __rs_lua_methods.Invoke(self, ...)
    member(self, "BindableFunction", "Invoke")
    local cb = __rs_get_callback(self, "OnInvoke")
    if not cb then error("OnInvoke is not set", 2) end
    return cb(...)
end
function __rs_lua_methods.InvokeServer(self, ...)
    member(self, "RemoteFunction", "InvokeServer")
    local cb = __rs_get_callback(self, "OnServerInvoke")
    if not cb then error("OnServerInvoke is not set", 2) end
    return cb(game:GetService("Players").LocalPlayer, ...)
end
function __rs_lua_methods.InvokeClient(self, player, ...)
    member(self, "RemoteFunction", "InvokeClient")
    error("InvokeClient can only be called from the server", 2)
end

return inst_index, inst_newindex, wrap
"##;

// ---------------------------------------------------------------- runtime

/// `getmetatable` of every bridge object answers "The metatable is locked",
/// as in Studio: one throwaway object of each type.
fn lock_bridge(lua: &Lua) -> mlua::Result<()> {
    use crate::cmeta::lock;
    lock(lua, &lua.create_userdata(LInst(Ref::none()))?)?;
    lock(lua, &lua.create_userdata(LObj(Ref::none()))?)?;
    lock(lua, &lua.create_userdata(LConn { key: (Ref::none(), String::new()), id: 0 })?)?;
    let sig: AnyUserData = lua
        .globals()
        .get::<Function>("__rs_make_signal")?
        .call((lua.create_table()?, lua.create_table()?))?;
    lock(lua, &sig)?;
    crate::ltypes::lock_enums(lua)
}

/// Turns a Lua error into one Output line: `Script.Path:line: message`.
pub fn describe_error(e: &mlua::Error) -> String {
    fn cause_msg(e: &mlua::Error) -> String {
        match e {
            mlua::Error::RuntimeError(m) => m.lines().next().unwrap_or("").to_string(),
            mlua::Error::SyntaxError { message, .. } => {
                message.lines().next().unwrap_or("").to_string()
            }
            mlua::Error::CallbackError { cause, .. } => cause_msg(cause),
            mlua::Error::WithContext { cause, .. } => cause_msg(cause),
            other => other.to_string().lines().next().unwrap_or("").to_string(),
        }
    }
    // Errors raised from Rust callbacks are engine errors: the message
    // alone, as Studio prints "Downloading asset failed for asset id …"
    // (round 26); where it happened is the stack after it (callback_origin).
    let m = cause_msg(e);
    match m.strip_prefix(crate::cmeta::LOCATED) {
        Some(rest) => rest.to_string(),
        None => m,
    }
}

/// Where a Rust callback's error was raised: the first Lua frame of its
/// traceback.
fn callback_origin(e: &mlua::Error) -> Option<(String, u32)> {
    let mlua::Error::CallbackError { traceback, .. } = e else {
        return None;
    };
    for line in traceback.lines() {
        let tok = line.split_whitespace().next().unwrap_or("");
        if tok.is_empty() || tok.starts_with('[') || tok.starts_with("stack") {
            continue;
        }
        if let Some((src, ln)) = tok.trim_end_matches(':').rsplit_once(':') {
            if let Ok(ln) = ln.parse::<u32>() {
                if !src.starts_with("rustudio.") {
                    return Some((src.to_string(), ln));
                }
            }
        }
    }
    None
}

fn thread_arg(lua: &Lua, v: Option<Value>, what: &str) -> mlua::Result<Thread> {
    match v {
        Some(Value::Function(f)) => domains::assets_cloud::scene::create_thread(lua,f),
        Some(Value::Thread(t)) => Ok(t),
        _ => Err(rt_err(format!("{what} expects a function or thread"))),
    }
}

/// Luau feature flags Roblox ships on. The compiler flags reuse a dead
/// local's register (Luau 745); without them a big script like Place A'
/// Weapons client fails with "Out of local registers". Flags are global and
/// must be set before any VM exists or compiles, so this runs once.
pub fn luau_flags() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        for f in ["LuauCompileReuseLocalRegs", "LuauCompileReuseLowersLimit"] {
            if Lua::set_fflag(f, true).is_err() {
                eprintln!("rustudio: Luau flag {f} missing");
            }
        }
    });
}

impl Runtime {
    pub fn new() -> Result<Self, String> {
        luau_flags();
        let lua = Lua::new();
        routes::install_atoms(&lua);
        crate::globals_parity::init_vm(&lua);
        // Studio compiles scripts at Luau -O1 (no inlining), so Output stacks
        // keep every frame; -O2 inlined local functions out of error stacks.
        // Studio's other compile options:
        // `Vector3.new` is Luau's vector constructor (the LBF_VECTOR fastcall:
        // no call into the engine for `Vector3.new(x, y, z)` with numbers),
        // and these globals stay mutable (not cached as imports).
        lua.set_compiler(
            mlua::chunk::Compiler::new()
                .set_optimization_level(1)
                .set_debug_level(1)
                .set_vector_ctor("Vector3.new")
                .set_vector_type("Vector3")
                .set_mutable_globals(["Game", "Workspace", "game", "plugin", "script", "shared", "workspace"]),
        );
        lua.set_app_data(Sched::default());
        lua.set_app_data(Signals::default());
        lua.set_app_data(signal_timing::Reentry::default());
        let weak = lua.create_table().map_err(|e| e.to_string())?;
        let mt = lua.create_table().map_err(|e| e.to_string())?;
        mt.raw_set("__mode", "v").map_err(|e| e.to_string())?;
        weak.set_metatable(Some(mt.clone())).map_err(|e| e.to_string())?;
        let sigs = lua.create_table().map_err(|e| e.to_string())?;
        sigs.set_metatable(Some(mt)).map_err(|e| e.to_string())?;
        lua.set_app_data(Cache { weak: Some(weak), sigs: Some(sigs), ..Default::default() });
        lua.set_app_data(Budget { start: None });
        lua.set_app_data(ChildIndex::default());
        lua.set_app_data(ProfileDepth::default());
        security::install_thread_hook(&lua).map_err(|e| e.to_string())?;
        let mut callbacks = 0usize;
        unsafe {
            lua.exec_raw::<()>((), |state| {
                callbacks = mlua::ffi::lua_callbacks(state) as usize;
            })
        }
        .map_err(|e| e.to_string())?;
        lua.set_interrupt(move |lua| {
            // Armed by the watchdog (interrupt_watchdog): one check, then off.
            interrupt_watchdog::disarm(callbacks);
            if let Some(b) = lua.app_data_ref::<Budget>() {
                if let Some(s) = b.start {
                    if s.elapsed() > Duration::from_secs_f32(mech::SCRIPT_TIMEOUT_S) {
                        // Name the script and line that ran too long.
                        let at = (0..32)
                            .map_while(|level| {
                                lua.inspect_stack(level, |d| {
                                    let src = d
                                        .source()
                                        .short_src
                                        .map(|s| s.trim_start_matches(['=', '@']).to_string())
                                        .unwrap_or_default();
                                    (src, d.current_line())
                                })
                            })
                            .find(|(s, _)| {
                                !s.is_empty() && !s.starts_with("rustudio.") && s != "[C]"
                            });
                        return Err(rt_err(match at {
                            Some((s, Some(l))) => {
                                format!("{s}:{l}: Script timeout: exhausted allowed execution time")
                            }
                            Some((s, None)) => {
                                format!("{s}: Script timeout: exhausted allowed execution time")
                            }
                            None => "Script timeout: exhausted allowed execution time".to_string(),
                        }));
                    }
                }
            }
            Ok(mlua::VmState::Continue)
        });
        let shared = lua.create_table().map_err(|e| e.to_string())?;
        interrupt_watchdog::register(callbacks as *mut mlua::ffi::lua_Callbacks);
        let rt = Runtime { lua, shared, callbacks };
        rt.install().map_err(|e| describe_error(&e))?;
        Ok(rt)
    }

    fn install(&self) -> mlua::Result<()> {
        let lua = &self.lua;
        let g = lua.globals();
        install(lua, &g)?;

        let inst = lua.create_table()?;
        inst.set(
            "new",
            lua.create_function(|lua, (class, parent): (String, Value)| {
                security::check_create(lua, &class)?;
                if !doc::is_script_creatable(&class) {
                    return Err(rt_err(format!(
                        "Unable to create an Instance of type \"{class}\""
                    )));
                }
                let r = {
                    let mut w = world(lua)?;
                    let r = w.dm().dom.insert(
                        Ref::none(),
                        InstanceBuilder::new(class.as_str()).with_name(class.as_str()),
                    );
                    w.structure_changed = true;
                    r
                };
                domains::assets_cloud::scene::track_instance(lua,r)?;
                if let Some(p) = opt_inst(&parent) {
                    set_parent(lua, r, p)?;
                }
                inst_value(lua, r)
            })?,
        )?;
        g.set("Instance", inst)?;

        let task = lua.create_table()?;
        task.set(
            "spawn",
            lua.create_function(|lua, args: Variadic<Value>| {
                let mut it = args.into_iter();
                let th = thread_arg(lua, it.next(), "task.spawn")?;
                resume(lua, &th, it.collect());
                Ok(th)
            })?,
        )?;
        task.set(
            "defer",
            lua.create_function(|lua, args: Variadic<Value>| {
                let mut it = args.into_iter();
                let th = thread_arg(lua, it.next(), "task.defer")?;
                sched(lua)?.deferred.push_back((th.clone(), it.collect()));
                Ok(th)
            })?,
        )?;
        task.set(
            "delay",
            lua.create_function(|lua, args: Variadic<Value>| {
                let mut it = args.into_iter();
                let t = it.next().and_then(|v| num(&v)).unwrap_or(0.0);
                let th = thread_arg(lua, it.next(), "task.delay")?;
                let now = world(lua)?.time;
                sched(lua)?.timers.push(Timer {
                    at: now + t,
                    started: now,
                    wall: Instant::now(),
                    thread: th.clone(),
                    args: Some(it.collect()),
                });
                Ok(th)
            })?,
        )?;
        task.set(
            "cancel",
            lua.create_function(|lua, th: Thread| {
                let mut s = sched(lua)?;
                s.timers.retain(|t| t.thread != th);
                s.deferred.retain(|(t, _)| *t != th);
                Ok(())
            })?,
        )?;
        g.set("task", task)?;
        g.set(
            "spawn",
            lua.create_function(|lua, f: Function| {
                let th = domains::assets_cloud::scene::create_thread(lua,f)?;
                sched(lua)?.deferred.push_back((th, vec![]));
                Ok(())
            })?,
        )?;
        g.set(
            "delay",
            lua.create_function(|lua, (t, f): (f64, Function)| {
                let th = domains::assets_cloud::scene::create_thread(lua,f)?;
                let now = world(lua)?.time;
                sched(lua)?.timers.push(Timer {
                    at: now + t,
                    started: now,
                    wall: Instant::now(),
                    thread: th,
                    args: Some(vec![]),
                });
                Ok(())
            })?,
        )?;
        g.set("time", lua.create_function(|lua, ()| Ok(world(lua)?.time))?)?;
        g.set(
            "elapsedTime",
            lua.create_function(|lua, ()| Ok(world(lua)?.time))?,
        )?;
        g.set(
            "tick",
            lua.create_function(|_, ()| {
                Ok(std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0))
            })?,
        )?;
        g.set("shared", lua.create_table()?)?;

        let rs_wait = lua.create_function(|lua, (th, t): (Thread, f64)| {
            let now = world(lua)?.time;
            // A zero wait still resumes on the next step.
            sched(lua)?.timers.push(Timer {
                at: now + t.max(0.0),
                started: now,
                wall: Instant::now(),
                thread: th,
                args: None,
            });
            Ok(())
        })?;
        let rs_wait_signal =
            lua.create_function(|lua, (inst, name, th): (LInst, String, Thread)| {
                let id = {
                    let mut s = signals(lua)?;
                    s.next_id += 1;
                    let id = s.next_id;
                    s.waiters.entry((inst.0, name)).or_default().push((id, th));
                    id
                };
                let mut w = world(lua)?;
                w.conn_seq = id;
                if w.listening.insert(inst.0) && w.side == Side::Client {
                    w.listen_new.push(inst.0);
                }
                Ok(())
            })?;
        let rs_output = lua.create_function(
            |lua, (kind, msg, src, line): (String, String, Option<String>, Option<i64>)| {
                let src = src.unwrap_or_default();
                let src = src.trim_start_matches(['=', '@']);
                let loc = match line {
                    Some(l) if l > 0 => format!("{src}:{l}"),
                    _ => src.to_string(),
                };
                let level = match kind.as_str() {
                    "warn" => Level::Warn,
                    "error" => Level::Error,
                    _ => Level::Info,
                };
                let line = if loc.is_empty() { msg } else { format!("{msg}  —  {loc}") };
                world(lua)?.output.push((level, line));
                Ok(())
            },
        )?;
        let rs_connect = lua.create_function(
            |lua, (inst, name, f, once): (LInst, String, Function, bool)| {
                let id = {
                    let mut s = signals(lua)?;
                    s.next_id += 1;
                    let id = s.next_id;
                    let list = s.conns.entry((inst.0, name.clone())).or_default();
                    let first = list.is_empty();
                    list.push(Conn { id, func: f, once, ident: security::current(lua) });
                    if first {
                        let names = s.conn_names.entry(inst.0).or_default();
                        if !names.contains(&name) {
                            names.push(name.clone());
                        }
                    }
                    id
                };
                if let Some(mut w) = lua.app_data_mut::<World>() {
                    w.conn_seq = id;
                    if w.listening.insert(inst.0) && w.side == Side::Client {
                        w.listen_new.push(inst.0);
                    }
                }
                Ok(LConn {
                    key: (inst.0, name),
                    id,
                })
            },
        )?;
        let rs_load_module = lua.create_function(|lua, m: LInst| {
            let shared: Table = lua.named_registry_value("rustudio_shared")?;
            compile_script(lua, m.0, &shared)
        })?;
        let rs_index = lua.create_function(|lua, (this, key, atom): (LInst, mlua::LuaString, Option<i32>)| {
            let key = key.to_str()?;
            let key: &str = &key;
            crate::bench::site(lua, "index", key);
            crate::bench::api_timed("index", key, || {
                checked1(lua, index_entry(lua, this.0, key, atom.unwrap_or(-1)))
            })
            .inspect(|_| signal_timing::flush_immediate(lua))
        })?;
        let rs_newindex =
            lua.create_function(|lua, (this, key, value, atom): (LInst, mlua::LuaString, Value, Option<i32>)| {
                let key = key.to_str()?;
                let key: &str = &key;
                crate::bench::site(lua, "newindex", key);
                crate::bench::api_timed("newindex", key, || {
                    checked1(lua, newindex_entry(lua, this.0, key, value, atom.unwrap_or(-1)).map(|_| Value::Nil))
                })
                .inspect(|_| signal_timing::flush_immediate(lua))
            })?;
        lua.set_named_registry_value("rustudio_shared", self.shared.clone())?;
        let (rs_index_raw, rs_newindex_raw) = (rs_index.clone(), rs_newindex.clone());
        security::install_globals(lua)?;
        let (inst_index, inst_newindex, wrap): (Function, Function, Function) =
            lua.load(PRELUDE).set_name("=rustudio.prelude").call((
                rs_wait,
                rs_wait_signal,
                rs_output,
                rs_connect,
                rs_load_module,
                rs_index,
                rs_newindex,
                mech::WAIT_MIN_S,
            ))?;
        // Roblox's are C functions, and an admin script checks (src/cmeta.rs); the
        // prelude's Lua shims stay for anything that calls them directly.
        let _ = (inst_index, inst_newindex);
        let _ = wrap;
        use crate::cmeta::INSTANCE;
        lua.set_named_registry_value(
            "rs_inst_index",
            crate::cmeta::wrap_meta_atom(lua, rs_index_raw, true, &INSTANCE, None)?,
        )?;
        lua.set_named_registry_value("rs_inst_newindex", crate::cmeta::wrap_meta_atom(lua, rs_newindex_raw, true, &INSTANCE, None)?)?;
        let rs_namecall = lua.create_function(|lua, mut args: MultiValue| {
            // The method name: a C string from lua_namecallatom, read now
            // (cmeta::namecall_fn), before anything else can run.
            let name = match args.pop_front() {
                Some(Value::LightUserData(p)) if !p.0.is_null() => {
                    unsafe { std::ffi::CStr::from_ptr(p.0 as *const std::ffi::c_char) }
                }
                _ => return checked(lua, Err(rt_err("attempt to call a nil value"))),
            };
            let atom = match args.pop_front() {
                Some(Value::Integer(a)) => a as i32,
                _ => -1,
            };
            let name = name.to_str().unwrap_or("");
            namecall(lua, name, atom, args)
        })?;
        lua.set_named_registry_value("rs_inst_namecall", crate::cmeta::namecall(lua, rs_namecall)?)?;
        lua.set_named_registry_value("rs_lua_methods", lua.globals().get::<Table>("__rs_lua_methods")?)?;
        lock_bridge(lua)?;
        // Studio's fast member reads (runtime_routes.rs), once the bridge
        // objects exist.
        if let Some((fast, data)) = routes::fast_index_fn(lua) {
            let f: Function = lua.named_registry_value("rs_inst_index")?;
            crate::cmeta::set_fast(lua, &f, fast, data)?;
        }
        if let Some((fast, data)) = routes::fast_newindex_fn(lua) {
            let f: Function = lua.named_registry_value("rs_inst_newindex")?;
            crate::cmeta::set_fast(lua, &f, fast, data)?;
        }
        if let Some((fast, data)) = routes::fast_namecall_fn(lua) {
            let f: Function = lua.named_registry_value("rs_inst_namecall")?;
            crate::cmeta::set_fast_call(lua, &f, fast, data)?;
        }

        // Engine services written in Lua (TweenService, Debris, DataStores…).
        let rs_create = lua.create_function(|lua, class: String| {
            let r = ext::create_any(lua, &class)?;
            inst_value(lua, r)
        })?;
        let rs_fire = lua.create_function(|lua, args: MultiValue| {
            let mut it = args.into_iter();
            let target = it
                .next()
                .and_then(|v| opt_inst(&v))
                .ok_or_else(|| rt_err("__rs_fire: Instance expected"))?;
            let name = match it.next() {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(rt_err("__rs_fire: event name expected")),
            };
            world(lua)?.fire(target, &name, it.collect());
            signal_timing::flush_immediate(lua);
            Ok(())
        })?;
        // A KeyframeSequence's poses and keyframes as library.luau's
        // parse_sequence reads them, straight from the tree: walking 84k
        // poses through the API took Place B's first spawn 1.4 s. Returns
        // (joints: name → {t, cf, style, dir} in tree order, frames:
        // {t, name, markers = {{name, value}}}, length); the caller sorts.
        g.set(
            "__rs_kfs_parts",
            lua.create_function(|lua, kfs: LInst| {
                let enum_name = |v: Option<Variant>, e: &str, dflt: &str| match v {
                    Some(Variant::Enum(x)) => LEnumItem::from_value(e, x.to_u32())
                        .map(|i| i.name)
                        .unwrap_or_else(|| dflt.to_string()),
                    _ => dflt.to_string(),
                };
                let num = |v: Option<Variant>, dflt: f64| match v {
                    Some(Variant::Float32(f)) => f as f64,
                    Some(Variant::Float64(f)) => f,
                    Some(Variant::Int32(i)) => i as f64,
                    _ => dflt,
                };
                struct PoseRow {
                    name: String,
                    t: f64,
                    cf: Variant,
                    style: String,
                    dir: String,
                }
                let (poses, keyframes) = {
                    let w = world(lua)?;
                    let d = w.d();
                    let mut poses: Vec<PoseRow> = Vec::new();
                    // (time, name, markers)
                    let mut keyframes: Vec<(f64, String, Vec<(String, String)>)> = Vec::new();
                    fn walk(
                        d: &Document,
                        parent: Ref,
                        t: f64,
                        out: &mut Vec<PoseRow>,
                        row: &dyn Fn(&Document, Ref, f64) -> Option<PoseRow>,
                    ) {
                        let Some(i) = d.dom.get_by_ref(parent) else { return };
                        for c in i.children() {
                            let Some(ci) = d.dom.get_by_ref(*c) else { continue };
                            if !doc::is_a(&ci.class, "Pose") {
                                continue;
                            }
                            if let Some(r) = row(d, *c, t) {
                                out.push(r);
                            }
                            walk(d, *c, t, out, row);
                        }
                    }
                    let row = |d: &Document, p: Ref, t: f64| -> Option<PoseRow> {
                        if num(d.prop(p, "Weight"), 1.0) <= 0.0 {
                            return None;
                        }
                        Some(PoseRow {
                            name: d.dom.get_by_ref(p)?.name.clone(),
                            t,
                            cf: d
                                .prop(p, "CFrame")
                                .unwrap_or(Variant::CFrame(rbx_dom_weak::types::CFrame::identity())),
                            style: enum_name(d.prop(p, "EasingStyle"), "PoseEasingStyle", "Linear"),
                            dir: enum_name(d.prop(p, "EasingDirection"), "PoseEasingDirection", "In"),
                        })
                    };
                    if let Some(ki) = d.dom.get_by_ref(kfs.0) {
                        for k in ki.children() {
                            let Some(kk) = d.dom.get_by_ref(*k) else { continue };
                            if !doc::is_a(&kk.class, "Keyframe") {
                                continue;
                            }
                            let t = num(d.prop(*k, "Time"), 0.0);
                            walk(d, *k, t, &mut poses, &row);
                            let markers = kk
                                .children()
                                .iter()
                                .filter_map(|m| d.dom.get_by_ref(*m))
                                .filter(|m| doc::is_a(&m.class, "KeyframeMarker"))
                                .map(|m| {
                                    let v = match d.prop(m.referent(), "Value") {
                                        Some(Variant::String(s)) => s,
                                        _ => String::new(),
                                    };
                                    (m.name.clone(), v)
                                })
                                .collect();
                            keyframes.push((t, kk.name.clone(), markers));
                        }
                    }
                    (poses, keyframes)
                };
                let joints = lua.create_table()?;
                for p in poses {
                    let list: Table = match joints.raw_get::<Option<Table>>(p.name.as_str())? {
                        Some(l) => l,
                        None => {
                            let l = lua.create_table()?;
                            joints.raw_set(p.name.as_str(), l.clone())?;
                            l
                        }
                    };
                    let e = lua.create_table()?;
                    e.raw_set("t", p.t)?;
                    e.raw_set("cf", variant_to_lua(lua, &p.cf, None)?)?;
                    e.raw_set("style", p.style)?;
                    e.raw_set("dir", p.dir)?;
                    list.raw_push(e)?;
                }
                let frames = lua.create_table()?;
                let mut length = 0.0f64;
                for (t, name, markers) in keyframes {
                    let ms = lua.create_table()?;
                    for (n, v) in markers {
                        let m = lua.create_table()?;
                        m.raw_push(n)?;
                        m.raw_push(v)?;
                        ms.raw_push(m)?;
                    }
                    let f = lua.create_table()?;
                    f.raw_set("t", t)?;
                    f.raw_set("name", name)?;
                    f.raw_set("markers", ms)?;
                    frames.raw_push(f)?;
                    length = length.max(t);
                }
                Ok((joints, frames, length))
            })?,
        )?;
        // A public asset's download state: true (on disk), false (downloading
        // in the background) or the reason it can't be had.
        g.set(
            "__rs_asset_ready",
            lua.create_function(|lua, id: f64| {
                Ok(match crate::assets::public_prefetch(id as u64) {
                    crate::assets::Fetch::Ready(_) => Value::Boolean(true),
                    crate::assets::Fetch::Pending => Value::Boolean(false),
                    crate::assets::Fetch::Unavailable(e) => Value::String(lua.create_string(&e)?),
                })
            })?,
        )?;
        // debug.profilebegin (+1) / profileend (-1) for the running thread;
        // false for an end with nothing open (library.luau warns).
        g.set(
            "__rs_profile",
            lua.create_function(|lua, delta: i32| {
                let th = lua.current_thread().to_pointer() as usize;
                let Some(mut p) = lua.app_data_mut::<ProfileDepth>() else {
                    return Ok(true);
                };
                let d = p.0.get(&th).copied().unwrap_or(0);
                if delta < 0 && d == 0 {
                    return Ok(false);
                }
                // Kept only while open, so the map stays small.
                match (d as i32 + delta).max(0) as u32 {
                    0 => p.0.remove(&th),
                    n => p.0.insert(th, n),
                };
                Ok(true)
            })?,
        )?;
        // MarketplaceService:GetProductInfo's record (library.luau): the
        // JSON text, false while it's being fetched, or nil and the error.
        g.set(
            "__rs_product_info",
            lua.create_function(|lua, (id, kind): (f64, String)| {
                Ok(match crate::assets::product_info(id as u64, &kind) {
                    crate::assets::Fetch::Ready(b) => (
                        Value::String(lua.create_string(&b[..])?),
                        Value::Nil,
                    ),
                    crate::assets::Fetch::Pending => (Value::Boolean(false), Value::Nil),
                    crate::assets::Fetch::Unavailable(e) => {
                        (Value::Nil, Value::String(lua.create_string(&e)?))
                    }
                })
            })?,
        )?;
        // ParticleEmitter:Emit(n) / :Clear() (n −1) for the viewport's
        // particles (src/effects.rs), on this DataModel.
        g.set(
            "__rs_particles",
            lua.create_function(|lua, (inst, n): (LInst, f64)| {
                let Some(id) = world(lua)?.doc.as_ref().map(|d| d.id) else {
                    return Ok(());
                };
                if n < 0.0 {
                    crate::effects::clear(id, inst.0);
                } else {
                    crate::effects::emit(id, inst.0, n.min(100_000.0) as u32);
                }
                Ok(())
            })?,
        )?;
        // PhysicsService collision groups (library.luau), in the session's
        // shared table.
        g.set(
            "__rs_cg",
            lua.create_function(|lua, (op, a, b, on): (String, Option<String>, Option<String>, Option<bool>)| {
                let shared = world(lua)?.groups.clone();
                let mut cg = shared.lock().map_err(|_| rt_err("collision groups poisoned"))?;
                let a = a.unwrap_or_default();
                let b = b.unwrap_or_default();
                Ok(match op.as_str() {
                    "register" => {
                        cg.register(&a).map_err(rt_err)?;
                        Value::Nil
                    }
                    "unregister" => {
                        crate::collision_groups::check_name(&a).map_err(rt_err)?;
                        cg.unregister(&a);
                        Value::Nil
                    }
                    "rename" => {
                        crate::collision_groups::check_name(&a).map_err(rt_err)?;
                        crate::collision_groups::check_name(&b).map_err(rt_err)?;
                        cg.rename(&a, &b);
                        Value::Nil
                    }
                    "set" => {
                        // Server only while the game runs.
                        if world(lua)?.side == Side::Client {
                            return Err(rt_err("This API can only be used on the server!"));
                        }
                        cg.set_collidable_checked(&a, &b, on.unwrap_or(true)).map_err(rt_err)?;
                        Value::Nil
                    }
                    "id" => match cg.id(&a) {
                        Some(i) => Value::Number(i as f64),
                        // GetCollisionGroupId.
                        None => return Err(rt_err("Collision group not found.")),
                    },
                    "name" => {
                        // GetCollisionGroupName: ids 0..31.
                        let id = a.parse::<i64>().unwrap_or(-1);
                        if !(0..crate::collision_groups::MAX_GROUPS as i64).contains(&id) {
                            return Err(rt_err(format!(
                                "The Collision group Id must be between 0 and {}",
                                crate::collision_groups::MAX_GROUPS
                            )));
                        }
                        let n = cg.names().get(id as usize).cloned().unwrap_or_default();
                        Value::String(lua.create_string(&n)?)
                    }
                    "collidable" => Value::Boolean(cg.collidable(&a, &b)),
                    "registered" => Value::Boolean(cg.is_registered(&a)),
                    "list" => {
                        let t = lua.create_table()?;
                        for (i, n) in cg.names().iter().enumerate() {
                            let e = lua.create_table()?;
                            e.set("name", n.as_str())?;
                            e.set("id", i)?;
                            e.set("mask", (cg.bits(n).1) as f64)?;
                            t.set(i + 1, e)?;
                        }
                        Value::Table(t)
                    }
                    _ => Value::Nil,
                })
            })?,
        )?;
        // A callback's function, for the engine's own invoke code (scripts
        // can't read callbacks: Studio's __index refuses, see `index`).
        g.set(
            "__rs_get_callback",
            lua.create_function(|lua, (inst, key): (LInst, String)| {
                Ok(cache(lua)?
                    .callbacks
                    .get(&(inst.0, key))
                    .cloned()
                    .map(Value::Function)
                    .unwrap_or(Value::Nil))
            })?,
        )?;
        // Engine-owned values that scripts may only read (Humanoid.MoveDirection
        // from Humanoid:Move): set from the library, read-only or not.
        g.set(
            "__rs_set_raw",
            lua.create_function(|lua, (inst, key, value): (LInst, String, Value)| {
                let (class, _, _) = inst_meta(lua, inst.0)?;
                let Some((canon, dt, _)) = prop_desc(&class, &key) else {
                    return Err(rt_err(format!("{key} is not a valid member of {class}")));
                };
                let existing = world(lua)?.d().prop(inst.0, canon);
                let (ty, en) = match &dt {
                    DataType::Value(t) => (*t, None),
                    DataType::Enum(e) => (VariantType::Enum, Some(e.to_string())),
                    _ => return Err(rt_err(format!("Unable to assign property {key}"))),
                };
                let v = lua_to_variant(&key, &value, ty, en.as_deref(), existing.as_ref())?;
                store_prop(lua, inst.0, canon, v)
            })?,
        )?;
        lua.load(include_str!("library.luau"))
            .set_name("=rustudio.library")
            .call::<()>((rs_create, rs_fire))?;
        // UserGameSettings' values, for its property hooks (domains/services).
        lua.set_named_registry_value("rs_user_game_settings", g.get::<Table>("__rs_user_game_settings")?)?;
        editable::install(lua)?;
        domains::install(lua)?;
        crate::dtmeta::install_all(lua)?;
        security::seal(lua)?;

        // Script environments read the public globals through `shared`
        // (src/globals_parity.rs: rustudio's own stay out of reach).
        let _ = g;
        crate::globals_parity::install(lua, &self.shared)?;
        Ok(())
    }

    /// Hands the play document (inside its World) to Lua for this frame.
    pub fn enter(&self, world: World) {
        self.lua.set_app_data(world);
        domains::services::on_enter(&self.lua);
    }

    pub fn leave(&self) -> World {
        domains::services::on_leave(&self.lua);
        self.lua.remove_app_data::<World>().expect("world entered")
    }

    pub fn world_mut(&self) -> Option<mlua::AppDataRefMut<'_, World>> {
        self.lua.app_data_mut::<World>()
    }

    /// Defines `game` and `workspace` once the DataModel is in place.
    pub fn bind_globals(&self) -> mlua::Result<()> {
        let lua = &self.lua;
        domains::data::bind_context(lua)?;
        let (root, ws) = {
            let w = world(lua)?;
            let d = w.d();
            (d.dom.root_ref(), d.workspace().unwrap_or_else(Ref::none))
        };
        domains::assets_cloud::scene::scan_access(lua,root)?;
        let g = lua.globals();
        g.set("game", inst_value(lua, root)?)?;
        g.set("Game", inst_value(lua, root)?)?;
        g.set("workspace", inst_value(lua, ws)?)?;
        g.set("Workspace", inst_value(lua, ws)?)?;
        // os.clock: Studio's counts from its own process start (2633 s into a
        // long session, measured; Studio clock, not the
        // machine's uptime that Luau's built-in gives on Linux). The fidelity
        // harness sets RUSTUDIO_CLOCK_BASE to Studio's reading at Play start
        // so time-driven game code (Place A's menu camera sway) runs in phase.
        let base = std::env::var("RUSTUDIO_CLOCK_BASE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok());
        let since = process_start();
        // (Phase taken once: Studio's reading at Play start + time() at the
        // first call; from there it runs on the wall clock like Studio's, so
        // it advances inside a frame. Frozen at the frame's time(), Place A's
        // time-sliced map loader never yielded and hit the script timeout.)
        let offset: std::cell::Cell<Option<f64>> = std::cell::Cell::new(None);
        // RUSTUDIO_CPU_CLOCK=1 (benchmarks only): the calling thread's CPU
        // time instead, so per-call costs can be timed on a loaded machine.
        let cpu_clock = std::env::var_os("RUSTUDIO_CPU_CLOCK").is_some();
        let clock = lua.create_function(move |lua, ()| {
            // (Unix only; elsewhere the wall clock below.)
            #[cfg(unix)]
            if cpu_clock {
                let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
                unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
                return Ok(ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9);
            }
            #[cfg(not(unix))]
            let _ = cpu_clock;
            let now = since.elapsed().as_secs_f64();
            Ok(match base {
                Some(b) => {
                    let off = offset.get().unwrap_or_else(|| {
                        let o = b + world(lua).map(|w| w.time).unwrap_or(0.0) - now;
                        offset.set(Some(o));
                        o
                    });
                    now + off
                }
                None => now,
            })
        })?;
        if let Ok(os) = g.get::<Table>("os") {
            os.set("clock", clock)?;
        }
        crate::globals_parity::sync(lua)?;
        Ok(())
    }

    /// Starts a script Play collected, if it is still somewhere it runs, as
    /// in Roblox: Play gathers scripts first and starts them one by one, and
    /// an earlier one can unparent or clone a later one's GUI in between
    /// (Place B' button scripts then ran with script.Parent nil).
    /// One left out stays unstarted, so moving it somewhere it runs starts it.
    pub fn start_collected(&self, r: Ref) {
        if world(&self.lua).is_ok_and(|w| w.runs_here(r)) {
            self.run_script(r);
        }
    }

    /// Starts a Script or LocalScript as a new coroutine.
    pub fn run_script(&self, r: Ref) {
        let lua = &self.lua;
        if let Ok(mut w) = world(lua) {
            if !w.started.insert(r) {
                return;
            }
        }
        // Like Studio, the classic free-model "Virus" scripts don't run.
        let flagged = world(lua).ok().and_then(|w| {
            let name = w.d().dom.get_by_ref(r)?.name.to_lowercase();
            name.contains("virus").then(|| w.full_name(r))
        });
        if let Some(full) = flagged {
            if let Ok(mut w) = world(lua) {
                w.output.push((
                    Level::Warn,
                    format!("Script {full} detected as malicious.  Script will not run."),
                ));
            }
            return;
        }
        let started = compile_script(lua, r, &self.shared)
            .and_then(|f| domains::assets_cloud::scene::create_thread(lua,f))
            .inspect(|th| security::set(th, security::GAME));
        match started {
            Ok(th) if crate::bench::lua_prof_on() => crate::bench::lua_timed(
                th.to_pointer() as usize,
                || format!("(start) {}", world(lua).map(|w| w.full_name(r)).unwrap_or_default()),
                || resume(lua, &th, vec![]),
            ),
            Ok(th) => resume(lua, &th, vec![]),
            Err(e) => report(lua, &e),
        }
    }

    /// Builds this side's world-query cache (rays, region queries) ahead of
    /// the first script query.
    pub fn warm_queries(&self) {
        ext::warm_queries(&self.lua);
    }

    pub fn fire(&self, r: Ref, name: &str, args: Vec<Value>) {
        if let Some(mut w) = self.lua.app_data_mut::<World>() {
            // A native frame step that follows this RunService event
            // (TweenService, Animator: domains::anim::hook_after).
            let hook = domains::anim::hook_after(w.side, name).map(|h| (h, args.first().cloned()));
            w.fire(r, name, args);
            if let Some((h, dt)) = hook {
                w.queue_hook(r, name, h, dt.unwrap_or(Value::Nil));
            }
        }
    }

    pub fn inst(&self, r: Ref) -> Value {
        inst_value(&self.lua, r).unwrap_or(Value::Nil)
    }

    /// Advances time: resumes due waits and delays, then dispatches.
    pub fn step(&self, dt: f64) {
        let lua = &self.lua;
        let (now, realtime) = {
            let Some(mut w) = lua.app_data_mut::<World>() else {
                return;
            };
            w.time += dt;
            (w.time, w.realtime)
        };
        {
            let _s = crate::bench::scope("rt_domains_step");
            let _ = domains::step(lua, dt);
        }
        // Frame signals (including the camera's RenderStepped callback) must
        // run before task.wait resumes at Heartbeat. Otherwise a script can
        // switch Custom back to Scriptable before the camera gets its frame.
        {
            let _s = crate::bench::scope("rt_dispatch1");
            self.dispatch();
        }
        let _st = crate::bench::scope("rt_timers");
        let mut due: Vec<Timer> = {
            let Ok(mut s) = sched(lua) else { return };
            let (due, keep): (Vec<Timer>, Vec<Timer>) = std::mem::take(&mut s.timers)
                .into_iter()
                .partition(|t| {
                    t.at <= now + 1e-9
                        || realtime && t.wall.elapsed().as_secs_f64() >= t.at - t.started
                });
            s.timers = keep;
            due
        };
        // Oldest first; whatever doesn't fit in this frame's budget resumes next frame.
        due.sort_by(|a, b| a.at.total_cmp(&b.at));
        let started = std::time::Instant::now();
        let mut due = due.into_iter();
        for t in due.by_ref() {
            let args = t
                .args
                .unwrap_or_else(|| {
                    let real = t.wall.elapsed().as_secs_f64();
                    vec![Value::Number((now - t.started).max(real))]
                });
            resume_thread_timed(lua, &t.thread, args);
            if started.elapsed().as_secs_f64() * 1000.0 > mech::SCRIPT_FRAME_BUDGET_MS {
                break;
            }
        }
        let rest: Vec<Timer> = due.collect();
        if !rest.is_empty() {
            if let Ok(mut s) = sched(lua) {
                s.timers.extend(rest);
            }
        }
        drop(_st);
        let _s2 = crate::bench::scope("rt_dispatch2");
        self.dispatch();
        drop(_s2);
        ext::flush_terrain(lua);
    }

    /// Frees unparented instances no script can reach any more, as Roblox's
    /// garbage collector does: an instance out of the DataModel goes away
    /// with its subtree once no Lua value of it or of any descendant is left
    /// (Destroy only unparents, locks Parent and drops the connections).
    /// Before this, everything ever made stayed in the DOM: the Death
    /// Machine's tracers, bullet holes and impact parts piled up for as long
    /// as Play ran (a live place, RSS past 24 GB). `pinned` keeps instances the
    /// play session still needs (replication pairs). Looks at no more than
    /// `budget` instances; returns the ones freed, after marking each in the
    /// document's journal so every reader drops it.
    pub fn collect(&self, pinned: &dyn Fn(Ref) -> bool, budget: usize) -> Vec<Ref> {
        let lua = &self.lua;
        let Some(mut c) = lua.app_data_mut::<Cache>() else {
            return Vec::new();
        };
        let Some(mut w) = lua.app_data_mut::<World>() else {
            return Vec::new();
        };
        if w.doc.is_none() {
            return Vec::new();
        }
        for r in std::mem::take(&mut c.fresh) {
            if w.d().dom.get_by_ref(r).is_some_and(|i| i.parent().is_none()) {
                w.orphans.insert(r);
            }
        }
        // Many new unparented instances since the last pass: garbage is
        // being made, so the Luau collector is stepped at the fast rate
        // (`gc_step`). A trickle (a GUI script swapping a few labels) isn't.
        let arrived = w.orphans.len().saturating_sub(w.orphans_seen) >= mech::GC_BUSY_ORPHANS;
        if w.orphans.is_empty() {
            w.orphans_seen = 0;
            w.gc_busy = false;
            return Vec::new();
        }
        let root = w.d().dom.root_ref();
        let candidates: Vec<Ref> = w.orphans.iter().copied().collect();
        let mut looked = 0usize;
        let mut freed: Vec<Ref> = Vec::new();
        for r in candidates {
            if looked >= budget {
                break;
            }
            let d = w.d();
            let Some(i) = d.dom.get_by_ref(r) else {
                w.orphans.remove(&r);
                continue;
            };
            if i.parent().is_some() || r == root {
                // Back in a tree (its new root is checked if that leaves).
                w.orphans.remove(&r);
                continue;
            }
            // Stops at the first instance something still holds: usually
            // the root itself (a template a script keeps unparented), so a
            // held subtree costs one lookup, not a walk of all of it. A
            // live Place A match keeps whole map pieces this way: walking them
            // every pass was 45-70 ms twice a second.
            let held = |x: Ref| {
                pinned(x)
                    || c.slots.get(&x).is_some_and(|slot| {
                        c.weak
                            .as_ref()
                            .and_then(|t| t.raw_get::<Value>(*slot).ok())
                            .is_some_and(|v| !matches!(v, Value::Nil))
                    })
            };
            let mut subtree: Vec<Ref> = Vec::new();
            let mut alive = false;
            for x in d.dom.descendants_of(r).map(|i| i.referent()) {
                looked += 1;
                if held(x) {
                    alive = true;
                    break;
                }
                subtree.push(x);
            }
            if alive {
                continue;
            }
            w.orphans.remove(&r);
            let dm = w.dm();
            for x in &subtree {
                dm.mark_tree(*x);
            }
            dm.dom.destroy(r);
            for x in &subtree {
                if let Some(slot) = c.slots.remove(x) {
                    if let Some(t) = c.weak.as_ref() {
                        let _ = t.raw_set(slot, Value::Nil);
                    }
                }
                w.destroyed.remove(x);
                w.dirty.remove(x);
                w.velocities.remove(x);
                w.humanoid_states.remove(x);
                w.states_disabled.remove(x);
                w.avatar_descs.remove(x);
                w.listening.remove(x);
                w.started.remove(x);
                w.orphans.remove(x);
                w.input.sound_pos.remove(x);
                w.input.sound_len.remove(x);
                w.input.sound_playing.remove(x);
                w.input.sound_paused.remove(x);
                w.input.gui_layout.remove(x);
            }
            freed.extend(subtree);
        }
        w.orphans_seen = w.orphans.len();
        w.gc_busy = arrived || freed.len() >= mech::GC_BUSY_ORPHANS;
        if freed.is_empty() {
            return freed;
        }
        w.freed += freed.len() as u64;
        w.structure_changed = true;
        let gone: HashSet<Ref> = freed.iter().copied().collect();
        c.callbacks.retain(|(r, _), _| !gone.contains(r));
        drop(c);
        drop(w);
        if let Some(mut s) = lua.app_data_mut::<Signals>() {
            s.conns.retain(|(r, _), _| !gone.contains(r));
            s.conn_names.retain(|r, _| !gone.contains(r));
            s.waiters.retain(|(r, _), _| !gone.contains(r));
        }
        if let Some(mut ix) = lua.app_data_mut::<ChildIndex>() {
            ix.by_parent.retain(|r, _| !gone.contains(r));
        }
        freed
    }

    /// Steps the Luau collector by this frame's share of a cycle. Roblox's
    /// collector runs all the time; Luau's runs as scripts allocate, so a
    /// quiet script could keep dead values (and the unparented instances
    /// behind them, which `collect` frees) for long. While many unparented
    /// instances keep arriving or going (`World::gc_busy`), a cycle (and
    /// with it the weak cache's dead entries) takes GC_CYCLE_PASSES
    /// collector intervals whatever the heap's size; while the only ones
    /// watched are those scripts still hold (cached templates: nothing to
    /// free), GC_IDLE_CYCLE_S. The work is spread over the frames, and no
    /// frame does more than Studio's LuauGc job does in one
    /// (GC_FRAME_MAX_KB; a huge heap's cycle takes longer instead): one
    /// step of heap / 2 (up to 16 MB) per pass was a 10-14 ms frame twice a
    /// second in Place E, 16 ms (89 ms at worst) in a Place A match.
    /// Nothing to do while no unparented instance is watched.
    pub fn gc_step(&self, dt: f64) {
        let lua = &self.lua;
        let Some(busy) = lua
            .app_data_ref::<World>()
            .filter(|w| w.doc.is_some() && !w.orphans.is_empty())
            .map(|w| w.gc_busy)
        else {
            return;
        };
        let heap_kb = (lua.used_memory() / 1024) as f64;
        let kb = if busy {
            let pass_kb = (heap_kb / mech::GC_CYCLE_PASSES as f64)
                .clamp(mech::GC_STEP_MIN_KB as f64, mech::GC_STEP_MAX_KB as f64);
            pass_kb * (dt / mech::GC_INTERVAL).clamp(0.0, 1.0)
        } else {
            heap_kb * (dt / mech::GC_IDLE_CYCLE_S).clamp(0.0, 1.0)
        };
        let kb = kb.ceil().clamp(1.0, mech::GC_FRAME_MAX_KB);
        let _ = unsafe {
            lua.exec_raw::<()>((), |state| {
                mlua::ffi::lua_gc(state, mlua::ffi::LUA_GCSTEP, kb as std::ffi::c_int);
            })
        };
    }

    /// Instances with a slot in the Lua value cache (bounded by the ones
    /// alive: a freed instance gives its slot back).
    pub fn lua_slots(&self) -> usize {
        self.lua.app_data_ref::<Cache>().map(|c| c.slots.len()).unwrap_or(0)
    }

    /// An instance the engine itself took out of the tree (a fallen part):
    /// the collector frees it once scripts can't reach it.
    pub fn note_orphan(&self, r: Ref) {
        if let Some(mut c) = self.lua.app_data_mut::<Cache>() {
            c.fresh.push(r);
        }
    }

    /// Lua-side counts for RUSTUDIO_MEMSTATS: (DOM instances reachable from
    /// the root, unparented instances being watched, instances freed so far,
    /// Lua values cached, connections, waiters, timers, Lua heap KB).
    pub fn mem_stats(&self) -> String {
        let lua = &self.lua;
        let (tree, orphans, freed, destroyed) = lua
            .app_data_ref::<World>()
            .and_then(|w| {
                let d = w.doc.as_ref()?;
                Some((d.dom.descendants().count(), w.orphans.len(), w.freed, w.destroyed.len()))
            })
            .unwrap_or_default();
        let (slots, _, cbs) = lua
            .app_data_ref::<Cache>()
            .map(|c| (c.slots.len(), 0usize, c.callbacks.len()))
            .unwrap_or_default();
        let (conns, waiters) = lua
            .app_data_ref::<Signals>()
            .map(|s| (s.conns.values().map(|v| v.len()).sum::<usize>(), s.waiters.values().map(|v| v.len()).sum::<usize>()))
            .unwrap_or_default();
        let (timers, deferred) = lua
            .app_data_ref::<Sched>()
            .map(|s| (s.timers.len(), s.deferred.len()))
            .unwrap_or_default();
        format!(
            "tree={tree} orphans={orphans} freed={freed} destroyed={destroyed} luavals={slots} cbs={cbs} conns={conns} waiters={waiters} timers={timers} deferred={deferred} luakb={}",
            lua.used_memory() / 1024
        )
    }

    /// Runs deferred threads and queued signal fires until both are empty.
    pub fn dispatch(&self) {
        let lua = &self.lua;
        for _ in 0..mech::MAX_DISPATCH_ROUNDS {
            let starting: Vec<Ref> = lua
                .app_data_mut::<World>()
                .map(|mut w| std::mem::take(&mut w.to_start))
                .unwrap_or_default();
            let _sc = crate::bench::scope("dis_start");
            for s in starting {
                let still = world(lua).map(|w| w.runs_here(s)).unwrap_or(false);
                if still {
                    self.run_script(s);
                }
            }
            let deferred: Vec<(Thread, Vec<Value>)> = sched(lua)
                .map(|mut s| s.deferred.drain(..).collect())
                .unwrap_or_default();
            // Fires made outside any script call (engine steps, input) that
            // the place runs immediately: run them first, at this point.
            signal_timing::flush_immediate(lua);
            let events: Vec<(Ref, String, Vec<Value>, u64, u32)> = lua
                .app_data_mut::<World>()
                .map(|mut w| std::mem::take(&mut w.events))
                .unwrap_or_default();
            if deferred.is_empty() && events.is_empty() {
                return;
            }
            drop(_sc);
            let _sc = crate::bench::scope("dis_deferred");
            for (th, args) in deferred {
                resume_thread_timed(lua, &th, args);
            }
            drop(_sc);
            let _sc = crate::bench::scope("dis_events");
            for (r, name, args, seq, depth) in events {
                if name.starts_with('\0') {
                    domains::anim::run_hook(lua, &name, &args);
                    continue;
                }
                signal_timing::deliver_deferred(lua, r, name, args, seq, depth);
            }
        }
        if let Some(mut w) = lua.app_data_mut::<World>() {
            w.output.push((
                Level::Warn,
                "Too many chained signal fires in one frame; the rest run next frame".into(),
            ));
        }
    }
}

/// The Luau frames of a thread stopped by an error, innermost first:
/// (chunk name, line, function name), read with Luau's own
/// `debug.traceback(thread)` ("Name:3 function f" / "Name:6" / "[C] function
/// error"). C frames and rustudio's own chunks are left out, as Studio's
/// stack printer skips frames without a line.
fn thread_frames(lua: &Lua, th: &Thread) -> Vec<(String, u32, Option<String>)> {
    let tb = lua
        .globals()
        .get::<Table>("debug")
        .and_then(|d| d.get::<Function>("traceback"))
        .and_then(|f| f.call::<String>(th.clone()));
    tb.map(|s| s.lines().filter_map(parse_frame).collect()).unwrap_or_default()
}

fn parse_frame(l: &str) -> Option<(String, u32, Option<String>)> {
    let l = l.trim();
    let (loc, func) = match l.split_once(" function ") {
        Some((a, f)) => (a, Some(f.trim().to_string()).filter(|f| !f.is_empty())),
        None => (l, None),
    };
    let (path, line) = loc.rsplit_once(':')?;
    let line = line.trim().parse().ok()?;
    (!path.starts_with('[') && !path.starts_with("rustudio.")).then(|| (path.to_string(), line, func))
}

fn report(lua: &Lua, e: &mlua::Error) {
    report_frames(lua, e, Vec::new());
}

/// Output for a script error, as Studio prints it: the error,
/// then in blue "Stack Begin", one "Script '{}', Line {} - function {}" (or
/// "Script '{}', Line {}" when the frame has no function name) per Luau
/// frame, "Stack End". With no frames known, the error's own "Name:line".
fn report_frames(lua: &Lua, e: &mlua::Error, mut frames: Vec<(String, u32, Option<String>)>) {
    let msg = describe_error(e);
    if frames.is_empty() {
        if let Some((path, line)) = crate::output_panel::error_origin(&msg) {
            frames.push((path.to_string(), line, None));
        } else if let Some((path, line)) = callback_origin(e) {
            frames.push((path, line, None));
        }
    }
    let target = {
        let Some(mut w) = lua.app_data_mut::<World>() else { return };
        w.output.push((Level::Error, msg.clone()));
        if !frames.is_empty() {
            let mark = crate::output_panel::STACK_MARK;
            w.output.push((Level::Info, format!("{mark}Stack Begin")));
            for (path, line, func) in &frames {
                let text = match func {
                    Some(f) => format!("Script '{path}', Line {line} - function {f}"),
                    None => format!("Script '{path}', Line {line}"),
                };
                w.output.push((Level::Info, format!("{mark}{text}  —  {path}:{line}")));
            }
            w.output.push((Level::Info, format!("{mark}Stack End")));
        }
        // ScriptContext.Error(message, stackTrace, script): the script from
        // the message's "Full.Name:line:" prefix when it resolves.
        w.doc.as_ref().and_then(|d| {
            let root = d.dom.root_ref();
            let sc = d.find_child(root, "ScriptContext")?;
            // An engine error has no prefix: the innermost frame's script.
            let path = match crate::output_panel::error_origin(&msg) {
                Some((p, _)) => p,
                None => frames.first().map(|f| f.0.as_str()).unwrap_or(""),
            };
            let mut cur = root;
            let mut found = !path.is_empty();
            for name in path.split('.') {
                match d.find_child(cur, name) {
                    Some(c) => cur = c,
                    None => {
                        found = false;
                        break;
                    }
                }
            }
            Some((sc, found.then_some(cur)))
        })
    };
    if let Some((sc, script)) = target {
        // The stack trace as Studio passes it: "Full.Name, line N\n" per
        // frame (" - function f" when named).
        let trace: String = frames
            .iter()
            .map(|(path, line, func)| match func {
                Some(f) => format!("{path}, line {line} - function {f}\n"),
                None => format!("{path}, line {line}\n"),
            })
            .collect();
        let args = (|| -> mlua::Result<Vec<Value>> {
            Ok(vec![
                Value::String(lua.create_string(&msg)?),
                Value::String(lua.create_string(&trace)?),
                match script {
                    Some(s) => inst_value(lua, s)?,
                    None => Value::Nil,
                },
            ])
        })();
        if let (Ok(args), Some(mut w)) = (args, lua.app_data_mut::<World>()) {
            w.fire(sc, "Error", args);
        }
    }
}

/// A bench key for a name (event, global).
fn name_key(name: &str) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    h.finish() as usize
}

/// "Chunk:line" where a function is defined (bench labels).
fn function_label(f: &Function) -> String {
    let i = f.info();
    format!(
        "{}:{}",
        i.short_src.or(i.source).unwrap_or_default().trim_start_matches('='),
        i.line_defined.unwrap_or(0)
    )
}

/// "Chunk:line" where a suspended thread is waiting (bench labels): the
/// first frame outside rustudio's own chunks (wait / task.wait).
fn thread_label(lua: &Lua, th: &Thread) -> String {
    let Ok(info) = lua
        .globals()
        .get::<Table>("debug")
        .and_then(|d| d.get::<Function>("info"))
    else {
        return "(resume) ?".to_string();
    };
    for level in 1..6 {
        match info.call::<(Option<String>, Option<i64>)>((th.clone(), level, "sl")) {
            Ok((Some(src), line))
                if !src.trim_start_matches('=').starts_with("rustudio.") && !src.starts_with('[') =>
            {
                return format!("(resume) {}:{}", src.trim_start_matches('='), line.unwrap_or(0));
            }
            Ok((Some(_), _)) => continue,
            _ => break,
        }
    }
    "(resume) ?".to_string()
}

/// `resume`, timed per thread while a bench runs.
fn resume_thread_timed(lua: &Lua, th: &Thread, args: Vec<Value>) {
    if crate::bench::lua_prof_on() && th.status() == ThreadStatus::Resumable {
        crate::bench::lua_timed(th.to_pointer() as usize, || thread_label(lua, th), || resume(lua, th, args))
    } else {
        resume(lua, th, args)
    }
}

/// Resumes a coroutine under the script timeout, routing errors to Output.
fn resume(lua: &Lua, th: &Thread, args: Vec<Value>) {
    if th.status() != ThreadStatus::Resumable {
        return;
    }
    let outermost = lua
        .app_data_ref::<Budget>()
        .is_some_and(|b| b.start.is_none());
    if outermost {
        if let Some(mut b) = lua.app_data_mut::<Budget>() {
            b.start = Some(Instant::now());
        }
    }
    let hitch = crate::bench::hitch_on().then(Instant::now);
    let label = hitch.map(|_| thread_label(lua, th));
    if hitch.is_some() && outermost {
        crate::bench::hitch_api_take();
    }
    if let Some(mut p) = lua.app_data_mut::<ProfileDepth>() {
        if !p.0.is_empty() {
            p.0.remove(&(th.to_pointer() as usize));
        }
    }
    let r = th.resume::<MultiValue>(MultiValue::from_iter(args));
    if let (Some(t0), Some(l)) = (hitch, label) {
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if ms > 10.0 {
            let after = thread_label(lua, th);
            let err = r.as_ref().err().map(|e| describe_error(e)).unwrap_or_default();
            eprintln!("HITCH resume {ms:.1} ms {l} / after {after} {err:.200}");
            if outermost && ms > 300.0 {
                for (k, t, n) in crate::bench::hitch_api_take().into_iter().take(12) {
                    eprintln!("HITCH   {t:.1} ms {n}x {k}");
                }
            }
        }
    }
    if outermost {
        if let Some(mut b) = lua.app_data_mut::<Budget>() {
            b.start = None;
        }
    }
    if let Err(e) = r {
        report_frames(lua, &e, thread_frames(lua, th));
    }
}

/// A place script's chunk name: `@` + its full name. Instance names are
/// place-controlled, so they must never land in the `=` namespace that
/// security::internal_chunk trusts as rustudio's own (a ModuleScript named
/// `rustudio.x` parented to nil). Luau shows `@` and `=` names alike
/// (short_src: errors, tracebacks, debug.info "s") up to LUA_IDSIZE; past it
/// `@` would keep the tail, so cut to the 255 bytes `=` shows.
pub(crate) fn place_chunk_name(full_name: &str) -> String {
    let mut end = full_name.len().min(255);
    while !full_name.is_char_boundary(end) {
        end -= 1;
    }
    format!("@{}", &full_name[..end])
}

/// Compiles a script's Source with `script` bound in its own environment.
fn compile_script(lua: &Lua, r: Ref, shared: &Table) -> mlua::Result<Function> {
    let (source, name) = {
        let w = world(lua)?;
        let src = match w.d().prop(r, "Source") {
            Some(Variant::String(s)) => s,
            _ => String::new(),
        };
        // A name with a NUL in it (an Place B script) can't be a chunk
        // name; Studio runs the script all the same.
        (src, w.full_name(r).replace('\0', ""))
    };
    let memory_category=domains::assets_cloud::scene::begin(lua,r)?;
    let result=(|| {
    // `_G`, `shared`, `script`, locked, reading through to `shared` (the
    // public globals) and flagged safe for Luau's fast paths (fastcall
    // builtins, pairs/ipairs loops, pcall), as Roblox's sandboxed script
    // envs are; Luau clears it itself if the script calls getfenv/setfenv.
    let _ = shared;
    let env = crate::globals_parity::new_env(lua, Some(inst_value(lua, r)?))?;
    lua.load(source)
        .set_name(place_chunk_name(&name))
        .set_environment(env)
        .into_function()
    })();
    domains::assets_cloud::scene::restore(lua,memory_category)?;
    result
}

// ------------------------------------------------------- client ↔ server

/// Converts a Lua value for the other VM. Functions, coroutines and other
/// non-transferable values become nil, as with Roblox remotes.
pub fn export(v: &Value, depth: usize) -> Xfer {
    match v {
        Value::Nil => Xfer::Nil,
        Value::Boolean(b) => Xfer::Bool(*b),
        Value::Number(n) => Xfer::Num(*n),
        Value::Integer(i) => Xfer::Num(*i as f64),
        Value::String(s) => Xfer::Str(s.as_bytes().to_vec()),
        Value::Buffer(b) => Xfer::Buffer(b.to_vec()),
        Value::Vector(_) => Xfer::V3(crate::ltypes::v3_opt(v).map(|x| x.0).unwrap_or_default()),
        Value::Table(t) if depth < 32 => {
            let mut out = Vec::new();
            for (k, v) in t.pairs::<Value, Value>().flatten() {
                out.push((export(&k, depth + 1), export(&v, depth + 1)));
            }
            Xfer::Table(out)
        }
        Value::UserData(_) => {
            if let Some(r) = opt_inst(v) {
                Xfer::Inst(r)
            } else if let Some(x) = opt_ud::<LVector3>(v) {
                Xfer::V3(x.0)
            } else if let Some(x) = opt_ud::<LVector2>(v) {
                Xfer::V2(x.0)
            } else if let Some(x) = opt_ud::<LCFrame>(v) {
                Xfer::CF(x.0)
            } else if let Some(x) = opt_ud::<LColor3>(v) {
                Xfer::C3(x.0)
            } else if let Some(x) = opt_ud::<LBrickColor>(v) {
                Xfer::Brick(x.0)
            } else if let Some(x) = opt_ud::<LUDim>(v) {
                Xfer::UDim(x)
            } else if let Some(x) = opt_ud::<LUDim2>(v) {
                Xfer::UDim2(x)
            } else if let Some(x) = opt_ud::<LEnumItem>(v) {
                Xfer::EnumItem(x)
            } else {
                Xfer::Nil
            }
        }
        _ => Xfer::Nil,
    }
}

/// Rebuilds a transferred value in this VM; `map` turns the sender's
/// instance refs into ours (unknown instances arrive as nil).
pub fn import(lua: &Lua, x: &Xfer, map: &dyn Fn(Ref) -> Option<Ref>) -> mlua::Result<Value> {
    let ud = |u: AnyUserData| Value::UserData(u);
    Ok(match x {
        Xfer::Nil => Value::Nil,
        Xfer::Bool(b) => Value::Boolean(*b),
        Xfer::Num(n) => Value::Number(*n),
        Xfer::Str(s) => Value::String(lua.create_string(s)?),
        Xfer::Buffer(b) => Value::Buffer(lua.create_buffer(b)?),
        Xfer::Table(items) => {
            let t = lua.create_table()?;
            for (k, v) in items {
                let k = import(lua, k, map)?;
                if !k.is_nil() {
                    t.raw_set(k, import(lua, v, map)?)?;
                }
            }
            Value::Table(t)
        }
        Xfer::Inst(r) => match map(*r) {
            Some(m) => inst_value(lua, m)?,
            None => Value::Nil,
        },
        Xfer::V3(v) => LVector3(*v).into_value(),
        Xfer::V2(v) => ud(lua.create_userdata(LVector2(*v))?),
        Xfer::CF(c) => ud(lua.create_userdata(LCFrame(*c))?),
        Xfer::C3(c) => ud(lua.create_userdata(LColor3(*c))?),
        Xfer::Brick(b) => ud(lua.create_userdata(LBrickColor(*b))?),
        Xfer::UDim(u) => ud(lua.create_userdata(*u)?),
        Xfer::UDim2(u) => ud(lua.create_userdata(*u)?),
        Xfer::EnumItem(e) => ud(lua.create_userdata(e.clone())?),
    })
}

/// Coroutines waiting in InvokeServer, by request id.
#[derive(Default)]
struct Invokes {
    waiting: HashMap<u64, Thread>,
    next: u64,
}

const INVOKE_PRELUDE: &str = r##"
local yield, running = coroutine.yield, coroutine.running
function __rs_lua_methods.InvokeServer(self, ...)
    __rs_member_check(self, "RemoteFunction", "InvokeServer")
    if __rs_has_peer() then
        __rs_invoke_server(self, running(), ...)
        return yield()
    end
    local cb = __rs_get_callback(self, "OnServerInvoke")
    if not cb then error("OnServerInvoke is not set", 2) end
    return cb(game:GetService("Players").LocalPlayer, ...)
end
function __rs_lua_methods.InvokeClient(self, player, ...)
    __rs_member_check(self, "RemoteFunction", "InvokeClient")
    if typeof(player) ~= "Instance" or not player:IsA("Player") then
        error("Unable to cast value to Object", 2)
    end
    if not __rs_has_peer() then
        error("InvokeClient can only be called from the server", 2)
    end
    __rs_invoke_server(self, running(), ...)
    return yield()
end
-- Client side of InvokeClient: like Roblox, the call is held until
-- OnClientInvoke is set.
function __rs_serve_client_invoke(remote, id, ...)
    local cb = __rs_get_callback(remote, "OnClientInvoke")
    while not cb do
        task.wait()
        if remote.Parent == nil then return end
        cb = __rs_get_callback(remote, "OnClientInvoke")
    end
    local res = table.pack(pcall(cb, ...))
    if res[1] then
        __rs_invoke_reply(id, true, table.unpack(res, 2, res.n))
    else
        __rs_invoke_reply(id, false, tostring(res[2]))
    end
end
function __rs_serve_invoke(remote, player, id, ...)
    local cb = __rs_get_callback(remote, "OnServerInvoke")
    if not cb then
        __rs_invoke_reply(id, false, "OnServerInvoke is not set")
        return
    end
    local res = table.pack(pcall(cb, player, ...))
    if res[1] then
        __rs_invoke_reply(id, true, table.unpack(res, 2, res.n))
    else
        __rs_invoke_reply(id, false, tostring(res[2]))
    end
end
"##;

impl Runtime {
    /// Marks this runtime as one side of a two-DataModel Play session.
    pub fn install_side(&self) -> mlua::Result<()> {
        let lua = &self.lua;
        lua.set_app_data(Invokes::default());
        let g = lua.globals();
        g.set(
            "__rs_has_peer",
            lua.create_function(|lua, ()| Ok(world(lua)?.has_peer))?,
        )?;
        g.set(
            "__rs_invoke_server",
            lua.create_function(|lua, args: MultiValue| {
                let mut it = args.into_iter();
                let remote = it
                    .next()
                    .and_then(|v| opt_inst(&v))
                    .ok_or_else(|| rt_err("InvokeServer: bad remote"))?;
                let th = match it.next() {
                    Some(Value::Thread(t)) => t,
                    _ => return Err(rt_err("InvokeServer must be called from a script thread")),
                };
                let payload: Vec<Xfer> = it.map(|v| export(&v, 0)).collect();
                let id = {
                    let mut inv = lua
                        .app_data_mut::<Invokes>()
                        .ok_or_else(|| rt_err("invokes missing"))?;
                    inv.next += 1;
                    let id = inv.next;
                    inv.waiting.insert(id, th);
                    id
                };
                world(lua)?.outbox.push(RemoteMsg {
                    inst: remote,
                    kind: RemoteKind::Invoke(id),
                    args: payload,
                });
                Ok(())
            })?,
        )?;
        g.set(
            "__rs_invoke_reply",
            lua.create_function(|lua, args: MultiValue| {
                let mut it = args.into_iter();
                let id = it.next().and_then(|v| num(&v)).unwrap_or(0.0) as u64;
                let ok = matches!(it.next(), Some(Value::Boolean(true)));
                let payload: Vec<Xfer> = it.map(|v| export(&v, 0)).collect();
                world(lua)?.outbox.push(RemoteMsg {
                    inst: Ref::none(),
                    kind: RemoteKind::Reply(id, ok),
                    args: payload,
                });
                Ok(())
            })?,
        )?;
        lua.load(INVOKE_PRELUDE).set_name("=rustudio.invoke").exec()
    }

    /// Fires `event` on a remote here, with arguments from the other side.
    pub fn deliver_event(
        &self,
        remote: Ref,
        event: &str,
        player: Option<Ref>,
        args: &[Xfer],
        map: &dyn Fn(Ref) -> Option<Ref>,
    ) {
        let lua = &self.lua;
        let mut vals = Vec::new();
        if let Some(p) = player {
            vals.push(self.inst(p));
        }
        for a in args {
            match import(lua, a, map) {
                Ok(v) => vals.push(v),
                Err(e) => report(lua, &e),
            }
        }
        self.fire(remote, event, vals);
    }

    /// Runs OnServerInvoke for a client's InvokeServer.
    pub fn serve_invoke(
        &self,
        remote: Ref,
        player: Ref,
        id: u64,
        args: &[Xfer],
        map: &dyn Fn(Ref) -> Option<Ref>,
    ) {
        let lua = &self.lua;
        let run = || -> mlua::Result<()> {
            let f: Function = lua.globals().get("__rs_serve_invoke")?;
            let th = domains::assets_cloud::scene::create_thread(lua,f)?;
            let mut vals = vec![
                self.inst(remote),
                self.inst(player),
                Value::Number(id as f64),
            ];
            for a in args {
                vals.push(import(lua, a, map)?);
            }
            resume(lua, &th, vals);
            Ok(())
        };
        if let Err(e) = run() {
            report(lua, &e);
        }
    }

    /// Runs OnClientInvoke for the server's InvokeClient (the reply waits
    /// until a callback is set).
    pub fn serve_client_invoke(
        &self,
        remote: Ref,
        id: u64,
        args: &[Xfer],
        map: &dyn Fn(Ref) -> Option<Ref>,
    ) {
        let lua = &self.lua;
        let run = || -> mlua::Result<()> {
            let f: Function = lua.globals().get("__rs_serve_client_invoke")?;
            let th = domains::assets_cloud::scene::create_thread(lua,f)?;
            let mut vals = vec![self.inst(remote), Value::Number(id as f64)];
            for a in args {
                vals.push(import(lua, a, map)?);
            }
            resume(lua, &th, vals);
            Ok(())
        };
        if let Err(e) = run() {
            report(lua, &e);
        }
    }

    /// Resumes the InvokeServer / InvokeClient caller with the other side's answer.
    pub fn finish_invoke(
        &self,
        id: u64,
        ok: bool,
        args: &[Xfer],
        map: &dyn Fn(Ref) -> Option<Ref>,
    ) {
        let lua = &self.lua;
        let Some(th) = lua.app_data_mut::<Invokes>().and_then(|mut i| {
            let t = i.waiting.get(&id).cloned();
            i.waiting.retain(|k, _| *k != id);
            t
        }) else {
            return;
        };
        if ok {
            let vals: Vec<Value> = args
                .iter()
                .filter_map(|a| import(lua, a, map).ok())
                .collect();
            resume(lua, &th, vals);
        } else {
            let msg = match args.first() {
                Some(Xfer::Str(s)) => String::from_utf8_lossy(s).into_owned(),
                _ => "remote invoke failed".into(),
            };
            if th.status() == ThreadStatus::Resumable {
                if let Err(e) = th.resume_error::<MultiValue>(msg) {
                    report(lua, &e);
                }
            }
        }
    }

    /// Sets a property the way a script would (Humanoid.Health → Died etc.).
    /// Runs a library global (`__rs_…`) in a new thread, like task.spawn:
    /// it may yield; errors go to Output.
    pub fn spawn_global(&self, name: &str, args: Vec<Value>) {
        let Ok(f) = self.lua.globals().get::<Function>(name) else {
            return;
        };
        match domains::assets_cloud::scene::create_thread(&self.lua,f) {
            Ok(th) if crate::bench::lua_prof_on() => crate::bench::lua_timed(
                name_key(name),
                || format!("(rustudio) {name}"),
                || resume(&self.lua, &th, args),
            ),
            Ok(th) => resume(&self.lua, &th, args),
            Err(e) => report(&self.lua, &e),
        }
    }

    pub fn set(&self, r: Ref, key: &str, v: Value) {
        if let Err(e) = newindex(&self.lua, r, key, v) {
            report(&self.lua, &e);
        }
    }

    /// Replication: sets a property as the server did (fires Changed here).
    pub fn apply_prop(&self, r: Ref, name: &str, v: Variant) {
        // Attributes: each one that changed fires AttributeChanged (and its
        // GetAttributeChangedSignal), as when a script sets it.
        let changed_attrs: Vec<String> = if name == "Attributes" {
            let old = world(&self.lua)
                .ok()
                .and_then(|w| match w.d().prop(r, "Attributes") {
                    Some(Variant::Attributes(a)) => Some(a),
                    _ => None,
                });
            match (&v, old) {
                (Variant::Attributes(new), old) => {
                    let old = old.unwrap_or_default();
                    let mut keys: Vec<String> = new
                        .iter()
                        .filter(|(k, val)| old.get(k.as_str()) != Some(*val))
                        .map(|(k, _)| k.to_string())
                        .collect();
                    keys.extend(
                        old.iter()
                            .filter(|(k, _)| new.get(k.as_str()).is_none())
                            .map(|(k, _)| k.to_string()),
                    );
                    keys
                }
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        // A Humanoid's Health from the server fires HealthChanged here, and
        // Died when it reaches 0 (Roblox fires both on the clients too).
        let health = (name == "Health")
            .then(|| {
                let w = world(&self.lua).ok()?;
                let d = w.d();
                if d.class_of(r).as_deref() != Some("Humanoid") {
                    return None;
                }
                let old = match d.prop(r, "Health") {
                    Some(Variant::Float32(h)) => h,
                    _ => 100.0,
                };
                match v {
                    Variant::Float32(h) => Some((old, h)),
                    _ => None,
                }
            })
            .flatten();
        if let Err(e) = store_prop(&self.lua, r, name, v) {
            report(&self.lua, &e);
        }
        if let (Some((old, new)), Ok(mut w)) = (health, world(&self.lua)) {
            if new != old {
                w.fire(r, "HealthChanged", vec![Value::Number(new as f64)]);
                if new <= 0.0 && old > 0.0 {
                    w.fire(r, "Died", vec![]);
                }
            }
        }
        if !changed_attrs.is_empty() {
            if let Ok(mut w) = world(&self.lua) {
                for k in changed_attrs {
                    if let Ok(s) = self.lua.create_string(&k) {
                        w.fire(r, "AttributeChanged", vec![Value::String(s)]);
                    }
                    w.fire(r, &format!("AttributeChanged:{k}"), vec![]);
                }
            }
        }
    }

    /// Replication: renames as the server did.
    pub fn apply_name(&self, r: Ref, name: &str) {
        let lua = &self.lua;
        let changed = world(lua).ok().and_then(|mut w| {
            let i = w.dm().dom.get_by_ref_mut(r)?;
            if i.name == name {
                return None;
            }
            i.name = name.to_string();
            w.dm().mark_prop(r, ustr("Name"));
            Some(())
        });
        if changed.is_some() {
            if let (Ok(s), Ok(mut w)) = (lua.create_string("Name"), world(lua)) {
                w.fire(r, "Changed", vec![Value::String(s)]);
                w.fire(r, "Changed:Name", vec![]);
            }
        }
    }

    /// Replication: reparents as the server did (fires ChildAdded etc.).
    pub fn apply_parent(&self, r: Ref, parent: Ref) {
        if let Err(e) = set_parent(&self.lua, r, parent) {
            report(&self.lua, &e);
        }
    }

    /// Replication: the server destroyed `r` (Destroying fires here too,
    /// Parent locks, connections go), as Studio's client does on a
    /// DestroyInstance item.
    pub fn apply_destroy(&self, r: Ref) {
        if let Err(e) = destroy_inst(&self.lua, r) {
            report(&self.lua, &e);
        }
    }
}

/// Compile-only check for the script editor: (line, message) of the first
/// syntax error.
pub fn check_syntax(source: &str, name: &str) -> Option<(usize, String)> {
    luau_flags();
    let lua = Lua::new();
    let err = lua
        .load(source)
        .set_name(format!("={name}"))
        .into_function()
        .err()?;
    let msg = describe_error(&err);
    let line = msg
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix(':'))
        .and_then(|rest| rest.split(':').next())
        .and_then(|n| n.trim().parse::<usize>().ok())
        .unwrap_or(1);
    Some((line, msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<(Level, String)> {
        let mut d = Document::new_place();
        let sss = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        let s = d.dom.insert(
            sss,
            InstanceBuilder::new("Script")
                .with_name("T")
                .with_property("Source", src.to_string()),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        for _ in 0..120 {
            rt.step(1.0 / 60.0);
        }
        rt.leave().output
    }

    #[test]
    fn render_callbacks_run_before_waiting_tasks_resume() {
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(Document::new_place(), RunMode::Run));
        rt.bind_globals().unwrap();
        rt.lua.load(r#"
            local seen = false
            game:GetService("RunService").RenderStepped:Connect(function() seen = true end)
            task.spawn(function() task.wait(); assert(seen, "render callback ran after task.wait") end)
        "#).exec().unwrap();
        let rs = { let w = world(&rt.lua).unwrap(); w.d().find_child(w.d().dom.root_ref(), "RunService").unwrap() };
        rt.fire(rs, "RenderStepped", vec![Value::Number(1.0 / 60.0)]);
        rt.step(1.0 / 60.0);
        let out = rt.leave().output;
        assert!(out.iter().all(|(level, _)| *level != Level::Error), "{out:?}");
    }

    #[test]
    fn value_changed_delivers_typed_value_and_property_signal_has_no_arguments() {
        let out = run(r##"
            local n = Instance.new("NumberValue")
            n.Changed:Connect(function(v) assert(type(v) == "number"); print("value", 1-v) end)
            n:GetPropertyChangedSignal("Value"):Connect(function(...) assert(select("#", ...) == 0); print("property") end)
            n.Value = .25
        "##);
        assert!(out.iter().all(|(level, _)| *level != Level::Error), "{out:?}");
        assert!(out.iter().any(|(_, s)| s.starts_with("value 0.75 ")), "{out:?}");
        assert!(out.iter().any(|(_, s)| s.starts_with("property ")), "{out:?}");
    }

    #[test]
    fn basics() {
        let out = run("local p = Instance.new(\"Part\")\n\
            p.Name = \"X\"\n\
            p.Size = Vector3.new(2, 3, 4)\n\
            p.Parent = workspace\n\
            local hits = 0\n\
            p.Changed:Connect(function(prop) hits += 1 end)\n\
            p.Transparency = 0.5\n\
            print(workspace:FindFirstChild(\"X\") == p, p.Size.Y, typeof(p), p:IsA(\"BasePart\"))\n\
            local t = task.wait(0.5)\n\
            print(\"waited\", t >= 0.49, hits)\n\
            print(tostring(CFrame.new(1, 2, 3) * Vector3.new(1, 0, 0)), Enum.Material.Neon.Value)\n\
            local ok, err = pcall(function() return p.Nope end)\n\
            print(ok, err)\n\
            error(\"boom\")\n");
        let text: Vec<&str> = out.iter().map(|(_, s)| s.as_str()).collect();
        assert!(text[0].starts_with("true 3 Instance true"), "{text:?}");
        assert!(text[0].contains("ServerScriptService.T:8"), "{text:?}");
        assert!(text[1].starts_with("waited true 1"), "{text:?}");
        assert!(text[2].starts_with("2, 2, 3 288"), "{text:?}");
        assert!(
            text[3].contains("Nope is not a valid member of Part"),
            "{text:?}"
        );
        assert!(
            out[4].0 == Level::Error && out[4].1.contains("ServerScriptService.T:14: boom"),
            "{out:?}"
        );
    }

    /// An error lists its Luau frames the way Studio's Output does.
    #[test]
    fn error_stack_lines() {
        let out = run("local function inner()\n\terror(\"deep\")\nend\nlocal function outer()\n\tinner()\nend\nouter()\n");
        let text: Vec<String> = out.iter().map(|(_, s)| s.replace(crate::output_panel::STACK_MARK, "")).collect();
        let at = text.iter().position(|t| t == "Stack Begin").expect("stack");
        assert_eq!(
            &text[at..at + 5],
            &[
                "Stack Begin".to_string(),
                "Script 'ServerScriptService.T', Line 2 - function inner  —  ServerScriptService.T:2".to_string(),
                "Script 'ServerScriptService.T', Line 5 - function outer  —  ServerScriptService.T:5".to_string(),
                "Script 'ServerScriptService.T', Line 7  —  ServerScriptService.T:7".to_string(),
                "Stack End".to_string(),
            ],
            "{text:?}"
        );
    }

    #[test]
    fn modules_and_signals() {
        let mut d = Document::new_place();
        let rs = d.find_child(d.dom.root_ref(), "ReplicatedStorage").unwrap();
        d.dom.insert(
            rs,
            InstanceBuilder::new("ModuleScript")
                .with_name("Lib")
                .with_property(
                    "Source",
                    "return { add = function(a, b) return a + b end }".to_string(),
                ),
        );
        let sss = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        let s = d.dom.insert(
            sss,
            InstanceBuilder::new("Script").with_name("T").with_property(
                "Source",
                "local Lib = require(game:GetService('ReplicatedStorage'):WaitForChild('Lib'))\n\
                 local ev = Instance.new('BindableEvent')\n\
                 task.delay(0.1, function() ev:Fire('hi', 7) end)\n\
                 local a, b = ev.Event:Wait()\n\
                 print(Lib.add(2, 3), a, b)"
                    .to_string(),
            ),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        for _ in 0..30 {
            rt.step(1.0 / 60.0);
        }
        let out = rt.leave().output;
        assert!(out.iter().any(|(_, s)| s.starts_with("5 hi 7")), "{out:?}");
    }

    #[test]
    fn timeout() {
        let out = run("while true do end");
        assert!(
            out.iter().any(|(_, s)| s.contains("Script timeout")),
            "{out:?}"
        );
    }
}

#[cfg(test)]
mod legacy_tests {
    use super::*;

    fn run(src: &str) -> Vec<(Level, String)> {
        run_in(Document::new_place(), src)
    }

    /// `run` with a Player in Players (scripts can't create one).
    fn run_with_player(src: &str) -> Vec<(Level, String)> {
        let mut d = Document::new_place();
        let root = d.dom.root_ref();
        let players = match d.find_child(root, "Players") {
            Some(p) => p,
            None => d.dom.insert(root, InstanceBuilder::new("Players").with_name("Players")),
        };
        d.dom.insert(players, InstanceBuilder::new("Player").with_name("Player"));
        run_in(d, src)
    }

    fn run_in(mut d: Document, src: &str) -> Vec<(Level, String)> {
        let sss = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        let s = d.dom.insert(
            sss,
            InstanceBuilder::new("Script")
                .with_name("T")
                .with_property("Source", src.to_string()),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        for _ in 0..120 {
            // Tweens advance on Heartbeat, as in a play session.
            let rs = rt
                .world_mut()
                .and_then(|w| w.d().find_child(w.d().dom.root_ref(), "RunService"));
            if let Some(rs) = rs {
                rt.fire(rs, "Heartbeat", vec![Value::Number(1.0 / 60.0)]);
            }
            rt.step(1.0 / 60.0);
        }
        rt.leave().output
    }

    /// Round 12's members, against Roblox's documented results.
    #[test]
    fn round12_api_members() {
        let out = run("local r = workspace:Spherecast(Vector3.new(50, 20, 50), 2, Vector3.new(0, -50, 0))\n\
            print('sphere', r.Instance.Name, math.floor(r.Position.Y * 100 + 0.5) / 100, r.Normal.Y, math.floor(r.Distance * 100 + 0.5) / 100)\n\
            local b = workspace:Blockcast(CFrame.new(50, 20, 50), Vector3.new(4, 4, 4), Vector3.new(0, -50, 0))\n\
            print('block', b.Instance.Name, math.floor(b.Distance * 100 + 0.5) / 100)\n\
            local p = Instance.new('Part'); p.Anchored = true; p.Size = Vector3.new(4, 4, 4); p.Position = Vector3.new(-50, 2, 0); p.Parent = workspace\n\
            local c = p:GetClosestPointOnSurface(Vector3.new(-40, 3, 0))\n\
            print('closest', c.X, c.Y, c.Z)\n\
            local m = Instance.new('Model'); p.Parent = m; m.Parent = workspace\n\
            print('pivot', m.WorldPivot.Position.X)\n\
            local f = Instance.new('Frame')\n\
            print('tween', f:TweenSize(UDim2.new(0, 100, 0, 50), 'Out', 'Quad', 0.1), f:TweenSize(UDim2.new(0, 10, 0, 5), 'Out', 'Quad', 0.1, false))\n\
            print('limb', Instance.new('Humanoid'):GetLimb(Instance.new('Part')).Name)\n\
            print('groups', workspace:GetMaxCollisionGroups(), game:GetService('PhysicsService'):GetCollisionGroupName(0))\n\
            print('sun', math.floor(game.Lighting:GetSunDirection().Magnitude * 100 + 0.5) / 100)\n\
            print('job', game.JobId == '', game.PlaceId)\n\
            local keep = Instance.new('Folder'); local drop = Instance.new('Part', keep); drop.Archivable = false; Instance.new('Part', keep)\n\
            print('archivable', #keep:Clone():GetChildren())\n\
            print('limits', select(2, pcall(workspace.Spherecast, workspace, Vector3.zero, 300, Vector3.new(0, -1, 0))))\n\
            print('limits', select(2, pcall(workspace.Spherecast, workspace, Vector3.zero, 1, Vector3.new(0, -2000, 0))))\n\
            print('limits', select(2, pcall(workspace.Spherecast, workspace, Vector3.zero, -1, Vector3.new(0, -1, 0))))\n");
        let text: Vec<&str> = out.iter().map(|(_, t)| t.as_str()).collect();
        let has = |s: &str| text.iter().any(|t| t.starts_with(s));
        assert!(has("sphere Baseplate 0 1 18"), "{text:?}");
        assert!(has("block Baseplate 18"), "{text:?}");
        assert!(has("closest -48 3 0"), "{text:?}");
        assert!(has("pivot -50"), "{text:?}");
        assert!(has("tween true false"), "{text:?}");
        assert!(has("limb Unknown"), "{text:?}");
        assert!(has("groups 32 Default"), "{text:?}");
        assert!(has("sun 1"), "{text:?}");
        assert!(has("job true 0"), "{text:?}");
        assert!(has("archivable 1"), "{text:?}");
        // Studio's own messages: max radius 256 = ShapecastMaxSize 512 / 2.
        let lim = |s: &str| text.iter().any(|t| t.starts_with("limits") && t.contains(s));
        assert!(lim("The maximum radius is 256.000000."), "{text:?}");
        assert!(lim("Attempt to shapecast with distance 2000.000000. The maximum distance is 1024."), "{text:?}");
        assert!(lim("Invalid Radius. Must be greater than or equal to 0, got -1.000000."), "{text:?}");
    }

    #[test]
    fn humanoid_health_and_legacy_names() {
        let out = run("local h = Instance.new(\"Humanoid\")\n\
            print(h.Health, h.MaxHealth, h.WalkSpeed)\n\
            print(script.parent == script.Parent, game.workspace == workspace, workspace:findFirstChild(\"Baseplate\") ~= nil)\n\
            local p = Instance.new(\"Part\", workspace)\n\
            local c = p:clone()\n\
            print(c.className, #workspace:getChildren() > 0, p:isA(\"BasePart\"))\n\
            p:remove()\n\
            print(p.Parent)\n\
            local r = Ray.new(Vector3.new(0, 50, 0), Vector3.new(0, -100, 0))\n\
            local hit, pos = workspace:FindPartOnRay(r)\n\
            print(hit and hit.Name, math.floor(pos.Y + 0.5))\n\
            local res = workspace:Raycast(Vector3.new(0, 50, 0), Vector3.new(0, -100, 0))\n\
            print(res.Instance.Name, res.Normal.Y)\n\
            local cs = game:GetService(\"CollectionService\")\n\
            cs:AddTag(workspace.Baseplate, \"Floor\")\n\
            print(#cs:GetTagged(\"Floor\"), workspace.Baseplate:HasTag(\"Floor\"))\n\
            local hs = game:GetService(\"HttpService\")\n\
            print(hs:JSONEncode({a = 1}), hs:JSONDecode('[1,2,3]')[3])\n\
            local t = game:GetService(\"TweenService\"):Create(workspace.Baseplate, TweenInfo.new(0.2), {Transparency = 1})\n\
            t.Completed:Connect(function(state) print(\"tween\", state.Name, workspace.Baseplate.Transparency) end)\n\
            t:Play()\n\
            local part = Instance.new(\"Part\", workspace)\n\
            game:GetService(\"Debris\"):AddItem(part, 0.5)\n\
            delay(1, function() print(\"debris\", part.Parent) end)\n\
            local okds, errds = pcall(function() return game:GetService(\"DataStoreService\"):GetDataStore(\"x\") end)\n\
            print(okds, string.find(tostring(errds), \"publish this place\") ~= nil)\n\
            print(typeof(NumberSequence.new(1)), typeof(Region3.new(Vector3.new(), Vector3.one)), ypcall ~= nil, version() ~= nil)\n");
        let text: Vec<String> = out
            .iter()
            .map(|(_, t)| t.split("  —").next().unwrap_or("").to_string())
            .collect();
        let errors: Vec<&(Level, String)> =
            out.iter().filter(|(l, _)| *l == Level::Error).collect();
        assert!(errors.is_empty(), "{out:?}");
        assert_eq!(text[0], "100 100 16", "{text:?}");
        assert_eq!(text[1], "true true true", "{text:?}");
        assert_eq!(text[2], "Part true true", "{text:?}");
        assert_eq!(text[3], "nil", "{text:?}");
        assert_eq!(text[4], "SpawnLocation 1", "{text:?}");
        assert_eq!(text[5], "SpawnLocation 1", "{text:?}");
        assert_eq!(text[6], "1 true", "{text:?}");
        assert_eq!(text[7], "{\"a\":1} 3", "{text:?}");
        assert!(
            text.iter().any(|t| t == "false true"),
            "unpublished DataStore errors like Studio: {text:?}"
        );
        assert!(
            text.iter().any(|t| t == "NumberSequence Region3 true true"),
            "{text:?}"
        );
        assert!(text.iter().any(|t| t == "tween Completed 1"), "{text:?}");
        assert!(text.iter().any(|t| t == "debris nil"), "{text:?}");
    }

    /// Require by string (another admin script's `require("./String")`) and a
    /// types-only module that ends `return nil` (round 25).
    #[test]
    fn require_by_string_and_nil_modules() {
        let module = |name: &str, src: &str| {
            InstanceBuilder::new("ModuleScript")
                .with_name(name)
                .with_property("Source", src.to_string())
        };
        let mut d = Document::new_place();
        let sss = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        let src = "local a = require('@self/A')\n\
            print('req', a.s, a.b, a.t == nil, a.t2 == nil)\n\
            print('missing', select(2, pcall(function() return require('@self/Missing') end)))\n\
            print('noret', select(2, pcall(function() return require('@self/NoRet') end)))\n\
            print('prefix', select(2, pcall(function() return require('A') end)))";
        let s = d.dom.insert(
            sss,
            InstanceBuilder::new("Script")
                .with_name("T")
                .with_property("Source", src.to_string())
                .with_child(
                    module("A", "return { s = require('./S'), b = require('@self/B'), t = require('@self/Types'), t2 = require('@self/Types') }")
                        .with_child(module("B", "return require('../S') .. '+b'"))
                        .with_child(module("Types", "export type X = number\nreturn nil")),
                )
                .with_child(module("S", "return 'sib'"))
                .with_child(module("NoRet", "local x = 1")),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        rt.step(1.0 / 60.0);
        let out = rt.leave().output;
        let text: Vec<&str> = out.iter().map(|(_, t)| t.as_str()).collect();
        assert!(text.iter().any(|t| t.starts_with("req sib sib+b true true")), "{text:?}");
        assert!(
            text.iter().any(|t| t.starts_with("missing") && t.contains("could not resolve child component \"Missing\"")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|t| t.starts_with("noret") && t.contains("did not return exactly one value")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|t| t.starts_with("prefix") && t.contains("must start with a valid prefix")),
            "{text:?}"
        );
    }

    /// A disabled script starts when a script enables it (round 26:
    /// Place B Newer's music GUI Handler), once.
    #[test]
    fn enabling_a_script_starts_it() {
        let mut d = Document::new_place();
        let sss = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        d.dom.insert(
            sss,
            InstanceBuilder::new("Script")
                .with_name("Later")
                .with_property("Disabled", true)
                .with_property("Source", "print('later ran')".to_string()),
        );
        let s = d.dom.insert(
            sss,
            InstanceBuilder::new("Script").with_name("T").with_property(
                "Source",
                "local l = script.Parent.Later\nprint('before', l.Enabled)\nl.Enabled = true\nl.Enabled = false\nl.Enabled = true\nprint('state', l.Enabled, l.Disabled)"
                    .to_string(),
            ),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        rt.step(1.0 / 60.0);
        let out = rt.leave().output;
        let ran = out.iter().filter(|(_, t)| t.starts_with("later ran")).count();
        assert_eq!(ran, 1, "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("before false")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("state true false")), "{out:?}");
    }

    /// An event is a userdata, typeof "RBXScriptSignal", and a bool property takes any
    /// value by truthiness, as in Roblox (round 26:
    /// another admin script's Flux connects FocusLost by typeof, binds Visible to a number).
    #[test]
    fn signal_typeof() {
        let mut d = Document::new_place();
        let sss = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        let s = d.dom.insert(
            sss,
            InstanceBuilder::new("Script").with_name("T").with_property(
                "Source",
                "local b = Instance.new('TextBox')\n\
                 local p = {FocusLost = function() end, Text = 'x'}\n\
                 for k, v in p do\n\
                     if typeof(b[k]) == 'RBXScriptSignal' then b[k]:Connect(v) else b[k] = v end\n\
                 end\n\
                 print('types', type(b.FocusLost), tostring(b.FocusLost), b.FocusLost == b.FocusLost, typeof(b.FocusLost), typeof(b.FocusLost:Connect(print)), typeof({}), typeof(b), b.Text)\n\
                 b.Visible = 0; local a = b.Visible; b.Visible = nil\n\
                 print('bool', a, b.Visible)"
                    .to_string(),
            ),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        rt.step(1.0 / 60.0);
        let out = rt.leave().output;
        assert!(
            out.iter()
                .any(|(_, t)| t.starts_with("types userdata Signal FocusLost true RBXScriptSignal RBXScriptConnection table Instance x")),
            "{out:?}"
        );
        assert!(out.iter().any(|(_, t)| t.starts_with("bool true false")), "{out:?}");
    }

    /// A legacy Script in a player's PlayerGui runs on the server, as in
    /// Studio (round 26: Place B Newer's "server version" label).
    #[test]
    fn player_gui_scripts_run() {
        let mut d = Document::new_place();
        let root = d.dom.root_ref();
        let players = match d.find_child(root, "Players") {
            Some(p) => p,
            None => d.dom.insert(root, InstanceBuilder::new("Players").with_name("Players")),
        };
        let p = d.dom.insert(players, InstanceBuilder::new("Player").with_name("Player"));
        let gui = d.dom.insert(p, InstanceBuilder::new("PlayerGui").with_name("PlayerGui"));
        d.dom.insert(
            gui,
            InstanceBuilder::new("Script")
                .with_name("version")
                .with_property("Disabled", true)
                .with_property("Source", "print('gui script ran')".to_string()),
        );
        let sss = d.find_child(root, "ServerScriptService").unwrap();
        let s = d.dom.insert(
            sss,
            InstanceBuilder::new("Script").with_name("T").with_property(
                "Source",
                "game.Players.Player.PlayerGui.version.Enabled = true".to_string(),
            ),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        rt.step(1.0 / 60.0);
        let out = rt.leave().output;
        assert!(out.iter().any(|(_, t)| t.starts_with("gui script ran")), "{out:?}");
    }

    /// Round 26's library gaps from Place D's second admin script and Place B's
    /// an admin script, on the client: settings() refused to a game script,
    /// TextService filtering refused on the client, a CoreGuiType by name,
    /// and a humanoid model with its Animator.
    #[test]
    fn script_library_gaps() {
        let mut d = Document::new_place();
        let sp = d.find_child(d.dom.root_ref(), "StarterPlayer").unwrap();
        let sps = d.find_child(sp, "StarterPlayerScripts").unwrap();
        let s = d.dom.insert(
            sps,
            InstanceBuilder::new("LocalScript").with_name("T").with_property(
                "Source",
                "print('settings', pcall(settings))\n\
                 print('filter', pcall(function() return game:GetService('TextService'):FilterStringAsync('hello', 1) end))\n\
                 local sg = game:GetService('StarterGui')\n\
                 sg:SetCoreGuiEnabled('Chat', false)\n\
                 print('coregui', sg:GetCoreGuiEnabled('Chat'), sg:GetCoreGuiEnabled(Enum.CoreGuiType.Chat), sg:GetCoreGuiEnabled(Enum.CoreGuiType.Backpack))\n\
                 local m = game:GetService('Players'):CreateHumanoidModelFromUserId(1)\n\
                 print('animator', m.Humanoid:FindFirstChild('Animator') ~= nil)"
                    .to_string(),
            ),
        );
        let rt = Runtime::new().unwrap();
        let mut w = World::new(d, RunMode::Run);
        w.side = Side::Client;
        rt.enter(w);
        rt.bind_globals().unwrap();
        rt.run_script(s);
        rt.step(1.0 / 60.0);
        let out = rt.leave().output;
        let has = |p: &str| out.iter().any(|(_, t)| t.starts_with(p));
        assert!(
            has("settings false The current thread cannot call 'settings' (lacking capability Plugin)"),
            "{out:?}"
        );
        // Server-only in Roblox: a LocalScript gets the engine's error.
        assert!(has("filter false Must only be called from server scripts"), "{out:?}");
        assert!(has("coregui false false true"), "{out:?}");
        assert!(has("animator true"), "{out:?}");
    }

    /// an admin script's client anti-tamper, condensed (Client.Core.Anti and
    /// Shared.Service ReadOnly): settings() refused inside a metamethod
    /// (rt0001 kicked every Play), Instance/Enum metamethods are unnamed C
    /// functions raising the same string each time, and a Lua-written
    /// method called with the wrong self says "Expected ':' not '.'".
    #[test]
    fn adonis_tamper_checks() {
        let mut d = Document::new_place();
        let sp = d.find_child(d.dom.root_ref(), "StarterPlayer").unwrap();
        let sps = d.find_child(sp, "StarterPlayerScripts").unwrap();
        let src = r#"
            local ro = setmetatable({}, { __index = function() return pcall(settings) end })
            print("ro", ro.x)
            local function mm(f)
                local m
                xpcall(f, function() m = debug.info(2, "f") end)
                local ok1, e1 = pcall(m)
                local ok2, e2 = pcall(m)
                local parts = { debug.info(m, "s"), debug.info(m, "l"), debug.info(m, "n") == "",
                    debug.info(m, "a"), ok1, type(e1), rawequal(e1, e2) }
                for i, v in parts do parts[i] = tostring(v) end
                return table.concat(parts, " ")
            end
            print("idx", mm(function() local c = game.____________ end))
            print("nidx", mm(function() game.____________ = 5 end))
            print("ncall", mm(function() game:____________() end))
            print("eidx", mm(function() local c = Enum.HumanoidStateType.____________ end))
            print("rf", pcall(Instance.new("RemoteFunction").InvokeServer, workspace))
        "#;
        let s = d.dom.insert(
            sps,
            InstanceBuilder::new("LocalScript")
                .with_name("T")
                .with_property("Source", src.to_string()),
        );
        let rt = Runtime::new().unwrap();
        let mut w = World::new(d, RunMode::Run);
        w.side = Side::Client;
        rt.enter(w);
        rt.bind_globals().unwrap();
        rt.run_script(s);
        rt.step(1.0 / 60.0);
        let out = rt.leave().output;
        let text: Vec<String> = out
            .iter()
            .map(|(_, t)| t.split("  —").next().unwrap_or("").to_string())
            .collect();
        for want in [
            "ro false",
            "idx [C] -1 true 0 false string true",
            "nidx [C] -1 true 0 false string true",
            "ncall [C] -1 true 0 false string true",
            "eidx [C] -1 true 0 false string true",
            "rf false Expected ':' not '.' calling member function InvokeServer",
        ] {
            assert!(text.iter().any(|t| t.starts_with(want)), "{want}: {text:?}");
        }
    }

    /// Engine errors caught by pcall are the engine's text without a
    /// position (round 26). (GetProductInfo, once tested here, is
    /// web_social_http's now: domains/web_social.rs tests it.)
    #[test]
    fn engine_error_text() {
        let mut d = Document::new_place();
        let sss = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        let src = "print('member', pcall(function() return workspace.Nope end))\n\
            print('dot', pcall(function() game:GetService('Players'):FindFirstChildOfClass('Player').Kick(workspace, 'x') end))\n\
            print('wskick', pcall(function() workspace:Kick('x') end))\n\
            local tr = game:GetService('LocalizationService'):GetTranslatorForLocaleAsync('en-us')\n\
            print('tr', tr.ClassName, tr.LocaleId, tr:Translate(game, 'hi'), tr:GetPropertyChangedSignal('LocaleId'):Connect(print).Connected)\n\
            print('ds', pcall(function() return game:GetService('DataStoreService'):GetDataStore('x') end))";
        let s = d.dom.insert(
            sss,
            InstanceBuilder::new("Script")
                .with_name("T")
                .with_property("Source", src.to_string()),
        );
        // Game scripts can't make a Player (WritePlayer); the test places one.
        let players = d.find_child(d.dom.root_ref(), "Players").unwrap();
        d.dom.insert(players, InstanceBuilder::new("Player").with_name("P"));
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        for _ in 0..10 {
            rt.step(1.0 / 60.0);
        }
        let out = rt.leave().output;
        let text: Vec<String> = out
            .iter()
            .map(|(_, t)| t.split("  —").next().unwrap_or("").to_string())
            .collect();
        assert!(
            text.iter().any(|t| t == "member false Nope is not a valid member of Workspace \"Workspace\""),
            "{text:?}"
        );
        assert!(text.iter().any(|t| t == "tr Translator en-us hi true"), "{text:?}");
        // an admin script's anti-kick check, word for word.
        assert!(
            text.iter().any(|t| t == "dot false Expected ':' not '.' calling member function Kick"),
            "{text:?}"
        );
        assert!(
            text.iter().any(|t| t == "wskick false Kick is not a valid member of Workspace \"Workspace\""),
            "{text:?}"
        );
        assert!(
            text.iter().any(|t| t == "ds false You must publish this place to the web to access DataStore."),
            "{text:?}"
        );
    }
    /// ClearAllChildren destroys the last child first, as Studio's
    /// Instance::clearAllChildren; Destroy cuts the whole
    /// subtree loose, parent before children, and locks every Parent.
    #[test]
    fn clear_all_children_last_first_and_destroy_detaches_all() {
        let out = run("local f = Instance.new('Folder', workspace)\n\
            for _, n in {'A', 'B', 'C'} do local x = Instance.new('Folder') x.Name = n x.Parent = f end\n\
            local order = {}\n\
            workspace.DescendantRemoving:Connect(function(d) table.insert(order, d.Name) end)\n\
            f:ClearAllChildren()\n\
            task.wait()\n\
            print('order', table.concat(order, ','))\n\
            local m = Instance.new('Model')\n\
            local a = Instance.new('Folder', m) local b = Instance.new('Folder', a) local c = Instance.new('Part', b)\n\
            for i = 1, 50 do Instance.new('Part', m) end\n\
            m.Parent = workspace\n\
            m:Destroy()\n\
            print('detached', m.Parent == nil, a.Parent == nil, b.Parent == nil, c.Parent == nil, #m:GetChildren(), #a:GetChildren(), #b:GetChildren())\n\
            print('locked', pcall(function() c.Parent = workspace end))\n\
            print('again', pcall(function() b:Destroy() end))\n");
        assert!(out.iter().any(|(_, t)| t.starts_with("order C,B,A")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("detached true true true true 0 0 0")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("locked false")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("again true")), "{out:?}");
    }

    /// class_signals: DescendantAdded / Removing on game for the watched
    /// classes only (IsA), on parenting in, reparenting and Destroy.
    #[test]
    fn class_signals_fire_only_for_watched_classes() {
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(Document::new_place(), RunMode::Run));
        rt.bind_globals().unwrap();
        let (added, removing) = class_signals(&rt.lua, vec!["ProximityPrompt".into(), "HandleAdornment".into()]).unwrap();
        rt.lua.globals().set("added_sig", added).unwrap();
        rt.lua.globals().set("removing_sig", removing).unwrap();
        rt.lua.load("_G.log = {}\n\
            added_sig:Connect(function(i) table.insert(_G.log, '+' .. i.Name) end)\n\
            removing_sig:Connect(function(i) table.insert(_G.log, '-' .. i.Name) end)\n\
            local m = Instance.new('Model') m.Name = 'M'\n\
            for i = 1, 20 do Instance.new('Part', m) end\n\
            local pp = Instance.new('ProximityPrompt') pp.Name = 'PP' pp.Parent = m.Part\n\
            local box = Instance.new('BoxHandleAdornment') box.Name = 'Box' box.Parent = m\n\
            m.Parent = workspace\n\
            local f = Instance.new('Folder', workspace) m.Parent = f\n\
            f:Destroy()").exec().unwrap();
        rt.step(1.0 / 60.0);
        let log: String = rt.lua.load("table.sort(_G.log) return table.concat(_G.log, ',')").eval().unwrap();
        assert_eq!(log, "+Box,+Box,+PP,+PP,-Box,-Box,-PP,-PP");
    }

    #[test]
    fn destroyed_parent_nil_is_idempotent() {
        let out = run("local root = Instance.new('Folder', workspace)\n\
            local p = Instance.new('Part', root)\n\
            root:Destroy()\n\
            print('detached', p.Parent == nil)\n\
            print('nilparent', pcall(function() p.Parent = nil end))\n\
            print('reparent', pcall(function() p.Parent = workspace end))\n");
        assert!(out.iter().any(|(_, t)| t.starts_with("detached true")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("nilparent true")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("reparent false")), "{out:?}");
    }

    /// Tagged instances entering / leaving the DataModel fire the tag's
    /// InstanceAdded / InstanceRemoved signals (Place A's box, upgrade machine and
    /// perk lights track replicated map pieces this way); AddTag outside
    /// the DataModel fires nothing until it is parented in. DescendantAdded
    /// fires for every descendant of the moved instance.
    #[test]
    fn tag_signals_follow_data_model_membership() {
        let out = run("local CS = game:GetService('CollectionService')\n\
            local added, removed, desc = {}, {}, {}\n\
            CS:GetInstanceAddedSignal('Box'):Connect(function(i) table.insert(added, i.Name) end)\n\
            CS:GetInstanceRemovedSignal('Box'):Connect(function(i) table.insert(removed, i.Name) end)\n\
            workspace.DescendantAdded:Connect(function(i) table.insert(desc, i.Name) end)\n\
            local m = Instance.new('Model'); m.Name = 'M'\n\
            local p = Instance.new('Part'); p.Name = 'Lid'; p.Parent = m\n\
            p:AddTag('Box')\n\
            print('before', #added)\n\
            m.Parent = workspace\n\
            task.wait()\n\
            print('added', table.concat(added, ','))\n\
            print('desc', table.concat(desc, ','))\n\
            local c = m:Clone(); c.Name = 'C'; c:FindFirstChild('Lid').Name = 'Lid2'; c.Parent = workspace\n\
            m.Parent = nil\n\
            task.wait()\n\
            print('added2', table.concat(added, ','))\n\
            print('removed', table.concat(removed, ','))\n");
        assert!(out.iter().any(|(_, t)| t.starts_with("before 0")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("added Lid")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("desc M,Lid")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("added2 Lid,Lid2")), "{out:?}");
        assert!(out.iter().any(|(_, t)| t.starts_with("removed Lid")), "{out:?}");
    }

    /// A profileend whose begin was before a yield warns, as in Studio
    /// (round 26); a balanced pair doesn't. Player:Move without a
    /// character warns once. Destroy twice is fine.
    #[test]
    fn profile_markers_and_move_warnings() {
        // (A game script can't Instance.new a Player: WritePlayer.)
        let out = run_with_player("debug.profilebegin('a')\n\
            debug.profileend()\n\
            debug.profilebegin('b')\n\
            task.wait()\n\
            debug.profileend()\n\
            local p = game:GetService('Players'):GetPlayers()[1]\n\
            p:Move(Vector3.new(1, 0, 0))\n\
            p:Move(Vector3.new(1, 0, 0))\n\
            local f = Instance.new('Folder')\n\
            local l = Instance.new('PointLight', f)\n\
            f:Destroy()\n\
            print('twice', pcall(l.Destroy, l))\n");
        // Destroying a destroyed instance's descendant is a no-op.
        assert!(out.iter().any(|(_, t)| t.starts_with("twice true")), "{out:?}");
        let warns: Vec<&String> = out.iter().filter(|(l, _)| *l == Level::Warn).map(|(_, t)| t).collect();
        assert_eq!(
            warns.iter().filter(|t| t.starts_with("debug.profileEnd() - No active profile annotation. At: Stack Begin")).count(),
            1,
            "{warns:?}"
        );
        assert_eq!(
            warns.iter().filter(|t| t.starts_with("Player:Move called, but player currently has no character.")).count(),
            1,
            "{warns:?}"
        );
    }
}

#[cfg(test)]
#[path = "selftest/attachment_runtime.rs"]
mod attachment_tests;

#[cfg(test)]
#[path = "selftest/bridge.rs"]
mod bridge_tests;
