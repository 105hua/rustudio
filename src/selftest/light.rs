//! Self-test steps for lighting: builds Studio's calibration scenes, reads
//! the viewport back and compares lit colours with the ones Studio showed
//! (offline Studio, Future, an 863×675 view from (4, 9, 22) toward
//! (4, 2, 0), FOV 70; numbers and sources in docs/MECHANICS.md "Lighting").

use glam::Vec3;
use rbx_dom_weak::types::{Color3, Enum, Ref, Variant};
use rbx_dom_weak::InstanceBuilder;

use super::{R6Test, SelfTest, Step};
use crate::app::Editor;
use crate::doc::{new_part, Document};

/// One scene: what to build and Studio's colours at world points.
struct LightScene {
    step: &'static str,
    what: &'static str,
    /// Lighting.Brightness and ClockTime; LightingStyle Soft.
    brightness: f32,
    clock: f32,
    soft: bool,
    /// Cube materials and colours (White, Grey, Red).
    materials: [u32; 3],
    colors: [[u8; 3]; 3],
    /// The night scene's PointLight and SpotLight.
    lamps: bool,
    /// The Soft scene: a ball and a point light, no ambient.
    soft_ball: bool,
    expect: &'static [(&'static str, [f32; 3], [u8; 3])],
}

const SCENES: [LightScene; 4] = [
    LightScene {
        step: "lighting-day",
        what: "sun at 14:00, Brightness 2, Ambient 70, OutdoorAmbient 128, shadows (Studio: studio_future_day)",
        brightness: 2.0,
        clock: 14.0,
        soft: false,
        materials: [272, 272, 272],
        colors: [[255, 255, 255], [128, 128, 128], [255, 0, 0]],
        lamps: false,
        soft_ball: false,
        expect: &[
            ("white top", [0.0, 4.0, 0.0], [227, 227, 227]),
            ("white front", [0.0, 2.0, 2.0], [192, 192, 192]),
            ("white side", [2.0, 2.0, -0.5], [129, 129, 129]),
            ("grey top", [-8.0, 4.0, 0.0], [138, 138, 138]),
            ("grey front", [-8.0, 2.0, 2.0], [91, 91, 91]),
            ("red top", [8.0, 4.0, 0.0], [227, 1, 1]),
            ("red front", [8.0, 2.0, 2.0], [192, 1, 1]),
            ("neon", [16.0, 2.0, 2.0], [0, 255, 0]),
            ("baseplate lit", [-3.0, 0.0, 8.0], [101, 96, 100]),
            ("baseplate in shadow", [3.2, 0.0, -1.2], [29, 27, 29]),
        ],
    },
    LightScene {
        step: "lighting-night",
        what: "midnight, a warm PointLight (Brightness 2, Range 16) and a blue SpotLight (Brightness 4, Range 24, Angle 60) (Studio: studio_future_night)",
        brightness: 0.0,
        clock: 0.0,
        soft: false,
        materials: [272, 272, 272],
        colors: [[255, 255, 255], [128, 128, 128], [255, 0, 0]],
        lamps: true,
        soft_ball: false,
        expect: &[
            ("white top", [0.0, 4.0, 0.0], [224, 204, 166]),
            ("white front", [0.0, 2.0, 2.0], [244, 231, 191]),
            ("white side", [2.0, 2.0, -0.5], [130, 129, 129]),
            ("grey top", [-8.0, 4.0, 0.0], [92, 75, 57]),
            ("grey front", [-8.0, 2.0, 2.0], [146, 116, 74]),
            ("red top", [8.0, 4.0, 0.0], [193, 0, 0]),
            ("red front", [8.0, 2.0, 2.0], [231, 2, 1]),
            ("baseplate lit", [-3.0, 0.0, 8.0], [150, 109, 64]),
            ("baseplate by the cube", [3.2, 0.0, -1.2], [65, 48, 36]),
        ],
    },
    LightScene {
        step: "lighting-materials",
        what: "white Snow, Slate and Wood cubes in the day scene (Studio: studio_mats)",
        brightness: 2.0,
        clock: 14.0,
        soft: false,
        materials: [1328, 800, 512],
        colors: [[255, 255, 255], [255, 255, 255], [255, 255, 255]],
        lamps: false,
        soft_ball: false,
        expect: &[
            ("snow top", [0.0, 4.0, 0.0], [212, 212, 212]),
            ("snow front", [0.0, 2.0, 2.0], [169, 169, 169]),
            ("slate top", [-8.0, 4.0, 0.0], [197, 197, 197]),
            ("slate front", [-8.0, 2.0, 2.0], [151, 151, 151]),
            ("wood top", [8.0, 4.0, 0.0], [216, 216, 216]),
            ("wood front", [8.0, 2.0, 2.0], [175, 175, 175]),
        ],
    },
    LightScene {
        step: "lighting-soft",
        what: "LightingStyle Soft: a white ball lit by a PointLight (Brightness 1, Range 60) 18 studs to its left is lit evenly all round (Studio: ball_soft, 224 at every angle)",
        brightness: 0.0,
        clock: 14.0,
        soft: true,
        materials: [272, 272, 272],
        colors: [[255, 255, 255], [128, 128, 128], [255, 0, 0]],
        lamps: false,
        soft_ball: true,
        expect: &[
            ("ball facing the light", [0.0, 5.0, 0.0], [224, 224, 224]),
            ("ball facing the camera", [4.0, 5.0, 2.0], [224, 224, 224]),
            ("ball facing away", [8.0, 5.0, 0.0], [224, 224, 224]),
        ],
    },
];

fn set(ed: &mut Editor, r: Ref, name: &str, v: Variant) {
    if let Some(i) = ed.doc.dom.get_by_ref_mut(r) {
        i.properties.insert(rbx_dom_weak::ustr(name), v);
    }
}

fn build(ed: &mut Editor, scene: &LightScene) {
    ed.set_document(Document::new_place());
    ed.show_grid = false;
    let root = ed.doc.dom.root_ref();
    let ws = ed.doc.workspace().expect("workspace");
    // Only our parts: take out what a new place starts with.
    let start: Vec<Ref> = ed
        .doc
        .dom
        .get_by_ref(ws)
        .map(|i| i.children().to_vec())
        .unwrap_or_default();
    let parts: Vec<Ref> = start
        .into_iter()
        .filter(|c| {
            ed.doc
                .class_of(*c)
                .is_some_and(|k| crate::doc::is_a(&k, "BasePart"))
        })
        .collect();
    ed.doc.remove(&parts);
    let lighting = match ed.doc.find_child(root, "Lighting") {
        Some(l) => l,
        None => ed
            .doc
            .insert(root, InstanceBuilder::new("Lighting").with_name("Lighting")),
    };
    let effects: Vec<Ref> = ed
        .doc
        .dom
        .get_by_ref(lighting)
        .map(|i| i.children().to_vec())
        .unwrap_or_default();
    ed.doc.remove(&effects);
    let grey = |v: f32| Variant::Color3(Color3::new(v, v, v));
    let ambient = if scene.soft_ball { 0.0 } else { 70.0 / 255.0 };
    let outdoor = if scene.soft_ball { 0.0 } else { 128.0 / 255.0 };
    for (name, v) in [
        ("Brightness", Variant::Float32(scene.brightness)),
        ("ClockTime", Variant::Float32(scene.clock)),
        ("GeographicLatitude", Variant::Float32(41.733)),
        ("Ambient", grey(ambient)),
        ("OutdoorAmbient", grey(outdoor)),
        ("EnvironmentDiffuseScale", Variant::Float32(0.0)),
        ("EnvironmentSpecularScale", Variant::Float32(0.0)),
        ("ExposureCompensation", Variant::Float32(0.0)),
        ("GlobalShadows", Variant::Bool(true)),
        ("ColorShift_Top", grey(0.0)),
        ("FogEnd", Variant::Float32(100_000.0)),
        (
            "LightingStyle",
            Variant::Enum(Enum::from_u32(scene.soft as u32)),
        ),
    ] {
        set(ed, lighting, name, v);
    }
    let smooth = || Enum::from_u32(272);
    ed.doc.insert(
        ws,
        new_part(
            "Baseplate",
            1,
            Vec3::new(0.0, -10.0, 0.0),
            Vec3::new(512.0, 20.0, 512.0),
            [99, 95, 98],
        )
        .with_property("Material", smooth()),
    );
    if scene.soft_ball {
        ed.doc.insert(
            ws,
            new_part(
                "Ball",
                0,
                Vec3::new(4.0, 5.0, -3.0),
                Vec3::splat(10.0),
                [255, 255, 255],
            )
            .with_property("Material", smooth()),
        );
        ed.doc.insert(
            ws,
            new_part(
                "PL",
                1,
                Vec3::new(-14.0, 5.0, -3.0),
                Vec3::splat(0.2),
                [255, 255, 255],
            )
            .with_property("Transparency", 1.0f32)
            .with_child(
                InstanceBuilder::new("PointLight")
                    .with_property("Brightness", 1.0f32)
                    .with_property("Range", 60.0f32)
                    .with_property("Shadows", false),
            ),
        );
    } else {
        for (i, (name, x)) in [("White", 0.0), ("Grey", -8.0), ("Red", 8.0)]
            .into_iter()
            .enumerate()
        {
            ed.doc.insert(
                ws,
                new_part(
                    name,
                    1,
                    Vec3::new(x, 2.0, 0.0),
                    Vec3::splat(4.0),
                    scene.colors[i],
                )
                .with_property("Material", Enum::from_u32(scene.materials[i])),
            );
        }
        ed.doc.insert(
            ws,
            new_part(
                "Neon",
                1,
                Vec3::new(16.0, 2.0, 0.0),
                Vec3::splat(4.0),
                [0, 255, 0],
            )
            .with_property("Material", Enum::from_u32(288)),
        );
    }
    if scene.lamps {
        ed.doc.insert(
            ws,
            new_part(
                "Lamp",
                1,
                Vec3::new(0.0, 8.0, 10.0),
                Vec3::ONE,
                [255, 255, 255],
            )
            .with_property("Material", smooth())
            .with_child(
                InstanceBuilder::new("PointLight")
                    .with_property("Brightness", 2.0f32)
                    .with_property("Color", Color3::new(1.0, 200.0 / 255.0, 120.0 / 255.0))
                    .with_property("Range", 16.0f32),
            ),
        );
        ed.doc.insert(
            ws,
            new_part(
                "Spot",
                1,
                Vec3::new(-14.0, 14.0, 10.0),
                Vec3::ONE,
                [255, 255, 255],
            )
            .with_property("Material", smooth())
            .with_child(
                InstanceBuilder::new("SpotLight")
                    .with_property("Brightness", 4.0f32)
                    .with_property("Color", Color3::new(120.0 / 255.0, 180.0 / 255.0, 1.0))
                    .with_property("Range", 24.0f32)
                    .with_property("Angle", 60.0f32)
                    .with_property("Face", Enum::from_u32(4)),
            ),
        );
    }
    // Studio's calibration view: from (4, 9, 22) toward (4, 2, 0), FOV 70,
    // rendered at Studio's 863×675 (glow and anything else that follows the
    // view's size come out as in its captures).
    ed.view_render_size = Some(STUDIO_VIEW);
    ed.cam.pos = Vec3::new(4.0, 9.0, 22.0);
    ed.cam.yaw = 0.0;
    ed.cam.pitch = (-7.0f32).atan2(22.0);
    ed.cam.fov_deg = 70.0;
}

/// The view size of Studio's captures.
const STUDIO_VIEW: [u32; 2] = [863, 675];

/// Mean colour of a 5×5 patch around a world point in the last frame.
fn sample(ed: &Editor, frame: &(u32, u32, Vec<u8>), p: Vec3) -> Option<[u8; 3]> {
    let (w, h, px) = frame;
    // (rendered at view_render_size pixels: project onto that)
    let rect = match ed.view_render_size {
        Some(s) => egui::Rect::from_min_size(ed.viewport_rect.min, egui::vec2(s[0] as f32, s[1] as f32)),
        None => ed.viewport_rect,
    };
    let scale = *w as f32 / rect.width().max(1.0);
    let sp = ed.cam.project(rect, p)?;
    let (cx, cy) = (
        ((sp.x - rect.min.x) * scale) as i64,
        ((sp.y - rect.min.y) * scale) as i64,
    );
    // Studio's patch was 5×5 pixels of an 863×675 view: the same world area
    // here, whatever height the viewport has (round 16's chrome changed it).
    let half = ((rect.height() * scale) / 675.0 * 2.0).round().max(1.0) as i64;
    let mut acc = [0u32; 3];
    let mut n = 0u32;
    for dy in -half..=half {
        for dx in -half..=half {
            let (x, y) = (cx + dx, cy + dy);
            if x < 0 || y < 0 || x >= *w as i64 || y >= *h as i64 {
                continue;
            }
            let o = ((y as u32 * w + x as u32) * 4) as usize;
            for (c, a) in acc.iter_mut().enumerate() {
                *a += px[o + c] as u32;
            }
            n += 1;
        }
    }
    (n > 0).then(|| acc.map(|a| (a as f32 / n as f32).round() as u8))
}

/// Sets out-of-range values the way a script would and prints what stuck.
const SETTERS_SRC: &str = r#"local L = game:GetService("Lighting")
local s = Instance.new("SpotLight")
s.Parent = workspace:WaitForChild("Lamp")
L.ClockTime = 25.5
local a = L.ClockTime
L.ClockTime = 7.7505
local b = L.ClockTime
L.EnvironmentDiffuseScale = 3
L.ShadowSoftness = -1
L.Brightness = 50
s.Range = 200
s.Angle = 270
print(string.format("setters %.4f %.4f %.2f %.2f %.2f %.1f %.1f", a, b, L.EnvironmentDiffuseScale, L.ShadowSoftness, L.Brightness, s.Range, s.Angle))"#;

impl SelfTest {
    /// Studio's property setters (read from its code, mech::setter_clamp):
    /// a script's out-of-range values are clamped the same way.
    fn tick_setters(&mut self, f: u64, ed: &mut Editor) -> bool {
        if self.r4.phase % 2 == 0 {
            ed.set_document(Document::new_place());
            let root = ed.doc.dom.root_ref();
            let ws = ed.doc.workspace().expect("workspace");
            ed.doc.insert(
                ws,
                new_part(
                    "Lamp",
                    1,
                    Vec3::new(0.0, 8.0, 0.0),
                    Vec3::ONE,
                    [255, 255, 255],
                ),
            );
            let sss = ed.doc.find_child(root, "ServerScriptService").expect("sss");
            ed.doc.insert(
                sss,
                InstanceBuilder::new("Script")
                    .with_name("Setters")
                    .with_property("Source", SETTERS_SRC.to_string()),
            );
            self.r6 = R6Test {
                started: f,
                mark: ed.log.len(),
                ..Default::default()
            };
            ed.start_play(crate::runtime::RunMode::Run);
            self.r4.phase += 1;
            return false;
        }
        if f.saturating_sub(self.r6.started) < 20 {
            return false;
        }
        let line = ed.log[self.r6.mark.min(ed.log.len())..]
            .iter()
            .find(|l| l.text.contains("setters "))
            .map(|l| l.text.clone());
        ed.stop_play();
        // 25.5 h → 24; 7.7505 h → 27901.8 s → 27901 s; diffuse 3 → 1;
        // softness −1 → 0; Brightness 50 kept; Range 200 → 120; Angle 270 → 180.
        let want = "setters 24.0000 7.7503 1.00 0.00 50.00 120.0 180.0";
        let ok = line.as_deref().is_some_and(|l| l.contains(want));
        self.steps.push(Step {
            step: "lighting-setters",
            passed: ok,
            detail: format!(
                "a script set ClockTime 25.5 and 7.7505, EnvironmentDiffuseScale 3, ShadowSoftness −1, Brightness 50, SpotLight Range 200 and Angle 270; Studio's setters give \"{want}\"; got {:?}",
                line.unwrap_or_else(|| "no output".into())
            ),
        });
        self.r4.phase += 1;
        false
    }

    /// Phases 40.. : even = build a scene, odd = wait, read back and compare.
    pub(super) fn tick_light(&mut self, f: u64, ed: &mut Editor) -> bool {
        let index = (self.r4.phase - 40) as usize / 2;
        if index == SCENES.len() {
            ed.view_render_size = None;
            return self.tick_setters(f, ed);
        }
        let Some(scene) = SCENES.get(index) else {
            ed.view_render_size = None;
            ed.show_grid = true;
            crate::matpack::set_enabled(true);
            self.r4.phase = 60;
            return false;
        };
        if self.r4.phase % 2 == 0 {
            // Studio's references were taken offline (no material packs).
            crate::matpack::set_enabled(false);
            build(ed, scene);
            self.r6 = R6Test {
                started: f,
                t0: Some(std::time::Instant::now()),
                ..Default::default()
            };
            self.r4.phase += 1;
            return false;
        }
        // The light grid waits LIGHTGRID_REFRESH_S of real time between
        // rebuilds; these frames run faster than that, so rebuild it now
        // that the scene is in, before reading the view.
        if f.saturating_sub(self.r6.started) == 20 {
            ed.lighting.refresh_lightgrid_now();
        }
        if f.saturating_sub(self.r6.started) < 30 {
            return false;
        }
        // Parts that reached the scene after that rebuild leave the grid stale
        // (it's rebuilt on a real-time timer): rebuild and read once it's
        // current, or the outdoor light read depends on timing.
        if ed.lighting.lightgrid_pending() && f.saturating_sub(self.r6.started) < 600 {
            ed.lighting.refresh_lightgrid_now();
            return false;
        }
        // The skybox loads in the background: until it is in, the previous
        // scene's sky light is used (a busy machine would read too early).
        if ed.lighting.sky_pending() && f.saturating_sub(self.r6.started) < 1200 {
            if f.saturating_sub(self.r6.started) % 10 == 0 {
                ed.lighting.refresh_lightgrid_now();
            }
            return false;
        }
        let frame = ed
            .renderer
            .as_ref()
            .zip(ed.render_state.as_ref())
            .and_then(|(r, rs)| r.read_frame(rs));
        // RUSTUDIO_LIGHT_DUMP=DIR: the frame each scene was read from (debugging).
        if let (Some(dir), Some((w, h, px))) = (std::env::var_os("RUSTUDIO_LIGHT_DUMP"), &frame) {
            let _ = image::save_buffer(
                std::path::Path::new(&dir).join(format!("{}.png", scene.step)),
                px,
                *w,
                *h,
                image::ExtendedColorType::Rgba8,
            );
        }
        let mut worst = 0u8;
        let mut notes = Vec::new();
        // The calibration view must not have moved (a key held from an
        // earlier step would fly it now that the fly keys work anywhere).
        let drift = (ed.cam.pos - Vec3::new(4.0, 9.0, 22.0)).length();
        if drift > 0.01 {
            notes.push(format!("camera moved {drift:.3} studs from the calibration view"));
        }
        if std::env::var_os("RUSTUDIO_LIGHT_DUMP").is_some() {
            notes.push(format!("view rect {:?}", ed.viewport_rect));
        }
        match &frame {
            Some(frame) => {
                for (name, p, want) in scene.expect {
                    match sample(ed, frame, Vec3::from(*p)) {
                        Some(got) => {
                            let diff = (0..3)
                                .map(|c| (got[c] as i32 - want[c] as i32).unsigned_abs())
                                .max()
                                .unwrap_or(255)
                                .min(255) as u8;
                            worst = worst.max(diff);
                            notes.push(format!("{name} {got:?} vs Studio {want:?}"));
                        }
                        None => {
                            worst = 255;
                            notes.push(format!("{name}: off screen"));
                        }
                    }
                }
            }
            None => {
                worst = 255;
                notes.push("couldn't read the viewport back".into());
            }
        }
        self.steps.push(Step {
            step: scene.step,
            passed: worst <= crate::mech::LIGHT_TEST_TOLERANCE,
            detail: format!(
                "{}: {}; largest difference {worst} levels (allowed {})",
                scene.what,
                notes.join(", "),
                crate::mech::LIGHT_TEST_TOLERANCE
            ),
        });
        self.r4.phase += 1;
        false
    }
}
