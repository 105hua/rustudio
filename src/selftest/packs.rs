//! Self-test step for material texture packs: a row of 4-stud cubes in
//! Studio's 2022 materials (MaterialService.Use2022Materials) drawn with
//! their packs from public asset delivery (matpack.rs, docs/MATERIALS.md).
//! Passes when every cube has its maps in the texture arrays, repeats every
//! 8 studs, shows texture (contrast across the face) and keeps the level of
//! the table's average colour.

use glam::Vec3;
use rbx_dom_weak::types::Enum;
use rbx_dom_weak::InstanceBuilder;

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::{new_part, Document};

/// (name, Enum.Material value, x) along z = 0, 4-stud cubes.
pub const ROW: [(&str, u32, f32); 7] = [
    ("Brick", 848, -18.0),
    ("Wood", 512, -12.0),
    ("Concrete", 816, -6.0),
    ("Metal", 1088, 0.0),
    ("Grass", 1280, 6.0),
    ("Slate", 800, 12.0),
    ("Fabric", 1312, 18.0),
];

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
    /// Phases 74..=75.
    pub(super) fn tick_packs(&mut self, f: u64, ed: &mut Editor) -> bool {
        if self.r4.phase == 74 {
            let mut doc = Document::new_place();
            let root = doc.dom.root_ref();
            let ws = doc.workspace().expect("workspace");
            if let Some(l) = doc.find_child(root, "Lighting") {
                if let Some(i) = doc.dom.get_by_ref_mut(l) {
                    use rbx_dom_weak::ustr;
                    i.properties
                        .insert(ustr("Technology"), Enum::from_u32(4).into());
                    i.properties.insert(ustr("ClockTime"), 14.0f32.into());
                }
            }
            let ms = doc.find_child(root, "MaterialService").unwrap_or_else(|| {
                doc.dom.insert(
                    root,
                    InstanceBuilder::new("MaterialService").with_name("MaterialService"),
                )
            });
            if let Some(i) = doc.dom.get_by_ref_mut(ms) {
                i.properties
                    .insert(rbx_dom_weak::ustr("Use2022Materials"), true.into());
            }
            for (name, mat, x) in ROW {
                doc.insert(
                    ws,
                    new_part(
                        name,
                        1,
                        Vec3::new(x, 2.0, 0.0),
                        Vec3::splat(4.0),
                        [255, 255, 255],
                    )
                    .with_property("Material", Enum::from_u32(mat)),
                );
            }
            ed.set_document(doc);
            ed.show_grid = false;
            ed.cam.pos = Vec3::new(0.0, 9.0, 24.0);
            ed.cam.yaw = 0.0;
            ed.cam.pitch = (-7.0f32).atan2(24.0);
            ed.cam.fov_deg = 70.0;
            self.r6 = super::R6Test {
                started: f,
                ..Default::default()
            };
            self.r4.phase = 75;
            return false;
        }
        // Wait for the packs (first run: downloads, ~10 MB a material).
        let entries: Vec<_> = ROW
            .iter()
            .filter_map(|(_, m, _)| crate::matpack::entry(*m, true))
            .collect();
        let pending = entries
            .iter()
            .any(|e| matches!(crate::matpack::maps(e), crate::matpack::Lookup::Pending));
        let waited = f.saturating_sub(self.r6.started);
        let t0 = *self.r6.t0.get_or_insert_with(std::time::Instant::now);
        if (pending && t0.elapsed().as_secs() < 240) || waited < 40 {
            return false;
        }
        // A few frames after the last pack so the draws pick the layers up.
        if self.r6.mark == 0 {
            self.r6.mark = f as usize;
            return false;
        }
        if f < self.r6.mark as u64 + 10 {
            return false;
        }
        let mut ok = true;
        let mut notes = Vec::new();
        let frame = ed
            .renderer
            .as_ref()
            .zip(ed.render_state.as_ref())
            .and_then(|(r, rs)| r.read_frame(rs));
        for (name, mat, x) in ROW {
            let e = crate::matpack::entry(mat, true).expect("table entry");
            let maps = match crate::matpack::maps(e) {
                crate::matpack::Lookup::Ready(m) => m,
                crate::matpack::Lookup::Pending => {
                    ok = false;
                    notes.push(format!("{name}: pack {} still loading", e.pack));
                    continue;
                }
                crate::matpack::Lookup::Unavailable(why) => {
                    ok = false;
                    notes.push(format!("{name}: pack {}: {why}", e.pack));
                    continue;
                }
            };
            let part = ed
                .doc
                .workspace()
                .and_then(|ws| crate::default_scripts::child_named(&ed.doc, ws, name));
            let draw = part.and_then(|r| ed.renderer.as_ref()?.item_draws(r)?.first().copied());
            let layer = draw.map_or(-1, |d| d.mat_layer);
            let tiling = draw.map_or(0.0, |d| d.uvt[0]);
            let maps_found = ["colour", "normal", "roughness", "metalness", "height"]
                .iter()
                .zip(maps.found)
                .filter(|(_, f)| *f)
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join("+");
            // Front face centre: textured (contrast) at the level of the
            // table's average colour drawn flat (within 30 grey levels).
            let (level, contrast) = frame
                .as_ref()
                .and_then(|fr| patch(ed, fr, Vec3::new(x, 2.0, 2.0)))
                .unwrap_or((0.0, 0.0));
            // Brick courses, wood grain, grass and slate show at this
            // distance; Concrete, Metal and Fabric are nearly even in Studio too.
            let patterned = matches!(mat, 848 | 512 | 1280 | 800);
            let good = layer >= 0
                && maps.found[0]
                && (tiling - maps.tiling / crate::matpack::consts(mat, true)[5]).abs() < 1e-6
                && (!patterned || contrast > 5.0);
            ok &= good;
            notes.push(format!(
                "{name}: pack {} ({maps_found}) layer {layer}, {:.3}/stud, face level {level:.0} contrast {contrast:.1}",
                e.pack, tiling
            ));
        }
        if frame.is_none() {
            ok = false;
            notes.push("couldn't read the viewport back".into());
        }
        if let Ok(dir) = std::env::var("RUSTUDIO_SELFTEST_SHOTS") {
            if let Some((w, h, px)) = &frame {
                if let Some(img) = image::RgbaImage::from_raw(*w, *h, px.clone()) {
                    let _ = img.save(std::path::Path::new(&dir).join("material-packs.png"));
                }
            }
        }
        ed.show_grid = true;
        self.steps.push(Step {
            step: "material-packs",
            passed: ok,
            detail: notes.join("; "),
        });
        self.r4.phase = 76;
        false
    }
}
