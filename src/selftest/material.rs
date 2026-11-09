//! Self-test step for material textures: legacy surfaces drawn from
//! Studio's studs.dds, Studio's default terrain colours, and a
//! MaterialVariant's ColorMap. Reference values were read from an offline Studio
//! Studio at the same view (docs/MECHANICS.md, "Materials").

use glam::Vec3;
use rbx_dom_weak::types::{ContentId, Enum};
use rbx_dom_weak::InstanceBuilder;

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::{new_part, Document};

/// (name, SurfaceType on every face, x): the surface row, front faces at z = 2.
const SURFACES: [(&str, u32, f32); 4] = [
    ("Studs", 3, -12.0),
    ("Inlet", 4, -6.0),
    ("Weld", 2, 0.0),
    ("Universal", 5, 6.0),
];
/// Studio's mean level and contrast (standard deviation) of a 15 × 15 px
/// patch at each front face's centre (offline Studio, Future, 607 × 522 view from
/// (−3, 9, 18) toward (−3, 2, −5), FOV 70).
const STUDIO: [(f32, f32); 4] = [(125.0, 16.8), (122.0, 18.9), (104.0, 34.1), (123.0, 18.8)];

/// Mean grey level and its standard deviation over a 15 × 15 patch.
fn patch(ed: &Editor, frame: &(u32, u32, Vec<u8>), p: Vec3) -> Option<(f32, f32)> {
    let (w, h, px) = frame;
    let rect = ed.viewport_rect;
    let scale = *w as f32 / rect.width().max(1.0);
    let sp = ed.cam.project(rect, p)?;
    let (cx, cy) = (
        ((sp.x - rect.min.x) * scale) as i64,
        ((sp.y - rect.min.y) * scale) as i64,
    );
    let mut vals = Vec::new();
    for dy in -7..=7 {
        for dx in -7..=7 {
            let (x, y) = (cx + dx, cy + dy);
            if x < 0 || y < 0 || x >= *w as i64 || y >= *h as i64 {
                continue;
            }
            let o = ((y as u32 * w + x as u32) * 4) as usize;
            vals.push((px[o] as f32 + px[o + 1] as f32 + px[o + 2] as f32) / 3.0);
        }
    }
    if vals.is_empty() {
        return None;
    }
    let mean = vals.iter().sum::<f32>() / vals.len() as f32;
    let sd = (vals.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / vals.len() as f32).sqrt();
    Some((mean, sd))
}

impl SelfTest {
    /// Phases 70..=71.
    pub(super) fn tick_material(&mut self, f: u64, ed: &mut Editor) -> bool {
        if self.r4.phase == 70 {
            let mut doc = Document::new_place();
            let root = doc.dom.root_ref();
            let ws = doc.workspace().expect("workspace");
            if let Some(l) = doc.find_child(root, "Lighting") {
                if let Some(i) = doc.dom.get_by_ref_mut(l) {
                    use rbx_dom_weak::types::Color3;
                    use rbx_dom_weak::ustr;
                    i.properties
                        .insert(ustr("Technology"), Enum::from_u32(4).into());
                    i.properties.insert(ustr("Brightness"), 2.0f32.into());
                    i.properties.insert(ustr("ClockTime"), 14.0f32.into());
                    i.properties.insert(
                        ustr("Ambient"),
                        Color3::new(70.0 / 255.0, 70.0 / 255.0, 70.0 / 255.0).into(),
                    );
                    i.properties.insert(
                        ustr("OutdoorAmbient"),
                        Color3::new(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0).into(),
                    );
                    i.properties
                        .insert(ustr("EnvironmentDiffuseScale"), 0.0f32.into());
                    i.properties
                        .insert(ustr("EnvironmentSpecularScale"), 0.0f32.into());
                }
            }
            for (name, st, x) in SURFACES {
                let mut b = new_part(
                    name,
                    1,
                    Vec3::new(x, 2.0, 0.0),
                    Vec3::splat(4.0),
                    [163, 162, 165],
                );
                for face in [
                    "TopSurface",
                    "BottomSurface",
                    "LeftSurface",
                    "RightSurface",
                    "FrontSurface",
                    "BackSurface",
                ] {
                    b = b.with_property(face, Enum::from_u32(st));
                }
                doc.insert(ws, b);
            }
            // A MaterialVariant on Brick (the content-folder test's PNG as its ColorMap).
            let ms = doc.find_child(root, "MaterialService").unwrap_or_else(|| {
                doc.dom.insert(
                    root,
                    InstanceBuilder::new("MaterialService").with_name("MaterialService"),
                )
            });
            doc.insert(
                ms,
                InstanceBuilder::new("MaterialVariant")
                    .with_name("TestBrick")
                    .with_property("BaseMaterial", Enum::from_u32(848))
                    .with_property(
                        "ColorMap",
                        ContentId::from("rbxasset://textures/selftest.png"),
                    )
                    .with_property("StudsPerTile", 2.0f32),
            );
            doc.insert(
                ws,
                new_part(
                    "Variant",
                    1,
                    Vec3::new(14.0, 2.0, -10.0),
                    Vec3::splat(4.0),
                    [255, 255, 255],
                )
                .with_property("Material", Enum::from_u32(848))
                .with_property("MaterialVariant", "TestBrick".to_string()),
            );
            ed.set_document(doc);
            ed.show_grid = false;
            ed.cam.pos = Vec3::new(-3.0, 9.0, 18.0);
            ed.cam.yaw = 0.0;
            ed.cam.pitch = (-7.0f32).atan2(23.0);
            ed.cam.fov_deg = 70.0;
            self.r6 = super::R6Test {
                started: f,
                ..Default::default()
            };
            self.r4.phase = 71;
            return false;
        }
        if f.saturating_sub(self.r6.started) < 40 {
            return false;
        }
        let mut ok = true;
        // Pin the shipped GlassTransparentFS.8 and PlasticReflectionFS.3
        // constants at the shader consumed by the actual GPU pipeline.
        for reflectance in [0.0, 0.5, 1.0] {
            for material in [256, 272, 1568] {
                ok &= crate::render::material_reflectance(material, reflectance) == reflectance;
            }
            ok &= crate::render::material_reflectance(1088, reflectance) == 0.0;
        }
        let shader = include_str!("../shader.wgsl");
        for law in [
            "mix(0.66, 1.0, (1.0 - alpha) * (1.0 - alpha))",
            "normal * (3.0 * (1.0 - eta))",
            "clamp(2.0 * alpha - 1.0 + a.a, 0.0, 1.0)",
            "0.8 * (1.0 - nv) * (1.0 - nv)",
            "dot(scene.rgb, vec3<f32>(1.0)) + 0.01",
            "env_reflect(reflect(-v, n), rough, 1.0, true)",
            "env_reflect(reflect(-v, n), rough, sv, reflective)",
        ] {
            ok &= shader.contains(law);
        }

        let mut notes = Vec::new();
        let installed = crate::default_scripts::studio_dir().is_some();
        let frame = ed
            .renderer
            .as_ref()
            .zip(ed.render_state.as_ref())
            .and_then(|(r, rs)| r.read_frame(rs));
        match (&frame, installed) {
            (Some(frame), true) => {
                for ((name, _, x), (want_mean, want_sd)) in SURFACES.iter().zip(STUDIO) {
                    match patch(ed, frame, Vec3::new(*x, 2.0, 2.0)) {
                        Some((mean, sd)) => {
                            // Same brightness as Studio's, and textured (a smooth face has no contrast).
                            let good = (mean - want_mean).abs() <= 12.0 && sd > 8.0;
                            ok &= good;
                            notes.push(format!("{name} face: level {mean:.0} (Studio {want_mean:.0}), contrast {sd:.1} (Studio {want_sd:.1})"));
                        }
                        None => {
                            ok = false;
                            notes.push(format!("{name}: off screen"));
                        }
                    }
                }
            }
            (None, _) => {
                ok = false;
                notes.push("couldn't read the viewport back".into());
            }
            (_, false) => notes.push("no Studio install: surfaces drawn smooth (skipped)".into()),
        }
        // Studio's default terrain colours (its materials.json; GetMaterialColor in an offline Studio).
        let grass =
            crate::terrain::material_color(crate::terrain::material_index("Grass").unwrap_or(2));
        let rock =
            crate::terrain::material_color(crate::terrain::material_index("Rock").unwrap_or(8));
        let terrain_ok = !installed || (grass == [106, 127, 63] && rock == [102, 108, 111]);
        ok &= terrain_ok;
        notes.push(format!("terrain Grass {grass:?}, Rock {rock:?} (Studio's GetMaterialColor: [106, 127, 63], [102, 108, 111])"));
        // The MaterialVariant's ColorMap covers the part (six tiled quads).
        let variant = ed
            .doc
            .workspace()
            .and_then(|ws| crate::default_scripts::child_named(&ed.doc, ws, "Variant"));
        let quads = variant
            .and_then(|v| ed.scene.parts.iter().find(|p| p.r == v))
            .map(|p| {
                p.decals
                    .iter()
                    .filter(|d| d.tiling == Some([2.0, 2.0, 0.0, 0.0]))
                    .count()
            })
            .unwrap_or(0);
        ok &= quads == 6;
        notes.push(format!("MaterialVariant TestBrick (ColorMap, StudsPerTile 2) on a Brick part: {quads} tiled faces (want 6)"));
        ed.show_grid = true;
        self.steps.push(Step {
            step: "material-textures",
            passed: ok,
            detail: notes.join("; "),
        });
        self.r4.phase = 250;
        false
    }
}
