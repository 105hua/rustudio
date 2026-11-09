//! Studio's slider editor for ranged number properties (checks/ui/studio/
//! props-slider.png, an offline Studio). A property gets one when
//! Studio's ReflectionMetadata.xml gives it UIMinimum and UIMaximum (303
//! members: Transparency, Reflectance, Lighting.Brightness / ClockTime /
//! ShadowSoftness, Sound.Volume, ParticleEmitter.Rate…). The step is
//! (UIMaximum − UIMinimum) / UINumTicks, and 20 steps when UINumTicks is
//! missing (measured: Transparency moves in 0.05, ParticleEmitter.Drag
//! 0–5 in 0.25, Lighting.ShadowSoftness with 100 ticks in 0.01).
//! SliderScaling "Square" spaces the values quadratically along the track.
//!
//! The editor, measured on a selected Transparency row: a 65-px spin box
//! (the value, then up / down arrows in its last 12 px) on (40, 41, 49),
//! then the slider: a 9 × 14 handle (53, 55, 65) with a (32, 34, 39) edge
//! over a 2-px (53, 55, 65) groove, the handle's centre running from 4 px
//! after the slider's start (minimum) to 4 px before its end (maximum).

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{Color32, Pos2, Rect, Stroke, Vec2};

pub const SPIN_W: f32 = 65.0;
pub const SPIN_BG: Color32 = Color32::from_rgb(40, 41, 49);
pub const HANDLE: Color32 = Color32::from_rgb(53, 55, 65);
pub const HANDLE_EDGE: Color32 = Color32::from_rgb(32, 34, 39);
pub const HANDLE_W: f32 = 9.0;
pub const HANDLE_H: f32 = 14.0;
/// Steps across the range when ReflectionMetadata gives no UINumTicks.
pub const DEFAULT_TICKS: f64 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderSpec {
    pub min: f64,
    pub max: f64,
    pub ticks: f64,
    pub square: bool,
}

impl SliderSpec {
    pub fn step(&self) -> f64 {
        (self.max - self.min) / self.ticks
    }

    /// The value at track fraction `t` (0..1), on a tick.
    pub fn value_at(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let pos = (t * self.ticks).round() / self.ticks;
        let f = if self.square { pos * pos } else { pos };
        let v = self.min + (self.max - self.min) * f;
        // Keep decimal steps clean (0.05 · 7 = 0.35, not 0.35000000000000003).
        (v * 1e6).round() / 1e6
    }

    /// The track fraction of a value.
    pub fn fraction(&self, v: f64) -> f64 {
        let f = ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0);
        if self.square { f.sqrt() } else { f }
    }

    /// One arrow click / wheel notch from `v` (the spin box's step).
    pub fn nudge(&self, v: f64, dir: f64) -> f64 {
        let n = ((v - self.min) / self.step()).round() + dir;
        ((self.min + n * self.step()).clamp(self.min, self.max) * 1e6).round() / 1e6
    }
}

/// UIMinimum / UIMaximum / UINumTicks / SliderScaling by (class, member),
/// read from Studio's ReflectionMetadata.xml at run time.
pub fn ranges() -> &'static HashMap<(String, String), SliderSpec> {
    static M: OnceLock<HashMap<(String, String), SliderSpec>> = OnceLock::new();
    M.get_or_init(|| {
        let mut out = HashMap::new();
        let Some(dir) = crate::default_scripts::studio_dir() else {
            return out;
        };
        let Ok(text) = std::fs::read_to_string(dir.join("ReflectionMetadata.xml")) else {
            return out;
        };
        parse(&text, &mut out);
        out
    })
}

fn parse(text: &str, out: &mut HashMap<(String, String), SliderSpec>) {
    let field = |chunk: &str, name: &str| -> Option<String> {
        ["string", "double", "float", "int"].iter().find_map(|tag| {
            let key = format!("<{tag} name=\"{name}\">");
            let i = chunk.find(&key)? + key.len();
            let j = chunk[i..].find('<')? + i;
            Some(chunk[i..j].trim().to_string())
        })
    };
    for chunk in text.split("<Item class=\"ReflectionMetadataClass\"").skip(1) {
        let head = chunk.split("</Properties>").next().unwrap_or("");
        let Some(class) = field(head, "Name") else {
            continue;
        };
        for mem in chunk.split("<Item class=\"ReflectionMetadataMember\"").skip(1) {
            let mh = mem.split("</Properties>").next().unwrap_or("");
            let Some(name) = field(mh, "Name") else {
                continue;
            };
            let num = |n: &str| field(mh, n).and_then(|v| v.parse::<f64>().ok());
            let (Some(min), Some(max)) = (num("UIMinimum"), num("UIMaximum")) else {
                continue;
            };
            if max <= min {
                continue;
            }
            let ticks = num("UINumTicks").filter(|t| *t >= 1.0).unwrap_or(DEFAULT_TICKS);
            let square = field(mh, "SliderScaling").is_some_and(|s| s.eq_ignore_ascii_case("Square"));
            out.insert((class.clone(), name), SliderSpec { min, max, ticks, square });
        }
    }
}

/// The slider for `class.prop`, looking through its superclasses (Studio
/// lists BasePart.Transparency under BasePart).
pub fn spec_for(class: &str, prop: &str) -> Option<SliderSpec> {
    let map = ranges();
    let db = crate::doc::db();
    let desc = db.classes.get(class)?;
    db.superclasses_iter(desc).find_map(|c| map.get(&(c.name.to_string(), prop.to_string())).copied())
}

/// The slider's track: the handle's centre runs from `min_x` to `max_x`.
pub fn track(slider: Rect) -> (f32, f32) {
    (slider.min.x + HANDLE_W * 0.5, slider.max.x - HANDLE_W * 0.5)
}

/// Draws the slider (groove and handle at fraction `t`).
pub fn paint_slider(painter: &egui::Painter, slider: Rect, t: f64) {
    let (a, b) = track(slider);
    let cx = a + (b - a) * t as f32;
    let cy = slider.center().y;
    let groove = Rect::from_min_max(Pos2::new(a, cy), Pos2::new(slider.max.x, cy + 2.0));
    painter.rect_filled(groove, 0.0, HANDLE);
    let h = Rect::from_center_size(Pos2::new(cx, cy + 0.5), Vec2::new(HANDLE_W, HANDLE_H));
    painter.rect_filled(h, 0.0, HANDLE);
    painter.rect_stroke(h, 0.0, Stroke::new(1.0, HANDLE_EDGE), egui::StrokeKind::Inside);
}

/// The fraction under the pointer's x on the slider.
pub fn fraction_at(slider: Rect, x: f32) -> f64 {
    let (a, b) = track(slider);
    (((x - a) / (b - a)) as f64).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rules measured in Studio: 0.05 steps with no ticks on 0–1, 0.25
    /// on 0–5, 0.01 with 100 ticks; Square spaces values quadratically.
    #[test]
    fn studio_steps() {
        let t = SliderSpec { min: 0.0, max: 1.0, ticks: DEFAULT_TICKS, square: false };
        assert_eq!(t.value_at(0.26), 0.25);
        assert_eq!(t.value_at(0.537), 0.55);
        assert_eq!(t.nudge(0.35, 1.0), 0.4);
        let d = SliderSpec { min: 0.0, max: 5.0, ticks: DEFAULT_TICKS, square: false };
        assert_eq!(d.value_at(0.49), 2.5);
        let s = SliderSpec { min: 0.0, max: 1.0, ticks: 100.0, square: false };
        assert_eq!(s.value_at(0.033), 0.03);
        let sq = SliderSpec { min: 0.0, max: 10.0, ticks: 1000.0, square: true };
        assert_eq!(sq.value_at(0.5), 2.5);
        assert!((sq.fraction(2.5) - 0.5).abs() < 1e-9);
        let mut m = HashMap::new();
        parse(
            r#"<Item class="ReflectionMetadataClass"><Properties><string name="Name">BasePart</string></Properties>
            <Item class="ReflectionMetadataMember"><Properties><string name="Name">Transparency</string>
            <double name="UIMinimum">0</double><double name="UIMaximum">1</double></Properties></Item>
            <Item class="ReflectionMetadataMember"><Properties><string name="Name">Volume</string>
            <double name="UIMinimum">0</double><double name="UIMaximum">10</double><double name="UINumTicks">1000</double>
            <string name="SliderScaling">Square</string></Properties></Item></Item>"#,
            &mut m,
        );
        assert_eq!(m[&("BasePart".to_string(), "Transparency".to_string())].ticks, 20.0);
        assert!(m[&("BasePart".to_string(), "Volume".to_string())].square);
    }
}
