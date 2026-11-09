//! Terrain regions (round 25): a box of voxels lifted out of the grid.
//! It is what Terrain:CopyRegion returns (a TerrainRegion) and what the
//! Terrain Editor's Region tools copy, paste, move, resize and rotate.
//! Cells keep their stored bytes (material, occupancy byte, shoreline
//! water), so a copy pasted back is the same terrain.
//!
//! Studio's Terrain Editor does the same with its plugin-only TerrainRegion
//! methods (Src/Util/Operations/TransformOperation): copy the box, clear the
//! source when moving, clear the target when Merge Empty is on, transform
//! the copy (rotation and size), paste it without its empty cells.

use crate::terrain::Voxels;

/// A voxel's stored bytes (`Voxels::get_raw`).
pub type Cell = (u8, u8, u16);

const AIR: Cell = (0, 0, 0);

/// A box of voxels, `size` cells on each axis. Only the part from `at`,
/// `n` cells, is stored (x fastest, then y, then z); the rest is Air, so
/// Terrain:CopyRegion(Terrain.MaxExtents) holds just the terrain there is.
#[derive(Clone, Debug)]
pub struct Region {
    pub size: [i32; 3],
    at: [i32; 3],
    n: [i32; 3],
    cells: Vec<Cell>,
}

/// An axis-aligned rotation in 90° steps: a matrix of 0 / ±1.
pub type Rot = [[i32; 3]; 3];

pub const IDENTITY: Rot = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];

/// A quarter turn (counter-clockwise looking down the axis, CFrame.Angles'
/// sense) about axis 0 / 1 / 2, `turns` times.
pub fn quarter_turns(axis: usize, turns: i32) -> Rot {
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut q = [[0; 3]; 3];
    q[axis][axis] = 1;
    q[b][a] = 1;
    q[a][b] = -1;
    let mut r = IDENTITY;
    for _ in 0..turns.rem_euclid(4) {
        r = mul(q, r);
    }
    r
}

pub fn mul(a: Rot, b: Rot) -> Rot {
    let mut out = [[0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn apply(r: &Rot, v: [i32; 3]) -> [i32; 3] {
    [0, 1, 2].map(|i| (0..3).map(|k| r[i][k] * v[k]).sum())
}

/// A box's size once rotated.
pub fn rotated_size(r: &Rot, size: [i32; 3]) -> [i32; 3] {
    apply(r, size).map(i32::abs)
}

/// The cells of a box (first cell, size), x fastest.
pub fn each(lo: [i32; 3], n: [i32; 3]) -> impl Iterator<Item = [i32; 3]> {
    (0..n[2]).flat_map(move |z| (0..n[1]).flat_map(move |y| (0..n[0]).map(move |x| [lo[0] + x, lo[1] + y, lo[2] + z])))
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The overlap of two boxes (first cell, size); None when they don't meet.
fn overlap(a: ([i32; 3], [i32; 3]), b: ([i32; 3], [i32; 3])) -> Option<([i32; 3], [i32; 3])> {
    let lo = [0, 1, 2].map(|k| a.0[k].max(b.0[k]));
    let hi = [0, 1, 2].map(|k| (a.0[k] + a.1[k]).min(b.0[k] + b.1[k]));
    (0..3).all(|k| hi[k] > lo[k]).then(|| (lo, [0, 1, 2].map(|k| hi[k] - lo[k])))
}

/// The grid's non-Air voxels as a box (first cell, size).
fn grid_box(v: &Voxels) -> Option<([i32; 3], [i32; 3])> {
    v.bounds().map(|(lo, hi)| (lo, [0, 1, 2].map(|k| hi[k] - lo[k] + 1)))
}

impl PartialEq for Region {
    fn eq(&self, o: &Region) -> bool {
        let lo = [0, 1, 2].map(|k| self.at[k].min(o.at[k]));
        let hi = [0, 1, 2].map(|k| (self.at[k] + self.n[k]).max(o.at[k] + o.n[k]));
        self.size == o.size && each(lo, [0, 1, 2].map(|k| hi[k] - lo[k])).all(|p| self.get(p) == o.get(p))
    }
}

impl Region {
    /// All Air.
    pub fn empty(size: [i32; 3]) -> Region {
        Region { size: size.map(|s| s.max(0)), at: [0; 3], n: [0; 3], cells: Vec::new() }
    }

    /// Air, with room for cells from `at`, `n` of them.
    fn stored(size: [i32; 3], at: [i32; 3], n: [i32; 3]) -> Region {
        let n = n.map(|s| s.max(0));
        let len = n.iter().map(|s| *s as usize).product();
        Region { size: size.map(|s| s.max(0)), at, n, cells: vec![AIR; len] }
    }

    fn index(&self, p: [i32; 3]) -> Option<usize> {
        let q = [0, 1, 2].map(|k| p[k] - self.at[k]);
        (0..3)
            .all(|k| q[k] >= 0 && q[k] < self.n[k])
            .then(|| (q[0] + self.n[0] * (q[1] + self.n[1] * q[2])) as usize)
    }

    pub fn get(&self, p: [i32; 3]) -> Cell {
        self.index(p).map_or(AIR, |i| self.cells[i])
    }

    fn set(&mut self, p: [i32; 3], c: Cell) {
        if let Some(i) = self.index(p) {
            self.cells[i] = c;
        }
    }

    /// Non-Air cells.
    pub fn count(&self) -> usize {
        self.cells.iter().filter(|c| c.0 != 0).count()
    }

    /// Terrain:CopyRegion: the cells from `lo` (voxel), `size` of them.
    pub fn copy(v: &Voxels, lo: [i32; 3], size: [i32; 3]) -> Region {
        let Some((wlo, n)) = grid_box(v).and_then(|g| overlap((lo, size), g)) else {
            return Region::empty(size);
        };
        let at = [0, 1, 2].map(|k| wlo[k] - lo[k]);
        let mut r = Region::stored(size, at, n);
        for p in each(at, n) {
            let c = v.get_raw(add(lo, p));
            if c.0 != 0 {
                r.set(p, c);
            }
        }
        r
    }

    /// Terrain:PasteRegion: writes the cells with their first corner at
    /// `corner`; empty cells clear what is there only with `paste_empty`.
    pub fn paste(&self, v: &mut Voxels, corner: [i32; 3], paste_empty: bool) {
        let mut cells: Vec<([i32; 3], Cell)> = Vec::new();
        if paste_empty {
            // The whole box is cleared first (only where there is terrain).
            if let Some((lo, n)) = grid_box(v).and_then(|g| overlap((corner, self.size), g)) {
                cells.extend(each(lo, n).map(|w| (w, AIR)));
            }
        }
        for p in each(self.at, self.n) {
            let c = self.get(p);
            if c.0 != 0 {
                cells.push((add(corner, p), c));
            }
        }
        v.set_raw_many(cells);
    }

    /// The region turned by `r` about its centre (whole cells, nothing lost).
    pub fn rotated(&self, r: &Rot) -> Region {
        let size = rotated_size(r, self.size);
        if self.n.contains(&0) {
            return Region::empty(size);
        }
        // Doubled coordinates from the centre stay whole.
        let turn = |p: [i32; 3]| {
            let q = apply(r, [0, 1, 2].map(|k| 2 * p[k] + 1 - self.size[k]));
            [0, 1, 2].map(|k| (q[k] + size[k] - 1) / 2)
        };
        let (a, b) = (turn(self.at), turn(add(self.at, self.n.map(|n| n - 1))));
        let at = [0, 1, 2].map(|k| a[k].min(b[k]));
        let mut out = Region::stored(size, at, [0, 1, 2].map(|k| (a[k] - b[k]).abs() + 1));
        for p in each(self.at, self.n) {
            let c = self.get(p);
            if c.0 != 0 {
                out.set(turn(p), c);
            }
        }
        out
    }

    /// The region stretched to `size` cells (nearest cell; the Resize tool).
    pub fn resized(&self, size: [i32; 3]) -> Region {
        if size == self.size {
            return self.clone();
        }
        if self.size.contains(&0) || self.n.contains(&0) {
            return Region::empty(size);
        }
        let mut out = Region::stored(size, [0; 3], size);
        for p in each([0; 3], out.size) {
            let s = [0, 1, 2].map(|k| ((2 * p[k] + 1) * self.size[k] / (2 * out.size[k])).min(self.size[k] - 1));
            let c = self.get(s);
            if c.0 != 0 {
                out.set(p, c);
            }
        }
        out
    }

    /// Whether any cell holds water (a Water voxel or shoreline water).
    pub fn has_water(&self) -> bool {
        self.cells.iter().any(|c| c.0 == crate::terrain::WATER || c.2 != 0)
    }

    /// The cells as a SmoothGrid blob in world cells from `min` (the
    /// region's ExtentsMin), as Studio saves a TerrainRegion (round 26: a
    /// CopyRegion of cells (10, 1, 20) .. (13, 2, 23) serialized with
    /// SerializationService holds them at those cells).
    pub fn to_smooth_grid(&self, min: [i32; 3]) -> Vec<u8> {
        let mut v = Voxels::empty();
        self.paste(&mut v, min, false);
        v.encode()
    }

    /// A TerrainRegion's cells back from its SmoothGrid and extents
    /// (ExtentsMax inclusive, as Studio saves it). A grid holding cells
    /// from (0, 0, 0) outside the extents (rustudio's round 25 copies) is
    /// read as region cells.
    pub fn from_smooth_grid(bytes: &[u8], min: [i32; 3], max: [i32; 3]) -> Result<Region, String> {
        let v = Voxels::decode(bytes)?;
        let size = [0, 1, 2].map(|k| (max[k] - min[k] + 1).max(0));
        let inside = |lo: [i32; 3], hi: [i32; 3], a: [i32; 3]| (0..3).all(|k| lo[k] >= a[k] && hi[k] < a[k] + size[k]);
        let local = v.bounds().is_some_and(|(lo, hi)| !inside(lo, hi, min) && inside(lo, hi, [0; 3]));
        Ok(Region::copy(&v, if local { [0; 3] } else { min }, size))
    }
}

/// Every voxel in the box from `lo`, `size` cells, set to `mat` at full
/// occupancy (Air clears them): Studio's Fill / Delete on a region.
pub fn set_box(v: &mut Voxels, lo: [i32; 3], size: [i32; 3], mat: u8) {
    let cell = if mat == 0 { AIR } else { (mat, 255, 0) };
    v.set_raw_many(each(lo, size).map(|w| (w, cell)).collect::<Vec<_>>());
}

/// `from` becomes `to` in the box; Air as `from` fills the empty cells,
/// Air as `to` clears (the Fill tool's Replace mode).
pub fn replace_box(v: &mut Voxels, lo: [i32; 3], size: [i32; 3], from: u8, to: u8) {
    let cells: Vec<_> = each(lo, size)
        .filter_map(|w| {
            let c = v.get_raw(w);
            (c.0 == from).then(|| (w, if to == 0 { AIR } else if from == 0 { (to, 255, 0) } else { (to, c.1, c.2) }))
        })
        .collect();
    v.set_raw_many(cells);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Voxels {
        // An L of Rock with a half-full Grass tip, and a lone Snow voxel.
        let mut v = Voxels::empty();
        for x in 0..4 {
            v.set([x, 0, 0], 8, 1.0);
        }
        v.set([0, 1, 0], 8, 1.0);
        v.set([0, 2, 0], 2, 0.5);
        v.set([3, 0, 2], 10, 0.25);
        v
    }

    #[test]
    fn copy_paste_round_trip() {
        let v = sample();
        let r = Region::copy(&v, [0, 0, 0], [4, 3, 3]);
        assert_eq!(r.count(), 7);
        let mut w = Voxels::empty();
        r.paste(&mut w, [10, -5, 20], true);
        for p in each([0; 3], [4, 3, 3]) {
            assert_eq!(w.get_raw([10 + p[0], -5 + p[1], 20 + p[2]]), v.get_raw(p), "{p:?}");
        }
        assert_eq!(w.count(), 7);
        // The SmoothGrid form of a TerrainRegion (world cells, Max
        // inclusive) reads back the same.
        let back = Region::from_smooth_grid(&r.to_smooth_grid([40, 8, 40]), [40, 8, 40], [43, 10, 42]).unwrap();
        assert_eq!(back, r);
        // Round 25 grids (cells from (0, 0, 0)) still read.
        let old = Region::from_smooth_grid(&r.to_smooth_grid([0; 3]), [40, 8, 40], [43, 10, 42]).unwrap();
        assert_eq!(old, r);
    }

    #[test]
    fn paste_keeps_or_clears_empty_cells() {
        let v = sample();
        let r = Region::copy(&v, [0, 0, 0], [2, 2, 1]);
        let mut w = Voxels::empty();
        w.set([1, 1, 0], 6, 1.0);
        r.paste(&mut w, [0, 0, 0], false);
        assert_eq!(w.get(1, 1, 0).0, 6, "merged: the sand stays");
        r.paste(&mut w, [0, 0, 0], true);
        assert_eq!(w.get(1, 1, 0).0, 0, "empty cells pasted: cleared");
    }

    #[test]
    fn rotate_quarter_turns() {
        let v = sample();
        let r = Region::copy(&v, [0, 0, 0], [4, 3, 3]);
        let y = quarter_turns(1, 1);
        // +X turns to −Z about +Y.
        assert_eq!(apply(&y, [1, 0, 0]), [0, 0, -1]);
        let t = r.rotated(&y);
        assert_eq!(t.size, [3, 3, 4]);
        assert_eq!(t.count(), r.count());
        // The rock row along +X now runs along −Z: from the box's z = 3 down.
        for x in 0..4 {
            assert_eq!(t.get([0, 0, 3 - x]).0, 8);
        }
        assert_eq!(t.get([0, 2, 3]).0, 2);
        // Four turns are the identity; two about X flip Y and Z.
        let mut back = r.clone();
        for _ in 0..4 {
            back = back.rotated(&y);
        }
        assert_eq!(back, r);
        let flip = r.rotated(&quarter_turns(0, 2));
        assert_eq!(flip.size, r.size);
        assert_eq!(flip.get([0, 2, 2]).0, 8);
        assert_eq!(flip.get([0, 0, 2]).0, 2);
    }

    #[test]
    fn resize_stretches_cells() {
        let v = sample();
        let r = Region::copy(&v, [0, 0, 0], [4, 3, 3]);
        let big = r.resized([8, 6, 6]);
        assert_eq!(big.count(), r.count() * 8);
        assert_eq!(big.get([7, 1, 0]).0, 8);
        let small = big.resized([4, 3, 3]);
        assert_eq!(small, r);
    }

    #[test]
    fn fill_and_replace_box() {
        let mut v = sample();
        set_box(&mut v, [0, 0, 0], [2, 2, 2], 6);
        assert_eq!(v.get(1, 1, 1), (6, 1.0));
        assert_eq!(v.get(2, 0, 0).0, 8, "outside the box");
        replace_box(&mut v, [0, 0, 0], [4, 1, 1], 8, 10);
        assert_eq!(v.get(3, 0, 0).0, 10);
        assert_eq!(v.get(0, 0, 0).0, 6, "Sand isn't the source");
        replace_box(&mut v, [0, 0, 0], [4, 4, 4], 0, 1);
        assert_eq!(v.get(3, 3, 3).0, 1, "Air source fills the empty cells");
        v.set([5, 0, 0], 8, 1.0);
        set_box(&mut v, [0, 0, 0], [4, 4, 4], 0);
        assert_eq!(v.get(5, 0, 0).0, 8, "outside");
        assert_eq!(v.count(), 1);
    }

    #[test]
    fn studio_saved_region_holds_world_cells() {
        // Round 26: Studio's own TerrainRegion from CopyRegion(Region3int16
        // (10, 1, 20) .. (13, 2, 23)) of a Rock block with Snow at its first
        // cell and Grass at its last (region_probe.luau, serialized with
        // SerializationService): its SmoothGrid, with ExtentsMin (10, 1, 20)
        // and ExtentsMax (13, 2, 23).
        let hex = format!(
            "010500000000000000000000000080ff80ff80ff80ff80ff80ff80890a8802801b8803801b8803801b880380ff80ff80ff\
             809b8803801b8803801b8803801b880202{}8011",
            "80ff".repeat(117)
        );
        let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let v = Voxels::decode(&bytes).unwrap();
        // World cells, the whole inclusive box.
        assert_eq!(v.bounds(), Some(([10, 1, 20], [13, 2, 23])));
        assert_eq!(v.count(), 32);
        let r = Region::from_smooth_grid(&bytes, [10, 1, 20], [13, 2, 23]).unwrap();
        assert_eq!(r.size, [4, 2, 4]);
        assert_eq!(r.get([0, 0, 0]).0, 10, "Snow first");
        assert_eq!(r.get([3, 1, 3]).0, 2, "Grass last");
        // rustudio writes the same blob.
        assert_eq!(Voxels::decode(&r.to_smooth_grid([10, 1, 20])).unwrap().encode(), v.encode());
    }
}
