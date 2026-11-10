//! Part material textures: Studio's material tables and their texture packs.
//!
//! Studio builds three tables of 43 materials in one function
//! `PartMaterialInfosPre2022`, then either the 4K 2022 table
//! (fast flag RenderEnablePartBaseMaterial4K, default true, registered at
//!) or the plain 2022 table. Each entry is a texture pack id
//! (colour / normal / roughness or metalness / height), a second "detail"
//! pack (a normal map only), a colour 0xAABBGGRR and one 16-bit word. The
//! colour is the material's average: drawn on its own when the maps aren't
//! there (offline Studio shows exactly these, squared), and close to the
//! colour map's mean (pre-2022 Fabric 0x666869 vs 101, 100, 98 measured).
//! Material order: the Enum.Material list.
//!
//! A pack is fetched from public asset delivery (docs/MATERIALS.md: no login,
//! gzip, two hops), its maps decoded, resized to one layer size and turned
//! into mip chains on background threads; results are cached by asset id
//! in ~/.cache/rustudio/assets. Studio's part shaders sample every map at
//! TEXCOORD0 × uvScale + uvOffset (DefaultUnified*VS, per-instance CB1 rows
//! 5 and 6): a box face repeats every `consts(..)[5]` studs (8 with the 2022
//! tables, 4–14.8 per material with the pre-2022 one; measured, mech.rs
//! PART_MATERIAL_*), times the shader's textureTiling. The material's second
//! ("detail") pack is a normal map added at `normalDetailTiling` times the
//! main repeat; 2022 materials of shader type 21 are drawn "detiled" (two
//! copies at noise-picked offsets).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// One material's row of a Studio material table. `detail` is the detail
/// normal pack (drawn with `consts`); `word` is kept as read, not drawn.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Entry {
    pub material: u32,
    /// Texture pack asset id (0: none, the material draws flat).
    pub pack: u64,
    /// The second pack: a detail normal map (0: none).
    pub detail: u64,
    /// Average colour, 0xAABBGGRR.
    pub color: u32,
    /// The entry's last 16-bit field (meaning not identified).
    pub word: u16,
}

const fn e(material: u32, pack: u64, detail: u64, color: u32, word: u16) -> Entry {
    Entry {
        material,
        pack,
        detail,
        color,
        word,
    }
}

/// RenderEnablePartBaseMaterial4K's default.
pub const PART_BASE_MATERIAL_4K: bool = true;
/// Studs per texture repeat of the 2022 tables (and of the studs atlas:
/// TEXCOORD0 = studs / 8); the pre-2022 table's are per material (`consts`).
pub const STUDS_PER_TILE: f32 = 8.0;
/// Every map is resized to this square layer size for the texture arrays.
pub const LAYER_SIZE: u32 = 1024;

pub static PRE_2022: [Entry; 43] = [
    e(256, 0, 0, 0xffffffff, 0x9900),                    // Plastic
    e(272, 0, 0, 0xffffffff, 0x3c00),                    // SmoothPlastic
    e(1328, 9439632387, 9438453972, 0xffdadada, 0x7d00), // Snow
    e(912, 9873521380, 9438453972, 0xf6a9abae, 0xc100),  // Sandstone
    e(288, 0, 0, 0xffffffff, 0x8000),                    // Neon
    e(512, 7547303225, 7547298786, 0xffe3e3e3, 0x8816),  // Wood
    e(528, 7547332968, 7546645118, 0xffcbd1d4, 0xa93d),  // WoodPlanks
    e(784, 7547177270, 7546645118, 0xffc7c7c7, 0x7d00),  // Marble
    e(800, 7547298114, 7547298323, 0xffc1c1c1, 0x8c00),  // Slate
    e(816, 7546653951, 7546654144, 0xffd0d0d0, 0x8300),  // Concrete
    e(832, 7547164710, 7546645118, 0xff717171, 0x9500),  // Granite
    e(848, 7546650097, 7546645118, 0xe8c8c9cc, 0xfc00),  // Brick
    e(864, 7547291361, 7546645118, 0xffd0d0d0, 0x9900),  // Pebble
    e(1040, 7547184629, 7546645118, 0xc85f779f, 0xba86), // CorrodedMetal
    e(1056, 7547162198, 7546645118, 0xffaaaaaa, 0x61a8), // DiamondPlate
    e(1072, 7546645012, 7546645118, 0xffffffff, 0x2bff), // Foil
    e(1088, 7547288171, 7546645118, 0xffc7c7c7, 0x59ca), // Metal
    e(1280, 7547169285, 7546645118, 0xff9fa5a5, 0xcd00), // Grass
    e(1296, 7547295153, 7546645118, 0xffdcdcdc, 0x8800), // Sand
    e(1312, 7547101130, 7546645118, 0xf4666869, 0xef00), // Fabric
    e(1536, 7547171356, 7546645118, 0xffffffff, 0x2d00), // Ice
    e(880, 7546652947, 7546645118, 0xfabbc8d4, 0x8000),  // Cobblestone
    e(1568, 7547304948, 7546645118, 0x07fefefe, 0x1d00), // Glass
    e(1584, 0, 0, 0xffffffff, 0x8000),                   // ForceField
    e(1344, 9873319819, 9438453972, 0xfc828282, 0x8800), // Mud
    e(820, 9873289812, 9438453972, 0xfae6eaeb, 0xaa00),  // Limestone
    e(788, 9873270487, 9438413638, 0xee999a9a, 0xcf00),  // Basalt
    e(1360, 9439044431, 9438453972, 0xf0707272, 0xe700), // Ground
    e(1284, 9873288083, 9438453972, 0xea717579, 0xe900), // LeafyGrass
    e(896, 9873515198, 9438453972, 0xf89a9a9a, 0xb100),  // Rock
    e(836, 9873322398, 9438453972, 0xec908e8e, 0xc300),  // Pavement
    e(1552, 9438851661, 9438453972, 0xf3e5e5e2, 0x6400), // Glacier
    e(804, 9438582231, 9438453972, 0x9c504e4a, 0xbc00),  // CrackedLava
    e(1392, 9439566986, 9438453972, 0xffdddcdc, 0xa200), // Salt
    e(1376, 9873267379, 9438410548, 0xea7b7b7b, 0xdf00), // Asphalt
    e(2304, 14108654559, 0, 0xffa9a9a9, 0xab00),         // Cardboard
    e(2305, 14108663921, 0, 0xe5cecece, 0xcd00),         // Carpet
    e(2306, 17429458668, 0, 0xfaefefef, 0x4300),         // CeramicTiles
    e(2307, 18147685875, 0, 0xff9a9a9a, 0x8600),         // ClayRoofTiles
    e(2308, 91579661064923, 0, 0xffacb6b5, 0xc200),      // RoofShingles
    e(2309, 14108671010, 0, 0xffe1e1e1, 0x7600),         // Leather
    e(2310, 14108672569, 0, 0xffdcdcdc, 0xf200),         // Plaster
    e(2311, 14108675388, 0, 0xffd1d1d1, 0x932f),         // Rubber
];
pub static Y2022_4K: [Entry; 43] = [
    e(256, 0, 83066014297293, 0xffffffff, 0x9900), // Plastic
    e(272, 0, 0, 0xffffffff, 0x3c00),              // SmoothPlastic
    e(1328, 106058971152252, 9438453972, 0xfff0efef, 0x7900), // Snow
    e(912, 109869731739139, 9438453972, 0xf6e8e9f0, 0xbe00), // Sandstone
    e(288, 0, 0, 0xffffffff, 0x8000),              // Neon
    e(512, 73489845523797, 104555143962180, 0xffd3d3d9, 0xa200), // Wood
    e(528, 78212019144684, 9438453972, 0xfecfd1d1, 0x8f00), // WoodPlanks
    e(784, 104749709354369, 9438453972, 0xf8c1b7b4, 0x6e00), // Marble
    e(800, 130174615834794, 75747241361350, 0xfdebebeb, 0x9300), // Slate
    e(816, 115089551732435, 9438453972, 0xffe0e0e0, 0xb500), // Concrete
    e(832, 125518382244973, 9438453972, 0xffc8ccd0, 0x2800), // Granite
    e(848, 85077012310177, 9438453972, 0xe3cdd6e6, 0x9800), // Brick
    e(864, 87820215580582, 9438453972, 0xebcbcdcd, 0xd700), // Pebble
    e(1040, 122577965407434, 104387492555360, 0xb3a4b2cf, 0x9b29), // CorrodedMetal
    e(1056, 124320368131155, 9438453972, 0xffd2d2d2, 0x56ff), // DiamondPlate
    e(1072, 110583221457338, 136319143816079, 0xffeeeeee, 0x3cff), // Foil
    e(1088, 71003343637114, 9438453972, 0xffe3e3e3, 0x2dff), // Metal
    e(1280, 106637943980559, 9438453972, 0xf3c4c6c9, 0xe800), // Grass
    e(1296, 117101875286338, 9438453972, 0xefc7ced1, 0xbe00), // Sand
    e(1312, 132264823269423, 9438453972, 0xffe0e0e0, 0xdf00), // Fabric
    e(1536, 91952184132897, 9438453972, 0xf8f1efeb, 0x6600), // Ice
    e(880, 129616831094994, 9438453972, 0xf4dbdbdb, 0xe600), // Cobblestone
    e(1568, 88609722573469, 9438453972, 0x07ffffff, 0x1d00), // Glass
    e(1584, 0, 0, 0xffffffff, 0x8000),             // ForceField
    e(1344, 81832637858593, 9438453972, 0xfbbcbcbd, 0x8800), // Mud
    e(820, 136792039489433, 9438453972, 0xfbecf4f8, 0xae00), // Limestone
    e(788, 103124999245988, 103955051687847, 0xefa0a2a2, 0xd200), // Basalt
    e(1360, 122356188009715, 9438453972, 0xf39ea3a3, 0xe800), // Ground
    e(1284, 75665810846977, 9438453972, 0xebb4b4b8, 0xeb00), // LeafyGrass
    e(896, 112102163340384, 9438453972, 0xf9d2d3d4, 0xc200), // Rock
    e(836, 100821413010093, 9438453972, 0xebd8dad9, 0xc200), // Pavement
    e(1552, 85752927082668, 9438453972, 0xf3e5e5e2, 0x6400), // Glacier
    e(804, 121276091081821, 9438453972, 0xf77d7d7c, 0xa400), // CrackedLava
    e(1392, 137066264708856, 9438453972, 0xfff7f7f7, 0xa300), // Salt
    e(1376, 77679553820203, 105970962053043, 0xebe6e5e4, 0xe000), // Asphalt
    e(2304, 112994935967349, 0, 0xffa6a6a6, 0xab00), // Cardboard
    e(2305, 119478636506120, 0, 0xe5cdcdcd, 0xcd00), // Carpet
    e(2306, 126380627739075, 0, 0xfbefefef, 0x4a00), // CeramicTiles
    e(2307, 114168170326154, 0, 0xffafaeae, 0xa500), // ClayRoofTiles
    e(2308, 107847976513485, 0, 0xffc4cbca, 0xd300), // RoofShingles
    e(2309, 111918862958038, 0, 0xffe2e2e2, 0x7500), // Leather
    e(2310, 104555660139073, 0, 0xffdbdbdb, 0xed00), // Plaster
    e(2311, 112374357118714, 0, 0xffdbdbdb, 0xed2f), // Rubber
];
pub static Y2022: [Entry; 43] = [
    e(256, 0, 9475422736, 0xffffffff, 0x9900), // Plastic
    e(272, 0, 0, 0xffffffff, 0x3c00),          // SmoothPlastic
    e(1328, 9920620451, 9438453972, 0xfff0f0ef, 0x7d00), // Snow
    e(912, 9920596353, 9438453972, 0xf6e6eaf1, 0xc100), // Sandstone
    e(288, 0, 0, 0xffffffff, 0x8000),          // Neon
    e(512, 9920625499, 9439649548, 0xffd0d1d9, 0xa000), // Wood
    e(528, 9920626896, 9438453972, 0xfeced0cf, 0x8d00), // WoodPlanks
    e(784, 9873292869, 9438453972, 0xf9c1b7b5, 0x6e00), // Marble
    e(800, 9920600052, 9439613006, 0xfeebeaeb, 0x9400), // Slate
    e(816, 9920484334, 9438453972, 0xffe0e1e1, 0xb500), // Concrete
    e(832, 9920550720, 9438453972, 0xffc8ced2, 0x2800), // Granite
    e(848, 9920482992, 9438453972, 0xe3cdd6e5, 0x9700), // Brick
    e(864, 9920581197, 9438453972, 0xeac9cbcc, 0xd500), // Pebble
    e(1040, 9920589512, 9439557520, 0xb4a3b1ce, 0x9928), // CorrodedMetal
    e(1056, 10237721036, 9438453972, 0xffd2d2d2, 0x56ff), // DiamondPlate
    e(1072, 9873266399, 9438410239, 0xffeeeeee, 0x3cff), // Foil
    e(1088, 9920574966, 9438453972, 0xffe2e2e2, 0x2eff), // Metal
    e(1280, 9920552044, 9438453972, 0xf1bdc4c4, 0xe700), // Grass
    e(1296, 9920591862, 9438453972, 0xf0d2d8da, 0xbe00), // Sand
    e(1312, 9920517963, 9438453972, 0xffdddddd, 0xe500), // Fabric
    e(1536, 9920556429, 9438453972, 0xf8f1efeb, 0x6400), // Ice
    e(880, 9919719550, 9438453972, 0xf3dbdbda, 0xde00), // Cobblestone
    e(1568, 9873284556, 9438453972, 0x07fefefe, 0x1d00), // Glass
    e(1584, 0, 0, 0xffffffff, 0x8000),         // ForceField
    e(1344, 9920578676, 9438453972, 0xfcc1c0c1, 0x8800), // Mud
    e(820, 9920561624, 9438453972, 0xfaf3f8fa, 0xaa00), // Limestone
    e(788, 9920482224, 9438413638, 0xee9ea0a0, 0xcf00), // Basalt
    e(1360, 9920554695, 9438453972, 0xf0a0a5a5, 0xe700), // Ground
    e(1284, 9920558145, 9438453972, 0xeaafb2b6, 0xe900), // LeafyGrass
    e(896, 10129366149, 9438453972, 0xf8d2d3d3, 0xb100), // Rock
    e(836, 9920580094, 9438453972, 0xecdbdada, 0xc300), // Pavement
    e(1552, 9920518995, 9438453972, 0xf3e5e5e1, 0x6400), // Glacier
    e(804, 9920485426, 9438453972, 0x9c514f4c, 0xbc00), // CrackedLava
    e(1392, 9920590478, 9438453972, 0xfff9f9f9, 0xa200), // Salt
    e(1376, 9930003180, 9438410548, 0xeae4e3e3, 0xdf00), // Asphalt
    e(2304, 14108654559, 0, 0xffa9a9a9, 0xab00), // Cardboard
    e(2305, 14108663921, 0, 0xe5cecece, 0xcd00), // Carpet
    e(2306, 17429458668, 0, 0xfaefefef, 0x4300), // CeramicTiles
    e(2307, 18147685875, 0, 0xff9a9a9a, 0x8600), // ClayRoofTiles
    e(2308, 91579661064923, 0, 0xffacb6b5, 0xc200), // RoofShingles
    e(2309, 14108671010, 0, 0xffe1e1e1, 0x7600), // Leather
    e(2310, 14108672569, 0, 0xffdcdcdc, 0xf200), // Plaster
    e(2311, 14108675388, 0, 0xffd1d1d1, 0x932f), // Rubber
];
/// The table Studio draws parts with: MaterialService.Use2022Materials picks
/// a 2022 table (4K by default), false the pre-2022 one.
pub fn table(use2022: bool) -> &'static [Entry; 43] {
    match (use2022, PART_BASE_MATERIAL_4K) {
        (false, _) => &PRE_2022,
        (true, true) => &Y2022_4K,
        (true, false) => &Y2022,
    }
}

pub fn entry(material: u32, use2022: bool) -> Option<&'static Entry> {
    table(use2022).iter().find(|e| e.material == material)
}

/// Studio's per-material shader constants for a part (mech.rs
/// PART_MATERIAL_*): textureTiling, normalDetailTiling, normalDetailScale,
/// noiseScale, detiled (1/0), studs per tile. Unknown materials: plain
/// tiling every STUDS_PER_TILE studs.
pub fn consts(material: u32, use2022: bool) -> [f32; 6] {
    let rows = if use2022 {
        &crate::mech::PART_MATERIAL_2022
    } else {
        &crate::mech::PART_MATERIAL_PRE2022
    };
    table(use2022)
        .iter()
        .position(|e| e.material == material)
        .map(|i| rows[i])
        .unwrap_or([1.0, 0.0, 0.0, 0.0, 0.0, STUDS_PER_TILE])
}

/// Full type/constant row for a loaded pack. Packs identify the table even
/// when old and new materials coexist; detail layers keep neutral defaults.
pub fn full_consts(pack: u64) -> Option<&'static [f32; 21]> {
    for (entries, rows) in [
        (&PRE_2022, &crate::mech::PART_MATERIAL_FULL_PRE2022),
        (&Y2022, &crate::mech::PART_MATERIAL_FULL_2022),
        (&Y2022_4K, &crate::mech::PART_MATERIAL_FULL_2022),
    ] {
        if let Some(i) = entries.iter().position(|e| e.pack != 0 && e.pack == pack) {
            return Some(&rows[i]);
        }
    }
    None
}

/// Wang index is DDS A8L8, uploaded by Studio as two channels (L, A).
/// Four 4-bit cells per f32, all integers exactly representable; the shader
/// uses nearest/wrap, matching slot 9's filter=0.
pub fn wang_index() -> Vec<[f32; 4]> {
    let mut out = vec![[0.0; 4]; 1024];
    let img = crate::assets::resolve(crate::mech::WANG_INDEX, None)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| crate::texture::decode(&b).ok());
    if let Some(img) = img.filter(|i| i.width == 128 && i.height == 128) {
        for (i, px) in img.pixels.chunks_exact(4).enumerate() {
            let cell = (px[0] / 64) as u32 | (((px[3] / 64) as u32) << 2);
            out[i / 16][(i / 4) % 4] += (cell << ((i % 4) * 4)) as f32;
        }
    }
    out
}

/// hashes an instance allocation's eight address bytes with
/// FNV-1a; turns the hash into the part's offset. The renderer
/// supplies a stable random referent identity in place of Studio's address.
pub fn uv_offset(size: glam::Vec3, material: u32, identity: u64) -> [f32; 2] {
    let hash = identity.to_le_bytes().iter().fold(crate::mech::MATERIAL_UV_FNV_OFFSET,
        |h, b| (h ^ *b as u64).wrapping_mul(crate::mech::MATERIAL_UV_FNV_PRIME));
    let u = ((hash >> 3) & 63) as usize;
    let v = ((hash >> 9) & 63) as usize;
    if matches!(material, 256 | 272) {
        return [(u & !1) as f32, (v & !1) as f32];
    }
    if matches!(material, 528 | 848 | 2304..=2311) {
        return [u as f32, v as f32];
    }
    let k = (size.x + size.y + size.z) * 0.5;
    [u, v].map(|i| (crate::mech::MATERIAL_UV_RANDOM[i] * 2.0 - 1.0) * k)
}

/// Studio's detiling noise (mech::DETILE_NOISE, 32 × 32): its red channel,
/// 0..1, rows top to bottom; flat 0 without the file.
pub fn detile_noise() -> &'static [f32; 1024] {
    static N: OnceLock<[f32; 1024]> = OnceLock::new();
    N.get_or_init(|| {
        let mut out = [0.0f32; 1024];
        let img = crate::assets::resolve(crate::mech::DETILE_NOISE, None)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| crate::texture::decode(&b).ok());
        if let Some(img) = img.filter(|i| i.width == 32 && i.height == 32) {
            for (i, v) in out.iter_mut().enumerate() {
                *v = img.pixels[i * 4] as f32 / 255.0;
            }
        }
        out
    })
}

/// The table colour as RGB bytes (it is stored 0xAABBGGRR).
pub fn rgb(color: u32) -> [u8; 3] {
    [color as u8, (color >> 8) as u8, (color >> 16) as u8]
}

/// A material's colour without its maps, linear (squared, as Studio's
/// shaders square their inputs): what the part colour is multiplied by.
pub fn fallback_albedo(material: u32, use2022: bool) -> [f32; 3] {
    match entry(material, use2022) {
        Some(e) => rgb(e.color).map(|c| (c as f32 / 255.0).powi(2)),
        None => [1.0; 3],
    }
}

/// The child image ids a texture pack XML names.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Pack {
    pub tiling: f32,
    pub color: Option<u64>,
    pub normal: Option<u64>,
    pub roughness: Option<u64>,
    pub metalness: Option<u64>,
    pub height: Option<u64>,
    pub emissive: Option<u64>,
}

pub fn parse_pack(xml: &str) -> Result<Pack, String> {
    if !xml.contains("<roblox") || !xml.contains("texturepack_version") {
        return Err("not a texture pack".into());
    }
    let field = |tag: &str| -> Option<&str> {
        let open = xml.find(&format!("<{tag}"))?;
        let start = open + xml[open..].find('>')? + 1;
        let end = start + xml[start..].find(&format!("</{tag}>"))?;
        Some(xml[start..end].trim())
    };
    let id = |tag: &str| field(tag).and_then(|v| v.parse::<u64>().ok());
    Ok(Pack {
        tiling: field("tiling").and_then(|v| v.parse().ok()).unwrap_or(1.0),
        color: id("color"),
        normal: id("normal"),
        roughness: id("roughness"),
        metalness: id("metalness"),
        height: id("height"),
        emissive: id("emissive"),
    })
}

/// A decoded pack: two RGBA8 mip chains and a scalar emission mip chain.
/// `albedo`: colour RGB and its alpha tint mask (not height). `surface`:
/// normal X/Y (OpenGL convention), roughness, metalness.
pub struct Maps {
    pub pack: u64,
    pub tiling: f32,
    pub albedo: Vec<u8>,
    pub surface: Vec<u8>,
    /// Scalar emissive mask, linear R8 mip chain.
    pub emissive: Vec<u8>,
    /// Which maps the pack had: colour, normal, roughness, metalness, height.
    pub found: [bool; 5],
}

#[derive(Clone)]
enum State {
    Pending,
    Ready(Arc<Maps>),
    Failed(String),
}

struct Store {
    states: Mutex<HashMap<u64, State>>,
    queue: Mutex<VecDeque<(u64, u32)>>,
    wake: Condvar,
    /// Bumped whenever a pack finishes (or fails), so draws are rebuilt.
    generation: AtomicU64,
}

fn store() -> &'static Store {
    static S: OnceLock<Store> = OnceLock::new();
    S.get_or_init(|| {
        for _ in 0..2 {
            std::thread::Builder::new()
                .name("material packs".into())
                .spawn(|| loop {
                    let (pack, color) = {
                        let s = store();
                        let mut q = s.queue.lock().expect("material queue");
                        loop {
                            if let Some(j) = q.pop_front() {
                                break j;
                            }
                            q = s.wake.wait(q).expect("material queue");
                        }
                    };
                    let st = match build(pack, color) {
                        Ok(m) => State::Ready(Arc::new(m)),
                        Err(e) => State::Failed(e),
                    };
                    let s = store();
                    s.states.lock().expect("material states").insert(pack, st);
                    s.generation.fetch_add(1, Ordering::Relaxed);
                })
                .ok();
        }
        Store {
            states: Mutex::new(HashMap::new()),
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
        }
    })
}

pub enum Lookup {
    Ready(Arc<Maps>),
    Pending,
    Unavailable(String),
}

/// A table entry's maps: ready, being fetched (queued on first ask), or
/// unavailable (no pack, or the fetch failed).
pub fn maps(e: &Entry) -> Lookup {
    pack_maps(e.pack, e.color)
}

/// A pack's maps by id (terrain packs, round 21): as `maps`; `color` stands
/// in for a pack without a colour map.
pub fn pack_maps(pack: u64, color: u32) -> Lookup {
    if PACKS_OFF.load(Ordering::Relaxed) {
        return Lookup::Unavailable("material packs are switched off".into());
    }
    if pack == 0 {
        return Lookup::Unavailable("the material has no texture pack".into());
    }
    let e = e(0, pack, 0, color, 0);
    let s = store();
    let mut states = s.states.lock().expect("material states");
    match states.get(&e.pack) {
        Some(State::Ready(m)) => Lookup::Ready(m.clone()),
        Some(State::Pending) => Lookup::Pending,
        Some(State::Failed(why)) => Lookup::Unavailable(why.clone()),
        None => {
            states.insert(e.pack, State::Pending);
            drop(states);
            s.queue
                .lock()
                .expect("material queue")
                .push_back((e.pack, e.color));
            s.wake.notify_one();
            Lookup::Pending
        }
    }
}

static PACKS_OFF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// RUSTUDIO_MATPACKS=0 turns material packs (parts and terrain) off for the
/// run: test runs compared with the offline Studio, which can't load them.
pub fn init_from_env() {
    if std::env::var("RUSTUDIO_MATPACKS").is_ok_and(|v| v == "0") {
        set_enabled(false);
    }
}

/// Draws materials with their flat table colours (packs off) or their
/// texture packs. The self-test's lighting steps compare against Studio
/// captured offline (offline Studio can't download packs), so they switch them off.
pub fn set_enabled(on: bool) {
    if PACKS_OFF.swap(!on, Ordering::Relaxed) == on {
        store().generation.fetch_add(1, Ordering::Relaxed);
    }
}

/// Material packs are on (RUSTUDIO_MATPACKS isn't 0).
pub fn enabled() -> bool {
    !PACKS_OFF.load(Ordering::Relaxed)
}

/// Changes whenever a pack finished loading or failed.
pub fn generation() -> u64 {
    store().generation.load(Ordering::Relaxed)
}

/// Blocks until a pack is ready or failed (tests).
#[cfg(test)]
pub fn wait(e: &Entry, timeout: std::time::Duration) -> Lookup {
    let t0 = std::time::Instant::now();
    loop {
        match maps(e) {
            Lookup::Pending if t0.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            other => return other,
        }
    }
}

fn image(id: u64) -> Result<crate::texture::Rgba, String> {
    let bytes = crate::assets::public_cached(id)?;
    let img = crate::texture::decode(&bytes).map_err(|e| format!("image {id}: {e}"))?;
    Ok(resized(img, LAYER_SIZE))
}

fn resized(img: crate::texture::Rgba, size: u32) -> crate::texture::Rgba {
    if img.width == size && img.height == size {
        return img;
    }
    let Some(buf) = image::RgbaImage::from_raw(img.width, img.height, img.pixels) else {
        return crate::texture::Rgba {
            width: size,
            height: size,
            pixels: vec![255; (size * size * 4) as usize],
        };
    };
    let out = image::imageops::resize(&buf, size, size, image::imageops::FilterType::Triangle);
    crate::texture::Rgba {
        width: size,
        height: size,
        pixels: out.into_raw(),
    }
}

fn build(pack_id: u64, color: u32) -> Result<Maps, String> {
    let xml = crate::assets::public_cached(pack_id)?;
    let pack =
        parse_pack(&String::from_utf8_lossy(&xml)).map_err(|e| format!("pack {pack_id}: {e}"))?;
    let n = (LAYER_SIZE * LAYER_SIZE) as usize;
    let get = |id: Option<u64>| -> Result<Option<crate::texture::Rgba>, String> {
        id.map(image).transpose()
    };
    let colour = get(pack.color)?;
    let normal = get(pack.normal)?;
    let rough = get(pack.roughness)?;
    let metal = get(pack.metalness)?;
    let height = get(pack.height)?;
    let emission = get(pack.emissive)?;
    let found = [
        colour.is_some(),
        normal.is_some(),
        rough.is_some(),
        metal.is_some(),
        height.is_some(),
    ];
    let [r, g, b] = rgb(color);
    let mut albedo = vec![0u8; n * 4];
    let mut surface = vec![0u8; n * 4];
    let mut emissive = vec![0u8; n * 4];
    for i in 0..n {
        let px = |m: &Option<crate::texture::Rgba>, c: usize, none: u8| {
            m.as_ref().map_or(none, |m| m.pixels[i * 4 + c])
        };
        albedo[i * 4] = px(&colour, 0, r);
        albedo[i * 4 + 1] = px(&colour, 1, g);
        albedo[i * 4 + 2] = px(&colour, 2, b);
        albedo[i * 4 + 3] = px(&colour, 3, 255);
        emissive[i * 4] = px(&emission, 0, 0);
        surface[i * 4] = px(&normal, 0, 128);
        surface[i * 4 + 1] = px(&normal, 1, 128);
        surface[i * 4 + 2] = px(&rough, 0, 255);
        surface[i * 4 + 3] = px(&metal, 0, 0);
    }
    Ok(Maps {
        pack: pack_id,
        // Texture-pack <tiling> is a 0..2 layout enum, not a UV scale
        // (as Studio reads it). In particular legacy Wang packs use 2.
        // Repeat density comes from uvScale and PartMaterialConsts instead.
        tiling: 1.0,
        albedo: mip_chain(&albedo, true),
        surface: mip_chain(&surface, false),
        emissive: mip_chain(&emissive, false).into_iter().step_by(4).collect(),
        found,
    })
}

/// LAYER_SIZE² RGBA8 plus its mips down to 1×1. `squared`: RGB averaged as
/// squares (Studio squares colour texels), else plain averages.
fn mip_chain(level0: &[u8], squared: bool) -> Vec<u8> {
    let mut out = level0.to_vec();
    let mut prev = level0.to_vec();
    let mut size = LAYER_SIZE;
    while size > 1 {
        let half = size / 2;
        let mut next = vec![0u8; (half * half * 4) as usize];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let at = |dx: u32, dy: u32| {
                        prev[(((y * 2 + dy) * size + x * 2 + dx) * 4) as usize + c] as f32 / 255.0
                    };
                    let v = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                    let avg = if squared && c < 3 {
                        (v.iter().map(|v| v * v).sum::<f32>() / 4.0).sqrt()
                    } else {
                        v.iter().sum::<f32>() / 4.0
                    };
                    next[((y * half + x) * 4) as usize + c] = (avg * 255.0 + 0.5) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        size = half;
    }
    out
}

/// Byte offset and size of each mip level in a chain from [`mip_chain`].
pub fn mip_levels() -> Vec<(usize, u32)> {
    let mut v = Vec::new();
    let (mut at, mut size) = (0usize, LAYER_SIZE);
    loop {
        v.push((at, size));
        at += (size * size * 4) as usize;
        if size == 1 {
            break;
        }
        size /= 2;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_follow_studio() {
        // Brick (848) in each table, as Studio's tables have it.
        assert_eq!(entry(848, false).unwrap().pack, 7546650097);
        assert_eq!(table(true)[11].material, 848);
        assert_eq!(Y2022[11].pack, 9920482992);
        // Pre-2022 Fabric's average colour is its colour map's mean.
        assert_eq!(rgb(entry(1312, false).unwrap().color), [0x69, 0x68, 0x66]);
        // The fallback darkening matches what offline Studio draws.
        let c = fallback_albedo(1040, false);
        assert!((c[0] - 0.39).abs() < 0.01 && (c[2] - 0.14).abs() < 0.01);
        // Pre-2022 table colours are what offline Studio drew in the
        // lighting calibration (mech::material_albedo, measured in an offline Studio).
        for e in PRE_2022.iter() {
            let (a, b) = (
                fallback_albedo(e.material, false),
                crate::mech::material_albedo(e.material),
            );
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() < 0.015, "{} {a:?} {b:?}", e.material);
            }
        }
        for t in [&PRE_2022, &Y2022, &Y2022_4K] {
            let mut seen = std::collections::HashSet::new();
            assert!(t.iter().all(|e| seen.insert(e.material)));
        }
    }

    #[test]
    fn parses_texture_packs() {
        let p = parse_pack(
            "<roblox>\n  <texturepack_version>2</texturepack_version>\n  <usage version=\"1\">3</usage>\n  <alphamode>2</alphamode>\n  <tiling>1</tiling>\n  <color>9439631732</color>\n  <normal>9439632006</normal>\n  <roughness>9439632145</roughness>\n  <height>9439632299</height>\n</roblox>",
        )
        .unwrap();
        assert_eq!(p.color, Some(9439631732));
        assert_eq!(p.height, Some(9439632299));
        assert_eq!(p.metalness, None);
        assert_eq!(p.tiling, 1.0);
        assert!(parse_pack("<roblox!binary").is_err());
    }

    #[test]
    fn mip_chain_reaches_one_texel() {
        let lv = mip_levels();
        assert_eq!(lv.last().unwrap().1, 1);
        let n = (LAYER_SIZE * LAYER_SIZE * 4) as usize;
        let chain = mip_chain(&vec![200u8; n], true);
        let (at, _) = *lv.last().unwrap();
        assert_eq!(chain.len(), at + 4);
        assert_eq!(chain[at], 200);
    }
}

#[cfg(test)]
mod net_tests {
    use super::*;

    /// Downloads one real pack (network; `cargo test -- --ignored`).
    #[test]
    #[ignore]
    fn loads_brick_2022_4k() {
        let e = entry(848, true).unwrap();
        let t0 = std::time::Instant::now();
        match wait(e, std::time::Duration::from_secs(300)) {
            Lookup::Ready(m) => {
                eprintln!(
                    "brick {} in {:?}: found {:?}",
                    m.pack,
                    t0.elapsed(),
                    m.found
                );
                assert!(m.found[0] && m.found[1]);
            }
            Lookup::Pending => panic!("still pending"),
            Lookup::Unavailable(w) => panic!("{w}"),
        }
    }
}

#[cfg(test)]
mod material_shading_contract {
    #[test]
    fn studio_neon_emission_transfer() {
        // DefaultUnifiedNeonFS.3, before exposure and composition.
        let shader = include_str!("shader.wgsl");
        assert!(shader.contains("const NEON_GLOW: f32 = 13.286025;"));
        assert!(shader.contains("c = base * base * NEON_GLOW;"));
        for (color, expected) in [(0.0_f32, 0.0_f32), (0.5, 0.83037656), (1.0, 13.286025)] {
            let studio = 4.0 * (1.35 * color).powi(4);
            assert!((studio - expected).abs() < 0.00001);
        }
    }
}
