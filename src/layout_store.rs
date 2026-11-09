//! Saved layout: the dock (where Explorer / Properties / Output / … sit and
//! their sizes, floating panels too), the window's size, position and
//! maximized state, UI zoom, ribbon tab, run type, snapping and grid.
//! Studio keeps these between sessions; so do we, in
//! `$XDG_CONFIG_HOME/rustudio/layout.json`.
//!
//! Only plain interactive runs read or write it (`Harness::plain`, no
//! `--window`): tests, benches and screenshots keep the default layout.
//! `RUSTUDIO_NO_LAYOUT=1` ignores it too; `RUSTUDIO_LAYOUT=1` forces it on. Script tabs aren't kept (they
//! name instances of the place that was open).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui_dock::DockState;
use serde::{Deserialize, Serialize};

use crate::app::{Editor, Harness, Tab};
use crate::studio_ui::{RibbonTab, RunType};

const VERSION: u32 = 1;
/// How often a changed layout is written while running (and on exit).
const SAVE_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavedTab {
    Viewport,
    Explorer,
    Properties,
    Output,
    AvatarSettings,
    TerrainEditor,
}

impl SavedTab {
    fn from_tab(t: &Tab) -> Option<Self> {
        Some(match t {
            Tab::Viewport => SavedTab::Viewport,
            Tab::Explorer => SavedTab::Explorer,
            Tab::Properties => SavedTab::Properties,
            Tab::Output => SavedTab::Output,
            Tab::AvatarSettings => SavedTab::AvatarSettings,
            Tab::TerrainEditor => SavedTab::TerrainEditor,
            Tab::Script(_) => return None,
        })
    }
    fn tab(self) -> Tab {
        match self {
            SavedTab::Viewport => Tab::Viewport,
            SavedTab::Explorer => Tab::Explorer,
            SavedTab::Properties => Tab::Properties,
            SavedTab::Output => Tab::Output,
            SavedTab::AvatarSettings => Tab::AvatarSettings,
            SavedTab::TerrainEditor => Tab::TerrainEditor,
        }
    }
}

/// The window, in logical pixels (what `ViewportBuilder` takes).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeom {
    pub size: [f32; 2],
    /// Outer top-left; X11 only (Wayland never tells a window where it is).
    #[serde(default)]
    pub pos: Option<[f32; 2]>,
    #[serde(default)]
    pub maximized: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub version: u32,
    pub window: Option<WindowGeom>,
    pub dock: Option<DockState<SavedTab>>,
    pub zoom: Option<f32>,
    pub ribbon_tab: Option<String>,
    pub run_type: Option<String>,
    pub show_grid: Option<bool>,
    pub grid_size: Option<f32>,
    pub show_gui: Option<bool>,
    pub snap_move: Option<bool>,
    pub move_inc: Option<f32>,
    pub snap_rotate: Option<bool>,
    pub rotate_inc: Option<f32>,
    pub local_space: Option<bool>,
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("rustudio").join("layout.json")
}

impl Harness {
    /// A run a person drives (not a test, bench, screenshot or scripted run).
    pub fn plain(&self) -> bool {
        self.selftest.is_none()
            && self.bench.is_none()
            && self.screenshot.is_none()
            && self.ui_shots.is_none()
            && self.actions.is_empty()
            && self.shots.is_empty()
            && !self.start_play
            && self.open_roblox.is_none()
            && self.roblox_picker.is_none()
            && !self.roblox_settings
    }
}

/// Whether this run keeps its layout (see the module doc).
pub fn enabled(h: &Harness) -> bool {
    // RUSTUDIO_LAYOUT=1 forces it (checking a saved layout with --screenshot).
    if std::env::var_os("RUSTUDIO_LAYOUT").is_some() {
        return true;
    }
    let window_arg = std::env::args().any(|a| a == "--window");
    h.plain() && !window_arg && std::env::var_os("RUSTUDIO_NO_LAYOUT").is_none()
}

pub fn load() -> Option<Layout> {
    let text = std::fs::read_to_string(path()).ok()?;
    match serde_json::from_str::<Layout>(&text) {
        Ok(l) if l.version == VERSION => Some(l),
        Ok(_) => None,
        Err(e) => {
            eprintln!("rustudio: ignoring {}: {e}", path().display());
            None
        }
    }
}

fn finite(v: f32) -> Option<f32> {
    v.is_finite().then_some(v)
}

impl Layout {
    /// The saved dock, if it still has the 3D view (else the default is used).
    pub fn dock(&self) -> Option<DockState<Tab>> {
        let d = self.dock.as_ref()?;
        let dock = d.map_tabs(|t| t.tab());
        dock.find_tab(&Tab::Viewport).is_some().then_some(dock)
    }

    /// Puts the saved settings on the editor.
    pub fn apply(&self, ed: &mut Editor) {
        if let Some(z) = self.zoom.and_then(finite) {
            ed.ui.zoom = z.clamp(0.5, 3.0);
        }
        if let Some(t) = &self.ribbon_tab {
            if let Some(t) = RibbonTab::ALL.into_iter().find(|r| format!("{r:?}") == *t) {
                ed.ui.tab = t;
            }
        }
        if let Some(t) = &self.run_type {
            if let Some(t) = RunType::ALL.into_iter().find(|r| format!("{r:?}") == *t) {
                ed.ui.run_type = t;
            }
        }
        if let Some(v) = self.show_grid { ed.show_grid = v; }
        if let Some(v) = self.grid_size.and_then(finite).filter(|v| *v > 0.0) { ed.ui.grid_size = v; }
        if let Some(v) = self.show_gui { ed.ui.show_gui = v; }
        if let Some(v) = self.snap_move { ed.snap_move = v; }
        if let Some(v) = self.move_inc.and_then(finite).filter(|v| *v > 0.0) { ed.move_inc = v; }
        if let Some(v) = self.snap_rotate { ed.snap_rotate = v; }
        if let Some(v) = self.rotate_inc.and_then(finite).filter(|v| *v > 0.0) { ed.rotate_inc = v; }
        if let Some(v) = self.local_space { ed.local_space = v; }
    }

    /// Size, position and maximized state for the window about to open.
    pub fn window_builder(&self, b: egui::ViewportBuilder) -> egui::ViewportBuilder {
        let Some(w) = self.window else { return b };
        let mut b = b;
        if w.size.iter().all(|v| v.is_finite() && *v >= 200.0) {
            b = b.with_inner_size(w.size);
        }
        if let Some(p) = w.pos.filter(|p| p.iter().all(|v| v.is_finite())) {
            b = b.with_position(p);
        }
        if w.maximized {
            b = b.with_maximized(true);
        }
        b
    }
}

/// Writes the layout when it changes (every couple of seconds) and on exit.
pub struct Saver {
    /// The JSON last written (or read), to skip unchanged writes.
    last: Option<String>,
    /// When the file was last written.
    written: Instant,
    /// The last un-maximized, windowed geometry (what to restore to).
    normal: Option<WindowGeom>,
}

impl Saver {
    pub fn new(loaded: Option<&Layout>) -> Self {
        Saver {
            last: loaded.and_then(|l| serde_json::to_string_pretty(l).ok()),
            written: Instant::now() - SAVE_EVERY,
            normal: loaded.and_then(|l| l.window),
        }
    }

    /// Call every frame. A change is written at most every SAVE_EVERY; one
    /// that has to wait asks for a frame then (egui only redraws on input).
    pub fn tick(&mut self, ctx: &egui::Context, dock: &DockState<Tab>, ed: &Editor) {
        self.track_window(ctx);
        let text = self.text(dock, ed);
        if self.last.as_deref() == Some(text.as_str()) {
            return;
        }
        let since = self.written.elapsed();
        if since < SAVE_EVERY {
            ctx.request_repaint_after(SAVE_EVERY - since);
            return;
        }
        self.write(text);
    }

    fn track_window(&mut self, ctx: &egui::Context) {
        let zoom = ctx.zoom_factor();
        let (inner, outer, maximized, fullscreen, minimized) = ctx.input(|i| {
            let v = i.viewport();
            (
                v.inner_rect,
                v.outer_rect,
                v.maximized.unwrap_or(false),
                v.fullscreen.unwrap_or(false),
                v.minimized.unwrap_or(false),
            )
        });
        if fullscreen || minimized {
            return;
        }
        let Some(inner) = inner else { return };
        let prev = self.normal;
        if maximized {
            // Keep the size to come back to; just note it's maximized.
            let mut g = prev.unwrap_or(WindowGeom { size: [1600.0, 900.0], pos: None, maximized: true });
            g.maximized = true;
            self.normal = Some(g);
            return;
        }
        let size = inner.size() * zoom;
        let pos = outer.map(|o| [o.min.x * zoom, o.min.y * zoom]);
        self.normal = Some(WindowGeom { size: [size.x.round(), size.y.round()], pos, maximized: false });
    }

    /// Writes now if anything changed (on exit).
    pub fn save(&mut self, dock: &DockState<Tab>, ed: &Editor) {
        let text = self.text(dock, ed);
        if self.last.as_deref() != Some(text.as_str()) {
            self.write(text);
        }
    }

    fn text(&self, dock: &DockState<Tab>, ed: &Editor) -> String {
        let layout = Layout {
            version: VERSION,
            window: self.normal,
            dock: Some(dock.filter_map_tabs(SavedTab::from_tab)),
            zoom: Some(ed.ui.zoom),
            ribbon_tab: Some(format!("{:?}", ed.ui.tab)),
            run_type: Some(format!("{:?}", ed.ui.run_type)),
            show_grid: Some(ed.show_grid),
            grid_size: Some(ed.ui.grid_size),
            show_gui: Some(ed.ui.show_gui),
            snap_move: Some(ed.snap_move),
            move_inc: Some(ed.move_inc),
            snap_rotate: Some(ed.snap_rotate),
            rotate_inc: Some(ed.rotate_inc),
            local_space: Some(ed.local_space),
        };
        serde_json::to_string_pretty(&layout).unwrap_or_default()
    }

    fn write(&mut self, text: String) {
        self.written = Instant::now();
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, &text).is_ok() && std::fs::rename(&tmp, &p).is_ok() {
            self.last = Some(text);
        }
    }
}
