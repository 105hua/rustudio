//! Self-test step for Neon bloom (round 19, phases 114–115): an orange
//! Neon ball over black, read back and judged on its glow's radial profile.
//! The glow must fall off smoothly (no rings: no rise, no kinks), evenly all
//! round (no petals), in the part's hue, and as Studio's does (offline Studio
//! Studio, tools/fidelity bloom: Ball4, the same edge rule).

use glam::Vec3;
use rbx_dom_weak::types::{Color3, Enum, Ref, Variant};
use rbx_dom_weak::InstanceBuilder;

use super::{R6Test, SelfTest, Step};
use crate::app::Editor;
use crate::doc::{new_part, Document};

const CAM: Vec3 = Vec3::new(0.0, 50.0, 0.0);
const BALL: Vec3 = Vec3::new(0.0, 50.0, -30.0);
/// Studio's glow (max channel) this many pixels past the ball's edge
/// (pixels with a channel ≥ 250), 4-stud orange Neon ball 30 studs away.
const STUDIO_PROFILE: [(f32, f32); 7] = [
    (2.0, 219.1),
    (4.0, 194.2),
    (6.0, 162.7),
    (9.0, 111.8),
    (12.0, 70.8),
    (16.0, 32.1),
    (20.0, 11.3),
];
/// The view size of that capture (checks/compare/bloom/studio). Studio's
/// glow is a pyramid whose depth follows the view's size, so
/// the step renders the view at exactly this size.
const STUDIO_VIEW: [u32; 2] = [863, 675];
/// Studio's halo green / red six pixels out (the orange keeps its hue).
const STUDIO_GREEN_SHARE: f32 = 0.77;
const PROFILE_TOLERANCE: f32 = 14.0;
/// Studio's own profile: steps never rise, kinks under 3 levels.
const MAX_RISE: f32 = 1.0;
const MAX_KINK: f32 = 6.0;
/// Sectors differ by the ball's facets (about a pixel of edge): petals
/// are judged as that difference over the ring's slope, in pixels.
const MAX_PETALS: f32 = 2.5;

fn build(ed: &mut Editor) {
    ed.set_document(Document::new_place());
    ed.show_grid = false;
    let root = ed.doc.dom.root_ref();
    let ws = ed.doc.workspace().expect("workspace");
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
    let black = Variant::Color3(Color3::new(0.0, 0.0, 0.0));
    for (name, v) in [
        ("Brightness", Variant::Float32(0.0)),
        ("ClockTime", Variant::Float32(0.0)),
        ("Ambient", black.clone()),
        ("OutdoorAmbient", black),
        ("EnvironmentDiffuseScale", Variant::Float32(0.0)),
        ("EnvironmentSpecularScale", Variant::Float32(0.0)),
        ("ExposureCompensation", Variant::Float32(0.0)),
        ("GlobalShadows", Variant::Bool(false)),
        ("FogEnd", Variant::Float32(100_000.0)),
    ] {
        if let Some(i) = ed.doc.dom.get_by_ref_mut(lighting) {
            i.properties.insert(rbx_dom_weak::ustr(name), v);
        }
    }
    ed.doc.insert(
        ws,
        new_part("Ball", 0, BALL, Vec3::splat(4.0), [255, 170, 0])
            .with_property("Material", Enum::from_u32(288))
            .with_property("CastShadow", false),
    );
    ed.doc.insert(
        ws,
        new_part(
            "Backdrop",
            1,
            CAM + Vec3::new(0.0, 0.0, -150.0),
            Vec3::new(600.0, 600.0, 2.0),
            [0, 0, 0],
        )
        .with_property("Material", Enum::from_u32(272)),
    );
    place_camera(ed);
}

/// The calibration view (held every frame: an earlier step may leave the
/// camera orbiting or a key that flies it).
fn place_camera(ed: &mut Editor) {
    ed.view_render_size = Some(STUDIO_VIEW);
    ed.cam.mode = crate::camera::CameraMode::Fly;
    ed.cam.basis = None;
    ed.cam.pos = CAM;
    ed.cam.yaw = 0.0;
    ed.cam.pitch = 0.0;
    ed.cam.fov_deg = 70.0;
    ed.cam.focus = BALL;
}

/// The glow's rings around `c` (pixels): per one-pixel ring past the edge,
/// the mean max channel, and the mean red and green.
struct Profile {
    radius: f32,
    rings: Vec<(f32, f32, f32)>,
    /// Largest difference between 16 sectors' means on one ring (3..20 px
    /// out) over the profile's slope there: how far out of round, in pixels.
    petals: f32,
}

fn profile(frame: &(u32, u32, Vec<u8>), c: (f32, f32)) -> Profile {
    let (w, h, px) = frame;
    let at = |x: i64, y: i64| -> Option<[f32; 3]> {
        if x < 0 || y < 0 || x >= *w as i64 || y >= *h as i64 {
            return None;
        }
        let o = ((y as u32 * w + x as u32) * 4) as usize;
        Some([px[o] as f32, px[o + 1] as f32, px[o + 2] as f32])
    };
    let mut lit = 0usize;
    let span = 80i64;
    for y in -span..=span {
        for x in -span..=span {
            if at(c.0 as i64 + x, c.1 as i64 + y).is_some_and(|p| p.iter().cloned().fold(0.0, f32::max) >= 250.0) {
                lit += 1;
            }
        }
    }
    let radius = (lit as f32 / std::f32::consts::PI).sqrt();
    const RINGS: usize = 30;
    const SECTORS: usize = 16;
    let mut acc = vec![[0f32; 4]; RINGS];
    let mut sec = vec![[0f32; 2 * SECTORS]; RINGS];
    let reach = (radius + RINGS as f32 + 2.0) as i64;
    for y in -reach..=reach {
        for x in -reach..=reach {
            let (fx, fy) = (c.0.floor() + x as f32 + 0.5 - c.0, c.1.floor() + y as f32 + 0.5 - c.1);
            let d = (fx * fx + fy * fy).sqrt() - radius;
            if d < 0.5 {
                continue;
            }
            let k = (d - 0.5).floor() as usize;
            if k >= RINGS {
                continue;
            }
            let Some(p) = at(c.0 as i64 + x, c.1 as i64 + y) else {
                continue;
            };
            let m = p[0].max(p[1]).max(p[2]);
            acc[k][0] += m;
            acc[k][1] += p[0];
            acc[k][2] += p[1];
            acc[k][3] += 1.0;
            let s = (((fy.atan2(fx) + std::f32::consts::PI) / std::f32::consts::TAU * SECTORS as f32) as usize).min(SECTORS - 1);
            sec[k][2 * s] += m;
            sec[k][2 * s + 1] += 1.0;
        }
    }
    let rings: Vec<(f32, f32, f32)> = acc
        .iter()
        .map(|a| {
            let n = a[3].max(1.0);
            (a[0] / n, a[1] / n, a[2] / n)
        })
        .collect();
    let mut petals = 0f32;
    for k in 3..20 {
        let s = &sec[k];
        let means: Vec<f32> = (0..SECTORS)
            .filter(|i| s[2 * i + 1] > 0.0)
            .map(|i| s[2 * i] / s[2 * i + 1])
            .collect();
        let slope = ((rings[k - 1].0 - rings[k + 1].0) / 2.0).abs().max(2.0);
        if let (Some(lo), Some(hi)) = (
            means.iter().cloned().reduce(f32::min),
            means.iter().cloned().reduce(f32::max),
        ) {
            petals = petals.max((hi - lo) / slope);
        }
    }
    Profile { radius, rings, petals }
}

impl SelfTest {
    /// Phase 114 builds the scene, 115 waits, reads the view and judges it.
    pub(super) fn tick_bloom(&mut self, f: u64, ed: &mut Editor) -> bool {
        if self.r4.phase == 114 {
            build(ed);
            self.r6 = R6Test {
                started: f,
                ..Default::default()
            };
            self.r4.phase = 115;
            return false;
        }
        if f.saturating_sub(self.r6.started) < 30 {
            place_camera(ed);
            return false;
        }
        let frame = ed
            .renderer
            .as_ref()
            .zip(ed.render_state.as_ref())
            .and_then(|(r, rs)| r.read_frame(rs));
        if let (Some(dir), Some((w, h, px))) = (std::env::var_os("RUSTUDIO_LIGHT_DUMP"), &frame) {
            let _ = image::save_buffer(
                std::path::Path::new(&dir).join("bloom-smooth.png"),
                px,
                *w,
                *h,
                image::ExtendedColorType::Rgba8,
            );
        }
        // (the view was rendered at STUDIO_VIEW pixels: project onto that)
        let rect = egui::Rect::from_min_size(
            ed.viewport_rect.min,
            egui::vec2(STUDIO_VIEW[0] as f32, STUDIO_VIEW[1] as f32),
        );
        let mut notes = Vec::new();
        let mut ok = false;
        match (&frame, ed.cam.project(rect, BALL)) {
            (Some(frame), Some(sp)) => {
                let scale = frame.0 as f32 / rect.width().max(1.0);
                let c = ((sp.x - rect.min.x) * scale, (sp.y - rect.min.y) * scale);
                let p = profile(frame, c);
                let m: Vec<f32> = p.rings.iter().map(|r| r.0).collect();
                // Rings start one pixel past the edge (ring k is k+1 px out).
                let rise = m.windows(2).map(|w| w[1] - w[0]).fold(f32::MIN, f32::max);
                let kink = m
                    .windows(3)
                    .map(|w| (w[0] - 2.0 * w[1] + w[2]).abs())
                    .fold(0.0, f32::max);
                let mut worst = 0f32;
                let mut vs = Vec::new();
                for (d, want) in STUDIO_PROFILE {
                    let got = m.get(d as usize - 1).copied().unwrap_or(0.0);
                    worst = worst.max((got - want).abs());
                    vs.push(format!("+{d}: {got:.0} vs {want:.0}"));
                }
                let six = p.rings.get(5).copied().unwrap_or_default();
                let green = if six.1 > 1.0 { six.2 / six.1 } else { 0.0 };
                ok = p.radius > 10.0
                    && rise <= MAX_RISE
                    && kink <= MAX_KINK
                    && p.petals <= MAX_PETALS
                    && worst <= PROFILE_TOLERANCE
                    && (green - STUDIO_GREEN_SHARE).abs() <= 0.08;
                notes.push(format!(
                    "ball radius {:.1} px in a {}x{} view; largest rise {rise:.1} (allowed {MAX_RISE}), kink {kink:.1} ({MAX_KINK}), petals {:.1} px ({MAX_PETALS}); Studio's profile {} (largest difference {worst:.1}, allowed {PROFILE_TOLERANCE}); halo green / red {green:.2} vs Studio {STUDIO_GREEN_SHARE}",
                    p.radius,
                    frame.0,
                    frame.1,
                    p.petals,
                    vs.join(", ")
                ));
            }
            _ => notes.push("couldn't read the viewport back or project the ball".into()),
        }
        self.steps.push(Step {
            step: "bloom-smooth",
            passed: ok,
            detail: format!(
                "an orange Neon ball (4 studs, 30 studs away) over black at night: its glow falls off smoothly, evenly and in its own hue, as in Studio: {}",
                notes.join("; ")
            ),
        });
        ed.show_grid = true;
        ed.view_render_size = None;
        ed.new_place();
        self.r4.phase = 116;
        false
    }
}
