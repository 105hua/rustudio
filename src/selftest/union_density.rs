//! Rendered regression: unresized and stretched CSG beside a plain Slate Part.
use super::{SelfTest, Step};
use crate::{app::Editor, doc::Document};
use glam::Vec3;

impl SelfTest {
    pub(super) fn tick_union_density(&mut self, f: u64, ed: &mut Editor) -> bool {
        if self.r4.phase == 250 {
            let doc = Document::from_place_bytes(include_bytes!("../../tools/fidelity/corpus/union-density.rbxlx"), "Union density".into()).unwrap();
            ed.set_document(doc);
            ed.show_grid = false;
            ed.cam.pos = Vec3::new(0.0, 4.0, 63.0);
            ed.cam.yaw = 0.0;
            ed.cam.pitch = 0.0;
            ed.cam.fov_deg = 45.0;
            self.r6.started = f;
            self.r4.phase = 251;
            return false;
        }
        if f.saturating_sub(self.r6.started) < 120 { return false; }
        let frame = ed.renderer.as_ref().zip(ed.render_state.as_ref()).and_then(|(r,rs)| r.read_frame(rs));
        // Compare the spatial frequency of equal stud-sized interior patches.
        // Gradient energy / variance rejects a stretched texture independently
        // of its phase, tint, and overall illumination.
        let mut frequencies = Vec::new();
        if let Some((w,h,px)) = &frame {
            let rect = ed.viewport_rect;
            let scale = *w as f32 / rect.width().max(1.0);
            let radius = (rect.height() * scale / 675.0 * 32.0).max(16.0) as i32;
            for x in [-16.0,0.0,16.0] {
                if let Some(p) = ed.cam.project(rect, Vec3::new(x,4.0,0.5)) {
                    let cx = ((p.x-rect.min.x)*scale) as i32;
                    let cy = ((p.y-rect.min.y)*scale) as i32;
                    let mut values = Vec::new(); let mut energy = 0.0;
                    for y in cy-radius..cy+radius { for x in cx-radius..cx+radius {
                        if x < 0 || y < 0 || x+1 >= *w as i32 || y >= *h as i32 { continue; }
                        let at = ((y as u32*w+x as u32)*4) as usize;
                        let a = px[at] as f32; let b = px[at+4] as f32;
                        values.push(a); energy += (a-b).powi(2);
                    }}
                    let n = values.len().max(1) as f32;
                    let mean = values.iter().sum::<f32>()/n;
                    let variance = values.iter().map(|v|(v-mean).powi(2)).sum::<f32>()/n;
                    frequencies.push((energy/n)/variance.max(0.01));
                }
            }
            let _ = image::save_buffer(self.out.with_extension("union-density.png"), px, *w,*h,image::ColorType::Rgba8);
        }
        let passed = frequencies.len()==3 && frequencies.iter().all(|v| *v > 0.01 && (*v/frequencies[2]-1.0).abs()<0.45);
        self.steps.push(Step { step:"union-material-density", passed, detail:format!("normalized spatial gradient energy (union 8/8, union 12/4, Part): {frequencies:?}") });
        ed.show_grid = true;
        self.r4.phase = 252;
        false
    }
}
