//! ParticleEmitter2D and UIBlur in the GUI walk.
//!
//! Studio 0.744 has both classes but draws neither (its UIParticles /
//! UIBlur flags are off; forced on in a offline Studio they still drew
//! nothing), so these follow the properties' meaning, ParticleEmitter's
//! rules carried to the plane (checks/parity/gui_input.md):
//!
//! - ParticleEmitter2D, a child of a GuiObject: Rate per second while
//!   Enabled, plus Emit(n); born at the parent's centre (EmissionMode Point)
//!   or anywhere in its box (BoundingBox), moved by EmissionOffset and a
//!   random share of EmissionSpread; heading EmissionAngle (degrees
//!   clockwise from up) ± SpreadAngle / 2 at Speed px/s; Acceleration px/s²,
//!   Drag halving speed every 1 / Drag s; Lifetime, Rotation, RotSpeed;
//!   Size / Squash / Transparency / Color over the particle's life (each
//!   particle keeps its own envelope draw); Brightness and LightEmission
//!   lighten; TimeScale; LockedToParent keeps them in the parent's frame;
//!   flipbooks (Grid2x2 / 4x4 / 8x8 / Custom, Loop / OneShot / PingPong /
//!   Random at FlipbookFramerate).
//! - UIBlur, a child of a GuiObject on a ScreenGui: what lies behind the
//!   object (the 3D view and the GUI drawn before it) averaged over a disc
//!   of taps (Light 4, Medium 8, Heavy 16 px), tinted by Color, faded out by
//!   Transparency.

use egui::epaint::Vertex;
use egui::Shape;
use glam::Vec2;

use super::walk::{Down, Item, Walk};
use super::*;

#[derive(Default)]
pub(crate) struct Emitter {
    parts: Vec<Particle>,
    accum: f32,
    pending: u32,
    rng: u64,
}

struct Particle {
    pos: Vec2,
    vel: Vec2,
    age: f32,
    life: f32,
    rot: f32,
    spin: f32,
    /// −1..1, this particle's draw within the sequences' envelopes.
    env: f32,
    frame0: f32,
    fps: f32,
}

impl Emitter {
    fn rand(&mut self) -> f32 {
        // xorshift64*
        if self.rng == 0 {
            self.rng = 0x9E37_79B9_7F4A_7C15;
        }
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
    }
}

fn nseq(d: &Document, r: Ref, k: &str, dflt: f32) -> Vec<(f32, f32, f32)> {
    match d.prop(r, k) {
        Some(Variant::NumberSequence(s)) if !s.keypoints.is_empty() => {
            s.keypoints.iter().map(|k| (k.time, k.value, k.envelope)).collect()
        }
        Some(v) => vec![(0.0, f32_of(Some(v)).unwrap_or(dflt), 0.0)],
        None => vec![(0.0, dflt, 0.0)],
    }
}

fn nseq_at(k: &[(f32, f32, f32)], t: f32, e: f32) -> f32 {
    let v = |i: usize| k[i].1 + k[i].2 * e;
    if k.len() == 1 || t <= k[0].0 {
        return v(0);
    }
    for i in 1..k.len() {
        if t <= k[i].0 {
            let span = (k[i].0 - k[i - 1].0).max(1e-6);
            return v(i - 1) + (v(i) - v(i - 1)) * ((t - k[i - 1].0) / span);
        }
    }
    v(k.len() - 1)
}

fn cseq(d: &Document, r: Ref) -> Vec<(f32, [f32; 3])> {
    match d.prop(r, "Color") {
        Some(Variant::ColorSequence(s)) if !s.keypoints.is_empty() => {
            s.keypoints.iter().map(|k| (k.time, [k.color.r, k.color.g, k.color.b])).collect()
        }
        Some(Variant::Color3(c)) => vec![(0.0, [c.r, c.g, c.b])],
        _ => vec![(0.0, [1.0; 3])],
    }
}

fn cseq_at(k: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    if k.len() == 1 || t <= k[0].0 {
        return k[0].1;
    }
    for i in 1..k.len() {
        if t <= k[i].0 {
            let f = (t - k[i - 1].0) / (k[i].0 - k[i - 1].0).max(1e-6);
            let (a, b) = (k[i - 1].1, k[i].1);
            return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
        }
    }
    k[k.len() - 1].1
}

fn range(d: &Document, r: Ref, k: &str, dflt: (f32, f32)) -> (f32, f32) {
    match d.prop(r, k) {
        Some(Variant::NumberRange(n)) => (n.min, n.max),
        Some(v) => {
            let x = f32_of(Some(v)).unwrap_or(dflt.0);
            (x, x)
        }
        None => dflt,
    }
}

fn v2(d: &Document, r: Ref, k: &str) -> Vec2 {
    match d.prop(r, k) {
        Some(Variant::Vector2(v)) => Vec2::new(v.x, v.y),
        _ => Vec2::ZERO,
    }
}

fn texture_of(d: &Document, r: Ref) -> String {
    match d.prop(r, "TextureContent") {
        Some(Variant::Content(c)) => c.as_uri().map(|s| s.to_string()).unwrap_or_default(),
        Some(Variant::ContentId(c)) => c.as_str().to_string(),
        Some(Variant::String(s)) => s,
        _ => "rbxasset://textures/particles/sparkles_main.dds".to_string(),
    }
}

impl Gui {
    /// Once a frame, before walking: the frame time and the scripts'
    /// Emit / Clear calls.
    pub(super) fn fx_frame(&mut self, doc: &Document) {
        let now = std::time::Instant::now();
        let dt = self
            .p2d_clock
            .map(|t| now.duration_since(t).as_secs_f32().min(0.1))
            .unwrap_or(0.0);
        self.p2d_clock = Some(now);
        self.p2d_dt = dt;
        for (r, n) in std::mem::take(&mut super::share().p2d) {
            let e = self.p2d.entry(r).or_default();
            if n == 0 {
                e.parts.clear();
                e.pending = 0;
            } else {
                e.pending += n;
            }
        }
        self.p2d.retain(|r, _| doc.exists(*r));
    }
}

impl Walk<'_, '_> {
    /// The ParticleEmitter2D children of `host` (drawn over it, under its
    /// children).
    pub(super) fn particles(&mut self, host: Ref, rect: Rect, me: &Down) {
        let d = self.input.doc;
        let emitters: Vec<Ref> = children(d, host)
            .into_iter()
            .filter(|c| d.class_of(*c).is_some_and(|k| k == "ParticleEmitter2D"))
            .collect();
        // (particles move every frame)
        if !emitters.is_empty() { self.gui.no_cache = Some("particles2d"); }
        for em in emitters {
            self.emitter(em, rect, me);
        }
    }

    fn emitter(&mut self, em: Ref, rect: Rect, me: &Down) {
        let d = self.input.doc;
        let ts = num(d, em, "TimeScale", 1.0).max(0.0);
        let dt = self.gui.p2d_dt * ts;
        let locked = boolp(d, em, "LockedToParent", false);
        let origin = Vec2::new(rect.min.x, rect.min.y);
        let rate = num(d, em, "Rate", 20.0).max(0.0);
        let enabled = boolp(d, em, "Enabled", true);
        let life = range(d, em, "Lifetime", (5.0, 10.0));
        let speed = range(d, em, "Speed", (64.0, 64.0));
        let rotation = range(d, em, "Rotation", (0.0, 0.0));
        let rotspeed = range(d, em, "RotSpeed", (0.0, 0.0));
        let angle = num(d, em, "EmissionAngle", 0.0);
        let spread = num(d, em, "SpreadAngle", 0.0);
        let point = enum_u32(d.prop(em, "EmissionMode")).unwrap_or(1) == 0;
        let offset = v2(d, em, "EmissionOffset") * me.scale;
        let espread = v2(d, em, "EmissionSpread") * me.scale;
        let accel = v2(d, em, "Acceleration") * me.scale;
        let drag = num(d, em, "Drag", 0.0);
        let fps_r = range(d, em, "FlipbookFramerate", (1.0, 1.0));
        let flip_mode = enum_u32(d.prop(em, "FlipbookMode")).unwrap_or(0);
        let layout = enum_u32(d.prop(em, "FlipbookLayout")).unwrap_or(0);
        let (fx, fy) = match layout {
            1 => (2, 2),
            2 => (4, 4),
            3 => (8, 8),
            4 => (
                (num(d, em, "FlipbookSizeX", 1.0) as u32).max(1),
                (num(d, em, "FlipbookSizeY", 1.0) as u32).max(1),
            ),
            _ => (1, 1),
        };
        let frames = (fx * fy) as f32;
        let start_random = boolp(d, em, "FlipbookStartRandom", false);
        let e = self.gui.p2d.entry(em).or_default();
        // Births.
        let mut births = std::mem::take(&mut e.pending);
        if enabled && dt > 0.0 {
            e.accum += rate * dt;
            let n = e.accum.floor();
            e.accum -= n;
            births += n as u32;
        }
        for _ in 0..births.min(10_000) {
            let mut p = if point {
                Vec2::new(rect.center().x, rect.center().y)
            } else {
                Vec2::new(rect.min.x + e.rand() * rect.width(), rect.min.y + e.rand() * rect.height())
            };
            p += offset + Vec2::new((e.rand() - 0.5) * espread.x, (e.rand() - 0.5) * espread.y);
            if locked {
                p -= origin;
            }
            let a = (angle + (e.rand() - 0.5) * spread).to_radians();
            let s = (speed.0 + e.rand() * (speed.1 - speed.0)) * me.scale;
            let lf = life.0 + e.rand() * (life.1 - life.0);
            let part = Particle {
                pos: p,
                vel: Vec2::new(a.sin(), -a.cos()) * s,
                age: 0.0,
                life: lf.max(1e-3),
                rot: rotation.0 + e.rand() * (rotation.1 - rotation.0),
                spin: rotspeed.0 + e.rand() * (rotspeed.1 - rotspeed.0),
                env: e.rand() * 2.0 - 1.0,
                frame0: if start_random { (e.rand() * frames).floor() } else { 0.0 },
                fps: fps_r.0 + e.rand() * (fps_r.1 - fps_r.0),
            };
            e.parts.push(part);
        }
        // Motion.
        let keep = 0.5f32.powf(drag * dt);
        for p in &mut e.parts {
            p.vel += accel * dt;
            p.vel *= keep;
            p.pos += p.vel * dt;
            p.rot += p.spin * dt;
            p.age += dt;
        }
        e.parts.retain(|p| p.age < p.life);
        if e.parts.is_empty() {
            return;
        }
        // Drawing.
        let size = nseq(d, em, "Size", 32.0);
        let squash = nseq(d, em, "Squash", 0.0);
        let transp = nseq(d, em, "Transparency", 0.0);
        let color = cseq(d, em);
        let bright = num(d, em, "Brightness", 1.0).max(0.0);
        let light = num(d, em, "LightEmission", 0.0).clamp(0.0, 1.0);
        let tex_id = texture_of(d, em);
        let tex = self
            .gui
            .texture(self.ctx, self.input.store, self.input.doc_dir, &tex_id)
            .map(|t| t.0);
        let e = self.gui.p2d.get(&em).expect("emitter");
        let mut mesh = egui::epaint::Mesh::with_texture(tex.unwrap_or_default());
        for p in &e.parts {
            let t = (p.age / p.life).clamp(0.0, 1.0);
            let s = nseq_at(&size, t, p.env).max(0.0) * me.scale;
            let q = nseq_at(&squash, t, p.env);
            let (w, h) = (s * 2f32.powf(-q) * 0.5, s * 2f32.powf(q) * 0.5);
            let a = (1.0 - nseq_at(&transp, t, p.env)).clamp(0.0, 1.0) * me.alpha;
            if a <= 0.0 || s <= 0.0 {
                continue;
            }
            let c = cseq_at(&color, t);
            let k = bright * (1.0 + light);
            let col = Color32::from_rgba_unmultiplied(
                ((c[0] * k * me.tint[0]).min(1.0) * 255.0) as u8,
                ((c[1] * k * me.tint[1]).min(1.0) * 255.0) as u8,
                ((c[2] * k * me.tint[2]).min(1.0) * 255.0) as u8,
                (a * 255.0) as u8,
            );
            // Flipbook frame.
            let frame = if frames > 1.0 {
                let n = p.frame0 + p.age * p.fps;
                (match flip_mode {
                    1 => n.min(frames - 1.0),
                    2 => {
                        let period = (2.0 * frames - 2.0).max(1.0);
                        let m = n % period;
                        if m < frames { m } else { period - m }
                    }
                    3 => p.frame0,
                    _ => n % frames,
                })
                .floor()
            } else {
                0.0
            };
            let (col_i, row_i) = (frame as u32 % fx, frame as u32 / fx);
            let uv0 = egui::pos2(col_i as f32 / fx as f32, row_i as f32 / fy as f32);
            let uv1 = egui::pos2((col_i + 1) as f32 / fx as f32, (row_i + 1) as f32 / fy as f32);
            let centre = if locked { p.pos + origin } else { p.pos };
            let (sn, cs) = p.rot.to_radians().sin_cos();
            let corner = |x: f32, y: f32| egui::pos2(centre.x + x * cs - y * sn, centre.y + x * sn + y * cs);
            let base = mesh.vertices.len() as u32;
            for (pos, uv) in [
                (corner(-w, -h), uv0),
                (corner(w, -h), egui::pos2(uv1.x, uv0.y)),
                (corner(w, h), uv1),
                (corner(-w, h), egui::pos2(uv0.x, uv1.y)),
            ] {
                mesh.vertices.push(Vertex { pos, uv: if tex.is_some() { uv } else { egui::epaint::WHITE_UV }, color: col });
            }
            mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        if !mesh.indices.is_empty() {
            let mut at = *me;
            if self.global_z {
                at.z = num(d, em, "ZIndex", 1.0) as i32;
            }
            self.push_item(Shape::Mesh(mesh.into()), &at);
        }
    }

    /// A UIBlur child of `host`: the view behind it, blurred.
    pub(super) fn blur(&mut self, host: Ref, rect: Rect, me: &Down) {
        let d = self.input.doc;
        let Some(b) = children(d, host)
            .into_iter()
            .find(|c| d.class_of(*c).is_some_and(|k| k == "UIBlur"))
        else {
            return;
        };
        if !boolp(d, b, "Enabled", true) || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        // (it shows the 3D view behind it, which changes every frame)
        self.gui.no_cache = Some("uiblur");
        let Some(origin) = self.screen else { return };
        let radius = match enum_u32(d.prop(b, "Strength")).unwrap_or(2) {
            0 => 4.0,
            1 => 8.0,
            _ => 16.0,
        } * me.scale;
        let tint = color(d, b, "Color", [1.0, 1.0, 1.0]);
        let keep = (1.0 - num(d, b, "Transparency", 0.0)).clamp(0.0, 1.0) * me.alpha;
        if keep <= 0.0 {
            return;
        }
        let scene = super::share().scene;
        // What was drawn behind it so far in this GUI.
        let behind: Vec<(Shape, Rect, super::walk::Aff, i32)> = self
            .items
            .iter()
            .filter(|it| it.z <= me.z || !self.global_z)
            .filter(|it| it.shape.visual_bounding_rect().intersects(rect.expand(radius)))
            .map(|it| (it.shape.clone(), it.clip, it.aff, it.z))
            .collect();
        const TAPS: usize = 12;
        for k in 0..TAPS {
            // Golden-angle disc.
            let f = (k as f32 + 0.5) / TAPS as f32;
            let ang = k as f32 * 2.399_963;
            let off = egui::vec2(ang.cos(), ang.sin()) * radius * f.sqrt();
            let a = keep / (k + 1) as f32;
            if let Some((tex, srect)) = scene {
                let s0 = rect.min + origin.to_vec2() + off;
                let s1 = rect.max + origin.to_vec2() + off;
                let uv = Rect::from_min_max(
                    egui::pos2((s0.x - srect.min.x) / srect.width(), (s0.y - srect.min.y) / srect.height()),
                    egui::pos2((s1.x - srect.min.x) / srect.width(), (s1.y - srect.min.y) / srect.height()),
                );
                let c = Color32::from_rgba_unmultiplied(
                    (tint[0] * 255.0) as u8,
                    (tint[1] * 255.0) as u8,
                    (tint[2] * 255.0) as u8,
                    (a * 255.0) as u8,
                );
                let m = quad_mesh(rect, uv, tex, &|_| c, 1);
                self.push_item(Shape::Mesh(m.into()), me);
            }
            for (shape, clip, aff, z) in &behind {
                let mut s = shape.clone();
                s.translate(off);
                fade(&mut s, a, tint);
                self.seq += 1;
                self.items.push(Item {
                    shape: s,
                    clip: clip.intersect(rect),
                    aff: *aff,
                    z: (*z).max(me.z),
                    seq: self.seq,
                    by: self.cur,
                    tess: Default::default(),
                });
            }
        }
    }

    /// Frame.Style ChatBlue / ChatGreen / ChatRed: textures/ui/dialog_*.png
    /// (40 px, 10 px corners) 9-sliced over the frame (offline Studio: the image
    /// fills the frame's box exactly, its 6 px bar on the left).
    pub(super) fn chat_frame(&mut self, style: u32, rect: Rect, me: &Down) {
        let file = match style {
            4 => "rbxasset://textures/ui/dialog_green.png",
            5 => "rbxasset://textures/ui/dialog_red.png",
            _ => "rbxasset://textures/ui/dialog_blue.png",
        };
        let Some((tex, size)) = self.gui.texture(self.ctx, self.input.store, self.input.doc_dir, file) else {
            return;
        };
        let (iw, ih) = (size[0] as f32, size[1] as f32);
        let c = 10.0f32.min(rect.width() * 0.5).min(rect.height() * 0.5);
        let a = (me.alpha * 255.0) as u8;
        let col = Color32::from_rgba_unmultiplied((me.tint[0] * 255.0) as u8, (me.tint[1] * 255.0) as u8, (me.tint[2] * 255.0) as u8, a);
        let xs = [rect.min.x, rect.min.x + c, rect.max.x - c, rect.max.x];
        let ys = [rect.min.y, rect.min.y + c, rect.max.y - c, rect.max.y];
        let us = [0.0, 10.0 / iw, 1.0 - 10.0 / iw, 1.0];
        let vs = [0.0, 10.0 / ih, 1.0 - 10.0 / ih, 1.0];
        let mut mesh = egui::epaint::Mesh::with_texture(tex);
        for j in 0..3 {
            for i in 0..3 {
                let r = Rect::from_min_max(egui::pos2(xs[i], ys[j]), egui::pos2(xs[i + 1], ys[j + 1]));
                let uv = Rect::from_min_max(egui::pos2(us[i], vs[j]), egui::pos2(us[i + 1], vs[j + 1]));
                mesh.add_rect_with_uv(r, uv, col);
            }
        }
        self.push_item(Shape::Mesh(mesh.into()), me);
    }

    fn push_item(&mut self, shape: Shape, dn: &Down) {
        self.seq += 1;
        self.items.push(Item { shape, clip: dn.clip, aff: dn.aff, z: dn.z, seq: self.seq, by: self.cur, tess: Default::default() });
    }
}

/// A shape's colours times `a` (premultiplied) and a tint.
fn fade(s: &mut Shape, a: f32, tint: [f32; 3]) {
    let f = |c: Color32| {
        Color32::from_rgba_premultiplied(
            (c.r() as f32 * a * tint[0]) as u8,
            (c.g() as f32 * a * tint[1]) as u8,
            (c.b() as f32 * a * tint[2]) as u8,
            (c.a() as f32 * a) as u8,
        )
    };
    match s {
        Shape::Mesh(m) => {
            let m = std::sync::Arc::make_mut(m);
            for v in &mut m.vertices {
                v.color = f(v.color);
            }
        }
        Shape::Text(t) => t.opacity_factor *= a,
        Shape::Vec(v) => v.iter_mut().for_each(|s| fade(s, a, tint)),
        Shape::Path(p) => {
            p.fill = f(p.fill);
            p.stroke.color = egui::epaint::ColorMode::Solid(match p.stroke.color {
                egui::epaint::ColorMode::Solid(c) => f(c),
                _ => Color32::TRANSPARENT,
            });
        }
        _ => {}
    }
}
