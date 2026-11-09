//! The Terrain Editor (round 21): Studio's View > Terrain Editor panel.
//! Create: Generate, Clear. Edit: Draw (Add / Subtract), Sculpt (Grow /
//! Erode), Smooth, Flatten, Paint, Sea Level, Replace. Brush tools work in
//! the 3D view while the left button is held (Ctrl inverts Draw and
//! Sculpt); a stroke is one undo step, and only the chunks it touched are
//! re-meshed while it runs (the grid is written back when it ends).
//! Region (round 25): Select, Move, Resize, Rotate, Fill on a box snapped
//! to the voxel grid, with Copy, Cut, Paste, Duplicate and Delete; each
//! change to the terrain is one undo step.

use egui::{Color32, Pos2, Rect, RichText, Stroke};
use glam::Vec3;
use rbx_dom_weak::types::{BinaryString, Ref, Variant};
use rbx_dom_weak::ustr;

use crate::app::Editor;
use crate::terrain::{self, Voxels, VOXEL, WATER};
use crate::terrain_region::{self as region, Region, Rot};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Generate,
    Clear,
    Draw,
    Sculpt,
    Smooth,
    Flatten,
    Paint,
    SeaLevel,
    Replace,
    Select,
    Move,
    Resize,
    Rotate,
    Fill,
}

impl Tool {
    const CREATE: [(Tool, &'static str); 2] = [(Tool::Generate, "Generate"), (Tool::Clear, "Clear")];
    const EDIT: [(Tool, &'static str); 7] = [
        (Tool::Draw, "Draw"),
        (Tool::Sculpt, "Sculpt"),
        (Tool::Smooth, "Smooth"),
        (Tool::Flatten, "Flatten"),
        (Tool::Paint, "Paint"),
        (Tool::SeaLevel, "Sea Level"),
        (Tool::Replace, "Replace"),
    ];
    const REGION: [(Tool, &'static str); 5] = [
        (Tool::Select, "Select"),
        (Tool::Move, "Move"),
        (Tool::Resize, "Resize"),
        (Tool::Rotate, "Rotate"),
        (Tool::Fill, "Fill"),
    ];

    /// Used with the brush in the 3D view.
    pub fn is_brush(self) -> bool {
        matches!(self, Tool::Draw | Tool::Sculpt | Tool::Smooth | Tool::Flatten | Tool::Paint)
    }

    /// Works on the selected region box.
    pub fn is_region(self) -> bool {
        matches!(self, Tool::Select | Tool::Move | Tool::Resize | Tool::Rotate | Tool::Fill)
    }

    /// Changes the terrain in the box as the box changes.
    fn transforms(self) -> bool {
        matches!(self, Tool::Move | Tool::Resize | Tool::Rotate)
    }
}

/// A region of whole voxels: its first voxel and size in voxels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RegionBox {
    pub lo: [i32; 3],
    pub size: [i32; 3],
}

impl RegionBox {
    /// The voxels a box of studs (centre, size) covers, snapped outward
    /// to the grid as Studio's getVoxelAlignedRegion does.
    pub fn from_studs(pos: [f32; 3], size: [f32; 3]) -> RegionBox {
        let lo = [0, 1, 2].map(|k| ((pos[k] - size[k].abs() * 0.5) / VOXEL).floor() as i32);
        let hi = [0, 1, 2].map(|k| ((pos[k] + size[k].abs() * 0.5) / VOXEL).ceil() as i32);
        RegionBox { lo, size: [0, 1, 2].map(|k| (hi[k] - lo[k]).max(1)) }
    }

    pub fn min(&self) -> Vec3 {
        Vec3::from(self.lo.map(|c| c as f32)) * VOXEL
    }

    pub fn max(&self) -> Vec3 {
        Vec3::from([0, 1, 2].map(|k| (self.lo[k] + self.size[k]) as f32)) * VOXEL
    }

    pub fn center(&self) -> Vec3 {
        (self.min() + self.max()) * 0.5
    }

    /// The box turned by `r` about its centre (rounded down to a voxel
    /// when the new size and the old one differ by an odd count).
    fn rotated(&self, r: &Rot) -> RegionBox {
        let size = region::rotated_size(r, self.size);
        let lo = [0, 1, 2].map(|k| (2 * self.lo[k] + self.size[k] - size[k]).div_euclid(2));
        RegionBox { lo, size }
    }
}

/// Fill's two modes: the whole box takes the material, or only one
/// material in it is swapped for another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillMode {
    Fill,
    Replace,
}

/// A Move / Resize / Rotate in progress: the terrain as it was when the
/// box was lifted and what the box held, placed again (turned and
/// stretched to the box) every time the box changes. Ends when anything
/// else changes the document (undo included): the next change lifts what
/// the box holds then.
struct Lift {
    base: Option<Variant>,
    content: Region,
    /// Where it came from (cleared when placing); None for Paste and
    /// Duplicate, which leave the source alone.
    from: Option<RegionBox>,
    rot: Rot,
    revision: u64,
    /// `base` decoded, once a drag first previews (round 26).
    base_grid: Option<Voxels>,
    /// The box and turn the drawn terrain shows the content at (a live
    /// preview while dragging, or the last placement); the next preview
    /// restores those voxels from `base_grid` first.
    preview: Option<(RegionBox, Rot)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RegionHandle {
    /// A face (axis, ±1): moves the box, or moves that face.
    Face(usize, i32),
    /// The ring about an axis.
    Ring(usize),
}

/// A drag in the 3D view with a Region tool.
struct RegionDrag {
    /// None: a new box from where the drag started.
    handle: Option<RegionHandle>,
    start: RegionBox,
    origin: Vec3,
    start_param: f32,
    center_px: Pos2,
    last_angle: f32,
    turned: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Sphere,
    Cube,
    Cylinder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlattenMode {
    Erode,
    Both,
    Grow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Plains,
    Hills,
    Mountains,
    Dunes,
}

/// A brush stroke in progress: the grid before it (for undo) and the plane
/// Flatten keeps.
struct Stroke_ {
    before: Option<Variant>,
    plane_y: f32,
    last: std::time::Instant,
    applied: bool,
}

pub struct TerrainEditor {
    pub tool: Option<Tool>,
    pub size: f32,
    pub strength: f32,
    pub shape: Shape,
    pub material: u8,
    /// Draw: Subtract instead of Add. Sculpt: Erode instead of Grow.
    pub invert: bool,
    pub flatten: FlattenMode,
    pub ignore_water: bool,
    /// Sea Level / Replace / Generate region: centre and size (studs).
    pub region_pos: [f32; 3],
    pub region_size: [f32; 3],
    pub replace_from: u8,
    pub replace_to: u8,
    pub seed: u32,
    pub biome: Biome,
    pub gen_water: bool,
    /// The brush's centre and the surface normal there (last frame).
    pub hover: Option<(Vec3, Vec3)>,
    stroke: Option<Stroke_>,
    /// Region tools: the selected box, Merge Empty (the box's empty
    /// voxels clear what they land on), Fill's mode, the copied region
    /// (with the box it came from) and the Move / Resize / Rotate going on.
    pub selection: Option<RegionBox>,
    pub merge_empty: bool,
    pub fill_mode: FillMode,
    pub clipboard: Option<(Region, RegionBox)>,
    lift: Option<Lift>,
    region_drag: Option<RegionDrag>,
}

impl Default for TerrainEditor {
    fn default() -> Self {
        TerrainEditor {
            tool: None,
            size: 12.0,
            strength: 0.5,
            shape: Shape::Sphere,
            material: 2,
            invert: false,
            flatten: FlattenMode::Both,
            ignore_water: true,
            region_pos: [0.0, 0.0, 0.0],
            region_size: [512.0, 128.0, 512.0],
            replace_from: 2,
            replace_to: 6,
            seed: 618033988,
            biome: Biome::Hills,
            gen_water: true,
            hover: None,
            stroke: None,
            selection: None,
            merge_empty: false,
            fill_mode: FillMode::Fill,
            clipboard: None,
            lift: None,
            region_drag: None,
        }
    }
}

/// The materials a brush can lay (Air left out; Water only for Draw).
const PICKABLE: [u8; 22] = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 1];

fn swatch(m: u8) -> Color32 {
    let c = terrain::material_color(m);
    Color32::from_rgb(c[0], c[1], c[2])
}

// ------------------------------------------------------------ voxel ops

/// Signed distance (studs, negative inside) from `p` to the brush shape
/// centred at `c` with radius `r`.
fn shape_distance(shape: Shape, p: Vec3, c: Vec3, r: f32) -> f32 {
    let d = p - c;
    match shape {
        Shape::Sphere => d.length() - r,
        Shape::Cube => (d.abs() - Vec3::splat(r)).max_element(),
        Shape::Cylinder => ((d.x * d.x + d.z * d.z).sqrt() - r).max(d.y.abs() - r),
    }
}

fn voxel_center(v: [i32; 3]) -> Vec3 {
    Vec3::new(v[0] as f32 + 0.5, v[1] as f32 + 0.5, v[2] as f32 + 0.5) * VOXEL
}

/// The voxel box around a brush (one voxel of margin).
fn brush_box(c: Vec3, r: f32) -> ([i32; 3], [i32; 3]) {
    let lo = ((c - Vec3::splat(r + VOXEL)) / VOXEL).floor();
    let hi = ((c + Vec3::splat(r + VOXEL)) / VOXEL).floor();
    (
        [lo.x as i32, lo.y as i32, lo.z as i32],
        [hi.x as i32, hi.y as i32, hi.z as i32],
    )
}

fn solid(m: u8, ignore_water: bool) -> bool {
    m != 0 && !(ignore_water && m == WATER)
}

/// The most common solid material around a voxel (Sculpt's Grow lays it).
fn neighbour_material(v: &Voxels, p: [i32; 3], fallback: u8) -> u8 {
    let mut count = [0u32; 23];
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (m, o) = v.get(p[0] + dx, p[1] + dy, p[2] + dz);
                if m != 0 && m != WATER && (m as usize) < 23 {
                    count[m as usize] += (o * 16.0) as u32 + 1;
                }
            }
        }
    }
    (2..23u8).max_by_key(|m| count[*m as usize]).filter(|m| count[*m as usize] > 0).unwrap_or(fallback)
}

/// One application of a brush tool at `c`: the voxel box it touched.
pub fn apply_brush(ed: &TerrainEditor, v: &mut Voxels, tool: Tool, c: Vec3, plane_y: f32) -> ([i32; 3], [i32; 3]) {
    let r = ed.size * 0.5;
    let s = ed.strength.clamp(0.0, 1.0);
    let (lo, hi) = brush_box(c, r);
    let mut writes: Vec<([i32; 3], u8, f32)> = Vec::new();
    for y in lo[1]..=hi[1] {
        for z in lo[2]..=hi[2] {
            for x in lo[0]..=hi[0] {
                let p = [x, y, z];
                let pc = voxel_center(p);
                // Share of the voxel inside the brush, as Terrain:Fill* does.
                let dist = shape_distance(ed.shape, pc, c, r);
                let share = (0.5 - dist / VOXEL).clamp(0.0, 1.0);
                if share <= 0.0 {
                    continue;
                }
                // Softer towards the edge for the sculpting tools.
                let falloff = (1.0 - ((pc - c).length() / r.max(1e-3))).clamp(0.0, 1.0);
                let (m, o) = v.get(x, y, z);
                let here = solid(m, ed.ignore_water);
                let w = match tool {
                    Tool::Draw if !ed.invert => {
                        let target = o + (share - o).max(0.0) * s.max(0.25);
                        if target <= o + 1e-4 {
                            continue;
                        }
                        let mat = if here { m } else { ed.material };
                        (p, mat, target)
                    }
                    Tool::Draw => {
                        if !here {
                            continue;
                        }
                        // Toward Terrain:Fill* with Air: at most 1 − share left.
                        let left = o - (o - o.min(1.0 - share)) * s.max(0.25);
                        (p, m, if left < 1.0 / 256.0 { 0.0 } else { left })
                    }
                    Tool::Sculpt => {
                        let amount = s * 0.35 * falloff * share;
                        if amount <= 0.0 {
                            continue;
                        }
                        if !ed.invert {
                            // Grow: only next to what is already there.
                            let mat = if here { m } else { neighbour_material(v, p, 0) };
                            if mat == 0 {
                                continue;
                            }
                            (p, mat, (o + amount).min(1.0))
                        } else {
                            if !here {
                                continue;
                            }
                            let left = o - amount;
                            (p, m, if left < 1.0 / 256.0 { 0.0 } else { left })
                        }
                    }
                    Tool::Smooth => {
                        let mut sum = 0.0;
                        for d in [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]] {
                            let (nm, no) = v.get(x + d[0], y + d[1], z + d[2]);
                            if solid(nm, ed.ignore_water) {
                                sum += no;
                            }
                        }
                        let own = if here { o } else { 0.0 };
                        let avg = (sum + own * 2.0) / 8.0;
                        let target = own + (avg - own) * s * falloff;
                        if (target - own).abs() < 1.0 / 512.0 {
                            continue;
                        }
                        let mat = if here { m } else { neighbour_material(v, p, 0) };
                        if mat == 0 {
                            continue;
                        }
                        (p, mat, if target < 1.0 / 256.0 { 0.0 } else { target })
                    }
                    Tool::Flatten => {
                        // The voxel's share under the plane.
                        let under = (0.5 - (pc.y - plane_y) / VOXEL).clamp(0.0, 1.0);
                        let own = if here { o } else { 0.0 };
                        let wanted = match ed.flatten {
                            FlattenMode::Erode => own.min(under),
                            FlattenMode::Grow => own.max(under),
                            FlattenMode::Both => under,
                        };
                        let target = own + (wanted - own) * s.max(0.25) * share;
                        if (target - own).abs() < 1.0 / 512.0 {
                            continue;
                        }
                        let mat = if here { m } else { neighbour_material(v, p, ed.material) };
                        (p, mat, if target < 1.0 / 256.0 { 0.0 } else { target })
                    }
                    Tool::Paint => {
                        if !here || m == ed.material || ed.material == WATER {
                            continue;
                        }
                        (p, ed.material, o)
                    }
                    _ => continue,
                };
                writes.push(w);
            }
        }
    }
    v.edit(writes);
    (lo, hi)
}

/// Sea Level: water fills the region's air (or, `evaporate`, leaves it).
pub fn sea_level(v: &mut Voxels, pos: [f32; 3], size: [f32; 3], evaporate: bool) {
    let (lo, hi) = region_box(pos, size);
    let mut writes = Vec::new();
    for y in lo[1]..=hi[1] {
        for z in lo[2]..=hi[2] {
            for x in lo[0]..=hi[0] {
                let (m, _) = v.get(x, y, z);
                if evaporate && m == WATER {
                    writes.push(([x, y, z], 0, 0.0));
                } else if !evaporate && m == 0 {
                    writes.push(([x, y, z], WATER, 1.0));
                }
            }
        }
    }
    v.edit(writes);
}

/// The voxels whose centres lie in a region (centre, size in studs).
fn region_box(pos: [f32; 3], size: [f32; 3]) -> ([i32; 3], [i32; 3]) {
    let lo = [0, 1, 2].map(|k| ((pos[k] - size[k] * 0.5) / VOXEL).round() as i32);
    let hi = [0, 1, 2].map(|k| ((pos[k] + size[k] * 0.5) / VOXEL).round() as i32 - 1);
    (lo, hi)
}

/// Smooth 2D value noise in 0..1 (cell size 1), for Generate.
fn value_noise(seed: u32, x: f32, z: f32) -> f32 {
    let hash = |ix: i32, iz: i32| -> f32 {
        let mut h = (ix as u32).wrapping_mul(0x9e37_79b1) ^ (iz as u32).wrapping_mul(0x85eb_ca77) ^ seed;
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        h = h.wrapping_mul(0x297a_2d39);
        h ^= h >> 15;
        h as f32 / u32::MAX as f32
    };
    let (fx, fz) = (x.floor(), z.floor());
    let (tx, tz) = (x - fx, z - fz);
    let (sx, sz) = (tx * tx * (3.0 - 2.0 * tx), tz * tz * (3.0 - 2.0 * tz));
    let (ix, iz) = (fx as i32, fz as i32);
    let a = hash(ix, iz) + (hash(ix + 1, iz) - hash(ix, iz)) * sx;
    let b = hash(ix, iz + 1) + (hash(ix + 1, iz + 1) - hash(ix, iz + 1)) * sx;
    a + (b - a) * sz
}

fn fbm(seed: u32, x: f32, z: f32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..5u32 {
        sum += value_noise(seed.wrapping_add(o * 1013), x * f, z * f) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// Generate: rolling ground over the region from a seed: Grass on top,
/// Rock on steep ground, Sand by the water, Snow high up; water to a
/// third of the height when asked.
pub fn generate(v: &mut Voxels, pos: [f32; 3], size: [f32; 3], seed: u32, biome: Biome, water: bool) {
    let (lo, hi) = region_box(pos, size);
    let base_y = pos[1] - size[1] * 0.5;
    let (amp, scale, top) = match biome {
        Biome::Plains => (0.12, 220.0, 2u8),
        Biome::Hills => (0.45, 160.0, 2),
        Biome::Mountains => (0.95, 120.0, 8),
        Biome::Dunes => (0.3, 90.0, 6),
    };
    let sea = base_y + size[1] * 0.3;
    let height = |x: f32, z: f32| -> f32 {
        let n = fbm(seed, x / scale, z / scale);
        let n = if biome == Biome::Mountains { n * n * 1.6 } else { n };
        base_y + size[1] * (0.15 + amp * n * 0.8)
    };
    let mut writes = Vec::new();
    for z in lo[2]..=hi[2] {
        for x in lo[0]..=hi[0] {
            let (wx, wz) = ((x as f32 + 0.5) * VOXEL, (z as f32 + 0.5) * VOXEL);
            let h = height(wx, wz);
            let slope = ((height(wx + VOXEL, wz) - height(wx - VOXEL, wz)).powi(2)
                + (height(wx, wz + VOXEL) - height(wx, wz - VOXEL)).powi(2))
            .sqrt()
                / (2.0 * VOXEL);
            for y in lo[1]..=hi[1] {
                let cy = (y as f32 + 0.5) * VOXEL;
                let share = (0.5 - (cy - h) / VOXEL).clamp(0.0, 1.0);
                if share > 0.0 {
                    let deep = h - cy > 8.0;
                    let mat = if deep {
                        8
                    } else if slope > 0.9 {
                        8
                    } else if water && h < sea + 3.0 {
                        6
                    } else if biome == Biome::Mountains && h > base_y + size[1] * 0.7 {
                        10
                    } else {
                        top
                    };
                    writes.push(([x, y, z], mat, share));
                } else if water && cy < sea {
                    writes.push(([x, y, z], WATER, 1.0));
                }
            }
        }
    }
    v.edit(writes);
}

/// First solid point along a ray through the voxels (half-stud steps),
/// with the occupancy gradient's normal there.
pub fn ray_voxels(v: &Voxels, origin: Vec3, dir: Vec3, max: f32, ignore_water: bool) -> Option<(Vec3, Vec3)> {
    let dir = dir.normalize_or_zero();
    let occ = |p: Vec3| -> f32 {
        let q = (p / VOXEL).floor();
        let (m, o) = v.get(q.x as i32, q.y as i32, q.z as i32);
        if solid(m, ignore_water) {
            o
        } else {
            0.0
        }
    };
    let mut t = 0.0;
    while t < max {
        let p = origin + dir * t;
        if occ(p) > 0.0 {
            let e = VOXEL;
            let g = Vec3::new(
                occ(p - Vec3::X * e) - occ(p + Vec3::X * e),
                occ(p - Vec3::Y * e) - occ(p + Vec3::Y * e),
                occ(p - Vec3::Z * e) - occ(p + Vec3::Z * e),
            );
            let n = if g.length_squared() > 1e-6 { g.normalize() } else { -dir };
            return Some((p, n));
        }
        t += 0.5;
    }
    None
}

// ------------------------------------------------------------ panel parts

fn material_picker(ui: &mut egui::Ui, label: &str, m: &mut u8, water: bool) {
    ui.label(format!("{label}: {}", terrain::material_name(*m)));
    ui.horizontal_wrapped(|ui| {
        for p in PICKABLE {
            if p == WATER && !water {
                continue;
            }
            let (rect, r) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
            ui.painter().rect_filled(rect.shrink(1.0), 0.0, swatch(p));
            if *m == p {
                ui.painter().rect_stroke(rect, 0.0, Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
            }
            if r.on_hover_text(terrain::material_name(p)).clicked() {
                *m = p;
            }
        }
    });
}

/// The Region tools' settings: the box (centre and size in studs, snapped
/// to the voxel grid), the clipboard row, Merge Empty, Rotate's quarter
/// turns and Fill. Returns the button pressed.
fn region_panel(ui: &mut egui::Ui, te: &mut TerrainEditor, tool: Tool) -> Option<&'static str> {
    let mut action = None;
    let has = te.selection.is_some();
    egui::Grid::new("terrain_region_settings").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        if let Some(b) = te.selection {
            let (mut pos, mut size) = (b.center().to_array(), (b.max() - b.min()).to_array());
            let mut changed = false;
            for (label, vals) in [("Position", &mut pos), ("Size", &mut size)] {
                ui.label(label);
                ui.horizontal(|ui| {
                    for (k, v) in vals.iter_mut().enumerate() {
                        ui.label(["X", "Y", "Z"][k]);
                        changed |= ui.add(egui::DragValue::new(v).speed(VOXEL).max_decimals(1)).changed();
                    }
                });
                ui.end_row();
            }
            if changed {
                // Whole voxels: the size first, then the centre placed on it.
                let n = size.map(|s| ((s / VOXEL).round() as i32).max(1));
                let lo = [0, 1, 2].map(|k| ((pos[k] - n[k] as f32 * VOXEL * 0.5) / VOXEL).round() as i32);
                te.selection = Some(RegionBox { lo, size: n });
            }
        }
        if tool.transforms() || tool == Tool::Select {
            ui.label("Merge Empty");
            ui.checkbox(&mut te.merge_empty, "")
                .on_hover_text("Empty voxels in the region clear the terrain they land on");
            ui.end_row();
        }
        if tool == Tool::Fill {
            ui.label("Mode");
            ui.horizontal(|ui| {
                ui.radio_value(&mut te.fill_mode, FillMode::Fill, "Fill");
                ui.radio_value(&mut te.fill_mode, FillMode::Replace, "Replace");
            });
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (name, a, enabled) in [
            ("Copy", "copy", has),
            ("Cut", "cut", has),
            ("Paste", "paste", te.clipboard.is_some()),
            ("Duplicate", "duplicate", has),
            ("Delete", "delete", has),
            ("Deselect", "deselect", has),
        ] {
            if ui.add_enabled(enabled, egui::Button::new(name)).clicked() {
                action = Some(a);
            }
        }
    });
    match tool {
        Tool::Rotate => {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Rotate 90°");
                for (n, a) in [("X", "rotate_x"), ("Y", "rotate_y"), ("Z", "rotate_z")] {
                    if ui.add_enabled(has, egui::Button::new(n)).clicked() {
                        action = Some(a);
                    }
                }
            });
        }
        Tool::Fill => {
            ui.add_space(4.0);
            if te.fill_mode == FillMode::Fill {
                material_picker(ui, "Material", &mut te.material, true);
            } else {
                material_picker(ui, "Source", &mut te.replace_from, true);
                material_picker(ui, "Target", &mut te.replace_to, true);
            }
            ui.add_space(4.0);
            let label = if te.fill_mode == FillMode::Fill { "Fill" } else { "Replace" };
            if ui.add_enabled(has, egui::Button::new(label)).clicked() {
                action = Some("fill");
            }
        }
        _ => {}
    }
    ui.add_space(6.0);
    let hint = match (tool, has) {
        (_, false) => "Drag in the 3D view with Select to pick a region.",
        (Tool::Select, true) => "Drag a face dot to resize the region, or drag elsewhere for a new one.",
        (Tool::Move, true) => "Drag an arrow to move the region and its terrain.",
        (Tool::Resize, true) => "Drag a face dot to stretch the region's terrain.",
        (Tool::Rotate, true) => "Drag a ring to turn the region's terrain in 90° steps.",
        _ => "Fills the selected region.",
    };
    ui.label(RichText::new(hint).color(theme::TEXT_DIM).small());
    action
}

/// The box's twelve edges, on screen.
fn box_edges(cam: &crate::camera::Camera, rect: Rect, b: &RegionBox) -> Vec<[Pos2; 2]> {
    let (lo, hi) = (b.min(), b.max());
    let corner = |i: usize| Vec3::new(if i & 1 != 0 { hi.x } else { lo.x }, if i & 2 != 0 { hi.y } else { lo.y }, if i & 4 != 0 { hi.z } else { lo.z });
    let mut out = Vec::new();
    for i in 0..8usize {
        for bit in [1usize, 2, 4] {
            if i & bit == 0 {
                if let (Some(a), Some(c)) = (cam.project(rect, corner(i)), cam.project(rect, corner(i | bit))) {
                    out.push([a, c]);
                }
            }
        }
    }
    out
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

// ------------------------------------------------------------ editor glue

impl Editor {
    fn terrain_ref(&self) -> Option<Ref> {
        self.doc.workspace().and_then(|ws| self.doc.find_child(ws, "Terrain"))
    }

    /// The voxels as the document holds them now.
    fn terrain_voxels(&self) -> Voxels {
        match Voxels::from_dom(&self.doc.dom) {
            Some(Ok(v)) => v,
            _ => Voxels::empty(),
        }
    }

    /// Writes a whole new grid as one undo step (Generate, Clear, Sea
    /// Level, Replace, the end of a brush stroke).
    fn commit_terrain(&mut self, label: &str, v: &Voxels) {
        let Some(t) = self.terrain_ref() else {
            self.warn("This place has no Workspace.Terrain.");
            return;
        };
        self.doc.set_props(
            label,
            vec![(t, ustr("SmoothGrid"), Variant::BinaryString(BinaryString::from(v.encode())))],
            None,
        );
    }

    fn terrain_editing_allowed(&self) -> bool {
        self.play.is_none() && self.doc.kind == crate::doc::DocKind::Place
    }

    /// The Terrain Editor panel.
    pub fn terrain_editor_ui(&mut self, ui: &mut egui::Ui) {
        let te = &mut self.terrain_ed;
        ui.add_space(6.0);
        let allowed = self.play.is_none() && self.doc.kind == crate::doc::DocKind::Place;
        if !allowed {
            ui.label(RichText::new("Stop the game to edit terrain.").color(theme::TEXT_DIM).small());
        }
        let mut action: Option<&'static str> = None;
        let sel_before = te.selection;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| ui.add_enabled_ui(allowed, |ui| {
            for (title, tools) in [("Create", &Tool::CREATE[..]), ("Edit", &Tool::EDIT[..]), ("Region", &Tool::REGION[..])] {
                ui.label(RichText::new(title).strong());
                ui.horizontal_wrapped(|ui| {
                    for (t, name) in tools {
                        let on = te.tool == Some(*t);
                        let r = ui.selectable_label(on, *name);
                        if r.clicked() {
                            te.tool = if on { None } else { Some(*t) };
                            te.invert = false;
                        }
                    }
                });
                ui.add_space(4.0);
            }
            ui.separator();
            let Some(tool) = te.tool else {
                ui.label(RichText::new("Pick a tool.").color(theme::TEXT_DIM).small());
                return;
            };
            if tool.is_region() {
                action = region_panel(ui, te, tool);
                return;
            }
            egui::Grid::new("terrain_editor_settings").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                if tool.is_brush() {
                    ui.label("Brush Size");
                    ui.add(egui::Slider::new(&mut te.size, 2.0..=64.0).suffix(" studs"));
                    ui.end_row();
                    ui.label("Strength");
                    ui.add(egui::Slider::new(&mut te.strength, 0.1..=1.0));
                    ui.end_row();
                    ui.label("Shape");
                    ui.horizontal_wrapped(|ui| {
                        for (s, n) in [(Shape::Sphere, "Sphere"), (Shape::Cube, "Cube"), (Shape::Cylinder, "Cylinder")] {
                            ui.radio_value(&mut te.shape, s, n);
                        }
                    });
                    ui.end_row();
                    ui.label("Ignore Water");
                    ui.checkbox(&mut te.ignore_water, "");
                    ui.end_row();
                }
                match tool {
                    Tool::Draw => {
                        ui.label("Mode");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut te.invert, false, "Add");
                            ui.radio_value(&mut te.invert, true, "Subtract");
                        });
                        ui.end_row();
                    }
                    Tool::Sculpt => {
                        ui.label("Mode");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut te.invert, false, "Grow");
                            ui.radio_value(&mut te.invert, true, "Erode");
                        });
                        ui.end_row();
                    }
                    Tool::Flatten => {
                        ui.label("Flatten Mode");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut te.flatten, FlattenMode::Erode, "Erode");
                            ui.radio_value(&mut te.flatten, FlattenMode::Both, "Both");
                            ui.radio_value(&mut te.flatten, FlattenMode::Grow, "Grow");
                        });
                        ui.end_row();
                    }
                    _ => {}
                }
                if matches!(tool, Tool::Generate | Tool::SeaLevel | Tool::Replace) {
                    for (label, vals) in [("Position", &mut te.region_pos), ("Size", &mut te.region_size)] {
                        ui.label(label);
                        ui.horizontal(|ui| {
                            for (k, v) in vals.iter_mut().enumerate() {
                                ui.label(["X", "Y", "Z"][k]);
                                ui.add(egui::DragValue::new(v).speed(4.0));
                            }
                        });
                        ui.end_row();
                    }
                }
                if tool == Tool::Generate {
                    ui.label("Biome");
                    ui.horizontal_wrapped(|ui| {
                        for (b, n) in [(Biome::Plains, "Plains"), (Biome::Hills, "Hills"), (Biome::Mountains, "Mountains"), (Biome::Dunes, "Dunes")] {
                            ui.radio_value(&mut te.biome, b, n);
                        }
                    });
                    ui.end_row();
                    ui.label("Water");
                    ui.checkbox(&mut te.gen_water, "");
                    ui.end_row();
                    ui.label("Seed");
                    ui.add(egui::DragValue::new(&mut te.seed));
                    ui.end_row();
                }
            });
            let picker = material_picker;
            match tool {
                Tool::Draw | Tool::Paint | Tool::Flatten => picker(ui, "Material", &mut te.material, tool == Tool::Draw),
                Tool::Replace => {
                    picker(ui, "Source", &mut te.replace_from, true);
                    picker(ui, "Target", &mut te.replace_to, true);
                }
                _ => {}
            }
            ui.add_space(6.0);
            match tool {
                Tool::Generate => {
                    if ui.button("Generate").clicked() {
                        action = Some("generate");
                    }
                }
                Tool::Clear => {
                    ui.label(RichText::new("Removes all terrain (undo brings it back).").color(theme::TEXT_DIM).small());
                    if ui.button("Clear").clicked() {
                        action = Some("clear");
                    }
                }
                Tool::SeaLevel => {
                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            action = Some("sea");
                        }
                        if ui.button("Evaporate").clicked() {
                            action = Some("evaporate");
                        }
                    });
                }
                Tool::Replace => {
                    if ui.button("Replace").clicked() {
                        action = Some("replace");
                    }
                }
                _ => {
                    ui.label(
                        RichText::new("Hold the left button in the 3D view; Ctrl inverts Draw and Sculpt.")
                            .color(theme::TEXT_DIM)
                            .small(),
                    );
                }
            }
        }));
        // A box typed in: Move and Resize carry the terrain with it.
        let sel_after = self.terrain_ed.selection;
        if sel_after != sel_before {
            match (self.terrain_ed.tool, sel_before, sel_after) {
                (Some(t @ (Tool::Move | Tool::Resize)), Some(old), Some(_)) => {
                    self.terrain_ed.selection = sel_before;
                    self.lift_region(old);
                    self.terrain_ed.selection = sel_after;
                    self.place_lift(if t == Tool::Move { "Terrain Move" } else { "Terrain Resize" });
                }
                _ => self.terrain_ed.lift = None,
            }
        }
        if let Some(a) = action {
            self.terrain_action(a);
        }
    }

    /// The panel's one-shot buttons.
    pub fn terrain_action(&mut self, a: &str) {
        if !self.terrain_editing_allowed() {
            return;
        }
        if self.terrain_region_action(a) {
            return;
        }
        let mut v = self.terrain_voxels();
        let te = &self.terrain_ed;
        let label = match a {
            "generate" => {
                generate(&mut v, te.region_pos, te.region_size, te.seed, te.biome, te.gen_water);
                "Generate Terrain"
            }
            "clear" => {
                v.clear();
                "Clear Terrain"
            }
            "sea" | "evaporate" => {
                sea_level(&mut v, te.region_pos, te.region_size, a == "evaporate");
                "Sea Level"
            }
            "replace" => {
                let (lo, hi) = region_box(te.region_pos, te.region_size);
                v.replace(lo, hi.map(|c| c + 1), te.replace_from, te.replace_to);
                "Replace Material"
            }
            _ => return,
        };
        self.commit_terrain(label, &v);
    }

    /// The brush in the 3D view: under the mouse, and painting while the
    /// left button is held. True while it has the left button (no picking).
    pub fn terrain_brush_ui(
        &mut self,
        ctx: &egui::Context,
        rect: Rect,
        resp: &egui::Response,
        pointer: Option<Pos2>,
        painter: &egui::Painter,
    ) -> bool {
        if self.terrain_ed.tool.is_some_and(Tool::is_region) && self.terrain_editing_allowed() {
            self.terrain_ed.hover = None;
            return self.terrain_region_view(ctx, rect, resp, pointer, painter);
        }
        let tool = match self.terrain_ed.tool {
            Some(t) if t.is_brush() && self.terrain_editing_allowed() => t,
            _ => {
                self.terrain_ed.hover = None;
                return false;
            }
        };
        let ctrl = ctx.input(|i| i.modifiers.command);
        let down = ctx.input(|i| i.pointer.primary_down());
        // The brush centre: the voxels under the mouse, else the y = 0 plane.
        self.terrain_ed.hover = pointer.filter(|p| rect.contains(*p)).and_then(|p| {
            let ray = self.cam.ray(rect, p);
            let v = self.terrain_drawn.as_ref().map(|(_, _, v)| v);
            v.and_then(|v| ray_voxels(v, ray.origin, ray.dir, 4000.0, self.terrain_ed.ignore_water))
                .or_else(|| {
                    (ray.dir.y.abs() > 1e-4)
                        .then(|| -ray.origin.y / ray.dir.y)
                        .filter(|t| *t > 0.0)
                        .map(|t| (ray.origin + ray.dir * t, Vec3::Y))
                })
        });
        // Outline: a ring on the surface.
        if let Some((c, n)) = self.terrain_ed.hover {
            let r = self.terrain_ed.size * 0.5;
            let a = if n.y.abs() < 0.9 { Vec3::Y.cross(n).normalize() } else { Vec3::X };
            let b = n.cross(a).normalize();
            let pts: Vec<Pos2> = (0..=48)
                .filter_map(|i| {
                    let t = i as f32 / 48.0 * std::f32::consts::TAU;
                    self.cam.project(rect, c + (a * t.cos() + b * t.sin()) * r + n * 0.1)
                })
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(1.5, theme::ACCENT)));
        }
        let pressed = resp.drag_started_by(egui::PointerButton::Primary) || resp.clicked_by(egui::PointerButton::Primary)
            || (resp.is_pointer_button_down_on() && down && self.terrain_ed.stroke.is_none());
        if pressed && self.terrain_ed.stroke.is_none() && resp.hovered() {
            let before = self.terrain_ref().and_then(|t| self.doc.prop(t, "SmoothGrid"));
            let plane_y = self.terrain_ed.hover.map(|h| h.0.y).unwrap_or(0.0);
            self.terrain_ed.stroke = Some(Stroke_ {
                before,
                plane_y,
                last: std::time::Instant::now() - std::time::Duration::from_secs(1),
                applied: false,
            });
        }
        let Some(stroke) = self.terrain_ed.stroke.as_mut() else {
            return resp.hovered();
        };
        if down {
            let due = stroke.last.elapsed().as_secs_f32() >= crate::mech::TERRAIN_BRUSH_INTERVAL_S;
            if let (true, Some((c, _))) = (due, self.terrain_ed.hover) {
                stroke.last = std::time::Instant::now();
                stroke.applied = true;
                let plane_y = stroke.plane_y;
                if let Some((id, r, mut v)) = self.terrain_drawn.take() {
                    let mut ed = std::mem::take(&mut self.terrain_ed);
                    let invert = ed.invert;
                    ed.invert = invert != ctrl;
                    let (lo, hi) = apply_brush(&ed, &mut v, tool, c, plane_y);
                    ed.invert = invert;
                    let water = ed.material == WATER || !ed.ignore_water;
                    self.terrain_ed = ed;
                    // Re-mesh just what it touched (sync_terrain, next frame).
                    self.terrain_brushed = Some((v, crate::terrain::dirty_box(lo, hi), water));
                    self.terrain_drawn = Some((id, r, Voxels::empty()));
                    self.scene.terrain_dirty = true;
                }
            }
            ctx.request_repaint();
            return true;
        }
        // Released: the stroke becomes one undo step.
        let stroke = self.terrain_ed.stroke.take().expect("stroke");
        if stroke.applied {
            if let (Some(t), Some((_, _, v))) = (self.terrain_ref(), self.terrain_drawn.as_ref()) {
                let after = Variant::BinaryString(BinaryString::from(v.encode()));
                if let Some(i) = self.doc.dom.get_by_ref_mut(t) {
                    i.properties.insert(ustr("SmoothGrid"), after.clone());
                }
                let name = match tool {
                    Tool::Draw => "Terrain Draw",
                    Tool::Sculpt => "Terrain Sculpt",
                    Tool::Smooth => "Terrain Smooth",
                    Tool::Flatten => "Terrain Flatten",
                    _ => "Terrain Paint",
                };
                self.doc.record_props(name, vec![(t, ustr("SmoothGrid"), stroke.before, after)]);
            }
        }
        true
    }
}

// ------------------------------------------------------------ region tools

/// The voxels in a SmoothGrid value (empty when there is none).
fn grid_of(v: Option<&Variant>) -> Voxels {
    let bytes: &[u8] = match v {
        Some(Variant::BinaryString(b)) => b.as_ref(),
        Some(Variant::SharedString(s)) => s.data(),
        _ => &[],
    };
    Voxels::decode(bytes).unwrap_or_else(|_| Voxels::empty())
}

/// Places lifted terrain: `from` cleared (a Move), `content` turned by
/// `rot` and stretched to `to`, its empty voxels clearing what is under
/// them only with `merge_empty`.
pub fn place(v: &mut Voxels, content: &Region, from: Option<RegionBox>, rot: &Rot, to: RegionBox, merge_empty: bool) {
    if let Some(f) = from {
        region::set_box(v, f.lo, f.size, 0);
    }
    content.rotated(rot).resized(to.size).paste(v, to.lo, merge_empty);
}

/// The handle under a screen point.
fn region_handle_at(handles: &[(RegionHandle, Vec<Pos2>)], p: Pos2) -> Option<RegionHandle> {
    let mut best: Option<(f32, RegionHandle)> = None;
    for (h, pts) in handles {
        let d = match pts.len() {
            1 => (pts[0] - p).length() - 3.0,
            _ => pts.windows(2).map(|w| dist_to_segment(p, w[0], w[1])).fold(f32::MAX, f32::min),
        };
        if d < crate::mech::GIZMO_HIT_PX && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, *h));
        }
    }
    best.map(|b| b.1)
}

/// A new box between two points on the ground: whole voxels across, and
/// two voxels above and below them so it holds the surface.
fn box_between(a: Vec3, b: Vec3) -> RegionBox {
    let (lo, hi) = (a.min(b) - Vec3::new(0.0, 2.0 * VOXEL, 0.0), a.max(b) + Vec3::new(0.0, 2.0 * VOXEL, 0.0));
    let lo = (lo / VOXEL).floor();
    let hi = (hi / VOXEL).floor() + Vec3::ONE;
    RegionBox {
        lo: [lo.x as i32, lo.y as i32, lo.z as i32],
        size: [0, 1, 2].map(|k| ((hi[k] - lo[k]) as i32).max(1)),
    }
}

impl Editor {
    /// Copy, Cut, Paste, Duplicate, Delete, Deselect, Fill and the quarter
    /// turns on the selected region: the panel's buttons and, while a Region
    /// tool is picked, Ctrl+C / X / V / D and Delete. False when `a` isn't
    /// one of them.
    pub fn terrain_region_action(&mut self, a: &str) -> bool {
        if !matches!(
            a,
            "copy" | "cut" | "paste" | "duplicate" | "delete" | "deselect" | "fill" | "rotate_x" | "rotate_y" | "rotate_z"
        ) {
            return false;
        }
        if !self.terrain_editing_allowed() {
            return true;
        }
        match a {
            "paste" => {
                // Studio pastes where it was copied, then hands over to the
                // transform tool to place it (RegionSelect.paste).
                let Some((content, at)) = self.terrain_ed.clipboard.clone() else {
                    return true;
                };
                let base = self.terrain_ref().and_then(|t| self.doc.prop(t, "SmoothGrid"));
                self.terrain_ed.selection = Some(at);
                self.terrain_ed.lift = Some(Lift { base, content, from: None, rot: region::IDENTITY, revision: self.doc.revision, base_grid: None, preview: None });
                self.place_lift("Terrain Paste");
                self.terrain_ed.tool = Some(Tool::Move);
                return true;
            }
            "deselect" => {
                self.terrain_ed.selection = None;
                self.terrain_ed.lift = None;
                return true;
            }
            _ => {}
        }
        let Some(b) = self.terrain_ed.selection else {
            return true;
        };
        match a {
            "copy" | "cut" => {
                let mut v = self.terrain_voxels();
                let copied = Region::copy(&v, b.lo, b.size);
                self.info(format!("Copied a terrain region: {} voxels", copied.count()));
                self.terrain_ed.clipboard = Some((copied, b));
                if a == "cut" {
                    region::set_box(&mut v, b.lo, b.size, 0);
                    self.commit_terrain("Terrain Cut", &v);
                }
            }
            "delete" => {
                let mut v = self.terrain_voxels();
                region::set_box(&mut v, b.lo, b.size, 0);
                self.commit_terrain("Terrain Delete", &v);
            }
            "duplicate" => {
                // A copy lifted in place: moving it leaves the original.
                let base = self.terrain_ref().and_then(|t| self.doc.prop(t, "SmoothGrid"));
                let content = Region::copy(&grid_of(base.as_ref()), b.lo, b.size);
                self.terrain_ed.lift = Some(Lift { base, content, from: None, rot: region::IDENTITY, revision: self.doc.revision, base_grid: None, preview: None });
                self.terrain_ed.tool = Some(Tool::Move);
            }
            "fill" => {
                let mut v = self.terrain_voxels();
                let te = &self.terrain_ed;
                let label = match te.fill_mode {
                    FillMode::Fill => {
                        region::set_box(&mut v, b.lo, b.size, te.material);
                        "Terrain Fill"
                    }
                    FillMode::Replace => {
                        region::replace_box(&mut v, b.lo, b.size, te.replace_from, te.replace_to);
                        "Terrain Replace"
                    }
                };
                self.commit_terrain(label, &v);
            }
            _ => {
                let axis = match a {
                    "rotate_x" => 0,
                    "rotate_y" => 1,
                    _ => 2,
                };
                self.rotate_region(axis, 1);
            }
        }
        true
    }

    /// Whether a Region tool drag is going on in the 3D view.
    pub fn region_dragging(&self) -> bool {
        self.terrain_ed.region_drag.is_some()
    }

    /// Starts a Move of what `at` holds, unless one is going on that nothing
    /// else has changed since.
    fn lift_region(&mut self, at: RegionBox) {
        if self.terrain_ed.lift.as_ref().is_some_and(|l| l.revision == self.doc.revision) {
            return;
        }
        let base = self.terrain_ref().and_then(|t| self.doc.prop(t, "SmoothGrid"));
        let content = Region::copy(&grid_of(base.as_ref()), at.lo, at.size);
        self.terrain_ed.lift = Some(Lift { base, content, from: Some(at), rot: region::IDENTITY, revision: self.doc.revision, base_grid: None, preview: None });
    }

    /// Places the lifted terrain in the selected box, as one undo step:
    /// the terrain from before the lift, its source cleared (Move), the
    /// content turned and stretched to the box, pasted with its empty
    /// voxels only under Merge Empty (TransformOperation.onStep).
    fn place_lift(&mut self, label: &str) {
        let (Some(b), Some(lift)) = (self.terrain_ed.selection, self.terrain_ed.lift.as_ref()) else {
            return;
        };
        let mut v = grid_of(lift.base.as_ref());
        place(&mut v, &lift.content, lift.from, &lift.rot, b, self.terrain_ed.merge_empty);
        self.commit_terrain(label, &v);
        let revision = self.doc.revision;
        if let Some(l) = self.terrain_ed.lift.as_mut() {
            l.revision = revision;
            l.preview = Some((b, l.rot));
        }
    }

    /// Shows the lifted terrain placed in `to` (turned by `rot`) while a
    /// handle is dragged, without touching the document: Studio's Live
    /// Edit (TransformOperation restores the last preview's backup, then
    /// places again). Only the drawn grid changes: the voxels the last
    /// preview and the source covered come back from the grid as it was at
    /// the lift, the content goes in, and just those chunks are re-meshed
    /// (as a brush stroke). Release commits it as one undo step.
    fn preview_lift(&mut self, to: RegionBox, rot: Rot) {
        let merge_empty = self.terrain_ed.merge_empty;
        let Some(lift) = self.terrain_ed.lift.as_mut() else {
            return;
        };
        if lift.revision != self.doc.revision || lift.preview == Some((to, rot)) {
            return;
        }
        let Some((id, r, mut v)) = self.terrain_drawn.take() else {
            return;
        };
        let base = lift.base_grid.get_or_insert_with(|| grid_of(lift.base.as_ref()));
        let mut boxes = vec![to];
        boxes.extend(lift.from);
        boxes.extend(lift.preview.map(|p| p.0));
        let mut water = false;
        for b in &boxes[1..] {
            let cells: Vec<_> = region::each(b.lo, b.size).map(|w| (w, base.get_raw(w))).collect();
            water |= cells.iter().any(|(_, c)| c.0 == WATER || c.2 != 0);
            v.set_raw_many(cells);
        }
        place(&mut v, &lift.content, lift.from, &rot, to, merge_empty);
        water |= lift.content.has_water();
        let lo = [0, 1, 2].map(|k| boxes.iter().map(|b| b.lo[k]).min().unwrap_or(0));
        let hi = [0, 1, 2].map(|k| boxes.iter().map(|b| b.lo[k] + b.size[k] - 1).max().unwrap_or(0));
        lift.preview = Some((to, rot));
        self.terrain_brushed = Some((v, terrain::dirty_box(lo, hi), water));
        self.terrain_drawn = Some((id, r, Voxels::empty()));
        self.scene.terrain_dirty = true;
    }

    /// Turns the selected region `turns` quarter turns about an axis.
    fn rotate_region(&mut self, axis: usize, turns: i32) {
        let Some(b) = self.terrain_ed.selection else {
            return;
        };
        if turns.rem_euclid(4) == 0 {
            return;
        }
        self.lift_region(b);
        let q = region::quarter_turns(axis, turns);
        if let Some(l) = self.terrain_ed.lift.as_mut() {
            l.rot = region::mul(q, l.rot);
        }
        self.terrain_ed.selection = Some(b.rotated(&q));
        self.place_lift("Terrain Rotate");
    }

    /// The ground under a screen point: the voxels, else the y = 0 plane.
    fn region_ground(&self, rect: Rect, p: Pos2) -> Option<Vec3> {
        let ray = self.cam.ray(rect, p);
        let v = self.terrain_drawn.as_ref().map(|(_, _, v)| v);
        v.and_then(|v| ray_voxels(v, ray.origin, ray.dir, 4000.0, true).map(|h| h.0)).or_else(|| {
            (ray.dir.y.abs() > 1e-4)
                .then(|| -ray.origin.y / ray.dir.y)
                .filter(|t| *t > 0.0)
                .map(|t| ray.origin + ray.dir * t)
        })
    }

    /// The selected box's handles for a tool, on screen: arrows (Move),
    /// face dots (Select, Resize, Fill) or rings (Rotate).
    pub(crate) fn region_handles(&self, rect: Rect, tool: Tool) -> Vec<(RegionHandle, Vec<Pos2>)> {
        let Some(b) = self.terrain_ed.selection else {
            return Vec::new();
        };
        let (c, half) = (b.center(), (b.max() - b.min()) * 0.5);
        let wpp = self.cam.world_per_px(rect, c);
        let mut out = Vec::new();
        for axis in 0..3 {
            let dir = Vec3::AXES[axis];
            if tool == Tool::Rotate {
                let radius = (half.length() + 6.0 * wpp).max(crate::mech::GIZMO_RING_PX * wpp);
                let (u, v) = (Vec3::AXES[(axis + 1) % 3], Vec3::AXES[(axis + 2) % 3]);
                let pts: Vec<Pos2> = (0..=64)
                    .filter_map(|i| {
                        let a = i as f32 / 64.0 * std::f32::consts::TAU;
                        self.cam.project(rect, c + (u * a.cos() + v * a.sin()) * radius)
                    })
                    .collect();
                if pts.len() > 2 {
                    out.push((RegionHandle::Ring(axis), pts));
                }
                continue;
            }
            for sign in [1, -1] {
                let face = c + dir * sign as f32 * half[axis];
                let pts = if tool == Tool::Move {
                    let base = face + dir * sign as f32 * 6.0 * wpp;
                    let tip = base + dir * sign as f32 * crate::mech::GIZMO_ARROW_PX * 0.55 * wpp;
                    match (self.cam.project(rect, base), self.cam.project(rect, tip)) {
                        (Some(a), Some(t)) => vec![a, t],
                        _ => continue,
                    }
                } else {
                    match self.cam.project(rect, face) {
                        Some(p) => vec![p],
                        None => continue,
                    }
                };
                out.push((RegionHandle::Face(axis, sign), pts));
            }
        }
        out
    }

    /// The Region tools in the 3D view: the box and its handles, a drag on
    /// the ground for a new box (Select, Fill), a drag on a handle to move,
    /// resize or turn it. True while it has the left button.
    fn terrain_region_view(
        &mut self,
        ctx: &egui::Context,
        rect: Rect,
        resp: &egui::Response,
        pointer: Option<Pos2>,
        painter: &egui::Painter,
    ) -> bool {
        let Some(tool) = self.terrain_ed.tool else {
            return false;
        };
        let handles = self.region_handles(rect, tool);
        let hovered = pointer.and_then(|p| region_handle_at(&handles, p));
        // The box, and where a ring drag would turn it to.
        if let Some(b) = self.terrain_ed.selection {
            for [a, c] in box_edges(&self.cam, rect, &b) {
                painter.line_segment([a, c], Stroke::new(1.5, theme::SELECT));
            }
        }
        let active = self.terrain_ed.region_drag.as_ref().and_then(|d| d.handle);
        for (h, pts) in &handles {
            let axis = match h {
                RegionHandle::Face(a, _) | RegionHandle::Ring(a) => *a,
            };
            let col = if hovered == Some(*h) || active == Some(*h) {
                Color32::from_rgb(255, 214, 92)
            } else {
                crate::viewport::AXIS_COLORS[axis]
            };
            match (h, pts.len()) {
                (RegionHandle::Ring(_), _) => {
                    painter.add(egui::Shape::line(pts.clone(), Stroke::new(2.0, col)));
                }
                (_, 1) => {
                    painter.circle_filled(pts[0], 6.0, col);
                    painter.circle_stroke(pts[0], 6.0, Stroke::new(1.0, Color32::BLACK));
                }
                _ => {
                    painter.line_segment([pts[0], pts[1]], Stroke::new(3.0, col));
                    painter.circle_filled(pts[1], 5.0, col);
                }
            }
        }
        // Press: a handle, else (Select, Fill) a new box from the ground.
        if resp.drag_started_by(egui::PointerButton::Primary) && self.terrain_ed.region_drag.is_none() {
            let start = ctx.input(|i| i.pointer.press_origin()).or(pointer);
            if let (Some(p), true) = (start, rect.contains(start.unwrap_or_default())) {
                let h = region_handle_at(&handles, p);
                let ground = self.region_ground(rect, p);
                let sel = self.terrain_ed.selection;
                let drag = match (h, sel) {
                    (Some(h), Some(b)) => {
                        if tool.transforms() {
                            self.lift_region(b);
                        }
                        let ray = self.cam.ray(rect, p);
                        let (origin, start_param) = match h {
                            RegionHandle::Face(axis, sign) => {
                                let dir = Vec3::AXES[axis] * sign as f32;
                                let o = b.center() + dir * (b.max() - b.min())[axis] * 0.5;
                                (o, ray.closest_on_line(o, dir).unwrap_or(0.0))
                            }
                            RegionHandle::Ring(_) => (b.center(), 0.0),
                        };
                        let center_px = self.cam.project(rect, b.center()).unwrap_or(p);
                        Some(RegionDrag {
                            handle: Some(h),
                            start: b,
                            origin,
                            start_param,
                            center_px,
                            last_angle: (p.y - center_px.y).atan2(p.x - center_px.x),
                            turned: 0.0,
                        })
                    }
                    _ if matches!(tool, Tool::Select | Tool::Fill) => ground.map(|g| RegionDrag {
                        handle: None,
                        start: box_between(g, g),
                        origin: g,
                        start_param: 0.0,
                        center_px: p,
                        last_angle: 0.0,
                        turned: 0.0,
                    }),
                    _ => None,
                };
                if let Some(d) = drag {
                    if d.handle.is_none() {
                        self.terrain_ed.selection = Some(d.start);
                        self.terrain_ed.lift = None;
                    }
                    self.terrain_ed.region_drag = Some(d);
                }
            }
        }
        let Some(mut drag) = self.terrain_ed.region_drag.take() else {
            return resp.hovered();
        };
        // Held: the box follows the mouse in whole voxels.
        if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
            let ray = self.cam.ray(rect, p);
            let mut b = drag.start;
            match drag.handle {
                None => {
                    if let Some(g) = self.region_ground(rect, p) {
                        b = box_between(drag.origin, g);
                    }
                }
                Some(RegionHandle::Face(axis, sign)) => {
                    let dir = Vec3::AXES[axis] * sign as f32;
                    let t = ray.closest_on_line(drag.origin, dir).unwrap_or(drag.start_param);
                    let n = ((t - drag.start_param) / VOXEL).round() as i32;
                    if tool == Tool::Move {
                        b.lo[axis] += sign * n;
                    } else {
                        let size = (drag.start.size[axis] + n).max(1);
                        if sign < 0 {
                            b.lo[axis] = drag.start.lo[axis] + drag.start.size[axis] - size;
                        }
                        b.size[axis] = size;
                    }
                }
                Some(RegionHandle::Ring(axis)) => {
                    let a = (p.y - drag.center_px.y).atan2(p.x - drag.center_px.x);
                    let pi = std::f32::consts::PI;
                    drag.turned += (a - drag.last_angle + pi).rem_euclid(2.0 * pi) - pi;
                    drag.last_angle = a;
                    // As the part Rotate tool: screen y points down.
                    let facing = Vec3::AXES[axis].dot(self.cam.pos - drag.origin) > 0.0;
                    let deg = if facing { -drag.turned.to_degrees() } else { drag.turned.to_degrees() };
                    let k = (deg / 90.0).round() as i32;
                    b = drag.start.rotated(&region::quarter_turns(axis, k));
                    painter.text(
                        drag.center_px,
                        egui::Align2::CENTER_CENTER,
                        format!("{}°", k * 90),
                        egui::FontId::proportional(14.0),
                        Color32::WHITE,
                    );
                }
            }
            self.terrain_ed.selection = Some(b);
            // Live preview: the terrain follows the box (round 26).
            if let (Some(h), true) = (drag.handle, tool.transforms()) {
                let rot = self.terrain_ed.lift.as_ref().map(|l| l.rot).unwrap_or(region::IDENTITY);
                let rot = match h {
                    RegionHandle::Ring(axis) => {
                        let facing = Vec3::AXES[axis].dot(self.cam.pos - drag.origin) > 0.0;
                        let deg = if facing { -drag.turned.to_degrees() } else { drag.turned.to_degrees() };
                        region::mul(region::quarter_turns(axis, (deg / 90.0).round() as i32), rot)
                    }
                    RegionHandle::Face(..) => rot,
                };
                self.preview_lift(b, rot);
            }
        }
        if ctx.input(|i| i.pointer.primary_down()) {
            self.terrain_ed.region_drag = Some(drag);
            ctx.request_repaint();
            return true;
        }
        // Released: Move / Resize / Rotate place the terrain (one undo step).
        // The document still shows the content where the drag began; the
        // drawn grid goes back to it below, so the next preview restores
        // that box (a placement below moves it on).
        if let (Some(_), Some(l)) = (drag.handle, self.terrain_ed.lift.as_mut()) {
            l.preview = Some((drag.start, l.rot));
        }
        match (drag.handle, tool) {
            (Some(RegionHandle::Ring(axis)), _) => {
                self.terrain_ed.selection = Some(drag.start);
                let facing = Vec3::AXES[axis].dot(self.cam.pos - drag.origin) > 0.0;
                let deg = if facing { -drag.turned.to_degrees() } else { drag.turned.to_degrees() };
                self.rotate_region(axis, (deg / 90.0).round() as i32);
            }
            (Some(_), Tool::Move) => self.place_lift("Terrain Move"),
            (Some(_), Tool::Resize) => self.place_lift("Terrain Resize"),
            _ => self.terrain_ed.lift = None,
        }
        // The drawn grid goes back to the document's (a ring drag that ends
        // on no quarter turn commits nothing after previewing one).
        self.scene.terrain_dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_adds_and_subtract_removes() {
        let mut v = Voxels::empty();
        let ed = TerrainEditor { strength: 1.0, size: 12.0, ..Default::default() };
        apply_brush(&ed, &mut v, Tool::Draw, Vec3::new(10.0, 10.0, 10.0), 0.0);
        let n = v.count();
        assert!(n > 20, "added {n}");
        assert_eq!(v.get(2, 2, 2).0, 2, "grass at the centre");
        let sub = TerrainEditor { invert: true, ..ed };
        for _ in 0..4 {
            apply_brush(&sub, &mut v, Tool::Draw, Vec3::new(10.0, 10.0, 10.0), 0.0);
        }
        assert_eq!(v.get(2, 2, 2).0, 0, "the centre carved");
        assert!(v.count() < n, "left {} of {n}", v.count());
    }

    #[test]
    fn flatten_levels_to_the_plane() {
        let mut v = Voxels::empty();
        generate(&mut v, [0.0, 32.0, 0.0], [64.0, 64.0, 64.0], 7, Biome::Mountains, false);
        let ed = TerrainEditor { strength: 1.0, size: 24.0, shape: Shape::Cylinder, ..Default::default() };
        for _ in 0..4 {
            apply_brush(&ed, &mut v, Tool::Flatten, Vec3::new(2.0, 20.0, 2.0), 20.0);
        }
        // Voxels well under the plane are full, well over it empty.
        assert!(v.get(0, 3, 0).1 > 0.9);
        assert_eq!(v.get(0, 7, 0).0, 0);
    }

    #[test]
    fn sea_level_fills_air_only() {
        let mut v = Voxels::empty();
        v.set([0, 0, 0], 8, 1.0);
        sea_level(&mut v, [0.0, 0.0, 0.0], [16.0, 16.0, 16.0], false);
        assert_eq!(v.get(0, 0, 0).0, 8);
        assert_eq!(v.get(1, 1, 1).0, WATER);
        sea_level(&mut v, [0.0, 0.0, 0.0], [16.0, 16.0, 16.0], true);
        assert_eq!(v.get(1, 1, 1).0, 0);
        assert_eq!(v.count(), 1);
    }

    #[test]
    fn region_move_merges_or_replaces() {
        // A 2×2×2 Rock block with one empty corner, moved onto Sand.
        let mut v = Voxels::empty();
        region::set_box(&mut v, [0, 0, 0], [2, 2, 2], 8);
        v.set([1, 1, 1], 0, 0.0);
        region::set_box(&mut v, [10, 0, 0], [2, 2, 2], 6);
        let from = RegionBox { lo: [0, 0, 0], size: [2, 2, 2] };
        let to = RegionBox { lo: [10, 0, 0], size: [2, 2, 2] };
        let content = Region::copy(&v, from.lo, from.size);
        let mut merged = Voxels::decode(&v.encode()).unwrap();
        place(&mut merged, &content, Some(from), &region::IDENTITY, to, false);
        assert_eq!(merged.get(0, 0, 0).0, 0, "the source is cleared");
        assert_eq!(merged.get(10, 0, 0).0, 8);
        assert_eq!(merged.get(11, 1, 1).0, 6, "the empty corner keeps the Sand");
        let mut replaced = Voxels::decode(&v.encode()).unwrap();
        place(&mut replaced, &content, Some(from), &region::IDENTITY, to, true);
        assert_eq!(replaced.get(11, 1, 1).0, 0, "Merge Empty: the corner clears it");
        assert_eq!(replaced.count(), 7);
        // Duplicate (no source) keeps the original.
        let mut dup = Voxels::decode(&v.encode()).unwrap();
        place(&mut dup, &content, None, &region::IDENTITY, to, false);
        assert_eq!(dup.get(0, 0, 0).0, 8);
    }

    #[test]
    fn region_box_rotates_and_snaps() {
        let b = RegionBox { lo: [0, 0, 0], size: [4, 2, 2] };
        let r = b.rotated(&region::quarter_turns(1, 1));
        assert_eq!(r, RegionBox { lo: [1, 0, -1], size: [2, 2, 4] });
        assert_eq!(r.center(), b.center());
        assert_eq!(r.rotated(&region::quarter_turns(1, -1)), b);
        // Studs snap outward to whole voxels.
        let s = RegionBox::from_studs([2.0, 1.0, 0.0], [6.0, 2.0, 3.0]);
        assert_eq!(s, RegionBox { lo: [-1, 0, -1], size: [3, 1, 2] });
    }

    #[test]
    fn ray_finds_the_ground() {
        let mut v = Voxels::empty();
        generate(&mut v, [0.0, 16.0, 0.0], [64.0, 32.0, 64.0], 1, Biome::Plains, false);
        let hit = ray_voxels(&v, Vec3::new(0.0, 100.0, 0.0), -Vec3::Y, 200.0, true);
        let (p, n) = hit.expect("hit");
        assert!(p.y > 0.0 && p.y < 32.0, "{p}");
        assert!(n.y > 0.5, "{n}");
    }
}
