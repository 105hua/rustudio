//! Self-test step for the Terrain Editor (round 21, phase 116):
//! `terrain-editor` opens the panel, Generates hills (one undo step), then
//! holds the left button in the 3D view with Draw and Snow picked: the
//! stroke adds Snow under the brush, re-meshes, and is one undo step; undo
//! takes the Snow away and leaves the hills; Clear empties the grid and
//! undo brings it back. Then `terrain-region` (round 25) runs the Region
//! tools on the hills (see `tick_terrain_region`).

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use glam::Vec3;

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::terrain::Voxels;
use crate::terrain_editor::{Biome, RegionBox, RegionHandle, Tool};
use crate::terrain_region::{self as region, Region};

#[derive(Default)]
pub(super) struct TerrainEdTest {
    started: u64,
    undo_before: usize,
    tris_before: usize,
    notes: Vec<String>,
    ok: bool,
    at: Pos2,
    /// Region tools: the grid before, what the box held, the drag's way.
    grid: Vec<u8>,
    content: Option<Region>,
    dir: Vec2,
    /// Which region check is next, and the frame the Move drag began.
    stage: u8,
    t0: u64,
    region: RegionBox,
}

const SNOW: u8 = 10;

fn voxels(ed: &Editor) -> Voxels {
    match Voxels::from_dom(&ed.doc.dom) {
        Some(Ok(v)) => v,
        _ => Voxels::empty(),
    }
}

fn snow_count(v: &Voxels) -> usize {
    let Some((lo, hi)) = v.bounds() else { return 0 };
    let mut n = 0;
    for y in lo[1]..=hi[1] {
        for z in lo[2]..=hi[2] {
            for x in lo[0]..=hi[0] {
                n += usize::from(v.get(x, y, z).0 == SNOW);
            }
        }
    }
    n
}

fn button(q: &mut Vec<Event>, p: Pos2, down: bool) {
    q.push(Event::PointerMoved(p));
    q.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
}

impl SelfTest {
    pub(super) fn tick_terrain_ed(&mut self, f: u64, ed: &mut Editor) -> bool {
        if self.ted.started == 0 {
            if ed.play.is_some() {
                ed.stop_play();
            }
            ed.new_place();
            ed.ui.panel_requests.push((crate::app::Tab::TerrainEditor, Some(true)));
            ed.terrain_ed.tool = Some(Tool::Generate);
            ed.terrain_ed.region_pos = [0.0, 16.0, 0.0];
            ed.terrain_ed.region_size = [128.0, 32.0, 128.0];
            ed.terrain_ed.biome = Biome::Hills;
            ed.terrain_ed.gen_water = false;
            let before = ed.doc.undo_len();
            ed.terrain_action("generate");
            self.ted = TerrainEdTest { started: f, undo_before: before, ..Default::default() };
            // Looking down on the middle of the hills.
            ed.cam.mode = crate::camera::CameraMode::Fly;
            ed.cam.basis = None;
            ed.cam.pos = Vec3::new(0.0, 90.0, 40.0);
            ed.cam.focus = Vec3::ZERO;
            ed.cam.yaw = 0.0;
            ed.cam.pitch = -1.15;
            ed.cam.fov_deg = 70.0;
            return false;
        }
        let t = f - self.ted.started;
        match t {
            10 => {
                let v = voxels(ed);
                let open = ed.ui.panels_open.contains("Terrain Editor");
                let steps = ed.doc.undo_len() - self.ted.undo_before;
                self.ted.ok = open && v.count() > 1000 && ed.terrain_tris > 0 && steps == 1;
                self.ted.notes.push(format!(
                    "panel open: {open}; Generate: {} voxels, {} triangles, {steps} undo step",
                    v.count(),
                    ed.terrain_tris
                ));
                self.ted.tris_before = ed.terrain_tris;
                ed.terrain_ed.tool = Some(Tool::Draw);
                ed.terrain_ed.material = SNOW;
                ed.terrain_ed.size = 16.0;
                ed.terrain_ed.strength = 1.0;
                self.ted.undo_before = ed.doc.undo_len();
                self.ted.at = ed.viewport_rect.center();
                self.queue.push(Event::PointerMoved(self.ted.at));
            }
            12 => {
                let hover = ed.terrain_ed.hover.map(|h| h.0);
                self.ted.ok &= hover.is_some_and(|p| p.y > 0.0 && p.y < 40.0);
                self.ted.notes.push(format!("brush over the ground at {hover:?}"));
                button(&mut self.queue, self.ted.at, true);
            }
            13..=30 => {
                // A short drag to the right while held.
                let p = self.ted.at + Vec2::new((t - 12) as f32 * 3.0, 0.0);
                self.queue.push(Event::PointerMoved(p));
            }
            31 => button(&mut self.queue, self.ted.at + Vec2::new(54.0, 0.0), false),
            36 => {
                let v = voxels(ed);
                let snow = snow_count(&v);
                let steps = ed.doc.undo_len() - self.ted.undo_before;
                let remeshed = ed.terrain_tris != self.ted.tris_before;
                ed.undo();
                let after_undo = voxels(ed);
                let snow_back = snow_count(&after_undo);
                ed.terrain_action("clear");
                let cleared = voxels(ed).count();
                ed.undo();
                let restored = voxels(ed).count();
                self.ted.ok &= snow > 20 && steps == 1 && remeshed && snow_back == 0 && after_undo.count() > 1000 && cleared == 0 && restored == after_undo.count();
                self.ted.notes.push(format!(
                    "Draw with Snow held for 18 frames: {snow} Snow voxels, {steps} undo step, re-meshed: {remeshed}; undo → {snow_back} Snow, {} voxels; Clear → {cleared}, undo → {restored}",
                    after_undo.count()
                ));
                self.steps.push(Step {
                    step: "terrain-editor",
                    passed: self.ted.ok,
                    detail: self.ted.notes.join("; "),
                });
                self.ted.notes.clear();
                self.ted.ok = true;
            }
            37.. => return self.tick_terrain_region(t, ed),
            _ => {}
        }
        false
    }

    /// Round 25: the Region tools on the same hills. Select drags a box on
    /// the ground; Copy, Delete, a Move drag on the +X arrow (Merge Empty),
    /// a quarter turn, Fill and Paste each change the terrain as one undo
    /// step, and undo gives back the hills byte for byte.
    fn tick_terrain_region(&mut self, t: u64, ed: &mut Editor) -> bool {
        let rect = ed.viewport_rect;
        let start = self.ted.at - Vec2::new(60.0, 30.0);
        let undo_steps = |ed: &Editor, before: usize| ed.doc.undo_len() as i64 - before as i64;
        match t {
            37 => {
                // A fly key an earlier step left down would drift the camera
                // under the drags.
                for k in [Key::W, Key::A, Key::S, Key::D, Key::Q, Key::E] {
                    self.queue.push(super::key(k, false, Modifiers::NONE));
                }
                ed.terrain_ed.tool = Some(Tool::Select);
                ed.terrain_ed.selection = None;
                button(&mut self.queue, start, true);
            }
            38..=43 => self.queue.push(Event::PointerMoved(start + Vec2::new(20.0, 10.0) * (t - 37) as f32)),
            44 => button(&mut self.queue, start + Vec2::new(120.0, 60.0), false),
            // Each check waits for the drag before it to be over (a loaded
            // machine runs frames late).
            _ if self.ted.stage == 0 && t >= 48 && !ed.region_dragging() && (ed.terrain_ed.selection.is_some() || t > 120) => {
                self.ted.stage = 1;
                self.ted.t0 = t;
                let sel = ed.terrain_ed.selection;
                self.ted.ok &= sel.is_some_and(|b| b.size[0] > 2 && b.size[2] > 2);
                self.ted.notes.push(format!("Select drag on the ground: {sel:?}"));
                // A known box for the rest: 32 studs a side in the middle of
                // the hills, looked at from above (the camera may have moved).
                let b = RegionBox { lo: [-4, 0, -4], size: [8, 8, 8] };
                ed.cam.mode = crate::camera::CameraMode::Fly;
                ed.cam.basis = None;
                ed.cam.pos = Vec3::new(0.0, 90.0, 40.0);
                ed.cam.focus = Vec3::ZERO;
                ed.cam.yaw = 0.0;
                ed.cam.pitch = -1.15;
                self.ted.region = b;
                ed.terrain_ed.selection = Some(b);
                ed.terrain_ed.merge_empty = true;
                let before = voxels(ed);
                self.ted.grid = before.encode();
                self.ted.content = Some(Region::copy(&before, b.lo, b.size));
                ed.terrain_region_action("copy");
                let u = ed.doc.undo_len();
                ed.terrain_region_action("delete");
                let left = Region::copy(&voxels(ed), b.lo, b.size).count();
                let steps = undo_steps(ed, u);
                ed.undo();
                let back = voxels(ed).encode() == self.ted.grid;
                let n = self.ted.content.as_ref().map_or(0, |c| c.count());
                self.ted.ok &= ed.terrain_ed.clipboard.is_some() && n > 50 && left == 0 && steps == 1 && back;
                self.ted.notes.push(format!("Copy {n} voxels; Delete → {left} left, {steps} undo step; undo → same grid: {back}"));
                // Move: press on the +X arrow and drag along it.
                ed.terrain_ed.tool = Some(Tool::Move);
                self.ted.undo_before = ed.doc.undo_len();
                let arrow = ed
                    .region_handles(rect, Tool::Move)
                    .into_iter()
                    .find(|(h, _)| *h == RegionHandle::Face(0, 1))
                    .map(|(_, p)| p);
                match arrow {
                    Some(p) if p.len() == 2 => {
                        self.ted.at = p[0] + (p[1] - p[0]) * 0.5;
                        self.ted.dir = (p[1] - p[0]).normalized();
                        button(&mut self.queue, self.ted.at, true);
                    }
                    _ => {
                        self.ted.ok = false;
                        self.ted.notes.push("no +X arrow on screen".into());
                    }
                }
            }
            _ if self.ted.stage == 1 && t > self.ted.t0 && t <= self.ted.t0 + 8 => {
                self.queue.push(Event::PointerMoved(self.ted.at + self.ted.dir * 12.0 * (t - self.ted.t0) as f32));
                if t == self.ted.t0 + 5 {
                    self.ted.notes.push(format!("arrow pressed at {:?} in {rect:?}: dragging {}", self.ted.at, ed.region_dragging()));
                }
                if t == self.ted.t0 + 8 {
                    // Round 26: still held, the drawn terrain already shows
                    // the content in the moved box (Live Edit), while the
                    // document and its undo stack are untouched.
                    let sel = ed.terrain_ed.selection.expect("selection");
                    let shown = match (ed.terrain_brushed.as_ref(), ed.terrain_drawn.as_ref()) {
                        (Some((v, _, _)), _) | (None, Some((_, _, v))) => Region::copy(v, sel.lo, sel.size),
                        _ => Region::copy(&Voxels::empty(), sel.lo, sel.size),
                    };
                    let live = ed.region_dragging()
                        && sel.lo[0] > self.ted.region.lo[0]
                        && self.ted.content.as_ref() == Some(&shown);
                    let doc_same = voxels(ed).encode() == self.ted.grid && ed.doc.undo_len() == self.ted.undo_before;
                    self.ted.ok &= live && doc_same;
                    self.ted.notes.push(format!("mid-drag at {:?}: terrain shown in the box {live}, document untouched {doc_same}", sel.lo));
                }
            }
            _ if self.ted.stage == 1 && t == self.ted.t0 + 9 => {
                self.ted.stage = 2;
                button(&mut self.queue, self.ted.at + self.ted.dir * 96.0, false)
            }
            _ if self.ted.stage == 2 && t >= self.ted.t0 + 13 && !ed.region_dragging() => {
                let sel = ed.terrain_ed.selection.expect("selection");
                let now = voxels(ed);
                let moved = sel.lo[0] > self.ted.region.lo[0] && sel.lo[1] == 0 && sel.size == [8, 8, 8];
                let content = self.ted.content.clone().expect("content");
                let same = Region::copy(&now, sel.lo, sel.size) == content;
                let steps = undo_steps(ed, self.ted.undo_before);
                if steps > 0 {
                    ed.undo();
                }
                let back = voxels(ed).encode() == self.ted.grid;
                self.ted.ok &= moved && same && steps == 1 && back;
                self.ted.notes.push(format!(
                    "Move drag on +X: box at {:?}, terrain moved with it: {same}, {steps} undo step; undo → same grid: {back}",
                    sel.lo
                ));
                // Rotate: a quarter turn about Y of the cube in place.
                let b = self.ted.region;
                ed.terrain_ed.selection = Some(b);
                ed.terrain_ed.tool = Some(Tool::Rotate);
                let u = ed.doc.undo_len();
                ed.terrain_region_action("rotate_y");
                let turned = Region::copy(&voxels(ed), b.lo, b.size) == content.rotated(&region::quarter_turns(1, 1));
                let steps = undo_steps(ed, u);
                ed.undo();
                let back = voxels(ed).encode() == self.ted.grid;
                self.ted.ok &= turned && steps == 1 && back;
                self.ted.notes.push(format!("Rotate 90° about Y: turned {turned}, {steps} undo step; undo → same grid: {back}"));
                // Fill with Snow, then Paste the copy back where it came from.
                ed.terrain_ed.tool = Some(Tool::Fill);
                ed.terrain_ed.fill_mode = crate::terrain_editor::FillMode::Fill;
                ed.terrain_ed.material = SNOW;
                let u = ed.doc.undo_len();
                ed.terrain_region_action("fill");
                let snow = snow_count(&voxels(ed));
                let steps = undo_steps(ed, u);
                ed.terrain_region_action("paste");
                let pasted = voxels(ed).encode() == self.ted.grid;
                let paste_steps = undo_steps(ed, u) - steps;
                let tool = ed.terrain_ed.tool;
                ed.undo();
                ed.undo();
                let back = voxels(ed).encode() == self.ted.grid;
                self.ted.ok &= snow == 512 && steps == 1 && pasted && paste_steps == 1 && tool == Some(Tool::Move) && back;
                self.ted.notes.push(format!(
                    "Fill Snow → {snow} Snow voxels, {steps} undo step; Paste over it → the hills again: {pasted} ({paste_steps} step, then {tool:?}); two undos → same grid: {back}"
                ));
                self.steps.push(Step {
                    step: "terrain-region",
                    passed: self.ted.ok,
                    detail: self.ted.notes.join("; "),
                });
                ed.terrain_ed = Default::default();
                ed.new_place();
                self.r4.phase = 4;
            }
            _ if t > 400 => {
                self.ted.notes.push(format!("a drag never ended (stage {})", self.ted.stage));
                self.steps.push(Step { step: "terrain-region", passed: false, detail: self.ted.notes.join("; ") });
                ed.terrain_ed = Default::default();
                ed.new_place();
                self.r4.phase = 4;
            }
            _ => {}
        }
        false
    }
}
