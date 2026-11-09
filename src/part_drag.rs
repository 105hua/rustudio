//! Dragging parts in the 3D view with the Select (or Move) tool, as in the
//! offline Studio runs (measured over a Studio MCP bridge, docs/MECHANICS.md "Round 17"):
//!
//! - pressing on a part and dragging moves it; an unselected part is
//!   selected first, and the whole selection moves together;
//! - the dragged part follows the surface under the cursor: it rests on that
//!   surface (a 4-stud cube dragged over the ground kept Y = 2) and keeps
//!   the offset from where it was grabbed;
//! - with the move snap on, its position snaps to the increment within the
//!   surface's plane (1 stud: −7, 2, −2); with it off it lands where the
//!   cursor puts it (−9.33, 2, −1.98);
//! - the parts the drag carries are ignored by the surface ray;
//! - one undo step per drag.
//!
//! The rules are Studio's own DraggerFramework (SelectDragger.rbxm, read
//! from its Luau bytecode; docs/MECHANICS.md "Round 17, decompiled"):
//! - a box face's surface frame has its origin at the face corner nearest
//!   the point, Y along the face normal and X / Z along the face's edges
//!   (DragHelper.getSurfaceSnapForPartTreatedAsBox, buildSurfaceMatrix);
//! - with grid snap on, the grab point is rounded to the grid in the clicked
//!   face's frame when the drag starts (dispatchWorldClick
//!   .buildFreeformDragInfo), and the mouse hit is rounded in the target
//!   face's frame on X and Z only, `floor(x / grid + 0.5) * grid`
//!   (DragHelper.computeDragSnapping);
//! - the grab point goes to the snapped hit and the selection's bounding box
//!   rests on the surface: lift `0.5 * size.Y - offset.Y` along the normal
//!   (DragHelper.getDragTargetNew).

use std::collections::HashSet;

use egui::{Pos2, Rect};
use glam::Vec3;
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::ustr;

use crate::app::Editor;
use crate::math::{Ray, Xform};

pub struct PartDrag {
    /// The part under the cursor when the drag began.
    grabbed: Ref,
    /// The selection's bounding box when the drag began (the grabbed part's
    /// own box for one part, else the world-aligned box of all).
    bounds_start: Xform,
    bounds_size: Vec3,
    /// From the bounding box's centre to the (snapped) grab point.
    grab_offset: Vec3,
    /// Every part carried, with where it started.
    parts: Vec<(Ref, Xform)>,
    carried: HashSet<Ref>,
    pub moved: bool,
}

/// Half the extent of a box (rotation `xf.rot`, `size`) along unit `n`.
fn half_extent(xf: &Xform, size: Vec3, n: Vec3) -> f32 {
    (0..3).map(|i| n.dot(xf.rot.col(i)).abs() * size[i] * 0.5).sum()
}

/// Studio's grid rounding (`math.floor(x / grid + 0.5) * grid`).
fn round_grid(x: f32, grid: f32) -> f32 {
    (x / grid + 0.5).floor() * grid
}

/// A surface frame as Studio's DragHelper builds it for a box face: origin
/// at the face corner nearest the point, the face normal, and the face's
/// two edge axes. The ground plane is the world frame at the origin.
#[derive(Clone, Copy)]
struct Face {
    corner: Vec3,
    n: Vec3,
    u: Vec3,
    v: Vec3,
}

impl Face {
    const GROUND: Face = Face { corner: Vec3::ZERO, n: Vec3::Y, u: Vec3::X, v: Vec3::Z };

    fn of_box(xf: &Xform, size: Vec3, p: Vec3) -> Face {
        let n = face_normal(xf, size, p);
        let i = (0..3).max_by(|a, b| n.dot(xf.rot.col(*a)).abs().total_cmp(&n.dot(xf.rot.col(*b)).abs())).unwrap_or(1);
        let half = size * 0.5;
        let centre = xf.pos + n * half[i];
        let (a, b) = ((i + 1) % 3, (i + 2) % 3);
        let (u, v) = (xf.rot.col(a), xf.rot.col(b));
        let d = p - centre;
        let corner = centre
            + u * if d.dot(u) >= 0.0 { half[a] } else { -half[a] }
            + v * if d.dot(v) >= 0.0 { half[b] } else { -half[b] };
        Face { corner, n, u, v }
    }

    /// `p` rounded to the grid along the face's edges (not along the normal).
    fn snap(&self, p: Vec3, grid: f32) -> Vec3 {
        let d = p - self.corner;
        self.corner + self.u * round_grid(d.dot(self.u), grid) + self.v * round_grid(d.dot(self.v), grid) + self.n * d.dot(self.n)
    }
}

/// The face normal of a box at a point on its surface.
fn face_normal(xf: &Xform, size: Vec3, p: Vec3) -> Vec3 {
    let local = xf.rot.transpose() * (p - xf.pos);
    let half = size * 0.5;
    let mut best = (0usize, f32::NEG_INFINITY);
    for i in 0..3 {
        let d = local[i].abs() / half[i].max(1e-4);
        if d > best.1 {
            best = (i, d);
        }
    }
    let sign = if local[best.0] >= 0.0 { 1.0 } else { -1.0 };
    xf.rot.col(best.0) * sign
}

impl Editor {
    /// The nearest part surface under `ray` that the drag doesn't carry:
    /// (point, that face's surface frame).
    fn drag_surface(&self, ray: &Ray, carried: &HashSet<Ref>) -> Option<(Vec3, Face)> {
        let mut best: Option<(f32, Xform, Vec3)> = None;
        for sp in &self.scene.parts {
            if carried.contains(&sp.r) || sp.draw.alpha < 0.02 {
                continue;
            }
            if let Some(t) = ray.hit_obb(&sp.xf, sp.size) {
                if best.is_none_or(|(bt, _, _)| t < bt) {
                    best = Some((t, sp.xf, sp.size));
                }
            }
        }
        if let Some((t, xf, size)) = best {
            let p = ray.origin + ray.dir * t;
            return Some((p, Face::of_box(&xf, size, p)));
        }
        // Nothing under the cursor: the ground plane (Y = 0).
        if ray.dir.y.abs() > 1e-4 {
            let t = -ray.origin.y / ray.dir.y;
            if t > 0.0 {
                return Some((ray.origin + ray.dir * t, Face::GROUND));
            }
        }
        None
    }

    /// Starts dragging the part under `p`; false when there is none (the
    /// caller starts a selection box instead).
    pub fn start_part_drag(&mut self, rect: Rect, p: Pos2, add: bool) -> bool {
        let Some(raw) = self.pick(rect, p, true) else {
            return false;
        };
        let target = self.pick(rect, p, false).unwrap_or(raw);
        if !self.doc.selection.contains(&target) {
            if add {
                let mut sel = self.doc.selection.clone();
                sel.push(target);
                self.select(sel);
            } else {
                self.select(vec![target]);
            }
        }
        let Some(gp) = self.scene.get(raw) else {
            return false;
        };
        let (grabbed_start, grabbed_size) = (gp.xf, gp.size);
        let ray = self.cam.ray(rect, p);
        let t = ray.hit_obb(&grabbed_start, grabbed_size).unwrap_or(0.0);
        let mut grab = ray.origin + ray.dir * t;
        // With grid snap on, the grab point is rounded on the clicked face.
        if self.snap_move && self.move_inc > 0.0 {
            grab = Face::of_box(&grabbed_start, grabbed_size, grab).snap(grab, self.move_inc);
        }
        let parts: Vec<(Ref, Xform)> = self
            .selected_parts()
            .into_iter()
            .filter(|r| !matches!(self.doc.prop(*r, "Locked"), Some(Variant::Bool(true))))
            .filter_map(|r| self.scene.get(r).map(|sp| (r, sp.xf)))
            .collect();
        if parts.is_empty() {
            return false;
        }
        // The selection's bounding box: one part's own box, else the
        // world-aligned box around them all.
        let (bounds_start, bounds_size) = if parts.len() == 1 {
            let sp = self.scene.get(parts[0].0).map(|sp| (sp.xf, sp.size));
            sp.unwrap_or((grabbed_start, grabbed_size))
        } else {
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for (r, xf) in &parts {
                let size = self.scene.get(*r).map(|sp| sp.size).unwrap_or(Vec3::ONE);
                let half = (xf.rot.col(0).abs() * size.x + xf.rot.col(1).abs() * size.y + xf.rot.col(2).abs() * size.z) * 0.5;
                lo = lo.min(xf.pos - half);
                hi = hi.max(xf.pos + half);
            }
            (Xform { pos: (lo + hi) * 0.5, rot: glam::Mat3::IDENTITY }, hi - lo)
        };
        let grab_offset = grab - bounds_start.pos;
        let carried = parts.iter().map(|(r, _)| *r).collect();
        self.part_drag = Some(PartDrag {
            grabbed: raw,
            bounds_start,
            bounds_size,
            grab_offset,
            parts,
            carried,
            moved: false,
        });
        true
    }

    /// Moves the carried parts so the grabbed one rests on the surface under `p`.
    pub fn update_part_drag(&mut self, rect: Rect, p: Pos2) {
        let Some(d) = self.part_drag.as_ref() else {
            return;
        };
        let ray = self.cam.ray(rect, p);
        let Some((mut hit, face)) = self.drag_surface(&ray, &d.carried) else {
            return;
        };
        if self.snap_move && self.move_inc > 0.0 {
            hit = face.snap(hit, self.move_inc);
        }
        // The grab point goes to the hit (its offset kept within the
        // surface's plane) and the bounding box rests on the surface.
        let n = face.n;
        let off = d.grab_offset - n * d.grab_offset.dot(n);
        let center = hit - off + n * half_extent(&d.bounds_start, d.bounds_size, n);
        let delta = center - d.bounds_start.pos;
        let moves: Vec<(Ref, Xform)> = d
            .parts
            .iter()
            .map(|(r, xf)| (*r, Xform { pos: xf.pos + delta, rot: xf.rot }))
            .collect();
        let _ = d.grabbed;
        for (r, xf) in moves {
            self.doc.set_prop_live(r, ustr("CFrame"), Variant::CFrame(xf.to_cframe()));
        }
        if let Some(d) = self.part_drag.as_mut() {
            d.moved |= delta.length() > 1e-4;
        }
    }

    /// Ends the drag: one undo step for everything it moved.
    pub fn finish_part_drag(&mut self) {
        let Some(d) = self.part_drag.take() else {
            return;
        };
        if !d.moved {
            return;
        }
        let changes = d
            .parts
            .iter()
            .filter_map(|(r, xf)| {
                let new = self.doc.dom.get_by_ref(*r)?.properties.get(&ustr("CFrame")).cloned()?;
                Some((*r, ustr("CFrame"), Some(Variant::CFrame(xf.to_cframe())), new))
            })
            .collect();
        self.doc.record_props("Drag", changes);
    }
}
