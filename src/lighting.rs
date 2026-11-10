//! Lighting as Roblox Studio does it (measured in an offline Studio and read from
//! Studio's own shaders; numbers and sources in docs/MECHANICS.md, "Lighting"):
//! the per-frame lighting constants, the local lights (PointLight, SpotLight,
//! SurfaceLight), the sky's faces, and the "sky visibility" map that decides
//! how much OutdoorAmbient a point gets.

use glam::Vec3;
use rbx_dom_weak::types::{Ref, Variant};

use crate::camera::Camera;
use crate::doc::{self, Document};
use crate::mech;
use crate::render::{mat_cols, Globals, LightRaw};
use crate::scene::{enum_u32, f32_of, vec3_of, xform_of};

/// RUSTUDIO_SHADE_DEBUG: one view (`albedo`), or views by run time
/// (`light@80,local@85,sun@90`: each from that many seconds after start).
fn shade_debug() -> Option<String> {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let v = std::env::var("RUSTUDIO_SHADE_DEBUG").ok()?;
    if !v.contains('@') {
        return Some(v);
    }
    let t = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f32();
    let mut cur = None;
    for item in v.split(',') {
        let (m, at) = item.split_once('@')?;
        if t >= at.parse::<f32>().ok()? {
            cur = Some(m.to_string());
        }
    }
    cur
}

/// RUSTUDIO_LIGHT_OVERRIDE (lighting checks): `Brightness=0;Ambient=128,128,128;nolocal;nopost;noatmos`
/// overrides Lighting properties (numbers, or colours as 0–255 r,g,b) and
/// drops local lights / post effects / the Atmosphere, in any mode.
fn light_overrides() -> &'static Vec<(String, String)> {
    static O: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    O.get_or_init(|| {
        std::env::var("RUSTUDIO_LIGHT_OVERRIDE")
            .unwrap_or_default()
            .split(';')
            .filter(|t| !t.is_empty())
            .map(|t| match t.split_once('=') {
                Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
                None => (t.trim().to_string(), String::new()),
            })
            .collect()
    })
}

fn override_flag(name: &str) -> bool {
    light_overrides().iter().any(|(k, v)| k == name && v.is_empty())
}

fn override_prop(name: &str) -> Option<Variant> {
    let (_, v) = light_overrides().iter().find(|(k, v)| k == name && !v.is_empty())?;
    let parts: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    match parts[..] {
        [x] => Some(Variant::Float32(x)),
        [r, g, b] => Some(Variant::Color3uint8(rbx_dom_weak::types::Color3uint8::new(r as u8, g as u8, b as u8))),
        _ => None,
    }
}

/// Lighting.Technology as the place saved it (None: not saved).
pub fn saved_technology(d: &Document) -> Option<u32> {
    let l = d.find_child(d.dom.root_ref(), "Lighting")?;
    let i = d.dom.get_by_ref(l)?;
    enum_u32(i.properties.get(&rbx_dom_weak::ustr("Technology")).cloned())
}

/// Whether Studio's Default tone curve applies (before Retro grading): a
/// saved Technology other than Compatibility. See `globals`.
pub fn tone_curve_on(d: &Document) -> bool {
    saved_technology(d).is_some_and(|t| !mech::TECHNOLOGY_NO_TONE_CURVE.contains(&t))
}

/// Studio's linearisation: a colour channel squared (gamma 2.0).
pub fn sq(c: [u8; 3]) -> Vec3 {
    let f = |v: u8| (v as f32 / 255.0).powi(2);
    Vec3::new(f(c[0]), f(c[1]), f(c[2]))
}

fn color_of(v: Option<Variant>) -> Option<[u8; 3]> {
    match v? {
        Variant::Color3uint8(c) => Some([c.r, c.g, c.b]),
        Variant::Color3(c) => Some([
            (c.r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (c.g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (c.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]),
        _ => None,
    }
}

fn boolp(d: &Document, r: Ref, name: &str, default: bool) -> bool {
    match d.prop(r, name) {
        Some(Variant::Bool(b)) => b,
        _ => default,
    }
}

/// The sun's direction (toward the sun) for a ClockTime and latitude, as
/// Lighting:GetSunDirection returns it in Studio: θ = (ClockTime − 6) × 15°
/// around the sky, tilted by φ = latitude − 23.5°.
pub fn sun_direction(clock: f32, latitude: f32) -> Vec3 {
    let th = ((clock - 6.0) * 15.0).to_radians();
    let ph = (latitude - mech::SUN_AXIAL_TILT_DEG).to_radians();
    Vec3::new(th.cos() * ph.cos(), th.sin() * ph.cos(), ph.sin())
}

/// Lighting.LightingStyle is Soft (enum 1; Realistic 0).
/// Old places and live joins may contain Technology without LightingStyle.
/// Apply the same migration as Document::ensure_services for file loads.
pub fn soft_style(d: &Document) -> bool {
    let Some(l) = d.find_child(d.dom.root_ref(), "Lighting") else {
        return false;
    };
    let own = d.dom.get_by_ref(l).and_then(|i| {
        i.properties
            .get(&rbx_dom_weak::ustr("LightingStyle"))
            .cloned()
    });
    match enum_u32(own) {
        Some(style) => style == 1,
        None => mech::TECHNOLOGY_SOFT.contains(&enum_u32(d.prop(l, "Technology")).unwrap_or(mech::TECHNOLOGY_SOFT[1])),
    }
}

/// How bright the sky (and the light it gives) is for a sun height: full
/// by day, a dim blue after dusk (measured at 20:18, sun 0.54 below:
/// SKY_NIGHT_TINT of the day sky), nothing deep in the night (00:00).
pub fn sky_tint(sun_y: f32) -> Vec3 {
    let night = Vec3::from(mech::SKY_NIGHT_TINT)
        * ((sun_y - mech::SKY_DARK_SUN_Y) / (mech::SKY_NIGHT_TINT_SUN_Y - mech::SKY_DARK_SUN_Y))
            .clamp(0.0, 1.0);
    let day = ((sun_y - mech::SKY_NIGHT_SUN_Y) / (mech::SKY_DAY_SUN_Y - mech::SKY_NIGHT_SUN_Y))
        .clamp(0.0, 1.0);
    night.lerp(Vec3::ONE, day)
}

/// Post effects under Lighting and the camera (Enabled ones; several of a
/// kind combine).
#[derive(Default, Clone, Copy)]
pub struct Effects {
    pub bloom: Option<(f32, f32, f32)>,
    pub cc: Option<(f32, f32, f32, Vec3)>,
    pub blur: f32,
    pub dof: Option<[f32; 4]>,
    pub retro: bool,
}

pub fn effects(d: &Document, cam_obj: Option<Ref>) -> Effects {
    let mut e = Effects::default();
    let root = d.dom.root_ref();
    let parents: Vec<Ref> = d
        .find_child(root, "Lighting")
        .into_iter()
        .chain(cam_obj)
        .collect();
    let mut cc_acc: Option<(f32, f32, f32, Vec3)> = None;
    for p in parents {
        for c in d
            .dom
            .get_by_ref(p)
            .map(|i| i.children().to_vec())
            .unwrap_or_default()
        {
            let Some(class) = d.class_of(c) else { continue };
            if !boolp(d, c, "Enabled", true) {
                continue;
            }
            let num = |n: &str, dflt: f32| f32_of(d.prop(c, n)).unwrap_or(dflt);
            match class.as_str() {
                "DepthOfFieldEffect" => {
                    e.dof = Some([num("FocusDistance", 0.05).max(0.0), num("InFocusRadius", 10.0).max(0.0),
                        num("NearIntensity", 0.75).clamp(0.0, 1.0), num("FarIntensity", 0.75).clamp(0.0, 1.0)]);
                }
                "BloomEffect" => {
                    e.bloom = Some((
                        num("Intensity", 1.0),
                        num("Size", 24.0),
                        num("Threshold", 2.0),
                    ))
                }
                "ColorCorrectionEffect" => {
                    let tint = color_of(d.prop(c, "TintColor"))
                        .map(|t| Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32) / 255.0)
                        .unwrap_or(Vec3::ONE);
                    let (b, ct, s, t) = cc_acc.unwrap_or((0.0, 0.0, 0.0, Vec3::ONE));
                    cc_acc = Some((
                        b + num("Brightness", 0.0),
                        ct + num("Contrast", 0.0),
                        s + num("Saturation", 0.0),
                        t * tint,
                    ));
                }
                "BlurEffect" => e.blur = e.blur.max(num("Size", 24.0)),
                // TonemapperPreset: Default 0, Retro 1.
                "ColorGradingEffect" => {
                    e.retro = enum_u32(d.prop(c, "TonemapperPreset")) == Some(1)
                }
                _ => {}
            }
        }
    }
    e.cc = cc_acc;
    e
}

/// Lighting.Atmosphere (the first Atmosphere child) as Studio's renderer
/// turns it into AdvSky props and fog.
#[derive(Clone, Copy, Debug)]
pub struct Atmos {
    /// Color² (FogColor before the sky's irradiance).
    pub color_sq: Vec3,
    /// AdvSkyFS fogOffset.x (−A / ln 2) and fogOffset.y (B).
    pub fog_offset: [f32; 2],
    /// Glare, and its per-channel exponents (from Decay).
    pub glare: f32,
    pub glare_exp: Vec3,
    /// Offset: the least blur of the sky the fog fades into.
    pub offset: f32,
    /// How fast objects fade into the sky, per stud (natural log).
    pub fog_k: f32,
}

/// The Atmosphere's props as Studio computes them: the change hook at
/// (A, B from Haze), the Decay transform and the
/// fog setup.
pub fn atmosphere(d: &Document) -> Option<Atmos> {
    let l = d.find_child(d.dom.root_ref(), "Lighting")?;
    let a = d.find_child(l, "Atmosphere")?;
    let num = |n: &str, dflt: f32| f32_of(d.prop(a, n)).unwrap_or(dflt);
    // Defaults: the Atmosphere constructor's: Density 0.395,
    // Color (199, 170, 107), Decay (92, 60, 13), the rest 0.
    let density = num("Density", 0.395);
    let haze = num("Haze", 0.0).clamp(0.0, 10.0);
    let glare = num("Glare", 0.0);
    let offset = num("Offset", 0.0);
    let color = color_of(d.prop(a, "Color")).unwrap_or([199, 170, 107]);
    let decay = color_of(d.prop(a, "Decay")).unwrap_or([92, 60, 13]);
    let ca = (haze - mech::ATMOS_HAZE_THRESHOLD).clamp(mech::ATMOS_A_MIN, 1.0);
    let cb = haze * mech::ATMOS_B_HAZE + if haze != 0.0 { mech::ATMOS_B_ON } else { 0.0 };
    let dexp = |c: u8| {
        let u = 1.0 - c as f32 / 255.0;
        (u * u + mech::ATMOS_DECAY_LIN * u).exp2() - 1.0
    };
    let lin = |c: [u8; 3]| Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) / 255.0;
    let c = lin(color);
    Some(Atmos {
        color_sq: c * c,
        fog_offset: [-ca / std::f32::consts::LN_2, cb],
        glare,
        glare_exp: Vec3::new(dexp(decay[0]), dexp(decay[1]), dexp(decay[2])),
        offset,
        fog_k: mech::ATMOS_FOG_DENSITY_K
            * density.clamp(0.0, 1.0).powf(mech::ATMOS_FOG_DENSITY_POW),
    })
}

/// AdvSkyFS irradiance × FogColor on Studio's desktop renderer. At
/// → byte >= 2 bypasses
/// the legacy mean-irradiance/sky-level factor and uploads Color² directly.
pub fn atmos_haze_cube(a: &Atmos, sky_irr: &[Vec3; 6]) -> [Vec3; 6] {
    sky_irr.map(|e| e * a.color_sq)
}

/// Everything the shaders need for a frame.
pub struct FrameLighting {
    pub globals: Globals,
    pub sun: Vec3,
    /// Toward the sun (Lighting:GetSunDirection), even at night: the light
    /// grid's sun channel flips it to the moon itself.
    pub sun_true: Vec3,
    pub shadows: bool,
    /// Lighting.ShadowSoftness: the sun's shadow blur (sun_shadow.rs).
    pub shadow_softness: f32,
}

/// Lighting's time-of-day sky levels for a ClockTime and
/// EnvironmentDiffuseScale `s`: (c0, c1, k) as Studio keeps them in its
/// Lighting members (mech:SKY_LEVELS). c0² and c1²
/// are the shaders' SkyGradientTop / Bottom.
pub fn sky_levels(clock: f32, s: f32) -> (Vec3, Vec3, f32) {
    let t = mech::SKY_LEVELS;
    let n = t.len() / mech::SKY_LEVEL_COLUMNS;
    let row = |i: usize| &t[(i % n) * mech::SKY_LEVEL_COLUMNS..(i % n + 1) * mech::SKY_LEVEL_COLUMNS];
    let h = clock.rem_euclid(24.0);
    let mut i = n - 1;
    for k in 0..n {
        if row(k)[0] <= h {
            i = k;
        }
    }
    let (a, b) = (row(i), row(i + 1));
    let tb = if i + 1 >= n { b[0] + 24.0 } else { b[0] };
    let f = if tb > a[0] { ((h - a[0]) / (tb - a[0])).clamp(0.0, 1.0) } else { 0.0 };
    let v = |j: usize| a[j] + (b[j] - a[j]) * f;
    let v3 = |j: usize| Vec3::new(v(j), v(j + 1), v(j + 2));
    let s = s.clamp(0.0, 1.0);
    let c0 = v3(1).lerp(v3(4), s);
    let c1 = v3(7).lerp(v3(10), s);
    let k = v(13).max(s * v(14));
    (c0, c1, k)
}

pub fn globals(
    d: &Document,
    cam: &Camera,
    rect: egui::Rect,
    light_count: usize,
    skybox: bool,
    sky_irr: &[Vec3; 6],
    indoor_irr: &[Vec3; 6],
) -> FrameLighting {
    let lighting = d.find_child(d.dom.root_ref(), "Lighting");
    let get = |name: &str| override_prop(name).or_else(|| lighting.and_then(|l| d.prop(l, name)));
    // Files keep the time as TimeOfDay ("HH:MM:SS"); scripts set ClockTime.
    let time_of_day = || match get("TimeOfDay") {
        Some(Variant::String(s)) => {
            let p: Vec<f32> = s
                .split(':')
                .map(|x| x.trim().parse::<f32>().unwrap_or(0.0))
                .collect();
            Some(
                p.first().copied().unwrap_or(0.0)
                    + p.get(1).copied().unwrap_or(0.0) / 60.0
                    + p.get(2).copied().unwrap_or(0.0) / 3600.0,
            )
        }
        _ => None,
    };
    let clock = f32_of(get("ClockTime"))
        .or_else(time_of_day)
        .map(mech::clock_time)
.unwrap_or(mech::CLOCK_TIME); // clamp + whole seconds
    let lat = f32_of(get("GeographicLatitude")).unwrap_or(mech::GEOGRAPHIC_LATITUDE);
    let brightness = f32_of(get("Brightness")).unwrap_or(mech::LIGHT_BRIGHTNESS);
    let exposure = f32_of(get("ExposureCompensation")).unwrap_or(0.0);
    let env_diffuse = f32_of(get("EnvironmentDiffuseScale"))
        .unwrap_or(0.0)
.clamp(0.0, mech::ENV_SCALE_MAX);
    let env_specular = f32_of(get("EnvironmentSpecularScale"))
        .unwrap_or(0.0)
        .clamp(0.0, mech::ENV_SCALE_MAX);
    let ambient = sq(color_of(get("Ambient")).unwrap_or(mech::AMBIENT));
    let outdoor = sq(color_of(get("OutdoorAmbient")).unwrap_or(mech::OUTDOOR_AMBIENT));
    let shift = color_of(get("ColorShift_Top")).unwrap_or([0, 0, 0]);
    let shadows = matches!(get("GlobalShadows"), Some(Variant::Bool(true)) | None);
    // Tone curve (offline Studio 2026-11-08, lightov/tone/REPORT.md): a saved
    // Legacy / Voxel / ShadowMap / Future draws Studio's Default curve;
    // Compatibility is migrated to Voxel + a Retro ColorGradingEffect
    // (≈ sqrt). A place saving no Technology at all (Place A) draws without the
    // curve too: its Studio lobby capture fits sqrt (fidelity-place_a MAE 14.5
    // vs 21.2 with the curve).
    let tech = saved_technology(d).unwrap_or(mech::TECHNOLOGY_SOFT[1]);
    let no_tone = !tone_curve_on(d);

    let sun = sun_direction(clock, lat);
    let (c0, c1, sky_k) = sky_levels(clock, env_diffuse);
    // The sun / moon (Lamp0) and the back light (Lamp1), as Studio builds
    // them: the sky level k² shifted
    // toward ColorShift_Top² / _Bottom² by EnvironmentDiffuseScale, ×
    // Brightness × 0.9 / 0.36. Lamp0 comes from the moon (−sun) once the sun
    // is below LAMP_MOON_BELOW_SUN_Y.
    let lamp_mix = |a: Vec3, b: Vec3, c: Vec3| -> Vec3 {
        let la = a.length();
        let mean = (c.x + c.y + c.z) * mech::LAMP_COLOR_MEAN;
        let dc = c - Vec3::splat(mean);
        let shifted = a + dc * la;
        let sat = dc.length();
        ((c * la - b) * sat + b - shifted) * env_diffuse + shifted
    };
    let k2 = Vec3::splat(sky_k * sky_k);
    let shift_bottom = color_of(get("ColorShift_Bottom")).unwrap_or([0, 0, 0]);
    let b = brightness.max(0.0);
    let lamp0 = lamp_mix(k2, k2, sq(shift)) * (mech::LAMP0_GAIN * b);
    let lamp1 = lamp_mix(k2, Vec3::ZERO, sq(shift_bottom)) * (mech::LAMP1_GAIN * b);
    let light_dir = if sun.y >= mech::LAMP_MOON_BELOW_SUN_Y { sun } else { -sun };
    let light_irr = ((lamp0.x + lamp0.y + lamp0.z) / 3.0).max(0.0);
    let sun_color = if light_irr > 1e-6 { lamp0 / light_irr } else { Vec3::ONE };
    let day = ((sun.y - mech::SKY_NIGHT_SUN_Y) / (mech::SKY_DAY_SUN_Y - mech::SKY_NIGHT_SUN_Y))
        .clamp(0.0, 1.0);
    // Outdoors a point gets the larger of Ambient and OutdoorAmbient, at
    // night too (measured at 00:00: side faces 130 vs 132 predicted).
    let outdoor_extra = (outdoor - ambient).max(Vec3::ZERO);

    let cam_obj = d
        .workspace()
        .and_then(|ws| match d.prop(ws, "CurrentCamera") {
            Some(Variant::Ref(c)) if d.exists(c) => Some(c),
            _ => None,
        });
    let mut fx = effects(d, cam_obj);
    if override_flag("nopost") {
        fx.cc = None;
        fx.bloom = Some((0.0, mech::DEFAULT_GLOW_SIZE_PX, mech::DEFAULT_GLOW_THRESHOLD));
    }
    let no_tone = no_tone || fx.retro;
    let fog_start = f32_of(get("FogStart")).unwrap_or(0.0);
    let fog_end = f32_of(get("FogEnd")).unwrap_or(100_000.0);
    let fog_color = sq(color_of(get("FogColor")).unwrap_or([192, 192, 192]));
    let atmos = if override_flag("noatmos") { None } else { atmosphere(d) };
    // With an Atmosphere, Lighting's FogStart / FogEnd don't apply (Roblox
    // docs): objects fade into the sky behind them, e^(−k·d) (fog.z 3,
    // exp2 form as Studio's FogParams).
    let fog = match atmos {
        Some(a) if a.fog_k > 0.0 => [0.0, a.fog_k / std::f32::consts::LN_2, 3.0, day],
        Some(_) => [0.0, 1e9, 0.0, day],
        None => [
            fog_start,
            fog_end,
            if fog_end < 99_999.0 { 1.0 } else { 0.0 },
            day,
        ],
    };
    // Studio's default sky swaps to its night faces after dark, so the haze
    // it lights dims with them (Haze 1.9 at 02:48: (3, 5, 5) on screen with
    // the default sky, (62, 77, 108) with a custom one); rustudio keeps the
    // day faces and dims them by sky_tint, so the haze follows sky_tint too.
    let default_sky = sky_faces(d)[0].starts_with("rbxasset://textures/sky/sky512");
    // AdvSky's irradiance stays in the source sky frame.
    // Re-express the ambient cube supplied by the rotated reflection sky.
    // Axis-aligned rotations are exact; general rotations use the same
    // squared-axis reconstruction as AdvSkyFS.
    let orient = sky_orientation(d);
    let r = glam::Mat3::from_rotation_z(orient.z.to_radians())
        * glam::Mat3::from_rotation_x(orient.x.to_radians())
        * glam::Mat3::from_rotation_y(orient.y.to_radians());
    let axes = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
    let atmos_irr = axes.map(|axis| {
        let v = r * axis;
        sky_irr[if v.x >= 0.0 { 0 } else { 1 }] * v.x * v.x
            + sky_irr[if v.y >= 0.0 { 2 } else { 3 }] * v.y * v.y
            + sky_irr[if v.z >= 0.0 { 4 } else { 5 }] * v.z * v.z
    });
    let haze_cube = atmos.map(|a| {
        let tint = if default_sky { sky_tint(sun.y) } else { Vec3::ONE };
        let cube = atmos_haze_cube(&a, &atmos_irr);
        cube.map(|c| c * tint)
    });
    let stars = d
        .find_child(d.dom.root_ref(), "Lighting")
        .and_then(|l| sky_child(d, l))
        .map(|s| f32_of(d.prop(s, "StarCount")).unwrap_or(3000.0) > 0.0)
        .unwrap_or(true);

    // Without a BloomEffect Studio still glows what is over-bright (Neon):
    // tools/fidelity control shots show a ~12 px halo around a 1-stud Neon bar
    // in a place with no post effects.
    let (bloom_i, bloom_size, bloom_thr) = fx.bloom.unwrap_or((
        mech::DEFAULT_GLOW_INTENSITY,
        mech::DEFAULT_GLOW_SIZE_PX,
        mech::DEFAULT_GLOW_THRESHOLD,
    ));
    let (ccb, ccc, ccs, cct) = fx.cc.unwrap_or((0.0, 0.0, 0.0, Vec3::ONE));
    // Light debug views read raw: no curve, colour correction or bloom,
    // so the screen is sqrt(light / 4).
    let raw_debug = matches!(shade_debug().as_deref(), Some("light" | "local" | "sun" | "ambient" | "albedo" | "shadowmap" | "voxel-sun" | "sky-raw"));
    let no_tone = no_tone || raw_debug;
    let bloom_i = if raw_debug { 0.0 } else { bloom_i };
    let fx_cc_on = fx.cc.is_some() && !raw_debug;
    let globals = Globals {
        view_proj: mat_cols(cam.view_proj_reverse_z(rect)),
        inv_view_proj: mat_cols(cam.inv_view_proj_rel(rect)),
        cam_pos: cam.pos.extend(1.0).to_array(),
        sun_dir: light_dir.extend(light_irr).to_array(),
        // w: Lighting.LightingStyle (Realistic 0, Soft 1).
        sun_color: sun_color
            .extend(if soft_style(d) { 1.0 } else { 0.0 })
            .to_array(),
        ambient: ambient.extend(1.0).to_array(),
        outdoor: outdoor_extra.extend(1.0).to_array(),
        sky_top: (sq(mech::SKY_GRADIENT_TOP)).extend(1.0).to_array(),
        sky_horizon: (sq(mech::SKY_GRADIENT_HORIZON)).extend(1.0).to_array(),
        params: [
            light_count as f32,
            2f32.powf(exposure),
            if no_tone { 1.0 } else { 0.0 },
            if skybox { 1.0 } else { 0.0 },
        ],
        fog,
        fog_color: fog_color.extend(1.0).to_array(),
        cc: [ccb, ccc, ccs, if fx_cc_on { 1.0 } else { 0.0 }],
        cc_tint: [cct.x, cct.y, cct.z, bloom_i],
        bloom: [
            bloom_thr,
            bloom_size,
            if shadows && light_irr > 0.0 { 1.0 } else { 0.0 },
            0.0,
        ],
        sun_casc: [[[0.0; 4]; 4]; 4],
        sun_sphere: [[0.0; 4]; 4],
        sun_rect: [[0.0; 4]; 4],
        // z: RUSTUDIO_SHADE_DEBUG=normals|albedo shows those instead of light.
        sky_rot: [
            if stars { 1.0 } else { 0.0 },
            fx.blur,
            match shade_debug().as_deref() {
                Some("normals") => 1.0,
                Some("albedo") => 2.0,
                Some("uv") => 3.0,
                Some("skylight") => 4.0,
                Some("light") => 5.0,
                Some("local") => 6.0,
                Some("sun") => 7.0,
                Some("ambient") => 8.0,
                Some("shadowmap") => 9.0,
                Some("voxel-sun") => 10.0,
                Some("sky-raw") => 11.0,
                _ => 0.0,
            },
            0.0,
        ],
        // AmbientCube[0..5] + [6..11] as Studio builds them:
        // the sky's irradiance per axis × EnvironmentDiffuseScale × the sky
        // level (+Y c0², −Y c1², sides their mean); the shader blends it
        // with the indoor cube by skylight.
        env: {
            let (a, b) = (c0 * c0, c1 * c1);
            let side = (a + b) * 0.5;
            let lv = [side, side, a, b, side, side];
            std::array::from_fn(|i| (sky_irr[i] * lv[i] * env_diffuse).extend(1.0).to_array())
        },
        env_indoor: indoor_irr.map(|e| (e * env_diffuse).extend(1.0).to_array()),
        sky_grad: [
            (c0 * c0).extend(env_diffuse).to_array(),
            (c1 * c1).extend(env_specular).to_array(),
        ],
        lamp1: lamp1.extend(1.0).to_array(),
        ssao: {
            // Studio's SSAOParams.xy: half the projection's diagonal.
            let f = 1.0 / (cam.fov_deg.to_radians() * 0.5).tan();
            let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
            // (Off for an orthographic camera: the AO reconstructs a
            // perspective view.)
            [0.5 * f / aspect, 0.5 * f, mech::NEAR, if cam.ortho.is_some() { 0.0 } else { 1.0 }]
        },
        // w: 1 dims the drawn skybox by the tint (Studio's default sky, whose
        // night faces rustudio stands in for); a custom Sky's faces are drawn
        // as they are at any time of day (Place D's night skybox, round 27).
        sky_tint: sky_tint(sun.y).extend(if default_sky { 1.0 } else { 0.0 }).to_array(),
        lightgrid: [0.0; 4],
        // w 1 = an Atmosphere: rgb its haze toward the horizon (world GUI fog).
        atmos: haze_cube
            .map(|c| ((c[0] + c[1] + c[4] + c[5]) * 0.25).extend(1.0).to_array())
            .unwrap_or([0.0; 4]),
        atmos_cube: haze_cube
            .map(|c| c.map(|e| e.extend(1.0).to_array()))
            .unwrap_or([[0.0; 4]; 6]),
        atmos_sky: atmos
            .map(|a| [a.fog_offset[0], a.fog_offset[1], a.glare, a.offset])
            .unwrap_or([0.0; 4]),
        atmos_glare: atmos
            .map(|a| a.glare_exp.extend(0.0).to_array())
            .unwrap_or([0.0; 4]),
        // Only ShadowMap / Future part shaders reflect the sky and draw the
        // sun's highlight (Voxel / Compatibility's FS uses the scale for
        // the sun highlight only).
        env_spec: if !soft_style(d) || mech::TECHNOLOGY_QUALITY.contains(&tech) {
            [env_specular, mech::ENV_SPEC_ROUGHNESS, 1.0, 0.0]
        } else {
            [0.0, mech::ENV_SPEC_ROUGHNESS, 0.0, 0.0]
        },
        lightocc: [0.0; 4],
        dof: fx.dof.unwrap_or([0.0; 4]),

    };
    if std::env::var_os("RUSTUDIO_GLOBALS_DEBUG").is_some() {
        static LAST: std::sync::Mutex<Vec<(u32, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());
        let mut last = LAST.lock().unwrap();
        let key = (clock * 100.0) as u32 ^ (tech << 24);
        let due = match last.iter_mut().find(|e| e.0 == key) {
            Some(e) if e.1.elapsed().as_secs_f32() < 5.0 => false,
            Some(e) => { e.1 = std::time::Instant::now(); true }
            None => { last.push((key, std::time::Instant::now())); true }
        };
        if due {
            eprintln!("ATMOS| irr {:?} cube {:?} c0 {:?} c1 {:?}", sky_irr, globals.atmos_cube, c0, c1);
            let gl = &globals;
            eprintln!(
                "GLOBALS| tech {tech} clock {clock} sun_dir {:?} sun_color {:?} ambient {:?} outdoor {:?} params {:?} env_spec {:?} lamp1 {:?} env {:?} env_indoor {:?} sky_grad {:?} bloom {:?} lightgrid {:?} fog {:?} cc {:?}",
                gl.sun_dir, gl.sun_color, gl.ambient, gl.outdoor, gl.params, gl.env_spec, gl.lamp1, gl.env, gl.env_indoor, gl.sky_grad, gl.bloom, gl.lightgrid, gl.fog, gl.cc
            );
        }
    }
    FrameLighting {
        globals,
        sun: light_dir,
        sun_true: sun,
        shadows: shadows && light_irr > 0.0,
        shadow_softness: f32_of(get("ShadowSoftness")).unwrap_or(mech::SHADOW_SOFTNESS).clamp(0.0, mech::SHADOW_SOFTNESS_MAX),
    }
}

// ------------------------------------------------------------ local lights

/// Lights in the document (rebuilt when the tree changes).
#[derive(Default)]
pub struct LightCache {
    doc_id: u64,
    /// The lights under Workspace, in tree order (then as added), kept from
    /// the document's journal: rescanning Place B's 116k instances on every
    /// tree change was most of the lighting time.
    lights: Vec<Ref>,
    set: std::collections::HashSet<Ref>,
}

fn is_light(d: &Document, r: Ref) -> bool {
    d.dom
        .get_by_ref(r)
        .is_some_and(|i| matches!(i.class.as_str(), "PointLight" | "SpotLight" | "SurfaceLight"))
}

impl LightCache {
    fn refresh(&mut self, d: &mut Document) {
        let marks = d.marks_for(doc::Reader::Lights);
        if let doc::Marks::Some(marks) = marks {
            if self.doc_id == d.id {
                let ws = d.workspace();
                let mut removed = false;
                for m in marks {
                    let doc::Mark::Tree(t) = m else { continue };
                    for r in d.subtree(t) {
                        let keep = is_light(d, r) && ws.is_some_and(|w| d.is_ancestor(w, r));
                        if keep && self.set.insert(r) {
                            self.lights.push(r);
                        } else if !keep && self.set.remove(&r) {
                            removed = true;
                        }
                    }
                }
                if removed {
                    let set = &self.set;
                    self.lights.retain(|r| set.contains(r));
                }
                return;
            }
        }
        self.doc_id = d.id;
        self.lights = match d.workspace() {
            Some(ws) => d
                .dom
                .descendants_of(ws)
                .filter(|i| {
                    matches!(
                        i.class.as_str(),
                        "PointLight" | "SpotLight" | "SurfaceLight"
                    )
                })
                .map(|i| i.referent())
                .collect(),
            None => Vec::new(),
        };
        self.set = self.lights.iter().copied().collect();
    }

    /// The parts lights hang from (directly or by an Attachment): they
    /// don't block their own light (round 26, lightocc.rs).
    pub fn hosts(&mut self, d: &mut Document) -> std::collections::HashSet<Ref> {
        self.refresh(d);
        let mut out = std::collections::HashSet::new();
        for &l in &self.lights {
            let Some(mut h) = d.dom.get_by_ref(l).map(|i| i.parent()) else { continue };
            while d.class_of(h).is_some_and(|c| doc::is_a(&c, "Attachment")) {
                match d.dom.get_by_ref(h) {
                    Some(i) => h = i.parent(),
                    None => continue,
                }
            }
            out.insert(h);
        }
        out
    }

    /// Enabled lights in the shader's layout. Realistic keeps the nearest
    /// MAX_LIGHTS reaching the view; Soft gathers the whole voxel volume.
    pub fn gather(
        &mut self,
        d: &mut Document,
        cam: &Camera,
        planes: &dyn Fn(Vec3, f32) -> bool,
    ) -> Vec<LightRaw> {
        self.refresh(d);
        if override_flag("nolocal") {
            return Vec::new();
        }
        let d: &Document = d;
        let soft = soft_style(d);
        let mut out: Vec<(f32, LightRaw)> = Vec::new();
        for &l in &self.lights {
            let Some(class) = d.class_of(l) else { continue };
            if !boolp(d, l, "Enabled", true) {
                continue;
            }
            let brightness = f32_of(d.prop(l, "Brightness")).unwrap_or(1.0);
            let range = f32_of(d.prop(l, "Range"))
                .unwrap_or(8.0)
                .clamp(0.0, mech::LIGHT_MAX_RANGE);
            if brightness <= 0.0 || range <= 0.0 {
                continue;
            }
            let Some(host) = d.dom.get_by_ref(l).map(|i| i.parent()) else {
                continue;
            };
            let Some(hclass) = d.class_of(host) else {
                continue;
            };
            // Where the light sits: a part's centre, or an attachment's world position.
            let (pos, rot, size) = if doc::is_a(&hclass, "BasePart") {
                let Some(xf) = xform_of(d, host) else {
                    continue;
                };
                (
                    xf.pos,
                    xf.rot,
                    vec3_of(d.prop(host, "Size")).unwrap_or(Vec3::ONE),
                )
            } else if doc::is_a(&hclass, "Attachment") {
                let Some(xf) = crate::attachment::world(d, host, true) else { continue };
                (xf.pos, xf.rot, Vec3::ZERO)
            } else {
                continue;
            };
            let source_color = d.prop(l, "Color");
            let rgb = color_of(source_color.clone()).unwrap_or([255, 255, 255]);
            // Realistic (Future): Studio's light record holds Color^2.2 ×
            // Brightness (read from its light buffer to 4 digits).
            let color = if soft {
                // round(Color * 256), saturate to a byte,
                // then square byte/255 before accumulating voxel radiance.
                let c = match source_color {
                    Some(Variant::Color3(c)) => Vec3::new(c.r, c.g, c.b),
                    _ => Vec3::from_array(rgb.map(|v| v as f32 / 255.0)),
                };
                let c = (c * 256.0).round().clamp(Vec3::ZERO, Vec3::splat(255.0)) / 255.0;
                c * c * brightness
            } else {
                (Vec3::new(rgb[0] as f32, rgb[1] as f32, rgb[2] as f32) / 255.0).powf(2.2) * brightness
            };
            // Face (NormalId): Right 0, Top 1, Back 2, Left 3, Bottom 4, Front 5.
            let face = |f: u32| match f {
                0 => Vec3::X,
                1 => Vec3::Y,
                2 => Vec3::Z,
                3 => -Vec3::X,
                4 => -Vec3::Y,
                _ => -Vec3::Z,
            };
            let cutoff = |angle: f32| {
                if soft {
                    // CPU voxel writer's approximation to 1-cos(Angle/2),
                    // coefficients, full angle in degrees.
                    let a = angle * angle;
                    1.0 - ((3.8077018e-5 - (2.4119384e-10 - a * 5.733313e-16) * a) * a + 1e-15)
                } else { (angle.to_radians() * 0.5).cos() }
            };
            let (kind, dir, cos_cut, centre, half) = match class.as_str() {
                "SpotLight" => {
                    let n = face(enum_u32(d.prop(l, "Face")).unwrap_or(5));
                    let angle = f32_of(d.prop(l, "Angle"))
                        .unwrap_or(90.0)
                        .clamp(0.0, mech::LIGHT_MAX_ANGLE);
                    (
                        1.0,
                        rot * n,
                        cutoff(angle),
                        pos,
                        [0.0, 0.0],
                    )
                }
                "SurfaceLight" => {
                    let fid = enum_u32(d.prop(l, "Face")).unwrap_or(5);
                    let n = face(fid);
                    let angle = f32_of(d.prop(l, "Angle"))
                        .unwrap_or(90.0)
                        .clamp(0.0, mech::LIGHT_MAX_ANGLE);
                    let h = size * 0.5;
                    let along = n.abs();
                    let centre = pos + rot * (n * (h * along).length());
                    let rest = h * (Vec3::ONE - along);
                    let mut ext = [rest.x, rest.y, rest.z];
                    ext.sort_by(|a, b| b.total_cmp(a));
                    (
                        2.0,
                        rot * n,
                        cutoff(angle),
                        centre,
                        [ext[0], ext[1]],
                    )
                }
                _ => (0.0, Vec3::Y, -1.0, pos, [0.0, 0.0]),
            };
            // Soft fills the camera's voxel volume, including lights behind
            // the frustum; Realistic evaluates the visible light spheres.
            if !soft && !planes(centre, range) {
                continue;
            }
            let dist = (centre - cam.pos).length();
            let fid = enum_u32(d.prop(l, "Face")).unwrap_or(5) % 3;
            let (u, v) = match fid {
                0 => (Vec3::Y, Vec3::Z), 1 => (Vec3::X, Vec3::Z), _ => (Vec3::X, Vec3::Y),
            };
            out.push((
                dist,
                LightRaw {
                    surface_u: (rot * u).extend(size.dot(u) * 0.5).to_array(),
                    surface_v: (rot * v).extend(size.dot(v) * 0.5).to_array(),
                    pos_range: centre.extend(range).to_array(),
                    color: color.extend(kind).to_array(),
                    dir: dir.normalize_or_zero().extend(cos_cut).to_array(),
                    // z: Shadows on, parts block it (round 26, lightocc.rs).
                    // w: its shadow-map slot (set below; −1 none).
                    extra: [half[0], half[1], if boolp(d, l, "Shadows", false) { 1.0 } else { 0.0 }, -1.0],
                },
            ));
        }
        if !soft {
            out.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.truncate(mech::MAX_LIGHTS);
        }
        // Realistic: the nearest Shadows lights within LOCAL_LIGHT_DIST get a
        // cube shadow map each (Studio's Future clustered lights).
        if !soft {
            let mut slot = 0usize;
            for (dist, l) in out.iter_mut() {
                if slot >= mech::LSHADOW_MAX_LIGHTS {
                    break;
                }
                if l.extra[2] > 0.5 && *dist - l.pos_range[3] <= mech::LOCAL_LIGHT_DIST {
                    l.extra[3] = slot as f32;
                    slot += 1;
                }
            }
        }
        if std::env::var_os("RUSTUDIO_LIGHT_DEBUG").is_some() {
            for (dist, l) in out.iter().take(8) {
                eprintln!("light d={dist:.1} {:?}", l);
            }
        }
        out.into_iter().map(|(_, l)| l).collect()
    }
}

// ------------------------------------------------------------- sky faces

/// Sky instances are selected by class; games commonly rename clones.
fn sky_child(d: &Document, parent: Ref) -> Option<Ref> {
    d.dom.get_by_ref(parent)?.children().iter().copied()
        .find(|r| d.class_of(*r).is_some_and(|c| c == "Sky"))
}

/// Sky.SkyboxOrientation in degrees (zero without a Sky). Studio turns the
/// whole skybox by it: the sky drawn toward a direction d is the unturned
/// sky's toward R⁻¹·d, R = Rz·Rx·Ry (each Roblox's rotation about that axis)
/// (docs/MECHANICS.md, "Skybox orientation").
pub fn sky_orientation(d: &Document) -> Vec3 {
    d.find_child(d.dom.root_ref(), "Lighting")
        .and_then(|l| sky_child(d, l))
        .and_then(|s| match d.prop(s, "SkyboxOrientation") {
            Some(Variant::Vector3(v)) => Some(Vec3::new(v.x, v.y, v.z)),
            _ => None,
        })
        .filter(|v| v.is_finite())
        .unwrap_or(Vec3::ZERO)
}

/// The six skybox images (Bk, Dn, Ft, Lf, Rt, Up order as Sky names them):
/// the Sky's own, else Studio's default sky.
pub fn sky_faces(d: &Document) -> [String; 6] {
    let sky = d
        .find_child(d.dom.root_ref(), "Lighting")
        .and_then(|l| sky_child(d, l));
    let names = [
        "SkyboxBk", "SkyboxDn", "SkyboxFt", "SkyboxLf", "SkyboxRt", "SkyboxUp",
    ];
    let defaults = ["bk", "dn", "ft", "lf", "rt", "up"];
    let mut out: [String; 6] = Default::default();
    for i in 0..6 {
        let own = sky.and_then(|s| match d.prop(s, names[i]) {
            Some(Variant::ContentId(c)) if !c.as_str().is_empty() => Some(c.as_str().to_string()),
            Some(Variant::Content(c)) => {
                c.as_uri().filter(|u| !u.is_empty()).map(|u| u.to_string())
            }
            _ => None,
        });
        out[i] =
            own.unwrap_or_else(|| format!("rbxasset://textures/sky/sky512_{}.tex", defaults[i]));
    }
    out
}

/// Light arriving from the skybox on a surface facing each axis (+X, −X,
/// +Y, −Y, +Z, −Z), from six square RGBA8 faces in the same cube order:
/// E(n) = 1/π ∫ L(ω) max(n·ω, 0) dω with texels squared (gamma 2.0). The
/// shader blends the six by n² (an "ambient cube").
pub fn sky_irradiance(faces: &[Vec<u8>; 6], size: u32) -> [Vec3; 6] {
    let axes = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
    let mut sum = [Vec3::ZERO; 6];
    let step = (size / 32).max(1);
    for (f, px) in faces.iter().enumerate() {
        for y in (0..size).step_by(step as usize) {
            for x in (0..size).step_by(step as usize) {
                let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                // Face texel → direction (wgpu / D3D cube convention).
                let d = match f {
                    0 => Vec3::new(1.0, -v, -u),
                    1 => Vec3::new(-1.0, -v, u),
                    2 => Vec3::new(u, 1.0, v),
                    3 => Vec3::new(u, -1.0, -v),
                    4 => Vec3::new(u, -v, 1.0),
                    _ => Vec3::new(-u, -v, -1.0),
                };
                let w = 1.0 / (1.0 + u * u + v * v).powf(1.5);
                let o = ((y * size + x) * 4) as usize;
                let Some(t) = px.get(o..o + 3) else { continue };
                let l = sq([t[0], t[1], t[2]]);
                let d = d.normalize();
                for (k, a) in axes.iter().enumerate() {
                    let c = a.dot(d);
                    if c > 0.0 {
                        sum[k] += l * c * w;
                    }
                }
            }
        }
    }
    // Each sample covers (2·step/size)² of the face; the whole sphere is 4π.
    let texel = (2.0 * step as f32 / size as f32).powi(2);
    sum.map(|s| s * texel / std::f32::consts::PI)
}


#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::{InstanceBuilder, WeakDom};

    #[test]
    fn renamed_sky_and_camera_effects() {
        let dom = WeakDom::new(InstanceBuilder::new("DataModel").with_child(
            InstanceBuilder::new("Lighting").with_name("Lighting")
                .with_child(InstanceBuilder::new("Sky").with_name("FactorySky")
                    .with_property("SkyboxBk", rbx_dom_weak::types::ContentId::from("rbxassetid://123")))
                .with_child(InstanceBuilder::new("DepthOfFieldEffect")
                    .with_property("FocusDistance", 4.72f32)
                    .with_property("FarIntensity", 0.567f32)
                    .with_property("NearIntensity", 0.0f32)
                    .with_property("InFocusRadius", 0.0f32))
                .with_child(InstanceBuilder::new("DepthOfFieldEffect").with_property("Enabled", false).with_property("FocusDistance", 999.0f32))));
        let d = Document::from_play_dom(dom, "effects".into());
        assert!(soft_style(&d), "live documents bypass file-load lighting migration");
        assert_eq!(sky_faces(&d)[0], "rbxassetid://123");
        assert_eq!(effects(&d, None).dof, Some([4.72, 0.0, 0.0, 0.567]));

    }
}
