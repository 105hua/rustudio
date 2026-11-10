//! HandleAdornments, Handles and ArcHandles drawn over the 3D view while
//! playing (the scripts hit-test them in src/domains/gui_input.luau, with
//! the same geometry): boxes / images / pyramids / cylinders / cones as
//! their faces, spheres as discs, lines as segments, in Color3 at
//! Transparency; Handles a sphere per face (half size + 2 studs out);
//! ArcHandles three rings of radius |half size| + 2 with two spheres each,
//! X red, Y green, Z blue (Studio's look, an offline Studio screenshot 2026-11-08).
//! Drawn over the world without depth (Studio draws AlwaysOnTop ones so;
//! the others it depth-tests).

use egui::{Pos2, Shape, Stroke};
use glam::{Mat3, Vec3};

use super::*;
use crate::math::Xform;
use crate::scene::{vec3_of, xform_of};

const HANDLE_R: f32 = 0.55;

pub(super) fn is_adornment(class: &str) -> bool {
    doc::is_a(class, "HandleAdornment") || doc::is_a(class, "HandlesBase")
}

fn adornee(d: &Document, a: Ref) -> Option<(Xform, Vec3)> {
    let ad = match d.prop(a, "Adornee") {
        Some(Variant::Ref(r)) if d.exists(r) => r,
        _ => return None,
    };
    let class = d.class_of(ad)?;
    if doc::is_a(&class, "BasePart") {
        return Some((xform_of(d, ad)?, vec3_of(d.prop(ad, "Size")).unwrap_or(Vec3::ONE)));
    }
    if class == "Model" {
        // Its parts' box, axis-aligned to the world.
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for r in d.subtree(ad) {
            if d.class_of(r).is_some_and(|c| doc::is_a(&c, "BasePart")) {
                if let (Some(x), Some(s)) = (xform_of(d, r), vec3_of(d.prop(r, "Size"))) {
                    let e = x.rot.x_axis.abs() * s.x * 0.5 + x.rot.y_axis.abs() * s.y * 0.5 + x.rot.z_axis.abs() * s.z * 0.5;
                    lo = lo.min(x.pos - e);
                    hi = hi.max(x.pos + e);
                }
            }
        }
        if lo.x <= hi.x {
            return Some((Xform { pos: (lo + hi) * 0.5, rot: Mat3::IDENTITY }, hi - lo));
        }
    }
    None
}

fn then(a: &Xform, b: &Xform) -> Xform {
    Xform { pos: a.apply(b.pos), rot: a.rot * b.rot }
}

impl Gui {
    /// Adornments the scripts made, under the LocalPlayer's PlayerGui and in
    /// Workspace (found with `sync_roots`' scan: `adorn_roots`).
    pub(super) fn draw_adornments(&mut self, d: &Document, cam: &Camera, rect: Rect, painter: &egui::Painter) {
        let mut list: Vec<Ref> = self.adorn_roots.iter().copied().filter(|r| d.exists(*r)).collect();
        // The LocalPlayer's PlayerGui's, in tree order (`sync_roots` keeps
        // the set from the journal; the order is found again when it changes).
        if !self.adorn_pg.is_empty() {
            if self.adorn_pg_order.is_none() {
                self.adorn_pg_order = Some(match self.adorn_pg_of {
                    Some(pg) => d.subtree(pg).into_iter().filter(|r| self.adorn_pg.contains(r)).collect(),
                    None => Vec::new(),
                });
            }
            list.extend(self.adorn_pg_order.iter().flatten().copied().filter(|r| d.exists(*r)));
        }
        let proj = |p: Vec3| cam.project(rect, p);
        for a in list {
            let Some(class) = d.class_of(a) else { continue };
            if !boolp(d, a, "Visible", true) {
                continue;
            }
            let Some((cf, size)) = adornee(d, a) else { continue };
            let alpha = 1.0 - num(d, a, "Transparency", 0.0);
            let col = rgba(color(d, a, "Color3", [13.0 / 255.0, 105.0 / 255.0, 172.0 / 255.0]), alpha.clamp(0.0, 1.0));
            let sphere = |c: Vec3, r: f32, colour: Color32| {
                if let Some(p) = proj(c) {
                    let px = r / cam.world_per_px(rect, c);
                    painter.circle_filled(p, px, colour);
                }
            };
            match class.as_str() {
                "Handles" => {
                    let faces = match d.prop(a, "Faces") {
                        Some(Variant::Faces(f)) => f,
                        _ => rbx_dom_weak::types::Faces::all(),
                    };
                    use rbx_dom_weak::types::Faces as F;
                    for (flag, n) in [
                        (F::RIGHT, Vec3::X),
                        (F::LEFT, -Vec3::X),
                        (F::TOP, Vec3::Y),
                        (F::BOTTOM, -Vec3::Y),
                        (F::BACK, Vec3::Z),
                        (F::FRONT, -Vec3::Z),
                    ] {
                        if faces.contains(flag) {
                            let out = (n * size).abs().max_element() * 0.5 + 2.0;
                            sphere(cf.apply(n * out), HANDLE_R, col);
                        }
                    }
                }
                "ArcHandles" => {
                    let radius = (size * 0.5).length() + 2.0;
                    let axes = match d.prop(a, "Axes") {
                        Some(Variant::Axes(x)) => x,
                        _ => rbx_dom_weak::types::Axes::all(),
                    };
                    use rbx_dom_weak::types::Axes as A;
                    for (flag, u, v, c) in [
                        (A::X, Vec3::Y, Vec3::Z, Color32::from_rgb(220, 30, 30)),
                        (A::Y, Vec3::X, Vec3::Z, Color32::from_rgb(40, 200, 40)),
                        (A::Z, Vec3::X, Vec3::Y, Color32::from_rgb(40, 40, 230)),
                    ] {
                        if !axes.contains(flag) {
                            continue;
                        }
                        let pts: Vec<Pos2> = (0..=64)
                            .filter_map(|i| {
                                let t = i as f32 / 64.0 * std::f32::consts::TAU;
                                proj(cf.apply((u * t.cos() + v * t.sin()) * radius))
                            })
                            .collect();
                        painter.add(Shape::line(pts, Stroke::new(1.0, c)));
                        let spot = if flag == A::X { Vec3::Z } else if flag == A::Y { Vec3::X } else { Vec3::Y };
                        sphere(cf.apply(spot * radius), HANDLE_R, c);
                        sphere(cf.apply(-spot * radius), HANDLE_R, c);
                    }
                }
                _ => {
                    let local = match d.prop(a, "CFrame") {
                        Some(Variant::CFrame(c)) => Xform::from_cframe(&c),
                        _ => Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY },
                    };
                    let rel = vec3_of(d.prop(a, "SizeRelativeOffset")).unwrap_or(Vec3::ZERO);
                    let base = then(&Xform { pos: cf.apply(size * rel * 0.5), rot: cf.rot }, &local);
                    let boxed = |s: Vec3, off: Vec3| {
                        let h = s * 0.5;
                        let c = |x: f32, y: f32, z: f32| base.apply(off + Vec3::new(x * h.x, y * h.y, z * h.z));
                        let quads = [
                            [c(1., -1., -1.), c(1., 1., -1.), c(1., 1., 1.), c(1., -1., 1.)],
                            [c(-1., -1., -1.), c(-1., -1., 1.), c(-1., 1., 1.), c(-1., 1., -1.)],
                            [c(-1., 1., -1.), c(-1., 1., 1.), c(1., 1., 1.), c(1., 1., -1.)],
                            [c(-1., -1., -1.), c(1., -1., -1.), c(1., -1., 1.), c(-1., -1., 1.)],
                            [c(-1., -1., 1.), c(1., -1., 1.), c(1., 1., 1.), c(-1., 1., 1.)],
                            [c(-1., -1., -1.), c(-1., 1., -1.), c(1., 1., -1.), c(1., -1., -1.)],
                        ];
                        // Faces turned to the camera, far first.
                        let mut faces: Vec<(f32, [Vec3; 4])> = quads
                            .iter()
                            .filter(|q| {
                                let n = (q[1] - q[0]).cross(q[2] - q[0]);
                                n.dot(cam.pos - q[0]) > 0.0
                            })
                            .map(|q| (((q[0] + q[2]) * 0.5 - cam.pos).length_squared(), *q))
                            .collect();
                        faces.sort_by(|a, b| b.0.total_cmp(&a.0));
                        for (_, q) in faces {
                            let p: Option<Vec<Pos2>> = q.iter().map(|v| proj(*v)).collect();
                            if let Some(mut p) = p {
                                // One screen winding for egui's fill.
                                let area: f32 = (0..p.len())
                                    .map(|i| {
                                        let (a, b) = (p[i], p[(i + 1) % p.len()]);
                                        a.x * b.y - b.x * a.y
                                    })
                                    .sum();
                                if area < 0.0 {
                                    p.reverse();
                                }
                                painter.add(Shape::convex_polygon(p, col, Stroke::NONE));
                            }
                        }
                    };
                    match class.as_str() {
                        "SphereHandleAdornment" => sphere(base.pos, num(d, a, "Radius", 1.0), col),
                        "BoxHandleAdornment" => boxed(vec3_of(d.prop(a, "Size")).unwrap_or(Vec3::ONE), Vec3::ZERO),
                        "ImageHandleAdornment" => {
                            let s = match d.prop(a, "Size") {
                                Some(Variant::Vector2(v)) => Vec3::new(v.x, v.y, 0.05),
                                _ => Vec3::new(1.0, 1.0, 0.05),
                            };
                            boxed(s, Vec3::ZERO)
                        }
                        "CylinderHandleAdornment" | "ConeHandleAdornment" => {
                            let r = num(d, a, "Radius", 1.0);
                            boxed(Vec3::new(r * 2.0, r * 2.0, num(d, a, "Height", 1.0)), Vec3::ZERO)
                        }
                        "PyramidHandleAdornment" => {
                            let h = num(d, a, "Height", 1.0);
                            let s = num(d, a, "Size", 1.0);
                            boxed(Vec3::new(s, s, h), Vec3::new(0.0, 0.0, -h * 0.5))
                        }
                        "LineHandleAdornment" => {
                            let l = num(d, a, "Length", 5.0);
                            if let (Some(p0), Some(p1)) = (proj(base.pos), proj(base.apply(Vec3::new(0.0, 0.0, -l)))) {
                                painter.line_segment([p0, p1], Stroke::new(num(d, a, "Thickness", 1.0).max(1.0), col));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
