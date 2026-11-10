//! Studio's terrain material table (round 21): per SmoothGrid material its
//! texture pack for the top, side and bottom faces, with tiling, detiling,
//! rotation and the height-blend range, and the colour table (default
//! MaterialColors and each pack's average albedo). Read from
//! assets/terrain-materials.json, a copy of the fast flags
//! FStringTerrainMaterialTablePre2022 / FStringTerrainMaterialTable2022 /
//! FStringTerrainColorTable of an offline Studio (0.744).
//!
//! How Studio's terrain shaders use it (SmoothCluster VS / FS HQ;
//! docs/MECHANICS.md "Round 21"):
//! - every triangle carries three corner materials; each corner's texture
//!   coordinate is the world position projected on one of 18 axes, times the
//!   face's `tiling`, turned by a random angle (× `rotation`) and moved by a
//!   random offset (× `detiling`), the randoms fixed per corner;
//! - the three samples blend by barycentric weight sharpened by each
//!   texture's height, remapped by `blend {min, max, scale}`;
//! - the albedo is recoloured: its luma scaled and its chroma turned and
//!   scaled so the pack's average albedo comes out as the MaterialColor.

use std::sync::OnceLock;

use serde_json::Value;

/// One texture of a material (a face variant).
#[derive(Clone, Debug, Default)]
pub struct Tex {
    pub pack: u64,
    /// Texture repeats per stud along the projection.
    pub tiling: f32,
    /// Random offset scale (Studio's "detiling").
    pub detiling: f32,
    /// Random rotation scale (0 = none, 1 = up to ±90°).
    pub rotation: f32,
    /// Height blend: remap height h to (h − min) / (max − min); sharpness.
    pub blend_min: f32,
    pub blend_max: f32,
    pub blend_scale: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Material {
    #[cfg_attr(not(test), allow(dead_code))]
    pub name: String,
    /// Top, side, bottom.
    pub faces: [Option<Tex>; 3],
    /// Brick, WoodPlanks: cube mapping (six projection axes only).
    pub cube: bool,
    /// How the material bends the surface (mesher).
    pub shape: crate::terrain::Shape,
    pub default_color: [u8; 3],
    pub avg_albedo: [u8; 3],
}

struct Tables {
    pre2022: Vec<Material>,
    v2022: Vec<Material>,
}

fn tex(v: &Value) -> Option<Tex> {
    let pack = v
        .get("texturePackId")?
        .as_str()?
        .trim_start_matches("rbxassetid://")
        .parse()
        .ok()?;
    let f = |k: &str, d: f64| v.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
    let b = v.get("blend");
    let bf = |k: &str, d: f64| b.and_then(|b| b.get(k)).and_then(Value::as_f64).unwrap_or(d) as f32;
    Some(Tex {
        pack,
        tiling: f("tiling", 0.5),
        detiling: f("detiling", 0.0),
        rotation: f("rotation", 0.0),
        blend_min: bf("min", 0.0),
        blend_max: bf("max", 1.0),
        blend_scale: bf("scale", 0.1),
    })
}

fn rgb(v: Option<&Value>) -> [u8; 3] {
    let a = v.and_then(Value::as_array);
    let c = |i: usize| a.and_then(|a| a.get(i)).and_then(Value::as_u64).unwrap_or(0) as u8;
    [c(0), c(1), c(2)]
}

fn parse(list: &Value, colors: &Value, base_key: &str, avg_key: &str) -> Vec<Material> {
    let empty = Vec::new();
    let list = list.as_array().unwrap_or(&empty);
    list.iter()
        .enumerate()
        .map(|(i, m)| {
            let all = m.get("texture").and_then(tex);
            let top = m.get("texture_top").and_then(tex).or_else(|| all.clone());
            let side = m.get("texture_side").and_then(tex).or_else(|| all.clone());
            // A material without its own bottom shows its side below.
            let bottom = m
                .get("texture_bottom")
                .and_then(tex)
                .or_else(|| side.clone())
                .or_else(|| all.clone());
            Material {
                name: m.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                faces: [top, side, bottom],
                cube: m.get("mapping").and_then(Value::as_str) == Some("cube"),
                shape: crate::terrain::Shape {
                    shift: m.get("shift").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    quantize: m.get("quantize").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    cubify: m.get("cubify").and_then(Value::as_f64).unwrap_or(0.0) > 0.0,
                    faceted: matches!(m.get("type").and_then(Value::as_str), Some("hard" | "hardsoft")),
                },
                default_color: rgb(colors.get(base_key).and_then(|c| c.get(i))),
                avg_albedo: rgb(colors.get(avg_key).and_then(|c| c.get(i))),
            }
        })
        .collect()
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let j: Value =
            serde_json::from_str(include_str!("../assets/terrain-materials.json")).unwrap_or(Value::Null);
        let colors = j.get("colors").cloned().unwrap_or(Value::Null);
        Tables {
            pre2022: parse(&j["pre2022"], &colors, "baseColorsPre2022", "avgAlbedoPre2022"),
            v2022: parse(&j["2022"], &colors, "baseColors2022", "avgAlbedo2022"),
        }
    })
}

/// The table entry of SmoothGrid material `m` (MaterialService.Use2022Materials picks the table).
pub fn material(m: u8, use2022: bool) -> Option<&'static Material> {
    let t = tables();
    if use2022 { &t.v2022 } else { &t.pre2022 }.get(m as usize)
}

/// Studio's recolour of a terrain albedo (FS HQ: CB4 = luma scale, cos,
/// sin): luma × `scale`, chroma (Co, Cg) turned and scaled by (cos, sin),
/// chosen so the pack's average albedo comes out as `color` (sRGB 0..1).
pub fn recolor(avg: [f32; 3], color: [f32; 3]) -> [f32; 3] {
    let ycc = |c: [f32; 3]| {
        let y = (c[0] + 2.0 * c[1] + c[2]) * 0.25;
        let co = (c[0] - c[2]) * 0.5;
        let cg = (2.0 * c[1] - c[0] - c[2]) * 0.25;
        (y, co, cg)
    };
    let (ya, coa, cga) = ycc(avg);
    let (yc, coc, cgc) = ycc(color);
    let scale = if ya > 1e-4 { yc / ya } else { 1.0 };
    // (Co + i Cg) of the colour over that of the average, as a complex number.
    let den = coa * coa + cga * cga;
    let (c, s) = if den > 1e-8 {
        ((coc * coa + cgc * cga) / den, (cgc * coa - coc * cga) / den)
    } else {
        (1.0, 0.0)
    };
    [scale, c, s]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grass_has_three_faces_and_its_colours() {
        let g = material(2, false).expect("grass");
        assert_eq!(g.name, "Grass");
        assert!(g.faces.iter().all(|f| f.is_some()));
        assert_eq!(g.faces[0].as_ref().unwrap().pack, 7547336797);
        assert_eq!(g.default_color, [106, 127, 63]);
        assert!(material(5, false).unwrap().cube, "Brick maps as a cube");
    }

    #[test]
    fn recolor_maps_the_average_onto_the_colour() {
        let avg = [91.0 / 255.0, 96.0 / 255.0, 51.0 / 255.0];
        let col = [200.0 / 255.0, 60.0 / 255.0, 60.0 / 255.0];
        let [k, c, s] = recolor(avg, col);
        let y = (avg[0] + 2.0 * avg[1] + avg[2]) * 0.25 * k;
        let (co, cg) = ((avg[0] - avg[2]) * 0.5, (2.0 * avg[1] - avg[0] - avg[2]) * 0.25);
        let (co2, cg2) = (c * co - s * cg, s * co + c * cg);
        let out = [y - cg2 + co2, y + cg2, y - cg2 - co2];
        for i in 0..3 {
            assert!((out[i] - col[i]).abs() < 1e-4, "{out:?}");
        }
    }
}
