//! Smooth terrain: decodes `Terrain.SmoothGrid` into sparse voxels and builds
//! smooth per-material surface meshes (render) plus a merged solid mesh
//! (collision).
//!
//! Format notes, from public community documentation only (rbx-dom's
//! `docs/smooth-grid.md` in pull request 444, zeux.io's 2017 "Voxel terrain:
//! storage" article, and the independent write-ups that checked it):
//!
//! * byte 0: version (only 1 is known), byte 1: log2 of the chunk edge (5 =>
//!   32^3 chunks; 0..=8 accepted).
//! * then chunks until the end of the blob, no count. Each chunk starts with
//!   its offset from the previous chunk (from 0,0,0 for the first), in chunk
//!   units: three big-endian `i32`s (X, Y, Z) whose bytes are interleaved
//!   (X0 Y0 Z0 X1 Y1 Z1 ...; byte 0 = most significant).
//! * voxels in the chunk follow as runs, X fastest, then Z, then Y. A run is a
//!   flag byte (bits 0-5 material index, bit 6 = occupancy byte follows,
//!   bit 7 = count byte follows), then the occupancy byte, then the count byte.
//!   A count byte `c > 0` means `c + 1` voxels; `c == 0` means one voxel
//!   followed by a shoreline water-occupancy byte. No occupancy byte means
//!   full. Occupancy byte `b` decodes to `(b + 1) / 256`; Air is always empty.
//! * voxel `v` covers studs `4v .. 4v + 4` (centre at `4v + 2`); chunk `c`
//!   starts at voxel `32c` (stud `128c`).
#![allow(dead_code)]

use std::collections::HashMap;

/// Studs per voxel edge.
pub const VOXEL: f32 = 4.0;
/// SmoothGrid material index of Water.
pub const WATER: u8 = 1;

const CHUNK_SHIFT: u32 = 5;
const CS: usize = 1 << CHUNK_SHIFT;
const CV: usize = CS * CS * CS;
const ISO: f32 = 0.5;
/// Largest accepted |voxel coordinate| (keeps all arithmetic inside i32).
const COORD_LIMIT: i64 = 1 << 29;

/// SmoothGrid material slots, in index order (a subset of Enum.Material).
const MATERIALS: [&str; 23] = [
    "Air",
    "Water",
    "Grass",
    "Slate",
    "Concrete",
    "Brick",
    "Sand",
    "WoodPlanks",
    "Rock",
    "Glacier",
    "Snow",
    "Sandstone",
    "Mud",
    "Basalt",
    "Ground",
    "CrackedLava",
    "Asphalt",
    "Cobblestone",
    "Ice",
    "LeafyGrass",
    "Salt",
    "Limestone",
    "Pavement",
];

/// Enum.Material name for a SmoothGrid material index ("Unknown" past the table).
pub fn material_name(m: u8) -> &'static str {
    MATERIALS.get(m as usize).copied().unwrap_or("Unknown")
}

/// SmoothGrid index for an Enum.Material name.
pub fn material_index(name: &str) -> Option<u8> {
    MATERIALS.iter().position(|n| *n == name).map(|i| i as u8)
}

/// Studio's default terrain colours: the `base_color` of each material in
/// its PlatformContent/pc/terrain/materials.json (read once, read only).
/// They are what Studio's Terrain:GetMaterialColor returns in a new place
/// (checked in an offline Studio for Grass, Rock, Sand, Snow, Mud, Slate and Brick).
fn studio_base_colors() -> Option<&'static std::collections::HashMap<String, [u8; 3]>> {
    use std::sync::OnceLock;
    static COLORS: OnceLock<Option<std::collections::HashMap<String, [u8; 3]>>> = OnceLock::new();
    COLORS
        .get_or_init(|| {
            let path = crate::default_scripts::studio_dir()?
                .join("PlatformContent/pc/terrain/materials.json");
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
            let mut out = std::collections::HashMap::new();
            for m in json.get("materials")?.as_array()? {
                let (Some(name), Some(c)) = (
                    m.get("name").and_then(|n| n.as_str()),
                    m.get("base_color").and_then(|c| c.as_array()),
                ) else {
                    continue;
                };
                let ch = |i: usize| c.get(i).and_then(|v| v.as_u64()).unwrap_or(0).min(255) as u8;
                out.insert(name.to_string(), [ch(0), ch(1), ch(2)]);
            }
            Some(out)
        })
        .as_ref()
}

/// Flat colour per material: Studio's default (`studio_base_colors`) when
/// its install is found, else our own estimate picked by eye. A place's
/// `Terrain.MaterialColors` blob (23 RGB triples in this same slot order,
/// first two reserved) overrides them.
pub fn material_color(m: u8) -> [u8; 3] {
    if let Some(c) = studio_base_colors().and_then(|t| t.get(material_name(m))) {
        return *c;
    }
    match m {
        0 => [0, 0, 0],
        1 => [44, 112, 158],   // Water
        2 => [98, 138, 60],    // Grass
        3 => [64, 68, 72],     // Slate
        4 => [128, 126, 121],  // Concrete
        5 => [138, 74, 54],    // Brick
        6 => [198, 182, 132],  // Sand
        7 => [146, 108, 70],   // WoodPlanks
        8 => [104, 100, 94],   // Rock
        9 => [152, 198, 216],  // Glacier
        10 => [232, 237, 242], // Snow
        11 => [176, 122, 84],  // Sandstone
        12 => [82, 64, 48],    // Mud
        13 => [46, 43, 45],    // Basalt
        14 => [102, 84, 60],   // Ground
        15 => [196, 92, 44],   // CrackedLava
        16 => [72, 72, 74],    // Asphalt
        17 => [116, 110, 102], // Cobblestone
        18 => [168, 204, 232], // Ice
        19 => [112, 148, 64],  // LeafyGrass
        20 => [208, 204, 198], // Salt
        21 => [204, 194, 166], // Limestone
        22 => [148, 148, 143], // Pavement
        _ => [200, 0, 200],
    }
}

pub fn is_water(m: u8) -> bool {
    m == WATER
}

/// Occupancy byte -> fraction (byte 255 = full, byte 0 = 1/256, not empty).
#[inline]
pub fn occupancy_from_byte(b: u8) -> f32 {
    (b as f32 + 1.0) / 256.0
}

/// The occupancy byte Studio stores for a share (0..1): round(share × 255),
/// read back as (byte + 1) / 256; a share that rounds to 0 leaves the voxel
/// Air (None). Measured with ReadVoxels after FillBlock / FillBall in the
/// offline Studio (round 21, tools/fidelity/terrain_probe.luau): 0.25 reads
/// 0.254, 0.5 reads 0.504, 0.9727 stays 0.9727, and the ball's faintest rim
/// isn't written.
pub fn occupancy_to_byte(share: f32) -> Option<u8> {
    let b = (share.clamp(0.0, 1.0) * crate::mech::TERRAIN_OCCUPANCY_SCALE).round() as i32;
    (b > 0).then_some(b.min(255) as u8)
}

/// Centre of voxel `v` in world studs.
pub fn voxel_center(v: [i32; 3]) -> [f32; 3] {
    v.map(|c| (c as f32 + 0.5) * VOXEL)
}

#[inline]
fn lidx(x: usize, y: usize, z: usize) -> usize {
    x + CS * (z + CS * y)
}

#[inline]
fn chunk_key(v: [i32; 3]) -> [i32; 3] {
    v.map(|c| c >> CHUNK_SHIFT)
}

#[inline]
fn local_index(v: [i32; 3]) -> usize {
    let m = (CS - 1) as i32;
    lidx(
        (v[0] & m) as usize,
        (v[1] & m) as usize,
        (v[2] & m) as usize,
    )
}

struct Chunk {
    mat: Box<[u8]>,
    /// Raw occupancy byte (meaningless where mat == 0).
    occ: Box<[u8]>,
    /// Shoreline water byte + 1 (0 = none); only allocated if the chunk has any.
    water: Option<Box<[u16]>>,
}

impl Chunk {
    fn new() -> Self {
        Chunk {
            mat: vec![0; CV].into_boxed_slice(),
            occ: vec![0; CV].into_boxed_slice(),
            water: None,
        }
    }

    fn write(&mut self, i: usize, n: usize, mat: u8, occ: u8, water: u16) {
        self.mat[i..i + n].fill(mat);
        self.occ[i..i + n].fill(occ);
        if water != 0 || self.water.is_some() {
            let w = self
                .water
                .get_or_insert_with(|| vec![0; CV].into_boxed_slice());
            w[i..i + n].fill(water);
        }
    }
}

/// Sparse voxel grid, stored as 32^3 chunks keyed by chunk coordinate.
/// `Voxels::get` with the last chunk remembered (see `Voxels::sampler`).
pub struct Sampler<'a> {
    v: &'a Voxels,
    last: std::cell::Cell<Option<([i32; 3], Option<&'a Chunk>)>>,
}

impl Sampler<'_> {
    pub fn get(&self, x: i32, y: i32, z: i32) -> (u8, f32) {
        let v = [x, y, z];
        let k = chunk_key(v);
        let c = match self.last.get() {
            Some((lk, c)) if lk == k => c,
            _ => {
                let c = self.v.chunks.get(&k);
                self.last.set(Some((k, c)));
                c
            }
        };
        match c {
            None => (0, 0.0),
            Some(c) => {
                let i = local_index(v);
                match c.mat[i] {
                    0 => (0, 0.0),
                    m => (m, occupancy_from_byte(c.occ[i])),
                }
            }
        }
    }
}

pub struct Voxels {
    chunks: HashMap<[i32; 3], Chunk>,
    count: usize,
}

impl std::fmt::Debug for Voxels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Voxels")
            .field("chunks", &self.chunks.len())
            .field("count", &self.count)
            .finish()
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, String> {
        let v = *self
            .b
            .get(self.pos)
            .ok_or_else(|| format!("SmoothGrid: truncated at byte {}", self.pos))?;
        self.pos += 1;
        Ok(v)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self
            .b
            .get(self.pos..self.pos + n)
            .ok_or_else(|| format!("SmoothGrid: truncated at byte {}", self.pos))?;
        self.pos += n;
        Ok(s)
    }
}

impl Voxels {
    pub fn empty() -> Self {
        Voxels {
            chunks: HashMap::new(),
            count: 0,
        }
    }

    /// Decodes a SmoothGrid blob. An empty blob is an empty grid.
    pub fn decode(bytes: &[u8]) -> Result<Voxels, String> {
        let mut out = Voxels::empty();
        if bytes.is_empty() {
            return Ok(out);
        }
        if bytes.len() < 2 {
            return Err("SmoothGrid: truncated header".into());
        }
        let version = bytes[0];
        if version != 1 {
            return Err(format!(
                "SmoothGrid: unsupported version {version} (only version 1 is known)"
            ));
        }
        let exp = bytes[1] as u32;
        if exp > 8 {
            return Err(format!(
                "SmoothGrid: chunk size exponent {exp} out of range"
            ));
        }
        let edge = 1usize << exp;
        let cells = edge * edge * edge;
        let mut r = Reader { b: bytes, pos: 2 };
        let mut cur = [0i32; 3];
        while r.pos < bytes.len() {
            let off = r.take(12)?;
            let mut d = [0u32; 3];
            for k in 0..4 {
                for a in 0..3 {
                    d[a] = (d[a] << 8) | off[k * 3 + a] as u32;
                }
            }
            for a in 0..3 {
                cur[a] = cur[a].wrapping_add(d[a] as i32);
            }
            let origin = cur.map(|c| c as i64 * edge as i64);
            if origin.iter().any(|o| o.abs() + edge as i64 > COORD_LIMIT) {
                return Err(format!(
                    "SmoothGrid: chunk {cur:?} is outside the supported range"
                ));
            }
            let origin = origin.map(|o| o as i32);
            let mut i = 0usize;
            while i < cells {
                let at = r.pos;
                let flag = r.byte()?;
                let mat = flag & 0x3f;
                if mat as usize >= MATERIALS.len() {
                    return Err(format!(
                        "SmoothGrid: unknown material index {mat} at byte {at}"
                    ));
                }
                let occ = if flag & 0x40 != 0 { r.byte()? } else { 255 };
                let mut n = 1usize;
                let mut water = 0u16;
                if flag & 0x80 != 0 {
                    let c = r.byte()?;
                    if c == 0 {
                        water = r.byte()? as u16 + 1;
                    } else {
                        n = c as usize + 1;
                    }
                }
                if i + n > cells {
                    return Err(format!(
                        "SmoothGrid: run at byte {at} runs past the end of its chunk"
                    ));
                }
                if mat != 0 {
                    // Shoreline water only exists beside a partial solid.
                    if mat == WATER || occ == 255 {
                        water = 0;
                    }
                    if exp == CHUNK_SHIFT {
                        out.chunk_mut(cur).write(i, n, mat, occ, water);
                    } else {
                        for j in i..i + n {
                            let l = [j % edge, j / (edge * edge), (j / edge) % edge];
                            let v = [
                                origin[0] + l[0] as i32,
                                origin[1] + l[1] as i32,
                                origin[2] + l[2] as i32,
                            ];
                            out.chunk_mut(chunk_key(v))
                                .write(local_index(v), 1, mat, occ, water);
                        }
                    }
                }
                i += n;
            }
        }
        out.finish();
        Ok(out)
    }

    fn chunk_mut(&mut self, key: [i32; 3]) -> &mut Chunk {
        self.chunks.entry(key).or_insert_with(Chunk::new)
    }

    fn finish(&mut self) {
        self.chunks.retain(|_, c| c.mat.iter().any(|&m| m != 0));
        self.count = self
            .chunks
            .values()
            .map(|c| c.mat.iter().filter(|&&m| m != 0).count())
            .sum();
    }

    /// Writes one voxel: SmoothGrid material index and occupancy (0..1);
    /// material 0 (Air) clears it.
    pub fn set(&mut self, v: [i32; 3], mat: u8, occupancy: f32) {
        self.set_many([(v, mat, occupancy)]);
    }

    /// Writes many voxels (Terrain:WriteVoxels): (voxel, material, occupancy),
    /// as `set`, with one recount at the end.
    pub fn set_many(&mut self, voxels: impl IntoIterator<Item = ([i32; 3], u8, f32)>) {
        for (v, mat, occupancy) in voxels {
            // No occupancy is Air whatever the material.
            let (mat, occ) = match occupancy_to_byte(occupancy) {
                Some(b) if mat != 0 => (mat, b),
                _ => (0, 0),
            };
            self.chunk_mut(chunk_key(v)).write(local_index(v), 1, mat, occ, 0);
        }
        self.finish();
    }

    /// Writes voxels as `set_many`, keeping the count as it goes (no full
    /// recount): the Terrain Editor's brush, every frame on big places.
    pub fn edit(&mut self, voxels: impl IntoIterator<Item = ([i32; 3], u8, f32)>) {
        let mut touched: std::collections::HashSet<[i32; 3]> = std::collections::HashSet::new();
        for (v, mat, occupancy) in voxels {
            let (mat, occ) = match occupancy_to_byte(occupancy) {
                Some(b) if mat != 0 => (mat, b),
                _ => (0, 0),
            };
            let (old, _) = self.get(v[0], v[1], v[2]);
            if old == 0 && mat == 0 {
                continue;
            }
            let key = chunk_key(v);
            self.chunk_mut(key).write(local_index(v), 1, mat, occ, 0);
            self.count = self.count + usize::from(mat != 0) - usize::from(old != 0);
            touched.insert(key);
        }
        for k in touched {
            if self.chunks.get(&k).is_some_and(|c| c.mat.iter().all(|&m| m == 0)) {
                self.chunks.remove(&k);
            }
        }
    }

    /// Number of non-Air voxels (Water included).
    pub fn count(&self) -> usize {
        self.count
    }

    /// Number of stored (non-empty) 32^3 chunks.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Material and solid occupancy (0..1) of a voxel; Air is `(0, 0.0)`.
    pub fn get(&self, x: i32, y: i32, z: i32) -> (u8, f32) {
        let v = [x, y, z];
        match self.chunks.get(&chunk_key(v)) {
            None => (0, 0.0),
            Some(c) => {
                let i = local_index(v);
                match c.mat[i] {
                    0 => (0, 0.0),
                    m => (m, occupancy_from_byte(c.occ[i])),
                }
            }
        }
    }

    /// Whether a world point is in water (a Water voxel, or shoreline water
    /// in a solid one).
    pub fn water_at(&self, p: [f32; 3]) -> bool {
        let v = p.map(|x| (x / VOXEL).floor() as i32);
        let Some(c) = self.chunks.get(&chunk_key(v)) else {
            return false;
        };
        let i = local_index(v);
        c.mat[i] == WATER || c.water.as_ref().is_some_and(|w| w[i] != 0)
    }

    /// Whether two grids hold the same solid (non-water) voxels, which is all
    /// collision is made of: a water fill (Place B's start) leaves it as it was.
    pub fn same_solid(&self, other: &Voxels) -> bool {
        let solid = |c: Option<&Chunk>, i: usize| match c {
            Some(c) if c.mat[i] != 0 && c.mat[i] != WATER => (c.mat[i], c.occ[i]),
            _ => (0, 0),
        };
        let keys: std::collections::HashSet<&[i32; 3]> =
            self.chunks.keys().chain(other.chunks.keys()).collect();
        keys.into_iter().all(|k| {
            let (a, b) = (self.chunks.get(k), other.chunks.get(k));
            (0..CV).all(|i| solid(a, i) == solid(b, i))
        })
    }

    /// A reader for many nearby `get`s (ray marches): it keeps the last
    /// chunk it looked up, so neighbouring voxels cost no hash lookup.
    pub fn sampler(&self) -> Sampler<'_> {
        Sampler {
            v: self,
            last: std::cell::Cell::new(None),
        }
    }

    /// Water occupancy (0..1): the occupancy of a Water voxel, or the shoreline
    /// water stored beside a partial solid.
    pub fn water(&self, x: i32, y: i32, z: i32) -> f32 {
        let v = [x, y, z];
        let Some(c) = self.chunks.get(&chunk_key(v)) else {
            return 0.0;
        };
        let i = local_index(v);
        if c.mat[i] == WATER {
            return occupancy_from_byte(c.occ[i]);
        }
        match &c.water {
            Some(w) if w[i] != 0 => w[i] as f32 / 256.0,
            _ => 0.0,
        }
    }

    /// Whether the 32³ storage chunk holding voxel `v` exists (it may still
    /// be all Air).
    pub fn chunk_has_voxels(&self, v: [i32; 3]) -> bool {
        self.chunks.contains_key(&chunk_key(v))
    }

    /// The lowest and highest voxel row any stored chunk covers (cheap: from
    /// the chunk keys, so it can be a little wider than the voxels).
    pub fn y_range(&self) -> Option<(i32, i32)> {
        let lo = self.chunks.keys().map(|k| k[1]).min()?;
        let hi = self.chunks.keys().map(|k| k[1]).max()?;
        Some((lo << CHUNK_SHIFT, (hi << CHUNK_SHIFT) + CS as i32 - 1))
    }

    /// Inclusive min/max voxel coordinates of non-Air voxels.
    pub fn bounds(&self) -> Option<([i32; 3], [i32; 3])> {
        let mut lo = [i32::MAX; 3];
        let mut hi = [i32::MIN; 3];
        for (k, c) in &self.chunks {
            let base = k.map(|c| c << CHUNK_SHIFT);
            for y in 0..CS {
                for z in 0..CS {
                    let s = lidx(0, y, z);
                    let row = &c.mat[s..s + CS];
                    let Some(x0) = row.iter().position(|&m| m != 0) else {
                        continue;
                    };
                    let x1 = row.iter().rposition(|&m| m != 0).unwrap_or(x0);
                    let a = [base[0] + x0 as i32, base[1] + y as i32, base[2] + z as i32];
                    let b = [base[0] + x1 as i32, a[1], a[2]];
                    for i in 0..3 {
                        lo[i] = lo[i].min(a[i]);
                        hi[i] = hi[i].max(b[i]);
                    }
                }
            }
        }
        (lo[0] <= hi[0]).then_some((lo, hi))
    }

    /// World-space (studs) box enclosing every non-Air voxel.
    pub fn world_bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        self.bounds().map(|(lo, hi)| {
            (
                lo.map(|c| c as f32 * VOXEL),
                hi.map(|c| (c + 1) as f32 * VOXEL),
            )
        })
    }

    /// Finds Workspace.Terrain and decodes its SmoothGrid. `None` when there is
    /// no Terrain or it has no SmoothGrid property.
    pub fn from_dom(dom: &rbx_dom_weak::WeakDom) -> Option<Result<Voxels, String>> {
        let ws = dom
            .root()
            .children()
            .iter()
            .filter_map(|r| dom.get_by_ref(*r))
            .find(|i| i.class.as_str() == "Workspace")?;
        let terrain = ws
            .children()
            .iter()
            .filter_map(|r| dom.get_by_ref(*r))
            .find(|i| i.class.as_str() == "Terrain")?;
        match terrain.properties.get(&rbx_dom_weak::ustr("SmoothGrid"))? {
            rbx_types::Variant::BinaryString(b) => Some(Voxels::decode(b.as_ref())),
            rbx_types::Variant::SharedString(s) => Some(Voxels::decode(s.data())),
            other => Some(Err(format!(
                "SmoothGrid has unexpected type {:?}",
                other.ty()
            ))),
        }
    }
}

// ---------------------------------------------------------------- editing

impl Voxels {
    /// The SmoothGrid blob `decode` reads: version 1, 32³ chunks, each a
    /// 12-byte offset from the previous chunk (four bytes per axis,
    /// interleaved, most significant first) then runs of
    /// `flag (material | 0x40 occupancy byte | 0x80 count byte) [occ] [n - 1]`,
    /// a count byte of 0 carrying one voxel's shoreline water instead.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![1u8, CHUNK_SHIFT as u8];
        let mut keys: Vec<[i32; 3]> = self.chunks.keys().copied().collect();
        keys.sort();
        let mut prev = [0i32; 3];
        for key in keys {
            let c = &self.chunks[&key];
            let d = [0, 1, 2].map(|a| key[a].wrapping_sub(prev[a]) as u32);
            prev = key;
            for k in 0..4 {
                for v in d {
                    out.push((v >> (24 - 8 * k)) as u8);
                }
            }
            let water = |i: usize| c.water.as_ref().map_or(0, |w| w[i]);
            let mut i = 0usize;
            while i < CV {
                let (mat, occ) = (c.mat[i], if c.mat[i] == 0 { 255 } else { c.occ[i] });
                let w = if mat != 0 && mat != WATER && occ != 255 { water(i) } else { 0 };
                let mut flag = mat;
                if occ != 255 {
                    flag |= 0x40;
                }
                if w != 0 {
                    out.extend([flag | 0x80]);
                    if occ != 255 {
                        out.push(occ);
                    }
                    out.extend([0, (w - 1) as u8]);
                    i += 1;
                    continue;
                }
                let mut n = 1usize;
                while i + n < CV
                    && n < 256
                    && c.mat[i + n] == mat
                    && (mat == 0 || c.occ[i + n] == occ)
                    && (mat == 0 || mat == WATER || occ == 255 || water(i + n) == 0)
                {
                    n += 1;
                }
                out.push(if n > 1 { flag | 0x80 } else { flag });
                if occ != 255 {
                    out.push(occ);
                }
                if n > 1 {
                    out.push((n - 1) as u8);
                }
                i += n;
            }
        }
        out
    }

    /// Terrain:Fill*: every voxel near `lo..hi` (world studs) takes the
    /// share of it inside the shape, from `inside` (signed distance in studs
    /// from the voxel's centre to the shape's surface, negative inside) as
    /// 0.5 − d / VOXEL. A material takes the voxel where it reaches it
    /// (occupancy the larger of the two); Air carves it (1 − share at most).
    pub fn fill(&mut self, lo: [f32; 3], hi: [f32; 3], mat: u8, inside: impl Fn([f32; 3]) -> f32) {
        let a = [0, 1, 2].map(|k| ((lo[k] / VOXEL).floor() as i32) - 1);
        let b = [0, 1, 2].map(|k| ((hi[k] / VOXEL).ceil() as i32) + 1);
        for y in a[1]..=b[1] {
            for z in a[2]..=b[2] {
                for x in a[0]..=b[0] {
                    let share = (0.5 - inside(voxel_center([x, y, z])) / VOXEL).clamp(0.0, 1.0);
                    if occupancy_to_byte(share).is_none() {
                        continue;
                    }
                    let v = [x, y, z];
                    let (old, occ) = self.get(x, y, z);
                    let (m, o) = if mat == 0 {
                        let left = occ.min(1.0 - share);
                        if old == 0 || left < 1.0 / 256.0 {
                            (0, 0.0)
                        } else {
                            (old, left)
                        }
                    } else {
                        (mat, if old == 0 { share } else { occ.max(share) })
                    };
                    self.write_one(v, m, o);
                }
            }
        }
        self.finish();
    }

    /// Terrain:ReplaceMaterial: `from` becomes `to` inside the voxel box.
    pub fn replace(&mut self, a: [i32; 3], b: [i32; 3], from: u8, to: u8) {
        for y in a[1]..b[1] {
            for z in a[2]..b[2] {
                for x in a[0]..b[0] {
                    let (m, o) = self.get(x, y, z);
                    if m == from && m != 0 {
                        self.write_one([x, y, z], to, if to == 0 { 0.0 } else { o });
                    }
                }
            }
        }
        self.finish();
    }

    /// Terrain:Clear.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.count = 0;
    }

    fn write_one(&mut self, v: [i32; 3], mat: u8, occupancy: f32) {
        let (mat, occ) = match occupancy_to_byte(occupancy) {
            Some(b) if mat != 0 => (mat, b),
            _ => (0, 0),
        };
        self.chunk_mut(chunk_key(v)).write(local_index(v), 1, mat, occ, 0);
    }
}

// ------------------------------------------------- raw cells (round 25)

impl Voxels {
    /// A voxel's stored bytes: material, occupancy byte (255 = full) and
    /// shoreline water (byte + 1, 0 = none); Air is (0, 0, 0). Region copies
    /// (terrain_region.rs) move these as they are, so copy and paste don't
    /// round occupancies through `occupancy_to_byte`.
    pub fn get_raw(&self, v: [i32; 3]) -> (u8, u8, u16) {
        match self.chunks.get(&chunk_key(v)) {
            None => (0, 0, 0),
            Some(c) => {
                let i = local_index(v);
                match c.mat[i] {
                    0 => (0, 0, 0),
                    m => (m, c.occ[i], c.water.as_ref().map_or(0, |w| w[i])),
                }
            }
        }
    }

    /// Writes stored bytes as `get_raw` reads them (material 0 is Air),
    /// with one recount at the end.
    pub fn set_raw_many(&mut self, cells: impl IntoIterator<Item = ([i32; 3], (u8, u8, u16))>) {
        for (v, (mat, occ, water)) in cells {
            let (occ, water) = if mat == 0 { (0, 0) } else { (occ, water) };
            let key = chunk_key(v);
            if mat == 0 && !self.chunks.contains_key(&key) {
                continue;
            }
            self.chunk_mut(key).write(local_index(v), 1, mat, occ, water);
        }
        self.finish();
    }
}

// ---------------------------------------------------------------- meshing

pub struct MaterialMesh {
    pub material: u8,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

/// Padded sample grid per chunk: local voxels -1..=33.
const P: usize = CS + 3;
const PP: usize = P * P;
/// Cells per axis: cell min-corners at local -1..=31.
const NC: usize = CS + 1;
const STRIDE: [usize; 3] = [1, P, PP];
const NO_VERT: u32 = u32::MAX;
const NO_MAT: u8 = u8::MAX;

const CUBE_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

#[inline]
fn pidx(x: usize, y: usize, z: usize) -> usize {
    x + P * y + PP * z
}

#[inline]
fn cidx(c: [usize; 3]) -> usize {
    c[0] + NC * (c[1] + NC * c[2])
}

/// Sample fields for one chunk plus a one/two voxel apron.
struct Grid {
    solid: Vec<f32>,
    water: Vec<f32>,
    mat: Vec<u8>,
    occ: [f32; 256],
}

#[derive(Default)]
struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

struct LocalVert {
    pos: [f32; 3],
    nrm: [f32; 3],
    mat: u8,
    idx: u32,
}

struct State {
    /// Per field (0 solid, 1 water): cell -> local vertex id.
    cell: [Vec<u32>; 2],
    local: Vec<LocalVert>,
    /// (local vertex, material) for vertices shared by several materials.
    overflow: HashMap<(u32, u8), u32>,
    /// Vertices of chunk-border cells, welded across chunks.
    seam: HashMap<(u8, u8, [i32; 3]), u32>,
    builders: Vec<Builder>,
}

fn copy_range(d: i32) -> (usize, usize, usize) {
    // (source start, padded destination start, length)
    match d {
        -1 => (CS - 1, 0, 1),
        0 => (0, 1, CS),
        _ => (0, CS + 1, 2),
    }
}

impl Grid {
    fn new() -> Self {
        let mut occ = [0.0; 256];
        for (b, o) in occ.iter_mut().enumerate() {
            *o = occupancy_from_byte(b as u8);
        }
        Grid {
            solid: vec![0.0; P * P * P],
            water: vec![0.0; P * P * P],
            mat: vec![0; P * P * P],
            occ,
        }
    }

    /// Fills the padded grid for chunk `key`; returns (any solid, all solid, any water).
    fn fill(&mut self, v: &Voxels, key: [i32; 3]) -> (bool, bool, bool) {
        self.solid.fill(0.0);
        self.water.fill(0.0);
        self.mat.fill(0);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let Some(ch) = v.chunks.get(&[key[0] + dx, key[1] + dy, key[2] + dz]) else {
                        continue;
                    };
                    let (rx, ry, rz) = (copy_range(dx), copy_range(dy), copy_range(dz));
                    for z in 0..rz.2 {
                        for y in 0..ry.2 {
                            let s = lidx(rx.0, ry.0 + y, rz.0 + z);
                            let d = pidx(rx.1, ry.1 + y, rz.1 + z);
                            for x in 0..rx.2 {
                                let m = ch.mat[s + x];
                                if m == 0 {
                                    continue;
                                }
                                let o = self.occ[ch.occ[s + x] as usize];
                                self.mat[d + x] = m;
                                if m == WATER {
                                    // Water as solid (round 21): any water
                                    // voxel is inside, its occupancy only
                                    // moves the surface, so part-filled
                                    // voxels in a lake make no inner faces.
                                    self.water[d + x] = 0.5 + 0.5 * o;
                                } else {
                                    // Any occupied voxel is inside; its
                                    // occupancy moves the surface (an edge to
                                    // air crosses at o / (1 + o) of the way):
                                    // Studio draws voxels under 0.5 too
                                    // (terrain_probe.luau, round 21).
                                    self.solid[d + x] = 0.5 + 0.5 * o;
                                    if let Some(w) = &ch.water {
                                        if w[s + x] != 0 {
                                            self.water[d + x] = 0.5 + 0.5 * w[s + x] as f32 / 256.0;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let (mut any_s, mut all_s, mut any_w) = (false, true, false);
        for (s, w) in self.solid.iter().zip(&self.water) {
            let i = *s >= ISO;
            any_s |= i;
            all_s &= i;
            any_w |= *w >= ISO;
        }
        (any_s, all_s, any_w)
    }

    /// Surface-nets vertex for cell `c` (min corner at padded sample `c`):
    /// average of the iso crossings on its edges, normal from the density
    /// gradient. Returns world-space position and normal.
    fn cell_vertex(&self, field: usize, c: [usize; 3], key: [i32; 3]) -> ([f32; 3], [f32; 3]) {
        let f = if field == 0 { &self.solid } else { &self.water };
        let base = pidx(c[0], c[1], c[2]);
        let mut d = [0f32; 8];
        for (i, di) in d.iter_mut().enumerate() {
            *di = f[base + (i & 1) + ((i >> 1) & 1) * P + ((i >> 2) & 1) * PP];
        }
        let mut sum = [0f32; 3];
        let mut n = 0u32;
        for &(a, b) in &CUBE_EDGES {
            let (da, db) = (d[a], d[b]);
            if (da >= ISO) != (db >= ISO) {
                // Solid: the surface crosses an edge from an occupied voxel
                // to air at its occupancy's share of the way (solid samples
                // hold 0.5 + 0.5 × occupancy). Studio's lone voxels top out
                // at 3.33 / 2.67 / 2.33 / 2.14 for occupancy 1 / .5 / .25 /
                // .1 and a full slab's top lies half a voxel above it
                // (terrain_probe.luau, round 21): exactly this.
                // Water samples are kept the same way.
                let t = if da >= ISO {
                    ((da - 0.5) * 2.0).clamp(0.0, 1.0)
                } else {
                    1.0 - ((db - 0.5) * 2.0).clamp(0.0, 1.0)
                };
                for (k, s) in sum.iter_mut().enumerate() {
                    let ca = ((a >> k) & 1) as f32;
                    let cb = ((b >> k) & 1) as f32;
                    *s += ca + (cb - ca) * t;
                }
                n += 1;
            }
        }
        let mut fp = if n > 0 {
            sum.map(|s| s / n as f32)
        } else {
            [0.5; 3]
        };
        let nrm = gradient_normal(&d, fp)
            .or_else(|| gradient_normal(&d, [0.5; 3]))
            .unwrap_or([0.0, 1.0, 0.0]);
        // The material's shape (Studio's terrain table): cubify puts the
        // vertex on the voxel grid's corner (box edges), quantize snaps it,
        // shift jitters it by a fixed random amount.
        let gc = [0, 1, 2].map(|k| (key[k] << CHUNK_SHIFT) + c[k] as i32 - 1);
        if field == 0 {
            let shape = shape_of(self.cell_material(c));
            if shape.cubify {
                fp = [0.5; 3];
            }
            if shape.quantize > 0.0 {
                fp = fp.map(|f| ((f - 0.5) / shape.quantize).round() * shape.quantize + 0.5);
            }
            if shape.shift > 0.0 {
                let r = [cell_rand(gc), cell_rand([gc[1], gc[2], gc[0]]), cell_rand([gc[2], gc[0], gc[1]])];
                for k in 0..3 {
                    fp[k] = (fp[k] + (r[k] as f32 / 255.0 - 0.5) * shape.shift).clamp(0.0, 1.0);
                }
            }
        }
        let mut pos = [0f32; 3];
        for k in 0..3 {
            let voxel = ((key[k] as i64) << CHUNK_SHIFT) + c[k] as i64 - 1;
            pos[k] = ((voxel as f64 + fp[k] as f64 + 0.5) * VOXEL as f64) as f32;
        }
        (pos, nrm)
    }
}

/// How a material bends the surface (Studio's terrain table: shift,
/// quantize in voxels; cubify; hard / hardsoft = faceted shading).
#[derive(Clone, Copy, Debug, Default)]
pub struct Shape {
    pub shift: f32,
    pub quantize: f32,
    pub cubify: bool,
    pub faceted: bool,
}

static SHAPE_2022: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Which table the mesher's material shapes come from
/// (MaterialService.Use2022Materials of the place being meshed).
pub fn set_use2022(on: bool) {
    SHAPE_2022.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn shape_of(m: u8) -> Shape {
    let use2022 = SHAPE_2022.load(std::sync::atomic::Ordering::Relaxed);
    crate::terrain_mat::material(m, use2022).map(|t| t.shape).unwrap_or_default()
}

/// Outward normal (minus the trilinear density gradient) at `f` in the cell.
fn gradient_normal(d: &[f32; 8], f: [f32; 3]) -> Option<[f32; 3]> {
    let [fx, fy, fz] = f;
    let (gx_, gy_, gz_) = (1.0 - fx, 1.0 - fy, 1.0 - fz);
    let gx = gy_ * gz_ * (d[1] - d[0])
        + fy * gz_ * (d[3] - d[2])
        + gy_ * fz * (d[5] - d[4])
        + fy * fz * (d[7] - d[6]);
    let gy = gx_ * gz_ * (d[2] - d[0])
        + fx * gz_ * (d[3] - d[1])
        + gx_ * fz * (d[6] - d[4])
        + fx * fz * (d[7] - d[5]);
    let gz = gx_ * gy_ * (d[4] - d[0])
        + fx * gy_ * (d[5] - d[1])
        + gx_ * fy * (d[6] - d[2])
        + fx * fy * (d[7] - d[3]);
    let len = (gx * gx + gy * gy + gz * gz).sqrt();
    (len > 1e-6).then(|| [-gx / len, -gy / len, -gz / len])
}

impl State {
    fn new() -> Self {
        State {
            cell: [vec![NO_VERT; NC * NC * NC], vec![NO_VERT; NC * NC * NC]],
            local: Vec::new(),
            overflow: HashMap::new(),
            seam: HashMap::new(),
            builders: (0..64).map(|_| Builder::default()).collect(),
        }
    }

    fn reset_chunk(&mut self) {
        self.cell[0].fill(NO_VERT);
        self.cell[1].fill(NO_VERT);
        self.local.clear();
        self.overflow.clear();
    }

    /// Index of cell `c`'s vertex in material `mat`'s mesh, creating it on demand.
    fn index(&mut self, g: &Grid, key: [i32; 3], field: usize, c: [usize; 3], mat: u8) -> u32 {
        let ci = cidx(c);
        let mut lv = self.cell[field][ci];
        if lv == NO_VERT {
            let (pos, nrm) = g.cell_vertex(field, c, key);
            lv = self.local.len() as u32;
            self.local.push(LocalVert {
                pos,
                nrm,
                mat: NO_MAT,
                idx: 0,
            });
            self.cell[field][ci] = lv;
        }
        let l = &self.local[lv as usize];
        if l.mat == mat {
            return l.idx;
        }
        let (pos, nrm, first_free) = (l.pos, l.nrm, l.mat == NO_MAT);
        let builders = &mut self.builders;
        let mut push = || {
            let b = &mut builders[mat as usize];
            b.positions.push(pos);
            b.normals.push(nrm);
            (b.positions.len() - 1) as u32
        };
        let idx = if c.iter().any(|&k| k == 0 || k == NC - 1) {
            let gc = [0, 1, 2].map(|k| (key[k] << CHUNK_SHIFT) + c[k] as i32 - 1);
            *self.seam.entry((field as u8, mat, gc)).or_insert_with(push)
        } else if first_free {
            push()
        } else {
            *self.overflow.entry((lv, mat)).or_insert_with(push)
        };
        if first_free {
            let l = &mut self.local[lv as usize];
            l.mat = mat;
            l.idx = idx;
        }
        idx
    }

    /// Emits the quads for every sign-changing edge whose low sample lies in
    /// this chunk (padded 1..=32 on each axis).
    fn edges(&mut self, g: &Grid, key: [i32; 3], field: usize) {
        let f = if field == 0 { &g.solid } else { &g.water };
        for z in 1..=CS {
            for y in 1..=CS {
                for x in 1..=CS {
                    let p = pidx(x, y, z);
                    let a_in = f[p] >= ISO;
                    for axis in 0..3 {
                        let q = p + STRIDE[axis];
                        if (f[q] >= ISO) == a_in {
                            continue;
                        }
                        let (inside, outside) = if a_in { (p, q) } else { (q, p) };
                        let mat = if field == 1 {
                            // Water hidden behind solid gets no surface.
                            if g.solid[outside] >= ISO {
                                continue;
                            }
                            WATER
                        } else {
                            g.mat[inside]
                        };
                        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                        let c00 = [x, y, z];
                        let mut c10 = c00;
                        c10[u] -= 1;
                        let mut c01 = c00;
                        c01[v] -= 1;
                        let mut c11 = c10;
                        c11[v] -= 1;
                        let ids = [
                            self.index(g, key, field, c00, mat),
                            self.index(g, key, field, c10, mat),
                            self.index(g, key, field, c11, mat),
                            self.index(g, key, field, c01, mat),
                        ];
                        // c00 -> c10 -> c11 -> c01 winds counter-clockwise about +axis.
                        let q4 = if a_in {
                            ids
                        } else {
                            [ids[0], ids[3], ids[2], ids[1]]
                        };
                        let b = &mut self.builders[mat as usize];
                        let pp = |i: u32| b.positions[i as usize];
                        if dist2(pp(q4[0]), pp(q4[2])) >= dist2(pp(q4[1]), pp(q4[3])) {
                            b.indices
                                .extend_from_slice(&[q4[0], q4[1], q4[2], q4[0], q4[2], q4[3]]);
                        } else {
                            b.indices
                                .extend_from_slice(&[q4[0], q4[1], q4[3], q4[1], q4[2], q4[3]]);
                        }
                    }
                }
            }
        }
    }
}

#[inline]
fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Smooth surface (surface nets over occupancy, iso 0.5, samples at voxel
/// centres) in world studs, one mesh per material. Water is its own mesh
/// (material 1) and only covers water that faces non-solid space. Vertices are
/// welded per material, including across chunk borders. Triangles wind
/// counter-clockwise seen from outside.
pub fn build_meshes(v: &Voxels) -> Vec<MaterialMesh> {
    build_fields(v, [true, true])
}

/// The water surface alone (the renderer draws solid terrain by chunk).
pub fn build_water_meshes(v: &Voxels) -> Vec<MaterialMesh> {
    build_fields(v, [false, true])
}

fn build_fields(v: &Voxels, fields: [bool; 2]) -> Vec<MaterialMesh> {
    if v.chunks.is_empty() {
        return Vec::new();
    }
    // A chunk owns edges whose low end is inside it, so chunks just below a
    // stored chunk on each axis can own surface too.
    let mut keys: Vec<[i32; 3]> = Vec::with_capacity(v.chunks.len() * 4);
    for k in v.chunks.keys() {
        keys.push(*k);
        for a in 0..3 {
            let mut n = *k;
            n[a] -= 1;
            keys.push(n);
        }
    }
    keys.sort_unstable();
    keys.dedup();

    let mut g = Grid::new();
    let mut st = State::new();
    for key in keys {
        let (any_s, all_s, any_w) = g.fill(v, key);
        if !any_w && (!any_s || all_s) {
            continue;
        }
        st.reset_chunk();
        if fields[0] && any_s && !all_s {
            st.edges(&g, key, 0);
        }
        if fields[1] && any_w {
            st.edges(&g, key, 1);
        }
    }
    st.builders
        .into_iter()
        .enumerate()
        .filter(|(_, b)| !b.indices.is_empty())
        .map(|(m, b)| MaterialMesh {
            material: m as u8,
            positions: b.positions,
            normals: b.normals,
            indices: b.indices,
        })
        .collect()
}

// ------------------------------------------------- render chunks (round 21)

/// A terrain vertex as Studio's terrain shader takes it (SmoothCluster VS):
/// every triangle carries its three corners' materials, projection axes and
/// random detiling, and each vertex says which corner it is (the GPU turns
/// that into barycentric weights). Vertices aren't shared.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertex {
    pub pos: [f32; 3],
    /// Normal × 127; w = which corner (0, 1, 2).
    pub normal: [i8; 4],
    /// The three corners' palette entries (material × 3 + face).
    pub palette: [u8; 4],
    /// The three corners' projection axes (0..17).
    pub axis: [u8; 4],
    /// The three corners' random bytes (detiling rotation / offset).
    pub rand: [u8; 4],
}

/// Palette entry of a material's face (Top 0, Side 1, Bottom 2).
pub fn palette(material: u8, face: u8) -> u8 {
    material * 3 + face
}

/// Studio's 18 projection axes: the six faces and the twelve cube edges.
pub const AXES: [[f32; 3]; 18] = {
    const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
    [
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
        [H, H, 0.0],
        [-H, H, 0.0],
        [H, -H, 0.0],
        [-H, -H, 0.0],
        [0.0, H, H],
        [0.0, -H, H],
        [0.0, H, -H],
        [0.0, -H, -H],
        [H, 0.0, H],
        [-H, 0.0, H],
        [H, 0.0, -H],
        [-H, 0.0, -H],
    ]
};

/// The projection axis nearest a normal (only the six faces for
/// cube-mapped materials: Brick, WoodPlanks).
pub fn nearest_axis(n: [f32; 3], cube: bool) -> u8 {
    let count = if cube { 6 } else { 18 };
    let mut best = (0u8, f32::MIN);
    for (i, a) in AXES.iter().take(count).enumerate() {
        let d = a[0] * n[0] + a[1] * n[1] + a[2] * n[2];
        if d > best.1 {
            best = (i as u8, d);
        }
    }
    best.0
}

/// Which face texture a corner with normal `n` shows.
fn face_of(n: [f32; 3]) -> u8 {
    if n[1] >= crate::mech::TERRAIN_TOP_NORMAL_Y {
        0
    } else if n[1] <= -crate::mech::TERRAIN_TOP_NORMAL_Y {
        2
    } else {
        1
    }
}

/// A stable random byte for a cell (the same in every triangle that shares it).
fn cell_rand(gc: [i32; 3]) -> u8 {
    let mut h = (gc[0] as u32).wrapping_mul(0x8da6_b343)
        ^ (gc[1] as u32).wrapping_mul(0xd816_3841)
        ^ (gc[2] as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xff) as u8
}

impl Grid {
    /// The material a cell's vertex shows: the one with the most occupancy
    /// among the cell's eight voxels, the lower SmoothGrid slot on a tie
    /// (round 21, against an offline Studio: its 4-material checker keeps
    /// Grass and Sand and shows Rock and Snow as small patches, and a lone
    /// Snow voxel on a Basalt slab stays a bump on Basalt).
    fn cell_material(&self, c: [usize; 3]) -> u8 {
        let base = pidx(c[0], c[1], c[2]);
        let mut sums: [(u8, f32); 8] = [(0, 0.0); 8];
        let mut n = 0;
        for i in 0..8 {
            let p = base + (i & 1) + ((i >> 1) & 1) * P + ((i >> 2) & 1) * PP;
            let m = self.mat[p];
            if m == 0 || m == WATER {
                continue;
            }
            let o = self.solid[p];
            match sums[..n].iter_mut().find(|s| s.0 == m) {
                Some(s) => s.1 += o,
                None => {
                    sums[n] = (m, o);
                    n += 1;
                }
            }
        }
        let mut best = (0u8, -1.0f32);
        for &(m, o) in &sums[..n] {
            if o > best.1 + 1e-6 || ((o - best.1).abs() <= 1e-6 && m < best.0) {
                best = (m, o);
            }
        }
        best.0
    }
}

/// The solid surface owned by chunk `key` as terrain-shader triangles (the
/// same cell vertices and quads as `build_meshes`, so the drawn shape is the
/// collision shape). `cube(material)` says which materials map as a cube.
pub fn chunk_triangles(v: &Voxels, key: [i32; 3], cube: &dyn Fn(u8) -> bool) -> Vec<TerrainVertex> {
    let mut g = Grid::new();
    let (any_s, all_s, _) = g.fill(v, key);
    if !any_s || all_s {
        return Vec::new();
    }
    struct Corner {
        pos: [f32; 3],
        nrm: [f32; 3],
        mat: u8,
        gc: [i32; 3],
    }
    let mut cells: HashMap<usize, Corner> = HashMap::new();
    let mut quads: Vec<[usize; 4]> = Vec::new();
    {
        let mut corner = |c: [usize; 3]| -> usize {
            let ci = cidx(c);
            cells.entry(ci).or_insert_with(|| {
                let (pos, nrm) = g.cell_vertex(0, c, key);
                let gc = [0, 1, 2].map(|k| (key[k] << CHUNK_SHIFT) + c[k] as i32 - 1);
                Corner { pos, nrm, mat: g.cell_material(c), gc }
            });
            ci
        };
        for z in 1..=CS {
            for y in 1..=CS {
                for x in 1..=CS {
                    let p = pidx(x, y, z);
                    let a_in = g.solid[p] >= ISO;
                    for axis in 0..3 {
                        let q = p + STRIDE[axis];
                        if (g.solid[q] >= ISO) == a_in {
                            continue;
                        }
                        let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
                        let c00 = [x, y, z];
                        let mut c10 = c00;
                        c10[u] -= 1;
                        let mut c01 = c00;
                        c01[w] -= 1;
                        let mut c11 = c10;
                        c11[w] -= 1;
                        let ids = [corner(c00), corner(c10), corner(c11), corner(c01)];
                        quads.push(if a_in { ids } else { [ids[0], ids[3], ids[2], ids[1]] });
                    }
                }
            }
        }
    }
    let mut out = Vec::with_capacity(quads.len() * 6);
    let mut tri = |t: [usize; 3]| {
        let c = t.map(|i| &cells[&i]);
        // Cells with no solid sample of their own take a neighbour's material.
        let fallback = c.iter().map(|k| k.mat).find(|&m| m != 0).unwrap_or(2);
        let mats = c.map(|k| if k.mat == 0 { fallback } else { k.mat });
        let pal = [0, 1, 2].map(|i| palette(mats[i], face_of(c[i].nrm)));
        let ax = [0, 1, 2].map(|i| nearest_axis(c[i].nrm, cube(mats[i])));
        let rnd = c.map(|k| cell_rand(k.gc));
        // Hard materials (Studio's "hard" / "hardsoft": Rock, Basalt, Brick…)
        // shade each triangle flat; the rest smooth.
        let (a, b, d) = (glam::Vec3::from(c[0].pos), glam::Vec3::from(c[1].pos), glam::Vec3::from(c[2].pos));
        let face = (b - a).cross(d - a).normalize_or_zero();
        for (i, k) in c.iter().enumerate() {
            let q = |f: f32| (f.clamp(-1.0, 1.0) * 127.0).round() as i8;
            let n = if shape_of(mats[i]).faceted && face != glam::Vec3::ZERO {
                face.to_array()
            } else {
                k.nrm
            };
            out.push(TerrainVertex {
                pos: k.pos,
                normal: [q(n[0]), q(n[1]), q(n[2]), i as i8],
                palette: [pal[0], pal[1], pal[2], 0],
                axis: [ax[0], ax[1], ax[2], 0],
                rand: [rnd[0], rnd[1], rnd[2], 0],
            });
        }
    };
    for q4 in quads {
        let pp = |i: usize| cells[&i].pos;
        if dist2(pp(q4[0]), pp(q4[2])) >= dist2(pp(q4[1]), pp(q4[3])) {
            tri([q4[0], q4[1], q4[2]]);
            tri([q4[0], q4[2], q4[3]]);
        } else {
            tri([q4[0], q4[1], q4[3]]);
            tri([q4[1], q4[2], q4[3]]);
        }
    }
    out
}

/// Terrain.Decoration grass for chunk `key` (round 21, after Studio's Grass
/// VS): blades standing on the chunk's Grass top faces, `length` = the
/// Terrain's GrassLength. Each blade is two triangles (both sides) of
/// terrain vertices flagged in `palette[3]` (1 = base, 2 = tip); a tip's
/// `rand[3]` holds the blade height × 32 so the vertex shader can sink it
/// into the ground with distance, as Studio's does.
pub fn grass_blades(v: &Voxels, key: [i32; 3], length: f32) -> Vec<TerrainVertex> {
    let tris = chunk_triangles(v, key, &|_| false);
    let grass_top = palette(2, 0);
    let mut out = Vec::new();
    let density = crate::mech::GRASS_BLADES_PER_STUD2;
    let base_h = (length * crate::mech::GRASS_HEIGHT_PER_LENGTH).max(0.05);
    for (ti, t) in tris.chunks_exact(3).enumerate() {
        if t[0].palette[..3].iter().any(|&p| p != grass_top) {
            continue;
        }
        let (a, b, c) = (glam::Vec3::from(t[0].pos), glam::Vec3::from(t[1].pos), glam::Vec3::from(t[2].pos));
        let n = (b - a).cross(c - a);
        let area = n.length() * 0.5;
        if area < 1e-4 || n.y / (2.0 * area) < crate::mech::TERRAIN_TOP_NORMAL_Y {
            continue;
        }
        // A fixed random stream per triangle (same blades every rebuild).
        let mut seed = (ti as u32).wrapping_mul(0x9e37_79b9) ^ (key[0] as u32).wrapping_mul(0x85eb_ca6b)
            ^ (key[1] as u32).wrapping_mul(0xc2b2_ae35) ^ (key[2] as u32).wrapping_mul(0x27d4_eb2f);
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed & 0x00ff_ffff) as f32 / 16_777_216.0
        };
        let count = (area * density + rnd()).floor() as usize;
        for _ in 0..count {
            let (mut u, mut w) = (rnd(), rnd());
            if u + w > 1.0 {
                u = 1.0 - u;
                w = 1.0 - w;
            }
            let p = a + (b - a) * u + (c - a) * w;
            let h = base_h * (0.6 + 0.8 * rnd());
            let ang = rnd() * std::f32::consts::TAU;
            let side = glam::Vec3::new(ang.cos(), 0.0, ang.sin()) * crate::mech::GRASS_BLADE_HALF_WIDTH;
            let lean = glam::Vec3::new(rnd() - 0.5, 0.0, rnd() - 0.5) * h * 0.6;
            let tip = p + glam::Vec3::Y * h + lean;
            let shade = (rnd() * 255.0) as u8;
            let hq = (h * 32.0).round().clamp(1.0, 255.0) as u8;
            let vert = |pos: glam::Vec3, corner: i8, flag: u8| TerrainVertex {
                pos: pos.to_array(),
                normal: [0, 127, 0, corner],
                palette: [grass_top, grass_top, grass_top, flag],
                axis: [2, 2, 2, 0],
                rand: [shade, shade, shade, if flag == 2 { hq } else { 0 }],
            };
            let (b1, b2) = (p - side, p + side);
            out.extend_from_slice(&[vert(b1, 0, 1), vert(b2, 1, 1), vert(tip, 2, 2)]);
            out.extend_from_slice(&[vert(b2, 0, 1), vert(b1, 1, 1), vert(tip, 2, 2)]);
        }
    }
    out
}

/// Whether chunk `key` has Grass on its top faces (decoration candidates).
pub fn chunk_has_grass_top(verts: &[TerrainVertex]) -> bool {
    let g = palette(2, 0);
    verts.chunks_exact(3).any(|t| t[0].palette[..3].iter().all(|&p| p == g))
}

/// The chunks a render must re-mesh after `old` became `new`: those whose
/// voxels changed and their neighbours (a chunk's surface reads one voxel
/// past its border), or every one when there was no old grid.
/// The render chunks to re-mesh after an edit inside the voxel box
/// `lo..=hi`: the chunks it touches (one voxel out) and their neighbours.
pub fn dirty_box(lo: [i32; 3], hi: [i32; 3]) -> Vec<[i32; 3]> {
    let a = chunk_key(lo.map(|c| c - 1));
    let b = chunk_key(hi.map(|c| c + 1));
    let mut v = Vec::new();
    for z in a[2] - 1..=b[2] + 1 {
        for y in a[1] - 1..=b[1] + 1 {
            for x in a[0] - 1..=b[0] + 1 {
                v.push([x, y, z]);
            }
        }
    }
    v
}

pub fn dirty_chunks(old: Option<&Voxels>, new: &Voxels) -> Vec<[i32; 3]> {
    let mut changed: Vec<[i32; 3]> = Vec::new();
    match old {
        None => changed.extend(new.chunks.keys().copied()),
        Some(o) => {
            for (k, c) in &new.chunks {
                match o.chunks.get(k) {
                    Some(oc) if oc.mat == c.mat && oc.occ == c.occ => {}
                    _ => changed.push(*k),
                }
            }
            changed.extend(o.chunks.keys().filter(|k| !new.chunks.contains_key(*k)).copied());
        }
    }
    let mut out: std::collections::HashSet<[i32; 3]> = std::collections::HashSet::new();
    for k in changed {
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    out.insert([k[0] + dx, k[1] + dy, k[2] + dz]);
                }
            }
        }
    }
    let mut v: Vec<[i32; 3]> = out.into_iter().collect();
    v.sort_unstable();
    v
}

impl Voxels {
    /// Whether chunk `key` or a chunk beside it holds any water (its surface
    /// depends on the solid voxels around it).
    pub fn has_water_near(&self, key: [i32; 3]) -> bool {
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if let Some(c) = self.chunks.get(&[key[0] + dx, key[1] + dy, key[2] + dz]) {
                        if c.mat.contains(&WATER) || c.water.is_some() {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// Which SmoothGrid materials the grid holds.
pub fn materials_present(v: &Voxels) -> [bool; 23] {
    let mut out = [false; 23];
    for c in v.chunks.values() {
        let mut seen = [false; 256];
        for &m in c.mat.iter() {
            seen[m as usize] = true;
        }
        for (i, o) in out.iter_mut().enumerate() {
            *o |= seen[i];
        }
    }
    out
}

/// Whether the water in two grids differs (so its mesh must be rebuilt).
pub fn water_changed(old: Option<&Voxels>, new: &Voxels) -> bool {
    let Some(o) = old else { return true };
    let water = |c: &Chunk| -> Vec<u8> {
        c.mat
            .iter()
            .zip(c.occ.iter())
            .enumerate()
            .map(|(i, (&m, &b))| {
                if m == WATER {
                    b.saturating_add(1)
                } else {
                    c.water.as_ref().map(|w| (w[i] != 0) as u8).unwrap_or(0) * 255
                }
            })
            .collect()
    };
    let keys: std::collections::HashSet<&[i32; 3]> = o.chunks.keys().chain(new.chunks.keys()).collect();
    keys.into_iter().any(|k| {
        let a = o.chunks.get(k).map(water).unwrap_or_default();
        let b = new.chunks.get(k).map(water).unwrap_or_default();
        let empty = |v: &Vec<u8>| v.iter().all(|&x| x == 0);
        if a.is_empty() || b.is_empty() {
            !(empty(&a) && empty(&b))
        } else {
            a != b
        }
    })
}

/// Collision geometry: every non-water mesh concatenated into one triangle soup
/// (vertices are not welded across materials).
pub fn collision_mesh(meshes: &[MaterialMesh]) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    for m in meshes.iter().filter(|m| !is_water(m.material)) {
        let base = pos.len() as u32;
        pos.extend_from_slice(&m.positions);
        idx.extend(m.indices.iter().map(|i| i + base));
    }
    (pos, idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// FillBlock / FillBall write what `encode` stores and `decode` reads back.
    #[test]
    fn fill_encode_round_trip() {
        let mut v = Voxels::empty();
        // A 64 × 8 × 64 slab of Grass whose top is at y = 2 (half a voxel up).
        v.fill([-32.0, -6.0, -32.0], [32.0, 2.0, 32.0], 2, |c| {
            let q = [c[0].abs() - 32.0, (c[1] + 2.0).abs() - 4.0, c[2].abs() - 32.0];
            let out = q.map(|x| x.max(0.0));
            (out[0] * out[0] + out[1] * out[1] + out[2] * out[2]).sqrt()
                + q[0].max(q[1]).max(q[2]).min(0.0)
        });
        // A ball of Sand, and a water voxel, and an Air carve.
        v.fill([36.0, -4.0, -4.0], [44.0, 4.0, 4.0], 6, |c| {
            ((c[0] - 40.0).powi(2) + c[1].powi(2) + c[2].powi(2)).sqrt() - 4.0
        });
        v.set([20, 5, 20], WATER, 0.5);
        v.fill([-4.0, -4.0, -4.0], [4.0, 4.0, 4.0], 0, |c| {
            (c[0].powi(2) + (c[1] + 2.0).powi(2) + c[2].powi(2)).sqrt() - 3.0
        });
        assert_eq!(v.get(5, -1, 5), (2, 1.0), "inside the slab: full Grass");
        let (m, top) = v.get(5, 0, 5);
        assert_eq!(m, 2);
        assert!((top - 0.5).abs() < 0.02, "the slab's top voxel is half full: {top}");
        assert_eq!(v.get(5, 1, 5).0, 0, "above the slab: Air");
        assert_eq!(v.get(10, 0, 0).0, 6, "the ball is Sand");
        assert!(v.get(0, -1, 0).1 < 1.0, "the Air ball carved the slab");
        let back = Voxels::decode(&v.encode()).expect("decode");
        assert_eq!(back.count(), v.count());
        for y in -3..3 {
            for z in -10..12 {
                for x in -10..12 {
                    assert_eq!(back.get(x, y, z), v.get(x, y, z), "voxel {x} {y} {z}");
                }
            }
        }
        assert_eq!(back.get(20, 5, 20).0, WATER);
        let mut c = back;
        c.clear();
        assert_eq!(c.count(), 0);
        assert_eq!(c.encode(), vec![1, CHUNK_SHIFT as u8]);
    }

    /// (material, occupancy byte, shoreline water byte)
    type Cell = (u8, u8, Option<u8>);

    /// Test-only encoder following the documented layout: chunks sorted by
    /// X, Y, Z; interleaved big-endian offsets; runs of up to 256.
    fn encode(cells: &HashMap<[i32; 3], Cell>) -> Vec<u8> {
        let mut chunks: BTreeMap<[i32; 3], Vec<Cell>> = BTreeMap::new();
        for (p, c) in cells {
            chunks
                .entry(chunk_key(*p))
                .or_insert_with(|| vec![(0, 255, None); CV])[local_index(*p)] = *c;
        }
        let norm = |c: Cell| -> Cell {
            match c {
                (0, _, _) => (0, 255, None),
                (m, o, _) if m == WATER || o == 255 => (m, o, None),
                c => c,
            }
        };
        let mut out = vec![1u8, 5];
        let mut prev = [0i32; 3];
        for (k, data) in chunks {
            let delta = [0, 1, 2].map(|a| k[a].wrapping_sub(prev[a]).to_be_bytes());
            for b in 0..4 {
                for d in &delta {
                    out.push(d[b]);
                }
            }
            prev = k;
            let mut i = 0;
            while i < CV {
                let (m, o, w) = norm(data[i]);
                let occ_flag = if o != 255 { 0x40 } else { 0 };
                if let Some(w) = w {
                    out.push(m | occ_flag | 0x80);
                    if o != 255 {
                        out.push(o);
                    }
                    out.extend_from_slice(&[0, w]);
                    i += 1;
                    continue;
                }
                let mut n = 1;
                while i + n < CV && n < 256 && norm(data[i + n]) == (m, o, w) {
                    n += 1;
                }
                out.push(m | occ_flag | if n > 1 { 0x80 } else { 0 });
                if o != 255 {
                    out.push(o);
                }
                if n > 1 {
                    out.push((n - 1) as u8);
                }
                i += n;
            }
        }
        out
    }

    fn base64(s: &str) -> Vec<u8> {
        let val = |c: u8| match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            _ => 63,
        };
        let mut out = Vec::new();
        let (mut acc, mut bits) = (0u32, 0);
        for c in s.bytes().filter(|&c| c != b'=') {
            acc = (acc << 6) | val(c) as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        out
    }

    /// Published test vector from rbx-dom PR 444 (two chunks at (0,0,0) and
    /// (1,0,0), each with Water at local (0,0,0) and Pavement at (1,0,0)).
    const RBXDOM_VECTOR: &str = "AQUAAAAAAAAAAAAAAAABFoD/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP0AAAAAAAAAAAABAAABFoD/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP+A/4D/gP0=";

    #[test]
    fn terrain_decodes_published_vector() {
        let bytes = base64(RBXDOM_VECTOR);
        let v = Voxels::decode(&bytes).unwrap();
        assert_eq!(v.count(), 4);
        assert_eq!(v.chunk_count(), 2);
        assert_eq!(v.get(0, 0, 0), (WATER, 1.0));
        assert_eq!(v.get(1, 0, 0), (22, 1.0));
        assert_eq!(v.get(32, 0, 0), (WATER, 1.0));
        assert_eq!(v.get(33, 0, 0), (22, 1.0));
        assert_eq!(v.get(2, 0, 0), (0, 0.0));
        assert_eq!(material_name(v.get(33, 0, 0).0), "Pavement");
        assert_eq!(v.bounds(), Some(([0, 0, 0], [33, 0, 0])));
        // Our test encoder must reproduce it byte for byte.
        let cells: HashMap<[i32; 3], Cell> = [
            ([0, 0, 0], (WATER, 255, None)),
            ([1, 0, 0], (22, 255, None)),
            ([32, 0, 0], (WATER, 255, None)),
            ([33, 0, 0], (22, 255, None)),
        ]
        .into_iter()
        .collect();
        assert_eq!(encode(&cells), bytes);
    }

    #[test]
    fn terrain_roundtrip() {
        let mut cells: HashMap<[i32; 3], Cell> = HashMap::new();
        cells.insert([0, 0, 0], (2, 255, None));
        cells.insert([-1, -1, -1], (8, 127, None));
        cells.insert([-33, 5, 40], (10, 0, None));
        cells.insert([5, -70, 3], (3, 100, Some(255)));
        cells.insert([6, -70, 3], (WATER, 200, None));
        for x in 100..400 {
            cells.insert([x, 7, -2], (14, 255, None)); // long run crossing chunks
        }
        let v = Voxels::decode(&encode(&cells)).unwrap();
        assert_eq!(v.count(), cells.len());
        for (p, (m, o, _)) in &cells {
            let (gm, go) = v.get(p[0], p[1], p[2]);
            assert_eq!(gm, *m, "material at {p:?}");
            assert!(
                (go - occupancy_from_byte(*o)).abs() < 1e-6,
                "occupancy at {p:?}"
            );
        }
        assert!((v.get(-1, -1, -1).1 - 0.5).abs() < 1e-6);
        assert!((v.get(-33, 5, 40).1 - 1.0 / 256.0).abs() < 1e-6);
        assert!((v.water(5, -70, 3) - 1.0).abs() < 1e-6);
        assert!((v.water(6, -70, 3) - occupancy_from_byte(200)).abs() < 1e-6);
        assert_eq!(v.water(0, 0, 0), 0.0);
        assert_eq!(v.get(1, 0, 0), (0, 0.0));
        assert_eq!(v.bounds(), Some(([-33, -70, -2], [399, 7, 40])));
        assert_eq!(v.world_bounds().unwrap().1, [1600.0, 32.0, 164.0]);
        assert_eq!(voxel_center([0, 0, -1]), [2.0, 2.0, -2.0]);
    }

    #[test]
    fn terrain_rejects_bad_input() {
        assert_eq!(Voxels::decode(&[]).unwrap().count(), 0);
        assert!(Voxels::decode(&[1]).is_err());
        assert!(Voxels::decode(&[2, 5]).unwrap_err().contains("version"));
        assert!(Voxels::decode(&[1, 9]).is_err());
        let mut b = vec![1, 5];
        b.extend_from_slice(&[0; 12]);
        assert!(Voxels::decode(&b).unwrap_err().contains("truncated"));
        let mut over = b.clone();
        for _ in 0..127 {
            over.extend_from_slice(&[0x80, 255]);
        }
        over.extend_from_slice(&[0, 0x80, 255]); // 32513 + 256 > 32768
        assert!(Voxels::decode(&over).unwrap_err().contains("past the end"));
        let mut unk = b.clone();
        unk.push(63);
        assert!(Voxels::decode(&unk)
            .unwrap_err()
            .contains("unknown material"));
        // Header + one empty chunk is a valid, empty grid.
        let mut air = b.clone();
        for _ in 0..128 {
            air.extend_from_slice(&[0x80, 255]);
        }
        let v = Voxels::decode(&air).unwrap();
        assert_eq!(
            (v.count(), v.bounds().is_none(), build_meshes(&v).len()),
            (0, true, 0)
        );
    }

    #[test]
    fn terrain_decodes_other_chunk_sizes() {
        // Exponent 1 (2^3 chunks): chunk (-1,0,3) => voxels x -2..-1, z 6..7.
        let mut b = vec![1, 1];
        let d = [
            (-1i32).to_be_bytes(),
            0i32.to_be_bytes(),
            3i32.to_be_bytes(),
        ];
        for k in 0..4 {
            for a in &d {
                b.push(a[k]);
            }
        }
        // index 1 = (x=1,z=0,y=0) Rock; index 6 = (x=0,z=1,y=1) Snow half.
        b.extend_from_slice(&[0, 8, 0x80, 3, 0x40 | 10, 127, 0]);
        let v = Voxels::decode(&b).unwrap();
        assert_eq!(v.count(), 2);
        assert_eq!(v.get(-1, 0, 6).0, 8);
        assert_eq!(v.get(-2, 1, 7).0, 10);
    }

    #[test]
    fn terrain_material_names() {
        assert_eq!(material_name(0), "Air");
        assert_eq!(material_name(1), "Water");
        assert_eq!(material_name(2), "Grass");
        assert_eq!(material_name(3), "Slate");
        assert_eq!(material_name(8), "Rock");
        assert_eq!(material_name(22), "Pavement");
        assert_eq!(material_name(23), "Unknown");
        assert_eq!(material_index("LeafyGrass"), Some(19));
        assert!(is_water(1) && !is_water(2));
        for m in 0..23u8 {
            assert_eq!(material_index(material_name(m)), Some(m));
        }
    }

    fn block(min: [i32; 3], max: [i32; 3], c: Cell) -> HashMap<[i32; 3], Cell> {
        let mut cells = HashMap::new();
        for x in min[0]..=max[0] {
            for y in min[1]..=max[1] {
                for z in min[2]..=max[2] {
                    cells.insert([x, y, z], c);
                }
            }
        }
        cells
    }

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    /// Closed, consistently wound, outward-facing surface around `center`.
    fn check_closed(m: &MaterialMesh, center: [f32; 3]) {
        assert_eq!(m.indices.len() % 3, 0);
        let mut directed: HashMap<(u32, u32), u32> = HashMap::new();
        for t in m.indices.chunks(3) {
            for k in 0..3 {
                *directed.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
            let p = [0, 1, 2].map(|k| m.positions[t[k] as usize]);
            let n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            let c = [0, 1, 2].map(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.0);
            assert!(dot(n, sub(c, center)) > 0.0, "triangle faces inward");
        }
        for (&(a, b), &n) in &directed {
            assert_eq!(n, 1, "edge used twice in the same direction");
            assert_eq!(directed.get(&(b, a)), Some(&1), "open edge");
        }
        for (p, n) in m.positions.iter().zip(&m.normals) {
            assert!(dot(*n, sub(*p, center)) > 0.0, "normal points inward");
            assert!((dot(*n, *n) - 1.0).abs() < 1e-4);
        }
        // Euler characteristic of a sphere.
        let (v, e, f) = (
            m.positions.len() as i64,
            directed.len() as i64 / 2,
            m.indices.len() as i64 / 3,
        );
        assert_eq!(v - e + f, 2);
    }

    #[test]
    fn terrain_block_meshes_closed() {
        let v = Voxels::decode(&encode(&block([0, 0, 0], [2, 2, 2], (2, 255, None)))).unwrap();
        assert_eq!(v.count(), 27);
        let meshes = build_meshes(&v);
        assert_eq!(meshes.len(), 1);
        let m = &meshes[0];
        assert_eq!(m.material, 2);
        check_closed(m, [6.0, 6.0, 6.0]);
        // Flat faces half a voxel outside the full voxels (0..12 studs),
        // as Studio's slab tops: -2..14.
        for k in 0..3 {
            let lo = m.positions.iter().map(|p| p[k]).fold(f32::MAX, f32::min);
            let hi = m.positions.iter().map(|p| p[k]).fold(f32::MIN, f32::max);
            assert!(
                (lo + 2.0).abs() < 1e-4 && (hi - 14.0).abs() < 1e-4,
                "axis {k}: {lo}..{hi}"
            );
        }
        let (cp, ci) = collision_mesh(&meshes);
        assert_eq!((cp.len(), ci.len()), (m.positions.len(), m.indices.len()));
    }

    #[test]
    fn terrain_block_across_chunk_corner_is_welded() {
        // 4x4x4 block straddling the corner shared by 8 chunks.
        let v = Voxels::decode(&encode(&block([-2, -2, -2], [1, 1, 1], (8, 255, None)))).unwrap();
        assert_eq!(v.chunk_count(), 8);
        let meshes = build_meshes(&v);
        assert_eq!(meshes.len(), 1);
        check_closed(&meshes[0], [0.0, 0.0, 0.0]);
    }

    #[test]
    fn terrain_two_materials_and_water() {
        let mut cells = block([0, 0, 0], [1, 1, 1], (2, 255, None));
        cells.extend(block([2, 0, 0], [3, 1, 1], (8, 255, None)));
        cells.extend(block([0, 0, 5], [1, 1, 6], (WATER, 255, None)));
        // Water touching solid: no water face against the rock.
        cells.extend(block([4, 0, 0], [4, 1, 1], (WATER, 255, None)));
        let v = Voxels::decode(&encode(&cells)).unwrap();
        let meshes = build_meshes(&v);
        let mats: Vec<u8> = meshes.iter().map(|m| m.material).collect();
        assert_eq!(mats, vec![WATER, 2, 8]);
        let water = &meshes[0];
        assert!(is_water(water.material));
        // No water vertex lies deep in the rock (x < 16): the rim's reach at
        // most half a voxel in, as every surface sits half a voxel out from
        // full voxels (round 21's crossing rule, water as solid).
        assert!(water
            .positions
            .iter()
            .all(|p| p[0] >= 14.0 - 1e-4 || p[2] >= 18.0 - 1e-4));
        let (pos, idx) = collision_mesh(&meshes);
        assert!(!pos.is_empty() && idx.len() % 3 == 0);
    }

    /// Run with: cargo test --release --bin rustudio terrain_perf -- --ignored --nocapture
    #[test]
    #[ignore]
    fn terrain_perf() {
        let mut cells = HashMap::new();
        for x in -256..256 {
            for z in -256..256 {
                let h = 8 + ((x as f32 * 0.05).sin() * 6.0 + (z as f32 * 0.07).cos() * 6.0) as i32;
                for y in -4..h {
                    let top = y == h - 1;
                    cells.insert(
                        [x, y, z],
                        (if top { 2 } else { 8 }, if top { 160 } else { 255 }, None),
                    );
                }
            }
        }
        let bytes = encode(&cells);
        let t = std::time::Instant::now();
        let v = Voxels::decode(&bytes).unwrap();
        let td = t.elapsed();
        let t = std::time::Instant::now();
        let meshes = build_meshes(&v);
        let tm = t.elapsed();
        let tris: usize = meshes.iter().map(|m| m.indices.len() / 3).sum();
        println!(
            "{} voxels, {} bytes: decode {td:?}, mesh {tm:?}, {tris} triangles",
            v.count(),
            bytes.len()
        );
    }
}
