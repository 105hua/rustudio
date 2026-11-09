//! Studio's window chrome, matched to an offline Studio (references in
//! checks/ui/studio): the menu bar (File, Edit, View, Plugins, Test, Window,
//! Help with Studio's items, order and shortcuts), the top row (run type,
//! Play / Pause / Stop on the left, the ribbon tabs in the middle, the
//! account at the right) and the ribbon tabs (Home, Model, Avatar, UI,
//! Script, Plugins). What rustudio can't do yet is shown disabled, never
//! left out or faked. Names and icons are rustudio's own (Tabler, src/ti.rs).
//!
//! This file: the state and the menus' contents; studio_actions.rs: what
//! each item does; studio_ribbon.rs: the top row and the ribbon.

use std::collections::HashSet;
use std::path::PathBuf;

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::app::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RibbonTab {
    Home,
    Model,
    Avatar,
    Ui,
    Script,
    Plugins,
}

impl RibbonTab {
    pub const ALL: [RibbonTab; 6] = [
        RibbonTab::Home,
        RibbonTab::Model,
        RibbonTab::Avatar,
        RibbonTab::Ui,
        RibbonTab::Script,
        RibbonTab::Plugins,
    ];
    pub fn label(self) -> &'static str {
        match self {
            RibbonTab::Home => "Home",
            RibbonTab::Model => "Model",
            RibbonTab::Avatar => "Avatar",
            RibbonTab::Ui => "UI",
            RibbonTab::Script => "Script",
            RibbonTab::Plugins => "Plugins",
        }
    }
}

/// Studio's test drop-down: what the Play button starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunType {
    Test,
    TestHere,
    Run,
    ServerClients,
}

impl RunType {
    pub const ALL: [RunType; 4] = [RunType::Test, RunType::TestHere, RunType::Run, RunType::ServerClients];
    pub fn label(self) -> &'static str {
        match self {
            RunType::Test => "Test",
            RunType::TestHere => "Test Here",
            RunType::Run => "Run",
            RunType::ServerClients => "Server & Clients",
        }
    }
    /// Server & Clients needs several client processes: not in rustudio yet.
    pub fn available(self) -> bool {
        self != RunType::ServerClients
    }
}

/// The Insert Object popup (Explorer row "+", Window > Insert Object).
pub struct InsertPopup {
    pub parent: rbx_dom_weak::types::Ref,
    pub at: egui::Pos2,
    pub query: String,
    pub opened_frame: u64,
}

pub struct UiState {
    pub tab: RibbonTab,
    pub run_type: RunType,
    /// Dock panels to open / close at the end of the frame (None = toggle).
    pub panel_requests: Vec<(Tab, Option<bool>)>,
    /// Panels open in the dock last frame (their menu checkmarks).
    pub panels_open: HashSet<&'static str>,
    pub zoom: f32,
    pub fullscreen: bool,
    pub show_gui: bool,
    pub grid_size: f32,
    pub about: bool,
    pub insert_popup: Option<InsertPopup>,
    pub recent: Vec<PathBuf>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            tab: RibbonTab::Home,
            run_type: RunType::Test,
            panel_requests: Vec::new(),
            panels_open: HashSet::new(),
            zoom: 1.0,
            fullscreen: false,
            show_gui: true,
            grid_size: crate::mech::GRID_SPACING,
            about: false,
            insert_popup: None,
            recent: load_recent(),
        }
    }
}

fn recent_file() -> PathBuf {
    let base = crate::paths::config();
    base.join("rustudio").join("recent.txt")
}

fn load_recent() -> Vec<PathBuf> {
    std::fs::read_to_string(recent_file())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .take(10)
        .collect()
}

/// Remembers an opened file or folder for File > Recent (newest first).
/// Test runs (RUSTUDIO_NO_RECENT; the harness sets it) don't.
pub fn remember_recent(ui: &mut UiState, p: &std::path::Path) {
    if std::env::var_os("RUSTUDIO_NO_RECENT").is_some() {
        return;
    }
    let p = p.to_path_buf();
    ui.recent.retain(|r| *r != p);
    ui.recent.insert(0, p);
    ui.recent.truncate(10);
    save_recent(ui);
}

/// Writes File > Recent (the Start Page's Remove from Recent too).
pub fn save_recent(ui: &UiState) {
    if std::env::var_os("RUSTUDIO_NO_RECENT").is_some() {
        return;
    }
    let f = recent_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let text: String = ui.recent.iter().map(|p| format!("{}\n", p.display())).collect();
    let _ = std::fs::write(f, text);
}

/// A menu entry: (label, shortcut, action), a separator, or a submenu.
pub enum M {
    I(&'static str, &'static str, &'static str),
    S,
    Sub(&'static str, &'static [M]),
}
use M::{Sub, I, S};

pub const FILE: &[M] = &[
    I("New", "Ctrl+N", "new"),
    I("Open from File", "Ctrl+O", "open"),
    I("Open from Roblox", "Ctrl+Shift+O", "open_roblox"),
    I("Open Folder…", "", "open_folder"),
    I("Save as Project Folder…", "", "export_project"),
    I("Project Settings…", "", "project_settings"),
    Sub("Recent", &[]),
    S,
    I("Close Place", "Ctrl+F4", "close_place"),
    S,
    I("Import", "Ctrl+M", "import_3d"),
    I("Import Roblox Model", "", "import_model"),
    I("Export as .obj", "", "export_obj"),
    I("Export as glTF", "", "export_gltf"),
    S,
    I("Save to File", "", "save"),
    I("Save to File As", "Ctrl+Shift+S", "save_as"),
    I("Save to Roblox", "", "save_roblox"),
    I("Save to Roblox As", "", "save_roblox"),
    S,
    I("Publish", "Alt+P", "publish"),
    I("Publish As", "", "publish"),
    I("Manage Audience", "", "manage_audience"),
    S,
    I("Experience Settings", "", "experience_settings"),
    I("Avatar Settings", "", "avatar_settings"),
    I("Open Configs", "", "open_configs"),
    S,
    I("Studio Settings", "Alt+S", "studio_settings"),
    I("Beta Features", "", "beta_features"),
    I("Customize Shortcuts", "", "customize_shortcuts"),
    S,
    I("Open Auto Saves", "", "auto_saves"),
    S,
    I("About rustudio", "", "about"),
    I("Restart rustudio", "", "restart"),
    S,
    I("Exit", "", "exit"),
];

pub const EDIT: &[M] = &[
    I("Undo", "Ctrl+Z", "undo"),
    I("Redo", "Ctrl+Y", "redo"),
    S,
    I("Cut", "Ctrl+X", "cut"),
    I("Copy", "Ctrl+C", "copy"),
    I("Paste", "Ctrl+V", "paste"),
    I("Paste Into", "Ctrl+Shift+V", "paste_into"),
    I("Paste at Original Location", "", "paste_original"),
    I("Paste into at Original Location", "", "paste_into_original"),
    I("Duplicate", "Ctrl+D", "duplicate"),
    I("Delete", "Del", "delete"),
    S,
    Sub("Group", &[I("Group as Model", "Ctrl+G", "group"), I("Group as Folder", "Ctrl+Alt+G", "group_folder")]),
    I("Ungroup", "Ctrl+U", "ungroup"),
    Sub("Lock", &[I("Lock Selection", "Alt+L", "lock"), I("Lock Tool", "", "lock_tool")]),
    I("Unlock All", "", "unlock_all"),
    Sub("Anchor", &[I("Anchor Selection", "Alt+A", "anchor"), I("Anchor Tool", "", "anchor_tool")]),
    I("Select All", "Ctrl+A", "select_all"),
    S,
    I("Convert to Package", "", "convert_package"),
    I("Get Latest Package", "", "latest_package"),
    S,
    I("Save as Default Instance", "", "default_instance"),
    S,
    I("Format Document", "", "format_document"),
    I("Format Selection", "Alt+Shift+F", "format_selection"),
    I("Indent", "Ctrl+]", "indent"),
    I("Unindent", "Ctrl+[", "unindent"),
];

pub const VIEW: &[M] = &[
    I("Zoom In", "Ctrl+=", "zoom_in"),
    I("Zoom Out", "Ctrl+-", "zoom_out"),
    I("Full Screen", "F11", "full_screen"),
    S,
    I("Show View Selector", "", "view_selector"),
    I("Show Wind Direction", "", "wind_direction"),
    Sub("Show Grid", &[
        I("Show Grid", "Alt+G", "show_grid"),
        I("2 Studs", "", "grid_2"),
        I("4 Studs", "", "grid_4"),
        I("16 Studs", "", "grid_16"),
    ]),
    I("Show Grid Material", "", "grid_material"),
    I("Show Wireframe Rendering", "", "wireframe"),
    I("Show UI", "", "show_ui"),
    S,
    I("Show Welds", "Alt+W", "show_welds"),
    I("Show Constraint Details", "Alt+D", "constraint_details"),
    S,
    I("Expand Script Folds", "Ctrl+E", "expand_folds"),
    I("Collapse Script Folds", "Ctrl+Shift+E", "collapse_folds"),
    I("Toggle Comment", "Ctrl+/", "toggle_comment"),
    S,
    I("Screenshot", "", "screenshot"),
    S,
    I("Mute", "", "mute"),
];

pub const PLUGINS: &[M] = &[
    I("Manage Plugins", "", "manage_plugins"),
    I("Plugins Folder", "", "plugins_folder"),
    S,
    I("Save as Local Plugin", "", "save_local_plugin"),
    I("Publish as Plugin", "", "publish_plugin"),
];

pub const TEST: &[M] = &[
    Sub("Start Test Session", &[
        I("Test", "F5", "test"),
        I("Test Here", "", "test_here"),
        I("Run", "F8", "run"),
        I("Server and Clients", "F7", "server_clients"),
        I("Team Test", "", "team_test"),
    ]),
    S,
    I("Device Simulator", "", "device_simulator"),
    I("Controller Emulator", "", "controller_emulator"),
    I("Player Emulator", "", "player_emulator"),
    S,
    Sub("Debug Error", &[
        I("Never", "", "break_never"),
        I("On Unhandled Exceptions", "", "break_unhandled"),
        I("On All Exceptions", "", "break_all"),
    ]),
    I("Reload Script", "Ctrl+R", "reload_script"),
    S,
    I("Toggle Client/Server View", "", "toggle_client_server"),
    I("Show Client Viewport", "", "client_viewport"),
    I("Show / Hide Server Viewport", "", "server_viewport"),
    I("Add Clients", "", "add_clients"),
    S,
    I("Pause", "", "pause"),
    I("Pause Client/Server", "", "pause_side"),
    S,
    I("Stop", "Shift+F5", "stop"),
    I("End Session", "", "end_session"),
    I("Exit Client", "", "exit_client"),
];

pub const WINDOW: &[M] = &[
    I("Toolbox", "", "toolbox"),
    I("Properties", "", "panel_properties"),
    I("Explorer", "Alt+X", "panel_explorer"),
    I("Output", "", "panel_output"),
    I("Activity History", "", "activity_history"),
    I("Asset Manager", "", "asset_manager"),
    I("Insert Object", "", "insert_object"),
    S,
    Sub("3D", &[
        I("Align", "", "align"),
        I("Collision Groups", "", "collision_groups"),
        I("Material", "", "material_manager"),
        I("Material Generator", "", "material_generator"),
        I("Terrain", "", "terrain_editor"),
    ]),
    Sub("Avatar", &[I("Clip Editor", "", "clip_editor"), I("Accessory", "", "accessory_editor"), I("Setup", "", "avatar_setup")]),
    Sub("Collaboration", &[I("Live Collaborators", "", "live_collaborators"), I("Invite Collaborators", "", "invite_collaborators")]),
    Sub("Debug", &[I("Breakpoints", "", "breakpoints"), I("Call Stack", "", "call_stack"), I("Watch", "", "watch")]),
    Sub("Localization", &[I("Localization Tools", "", "localization_tools")]),
    Sub("Performance Summary", &[
        I("Performance Diagnostics", "", "perf_diagnostics"),
        I("MicroProfiler", "Ctrl+Alt+F6", "microprofiler"),
        I("Network Summary", "Ctrl+Shift+F3", "net_summary"),
        I("Performance Summary", "Ctrl+Shift+F5", "perf_summary"),
        I("Physics", "Ctrl+Shift+F4", "physics_stats"),
        I("Render", "Ctrl+Shift+F2", "render_stats"),
        I("Stats", "Ctrl+Shift+F1", "stats"),
        I("Scene Analysis", "", "scene_analysis"),
    ]),
    Sub("Script", &[
        I("Command Bar", "", "command_bar"),
        I("Find in Place", "", "find_in_place"),
        I("Object Browser", "", "object_browser"),
        I("Analysis", "", "script_analysis"),
        I("Script Activity", "", "script_activity"),
        I("Run Script", "", "run_script"),
        I("Tag Editor", "", "tag_editor"),
        I("Task Scheduler", "", "task_scheduler"),
    ]),
    I("Reset Layout", "", "reset_layout"),
];

pub const HELP: &[M] = &[
    I("Documentation", "", "documentation"),
    I("Tutorials", "", "tutorials"),
    I("Developer Forum", "", "developer_forum"),
    S,
    I("Start Studio Tour", "", "studio_tour"),
    S,
    I("Open Logs Folder", "", "logs_folder"),
];

pub const MENUS: &[(&str, &[M])] = &[
    ("File", FILE),
    ("Edit", EDIT),
    ("View", VIEW),
    ("Plugins", PLUGINS),
    ("Test", TEST),
    ("Window", WINDOW),
    ("Help", HELP),
];

/// Shortcut text ("Ctrl+Shift+V", "Alt+P", "F5", "Del") → egui shortcut.
pub fn parse_shortcut(s: &str) -> Option<KeyboardShortcut> {
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split('+').collect();
    let (key, mods) = (parts.last().copied().unwrap_or(""), &parts[..parts.len() - 1]);
    let mut m = Modifiers::NONE;
    for p in mods {
        match *p {
            "Ctrl" => m = m | Modifiers::COMMAND,
            "Shift" => m = m | Modifiers::SHIFT,
            "Alt" => m = m | Modifiers::ALT,
            _ => return None,
        }
    }
    let k = match key {
        "Del" => Key::Delete,
        "=" => Key::Equals,
        "-" => Key::Minus,
        "/" => Key::Slash,
        "]" => Key::CloseBracket,
        "[" => Key::OpenBracket,
        other => Key::from_name(other)?,
    };
    Some(KeyboardShortcut::new(m, k))
}

/// Every (shortcut, action) in the menus, most modifiers first (so
/// Ctrl+Shift+V is taken before Ctrl+V).
pub fn menu_shortcuts() -> Vec<(KeyboardShortcut, &'static str)> {
    fn walk(items: &'static [M], out: &mut Vec<(KeyboardShortcut, &'static str)>) {
        for m in items {
            match m {
                I(_, sc, a) => {
                    if let Some(k) = parse_shortcut(sc) {
                        out.push((k, a));
                    }
                }
                Sub(_, items) => walk(items, out),
                S => {}
            }
        }
    }
    let mut out = Vec::new();
    for (_, items) in MENUS {
        walk(items, &mut out);
    }
    out.sort_by_key(|(k, _)| {
        std::cmp::Reverse(k.modifiers.command as u8 + k.modifiers.shift as u8 + k.modifiers.alt as u8)
    });
    out
}

pub fn menu_rect_key(title: &str) -> &'static str {
    match title {
        "File" => "menu.File",
        "Edit" => "menu.Edit",
        "View" => "menu.View",
        "Plugins" => "menu.Plugins",
        "Test" => "menu.Test",
        "Window" => "menu.Window",
        _ => "menu.Help",
    }
}

pub fn tab_rect_key(t: RibbonTab) -> &'static str {
    match t {
        RibbonTab::Home => "tab.Home",
        RibbonTab::Model => "tab.Model",
        RibbonTab::Avatar => "tab.Avatar",
        RibbonTab::Ui => "tab.UI",
        RibbonTab::Script => "tab.Script",
        RibbonTab::Plugins => "tab.Plugins",
    }
}

pub fn run_type_key(t: RunType) -> &'static str {
    match t {
        RunType::Test => "runtype.Test",
        RunType::TestHere => "runtype.TestHere",
        RunType::Run => "runtype.Run",
        RunType::ServerClients => "runtype.ServerClients",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every menu shortcut parses.
    #[test]
    fn shortcuts_parse() {
        fn walk(items: &[M], bad: &mut Vec<String>) {
            for m in items {
                match m {
                    I(l, sc, _) if !sc.is_empty() && parse_shortcut(sc).is_none() => bad.push(format!("{l}: {sc}")),
                    Sub(_, s) => walk(s, bad),
                    _ => {}
                }
            }
        }
        let mut bad = Vec::new();
        for (_, items) in MENUS {
            walk(items, &mut bad);
        }
        assert!(bad.is_empty(), "unparsed shortcuts: {bad:?}");
    }
}
