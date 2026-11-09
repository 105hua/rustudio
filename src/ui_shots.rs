//! `--ui-shots DIR`: pictures of rustudio's chrome for the side-by-side
//! comparison with Studio (checks/ui; tools/ui_compare.py makes the pairs):
//! each ribbon tab, each menu opened by a click on its title, the Explorer,
//! the test drop-down and the account menu. Each is DIR/<name>.png (the whole
//! window) with DIR/rects.json giving where to crop.

use std::path::PathBuf;

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

use crate::app::Editor;
use crate::studio_ui::{menu_rect_key, RibbonTab, MENUS};

enum Show {
    LiveCollaborators,
    LiveDraft,
    Tab(RibbonTab),
    Click(&'static str),
    Plain,
    /// Select this object (Studio's reference set) and show its Properties.
    Props(&'static str),
    /// Run Studio's Output test script (print, warn, error) with timestamps.
    OutputRun,
    /// Open one of the Properties editors on Studio's RefPart: "brickcolor"
    /// (the palette popup), "color" (Select Color) or "slider" (Transparency).
    Editor(&'static str),
    /// The Terrain Editor panel with this tool picked.
    Terrain(crate::terrain_editor::Tool),
}

pub struct UiShots {
    dir: PathBuf,
    shots: Vec<(String, Show)>,
    i: usize,
    t: u32,
    queue: Vec<Event>,
    asked: bool,
    rects: serde_json::Map<String, serde_json::Value>,
    pub done: bool,
}

impl UiShots {
    pub fn new(dir: PathBuf) -> Self {
        if std::env::var("RUSTUDIO_UI_SHOTS").as_deref()==Ok("live") {
            let _=std::fs::create_dir_all(&dir);
            return UiShots {dir,shots:vec![("live-collaborators".into(),Show::LiveCollaborators),("live-script-draft".into(),Show::LiveDraft)],i:0,t:0,queue:Vec::new(),asked:false,rects:Default::default(),done:false};
        }
        // RUSTUDIO_UI_SHOTS=props: the Properties panel for the objects
        // Studio's were captured with (run with a tall --window).
        if std::env::var("RUSTUDIO_UI_SHOTS").as_deref() == Ok("props") {
            let shots = ["Part", "Model", "Script", "ScreenGui", "Lighting"]
                .into_iter()
                .map(|n| (format!("props-{n}"), Show::Props(n)))
                .collect();
            let _ = std::fs::create_dir_all(&dir);
            return UiShots { dir, shots, i: 0, t: 0, queue: Vec::new(), asked: false, rects: Default::default(), done: false };
        }
        // RUSTUDIO_UI_SHOTS=editors: the Properties editors (round 18).
        if std::env::var("RUSTUDIO_UI_SHOTS").as_deref() == Ok("editors") {
            let _ = std::fs::create_dir_all(&dir);
            let shots = ["brickcolor", "color", "slider", "material-enum", "ribbon-material", "ribbon-color"].into_iter().map(|n| (format!("edit-{n}"), Show::Editor(n))).collect();
            return UiShots { dir, shots, i: 0, t: 0, queue: Vec::new(), asked: false, rects: Default::default(), done: false };
        }
        // RUSTUDIO_UI_SHOTS=terrain: the Terrain Editor with Draw, then Generate.
        if std::env::var("RUSTUDIO_UI_SHOTS").as_deref() == Ok("terrain") {
            let _ = std::fs::create_dir_all(&dir);
            use crate::terrain_editor::Tool;
            let shots = [
                ("draw", Tool::Draw),
                ("generate", Tool::Generate),
                ("select", Tool::Select),
                ("move", Tool::Move),
                ("rotate", Tool::Rotate),
                ("fill", Tool::Fill),
            ]
                .into_iter()
                .map(|(n, t)| (format!("terrain-{n}"), Show::Terrain(t)))
                .collect();
            return UiShots { dir, shots, i: 0, t: 0, queue: Vec::new(), asked: false, rects: Default::default(), done: false };
        }
        // RUSTUDIO_UI_SHOTS=output: the Output after Studio's test script.
        if std::env::var("RUSTUDIO_UI_SHOTS").as_deref() == Ok("output") {
            let _ = std::fs::create_dir_all(&dir);
            let shots = vec![("output-run-ts".to_string(), Show::OutputRun)];
            return UiShots { dir, shots, i: 0, t: 0, queue: Vec::new(), asked: false, rects: Default::default(), done: false };
        }
        let mut shots: Vec<(String, Show)> = RibbonTab::ALL
            .iter()
            .map(|t| (format!("ribbon-{}", t.label()), Show::Tab(*t)))
            .collect();
        for (title, _) in MENUS {
            shots.push((format!("menu-{title}"), Show::Click(menu_rect_key(title))));
        }
        shots.push(("explorer".into(), Show::Plain));
        shots.push(("test-dropdown".into(), Show::Click("test.type")));
        shots.push(("account-menu".into(), Show::Click("account")));
        let _ = std::fs::create_dir_all(&dir);
        UiShots { dir, shots, i: 0, t: 0, queue: Vec::new(), asked: false, rects: Default::default(), done: false }
    }

    pub fn inject(&mut self, raw: &mut RawInput) {
        raw.events.extend(self.queue.drain(..));
        raw.focused = true;
    }
}

impl Editor {
    /// Studio's reference objects (the capture's RefPart, RefModel with a
    /// part at (0, 10, 0), RefScript, RefGui; Lighting), made on first use.
    fn ui_shots_reference(&mut self, which: &str) -> Option<rbx_dom_weak::types::Ref> {
        use rbx_dom_weak::types::{CFrame, Matrix3, Variant, Vector3};
        use rbx_dom_weak::InstanceBuilder;
        let root = self.doc.dom.root_ref();
        let ws = self.doc.default_parent();
        let named = |ed: &Editor, parent: rbx_dom_weak::types::Ref, name: &str| {
            ed.doc.dom.get_by_ref(parent).and_then(|i| {
                i.children().iter().copied().find(|c| ed.doc.dom.get_by_ref(*c).is_some_and(|k| k.name == name))
            })
        };
        let at = |x: f32, y: f32, z: f32| Variant::CFrame(CFrame::new(Vector3::new(x, y, z), Matrix3::identity()));
        Some(match which {
            "Part" => match named(self, ws, "RefPart") {
                Some(r) => r,
                None => self.doc.insert(
                    ws,
                    InstanceBuilder::new("Part")
                        .with_name("RefPart")
                        .with_property("Anchored", true)
                        .with_property("CFrame", at(0.0, 6.0, -10.0)),
                ),
            },
            "Model" => match named(self, ws, "RefModel") {
                Some(r) => r,
                None => {
                    let m = self.doc.insert(ws, InstanceBuilder::new("Model").with_name("RefModel"));
                    self.doc.insert(m, InstanceBuilder::new("Part").with_name("Part").with_property("CFrame", at(0.0, 10.0, 0.0)));
                    m
                }
            },
            "Script" => {
                let sss = self.doc.find_child(root, "ServerScriptService")?;
                match named(self, sss, "RefScript") {
                    Some(r) => r,
                    None => self.doc.insert(sss, InstanceBuilder::new("Script").with_name("RefScript")),
                }
            }
            "ScreenGui" => {
                let sg = self.doc.find_child(root, "StarterGui")?;
                match named(self, sg, "RefGui") {
                    Some(r) => r,
                    None => self.doc.insert(sg, InstanceBuilder::new("ScreenGui").with_name("RefGui")),
                }
            }
            _ => self.doc.find_child(root, "Lighting")?,
        })
    }

    /// One step of `--ui-shots` per frame.
    pub fn ui_shots_tick(&mut self, ctx: &egui::Context) {
        let Some(mut s) = self.harness.ui_shots.take() else {
            return;
        };
        ctx.request_repaint();
        if self.frame_no < 30 || s.done {
            self.harness.ui_shots = Some(s);
            return;
        }
        let Some((name, show)) = s.shots.get(s.i) else {
            let _ = std::fs::write(s.dir.join("rects.json"), serde_json::to_string_pretty(&s.rects).unwrap_or_default());
            s.done = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.harness.ui_shots = Some(s);
            return;
        };
        let name = name.clone();
        if matches!(show,Show::LiveCollaborators | Show::LiveDraft) && self.live_team_create_place().is_none() {
            if self.harness.started.elapsed().as_secs()>120 {self.error("Live UI capture could not join the test place");ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
            self.harness.ui_shots=Some(s);return;
        }

        let click_at = |key: &str, ed: &Editor| ed.ui_rects.get(key).map(|r| r.center());
        match (s.t, show) {
            (0,Show::LiveCollaborators)=>self.cloud.live_collaborators_open=true,
            (0,Show::LiveDraft)=>{
                self.cloud.live_collaborators_open=false;
                let reference=self.doc.dom.descendants().find(|i|i.class.as_str()=="ModuleScript" && i.name=="rustudio_script_probe" && self.doc.dom.get_by_ref(i.parent()).is_some_and(|p|p.name=="rustudio_tc_test")).map(|i|i.referent());
                if let Some(reference)=reference {
                    self.open_script.push(reference);
                    self.doc.set_props("Test local script draft",vec![(reference,"Source".into(),rbx_dom_weak::types::Variant::String("-- rustudio private draft UI probe\nreturn 6\n".into()))],None);
                } else {self.error("The marked live script UI probe is unavailable");}
            }

            (0, Show::Props(which)) => {
                let target = self.ui_shots_reference(which);
                self.ui.panel_requests.push((crate::app::Tab::Explorer, Some(false)));
                self.select(target.into_iter().collect());
            }
            (0, Show::Editor(_)) => {
                let target = self.ui_shots_reference("Part");
                self.ui.panel_requests.push((crate::app::Tab::Explorer, Some(false)));
                self.select(target.into_iter().collect());
            }
            (3, Show::Editor(which)) => match *which {
                "brickcolor" => {
                    let v = self.ui_rects.get("props.brickcolor").copied().unwrap_or(egui::Rect::NOTHING);
                    self.props.brick_popup = Some(crate::color_editors::BrickPopup {
                        prop: "BrickColor".into(),
                        origin: v.left_bottom() + egui::vec2(0.0, 1.0),
                        opened_frame: self.frame_no,
                        hovered: None,
                    });
                }
                "color" => {
                    let rgb = match self.doc.selection.first().and_then(|r| self.doc.prop(*r, "Color")) {
                        Some(rbx_dom_weak::types::Variant::Color3uint8(c)) => [c.r, c.g, c.b],
                        _ => [163, 162, 165],
                    };
                    self.open_color_dialog("Color", rgb);
                }
                "material-enum" => egui::Popup::open_id(ctx, egui::Id::new(("enum_popup", "Material"))),
                "ribbon-material" | "ribbon-color" => {
                    let k = if *which == "ribbon-material" { "home.material" } else { "home.color" };
                    if let Some(p) = click_at(k, self) {
                        s.queue.push(Event::PointerMoved(p));
                        s.queue.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                    }
                }
                _ => self.props.active_row = Some("Transparency".into()),
            },
            (4, Show::Editor(which)) if which.starts_with("ribbon-") => {
                let k = if *which == "ribbon-material" { "home.material" } else { "home.color" };
                if let Some(p) = click_at(k, self) {
                    s.queue.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
                    s.queue.push(Event::PointerMoved(Pos2::new(p.x, p.y - 30.0)));
                }
            }
            (0, Show::OutputRun) => {
                let root = self.doc.dom.root_ref();
                if let Some(sss) = self.doc.find_child(root, "ServerScriptService") {
                    self.doc.insert(
                        sss,
                        rbx_dom_weak::InstanceBuilder::new("Script")
                            .with_name("RefScript")
                            .with_property("Source", "print(\"hello from a script\")\nwarn(\"careful\")\nerror(\"boom\")\n".to_string()),
                    );
                }
                self.log.clear();
                self.output_view.show_timestamp = true;
                self.start_run(crate::studio_ui::RunType::Run);
                self.log.retain(|l| l.text.contains("RefScript") || l.text.contains('\u{2063}'));
            }
            (5, Show::OutputRun) => self.log.retain(|l| l.text.contains("RefScript") || l.text.contains('\u{2063}')),
            (0, Show::Tab(t)) => self.ui.tab = *t,
            (0, Show::Terrain(tool)) => {
                self.ui.panel_requests.push((crate::app::Tab::TerrainEditor, Some(true)));
                self.terrain_ed.tool = Some(*tool);
                // Region tools (round 25): a box on some hills.
                if tool.is_region() && self.terrain_ed.selection.is_none() {
                    self.terrain_ed.region_pos = [0.0, 16.0, 0.0];
                    self.terrain_ed.region_size = [128.0, 32.0, 128.0];
                    self.terrain_action("generate");
                    self.terrain_ed.selection = Some(crate::terrain_editor::RegionBox::from_studs([0.0, 16.0, 0.0], [40.0, 32.0, 32.0]));
                    self.cam.mode = crate::camera::CameraMode::Fly;
                    self.cam.basis = None;
                    self.cam.pos = glam::Vec3::new(50.0, 75.0, 90.0);
                    self.cam.focus = glam::Vec3::ZERO;
                    self.cam.yaw = 0.5;
                    self.cam.pitch = -0.55;
                }
            }
            (0, Show::Click(k)) => {
                if let Some(p) = click_at(k, self) {
                    s.queue.push(Event::PointerMoved(p));
                    s.queue.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                }
            }
            (1, Show::Click(k)) => {
                if let Some(p) = click_at(k, self) {
                    s.queue.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
                    // Off the item, so nothing shows as hovered.
                    s.queue.push(Event::PointerMoved(Pos2::new(p.x + 2.0, p.y)));
                }
            }
            (8, show) if !s.asked && !matches!(show, Show::OutputRun) => {
                s.asked = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            (30, Show::OutputRun) if !s.asked => {
                s.asked = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            _ => {}
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot.filter(|_| s.asked) {
            let _ = crate::app::save_png(&image, &s.dir.join(format!("{name}.png")));
            let ppp = ctx.pixels_per_point();
            let rect_json = |r: egui::Rect| serde_json::json!([r.min.x * ppp, r.min.y * ppp, r.max.x * ppp, r.max.y * ppp]);
            let mut info = serde_json::Map::new();
            if let Show::Click(k) = show {
                if let Some(r) = self.ui_rects.get(*k) {
                    info.insert("anchor".into(), rect_json(*r));
                }
            }
            for k in ["test.type", "account", "explorer.search", "props.panel", "output.panel", "brick.popup", "color.dialog", "props.slider", "props.spin"] {
                if let Some(r) = self.ui_rects.get(k) {
                    info.insert(k.into(), rect_json(*r));
                }
            }
            info.insert("window".into(), serde_json::json!([image.size[0], image.size[1]]));
            s.rects.insert(name, serde_json::Value::Object(info));
            // Close whatever opened, next picture.
            s.queue.push(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            s.queue.push(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
            if matches!(show,Show::LiveDraft) {
                let reference=self.doc.dom.descendants().find(|i|i.class.as_str()=="ModuleScript" && i.name=="rustudio_script_probe" && self.doc.dom.get_by_ref(i.parent()).is_some_and(|p|p.name=="rustudio_tc_test")).map(|i|i.referent());
                if let Some(reference)=reference {self.discard_live_draft(reference);self.doc.mark_saved_at(self.doc.revision);}
            }
            s.i += 1;
            s.t = 0;
            s.asked = false;
        } else {
            s.t += 1;
        }
        self.harness.ui_shots = Some(s);
    }
}
