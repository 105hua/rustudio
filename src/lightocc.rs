// What blocks local lights (round 26): a voxel grid of how much of each
// voxel the parts fill, around the part of the world in view.
//
// Studio blocks a PointLight / SpotLight / SurfaceLight with Shadows on by
// the parts between it and a surface: Future with shadow maps (hard: a
// 0.2-stud wall leaves the floor behind it at the ambient level), Soft
// through its 4-stud lighting voxels (a closed room keeps its light in,
// but an open wall doesn't: the light gets round it). Lights with Shadows
// off go through everything in both. Measured on corpus lightwall-soft /
// lightwall-real (make_lightwall_place.py); docs/MECHANICS.md "Round 26".
//
// rustudio fills the grid from the built-in parts' boxes (MeshParts and
// unions are left out: their box is no guide to their shape) on a worker
// thread, and the part shader marches from the surface toward each shadowed
// light through it (`light_occlusion` in shader.wgsl), one nearest sample
// per voxel: fully solid voxels let exp(-LIGHTOCC_DENSITY × studs of solid)
// of the light through.

use glam::{Mat3, Vec3};

use crate::mech;
use crate::render::MeshKind;

/// The grid's size in voxels: (X, Y, Z).
pub fn dims() -> [usize; 3] {
    [mech::LIGHTOCC_CELLS_XZ, mech::LIGHTOCC_CELLS_Y, mech::LIGHTOCC_CELLS_XZ]
}

/// Studs per voxel: Soft's are Studio's 4-stud lighting voxels; Realistic's
/// finer, for its shadow-mapped lights.
pub fn voxel_size(soft: bool) -> f32 {
    if soft {
        mech::LIGHTOCC_VOXEL_SOFT
    } else {
        mech::LIGHTOCC_VOXEL_REAL
    }
}

/// Where a grid for this view starts, in voxels: centred LIGHTOCC_AHEAD
/// grid half-widths ahead of the camera (lights that reach what's in view
/// are mostly in front of it), snapped to LIGHTOCC_SNAP voxels so small
/// camera moves keep the same grid.
pub fn origin_for(cam: Vec3, forward: Vec3, voxel: f32) -> [i32; 3] {
    let d = dims();
    let half = d[0] as f32 * voxel * 0.5;
    let centre = cam + forward.normalize_or_zero() * half * mech::LIGHTOCC_AHEAD;
    let s = mech::LIGHTOCC_SNAP;
    let mut o = [0; 3];
    for i in 0..3 {
        let v = (centre[i] / voxel).floor() as i32;
        o[i] = v.div_euclid(s) * s - d[i] as i32 / 2;
    }
    o
}

/// One part as the grid needs it.
#[derive(Clone, Copy)]
pub struct Solid {
    pub pos: Vec3,
    pub rot: Mat3,
    pub half: Vec3,
    /// Share of the box the shape fills (a ball 0.52, a wedge 0.5).
    pub fill: f32,
}

/// The share of its box a built-in shape fills.
pub fn fill_of(mesh: MeshKind) -> f32 {
    match mesh {
        MeshKind::Block => 1.0,
        MeshKind::Ball => std::f32::consts::PI / 6.0,
        MeshKind::Cylinder => std::f32::consts::PI / 4.0,
        MeshKind::Wedge => 0.5,
        MeshKind::CornerWedge => 1.0 / 3.0,
    }
}

/// What a worker needs to fill a grid.
pub struct Gathered {
    pub origin: [i32; 3],
    pub voxel: f32,
    pub solids: Vec<Solid>,
}

/// A filled grid: occupancy 0..255 per voxel, X fastest, then Y, then Z
/// (the texture's width, height and depth).
pub struct OccGrid {
    pub origin: [i32; 3],
    pub voxel: f32,
    pub data: Vec<u8>,
}

impl Gathered {
    /// The grid's box in studs.
    pub fn bounds(origin: [i32; 3], voxel: f32) -> (Vec3, Vec3) {
        let d = dims();
        let lo = Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32) * voxel;
        let hi = lo + Vec3::new(d[0] as f32, d[1] as f32, d[2] as f32) * voxel;
        (lo, hi)
    }
}

/// Fills the grid: each voxel gets the share of it each box covers (summed,
/// at most full). Per box axis the voxel's extent along it (its projection)
/// is overlapped with the box: exact for boxes square to the world, a close
/// estimate for turned ones, and never misses a thin turned wall.
pub fn fill(g: Gathered) -> OccGrid {
    let d = dims();
    let (nx, ny, nz) = (d[0], d[1], d[2]);
    let v = g.voxel;
    let (lo, _) = Gathered::bounds(g.origin, v);
    let mut acc = vec![0u16; nx * ny * nz];
    for s in &g.solids {
        let r = s.rot;
        let axes = [r.x_axis, r.y_axis, r.z_axis];
        let ext = axes[0].abs() * s.half.x + axes[1].abs() * s.half.y + axes[2].abs() * s.half.z;
        let a = ((s.pos - ext - lo) / v).floor();
        let b = ((s.pos + ext - lo) / v).floor();
        if b.x < 0.0 || b.y < 0.0 || b.z < 0.0 || a.x >= nx as f32 || a.y >= ny as f32 || a.z >= nz as f32 {
            continue;
        }
        let (x0, y0, z0) = (a.x.max(0.0) as usize, a.y.max(0.0) as usize, a.z.max(0.0) as usize);
        let (x1, y1, z1) = (
            (b.x as usize).min(nx - 1),
            (b.y as usize).min(ny - 1),
            (b.z as usize).min(nz - 1),
        );
        // Half the voxel's extent along each box axis.
        let proj = [
            (axes[0].abs().x + axes[0].abs().y + axes[0].abs().z) * v * 0.5,
            (axes[1].abs().x + axes[1].abs().y + axes[1].abs().z) * v * 0.5,
            (axes[2].abs().x + axes[2].abs().y + axes[2].abs().z) * v * 0.5,
        ];
        let h = [s.half.x, s.half.y, s.half.z];
        let w = s.fill * 255.0;
        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let c = lo + (Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5)) * v;
                    let rel = c - s.pos;
                    let mut share = 1.0;
                    for i in 0..3 {
                        let t = axes[i].dot(rel);
                        let overlap = (t + proj[i]).min(h[i]) - (t - proj[i]).max(-h[i]);
                        if overlap <= 0.0 {
                            share = 0.0;
                            break;
                        }
                        share *= (overlap / (2.0 * proj[i])).min(1.0);
                    }
                    if share > 0.0 {
                        let k = (z * ny + y) * nx + x;
                        acc[k] = acc[k].saturating_add((share * w + 0.5) as u16);
                    }
                }
            }
        }
    }
    OccGrid {
        origin: g.origin,
        voxel: v,
        data: acc.into_iter().map(|a| a.min(255) as u8).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_with(solid: Solid, voxel: f32) -> OccGrid {
        fill(Gathered { origin: [0, 0, 0], voxel, solids: vec![solid] })
    }

    fn at(g: &OccGrid, x: usize, y: usize, z: usize) -> u8 {
        let d = dims();
        g.data[(z * d[1] + y) * d[0] + x]
    }

    #[test]
    fn square_box_fills_its_share() {
        // A 1-stud wall across voxel x 2 (studs 8..12), at studs 8.5..9.5.
        let g = grid_with(
            Solid { pos: Vec3::new(9.0, 20.0, 20.0), rot: Mat3::IDENTITY, half: Vec3::new(0.5, 8.0, 8.0), fill: 1.0 },
            4.0,
        );
        assert_eq!(at(&g, 2, 5, 5), 64); // a quarter of the voxel
        assert_eq!(at(&g, 1, 5, 5), 0);
        assert_eq!(at(&g, 3, 5, 5), 0);
    }

    #[test]
    fn turned_thin_wall_is_not_missed() {
        // A 0.5-stud wall turned 45° about Y through voxel centres.
        let rot = Mat3::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let g = grid_with(
            Solid { pos: Vec3::new(42.0, 42.0, 42.0), rot, half: Vec3::new(10.0, 10.0, 0.25), fill: 1.0 },
            4.0,
        );
        let hit = (8..=12).filter(|&x| at(&g, x, 10, 20 - x) > 0).count();
        assert_eq!(hit, 5, "voxels along the wall");
    }
}
