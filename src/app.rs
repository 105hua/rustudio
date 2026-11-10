//! The editor: state shared by every panel, menus, toolbar, shortcuts and
//! the docked layout.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use glam::Vec3;
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::InstanceBuilder;

use crate::account_ui::AccountUi;
use crate::camera::Camera;
use crate::doc::{self, DocKind, Document};
use crate::explorer::ExplorerState;
use crate::mech;
use crate::properties::PropertiesState;
use crate::render::{MeshKind, Renderer};
use crate::scene::Scene;
use crate::selftest::SelfTest;
use crate::theme;
use crate::viewport::GizmoDrag;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Move,
    Scale,
    Rotate,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Move => "Move",
            Tool::Scale => "Scale",
            Tool::Rotate => "Rotate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

pub struct LogLine {
    pub at: String,
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Viewport,
    Explorer,
    Properties,
    Output,
    /// Studio's Avatar Settings (File > Avatar Settings).
    AvatarSettings,
    /// Studio's Terrain Editor (Home / View > Terrain Editor).
    TerrainEditor,
    /// A script open in the code editor.
    Script(Ref),
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Viewport => "Viewport",
            Tab::Explorer => "Explorer",
            Tab::Properties => "Properties",
            Tab::Output => "Output",
            Tab::AvatarSettings => "Avatar Settings",
            Tab::TerrainEditor => "Terrain Editor",
            Tab::Script(_) => "Script",
        }
    }
}

/// Counters the self-test reads to know each panel was actually drawn.
#[derive(Default, Debug, Clone)]
pub struct HudStats {
    pub explorer_rows: usize,
    pub property_rows: usize,
    pub output_lines: usize,
    pub viewport_drawn: bool,
    pub toolbar_buttons: usize,
    pub status_drawn: bool,
}

pub struct Harness {
    /// `--shot-after S`: take the screenshot S seconds after start instead of
    /// after SCREENSHOT_FRAMES frames (Play sessions need time to load).
    pub shot_after: Option<f32>,
    /// `--log FILE`: write the Output lines there on exit.
    pub log_out: Option<PathBuf>,
    /// `--click X,Y@S` / `--key NAME@S`: input at S seconds (X,Y in the 3D
    /// view, pixels from its top-left).
    pub actions: Vec<(f32, HarnessAction)>,
    pub started: Instant,
    pub screenshot: Option<PathBuf>,
    pub selftest: Option<SelfTest>,
    /// `--ui-shots DIR`: pictures of the chrome (src/ui_shots.rs).
    pub ui_shots: Option<crate::ui_shots::UiShots>,
    pub screenshot_requested: bool,
    /// `--play`: start a Play session right after loading.
    pub start_play: bool,
    /// `--bench`: frame-time measurement.
    pub bench: Option<crate::bench::Bench>,
    /// `--camera x,y,z,yaw,pitch`: starting camera (degrees).
    pub camera: Option<[f32; 5]>,
    /// `--shift S:HOLD`: Shift held (egui reports it as a modifier, not a key).
    pub shift: bool,
    /// `--shots T1,T2,…`: a window and a viewport screenshot at each time
    /// (FILE-<t>s.png / FILE-<t>s.view.png), the Output log with each, then exit.
    pub shots: Vec<f32>,
    pub shot_next: usize,
    /// `[shot] PATH` lines for the shots written so far (stdout and --log).
    pub shot_lines: Vec<String>,
    /// `--from-play`: harness times count from the moment Play starts (as a
    /// Studio playtest's do), not from program start.
    pub from_play: bool,
    pub play_started: Option<Instant>,
    /// Frame times (ms) since Play started, for the fidelity report.
    pub frame_ms: Vec<f32>,
    pub last_frame: Option<Instant>,
    /// `--open-roblox ID|URL`: open that place from Roblox once the session
    /// is checked.
    pub open_roblox: Option<(u64, bool)>,
    pub team_create: Option<(u64, u64)>,
    /// `--roblox-picker TAB`: show File > Open from Roblox on that tab.
    pub roblox_picker: Option<crate::cloud_ui::PickerTab>,
    pub roblox_settings: bool,
    /// `--action NAME@S` menu actions now due (run where the egui context is).
    pub ui_due: Vec<String>,
}

impl Harness {
    /// Seconds on the harness clock (None before Play when --from-play).
    pub fn now(&self) -> Option<f32> {
        if self.from_play {
            self.play_started.map(|t| t.elapsed().as_secs_f32())
        } else {
            Some(self.started.elapsed().as_secs_f32())
        }
    }
}

pub struct Editor {
    pub doc: Document,
    pub cam: Camera,
    pub tool: Tool,
    pub snap_move: bool,
    pub move_inc: f32,
    pub snap_rotate: bool,
    pub rotate_inc: f32,
    pub local_space: bool,
    pub show_grid: bool,
    pub log: Vec<LogLine>,
    pub clipboard: Option<Vec<u8>>,
    pub render_state: Option<egui_wgpu::RenderState>,
    pub renderer: Option<Renderer>,
    pub scene: Scene,
    pub hover: Option<Ref>,
    pub drag: Option<GizmoDrag>,
    pub marquee: Option<egui::Pos2>,
    /// The selection when the box started (Shift / Ctrl add to it).
    pub marquee_base: Vec<Ref>,
    /// The Output panel's toolbar choices (src/output_panel.rs).
    pub output_view: crate::output_panel::OutputState,
    /// Dragging parts with the Select / Move tool (src/part_drag.rs).
    pub part_drag: Option<crate::part_drag::PartDrag>,
    pub explorer: ExplorerState,
    pub props: PropertiesState,
    pub account: AccountUi,
    /// Places stored on Roblox: File > Open from Roblox, Save to Roblox.
    pub cloud: crate::cloud_ui::CloudUi,
    /// Scripts to open (or focus) as editor tabs at the end of the frame.
    pub open_script: Vec<Ref>,
    pub scripts: crate::script_editor::ScriptTabs,
    /// The running Play / Run session, if any.
    pub play: Option<crate::play::PlaySession>,
    /// Fixed frame time for deterministic self-tests.
    pub fixed_dt: Option<f32>,
    /// ParticleEmitters, Beams and Trails (round 25).
    pub effects: crate::effects::Effects,
    /// Loaded MeshPart / SpecialMesh geometry.
    pub meshes: crate::assets::MeshCache,
    pub textures: crate::assets::TextureCache,
    /// Cloud asset downloads + disk cache.
    pub assets: crate::assets::AssetStore,
    /// ScreenGuis / SurfaceGuis / BillboardGuis: textures, fonts, hover, focus.
    pub gui: crate::gui::Gui,
    /// Clothing-template box meshes by body part name.
    pub clothing_meshes: std::collections::HashMap<String, u32>,
    /// The rig picker: Some(R15 / R6) is an explicit choice that wins over
    /// the place's setting and the avatar; None (Auto) lets them decide.
    pub play_rig: Option<crate::rig::RigType>,
    /// Play opens on the client's view, as Studio does (the self-test's
    /// older steps turn this off to watch the server).
    pub play_shows_client: bool,
    /// Last frame: (loaded meshes drawn, textured parts, decals, clothed body parts).
    pub draw_stats: [usize; 4],
    /// What the renderer holds per part.
    pub draw_cache: crate::viewport::DrawCache,
    /// Bumped whenever a downloaded asset arrives (parts waiting on one are redone).
    pub asset_gen: u64,
    pub draws_asset_gen: u64,
    /// matpack::generation() and wrap::generation() last seen.
    pub material_gen: u64,
    pub wrap_gen: u64,
    /// Uploaded layered-clothing pieces: (fitting, piece) → (mesh, centre, extent).
    pub wrap_meshes: HashMap<(usize, usize), (u32, glam::Vec3, glam::Vec3)>,
    /// Fitting failures already reported.
    pub wrap_warned: std::collections::HashSet<String>,
    /// Triangles in the drawn voxel terrain.
    pub terrain_tris: usize,
    /// The terrain as last drawn (document id, Terrain, voxels), so an edit
    /// re-meshes only the chunks it changed (round 21).
    pub terrain_drawn: Option<(u64, rbx_dom_weak::types::Ref, crate::terrain::Voxels)>,
    /// Terrain shader parameters to rebuild (colours, packs arriving), and
    /// the material-pack generation they were built at.
    pub terrain_params_dirty: bool,
    pub terrain_params_gen: u64,
    pub terrain_present: [bool; 23],
    /// Chunks with Grass tops (decoration candidates) and those whose blades
    /// are built.
    pub terrain_grass_chunks: std::collections::HashSet<[i32; 3]>,
    pub terrain_grass_built: std::collections::HashSet<[i32; 3]>,
    pub terrain_grass_length: f32,
    /// The water look last drawn (WaterColor, WaterTransparency, WaterReflectance).
    pub terrain_water_look: Option<([u8; 3], f32, f32)>,
    /// Terrain water's waves and clearness for the water pass (round 24).
    pub water_waves: crate::water::WaterLook,
    /// The Terrain Editor's tools and settings.
    pub terrain_ed: crate::terrain_editor::TerrainEditor,
    /// A brush stroke's voxels, the chunks it touched and whether water may
    /// have changed: drawn next frame without decoding the grid.
    pub terrain_brushed: Option<(crate::terrain::Voxels, Vec<[i32; 3]>, bool)>,
    /// Physical key positions held (movement keys; src/keys.rs).
    pub phys: crate::keys::PhysicalKeys,
    /// Seconds the fly keys have been held / released (speed ramp).
    pub fly_held: f32,
    pub fly_idle: f32,
    /// Last wheel zoom: (cursor ray direction, hit distance) for the self-test.
    pub last_zoom: Option<(Vec3, Option<f32>)>,
    pub hud: HudStats,
    pub ui_rects: HashMap<&'static str, egui::Rect>,
    /// Screen positions of gizmo handles last frame (axis, sign) -> point.
    pub gizmo_points: Vec<(crate::viewport::Handle, egui::Pos2)>,
    pub viewport_rect: egui::Rect,
    pub frame_no: u64,
    pub harness: Harness,
    pub last_frame: Instant,
    pub fps: f32,
    pub confirm_discard: Option<PendingAction>,
    /// Selection as of the last Explorer sync.
    pub last_selection: Vec<Ref>,
    /// Active right-drag camera look with the cursor grabbed.
    pub look_grab: Option<crate::viewport::LookGrab>,
    /// Locked grab failed once (e.g. X11): use Confined from then on.
    pub grab_use_confined: bool,
    /// The game holds the cursor (UserInputService.MouseBehavior while playing).
    pub game_grab: Option<crate::viewport::LookGrab>,
    pub game_cursor_hidden: bool,
    /// Cursor commands sent, newest last (the self-test checks them).
    pub cursor_log: Vec<String>,
    /// The 3D view rendered at this many pixels instead of its panel's
    /// (camera aspect from it; drawn scaled into the panel): self-test
    /// steps that compare with a Studio capture at Studio's view size.
    pub view_render_size: Option<[u32; 2]>,
    /// CPU time of the last frame's update + draw submission (ms).
    pub last_work_ms: f32,
    /// Local lights, sky faces and the sky-visibility map.
    pub lighting: crate::viewport::LightingState,
    /// The open project folder (files are the truth; src/project_sync.rs).
    pub project: Option<crate::project_sync::ProjectState>,
    /// A folder with several project files: (folder, the files) to choose from.
    pub project_choice: Option<(PathBuf, Vec<PathBuf>)>,
    /// File > Save as Project Folder… / Project Settings… (src/project_ui.rs).
    pub project_export: Option<crate::project_ui::ExportDialog>,
    pub project_settings: Option<crate::project_ui::SettingsDialog>,
    /// Studio's chrome: ribbon tab, run type, panel toggles, popups (src/studio_ui.rs).
    pub ui: crate::studio_ui::UiState,
    /// Test Here: the character's first spawn point (the camera's position).
    pub play_here: Option<Vec3>,
    /// View > Screenshot waiting for the frame.
    pub pending_user_shot: Option<PathBuf>,
    /// The Start Page (src/start_page.rs): shown when nothing is open.
    pub start: crate::start_page::StartPage,
    /// Requests switching to the main viewport tab.
    pub switch_to_viewport: bool,
}

#[derive(Debug, Clone)]
pub enum PendingAction {
    New,
    Open(Option<PathBuf>),
    OpenFolder,
    /// A place from Roblox: place id, a version (else the latest), and
    /// whether to take a local copy of a Team Create place.
    OpenCloud(u64, Option<u64>, bool),
    /// File > Close Place: back to the Start Page.
    Close,
    Quit,
    /// File > Restart rustudio: a fresh process on the same place.
    Restart,
}

pub struct RustudioApp {
    watchdog: crate::frame_watchdog::Watchdog,
    pub dock: DockState<Tab>,
    pub ed: Editor,
    /// Keeps the dock, window and view settings (src/layout_store.rs).
    layout: Option<crate::layout_store::Saver>,
}

pub fn default_dock() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Viewport]);
    let surface = dock.main_surface_mut();
    let [center, right] = surface.split_right(NodeIndex::root(), 0.76, vec![Tab::Explorer]);
    let [_, _props] = surface.split_below(right, 0.46, vec![Tab::Properties]);
    let [_, _out] = surface.split_below(center, 0.76, vec![Tab::Output]);
    dock
}

impl RustudioApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        open: Option<PathBuf>,
        harness: Harness,
        demo: bool,
    ) -> Self {
        crate::ti::install(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        let render_state = cc.wgpu_render_state.clone();
        let renderer = render_state
            .as_ref()
            .map(|rs| Renderer::new(&rs.device, &rs.queue));
        let mut ed = Editor {
            doc: Document::new_place(),
            cam: Camera::default(),
            tool: Tool::Select,
            snap_move: mech::SNAP_ON_BY_DEFAULT,
            move_inc: mech::MOVE_SNAP,
            snap_rotate: mech::SNAP_ON_BY_DEFAULT,
            rotate_inc: mech::ROTATE_SNAP_DEG,
            local_space: false,
            show_grid: true,
            log: Vec::new(),
            clipboard: None,
            render_state,
            renderer,
            scene: Scene::default(),
            hover: None,
            drag: None,
            marquee: None,
            marquee_base: Vec::new(),
            output_view: Default::default(),
            part_drag: None,
            explorer: ExplorerState::default(),
            props: PropertiesState::default(),
            account: AccountUi::new(),
            cloud: crate::cloud_ui::CloudUi::new(),
            open_script: Vec::new(),
            scripts: Default::default(),
            play: None,
            fixed_dt: None,
            effects: Default::default(),
            meshes: Default::default(),
            textures: Default::default(),
            assets: Default::default(),
            gui: Default::default(),
            clothing_meshes: Default::default(),
            play_rig: None,
            play_shows_client: true,
            draw_stats: [0; 4],
            draw_cache: Default::default(),
            asset_gen: 0,
            draws_asset_gen: 0,
            material_gen: 0,
            wrap_gen: 0,
            wrap_meshes: HashMap::new(),
            wrap_warned: Default::default(),
            terrain_tris: 0,
            terrain_drawn: None,
            terrain_params_dirty: true,
            terrain_params_gen: u64::MAX,
            terrain_present: [false; 23],
            terrain_grass_chunks: Default::default(),
            terrain_grass_built: Default::default(),
            terrain_grass_length: -1.0,
            terrain_water_look: None,
            water_waves: Default::default(),
            terrain_ed: Default::default(),
            terrain_brushed: None,
            phys: Default::default(),
            fly_held: 0.0,
            fly_idle: 0.0,
            last_zoom: None,
            hud: HudStats::default(),
            ui_rects: HashMap::new(),
            gizmo_points: Vec::new(),
            viewport_rect: egui::Rect::NOTHING,
            frame_no: 0,
            harness,
            last_frame: Instant::now(),
            fps: 0.0,
            confirm_discard: None,
            last_selection: Vec::new(),
            look_grab: None,
            grab_use_confined: false,
            game_grab: None,
            game_cursor_hidden: false,
            cursor_log: Vec::new(),
            view_render_size: None,
            last_work_ms: 0.0,
            lighting: Default::default(),
            project: None,
            project_choice: None,
            project_export: None,
            project_settings: None,
            ui: crate::studio_ui::UiState::default(),
            play_here: None,
            pending_user_shot: None,
            start: Default::default(),
            switch_to_viewport: false,
        };
        ed.info(format!(
            "rustudio {} ready. Reflection database has {} classes.",
            env!("CARGO_PKG_VERSION"),
            doc::db().classes.len()
        ));
        for line in crate::crash::unseen_reports() {
            ed.error(line);
        }
        if ed.renderer.is_none() {
            ed.error("No wgpu renderer available; the 3D viewport is disabled.");
        }
        let open_none = open.is_none();
        if let Some(path) = open {
            ed.open_path(path);
        } else if demo {
            ed.build_demo();
        }
        ed.explorer.expand_defaults(&ed.doc);
        if ed.harness.selftest.is_some() {
            // Deterministic play-session timing for the scripted test.
            ed.fixed_dt = Some(1.0 / 60.0);
        }
        if ((ed.harness.selftest.is_none() && ed.harness.bench.is_none())
            || std::env::var_os("RUSTUDIO_SESSION").is_some())
            && std::env::var_os("RUSTUDIO_NO_SESSION").is_none()
        {
            // The session (login keyring) is checked in the background, for
            // real playthroughs too (--screenshot): asset downloads and the
            // avatar. LoadAsset at Play start waits for the check. The
            // self-test and --bench opt in with RUSTUDIO_SESSION=1;
            // RUSTUDIO_NO_SESSION=1 keeps any run signed out.
            ed.account.restore();
        }
        if let Some((u, p)) = ed.harness.team_create { ed.open_team_create(u, p); }
        ed.cloud.open_when_ready = ed.harness.open_roblox.map(|(p, copy)| (p, None, copy));
        ed.cloud.picker_when_ready = ed.harness.roblox_picker;
        ed.cloud.settings_open = ed.harness.roblox_settings;
        if let Some([x, y, z, yaw, pitch]) = ed.harness.camera {
            ed.cam.pos = Vec3::new(x, y, z);
            ed.cam.yaw = yaw.to_radians();
            ed.cam.pitch = pitch.to_radians();
            ed.cam.focus = ed.cam.pos + ed.cam.forward() * 50.0;
        } else if ed.harness.bench.as_ref().is_some_and(|b| !b.play) {
            // Worst case for drawing: the whole place in view.
            ed.frame_all();
        }
        // Nothing to open: the Start Page, as Studio starts. Test and harness
        // runs (and RUSTUDIO_NO_START_PAGE=1) go straight to the editor.
        let plain = ed.harness.plain();
        // RUSTUDIO_START_PAGE=1 forces it (screenshots of the page itself).
        let forced = std::env::var_os("RUSTUDIO_START_PAGE").is_some();
        if forced || (open_none && !demo && plain && std::env::var_os("RUSTUDIO_NO_START_PAGE").is_none()) {
            ed.show_start_page();
            let page = std::env::var("RUSTUDIO_START_PAGE").unwrap_or_default();
            ed.start.open_page(&page);
        }
        // The last session's layout and view settings (plain runs only).
        let mut dock = default_dock();
        let mut layout = None;
        if crate::layout_store::enabled(&ed.harness) {
            let saved = crate::layout_store::load();
            if let Some(l) = &saved {
                l.apply(&mut ed);
                if let Some(d) = l.dock() {
                    dock = d;
                }
            }
            layout = Some(crate::layout_store::Saver::new(saved.as_ref()));
        }
        // `--play` starts once the 3D view has its size (Camera.ViewportSize).
        RustudioApp {
            watchdog: crate::frame_watchdog::Watchdog::new(cc.egui_ctx.clone()),
            dock,
            ed,
            layout,
        }
    }
}

// ------------------------------------------------------------------ logging

fn timestamp() -> String {
    // Studio's Output timestamp: HH:MM:SS.mmm in local time.
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let secs = (ms / 1000) as i64;
    let offset = chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| t.with_timezone(&chrono::Local).offset().local_minus_utc())
        .unwrap_or(0);
    let s = ((ms / 1000) as i64 + offset as i64).rem_euclid(86_400) as u64;
    format!("{:02}:{:02}:{:02}.{:03}", s / 3600, (s / 60) % 60, s % 60, ms % 1000)
}

impl Editor {
    pub fn push_log(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        if level != Level::Info {
            eprintln!("[{level:?}] {text}");
        }
        self.log.push(LogLine {
            at: timestamp(),
            level,
            text,
        });
        if self.log.len() > 5000 {
            self.log.drain(0..1000);
            self.output_view.log_drained(1000);
        }
    }
    pub fn info(&mut self, t: impl Into<String>) {
        self.push_log(Level::Info, t)
    }
    pub fn warn(&mut self, t: impl Into<String>) {
        self.push_log(Level::Warn, t)
    }
    pub fn error(&mut self, t: impl Into<String>) {
        self.push_log(Level::Error, t)
    }
}

// ------------------------------------------------------------------ actions

impl Editor {
    pub fn set_document(&mut self, doc: Document) {
        // Another document closes the open project (open_project sets it again).
        self.project = None;
        crate::project_config::set_active(None);
        self.doc = doc;
        self.scene = Scene::default();
        self.drag = None;
        self.hover = None;
        self.open_script.clear();
        self.scripts = Default::default();
        self.explorer = ExplorerState::default();
        self.explorer.expand_defaults(&self.doc);
        if !self.use_saved_camera() {
            self.frame_all();
        }
    }

    /// Starts at the place's saved camera (Workspace.CurrentCamera), as
    /// Studio does. False when the place has none.
    fn use_saved_camera(&mut self) -> bool {
        let Some(ws) = self.doc.workspace() else {
            return false;
        };
        let cam = match self.doc.prop(ws, "CurrentCamera") {
            Some(Variant::Ref(r)) if self.doc.exists(r) => Some(r),
            _ => self.doc.find_child(ws, "Camera"),
        };
        let Some(xf) = cam.and_then(|c| crate::scene::xform_of(&self.doc, c)) else {
            return false;
        };
        if !xf.pos.is_finite() || xf.pos.length() < 1e-3 {
            return false;
        }
        let look = -xf.rot.z_axis;
        let focus = cam.and_then(|c| match self.doc.prop(c, "Focus") {
            Some(Variant::CFrame(f)) => Some(crate::math::v3(f.position)),
            _ => None,
        });
        self.cam = Camera {
            mode: self.cam.mode,
            ..Camera::default()
        };
        self.cam.pos = xf.pos;
        self.cam.pitch = look.y.clamp(-1.0, 1.0).asin();
        self.cam.yaw = (-look.x).atan2(-look.z);
        self.cam.focus = focus
            .filter(|f| (*f - xf.pos).length() > 0.5)
            .unwrap_or(xf.pos + look * 20.0);
        true
    }

    pub fn open_path(&mut self, path: PathBuf) {
        // File > Recent remembers what a person opens (not test runs).
        let harness = self.harness.screenshot.is_some()
            || self.harness.selftest.is_some()
            || self.harness.bench.is_some()
            || self.harness.ui_shots.is_some();
        if !harness && path.exists() {
            crate::studio_ui::remember_recent(&mut self.ui, &path);
        }
        // A copy downloaded from Roblox (places/<id>/origin.json beside it):
        // assets are asked for in that place's context, as when opened from
        // Roblox. RUSTUDIO_PLACE_CONTEXT (harness) wins.
        if std::env::var_os("RUSTUDIO_PLACE_CONTEXT").is_none() {
            let origin = path.parent().map(|d| d.join("origin.json"));
            let place = origin
                .and_then(|o| std::fs::read_to_string(o).ok())
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| v.get("place_id")?.as_u64())
                .unwrap_or(0);
            rustudio_account::session::set_place_context(place);
        }
        // A project folder, or a project file itself.
        if path.is_dir() {
            return self.open_folder(path, false);
        }
        if crate::project::name_of(&path).ends_with(".project.json") {
            return self.open_project(&path);
        }
        match Document::open(&path) {
            Ok(d) => {
                let n = d.dom.descendants().count();
                self.set_document(d);
                self.info(format!("Opened {} ({n} instances)", path.display()));
            }
            Err(e) => self.error(format!("Could not open {}: {e}", path.display())),
        }
    }

    /// A project folder: its project file (asks which one when there are
    /// several and `ask`, else default.project.json or the first).
    pub fn open_folder(&mut self, dir: PathBuf, ask: bool) {
        let files = crate::project::project_files(&dir);
        match files.len() {
            0 => self.error(format!("{} has no *.project.json", dir.display())),
            1 => self.open_project(&files[0].clone()),
            _ if ask => self.project_choice = Some((dir, files)),
            _ => self.open_project(&files[0].clone()),
        }
    }

    pub fn open_project(&mut self, file: &std::path::Path) {
        match crate::project_sync::open(file) {
            Ok((doc, mut st)) => {
                let n = doc.dom.descendants().count();
                let warnings = std::mem::take(&mut st.warnings);
                let place = st
                    .place
                    .as_ref()
                    .map(|p| format!(", place {}", p.display()))
                    .unwrap_or_else(|| ", no place file (an empty DataModel under it)".into());
                let synced = st.map.len();
                self.set_document(doc);
                crate::project_config::set_active(Some(st.config.clone()));
                self.project = Some(st);
                self.info(format!(
                    "Opened project \"{}\" from {} ({n} instances, {synced} from files{place})",
                    self.project.as_ref().map(|p| p.name.as_str()).unwrap_or(""),
                    file.display()
                ));
                for w in warnings.iter().take(20) {
                    self.warn(format!("Project: {w}"));
                }
                if warnings.len() > 20 {
                    self.warn(format!("Project: {} more warnings", warnings.len() - 20));
                }
            }
            Err(e) => self.error(format!("Could not open project {}: {e}", file.display())),
        }
    }

    /// Once a frame: edits to synced instances go to their files, and files
    /// changed on disk come into the place (in edit mode; during Play they
    /// wait until Play stops).
    pub fn project_tick(&mut self) {
        let Some(mut st) = self.project.take() else { return };
        let playing = self.play.is_some();
        if !playing && self.doc.id == st.doc_id {
            st.write_back(&mut self.doc);
        }
        if let Some(changed) = st.poll_disk() {
            if playing {
                st.reapply_after_play = true;
            } else if self.doc.id == st.doc_id {
                let n = st.reconcile(&mut self.doc);
                if n > 0 {
                    st.sourcemap_dirty = true;
                    let mut names: Vec<String> = changed
                        .iter()
                        .filter_map(|p| p.strip_prefix(&st.root).ok().map(|x| x.display().to_string()))
                        .collect();
                    names.sort();
                    names.truncate(4);
                    self.info(format!("Project: {n} instance(s) updated from disk ({})", names.join(", ")));
                }
            }
        }
        if !playing && st.reapply_after_play && self.doc.id == st.doc_id {
            st.reapply_after_play = false;
            let n = st.reconcile(&mut self.doc);
            self.info(format!("Project: {n} instance(s) updated from files changed during Play"));
        }
        if !st.log.is_empty() {
            st.sourcemap_dirty = true;
        }
        if st.sourcemap_dirty && !playing && self.doc.id == st.doc_id {
            st.update_sourcemap(&self.doc);
        }
        for l in std::mem::take(&mut st.log) {
            self.info(format!("Project: {l}"));
        }
        for w in std::mem::take(&mut st.warnings) {
            self.warn(format!("Project: {w}"));
        }
        self.project = Some(st);
    }

    /// Ctrl+S with a project: synced instances are already in their files;
    /// the place file is written only when something outside the mapped
    /// paths changed (so an untouched project saves nothing).
    pub(crate) fn save_project(&mut self) {
        let Some(mut st) = self.project.take() else { return };
        if self.play.is_none() {
            st.write_back(&mut self.doc);
        }
        let dirty = st.place_dirty;
        let place = st.place.clone();
        self.project = Some(st);
        match (dirty, place) {
            (false, _) => self.info("Saved: every synced file is up to date; nothing outside them changed"),
            (true, Some(p)) => {
                self.save_to(p);
                if let Some(st) = self.project.as_mut() {
                    st.place_dirty = false;
                }
            }
            (true, None) => self.save_as(),
        }
    }

    pub fn open_folder_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.open_folder(dir, true);
        }
    }

    pub fn new_place(&mut self) {
        self.set_document(Document::new_place());
        self.cam = Camera::default();
        self.info("New place created");
    }

    pub fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Roblox place or model", &["rbxl", "rbxlx", "rbxm", "rbxmx"])
            .pick_file()
        {
            self.open_path(path);
        }
    }

    pub fn save(&mut self) {
        if self.project.is_some() {
            return self.save_project();
        }
        // A local copy of a Team Create place saves to a file.
        if self.doc.cloud.as_ref().is_some_and(|o| o.local_copy) {
            return match self.doc.path.clone() {
                Some(p) => self.save_to(p),
                None => self.save_as(),
            };
        }
        // A place opened from Roblox: Ctrl+S is Save to Roblox, as in Studio.
        if self.doc.cloud.is_some() {
            return self.save_cloud(rustudio_account::VersionType::Saved, false);
        }
        match self.doc.path.clone() {
            Some(p) => self.save_to(p),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) {
        let (name, exts): (&str, &[&str]) = match self.doc.kind {
            DocKind::Place => ("Roblox place", &["rbxl", "rbxlx"]),
            DocKind::Model => ("Roblox model", &["rbxm", "rbxmx"]),
        };
        let default = match self.doc.kind {
            DocKind::Place => "place.rbxl",
            DocKind::Model => "model.rbxm",
        };
        if let Some(p) = rfd::FileDialog::new()
            .add_filter(name, exts)
            .set_file_name(default)
            .save_file()
        {
            self.save_to(p);
        }
    }

    pub fn save_to(&mut self, p: PathBuf) {
        match self.doc.save_to(&p) {
            Ok(()) => self.info(format!("Saved {}", p.display())),
            Err(e) => self.error(format!("Save failed: {e}")),
        }
    }

    pub fn insert_from_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Roblox model", &["rbxm", "rbxmx"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                let parent = self.paste_target(false);
                let refs = self.doc.paste(&bytes, parent, "Insert from file");
                self.info(format!(
                    "Inserted {} object(s) from {}",
                    refs.len(),
                    path.display()
                ));
                self.select(refs);
            }
            Err(e) => self.error(format!("{}: {e}", path.display())),
        }
    }

    pub fn save_selection(&mut self) {
        if self.doc.selection.is_empty() {
            self.warn("Select something to save as a model");
            return;
        }
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Roblox model", &["rbxm", "rbxmx"])
            .set_file_name("model.rbxm")
            .save_file()
        {
            match self.doc.save_selection_to(&self.doc.selection.clone(), &p) {
                Ok(()) => self.info(format!("Saved selection to {}", p.display())),
                Err(e) => self.error(format!("Save failed: {e}")),
            }
        }
    }

    pub fn select(&mut self, refs: Vec<Ref>) {
        self.doc.selection = refs;
    }

    /// Whenever the selection changes (viewport click, undo/redo, paste,
    /// Explorer…), expand the ancestors of every selected object and scroll
    /// the newest one into view.
    fn sync_explorer_to_selection(&mut self) {
        if self.doc.selection == self.last_selection {
            return;
        }
        let added: Vec<Ref> = self
            .doc
            .selection
            .iter()
            .copied()
            .filter(|r| !self.last_selection.contains(r))
            .collect();
        for &r in &self.doc.selection {
            self.explorer.expand_ancestors(&self.doc, r);
        }
        if let Some(&r) = added.last().or(self.doc.selection.last()) {
            self.explorer.scroll_to = Some(r);
        }
        self.last_selection = self.doc.selection.clone();
    }

    pub fn undo(&mut self) {
        match self.doc.undo() {
            Some(l) => self.info(format!("Undo: {l}")),
            None => self.warn("Nothing to undo"),
        }
    }

    pub fn redo(&mut self) {
        match self.doc.redo() {
            Some(l) => self.info(format!("Redo: {l}")),
            None => self.warn("Nothing to redo"),
        }
    }

    pub fn copy(&mut self) -> bool {
        match self.doc.copy(&self.doc.selection.clone()) {
            Some(bytes) => {
                self.clipboard = Some(bytes);
                true
            }
            None => false,
        }
    }

    pub fn cut(&mut self) {
        if self.copy() {
            let sel = self.doc.selection.clone();
            self.doc.remove(&sel);
        }
    }

    /// Paste goes next to the selection (or into it with `into`), else Workspace.
    fn paste_target(&self, into: bool) -> Ref {
        match self.doc.selection.first() {
            Some(&r) if into => r,
            Some(&r) => self
                .doc
                .dom
                .get_by_ref(r)
                .map(|i| i.parent())
                .filter(|p| p.is_some())
                .unwrap_or(self.doc.default_parent()),
            None => self.doc.default_parent(),
        }
    }

    pub fn paste(&mut self, into: bool) {
        let Some(bytes) = self.clipboard.clone() else {
            self.warn("Clipboard is empty");
            return;
        };
        let parent = self.paste_target(into);
        let refs = self.doc.paste(&bytes, parent, "Paste");
        self.select(refs);
    }

    pub fn duplicate(&mut self) {
        let sel = self.doc.selection.clone();
        let refs = self.doc.duplicate(&sel);
        if !refs.is_empty() {
            self.select(refs);
        }
    }

    pub fn delete(&mut self) {
        let sel = self.doc.selection.clone();
        let n = self.doc.remove(&sel);
        if n == 0 && !sel.is_empty() {
            self.warn("Services and the root cannot be deleted");
        }
    }

    pub fn group(&mut self) {
        let sel = self.doc.selection.clone();
        if let Some(m) = self.doc.group(&sel) {
            self.select(vec![m]);
        }
    }

    pub fn ungroup(&mut self) {
        let sel = self.doc.selection.clone();
        let refs = self.doc.ungroup(&sel);
        if !refs.is_empty() {
            self.select(refs);
        }
    }

    /// Inserts a new part in front of the camera, on top of whatever is there.
    pub fn insert_part(&mut self, mesh: MeshKind) {
        let (class, shape) = match mesh {
            MeshKind::Block => ("Part", 1),
            MeshKind::Ball => ("Part", 0),
            MeshKind::Cylinder => ("Part", 2),
            MeshKind::Wedge => ("WedgePart", 1),
            MeshKind::CornerWedge => ("CornerWedgePart", 1),
        };
        let mut size = Vec3::from(mech::NEW_PART_SIZE);
        if mesh == MeshKind::Ball {
            size = Vec3::splat(2.0);
        } else if mesh == MeshKind::Cylinder {
            size = Vec3::new(2.0, 2.0, 2.0);
        }
        let pos = self.spawn_point(size);
        let mut b = doc::new_part(
            if class == "Part" { "Part" } else { class },
            shape,
            pos,
            size,
            mech::NEW_PART_COLOR,
        );
        if class != "Part" {
            b = b.with_class(class);
        }
        let parent = self.doc.default_parent();
        let r = self.doc.insert(parent, b);
        self.select(vec![r]);
    }

    /// Where a new part lands: the point under the screen centre, snapped.
    fn spawn_point(&mut self, size: Vec3) -> Vec3 {
        self.scene.sync(&mut self.doc);
        let rect = if self.viewport_rect.is_positive() {
            self.viewport_rect
        } else {
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))
        };
        let ray = self.cam.ray(rect, rect.center());
        let mut best: Option<f32> = None;
        for p in &self.scene.parts {
            if let Some(t) = ray.hit_obb(&p.xf, p.size) {
                if best.is_none_or(|b| t < b) {
                    best = Some(t);
                }
            }
        }
        let hit = match best {
            Some(t) if t < 500.0 => ray.at(t),
            _ => ray
                .hit_plane(Vec3::ZERO, Vec3::Y)
                .filter(|t| *t < 500.0)
                .map(|t| ray.at(t))
                .unwrap_or(self.cam.pos + self.cam.forward() * 20.0),
        };
        let snap = |v: f32| crate::math::snap(v, if self.snap_move { self.move_inc } else { 0.0 });
        Vec3::new(snap(hit.x), hit.y + size.y / 2.0, snap(hit.z))
    }

    pub fn insert_class(&mut self, class: &str, parent: Ref) {
        if class == "Part" {
            self.insert_part(MeshKind::Block);
            if parent != self.doc.default_parent() {
                let sel = self.doc.selection.clone();
                self.doc.reparent(&sel, parent);
            }
            return;
        }
        let mut b = InstanceBuilder::new(class).with_name(class);
        if doc::is_a(class, "BasePart") {
            let size = Vec3::from(mech::NEW_PART_SIZE);
            let pos = self.spawn_point(size);
            b = b
                .with_property("Anchored", true)
                .with_property(
                    "Size",
                    rbx_dom_weak::types::Vector3::new(size.x, size.y, size.z),
                )
                .with_property(
                    "CFrame",
                    rbx_dom_weak::types::CFrame::new(
                        crate::math::rv3(pos),
                        rbx_dom_weak::types::Matrix3::identity(),
                    ),
                );
        }
        if matches!(class, "Script" | "LocalScript" | "ModuleScript") {
            let src = if class == "ModuleScript" {
                "local module = {}\n\nreturn module\n"
            } else {
                "print(\"Hello world!\")\n"
            };
            b = b.with_property("Source", src.to_string());
        }
        let r = self.doc.insert(parent, b);
        self.select(vec![r]);
    }

    pub fn selected_parts(&self) -> Vec<Ref> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for &r in &self.doc.selection {
            if self.scene.get(r).is_some() && seen.insert(r) {
                out.push(r);
            }
            for d in self.doc.dom.descendants_of(r) {
                let dr = d.referent();
                if dr != r && self.scene.get(dr).is_some() && seen.insert(dr) {
                    out.push(dr);
                }
            }
        }
        out
    }

    /// Bounding box (min, max) of the selected parts.
    pub fn selection_bounds(&self) -> Option<(Vec3, Vec3)> {
        let parts = self.selected_parts();
        let mut it = parts.iter().filter_map(|r| self.scene.get(*r));
        let first = it.next()?;
        let mut b = crate::viewport::part_aabb(first);
        for p in it {
            let (mn, mx) = crate::viewport::part_aabb(p);
            b = (b.0.min(mn), b.1.max(mx));
        }
        Some(b)
    }

    pub fn focus_selection(&mut self) {
        self.scene.sync(&mut self.doc);
        match self.selection_bounds() {
            Some((mn, mx)) => self.cam.frame((mn + mx) * 0.5, (mx - mn).length() * 0.5),
            None => self.warn("Select a part to focus on"),
        }
    }

    /// Frames everything except huge baseplates.
    /// Frames where most of the parts are: the 10th–90th percentile box of
    /// part centres, so a few stray far-away parts don't throw it off.
    pub fn frame_all(&mut self) {
        self.scene.sync(&mut self.doc);
        let centers: Vec<Vec3> = self
            .scene
            .parts
            .iter()
            .filter(|p| p.size.max_element() <= 200.0 && p.xf.pos.is_finite())
            .map(|p| p.xf.pos)
            .collect();
        self.cam = Camera {
            mode: self.cam.mode,
            ..Camera::default()
        };
        if centers.is_empty() {
            return;
        }
        let quantile = |axis: usize, q: f32| {
            let mut v: Vec<f32> = centers.iter().map(|c| c[axis]).collect();
            v.sort_by(f32::total_cmp);
            v[((v.len() - 1) as f32 * q).round() as usize]
        };
        let lo = Vec3::new(quantile(0, 0.1), quantile(1, 0.1), quantile(2, 0.1));
        let hi = Vec3::new(quantile(0, 0.9), quantile(1, 0.9), quantile(2, 0.9));
        self.cam.frame(
            (lo + hi) * 0.5,
            ((hi - lo).length() * 0.5).clamp(8.0, 2000.0),
        );
    }

    /// The sample scene used for screenshots and `--demo`.
    pub fn build_demo(&mut self) {
        let last = build_demo_parts(&mut self.doc);
        self.doc.selection = vec![last];
        self.tool = Tool::Move;
        self.cam = Camera::default();
        self.cam.pos = Vec3::new(14.0, 22.0, 34.0);
        self.cam.yaw = 0.32;
        self.cam.pitch = -0.5;
        self.cam.focus = self.cam.pos + self.cam.forward() * 40.0;
        // Demo edits should not show up as undoable history.
        self.doc.clear_history();
        self.info("Loaded the sample scene");
    }
}

// ----------------------------------------------------------------- shortcuts

impl Editor {
    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Test controls work even while typing in a script.
        let (stop, play, run) = ctx.input_mut(|i| {
            let stop = i.consume_shortcut(&KeyboardShortcut::new(Modifiers::SHIFT, Key::F5));
            let play = i.consume_shortcut(&KeyboardShortcut::new(Modifiers::NONE, Key::F5));
            let run = i.consume_shortcut(&KeyboardShortcut::new(Modifiers::NONE, Key::F8));
            (stop, play, run)
        });
        if stop {
            self.stop_play();
        } else if play {
            self.start_run(crate::studio_ui::RunType::Test);
        } else if run {
            self.start_run(crate::studio_ui::RunType::Run);
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // While playing, WASD/Space/etc. belong to the game.
        if self.play_game_input() {
            let esc = ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key { key: Key::Escape, pressed: true, repeat: false, .. }
                    )
                })
            });
            if esc {
                self.toggle_game_menu();
            }
            // Presses this frame: a tap pressed and released within one frame
            // never shows in keys_down, yet scripts must see it. The key
            // positions are physical, like Roblox's KeyCodes (src/keys.rs).
            // (Escape belongs to the Roblox menu: games never hear it.)
            let taps: Vec<Key> = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Key {
                            key,
                            physical_key,
                            pressed: true,
                            repeat: false,
                            ..
                        } if physical_key.unwrap_or(*key) != Key::Escape => {
                            Some(physical_key.unwrap_or(*key))
                        }
                        _ => None,
                    })
                    .collect()
            });
            // The open menu takes the keys (R, L); the game hears none.
            if !self.game_menu_keys(&taps) {
                // (A focused TextBox has them: game-processed, Enter included.)
                let processed = self.gui.focus.is_some();
                if let Some(s) = self.play.as_mut() {
                    s.game.key_taps.extend(taps.into_iter().map(|k| (k, processed)));
                }
            }
            ctx.input_mut(|i| {
                i.events.retain(
                    |e| !matches!(e, egui::Event::Key { modifiers, .. } if modifiers.is_none()),
                );
            });
        }
        // Studio's menu accelerators (Ctrl+Shift+V, Ctrl+Alt+G, Alt+A, F11…).
        for a in self.menu_shortcut_actions(ctx) {
            self.ui_action(ctx, a);
        }
        for a in std::mem::take(&mut self.harness.ui_due) {
            self.ui_action(ctx, &a);
        }
        let sc = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        let mut actions: Vec<&str> = Vec::new();
        ctx.input_mut(|i| {
            let mut check = |s: KeyboardShortcut, name: &'static str| {
                if i.consume_shortcut(&s) {
                    actions.push(name);
                }
            };
            check(sc(cs, Key::P), "focus_props_filter");
            check(sc(cs, Key::Z), "redo");
            check(sc(cs, Key::S), "save_as");
            check(sc(cs, Key::V), "paste_into");
            check(sc(Modifiers::COMMAND, Key::Z), "undo");
            check(sc(Modifiers::COMMAND, Key::Y), "redo");
            check(sc(Modifiers::COMMAND, Key::D), "duplicate");
            check(sc(Modifiers::COMMAND, Key::G), "group");
            check(sc(Modifiers::COMMAND, Key::U), "ungroup");
            check(sc(Modifiers::COMMAND, Key::S), "save");
            check(sc(Modifiers::COMMAND, Key::O), "open");
            check(sc(Modifiers::COMMAND, Key::N), "new");
            check(sc(Modifiers::COMMAND, Key::I), "insert_object");
            check(sc(Modifiers::COMMAND, Key::L), "toggle_space");
            check(sc(Modifiers::COMMAND, Key::Num1), "tool_select");
            check(sc(Modifiers::COMMAND, Key::Num2), "tool_move");
            check(sc(Modifiers::COMMAND, Key::Num3), "tool_scale");
            check(sc(Modifiers::COMMAND, Key::Num4), "tool_rotate");
            check(sc(Modifiers::NONE, Key::Delete), "delete");
            check(sc(Modifiers::NONE, Key::F2), "rename");
            check(sc(Modifiers::NONE, Key::F), "focus");
            // egui turns Ctrl+C/X/V into clipboard events.
            for ev in &i.events {
                match ev {
                    egui::Event::Copy => actions.push("copy"),
                    egui::Event::Cut => actions.push("cut"),
                    // egui-winit sends Ctrl+V and Ctrl+Shift+V as Paste (only
                    // when the system clipboard holds text: copy puts a note
                    // there); Shift held means Paste Into.
                    egui::Event::Paste(_) if i.modifiers.shift => actions.push("paste_into"),
                    egui::Event::Paste(_) => actions.push("paste"),
                    _ => {}
                }
            }
        });
        if actions.contains(&"focus_props_filter") {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new(crate::properties::PROPS_FILTER_ID)));
        }
        let copied = actions.iter().any(|a| matches!(*a, "copy" | "cut"));
        for a in actions {
            self.run_action(a);
        }
        if copied {
            self.note_os_clipboard(ctx);
        }
    }

    /// Puts a line on the system clipboard after a copy: egui-winit only
    /// delivers Ctrl+V when the system clipboard holds text.
    pub fn note_os_clipboard(&self, ctx: &egui::Context) {
        if self.terrain_ed.tool.is_some_and(crate::terrain_editor::Tool::is_region) {
            if self.terrain_ed.clipboard.is_some() && self.harness.selftest.is_none() {
                ctx.copy_text("rustudio: terrain region copied".to_string());
            }
            return;
        }
        if self.clipboard.is_some() && self.harness.selftest.is_none() {
            ctx.copy_text(format!("rustudio: {} object(s) copied", self.doc.selection.len()));
        }
    }

    pub fn run_action(&mut self, a: &str) {
        // A Terrain Editor Region tool has Copy, Cut, Paste, Duplicate and Delete.
        if self.terrain_ed.tool.is_some_and(crate::terrain_editor::Tool::is_region)
            && matches!(a, "copy" | "cut" | "paste" | "duplicate" | "delete")
            && self.terrain_region_action(a)
        {
            return;
        }
        match a {
            "undo" => self.undo(),
            "redo" => self.redo(),
            "copy" => {
                self.copy();
            }
            "cut" => self.cut(),
            "paste" => self.paste(false),
            "paste_into" => self.paste(true),
            "duplicate" => self.duplicate(),
            "delete" => self.delete(),
            "group" => self.group(),
            "ungroup" => self.ungroup(),
            "save" => self.save(),
            "save_as" => self.save_as(),
            "open" => self.request(PendingAction::Open(None)),
            "new" => self.request(PendingAction::New),
            "insert_object" => {
                let parent = self.insert_parent();
                let at = self.viewport_rect.center() - egui::vec2(120.0, 200.0);
                self.open_insert_popup(parent, at);
            }
            "toggle_space" => self.local_space = !self.local_space,
            "tool_select" => self.tool = Tool::Select,
            "tool_move" => self.tool = Tool::Move,
            "tool_scale" => self.tool = Tool::Scale,
            "tool_rotate" => self.tool = Tool::Rotate,
            "rename" => {
                if let Some(&r) = self.doc.selection.first() {
                    let name = self
                        .doc
                        .dom
                        .get_by_ref(r)
                        .map(|i| i.name.clone())
                        .unwrap_or_default();
                    self.explorer.renaming = Some((r, name));
                }
            }
            "focus" => self.focus_selection(),
            "focus_props_filter" => self.ui.panel_requests.push((Tab::Properties, Some(true))),
            _ => {}
        }
    }

    /// Asks before throwing away unsaved changes.
    pub fn request(&mut self, action: PendingAction) {
        if self.doc.dirty() && self.harness.selftest.is_none() {
            self.confirm_discard = Some(action);
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: PendingAction) {
        match action {
            PendingAction::New => self.new_place(),
            PendingAction::Open(None) => self.open_dialog(),
            PendingAction::Open(Some(p)) => self.open_path(p),
            PendingAction::OpenFolder => self.open_folder_dialog(),
            PendingAction::OpenCloud(p, v, copy) => self.perform_open_cloud(p, v, copy),
            PendingAction::Close => {
                if self.play.is_some() {
                    self.stop_play();
                }
                self.new_place();
                self.show_start_page();
            }
            PendingAction::Quit => std::process::exit(0),
            PendingAction::Restart => self.restart(),
        }
    }

    /// Starts a new rustudio on what's open now (the live Team Create
    /// session, the file or project folder, or a Roblox place's local copy)
    /// and exits this one.
    fn restart(&mut self) {
        let mut args: Vec<String> = Vec::new();
        if let Some((universe, place)) = self.live_team_create_ids() {
            args = vec!["--team-create".into(), universe.to_string(), place.to_string()];
        } else if let Some(path) = &self.doc.path {
            args.push(path.display().to_string());
        } else if let Some(origin) = &self.doc.cloud {
            args = vec!["--open-roblox".into(), origin.place_id.to_string(), "--local-copy".into()];
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("rustudio"));
        match std::process::Command::new(&exe).args(&args).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => self.error(format!("Restart failed: couldn't start {}: {e}", exe.display())),
        }
    }
}

// --------------------------------------------------------------- menus & bars

impl Editor {
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        self.hud.status_drawn = true;
        ui.horizontal(|ui| {
            let sel = self.doc.selection.len();
            if let Some(s) = self.play_status() {
                ui.label(RichText::new(format!("● {s}")).color(theme::ERROR).strong());
                ui.separator();
            }
            ui.label(RichText::new(format!("{} tool", self.tool.label())).color(theme::ACCENT));
            ui.separator();
            ui.label(format!("{sel} selected"));
            ui.separator();
            ui.label(format!("{} parts", self.scene.parts.len()));
            ui.separator();
            let p = self.cam.pos;
            ui.label(format!("Camera {:.1}, {:.1}, {:.1}", p.x, p.y, p.z));
            ui.separator();
            ui.label(if self.snap_move {
                format!("Snap {} st / {}°", self.move_inc, self.rotate_inc)
            } else {
                "Snap off".into()
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.0} fps", self.fps)).color(theme::TEXT_DIM));
                if let Some(l) = self.log.last() {
                    let c = match l.level {
                        Level::Info => theme::TEXT_DIM,
                        Level::Warn => theme::WARN,
                        Level::Error => theme::ERROR,
                    };
                    ui.label(RichText::new(&l.text).color(c));
                }
            });
        });
    }

    /// Opens requested scripts as tabs next to the viewport (or focuses them).
    fn open_script_tabs(&mut self, dock: &mut DockState<Tab>) {
        for r in std::mem::take(&mut self.open_script) {
            let tree = dock.main_surface_mut();
            if let Some((node, tab)) = tree.find_tab(&Tab::Script(r)) {
                let _ = tree.set_active_tab(node, tab);
                tree.set_focused_node(node);
                continue;
            }
            if let Some((node, _)) = tree.find_tab(&Tab::Viewport) {
                tree.set_focused_node(node);
            }
            tree.push_to_focused_leaf(Tab::Script(r));
        }
    }

    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm_discard.clone() else {
            return;
        };
        let mut decided: Option<bool> = None;
        egui::Modal::new(egui::Id::new("confirm_discard")).show(ctx, |ui| {
            ui.label(RichText::new("Discard unsaved changes?").strong());
            ui.label(format!(
                "{} has changes that are not saved.",
                self.doc.title
            ));
            ui.horizontal(|ui| {
                if ui.button("Save first").clicked() {
                    self.save();
                    decided = Some(!self.doc.dirty());
                }
                if ui.button("Discard").clicked() {
                    decided = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    decided = Some(false);
                }
            });
        });
        if let Some(go) = decided {
            self.confirm_discard = None;
            if go {
                self.perform(action);
            }
        }
    }
}

struct Viewer<'a> {
    ed: &'a mut Editor,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Script(r) => format!("📜 {}", self.ed.script_title(*r)).into(),
            // Studio's document tab: the place's name.
            Tab::Viewport => match self.ed.play_status() {
                Some(s) => RichText::new(format!("{} — {s}", self.ed.doc.title))
                    .color(theme::ACCENT)
                    .into(),
                None => self.ed.doc.title.clone().into(),
            },
            Tab::Properties => crate::properties::panel_title(self.ed).into(),
            other => other.title().into(),
        }
    }

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        match tab {
            Tab::Script(r) => egui::Id::new(("script_tab", *r)),
            other => egui::Id::new(("tab", other.title())),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        let _p = crate::bench::scope(match tab {
            Tab::Viewport => "viewport",
            Tab::Explorer => "explorer",
            Tab::Properties => "properties",
            Tab::Output => "output",
            _ => "other_tab",
        });
        match tab {
            Tab::Viewport => self.ed.viewport_ui(ui),
            Tab::Explorer => self.ed.explorer_ui(ui),
            Tab::Properties => self.ed.properties_ui(ui),
            Tab::Output => self.ed.output_ui(ui),
            Tab::AvatarSettings => self.ed.avatar_settings_ui(ui),
            Tab::TerrainEditor => self.ed.terrain_editor_ui(ui),
            Tab::Script(r) => self.ed.script_tab_ui(ui, *r),
        }
    }

    fn is_closeable(&self, tab: &Tab) -> bool {
        // The place tab's × is File > Close Place (not while playing).
        *tab != Tab::Viewport || self.ed.play_status().is_none()
    }

    fn on_close(&mut self, tab: &mut Tab) -> egui_dock::tab_viewer::OnCloseResponse {
        if *tab == Tab::Viewport {
            // Closing the place goes back to the Start Page (asks first
            // when there are unsaved changes); the tab itself stays.
            self.ed.request(PendingAction::Close);
            return egui_dock::tab_viewer::OnCloseResponse::Ignore;
        }
        egui_dock::tab_viewer::OnCloseResponse::Close
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }

    fn clear_background(&self, tab: &Tab) -> bool {
        *tab != Tab::Viewport
    }
}

impl eframe::App for RustudioApp {
    fn on_exit(&mut self) {
        if let Some(s) = self.layout.as_mut() {
            s.save(&self.dock, &self.ed);
        }
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(t) = self.ed.harness.selftest.as_mut() {
            t.inject(raw);
        }
        if !self.ed.harness.actions.is_empty() {
            self.ed.harness_inject(raw);
        }
        if let Some(u) = self.ed.harness.ui_shots.as_mut() {
            u.inject(raw);
        }
        self.ed.phys.ingest(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.watchdog.begin(ctx.input(|i| !i.viewport().minimized.unwrap_or(false)) && (self.ed.play.is_some() || self.ed.harness.screenshot.is_some()));
        let ed = &mut self.ed;
        ed.frame_no += 1;
        let frame_start = Instant::now();
        let now = Instant::now();
        let dt = now.duration_since(ed.last_frame).as_secs_f32();
        ed.last_frame = now;
        if dt > 0.0 {
            ed.fps = ed.fps * 0.9 + (1.0 / dt) * 0.1;
        }
        ed.hud = HudStats::default();
        for (err, m) in crate::studio_content::take_messages() {
            if err { ed.error(m) } else { ed.info(m) }
        }
        ed.assets.set_client(ed.account.shared_client());
        let done = ed.assets.poll();
        if ed.textures.poll_decoded() {
            ed.asset_gen += 1;
        }
        if !done.is_empty() {
            ed.meshes.retry(&done);
            ed.textures.retry(&done);
            ed.asset_gen += 1;
        }
        // Material texture packs load on their own threads.
        let mat_gen = crate::matpack::generation();
        if mat_gen != ed.material_gen {
            ed.material_gen = mat_gen;
            ed.asset_gen += 1;
        }
        let wrap_gen = crate::wrap::generation();
        if wrap_gen != ed.wrap_gen {
            ed.wrap_gen = wrap_gen;
            ed.asset_gen += 1;
        }
        for msg in std::mem::take(&mut ed.assets.messages) {
            ed.info(msg);
        }
        for (level, text) in ed.account.poll() {
            ed.push_log(level, text);
        }
        self.watchdog.stage("asset polling", "Play tick: scripts/physics");
        if ed.play.is_some() {
            let step = ed.fixed_dt.unwrap_or(dt);
            let _p = crate::bench::scope("play_tick");
            ed.play_tick(&ctx, step);
        }
        self.watchdog.stage("Play tick", "project and scene synchronization");
        ed.project_tick();
        // A folder with several project files: which one to open.
        if let Some((dir, files)) = ed.project_choice.clone() {
            let mut pick = None;
            let mut open = true;
            egui::Window::new("Open project")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(&ctx, |ui| {
                    ui.label(format!("{} has several project files:", dir.display()));
                    for f in &files {
                        let n = crate::project::name_of(f).to_string();
                        let label = if n == "default.project.json" { format!("{n} (default)") } else { n };
                        if ui.button(label).clicked() {
                            pick = Some(f.clone());
                        }
                    }
                });
            if let Some(f) = pick {
                ed.project_choice = None;
                ed.open_project(&f);
            } else if !open {
                ed.project_choice = None;
            }
        }
        ed.project_windows(&ctx);
        ed.sync_explorer_to_selection();
        {
            let _p = crate::bench::scope("scene_sync");
            ed.scene.sync(&mut ed.doc);
        }

        if ctx.input(|i| i.viewport().close_requested())
            && ed.doc.dirty()
            && ed.harness.screenshot.is_none()
            && ed.harness.selftest.is_none()
            && ed.harness.bench.is_none()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ed.confirm_discard = Some(PendingAction::Quit);
        }
        if ctx
            .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("reset_dock")))
            .unwrap_or(false)
        {
            self.dock = default_dock();
        }

        let ed = &mut self.ed;
        if (ctx.zoom_factor() - ed.ui.zoom).abs() > 1e-3 {
            ctx.set_zoom_factor(ed.ui.zoom);
        }
        // The Start Page replaces the editor's chrome until something opens.
        if ed.start_page_ui(ui) {
            ed.confirm_ui(&ctx);
            ed.account_window(&ctx);
            ed.cloud_windows(&ctx);
            if ed.harness.screenshot.is_some() {
                ed.harness_tick(&ctx);
                ctx.request_repaint();
            }
            if ed.account.busy() || ed.assets.busy() {
                ctx.request_repaint();
            }
            self.watchdog.complete();
            return;
        }
        self.watchdog.stage("scene synchronization", "dock UI and GPU viewport preparation");
        ed.shortcuts(&ctx);

        egui::Panel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(theme::MENU_BAR)
                    .inner_margin(egui::Margin::symmetric(6, 2)),
            )
            .show(ui, |ui| ed.studio_menu_bar(ui));
        egui::Panel::top("testbar")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| ed.studio_top_row(ui));
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(6, 4)),
            )
            .show(ui, |ui| ed.studio_ribbon(ui));
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_DARK)
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ui, |ui| ed.status_bar(ui));

        // Editor::start_run requests this when a playtest begins.
        if std::mem::take(&mut self.ed.switch_to_viewport) {
            if let Some(loc) = self.dock.find_tab(&Tab::Viewport) {
                let _ = self.dock.set_active_tab(loc);
            }
        }

        let _dock_scope = crate::bench::scope("dock_panels");
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG))
            .show(ui, |ui| {
                DockArea::new(&mut self.dock)
                    .style(theme::dock_style(ui.style()))
                    .show_add_buttons(false)
                    .show_leaf_collapse_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut Viewer { ed: &mut self.ed });
            });

        drop(_dock_scope);
        // Window > Explorer / Properties / Output / Avatar Settings toggles,
        // and which panels are open (their checkmarks next frame).
        for (tab, want) in std::mem::take(&mut self.ed.ui.panel_requests) {
            let found = self.dock.find_tab(&tab);
            let open = want.unwrap_or(found.is_none());
            match (found, open) {
                (Some(at), false) => {
                    self.dock.remove_tab(at);
                }
                (Some(at), true) => {
                    let _ = self.dock.set_active_tab(at);
                }
                (None, true) => {
                    let tree = self.dock.main_surface_mut();
                    // Avatar Settings docks under the viewport, like Studio's;
                    // the Terrain Editor beside the Explorer.
                    let anchor = match tab {
                        Tab::AvatarSettings => Tab::Output,
                        Tab::TerrainEditor => Tab::Explorer,
                        _ => Tab::Viewport,
                    };
                    if let Some((node, _)) = tree.find_tab(&anchor) {
                        tree.set_focused_node(node);
                    }
                    tree.push_to_focused_leaf(tab);
                }
                (None, false) => {}
            }
        }
        self.ed.ui.panels_open = [
            (Tab::Explorer, "Explorer"),
            (Tab::Properties, "Properties"),
            (Tab::Output, "Output"),
            (Tab::AvatarSettings, "Avatar Settings"),
            (Tab::TerrainEditor, "Terrain Editor"),
        ]
        .into_iter()
        .filter(|(t, _)| self.dock.find_tab(t).is_some())
        .map(|(_, n)| n)
        .collect();
        if let Some(s) = self.layout.as_mut() {
            s.tick(&ctx, &self.dock, &self.ed);
        }
        let ed = &mut self.ed;
        ed.open_script_tabs(&mut self.dock);
        ed.insert_popup_ui(&ctx);
        ed.about_ui(&ctx);
        ed.user_screenshot_tick(&ctx);
        crate::runtime::domains::assets_cloud::capture::frame_tick(&ctx, ed.viewport_rect);
        ed.confirm_ui(&ctx);
        ed.account_window(&ctx);
        ed.cloud_windows(&ctx);
        self.watchdog.stage("dock UI", "harness and screenshot readback");
        ed.harness_tick(&ctx);
        ed.ui_shots_tick(&ctx);
        let work_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        ed.last_work_ms = work_ms;
        if let Some(b) = ed.harness.bench.as_mut() {
            // Edit-mode benchmark: turn the camera slowly so several views count.
            if !b.play {
                ed.cam.yaw += crate::mech::BENCH_TURN_DEG_PER_S.to_radians() * dt;
                ed.cam.focus = ed.cam.pos + ed.cam.forward() * (ed.cam.focus - ed.cam.pos).length();
            }
            // Play benches count from Play's start (a live join can take a
            // minute before Play), not from the window opening.
            let ready = !b.play || ed.harness.play_started.is_some();
            if ready && !b.done && b.frame(work_ms) {
                let mut report = b.report();
                if let Some(r) = ed.renderer.as_ref() {
                    report["render"] = serde_json::json!(format!("{:?}", r.stats));
                }
                if let Some(p) = ed.play.as_ref() {
                    let (dynamic, fixed) = p.physics.counts();
                    report["physics"] = serde_json::json!({"assemblies": dynamic, "awake": p.physics.awake(), "fixed_colliders": fixed, "steps": p.physics.steps, "stages": p.physics.stage_times()});
                    let sample: Vec<String> = p
                        .physics
                        .awake_sample(25)
                        .into_iter()
                        .map(|(r, pos, vel, n)| {
                            let mut path = Vec::new();
                            let mut cur = r;
                            while let Some(i) = ed.doc.dom.get_by_ref(cur) {
                                path.push(i.name.clone());
                                cur = i.parent();
                            }
                            path.reverse();
                            let kids: Vec<String> = ed
                                .doc
                                .dom
                                .get_by_ref(r)
                                .map(|i| {
                                    i.children()
                                        .iter()
                                        .filter_map(|c| ed.doc.class_of(*c).map(|k| k.to_string()))
                                        .collect()
                                })
                                .unwrap_or_default();
                            let size =
                                crate::scene::vec3_of(ed.doc.prop(r, "Size")).unwrap_or_default();
                            format!(
                                "{} n={n} size={size:.1} pos={pos:.1} vel={vel:.1} kids={kids:?}",
                                path.join(".")
                            )
                        })
                        .collect();
                    report["awake_sample"] = serde_json::json!(sample);
                }
                let text = serde_json::to_string_pretty(&report).unwrap_or_default();
                if let Some(out) = &ed.harness.log_out {
                    let log: String = ed
                        .log
                        .iter()
                        .map(|l| format!("{:?}: {}\n", l.level, l.text.replace(crate::output_panel::STACK_MARK, "")))
                        .collect();
                    let _ = std::fs::write(out, log);
                }
                if let Some(out) = &b.out {
                    if let Err(e) = std::fs::write(out, &text) {
                        eprintln!("bench: can't write {}: {e}", out.display());
                    }
                }
                println!("{text}");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
        if ed.drag.is_some()
            || ed.play.is_some()
            || ed.assets.busy()
            || ed.textures.decoding()
            || ctx.input(|i| i.pointer.any_down() || !i.keys_down.is_empty())
            || ed.harness.screenshot.is_some()
            || ed.harness.selftest.is_some()
            || ed.account.busy()
        {
            ctx.request_repaint();
        }
        self.watchdog.complete();
    }
}

// ------------------------------------------------------------------ harness

impl Editor {
    fn harness_tick(&mut self, ctx: &egui::Context) {
        if self.harness.start_play && self.viewport_rect.is_positive() && self.frame_no >= 2 && !self.place_loading() {
            self.harness.start_play = false;
            self.start_play(crate::runtime::RunMode::Play);
            self.harness.play_started = Some(Instant::now());
        }
        if self.harness.play_started.is_some() {
            let now = Instant::now();
            if let Some(last) = self.harness.last_frame.replace(now) {
                self.harness.frame_ms.push((now - last).as_secs_f32() * 1000.0);
            }
        }
        // Timed Luau (`--luau`), once Play runs.
        let Some(t) = self.harness.now() else { return };
        if self.play.is_some() {
            let due: Vec<(bool, String)> = self
                .harness
                .actions
                .iter()
                .filter_map(|(at, a)| match a {
                    HarnessAction::Luau(server, code) if *at <= t => Some((*server, code.clone())),
                    _ => None,
                })
                .collect();
            if !due.is_empty() {
                self.harness
                    .actions
                    .retain(|(at, a)| !(matches!(a, HarnessAction::Luau(..)) && *at <= t));
                for (server, code) in due {
                    let side = if server {
                        crate::runtime::Side::Server
                    } else {
                        crate::runtime::Side::Client
                    };
                    if let Some(e) = self.run_luau(side, &code) {
                        self.error(format!("--luau: {e}"));
                    }
                }
            }
        }
        if let Some(mut t) = self.harness.selftest.take() {
            let done = t.tick(self, ctx);
            self.harness.selftest = Some(t);
            if done {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        let Some(path) = self.harness.screenshot.clone() else {
            return;
        };
        let now = self.harness.now();
        // `--shots`: the next timed shot, else the single one.
        let multi = self.harness.shot_next < self.harness.shots.len();
        let due = if multi {
            now.is_some_and(|t| t >= self.harness.shots[self.harness.shot_next])
        } else if !self.harness.shots.is_empty() {
            false
        } else {
            match self.harness.shot_after {
                Some(s) => now.is_some_and(|t| t >= s),
                None => self.frame_no >= mech::SCREENSHOT_FRAMES,
            }
        };
        let path = if multi {
            let t = self.harness.shots[self.harness.shot_next];
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            path.with_file_name(format!("{stem}-{t}s.png"))
        } else {
            path
        };
        if due && !self.harness.screenshot_requested {
            self.harness.screenshot_requested = true;
            self.write_asset_report();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } if self.harness.screenshot_requested => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            if let Ok(hdr_path) = std::env::var("RUSTUDIO_DUMP_HDR") {
                if let Some((w,h,data)) = self.renderer.as_ref().zip(self.render_state.as_ref()).and_then(|(r,rs)| r.read_hdr(rs)) {
                    if std::fs::write(&hdr_path, data).is_ok() { eprintln!("HDR {w} {h} RGBA16Float {hdr_path}"); }
                }
            }
            let mut written = Vec::new();
            match save_png(&image, &path) {
                Ok(()) => written.push(path.clone()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            // Just the 3D view too (FILE.view.png), to compare with Studio's viewport.
            if (self.harness.shot_after.is_some() || multi) && self.viewport_rect.is_positive() {
                let ppp = ctx.pixels_per_point();
                let r = self.viewport_rect;
                let (x0, y0) = ((r.min.x * ppp) as usize, (r.min.y * ppp) as usize);
                let (w, h) = (
                    ((r.width() * ppp) as usize).min(image.size[0].saturating_sub(x0)),
                    ((r.height() * ppp) as usize).min(image.size[1].saturating_sub(y0)),
                );
                let pixels = (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .map(|(x, y)| image.pixels[(y0 + y) * image.size[0] + x0 + x])
                    .collect();
                let view = egui::ColorImage::new([w, h], pixels);
                let view_path = path.with_extension("view.png");
                if save_png(&view, &view_path).is_ok() {
                    written.push(view_path);
                }
            }
            // The written paths, for callers that don't rebuild the names.
            for p in written {
                let p = std::path::absolute(&p).unwrap_or(p);
                let line = format!("[shot] {}", p.display());
                println!("{line}");
                self.harness.shot_lines.push(line);
            }
            if let Some(out) = &self.harness.log_out {
                let text: String = self
                    .log
                    .iter()
                    .map(|l| format!("{:?}: {}\n", l.level, l.text.replace(crate::output_panel::STACK_MARK, "")))
                    .chain(self.harness.shot_lines.iter().map(|l| format!("{l}\n")))
                    .collect();
                if let Err(e) = std::fs::write(out, text) {
                    eprintln!("log: can't write {}: {e}", out.display());
                }
            }
            crate::telemetry::flush();
            if multi {
                self.harness.shot_next += 1;
                self.harness.screenshot_requested = false;
                self.write_frame_times(&path);
                self.write_cursor_state(&path);
                if self.harness.shot_next < self.harness.shots.len() {
                    return;
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.harness.shot_after.is_none()
            && self.harness.shots.is_empty()
            && self.frame_no > mech::SCREENSHOT_FRAMES + 600
        {
            eprintln!("screenshot timed out");
            std::process::exit(2);
        }
    }
}

impl Editor {
    /// FILE.frames.json beside a shot: frame times since Play started.
    fn write_frame_times(&self, shot: &std::path::Path) {
        let mut ms = self.harness.frame_ms.clone();
        if ms.is_empty() {
            return;
        }
        let mean = ms.iter().sum::<f32>() / ms.len() as f32;
        ms.sort_by(|a, b| a.total_cmp(b));
        let p99 = ms[((ms.len() as f32 * 0.99) as usize).min(ms.len() - 1)];
        let body = serde_json::json!({
            "frames": ms.len(),
            "mean_ms": mean,
            "p99_ms": p99,
            "fps": 1000.0 / mean.max(1e-3),
        });
        let _ = std::fs::write(shot.with_extension("frames.json"), body.to_string());
    }
}

#[derive(Debug, Clone)]
pub enum HarnessAction {
    Click(f32, f32),
    Key(egui::Key),
    /// A key held down (true) or let go (false): `--key NAME@S:HOLD`.
    KeyState(egui::Key, bool),
    /// `--wheel NOTCHES@S` over the 3D view's centre (positive = forward,
    /// zoom in), or `--wheel NOTCHES@S:X,Y` at that view position.
    Wheel(f32, Option<(f32, f32)>),
    /// `--type "TEXT"@S`: typed text (egui Text events: a focused TextBox
    /// receives the characters).
    Text(String),
    /// Shift held down or let go.
    Shift(bool),
    /// The right mouse button over the 3D view's centre, down or up.
    RightButton(bool),
    /// Raw mouse motion (pixels), as a grabbed cursor reports it.
    MouseMotion(f32, f32),
    /// The cursor at a view position (`--ldrag`), and the left button there
    /// going down / up.
    PointerAt(f32, f32),
    LeftButton(f32, f32, bool),
    /// `--luau server|client FILE@S`: run Luau on that side of Play.
    Luau(bool, String),
    /// `--action NAME@S`: a Studio menu action by id (`panel_output` closes
    /// or opens the Output panel, resizing the 3D view mid-run).
    UiAction(String),
}

impl HarnessAction {
    /// "X,Y@S" for clicks, "NAME@S" for a key tap, "NAME@S:HOLD" to hold
    /// it HOLD seconds.
    pub fn parse(s: &str, click: bool) -> Option<Vec<(f32, HarnessAction)>> {
        let (what, at) = s.split_once('@')?;
        if click {
            let at: f32 = at.trim().parse().ok()?;
            let (x, y) = what.split_once(',')?;
            return Some(vec![(
                at,
                HarnessAction::Click(x.trim().parse().ok()?, y.trim().parse().ok()?),
            )]);
        }
        let key = egui::Key::from_name(what.trim())?;
        match at.split_once(':') {
            Some((at, hold)) => {
                let (at, hold): (f32, f32) = (at.trim().parse().ok()?, hold.trim().parse().ok()?);
                Some(vec![
                    (at, HarnessAction::KeyState(key, true)),
                    (at + hold, HarnessAction::KeyState(key, false)),
                ])
            }
            None => Some(vec![(at.trim().parse().ok()?, HarnessAction::Key(key))]),
        }
    }

    /// `--wheel NOTCHES@S` or `--wheel NOTCHES@S:X,Y` (view coordinates).
    pub fn parse_wheel(s: &str) -> Option<(f32, HarnessAction)> {
        let (n, at) = s.split_once('@')?;
        let n: f32 = n.trim().parse().ok()?;
        let (at, pos) = match at.split_once(':') {
            Some((at, xy)) => {
                let (x, y) = xy.split_once(',')?;
                (at, Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
            }
            None => (at, None),
        };
        Some((at.trim().parse().ok()?, HarnessAction::Wheel(n, pos)))
    }

    /// `--type TEXT@S` (the last '@' splits, so the text may hold one).
    pub fn parse_type(s: &str) -> Option<(f32, HarnessAction)> {
        let (text, at) = s.rsplit_once('@')?;
        Some((at.trim().parse().ok()?, HarnessAction::Text(text.to_string())))
    }

    /// `--ldrag X1,Y1>X2,Y2@S`: left-drag in the 3D view (view coordinates):
    /// press at S, move in 10 steps over 0.5 s, let go at S + 0.6.
    pub fn parse_ldrag(s: &str) -> Option<Vec<(f32, HarnessAction)>> {
        let (what, at) = s.split_once('@')?;
        let at: f32 = at.trim().parse().ok()?;
        let (a, b) = what.split_once('>')?;
        let xy = |p: &str| -> Option<(f32, f32)> {
            let (x, y) = p.split_once(',')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        };
        let ((x1, y1), (x2, y2)) = (xy(a)?, xy(b)?);
        let mut v = vec![(at - 0.2, HarnessAction::PointerAt(x1, y1)), (at, HarnessAction::LeftButton(x1, y1, true))];
        for k in 1..=10 {
            let f = k as f32 / 10.0;
            v.push((at + 0.05 * k as f32, HarnessAction::PointerAt(x1 + (x2 - x1) * f, y1 + (y2 - y1) * f)));
        }
        v.push((at + 0.6, HarnessAction::LeftButton(x2, y2, false)));
        Some(v)
    }

    /// `--rdrag DX,DY@S`: right-drag over the view: press at S, move DX,DY
    /// pixels in 10 steps over 0.5 s, let go at S + 0.6.
    pub fn parse_rdrag(s: &str) -> Option<Vec<(f32, HarnessAction)>> {
        let (what, at) = s.split_once('@')?;
        let at: f32 = at.trim().parse().ok()?;
        let (x, y) = what.split_once(',')?;
        let (dx, dy): (f32, f32) = (x.trim().parse().ok()?, y.trim().parse().ok()?);
        let mut v = vec![(at, HarnessAction::RightButton(true))];
        for k in 1..=10 {
            v.push((
                at + 0.05 * k as f32,
                HarnessAction::MouseMotion(dx / 10.0, dy / 10.0),
            ));
        }
        v.push((at + 0.6, HarnessAction::RightButton(false)));
        Some(v)
    }
}

impl Editor {
    /// Timed harness input, injected like real events.
    pub fn harness_inject(&mut self, raw: &mut egui::RawInput) {
        let Some(t) = self.harness.now() else { return };
        // No 3D view yet (the start page): clicks are window coordinates.
        let vp = if self.viewport_rect.is_finite() {
            self.viewport_rect
        } else {
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::ZERO)
        };
        let mut i = 0;
        while i < self.harness.actions.len() {
            if self.harness.actions[i].0 > t {
                i += 1;
                continue;
            }
            if matches!(self.harness.actions[i].1, HarnessAction::Luau(..)) {
                i += 1;
                continue;
            }
            let (_, a) = self.harness.actions.remove(i);
            match a {
                HarnessAction::Luau(..) => {}
                HarnessAction::Click(x, y) => {
                    let p = vp.min + egui::vec2(x, y);
                    raw.events.push(egui::Event::PointerMoved(p));
                    for pressed in [true, false] {
                        raw.events.push(egui::Event::PointerButton {
                            pos: p,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
                HarnessAction::Key(key) => {
                    for pressed in [true, false] {
                        raw.events.push(egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
                HarnessAction::KeyState(key, pressed) => {
                    raw.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                HarnessAction::Wheel(n, at) => {
                    let p = at.map_or(vp.center(), |(x, y)| vp.min + egui::vec2(x, y));
                    raw.events.push(egui::Event::PointerMoved(p));
                    raw.events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta: egui::vec2(0.0, n),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                    });
                }
                HarnessAction::Text(text) => raw.events.push(egui::Event::Text(text)),
                HarnessAction::Shift(on) => self.harness.shift = on,
                HarnessAction::RightButton(pressed) => {
                    raw.events.push(egui::Event::PointerMoved(vp.center()));
                    raw.events.push(egui::Event::PointerButton {
                        pos: vp.center(),
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                HarnessAction::PointerAt(x, y) => {
                    raw.events.push(egui::Event::PointerMoved(vp.min + egui::vec2(x, y)));
                }
                HarnessAction::LeftButton(x, y, pressed) => {
                    let p = vp.min + egui::vec2(x, y);
                    raw.events.push(egui::Event::PointerMoved(p));
                    raw.events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                HarnessAction::MouseMotion(dx, dy) => {
                    raw.events.push(egui::Event::MouseMoved(egui::vec2(dx, dy)));
                }
                HarnessAction::UiAction(name) => self.harness.ui_due.push(name),
            }
        }
    }
}

pub fn save_png(image: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let [w, h] = image.size;
    let mut bytes = Vec::with_capacity(w * h * 4);
    for c in &image.pixels {
        bytes.extend_from_slice(&[c.r(), c.g(), c.b(), 255]);
    }
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }
    image::save_buffer(
        path,
        &bytes,
        w as u32,
        h as u32,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}

/// The sample scene's parts, scripts and lights, into a place document
/// (`--demo`, and the fidelity corpus's demo place). Returns the Ball.
pub fn build_demo_parts(doc: &mut Document) -> Ref {
        let ws = doc.default_parent();
        let parts: [(&str, u32, [f32; 3], [f32; 3], [u8; 3]); 6] = [
            (
                "Tower",
                1,
                [-14.0, 6.0, -10.0],
                [6.0, 12.0, 6.0],
                [196, 112, 70],
            ),
            (
                "Ball",
                0,
                [-4.0, 2.0, -4.0],
                [4.0, 4.0, 4.0],
                [80, 160, 220],
            ),
            (
                "Pipe",
                2,
                [8.0, 1.5, -8.0],
                [10.0, 3.0, 3.0],
                [110, 190, 120],
            ),
            ("Step", 1, [10.0, 0.5, 4.0], [8.0, 1.0, 8.0], [230, 200, 90]),
            (
                "Bridge",
                1,
                [-2.0, 8.0, -18.0],
                [24.0, 1.0, 4.0],
                [150, 150, 165],
            ),
            ("Lamp", 1, [3.0, 3.0, 6.0], [1.0, 6.0, 1.0], [250, 240, 220]),
        ];
        let mut last = Ref::none();
        for (name, shape, p, s, c) in parts {
            let r = doc.insert(
                ws,
                doc::new_part(name, shape, Vec3::from(p), Vec3::from(s), c),
            );
            last = if name == "Ball" { r } else { last };
        }
        let ramp = doc::new_part(
            "Ramp",
            1,
            Vec3::new(-14.0, 2.0, 0.0),
            Vec3::new(6.0, 4.0, 8.0),
            [196, 112, 70],
        )
        .with_class("WedgePart");
        doc.insert(ws, ramp);
        let model = doc
            .insert(ws, InstanceBuilder::new("Model").with_name("Statue"));
        doc.insert(
            model,
            InstanceBuilder::new("MeshPart")
                .with_name("StatueMesh")
                .with_property("Anchored", true)
                .with_property("Size", rbx_dom_weak::types::Vector3::new(4.0, 7.0, 4.0))
                .with_property(
                    "CFrame",
                    rbx_dom_weak::types::CFrame::new(
                        rbx_dom_weak::types::Vector3::new(18.0, 4.5, -14.0),
                        rbx_dom_weak::types::Matrix3::identity(),
                    ),
                )
                .with_property(
                    "Color",
                    rbx_dom_weak::types::Color3uint8::new(170, 140, 210),
                ),
        );
        doc.insert(
            model,
            doc::new_part(
                "Plinth",
                1,
                Vec3::new(18.0, 0.5, -14.0),
                Vec3::new(6.0, 1.0, 6.0),
                [90, 90, 100],
            ),
        );
        let glass = doc::new_part(
            "Glass",
            1,
            Vec3::new(2.0, 3.0, -12.0),
            Vec3::new(8.0, 6.0, 0.5),
            [180, 220, 255],
        )
        .with_property("Transparency", 0.5f32);
        doc.insert(ws, glass);
        let neon = doc::new_part(
            "NeonStrip",
            1,
            Vec3::new(3.0, 6.25, 6.0),
            Vec3::new(2.0, 0.5, 2.0),
            [255, 120, 60],
        )
        .with_property("Material", rbx_dom_weak::types::Enum::from_u32(288));
        doc.insert(ws, neon);
        let sp = doc.find_child(ws, "Script");
        if sp.is_none() {
            let sss = doc
                .find_child(doc.dom.root_ref(), "ServerScriptService")
                .unwrap_or(ws);
            doc.insert(
                sss,
                InstanceBuilder::new("Script")
                    .with_name("Main")
                    .with_property("Source", "print(\"Hello from rustudio\")\n".to_string()),
            );
        }
        last
}
