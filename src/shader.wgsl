// rustudio viewport shader: sky, parts, lines, shadows and the final image.
//
// Lighting follows Roblox Studio's own shaders and measurements (see
// docs/MECHANICS.md, "Lighting"): colours are linearised by squaring (gamma 2.0),
// light = Ambient + max(OutdoorAmbient - Ambient, 0) x sky visibility
// + sun x shadow + local lights, the scene is drawn into a float target,
// and the post pass applies exposure, the tone mapper (Default: Studio's
// measured curve; Retro: sqrt), colour correction and bloom.

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>, // camera-relative (camera at the origin): world = cam_pos + (inv * ndc).xyz / w
    cam_pos: vec4<f32>,
    sun_dir: vec4<f32>,      // xyz toward the sun, w irradiance (0.5 x Brightness)
    sun_color: vec4<f32>,    // rgb (ColorShift_Top hue, average 1), w LightingStyle (0 Realistic, 1 Soft)
    ambient: vec4<f32>,      // Ambient (squared)
    outdoor: vec4<f32>,      // max(OutdoorAmbient - Ambient, 0) (squared)
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    params: vec4<f32>,       // x lights, y exposure (2^ExposureCompensation), z tonemapper (0 Default, 1 Retro), w sky (0 gradient, 1 box)
    fog: vec4<f32>,          // x FogStart, y FogEnd, z on, w sky brightness
    fog_color: vec4<f32>,
    cc: vec4<f32>,           // ColorCorrection Brightness, Contrast, Saturation, on
    cc_tint: vec4<f32>,      // TintColor, w Bloom Intensity
    bloom: vec4<f32>,        // x Threshold, y Size (BloomEffect units, 24 = Studio's glow), z shadows on
    sun_casc: array<mat4x4<f32>, 4>, // the sun's shadow cascades: world to the tile's (u, v, depth)
    sun_sphere: array<vec4<f32>, 4>, // each cascade's sphere: centre, radius squared
    sun_rect: array<vec4<f32>, 4>,   // each tile in the atlas: corner, size (UV)
    sky_rot: vec4<f32>,      // x stars (0/1), y blur size
    env: array<vec4<f32>, 6>, // skybox irradiance x EnvironmentDiffuseScale (+X, -X, +Y, -Y, +Z, -Z)
    sky_tint: vec4<f32>,     // the sky (and its light) by time of day: 1 by day, a dim blue at dusk, 0 deep at night
    lightgrid: vec4<f32>,    // xyz Studio's light grid's first voxel, w studs out along the normal (0 = none yet)
    atmos: vec4<f32>,        // Atmosphere: rgb haze band colour, w band strength at the horizon (0 = none)
    env_spec: vec4<f32>,     // x EnvironmentSpecularScale (0 when the Technology has no sky reflection), y roughness
    lightocc: vec4<f32>,     // xyz the light occupancy grid's first voxel, w studs per voxel (0 = no grid)
    atmos_cube: array<vec4<f32>, 6>, // Atmosphere haze toward +X, -X, +Y, -Y, +Z, -Z (irradiance x FogColor)
    atmos_sky: vec4<f32>,    // x fogOffset.x, y fogOffset.y, z Glare, w Offset
    atmos_glare: vec4<f32>,  // xyz glare exponents (from Decay)
    env_indoor: array<vec4<f32>, 6>, // AmbientCube[6..11]: indoor environment irradiance x EnvironmentDiffuseScale
    sky_grad: array<vec4<f32>, 2>,   // SkyGradientTop (c0^2, w EnvDiffuse), SkyGradientBottom (c1^2, w EnvSpecular)
    lamp1: vec4<f32>,        // Lamp1: the back light (faces turned away from the sun / moon), rgb
    ssao: vec4<f32>,         // half projection diagonal, near plane, SSAO on
    dof: vec4<f32>,
};

struct Light {
    pos_range: vec4<f32>,    // position, Range
    color: vec4<f32>,        // rgb colour^2 x Brightness, w kind (0 point, 1 spot, 2 surface)
    dir: vec4<f32>,          // spot / surface direction, w cos(half angle)
    extra: vec4<f32>,        // surface: half width / height of the face, z 1 = Shadows (parts block it), w unused
    surface_u: vec4<f32>,
    surface_v: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> lights: array<Light>;
@group(0) @binding(2) var sky_tex: texture_cube<f32>;
@group(0) @binding(3) var sky_sampler: sampler;
// The sun's shadow cascades (sun_shadow.rs): EVSM moments, four tiles.
@group(0) @binding(4) var sun_evsm_tex: texture_2d<f32>;
@group(0) @binding(11) var lightgrid_tex: texture_3d<f32>;
// Local light shadows (Studio's Future path, Studio measurements "Local light
// shadows"): EVSM moments per cube face, packed in one atlas.
struct ShadowSlot {
    vp: array<mat4x4<f32>, 6>, // per face (+X, −X, +Y, −Y, +Z, −Z): view-projection
    pos: vec4<f32>,            // light position, w 1 / Range
};
@group(0) @binding(15) var lshadow_tex: texture_2d<f32>;
@group(0) @binding(16) var lshadow_sampler: sampler;
@group(0) @binding(17) var<storage, read> lshadow_slots: array<ShadowSlot>;
@group(0) @binding(20) var localgrid_tex: texture_3d<f32>;
@group(0) @binding(12) var lightocc_tex: texture_3d<f32>;
@group(0) @binding(13) var indoor_tex: texture_cube<f32>;
@group(0) @binding(14) var brdf_lut: texture_2d<f32>;

@group(2) @binding(0) var glass_scene: texture_2d<f32>;
@group(2) @binding(1) var glass_depth: texture_2d<f32>;

fn glass_scene_at(world: vec3<f32>, normal: vec3<f32>, alpha: f32) -> vec4<f32> {
    let delta = g.cam_pos.xyz - world;
    let eta = mix(0.66, 1.0, (1.0 - alpha) * (1.0 - alpha));
    let projected = g.view_proj * vec4<f32>(world, 1.0);
    let displaced = g.view_proj * vec4<f32>(g.cam_pos.xyz -
        (delta * (1.0 + 3.0 * eta / max(dot(delta, normal), 0.01)) + normal * (3.0 * (1.0 - eta))), 1.0);
    let uv = projected.xy / projected.w * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let refracted = displaced.xy / displaced.w * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let edge = refracted - vec2<f32>(0.5);
    let offset = (refracted - uv) * clamp(vec2<f32>(1.0) - 4.0 * edge * edge, vec2<f32>(0.0), vec2<f32>(1.0));
    let size = vec2<f32>(textureDimensions(glass_scene));
    let bias = offset / max(length(offset), 1e-8) / size;
    var behind = 1e9;
    for (var side = -1; side <= 1; side += 2) {
        let test_uv = clamp(refracted + f32(side) * bias, vec2<f32>(0.0), vec2<f32>(1.0));
        let pixel = clamp(vec2<i32>(test_uv * size), vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
        // Reversed-Z depth -> clip w (Studio's depth copy stores viewZ / 500).
        let z = textureLoad(glass_depth, pixel, 0).r;
        let pos = g.inv_view_proj * vec4<f32>(test_uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), z, 1.0);
        let clip = g.view_proj * vec4<f32>(pos.xyz / pos.w + g.cam_pos.xyz, 1.0);
        behind = min(behind, select(clip.w, 1e9, z == 0.0));
    }
    return textureSampleLevel(glass_scene, sky_sampler, uv + offset * clamp(behind - projected.w, 0.0, 1.0), 0.0);
}

fn sq(c: vec3<f32>) -> vec3<f32> {
    return c * c;
}

// DefaultUnifiedNeonFS.0..3: 4 * pow(1.35 * encoded Color3, 4).
// Instance colours are already squared, so square again in linear space.
const NEON_GLOW: f32 = 13.286025; // 4 * 1.35^4
// See-through Neon: NEON_SEE_THROUGH_CUT / _MAX come from mech.rs (round 26).
// Terrain water: the share of the sun its colour takes (all of it, shadowed,
// as Studio's LQ water: CB0[15] × skylight shadow), and
// how opaque its surface is seen from below (Studio draws it as the lit
// colour; estimated from an offline Studio's underwater shot).
const WATER_SUN: f32 = 1.0;
const WATER_BELOW_ALPHA: f32 = 0.9;
// With the camera under water, Studio draws the surface over it (terrain
// Place A) but none of the water's walls to air (Place B F4, round 25): only
// faces whose normal is at least this upright are drawn then (fog.z 2 is
// the underwater fog).
const WATER_INSIDE_MIN_UP: f32 = 0.5;
// Local lights (fitted to Studio, RMS ~3 levels): full strength out to
// LIGHT_FADE_START x Range, then x = d - 0.7 Range fades as
// 110 / (110 + x^2) times a smoothstep window to 0 at Range.
const LIGHT_FADE_START: f32 = 0.71; // Studio light record: inner = 0.71 × Range (mech::LIGHT_INNER)
const LIGHT_FALLOFF_K: f32 = 110.0;

// Glass reflection (fitted to Studio's Glass cubes, day and unlit).

/// ImageProcessFXCompositionLinearFS consumes linear scene values and emits
/// the rational curve directly. Its coefficients already map to display colour;
/// applying a second square root lifted highlights and flattened contrast.
fn tonemap(x: vec3<f32>) -> vec3<f32> {
    let y = max(x, vec3<f32>(0.0));
    if (g.params.z >= 0.5) { return sqrt(clamp(y,vec3<f32>(0.0),vec3<f32>(1.0))); }
    let mapped = y * (0.3 * y + vec3<f32>(0.4)) / (y * (0.3 * y + vec3<f32>(0.38)) + vec3<f32>(0.12));
    return clamp(mapped,vec3<f32>(0.0),vec3<f32>(1.0));
}

/// Split-sum environment BRDF terms (A, B) for a roughness and N·V: the
/// analytic fit to the usual lookup table (Karis 2014, mobile). Studio
/// reads PrecomputedBRDFTexture at (roughness, N·V).
fn env_brdf(rough: f32, nv: f32) -> vec2<f32> {
    let q = rough * vec4<f32>(-1.0, -0.0275, -0.572, 0.022) + vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let a004 = min(q.x * q.x, exp2(-9.28 * nv)) * q.x + q.y;
    return vec2<f32>(-1.04, 1.04) * a004 + q.zw;
}

/// Distance falloff of a local light at distance `d` with range `r`.
fn light_falloff(d: f32, r: f32) -> f32 {
    let start = LIGHT_FADE_START * r;
    let x = clamp(d, start, r) - start;
    let t = x / max(r - start, 1e-4);
    let window = 2.0 * t * t * t - 3.0 * t * t + 1.0;
    return window * LIGHT_FALLOFF_K / (LIGHT_FALLOFF_K + x * x);
}

// Atmosphere: Studio's AdvSkyFS. The
// sky shows through the haze by t = exp2(fogOffset.x / (0.001 +
// max(dir.y, 1e-5)^fogOffset.y)); the haze is the skybox's irradiance cube
// times FogColor, plus the sun's glare; sky = mix(haze, skybox, t).

/// How much of the skybox shows through the haze in this direction (1 = no Atmosphere).
fn atmos_t(dir: vec3<f32>) -> f32 {
    if (g.atmos.w < 0.5) {
        return 1.0;
    }
    let y = max(dir.y, 1e-5);
    return clamp(exp2(g.atmos_sky.x / (ATMOS_T_EPS + pow(y, g.atmos_sky.y))), 0.0, 1.0);
}

/// The haze in this direction: the irradiance cube weighted by dir² (AdvSkyFS).
fn atmos_haze(dir: vec3<f32>) -> vec3<f32> {
    let p = clamp(dir, vec3<f32>(0.0), vec3<f32>(1.0));
    let n = clamp(-dir, vec3<f32>(0.0), vec3<f32>(1.0));
    let p2 = p * p;
    let n2 = n * n;
    return g.atmos_cube[0].rgb * p2.x + g.atmos_cube[1].rgb * n2.x
        + g.atmos_cube[2].rgb * p2.y + g.atmos_cube[3].rgb * n2.y
        + g.atmos_cube[4].rgb * p2.z + g.atmos_cube[5].rgb * n2.z;
}

fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    return sky_color_lod(dir, 0.0);
}

/// The sky as a surface of this roughness reflects it: Studio reads its
/// prefiltered cube at mip roughness × ENV_SPEC_LOD_SPAN; here that span
/// ends at the skybox's coarsest mip.
fn sky_rough(dir: vec3<f32>, rough: f32) -> vec3<f32> {
    let top = f32(textureNumLevels(sky_tex)) - 1.0;
    return sky_color_lod(dir, max(top - ENV_SPEC_LOD_SPAN * (1.0 - rough), 0.0));
}

fn sky_color_lod(dir: vec3<f32>, lod: f32) -> vec3<f32> {
    var c: vec3<f32>;
    if (g.params.w > 0.5) {
        // Skybox faces (Sky or Studio's default), dimmed at night.
        let t = textureSampleLevel(sky_tex, sky_sampler, dir, lod).rgb;
        c = sq(t) * select(vec3<f32>(1.0), g.sky_tint.rgb, g.sky_tint.w > 0.5);
    } else {
        let h = clamp(dir.y, -1.0, 1.0);
        c = mix(g.sky_horizon.rgb, g.sky_top.rgb, pow(clamp(h, 0.0, 1.0), 0.55));
        if (h < 0.0) {
            c = mix(g.sky_horizon.rgb, g.sky_horizon.rgb * 0.55, clamp(-h * 3.0, 0.0, 1.0));
        }
        c = c * g.sky_tint.rgb;
    }
    if (g.atmos.w < 0.5) {
        return c;
    }
    let t = atmos_t(dir);
    // The sun's glare: Lamp0Color x clamp((0.5 + 0.5 dot(dir, toward sun))^exp x (1 - t)) x Glare.
    let gl = clamp(pow(vec3<f32>(0.5 + 0.5 * dot(dir, normalize(g.sun_dir.xyz))), g.atmos_glare.xyz) * (1.0 - t),
        vec3<f32>(0.0), vec3<f32>(1.0)) * g.atmos_sky.z;
    let haze = atmos_haze(dir) + g.sun_color.rgb * g.sun_dir.w * gl;
    return mix(haze, c, t);
}

// ------------------------------------------------------------------ sky

struct SkyOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var o: SkyOut;
    o.pos = vec4<f32>(x, y, 1.0, 1.0);
    o.ndc = vec2<f32>(x, y);
    return o;
}

/// The camera is in terrain water (fog.z 2; 3 is Atmosphere).
fn under_water() -> bool {
    return g.fog.z > 1.5 && g.fog.z < 2.5;
}

/// How fogged a point `d` studs away is: Lighting's linear fog, or under
/// terrain water (fog.z 2) Studio's exp2 curve, 2^(a − k·d) − 1 visible.
fn fog_amount(d: f32) -> f32 {
    if (g.fog.z > 2.5) {
        // Atmosphere: e^(−k·d), kept in exp2 form.
        return 1.0 - clamp(exp2(g.fog.x - g.fog.y * d), 0.0, 1.0);
    }
    if (g.fog.z > 1.5) {
        return 1.0 - clamp(exp2(g.fog.x - g.fog.y * d) - 1.0, 0.0, 1.0);
    }
    return clamp((d - g.fog.x) / max(g.fog.y - g.fog.x, 1e-3), 0.0, 1.0);
}

/// The fog's colour; under water the WaterColor lit by ambient and sun.
fn fog_rgb() -> vec3<f32> {
    if (under_water()) {
        return g.fog_color.rgb * (g.ambient.rgb + g.outdoor.rgb + g.sun_color.rgb * g.sun_dir.w * max(g.sun_dir.y, 0.0));
    }
    return g.fog_color.rgb;
}

/// The fog's colour for a point: with an Atmosphere, the sky behind it
/// (Studio's FS samples its sky cube along the view ray when FogParams.w
/// is 0); otherwise fog_rgb.
fn fog_rgb_at(world: vec3<f32>) -> vec3<f32> {
    if (g.fog.z > 2.5) {
        // Studio's part FS: the prefiltered sky cube along the view ray at
        // mip max(Offset, visibility) x 5; rustudio's skybox has more mips
        // than that cube, so its top levels are skipped.
        let rel = world - g.cam_pos.xyz;
        let vis = 1.0 - fog_amount(length(rel));
        let top = f32(textureNumLevels(sky_tex)) - 1.0;
        let lod = max(top - (ATMOS_ENV_CUBE_LEVELS - 1.0), 0.0) + max(g.atmos_sky.w, vis) * ATMOS_FOG_MIP;
        return sky_color_lod(normalize(rel), min(lod, top));
    }
    return fog_rgb();
}

fn hash3(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    // inv_view_proj is camera-relative, so the far point is the direction
    // (subtracting a far-out cam_pos here made the sky wobble as it moved).
    let far = g.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w);
    var c = sky_color(dir);
    if (under_water()) {
        return vec4<f32>(fog_rgb(), 1.0);
    }
    // Stars on a night sky (Sky.StarCount > 0): each its own brightness,
    // most faint (round 26: Studio's stars read 28 on screen at the median,
    // a few up to ~190; they were all 200 here), under the haze band.
    if (g.sky_rot.x > 0.5 && dir.y > 0.0) {
        let cell = floor(dir * 300.0);
        let s = step(0.9985, hash3(cell)) * pow(hash3(cell + vec3<f32>(7.31, 1.7, 3.9)), 4.0);
        c += vec3<f32>(s * 0.6) * (1.0 - g.fog.w) * atmos_t(dir);
    }
    return vec4<f32>(c, 1.0);
}

// ---------------------------------------------------------------- parts

struct PartIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) r0: vec4<f32>,   // right axis, pos.x
    @location(3) r1: vec4<f32>,   // up axis, pos.y
    @location(4) r2: vec4<f32>,   // back axis, pos.z
    @location(5) size: vec4<f32>, // size, w depth bias
    @location(6) color: vec4<f32>,
    @location(7) extra: vec4<f32>, // x material (1 neon, 2 smooth, 3 glass, 4 metal), y placeholder, z Reflectance (+ 2 hovered), w texture mode
    @location(8) uv: vec2<f32>,
    @location(12) projection: vec4<f32>,
    @location(9) uvt: vec4<f32>,   // uv scale (xy) and offset (zw)
    @location(10) emissive: vec4<f32>, // SurfaceAppearance EmissiveTint^2 x EmissiveStrength
    @location(11) vcol: vec4<f32>,     // per-vertex colour (sRGB; a 0 = the part's Color): union child colours
};

struct PartOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) extra: vec4<f32>,
    @location(4) local: vec3<f32>,
    @location(5) uv: vec2<f32>,
    @location(6) emissive: vec3<f32>,
    @location(7) emissive_albedo: f32,
    @location(8) half: vec3<f32>,
    @location(9) objn: vec3<f32>,
    // Material map scale per stud (uvt.xy).
    @location(10) mscale: vec4<f32>,
};

@group(0) @binding(7) var studs_tex: texture_2d<f32>;
@group(0) @binding(8) var studs_sampler: sampler;
// Material texture packs (matpack.rs), one layer per pack: colour + height,
// and normal X/Y + roughness + metalness.
@group(0) @binding(9) var mat_albedo: texture_2d_array<f32>;
@group(0) @binding(10) var mat_surface: texture_2d_array<f32>;
@group(0) @binding(19) var mat_emission: texture_2d_array<f32>;
// Studio's PartMaterialConsts per material layer (render.rs MaterialArrays,
// mech.rs PART_MATERIAL_FULL_*): five rows per layer; see render.rs.
// Then the 32² detiling noise and packed 128² Wang indices.
const MAT_PARAM_LAYERS: u32 = 256u;
@group(0) @binding(18) var<uniform> mat_params: array<vec4<f32>, 2560>;

/// The detiling noise at `uv`, bilinear and wrapping like Studio's sampler.
fn detile_noise(uv: vec2<f32>) -> f32 {
    let p = uv * 32.0 - vec2<f32>(0.5);
    let f = fract(p);
    let i = vec2<i32>(floor(p));
    var v: array<f32, 4>;
    for (var k = 0; k < 4; k++) {
        let c = (i + vec2<i32>(k & 1, k >> 1)) & vec2<i32>(31);
        let n = u32(c.y * 32 + c.x);
        v[k] = mat_params[5u * MAT_PARAM_LAYERS + n / 4u][n % 4u];
    }
    return mix(mix(v[0], v[1], f.x), mix(v[2], v[3], f.x), f.y);
}

// Point/wrap Wang index, DDS channels L and A exposed as RG by Studio.
fn wang_uv(uv: vec2<f32>) -> vec2<f32> {
    let tile = uv * WANG_CELLS;
    let cell = vec2<i32>(floor(tile)) & vec2<i32>(127);
    let n = u32(cell.y * 128 + cell.x);
    let packed = u32(mat_params[5u * MAT_PARAM_LAYERS + 256u + n / 16u][(n / 4u) % 4u]);
    let index = (packed >> ((n % 4u) * 4u)) & 15u;
    return (vec2<f32>(f32(index & 3u), f32(index >> 2u)) + fract(tile)) / WANG_CELLS;
}

/// A normal map texel (Roblox maps: X right, Y up the image) as (x, y, z).
fn mat_normal(t: vec4<f32>) -> vec3<f32> {
    let xy = t.rg * 2.0 - vec2<f32>(1.0);
    return vec3<f32>(xy, sqrt(clamp(1.0 - dot(xy, xy), 0.0, 1.0)));
}

@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;
@group(1) @binding(2) var emissive_tex: texture_2d<f32>;
@group(1) @binding(3) var surface_tex: texture_2d<f32>;

fn part_world(v: PartIn) -> vec3<f32> {
    let l = v.pos * v.size.xyz;
    let origin = vec3<f32>(v.r0.w, v.r1.w, v.r2.w);
    return v.r0.xyz * l.x + v.r1.xyz * l.y + v.r2.xyz * l.z + origin;
}

fn part_out(v: PartIn) -> PartOut {
    let world = part_world(v);
    let s = max(v.size.xyz, vec3<f32>(1e-4));
    let n = v.r0.xyz * (v.normal.x / s.x) + v.r1.xyz * (v.normal.y / s.y) + v.r2.xyz * (v.normal.z / s.z);
    var o: PartOut;
    o.clip = g.view_proj * vec4<f32>(world, 1.0);
    // Reverse-Z: a tiny stable nudge per part (size.w) so coplanar faces of
    // two parts don't flicker.
    o.clip.z = o.clip.z * (1.0 + v.size.w);
    o.world = world;
    o.normal = normalize(n);
    o.color = v.color;
    if (v.vcol.a > 0.5) {
        o.color = vec4<f32>(sq(v.vcol.rgb), v.color.a);
    }
    o.extra = v.extra;
    o.local = v.pos * v.size.xyz;
    o.uv = v.uv * v.uvt.xy + v.uvt.zw;
    if (v.uvt.w < 0.0) {
        // CSGMDL plane IDs: Studio. Tag zero preserves UVs.
        // MeshParts without TextureID share the normal-selected stud projection below.
        // Loaded positions are centered and divided by decoded extent.
        // o.local restores studs (normalized position * current Size).
        // Never substitute stored UVs merely because Size ~= extent:
        // their authored scale need not be one UV unit per stud.
        let p = o.local;
        let modern = v.projection.w >= 9.0;
        let tag = u32(v.projection.w) - select(0u, 8u, modern);
        var uv = v.uv;
        if (tag == 1u) { uv = vec2<f32>(-p.z, -p.y); }
        else if (tag == 2u) { uv = vec2<f32>(-p.x, -p.z); }
        else if (tag == 3u) { uv = vec2<f32>(p.x, -p.y); }
        else if (tag == 4u) { uv = vec2<f32>(p.z, -p.y); }
        else if (tag == 5u) { uv = vec2<f32>(p.x, -p.z); }
        else if (tag == 6u) { uv = vec2<f32>(-p.x, -p.y); }
        if (v.uvt.w < -1.5 || all(v.projection.xyz == vec3<f32>(0.0))) {
            // Uploaded mesh normals include the normalization extent.
            let source_normal = select(v.normal, v.normal / max(v.projection.xyz, vec3<f32>(1e-4)), v.uvt.w < -1.5);
            let a = abs(source_normal);
            uv = vec2<f32>(p.x, -p.y);
            if (a.x >= a.y && a.x >= a.z) {
                uv = vec2<f32>(select(p.z, -p.z, v.normal.x > 0.0), -p.y);
            } else if (a.y >= a.z) {
                uv = vec2<f32>(select(p.x, -p.x, v.normal.y > 0.0), -p.z);
            } else if (v.normal.z < 0.0) { uv = vec2<f32>(-p.x, -p.y); }
        }
        o.uv = uv * v.uvt.xy;
    }

    o.emissive = v.emissive.rgb;
    o.emissive_albedo = v.emissive.w;
    o.half = v.size.xyz * 0.5;
    o.objn = v.normal;
    o.mscale = v.uvt;
    return o;
}

@vertex
fn vs_part(v: PartIn) -> PartOut {
    return part_out(v);
}

/// Sun shadow: 1 lit, 0 shadowed. As Studio's part FS
/// (DefaultUnifiedFlatOpaqueFS 8): the cascades' visibility, handed to the voxel sun over
/// SUN_DIST .. SUN_DIST + 1 / HYBRID_LERP_SLOPE studs from the camera.
fn sun_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.bloom.z < 0.5) { return 1.0; }
    let voxel = light_grid_sun(world, n);
    if (g.sun_color.w > 0.5 && g.env_spec.z <= 0.5) { return voxel; }
    let mapped = sun_shadow_map(world);
    if (mapped < 0.0) { return voxel; }
    if (g.sun_color.w < 0.5) {
        return mix(mapped, voxel, clamp((length(world - g.cam_pos.xyz) - SUN_DIST) * HYBRID_LERP_SLOPE, 0.0, 1.0));
    }
    return mapped * voxel;
}

/// The cascades alone (sun_shadow.rs): the first whose sphere holds the
/// point, else the last; its EVSM moments at the point. −1 outside its tile.
fn sun_shadow_map(world: vec3<f32>) -> f32 {
    let d0 = world - g.sun_sphere[0].xyz;
    let d1 = world - g.sun_sphere[1].xyz;
    let d2 = world - g.sun_sphere[2].xyz;
    var i = 3;
    if (dot(d0, d0) < g.sun_sphere[0].w) {
        i = 0;
    } else if (dot(d1, d1) < g.sun_sphere[1].w) {
        i = 1;
    } else if (dot(d2, d2) < g.sun_sphere[2].w) {
        i = 2;
    }
    let p = g.sun_casc[i] * vec4<f32>(world, 1.0);
    if (any(p.xy <= vec2<f32>(0.0)) || any(p.xy >= vec2<f32>(1.0))) {
        return -1.0;
    }
    let r = g.sun_rect[i];
    let m = textureSampleLevel(sun_evsm_tex, lshadow_sampler, r.xy + p.xy * r.zw, 0.0);
    return evsm_visibility(m, 2.0 * p.z - 1.0);
}

/// Studio's EVSM test (the sun's cascades and the local lights' cube faces
/// alike): Chebyshev on the positive and the negative warp of depth `z`
/// (−1..1) with the bias as minimum variance, light-bleed cut
/// (p − 0.2) × 1.25, the lesser of the two.
fn evsm_visibility(m: vec4<f32>, z: f32) -> f32 {
    let ep = exp(EVSM_POS_EXP * z);
    let en = -exp(-EVSM_NEG_EXP * z);
    let mv = vec2<f32>(EVSM_POS_EXP, EVSM_NEG_EXP) * EVSM_SHADOW_BIAS * vec2<f32>(ep, en);
    let mv2 = mv * mv;
    let vp = max(m.y - m.x * m.x, mv2.x);
    let dp = ep - m.x;
    let vn = max(m.w - m.z * m.z, mv2.y);
    let dn = en - m.z;
    let pp = select(clamp((vp / (vp + dp * dp) - EVSM_BLEED_CUT) / (1.0 - EVSM_BLEED_CUT), 0.0, 1.0), 1.0, ep <= m.x);
    let pn = select(clamp((vn / (vn + dn * dn) - EVSM_BLEED_CUT) / (1.0 - EVSM_BLEED_CUT), 0.0, 1.0), 1.0, en <= m.z);
    return min(pp, pn);
}

/// Studio's light grid skylight (lightgrid.rs; Studio measurements "Light grid
/// (skylight)"): LightGridSkylight .x, read trilinearly at the point moved
/// LIGHTGRID_NORMAL_OFFSET studs out along `n` (DefaultUnified*VS,
/// SmoothClusterVS: pos + normal × 6). As Studio's FS, open sky (1) beyond
/// LIGHTGRID_BORDER of the grid's size from its centre on any axis (CB0
/// LightConfig2 / 3). Open air inside the grid reads 252 / 255. The texture
/// is (X, Z, Y).
fn light_grid(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.lightgrid.w <= 0.0) {
        return 1.0;
    }
    let p = (world + n * g.lightgrid.w) * 0.25 - g.lightgrid.xyz;
    let t = vec3<f32>(p.x, p.z, p.y) / vec3<f32>(textureDimensions(lightgrid_tex));
    if (any(abs(t - vec3<f32>(0.5)) >= vec3<f32>(LIGHTGRID_BORDER))) {
        return 1.0;
    }
    return textureSampleLevel(lightgrid_tex, sky_sampler, t, 0.0).r;
}

/// The light grid's sun channel (.y), read as the skylight; 1 outside it.
fn light_grid_sun(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.lightgrid.w <= 0.0) {
        return 1.0;
    }
    let p = (world + n * g.lightgrid.w) * 0.25 - g.lightgrid.xyz;
    let t = vec3<f32>(p.x, p.z, p.y) / vec3<f32>(textureDimensions(lightgrid_tex));
    if (any(abs(t - vec3<f32>(0.5)) >= vec3<f32>(LIGHTGRID_BORDER))) {
        return 1.0;
    }
    return textureSampleLevel(lightgrid_tex, sky_sampler, t, 0.0).g;
}

/// Environment diffuse light (EnvironmentDiffuseScale): Studio's two ambient
/// cubes, AmbientCube[0..5] (the sky's irradiance x the time-of-day sky
/// level) blended with AmbientCube[6..11] (the indoor environment) by the
/// skylight, each weighted by the normal's positive / negative parts squared
/// (DefaultUnified*FS HQ, Studio measurements "Environment light and AO").
fn env_cube(n: vec3<f32>, sky: f32) -> vec3<f32> {
    let w = n * n;
    let x = select(1, 0, n.x > 0.0);
    let y = select(3, 2, n.y > 0.0);
    let z = select(5, 4, n.z > 0.0);
    let outside = g.env[x].rgb * w.x + g.env[y].rgb * w.y + g.env[z].rgb * w.z;
    let inside = g.env_indoor[x].rgb * w.x + g.env_indoor[y].rgb * w.y + g.env_indoor[z].rgb * w.z;
    return mix(inside, outside, sky);
}

/// Studio's PrecomputedBRDFTexture (brdfLUT.dds) at (roughness, N·V): the
/// split-sum (A, B).
fn brdf_ab(rough: f32, nv: f32) -> vec2<f32> {
    return textureSampleLevel(brdf_lut, sky_sampler, vec2<f32>(rough, max(nv, 1e-4)), 0.0).rg;
}

/// The prefiltered environment a part reflects along `r` at roughness
/// `rough` (mip roughness x 5 of Studio's cube; rustudio's cubes have more
/// mips, so their top levels are skipped): indoors the indoor cube, outdoors
/// the sky tinted by SkyGradientBottom..Top by the ray's height, blended by
/// the skylight (HQ part FS).
fn env_reflect(r: vec3<f32>, rough: f32, sky: f32, sharp: bool) -> vec3<f32> {
    let m = select(rough * ENV_SPEC_MIP_PER_ROUGHNESS, 0.0, sharp);
    let top_s = f32(textureNumLevels(sky_tex)) - 1.0;
    let top_i = f32(textureNumLevels(indoor_tex)) - 1.0;
    let lod_s = min(max(top_s - (ATMOS_ENV_CUBE_LEVELS - 1.0), 0.0) + m, top_s);
    let lod_i = min(max(top_i - (ATMOS_ENV_CUBE_LEVELS - 1.0), 0.0) + m, top_i);
    let tint = mix(g.sky_grad[1].rgb, g.sky_grad[0].rgb, clamp(r.y * ENV_REFLECT_UP_GAIN, 0.0, 1.0));
    let outside = sky_color_lod(r, lod_s) * tint;
    let inside = sq(textureSampleLevel(indoor_tex, sky_sampler, r, lod_i).rgb);
    return mix(inside, outside, sky);
}

fn local_light(l: Light, world: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let to = l.pos_range.xyz - world;
    let d = length(to);
    let range = l.pos_range.w;
    if (d >= range || range <= 0.0) {
        return vec3<f32>(0.0);
    }
    let dir = to / max(d, 1e-4);
    var k = light_falloff(d, range);
    let kind = l.color.w;
    var ndl = max(dot(n, dir), 0.0);
    if (kind > 0.5) {
        // Spot / surface: inside the cone (Angle is the full cone), fading
        // as the square of the way from its edge to its axis.
        let c = dot(-dir, l.dir.xyz);
        let cone = clamp((c - l.dir.w) / max(1.0 - l.dir.w, 1e-3), 0.0, 1.0);
        k *= cone * cone;
        if (kind > 1.5) {
            // Surface lights shine from the whole face: wrapped by its size.
            k *= step(0.0, c);
            ndl = clamp(dot(n, dir) + sqrt(l.extra.x * l.extra.y) / max(d, 1e-3), 0.0, 1.0);
        }
    }
    return l.color.rgb * k * ndl;
}

/// How much of its voxel in the parts' grid (lightocc.rs) is filled; 0
/// outside it. Nearest, not trilinear: a blurred wall would also block the
/// way along it.
fn occ_at(p: vec3<f32>) -> f32 {
    let v = vec3<i32>(floor(p / g.lightocc.w - g.lightocc.xyz));
    if (any(v < vec3<i32>(0)) || any(v >= vec3<i32>(textureDimensions(lightocc_tex)))) {
        return 0.0;
    }
    return textureLoad(lightocc_tex, v, 0).r;
}

/// The share of light let through on the way from a to b: one sample per
/// voxel (at most LIGHTOCC_MAX_STEPS), exp(-density × studs of solid).
fn occ_transmit(a: vec3<f32>, b: vec3<f32>) -> f32 {
    let d = b - a;
    let len = length(d);
    if (len < 1e-3) {
        return 1.0;
    }
    let steps = clamp(ceil(len / g.lightocc.w), 1.0, LIGHTOCC_MAX_STEPS);
    var tau = 0.0;
    for (var i = 0.0; i < steps; i += 1.0) {
        tau += occ_at(a + d * ((i + 0.5) / steps));
    }
    return exp(-tau * (len / steps) * LIGHTOCC_DENSITY);
}

/// A shadowed local light (Shadows on) blocked by the parts between it and
/// a surface (round 26, docs/MECHANICS.md): marched from a voxel out along the
/// normal (past the surface's own part) to half a voxel short of the light.
fn light_occlusion(world: vec3<f32>, n: vec3<f32>, lpos: vec3<f32>) -> f32 {
    let v = g.lightocc.w;
    if (v <= 0.0) {
        return 1.0;
    }
    let start = world + n * (v * LIGHTOCC_START_VOXELS);
    let to = lpos - start;
    let d = length(to);
    let skip = v * LIGHTOCC_LIGHT_SKIP_VOXELS;
    if (d <= skip) {
        return 1.0;
    }
    return occ_transmit(start, lpos - to / d * skip);
}

/// Light from the local lights on a surface, the shadowed ones blocked by parts.
fn local_lights(world: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if (g.sun_color.w > 0.5) {
        if (g.lightgrid.w <= 0.0) { return vec3<f32>(0.0); }
        let p = (world + n * 6.0) * 0.25 - g.lightgrid.xyz;
        let t = p.xzy / vec3<f32>(textureDimensions(localgrid_tex));
        if (any(abs(t - vec3<f32>(0.5)) >= vec3<f32>(LIGHTGRID_BORDER))) { return vec3<f32>(0.0); }
        let v = textureSampleLevel(localgrid_tex, sky_sampler, t, 0.0);
        return v.rgb * (v.a * 120.0);
    }
    var sum = vec3<f32>(0.0);
    let count = u32(g.params.x);
    for (var i = 0u; i < count; i++) {
        let l = lights[i];
        var c = local_light(l, world, n);
        if (!any(c > vec3<f32>(0.0))) {
            continue;
        }
        if (l.extra.w >= 0.0) {
            // Realistic: the light's own cube shadow map.
            c *= light_shadow(i32(l.extra.w), world);
        } else if (l.extra.z > 0.5) {
            c *= light_occlusion(world, n, l.pos_range.xyz);
        }
        sum += c;
    }
    return sum;
}

// DefaultUnifiedTiledFS.8 clustered local-light BRDF: each light has the
// same GGX and Schlick terms as Lamp0, and removes reflected diffuse energy.
struct LocalPbr { diffuse: vec3<f32>, specular: vec3<f32> };
fn local_pbr(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32,
             f0: vec3<f32>, kd: f32, ss: f32) -> LocalPbr {
    var result: LocalPbr;
    result.diffuse = vec3<f32>(0.0);
    result.specular = vec3<f32>(0.0);
    if (g.sun_color.w > 0.5 || g.env_spec.z <= 0.5) {
        result.diffuse = local_lights(world, n);
        return result;
    }
    for (var i = 0u; i < u32(g.params.x); i++) {
        let light = lights[i];
        var radiance = local_light(light, world, n);
        if (!any(radiance > vec3<f32>(0.0))) { continue; }
        if (light.extra.w >= 0.0) { radiance *= light_shadow(i32(light.extra.w), world); }
        else if (light.extra.z > 0.5) { radiance *= light_occlusion(world, n, light.pos_range.xyz); }
        let direction = normalize(light.pos_range.xyz - world);
        let vh = v + direction;
        let h = vh * inverseSqrt(max(dot(vh, vh), 0.0001));
        let nh = max(dot(n, h), 0.001);
        let lh = dot(direction, h);
        let edge = pow(1.0 - lh, 5.0);
        let fresnel = vec3<f32>(edge) + f0 * (1.0 - edge);
        let a = pow(rough, 4.0);
        let denominator = (nh * a - nh) * nh + 1.0;
        let distribution = (a + a*a) / (denominator*denominator * (lh*3.0+0.5) * (nh*0.75+0.25));
        result.diffuse += (vec3<f32>(kd) - fresnel * ss * kd) * radiance;
        result.specular += fresnel * distribution * radiance;
    }
    return result;
}

/// Studio's local light shadow (DefaultUnifiedTiledFS variant 8): the cube
/// face along the light-to-point direction's largest axis, its EVSM moments
/// at the point, Chebyshev on both warps with the bias as minimum variance,
/// light-bleed cut (p − 0.2) × 1.25, the lesser of the two.
fn light_shadow(slot: i32, world: vec3<f32>) -> f32 {
    let s = lshadow_slots[slot];
    let v = world - s.pos.xyz;
    let a = abs(v);
    var f = 0u;
    if (a.x >= a.y && a.x >= a.z) {
        f = select(1u, 0u, v.x > 0.0);
    } else if (a.y >= a.z) {
        f = select(3u, 2u, v.y > 0.0);
    } else {
        f = select(5u, 4u, v.z > 0.0);
    }
    let clip = s.vp[f] * vec4<f32>(world, 1.0);
    let w = max(clip.w, 1e-4);
    let ndc = clamp(clip.xy / w, vec2<f32>(-1.0), vec2<f32>(1.0));
    let k = u32(slot) * 6u + f;
    let cell = vec2<f32>(f32(k % LSHADOW_COLS), f32(k / LSHADOW_COLS));
    let uv = (cell + ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * LSHADOW_CELL;
    let m = textureSampleLevel(lshadow_tex, lshadow_sampler, uv, 0.0);
    return evsm_visibility(m, 2.0 * (w * s.pos.w) - 1.0);
}

// The light shadow pass: one cube face at a time (render.rs), writing EVSM
// moments of the depth along the face axis ÷ Range, warped to [−1, 1].
struct LFace {
    vp: mat4x4<f32>,
    pos: vec4<f32>,  // light position, w 1 / Range
    axis: vec4<f32>, // the face's direction
};
@group(1) @binding(0) var<uniform> lface: LFace;
struct LShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) lin: f32,
};

@vertex
fn vs_light_shadow(v: PartIn) -> LShadowOut {
    var o: LShadowOut;
    let world = part_world(v);
    o.clip = lface.vp * vec4<f32>(world, 1.0);
    o.lin = dot(world - lface.pos.xyz, lface.axis.xyz) * lface.pos.w;
    if (((u32(v.extra.y + 0.5) >> 19u) & 1u) == 1u) {
        o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return o;
}

@fragment
fn fs_light_shadow(i: LShadowOut) -> @location(0) vec4<f32> {
    let z = 2.0 * clamp(i.lin, 0.0, 1.0) - 1.0;
    let ep = exp(EVSM_POS_EXP * z);
    let en = -exp(-EVSM_NEG_EXP * z);
    return vec4<f32>(ep, ep * ep, en, en * en);
}

/// Legacy surfaces (Studs, Weld / Glue, Inlet, Universal) from Studio's
/// studs.dds: four 512 px bands of a 128 px wide atlas, each 2 × 8 studs,
/// counted from the face's corner. Its grey 0.5 leaves the colour as it is;
/// lighter and darker texels (the studs' bevels) scale it, squared as Studio
/// squares its inputs. `packed`: 3 bits per face, Enum.NormalId order.
fn surface_shade(packed: u32, objn: vec3<f32>, local: vec3<f32>, half: vec3<f32>, ddx: vec3<f32>, ddy: vec3<f32>) -> vec3<f32> {
    let a = abs(objn);
    var face = 0u;
    var uv = vec2<f32>(0.0);
    var gx = vec2<f32>(0.0);
    var gy = vec2<f32>(0.0);
    let c = local + half;
    if (a.x >= a.y && a.x >= a.z) {
        face = select(3u, 0u, objn.x > 0.0);
        uv = c.zy;
        gx = ddx.zy;
        gy = ddy.zy;
    } else if (a.y >= a.z) {
        face = select(4u, 1u, objn.y > 0.0);
        uv = c.xz;
        gx = ddx.xz;
        gy = ddy.xz;
    } else {
        face = select(5u, 2u, objn.z > 0.0);
        uv = c.xy;
        gx = ddx.xy;
        gy = ddy.xy;
    }
    let band = (packed >> (3u * face)) & 7u;
    if (band == 0u) {
        return vec3<f32>(1.0);
    }
    // Continuous coordinates for the gradients, the band picked by fract.
    let k = vec2<f32>(0.5, 1.0 / 32.0);
    let at = vec2<f32>(uv.x * 0.5, (f32(band - 1u) + fract(uv.y / 8.0)) * 0.25);
    // Studio's part shader: colour × StudsMapTexture.r × 2, then squared.
    let t = textureSampleGrad(studs_tex, studs_sampler, at, gx * k, gy * k).r;
    return sq(vec3<f32>(t * 2.0));
}

/// A box face's material coordinate in studs from the face's corner
/// (Studio's TEXCOORD0 × 8; v runs down the side faces so maps stand
/// upright), the same faces as `surface_shade`.
fn face_studs(objn: vec3<f32>, local: vec3<f32>, half: vec3<f32>) -> vec2<f32> {
    let a = abs(objn);
    let c = local + half;
    if (a.x >= a.y && a.x >= a.z) {
        return vec2<f32>(c.z, 2.0 * half.y - c.y);
    } else if (a.y >= a.z) {
        return c.xz;
    }
    return vec2<f32>(c.x, 2.0 * half.y - c.y);
}

@fragment
fn fs_part(in: PartOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // extra.x: shading kind (0..7), + 8 mesh-UV mapping, + 16 × (layer + 1).
    let code = u32(in.extra.x + 0.5);
    let kind = f32(code & 7u);
    let layer = i32(code >> 4u) - 1;
    // Modern Realistic lighting uses the HQ material variant even when
    // an old Technology property is absent (env_spec.z is legacy state).
    let material_lq = layer >= 0 && g.env_spec.z <= 0.5 && g.sun_color.w > 0.5;
    // Material map coordinate and its screen gradients (taken here, in
    // uniform control flow): the mesh's UVs, or the box face in studs.
    var muv = face_studs(in.objn, in.local, in.half) * in.mscale.xy + in.mscale.zw;
    if ((code & 8u) != 0u) {
        muv = in.uv;
    }
    let mdx = dpdx(muv);
    let mdy = dpdy(muv);
    let wdx = dpdx(in.world);
    let wdy = dpdy(in.world);
    var n = normalize(in.normal);
    // Two-sided meshes: light the side that faces the camera, whatever the
    // triangle winding (imported meshes wind either way).
    if (dot(n, g.cam_pos.xyz - in.world) < 0.0) { n = -n; }
    var base = in.color.rgb;
    var alpha = in.color.a;
    // Textures: 1 = over the part colour (mesh textures, clothing), 2 =
    // decal (its own alpha, tinted by Color3). Studio squares the raw texel
    // (× Color3) like every colour (DefaultUnifiedFlatOpaqueFS,
    // SurfaceAppearanceFS: (texture × VARYING2)²); the texture is stored
    // sRGB here (shared with GUI pipelines), so the sample is re-encoded to
    // its byte value, then squared. The part colour is squared already.
    let t_s = textureSample(tex, tex_sampler, in.uv);
    let t_raw = select(1.055 * pow(max(t_s.rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), 12.92 * t_s.rgb, t_s.rgb <= vec3<f32>(0.0031308));
    let t = vec4<f32>(t_raw * t_raw, t_s.a);
    let emask = textureSample(emissive_tex, tex_sampler, in.uv).r;
    let mode = u32(in.extra.w + 0.5);
    if (kind == 6.0) {
        // DefaultUnifiedForcefieldFS: the texture encodes pulse phase in
        // red (not albedo), with a minimum 5% rim coverage. No lighting.
        let view = normalize(g.cam_pos.xyz - in.world);
        let rim = 1.0 - clamp(2.0 * abs(dot(n, view)), 0.0, 1.0);
        let phase = abs(cos(0.3 * g.cam_pos.w) * cos(g.cam_pos.w));
        // Texture samplers decode sRGB; the pulse uses the original byte.
        let red = t_raw.r;
        let pulse = select(0.0, (1.0 - clamp(13.0 * abs(red - phase), 0.0, 1.0)) * t.a, mode != 0u && red > 0.0);
        let edge = max(rim, pulse);
        let cover = alpha * max(mix(pulse, 1.0, edge) * edge, 0.05);
        let fog = fog_amount(length(in.world - g.cam_pos.xyz));
        return vec4<f32>(mix(base, fog_rgb_at(in.world), fog) * cover, cover * (1.0 - fog));
    }
    if (mode == 1u) {
        base = mix(base, t.rgb * in.emissive_albedo, t.a);
    } else if (mode == 2u) {
        base = t.rgb * base;
        alpha = alpha * t.a;
    } else if (mode == 3u) {
        base = t.rgb * in.emissive_albedo;
    } else if (mode == 5u) {
        base = t.rgb * in.color.rgb;
    } else if (mode == 4u) {
        base = t.rgb * in.emissive_albedo;
        if (t.a < 0.5) {
            discard;
        }
    }
    // extra.y: placeholder (bit 0) and the six surfaces above it.
    // Material maps (Studio's part shader): colour × part colour, squared;
    // the normal map (OpenGL convention, X right, Y up the image) bends the
    // normal in the face's tangent frame.
    // The material's roughness / metalness (SpecularMap .y / .x in Studio's
    // Tiled FS); -1 = the flat parts' per-material value (env_spec.y).
    var mat_rough = -1.0;
    var mat_metal = 0.0;
    let sa = textureSample(surface_tex, tex_sampler, in.uv);
    let udx = dpdx(in.uv);
    let udy = dpdy(in.uv);
    if ((mode == 1u || mode >= 3u) && textureDimensions(surface_tex).x > 1u) {
        mat_rough = sa.b;
        mat_metal = sa.a;
        let det = udx.x * udy.y - udx.y * udy.x;
        if (abs(det) > 1e-10) {
            let tangent = (wdx * udy.y - wdy * udx.y) / det;
            let bitangent = (wdy * udx.x - wdx * udy.x) / det;
            let tn = mat_normal(sa);
            let tt = normalize(tangent - n * dot(n, tangent));
            let bb = normalize(bitangent - n * dot(n, bitangent));
            n = normalize(tt * tn.x - bb * tn.y + n * tn.z);
        }
    }
    var material_emission = vec3<f32>(0.0);
    var glass_alpha = alpha;
    var glass_scene_alpha = -1.0;
    if (mode == 0u && layer >= 0) {
        // Studio's part shaders (Studio measurements "Part materials").
        var mp0 = vec4<f32>(-1.0, 0.0, 0.0, 1.0);
        var mp1 = vec4<f32>(0.0);
        var mf = vec4<f32>(1.0, 1.0, 1.0, 0.0);
        var mc = vec4<f32>(0.0);
        var ms = vec4<f32>(1.0, 1.0, 1.0, 0.0);
        if (u32(layer) < MAT_PARAM_LAYERS) {
            mp0 = mat_params[5u * u32(layer)];
            mp1 = mat_params[5u * u32(layer) + 1u];
            mf = mat_params[5u * u32(layer) + 2u];
            mc = mat_params[5u * u32(layer) + 3u];
            ms = mat_params[5u * u32(layer) + 4u];
        }
        let shader_type = u32(mp1.z + 0.5);
        let lq = material_lq;
        let depth = 1.0 / in.clip.w;
        let fades = clamp(vec2<f32>(1.0) - depth * vec2<f32>(1.0 / MATERIAL_DIFFUSE_FADE_DEPTH, ENV_SPEC_FADE), vec2<f32>(0.0), vec2<f32>(1.0));
        var a: vec4<f32>;
        var su: vec4<f32>;
        var nm: vec3<f32>;
        var duv = muv;
        var dk = 1.0;
        if (mp1.y > 0.5) {
            // DefaultUnifiedDetiledFS: two copies of the maps, moved by
            // sin((3, 7) × i) for the noise level i and i + 1, mixed by its
            // fraction (the uv's own gradients for both).
            let k = detile_noise(muv * mp1.x) * DETILE_LEVELS;
            let f = fract(k);
            let i = floor(k);
            let ua = muv + sin(vec2<f32>(3.0, 7.0) * i);
            let ub = muv + sin(vec2<f32>(3.0, 7.0) * (i + 1.0));
            a = mix(textureSampleGrad(mat_albedo, studs_sampler, ua, layer, mdx, mdy),
                textureSampleGrad(mat_albedo, studs_sampler, ub, layer, mdx, mdy), f);
            let sa = textureSampleGrad(mat_surface, studs_sampler, ua, layer, mdx, mdy);
            let sb = textureSampleGrad(mat_surface, studs_sampler, ub, layer, mdx, mdy);
            su = mix(sa, sb, f);
            nm = mix(mat_normal(sa), mat_normal(sb), f);
        } else if (shader_type == 6u) {
            // DefaultUnifiedWangFS: atlas coordinates, gradients of the
            // unsplit uv. Detail normal still uses the original tiled uv.
            duv = muv * mp0.w;
            dk = mp0.w;
            let atlas = wang_uv(duv);
            a = textureSampleGrad(mat_albedo, studs_sampler, atlas, layer, mdx * dk, mdy * dk);
            su = textureSampleGrad(mat_surface, studs_sampler, atlas, layer, mdx * dk, mdy * dk);
            nm = mat_normal(su);
        } else {
            // DefaultUnifiedTiledFS: uv × textureTiling.
            duv = muv * mp0.w;
            dk = mp0.w;
            a = textureSampleGrad(mat_albedo, studs_sampler, duv, layer, mdx * dk, mdy * dk);
            su = textureSampleGrad(mat_surface, studs_sampler, duv, layer, mdx * dk, mdy * dk);
            // LQ's diffuse switch uses unscaled VARYING0 and fixed cutoff
            // 0.6. HQ HideDiffuse/HideAll use the material's cutoff.
            if (lq || shader_type == 7u || shader_type == 8u) {
                let cutoff = select(mc.x, MATERIAL_LQ_DIFFUSE_CUTOFF, lq);
                let uv = select(duv, muv, lq);
                let dx = select(mdx * dk, mdx, lq);
                let dy = select(mdy * dk, mdy, lq);
                let far = textureSampleGrad(mat_albedo, studs_sampler, uv * mf.x, layer, dx * mf.x, dy * mf.x);
                let near = textureSampleGrad(mat_albedo, studs_sampler, uv, layer, dx, dy);
                a = mix(far, near, clamp((fades.x - cutoff) * ms.x, 0.0, 1.0));
            }
            if ((!lq && shader_type == 8u) || shader_type == 10u || shader_type == 11u) {
                let sn = textureSampleGrad(mat_surface, studs_sampler, duv * mf.y, layer, mdx * dk * mf.y, mdy * dk * mf.y);
                let ss = textureSampleGrad(mat_surface, studs_sampler, duv * mf.z, layer, mdx * dk * mf.z, mdy * dk * mf.z);
                su = vec4<f32>(mix(sn.rg, su.rg, clamp((fades.y - mc.y) * ms.y, 0.0, 1.0)), mix(ss.ba, su.ba, clamp((fades.y - mc.z) * ms.z, 0.0, 1.0)));
            }
            nm = mat_normal(su);
        }
        mat_rough = su.b;
        mat_metal = su.a;
        if (shader_type == 10u || shader_type == 11u) {
            glass_alpha = clamp(2.0 * alpha - 1.0 + a.a, 0.0, 1.0);
        }
        // Built-in repetition-suppression shaders use diffuse alpha as a
        // Color3 mask, in Studio's pre-square colour space.
        // GlassFS multiplies Color3 directly; diffuse alpha only controls coverage.
        let tint = select(1.0, a.a, mp1.w > 0.5 && shader_type != 10u && shader_type != 11u);
        base = sq(mix(vec3<f32>(1.0), sqrt(max(base, vec3<f32>(0.0))), vec3<f32>(tint)) * a.rgb);
        if (shader_type == 9u) {
            let mask = textureSampleGrad(mat_emission, studs_sampler, duv, layer, mdx * dk, mdy * dk).r;
            material_emission = vec3<f32>(mf.w, mc.w, ms.w) * mask;
        }
        var xy = nm.xy;
        let z = nm.z * select(1.0, 10.0, shader_type == 10u || shader_type == 11u);
        // The detail normal (the material's second pack) at normalDetailTiling
        // × the main repeat: its xy × normalDetailScale added (z kept).
        if (mp0.x >= 0.0 && mp0.z != 0.0) {
            let t = mp0.y * dk;
            let dn = textureSampleGrad(mat_surface, studs_sampler, duv * mp0.y, i32(mp0.x), mdx * t, mdy * t);
            xy += (dn.rg * 2.0 - vec2<f32>(1.0)) * mp0.z;
        }
        // Relief fades out with distance (CB0[28].y, as the reflections).
        xy *= fades.y;
        // Invert the UV derivatives to recover the authored tangent frame.
        // The cross-product cotangent shortcut changes handedness with the
        // screen-space derivatives (WebGPU Y points down), reversing relief.
        // Studio WangVS supplies a rotated object tangent; WangFS uses
        // T * normal.x + cross(N, T) * normal.y + N * normal.z.
        let det = mdx.x * mdy.y - mdx.y * mdy.x;
        if (!lq && abs(det) > 1e-20) {
            let t = (wdx * mdy.y - wdy * mdx.y) / det;
            let b = (wdy * mdx.x - wdx * mdy.x) / det;
            let tt = normalize(t - n * dot(n, t));
            let bb = normalize(b - n * dot(n, b));
            n = normalize(tt * xy.x - bb * xy.y + n * z);
        }
    }
    let packed = u32(in.extra.y + 0.5);
    let ddx = dpdx(in.local);
    let ddy = dpdy(in.local);
    // The six faces' surfaces (18 bits; bit 19 is CastShadow, vs_shadow's).
    let faces = (packed >> 1u) & 0x3FFFFu;
    if (faces != 0u) {
        base *= surface_shade(faces, in.objn, in.local, in.half, ddx, ddy);
    }
    // Mesh placeholder: diagonal stripes so unloaded meshes stand out.
    if ((packed & 1u) == 1u) {
        let k = fract((in.world.x + in.world.y + in.world.z) * 0.25);
        base = mix(base, base * 0.6 + vec3<f32>(0.18, 0.12, 0.3), step(0.5, k) * 0.6);
    }
    var c: vec3<f32>;
    if (kind > 4.5 && kind < 5.5) {
        // Terrain water (round 21, Studio's SmoothWater FS LQ): WaterColor
        // lit by ambient and the shadowed sun (no N·L), mixed with the sky's
        // reflection by (clamp(0.78 − 2.5|N·V|) + 0.3) × WaterReflectance;
        // WaterTransparency is how much of what's behind shows (the HQ
        // shader's depth blend). From below, the surface is the lit colour.
        // emissive.x = WaterReflectance, emissive.y = WaterTransparency.
        let up = normalize(in.normal);
        let below = dot(up, g.cam_pos.xyz - in.world) < 0.0;
        if (under_water() && (!below || up.y < WATER_INSIDE_MIN_UP)) {
            discard;
        }
        let vd = normalize(in.world - g.cam_pos.xyz);
        let shadow = sun_shadow(in.world, up);
        let lit = base * (g.ambient.rgb + g.outdoor.rgb * light_grid(in.world, up)
            + g.sun_color.rgb * g.sun_dir.w * shadow * WATER_SUN);
        let fres = clamp((clamp(0.78 - 2.5 * abs(dot(n, vd)), 0.0, 1.0) + 0.3) * in.emissive.x, 0.0, 1.0);
        if (below) {
            c = lit;
            alpha = WATER_BELOW_ALPHA;
        } else {
            c = mix(lit, sky_color(reflect(vd, n)), fres);
            alpha = clamp(1.0 - in.emissive.y * (1.0 - fres), 0.0, 1.0);
        }
        if (g.fog.z > 0.5) {
            let d = length(in.world - g.cam_pos.xyz);
            let f = fog_amount(d);
            c = mix(c, fog_rgb_at(in.world), f);
        }
        return vec4<f32>(c * alpha, alpha);
    } else if (kind > 0.5 && kind < 1.5) {
        // Neon glows at its own colour, unlit (and blooms).
        c = base * base * NEON_GLOW;
        if (alpha < 0.999) {
            c = clamp(c - vec3<f32>(NEON_SEE_THROUGH_CUT), vec3<f32>(0.0), vec3<f32>(NEON_SEE_THROUGH_MAX));
        }
    } else {
        let l = normalize(g.sun_dir.xyz);
        let ndl = max(dot(n, l), 0.0);
        let shadow = select(1.0, sun_shadow(in.world, normalize(in.normal)), ndl > 0.0);
        // Studio's skylight scales OutdoorAmbient and the sky's diffuse
        // light (SkyAmbient, AmbientCube[0..5] in the FS): the light grid,
        // parts and terrain alike.
        let sv = light_grid(in.world, normalize(in.normal));
        let is_glass = kind > 2.5 && kind < 3.5;
        // Studio's HQ part FS (DefaultUnifiedFlatOpaqueFS / TiledFS /
        // PlasticReflectionFS variant 3, Studio measurements "Environment light
        // and AO"). s = EnvironmentSpecularScale, fading out with distance;
        // ShadowMap / Future only (env_spec.z).
        let v = normalize(g.cam_pos.xyz - in.world);
        let nv = max(dot(n, v), 1e-4);
        let fade = clamp(1.0 - length(g.cam_pos.xyz - in.world) * ENV_SPEC_FADE, 0.0, 1.0);
        let sS = select(0.0, g.sky_grad[1].w * fade, g.env_spec.z > 0.5);
        let rough = select(g.env_spec.y, ENV_ROUGH_BASE + ENV_ROUGH_SCALE * mat_rough, mat_rough >= 0.0);
        // Reflectance: Studio draws such parts with PlasticReflectionFS: the
        // albedo becomes the sharp sky reflection by Reflectance, no metal.
        let refl = in.extra.z - 2.0 * step(1.5, in.extra.z);
        let reflective = refl > 0.0 && !is_glass;
        let metal = select(mat_metal * sS, 0.0, reflective || is_glass);
        let f0 = mix(vec3<f32>(0.04), base, metal);
        let ab = brdf_ab(rough, nv);
        let fe = (f0 * ab.x + vec3<f32>(ab.y)) / max(ab.x + ab.y, 1e-4);
        let kd = 1.0 - metal;
        // The sun: GGX (a = roughness⁴), Schlick Fresnel at L·H.
        var sun_spec = vec3<f32>(0.0);
        var fs = vec3<f32>(0.0);
        if (g.env_spec.z > 0.5) {
            let h = normalize(v + l);
            let nh = max(dot(n, h), 0.001);
            let lh = dot(l, h);
            let e = pow(1.0 - lh, 5.0);
            fs = vec3<f32>(e) + f0 * (1.0 - e);
            let a = pow(rough, 4.0);
            let d = (nh * a - nh) * nh + 1.0;
            let dterm = (a + a * a) / (d * d * (lh * 3.0 + 0.5) * (nh * 0.75 + 0.25));
            sun_spec = fs * dterm * ndl * shadow * g.sun_color.rgb * g.sun_dir.w;
        }
        let sun_light = g.sun_color.rgb * g.sun_dir.w * ndl * shadow;
        // Lamp1 lights what faces away from Lamp0, through the same shadow
        // term (HQ part FS: CB0[17] × (1 − metal) × clamp(−N·L × shadow)):
        // with the sun's shadow map on, a face turned from the light is in
        // its own object's shadow, so only without it does Lamp1 show.
        let nl = dot(n, l);
        // Voxel / Soft-LQ (DefaultUnifiedFlatOpaqueVS 0: VARYING5 = Lamp0 ×
        // sat(N·L) + Lamp1 × sat(−N·L), all × the grid's sun channel).
        let voxel_sun = g.sun_color.w > 0.5 && g.env_spec.z <= 0.5;
        var back = select(0.0, -nl, nl < 0.0 && g.bloom.z < 0.5);
        if (voxel_sun && nl < 0.0) {
            back = -nl * sun_shadow(in.world, normalize(in.normal));
        }
        // LQ TiledFS.0 uses AmbientColorNoIBL / SkyAmbientNoIBL and
        // geometric (vertex) normals, with no environment BRDF contribution.
        var light = select((kd - fe * sS * kd) * env_cube(n, sv), vec3<f32>(0.0), material_lq)
            + g.ambient.rgb + g.outdoor.rgb * sv
            + (kd - fs * sS * kd) * sun_light
            + g.lamp1.rgb * kd * back;
        let local = local_pbr(in.world, select(n, normalize(in.normal), g.sun_color.w > 0.5), v, rough, f0, kd, sS);
        light += local.diffuse;
        sun_spec += local.specular;
        // RUSTUDIO_SHADE_DEBUG light / local / sun / ambient: that light alone, no albedo.
        if (g.sky_rot.z > 4.5) {
            let dm = g.sky_rot.z;
            var dl = light;
            if (dm > 5.5 && dm < 6.5) { dl = local.diffuse; }
            else if (dm > 6.5 && dm < 7.5) { dl = (kd - fs * sS * kd) * sun_light; }
            else if (dm > 10.5) { dl = vec3<f32>(sv); }
            else if (dm > 9.5) { dl = vec3<f32>(light_grid_sun(in.world, n)); }
            else if (dm > 8.5) { dl = vec3<f32>(max(sun_shadow_map(in.world), 0.0)); }
            else if (dm > 7.5) { dl = light - local.diffuse - (kd - fs * sS * kd) * sun_light; }
            return vec4<f32>(dl * 0.25, 1.0);
        }
        var albedo = base;
        if (reflective) {
            // PlasticReflectionFS.3 _2293: outdoor cube, independent of skylight.
            albedo = mix(base, env_reflect(reflect(-v, n), rough, 1.0, true), refl);
        }
        c = light * albedo;
        if (!is_glass) {
            c += env_reflect(reflect(-v, n), rough, sv, reflective) * fe * sS + sun_spec;
        } else {
            // GlassFS.3: premultiplied base plus tinted outdoor reflection
            // at grazing angles. Environment BRDF and sun are additive;
            // Reflectance mixes the base with that same tinted reflection.
            let a = glass_alpha;
            let reflection = env_reflect(reflect(-v, n), rough, sv, false)
                * mix(vec3<f32>(1.0), base, 0.5);
            let grazing = 0.8 * (1.0 - nv) * (1.0 - nv) * clamp(2.0 * alpha, 0.0, 1.0);
            var scene = vec4<f32>(0.0);
            if (alpha < 0.999) { scene = glass_scene_at(in.world, n, alpha); }
            let tinted = scene.rgb * mix(vec3<f32>(1.0), sqrt(max(in.color.rgb, vec3<f32>(0.0))), clamp(2.0 * alpha, 0.0, 1.0));
            // GlassTransparentFS.8 _1111 includes alpha once here and
            // once in the scene composite below (the shader's intentional a²).
            let lit = mix(light * base * a, reflection, refl);
            c = mix(mix(tinted, lit, a), reflection, grazing)
                + env_reflect(reflect(-v, n), rough, sv, false) * fe * sS * a + sun_spec;
            // GlassTransparentFS.8 _1094/_1128: preserve scene alpha by
            // the ratio of tinted to untinted scene energy (linear HDR here).
            let energy = dot(tinted, vec3<f32>(1.0)) / (dot(scene.rgb, vec3<f32>(1.0)) + 0.01);
            glass_scene_alpha = mix(mix(mix(1.0, scene.a, energy), 1.0, a), 1.0, grazing);
            // Scene is already composited; replace the pixel before applying fog.
            alpha = 1.0;
        }
    }
    c += material_emission * base;
    // SurfaceAppearance emission: ColorMap x EmissiveMask x EmissiveTint x
    // EmissiveStrength, unlit.
    if ((mode == 1u || mode >= 3u) && any(in.emissive > vec3<f32>(0.0))) {
        c += t.rgb * emask * in.emissive;
    }
    // Debug views (RUSTUDIO_SHADE_DEBUG): world normal, or surface colour.
    if (g.sky_rot.z > 0.5 && g.sky_rot.z < 1.5) {
        c = sq(n * 0.5 + vec3<f32>(0.5));
    } else if (g.sky_rot.z > 1.5 && g.sky_rot.z < 2.5) {
        c = base;
    } else if (g.sky_rot.z > 2.5 && g.sky_rot.z < 3.5) {
        c = sq(vec3<f32>(fract(in.uv), 0.0));
    } else if (g.sky_rot.z > 3.5 && g.sky_rot.z < 4.5) {
        c = vec3<f32>(light_grid(in.world, n));
    }
    // Hover highlight.
    c = c + vec3<f32>(0.004, 0.008, 0.014) * step(1.5, in.extra.z);
    // Premultiplied (see-through parts blend One, OneMinusSrcAlpha): see-
    // through Neon adds its glow over what's behind without hiding it, as
    // Studio's does (round 26: the floor under lightwall's panels keeps its
    // red channel exactly).
    let glow_only = kind > 0.5 && kind < 1.5 && alpha < 0.999;
    var cover = alpha;
    if (glow_only) {
        cover = 0.0;
    }
    // Fog (FogStart..FogEnd, FogColor). The glow of see-through Neon fades
    // with it instead (what's behind is fogged already): Studio's Neon FS
    // mixes toward the fog colour by the same amount and covers what's
    // behind by 1 − visibility × alpha (DefaultUnifiedNeonFS).
    if (g.fog.z > 0.5) {
        let d = length(in.world - g.cam_pos.xyz);
        let f = fog_amount(d);
        if (glow_only) {
            c = c * (1.0 - f);
        } else {
            c = mix(c, fog_rgb_at(in.world), f);
        }
    }
    if (glass_scene_alpha >= 0.0) {
        let visibility = select(1.0, 1.0 - fog_amount(length(in.world - g.cam_pos.xyz)), g.fog.z > 0.5);
        return vec4<f32>(c, glass_scene_alpha * visibility);
    }
    return vec4<f32>(c * alpha, cover);
}

// --------------------------------------------------------------- water
// Terrain water after Studio's SmoothWater FS / VS (the HQ variant with
// GBufferColor / GBufferDepth; water.rs): the opaque
// scene shows through, bent by the ripples and fading to the lit
// WaterColor with depth; the sky reflects by the same Fresnel term as the
// LQ shader; GGX sun glint (F0 0.02) off the animated normal maps.

struct WaterParams {
    wave: vec4<f32>,   // x wave number, y phase, z amplitude, w frame blend
    frames: vec4<f32>, // x frame a, y frame b, z depth scale, w depth offset
    view: vec4<f32>,   // xy target size, z wave fade, w near plane
};

@group(2) @binding(0) var<uniform> wp: WaterParams;
@group(2) @binding(1) var water_normals: texture_2d_array<f32>;
@group(2) @binding(2) var water_sampler: sampler;
@group(2) @binding(3) var scene_color: texture_2d<f32>;
@group(2) @binding(4) var scene_depth: texture_depth_multisampled_2d;

@vertex
fn vs_water(v: PartIn) -> PartOut {
    var o = part_out(v);
    let p = o.world;
    // Two crossing sine waves (SmoothWater VS), fading with view depth.
    let fade = clamp(1.0 - length(p - g.cam_pos.xyz) * wp.view.z, 0.0, 1.0);
    let k = wp.wave.x;
    let dy = (sin((p.z - p.x) * k - wp.wave.y) + sin((p.z + p.x) * k + wp.wave.y)) * wp.wave.z * fade;
    o.world = vec3<f32>(p.x, p.y + dy, p.z);
    o.clip = g.view_proj * vec4<f32>(o.world, 1.0);
    return o;
}

/// A view ray's direction through NDC `ndc` (inv_view_proj is the
/// ordinary projection's inverse, camera-relative).
fn ray_dir(ndc: vec2<f32>) -> vec3<f32> {
    let a = g.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    let b = g.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(b.xyz / b.w - a.xyz / a.w);
}

/// The opaque scene's world position behind pixel `px`: the infinite
/// reverse-Z depth is NEAR / view depth (wp.view.w = NEAR).
fn scene_world(px: vec2<f32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(scene_depth));
    let c = clamp(vec2<i32>(px), vec2<i32>(0), size - vec2<i32>(1));
    let d = textureLoad(scene_depth, c, 0);
    let uv = (vec2<f32>(c) + 0.5) / vec2<f32>(size);
    let dir = ray_dir(vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0));
    let cam = g.cam_pos.xyz;
    if (d <= 1e-7) {
        return cam + dir * 1e6;
    }
    let fwd = ray_dir(vec2<f32>(0.0, 0.0));
    return cam + dir * ((wp.view.w / d) / max(dot(dir, fwd), 1e-3));
}

fn scene_rgb(px: vec2<f32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(scene_color));
    let c = clamp(vec2<i32>(px), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(scene_color, c, 0).rgb;
}

// SmoothWaterFSUnified.7: eight bounded screen-space reflection steps.
// Depth and ray travel use clip W (view depth), matching GBufferDepth.
fn water_reflection(world: vec3<f32>, ray: vec3<f32>, env: vec3<f32>) -> vec3<f32> {
    let origin = g.view_proj * vec4<f32>(world, 1.0);
    let direction = g.view_proj * vec4<f32>(ray, 0.0);
    let bound = origin.w * 0.2;
    var travel = 1.0;
    for (var i = 0u; i < 8u; i++) {
        let probe = origin + direction * travel;
        let uv = probe.xy / probe.w * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
        let hit = scene_world(uv * wp.view.xy);
        let depth = min((g.view_proj * vec4<f32>(hit, 1.0)).w, 500.0);
        travel += clamp(depth - probe.w, -bound, bound);
    }
    let probe = origin + direction * travel;
    let uv = probe.xy / probe.w * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let depth = min((g.view_proj * vec4<f32>(scene_world(uv * wp.view.xy), 1.0)).w, 500.0);
    let valid = abs(uv.x - 0.5) < 0.55 && abs(uv.y - 0.5) < 0.5
        && abs(depth - probe.w) < 10.0 && direction.w > 0.0;
    let weight = select(0.0, clamp(3.9 - max(origin.w, probe.w) * 0.008, 0.0, 1.0), valid);
    return mix(env, scene_rgb(uv * wp.view.xy), weight);
}

@fragment
fn fs_water(in: PartOut) -> @location(0) vec4<f32> {
    let cam = g.cam_pos.xyz;
    let px = in.clip.xy;
    let water_d = length(in.world - cam);
    // Its own depth test against the opaque scene (no depth attachment).
    let behind = scene_world(px);
    let scene_d0 = (g.view_proj * vec4<f32>(behind, 1.0)).w;
    let view_d = (g.view_proj * vec4<f32>(in.world, 1.0)).w;
    if (scene_d0 < view_d - 0.05) {
        discard;
    }
    var up = normalize(in.normal);
    let below = dot(up, cam - in.world) < 0.0;
    if (under_water() && (!below || up.y < WATER_INSIDE_MIN_UP)) {
        discard;
    }
    if (below) { up = -up; }
    let v = normalize(cam - in.world);
    // Ripples: the normal maps projected along the face's main axis every
    // 20 studs (× 0.05), two frames blended; normal X / Y in .w / .y.
    let a = abs(up);
    var uv = in.world.xz;
    var s = 1.0;
    if (a.x > a.y && a.x >= a.z) {
        uv = in.world.zy;
        s = 0.0;
    } else if (a.z > a.y && a.z > a.x) {
        uv = in.world.xy;
        s = 0.0;
    }
    uv = uv * 0.05;
    let t = mix(
        textureSample(water_normals, water_sampler, uv, i32(wp.frames.x)),
        textureSample(water_normals, water_sampler, uv, i32(wp.frames.y)),
        wp.wave.w);
    let xy = t.wy * 2.0 - vec2<f32>(1.0);
    let tz = sqrt(clamp(1.0 - dot(xy, xy), 0.0, 1.0));
    let tan_ = mix(vec3<f32>(up.z, 0.0, -up.x), vec3<f32>(-up.y, up.x, 0.0), s);
    let bit = mix(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, -up.z, up.y), s);
    var nm = normalize(tan_ * xy.x + bit * xy.y + up * tz);
    if (dot(nm, nm) < 0.5) { nm = up; }
    // SmoothWaterFSUnified: the reflection blend deliberately stays unnormalized.
    let nr = mix(up, nm, 0.25);
    // The lit water colour (as the LQ shader: no N·L).
    let shadow = sun_shadow(in.world, up);
    let lit = in.color.rgb * (g.ambient.rgb + g.outdoor.rgb * light_grid(in.world, up)
        + env_cube(nm, light_grid(in.world, up))
        + g.sun_color.rgb * g.sun_dir.w * shadow * WATER_SUN);
    // Refraction: the scene behind, offset by the ripples (0.05 of the
    // view), unless what's there is in front of the water.
    let size = wp.view.xy;
    let px1 = px + vec2<f32>(nm.x, -nm.z) * 0.05 * size;
    let scene_d1 = (g.view_proj * vec4<f32>(scene_world(px1), 1.0)).w;
    let k1 = clamp(scene_d1 - view_d, 0.0, 1.0);
    let refr = mix(scene_rgb(px), scene_rgb(px1), k1);
    // Studio's GBufferDepth holds view depth to 500 studs (× 500).
    let depth = min(mix(scene_d0, scene_d1, k1), 500.0) - view_d;
    // How much water colour covers it: by the depth of water looked
    // through, and all of it far away (view depth × 0.004 − 1).
    let cover = clamp(clamp(depth * wp.frames.z + wp.frames.w, 0.0, 1.0)
        + clamp(view_d * 0.004 - 1.0, 0.0, 1.0), 0.0, 1.0);
    var c: vec3<f32>;
    if (below) {
        c = mix(refr, lit, WATER_BELOW_ALPHA);
    } else {
        let under = mix(refr, lit, cover);
        let reflected = reflect(-v, nr);
        let tint = mix(g.sky_grad[1].rgb, g.sky_grad[0].rgb, clamp(reflected.y * 1.5882353, 0.0, 1.0));
        let env = sky_color(reflected) * tint * light_grid(in.world, up);
        let fres = clamp((clamp(0.78 - 2.5 * abs(dot(nr, v)), 0.0, 1.0) + 0.3) * in.emissive.x, 0.0, 1.0);
        c = mix(under, water_reflection(in.world, reflected, env), fres);
        // Sun glint: GGX, roughness 0.089 + 0.911 × the map's .x, F0 0.02.
        let l = normalize(g.sun_dir.xyz);
        let hv = normalize(v + l);
        let r = 0.089 + 0.911 * t.x;
        let a2 = (r * r) * (r * r);
        let ndh = max(0.001, dot(nm, hv));
        let ldh = dot(l, hv);
        let f5 = pow(1.0 - ldh, 5.0);
        let fr = f5 + 0.02 * (1.0 - f5);
        let den = (ndh * a2 - ndh) * ndh + 1.0;
        let dterm = (a2 + a2 * a2) / ((den * den) * (ldh * 3.0 + 0.5) * (ndh * 0.75 + 0.25));
        let spec_fade = clamp(1.0 - view_d * wp.view.z, 0.0, 1.0);
        c += fr * dterm * clamp(dot(nm, l) * shadow, 0.0, 1.0) * g.sun_color.rgb * g.sun_dir.w * spec_fade;
    }
    if (g.fog.z > 0.5) {
        c = mix(c, fog_rgb_at(in.world), fog_amount(water_d));
    }
    return vec4<f32>(c, 1.0);
}

// ------------------------------------------------------------- terrain
// Studio's smooth terrain shader (SmoothCluster VS / FS HQ;
// terrain.rs TerrainVertex, terrain_mat.rs): three corner
// materials per triangle, each projected on its own axis with its own random
// turn and offset, blended by barycentric weight sharpened by the textures'
// heights, recoloured in luma / chroma, lit with GGX (F0 0.04).

struct TerrainIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,  // snorm8: xyz, w = corner / 127
    @location(2) palette: vec4<u32>, // the corners' palette entries
    @location(3) axis: vec4<u32>,    // the corners' projection axes
    @location(4) rnd: vec4<u32>,     // the corners' random bytes
};

struct TerrainOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) bary: vec3<f32>,
    @location(3) uv01: vec4<f32>,
    @location(4) uv2: vec2<f32>,
    @location(5) @interpolate(flat) palette: vec4<u32>,
    @location(6) @interpolate(flat) axis: vec4<u32>,
    @location(7) @interpolate(flat) rnd: vec4<u32>,
};

// Per palette entry (material × 3 + face), four vec4s:
// [tiling, detiling, rotation, layer (−1 = none)], [height remap scale,
// offset, sharpness, 0], [luma scale, chroma cos, chroma sin, 0],
// [colour (linear) when the pack isn't loaded, 0].
struct TerrainParams {
    p: array<vec4<f32>, 276>,
};
@group(1) @binding(0) var<uniform> tp: TerrainParams;

const TERRAIN_AXES = array<vec3<f32>, 18>(
    vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0),
    vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, -1.0, 0.0),
    vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, -1.0),
    vec3<f32>(0.70710678, 0.70710678, 0.0), vec3<f32>(-0.70710678, 0.70710678, 0.0),
    vec3<f32>(0.70710678, -0.70710678, 0.0), vec3<f32>(-0.70710678, -0.70710678, 0.0),
    vec3<f32>(0.0, 0.70710678, 0.70710678), vec3<f32>(0.0, -0.70710678, 0.70710678),
    vec3<f32>(0.0, 0.70710678, -0.70710678), vec3<f32>(0.0, -0.70710678, -0.70710678),
    vec3<f32>(0.70710678, 0.0, 0.70710678), vec3<f32>(-0.70710678, 0.0, 0.70710678),
    vec3<f32>(0.70710678, 0.0, -0.70710678), vec3<f32>(-0.70710678, 0.0, -0.70710678),
);

/// The texture plane of a projection axis: (U, V), V up the image where it can be.
fn terrain_frame(axis: u32) -> mat2x3<f32> {
    let d = TERRAIN_AXES[min(axis, 17u)];
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(d.y) > 0.9) {
        up = vec3<f32>(0.0, 0.0, -1.0);
    }
    let u = normalize(cross(up, d));
    let v = cross(d, u);
    return mat2x3<f32>(u, v);
}

/// A corner's texture coordinate at `p`: projected, tiled, turned and moved
/// by the corner's random byte (Studio's detiling).
fn terrain_uv(p: vec3<f32>, pal: u32, axis: u32, rnd: u32) -> vec2<f32> {
    let a = tp.p[pal * 4u];
    let f = terrain_frame(axis);
    let uv = vec2<f32>(dot(p, f[0]), -dot(p, f[1])) * a.x;
    let r = (f32(rnd) / 128.0 - 1.0) * a.z;
    let turned = uv * sqrt(max(1.0 - r * r, 0.0)) + uv.yx * vec2<f32>(r, -r);
    let k = f32(rnd);
    let offset = vec2<f32>(k, floor(k * 2.6651442)) * a.y;
    return turned + offset;
}

// Decoration grass (terrain.rs grass_blades): a blade's tip sinks into the
// ground with distance (Studio's Grass VS: smoothstep over the fade range
// times the blade's height), so far grass shrinks instead of popping.
const GRASS_FADE_START: f32 = 150.0;
const GRASS_FADE_LEN: f32 = 100.0;
const GRASS_BLADE_SHADE: f32 = 0.62;

@vertex
fn vs_terrain(v: TerrainIn) -> TerrainOut {
    var o: TerrainOut;
    var p = v.pos;
    if (v.palette.w == 2u) {
        let d = length(g.cam_pos.xyz - p);
        p.y -= smoothstep(0.0, 1.0, (d - GRASS_FADE_START) / GRASS_FADE_LEN) * f32(v.rnd.w) / 32.0;
    }
    o.clip = g.view_proj * vec4<f32>(p, 1.0);
    o.world = p;
    o.normal = v.normal.xyz;
    let corner = u32(round(v.normal.w * 127.0));
    o.bary = vec3<f32>(f32(corner == 0u), f32(corner == 1u), f32(corner == 2u));
    o.uv01 = vec4<f32>(
        terrain_uv(v.pos, v.palette.x, v.axis.x, v.rnd.x),
        terrain_uv(v.pos, v.palette.y, v.axis.y, v.rnd.y),
    );
    o.uv2 = terrain_uv(v.pos, v.palette.z, v.axis.z, v.rnd.z);
    o.palette = v.palette;
    o.axis = v.axis;
    o.rnd = v.rnd;
    return o;
}

fn ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>((c.r + 2.0 * c.g + c.b) * 0.25, (c.r - c.b) * 0.5, (2.0 * c.g - c.r - c.b) * 0.25);
}

/// One corner's sample: recoloured albedo (sRGB), height, world normal, roughness.
struct TerrainSample {
    albedo: vec3<f32>,
    height: f32,
    n: vec3<f32>,
    rough: f32,
};

fn terrain_sample(uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, pal: u32, axis: u32, rnd: u32, n: vec3<f32>) -> TerrainSample {
    var s: TerrainSample;
    let a = tp.p[pal * 4u];
    let layer = i32(a.w);
    s.n = n;
    // Without its pack (not loaded, or packs off) a material shows its
    // average albedo (the colour table's avgAlbedo), recoloured like a texel:
    // what Studio draws before its texture arrays fill.
    var t = vec4<f32>(tp.p[pal * 4u + 3u].rgb, 0.5);
    var su = vec4<f32>(0.5, 0.5, 1.0, 0.0);
    if (layer >= 0) {
        t = textureSampleGrad(mat_albedo, studs_sampler, uv, layer, dx, dy);
        su = textureSampleGrad(mat_surface, studs_sampler, uv, layer, dx, dy);
    }
    let rc = tp.p[pal * 4u + 2u];
    let y = ycocg(t.rgb);
    let co = rc.y * y.y - rc.z * y.z;
    let cg = rc.z * y.y + rc.y * y.z;
    let l = y.x * rc.x;
    s.albedo = clamp(vec3<f32>(l - cg + co, l + cg, l - cg - co), vec3<f32>(0.0), vec3<f32>(1.0));
    s.height = t.a;
    s.rough = 0.089 + 0.911 * su.b;
    if (layer < 0) {
        return s;
    }
    // Normal map (X right, Y up the image) in the corner's projection frame,
    // turned with the texture.
    let xy = su.rg * 2.0 - vec2<f32>(1.0);
    let z = sqrt(clamp(1.0 - dot(xy, xy), 0.0, 1.0));
    let f = terrain_frame(axis);
    let r = (f32(rnd) / 128.0 - 1.0) * a.z;
    let c = sqrt(max(1.0 - r * r, 0.0));
    // uv' = R uv: the image axes in world space.
    let tu = f[0] * c - f[1] * r;
    let tv = -(f[0] * r + f[1] * c);
    let tn = normalize(tu - n * dot(tu, n));
    let bn = normalize(tv - n * dot(tv, n));
    let bent = tn * xy.x - bn * xy.y + n * z;
    if (dot(bent, bent) > 1e-8) {
        s.n = normalize(bent);
    }
    return s;
}

@fragment
fn fs_terrain(in: TerrainOut) -> @location(0) vec4<f32> {
    let n0 = normalize(in.normal);
    let d0x = dpdx(in.uv01.xy);
    let d0y = dpdy(in.uv01.xy);
    let d1x = dpdx(in.uv01.zw);
    let d1y = dpdy(in.uv01.zw);
    let d2x = dpdx(in.uv2);
    let d2y = dpdy(in.uv2);
    let s0 = terrain_sample(in.uv01.xy, d0x, d0y, in.palette.x, in.axis.x, in.rnd.x, n0);
    let s1 = terrain_sample(in.uv01.zw, d1x, d1y, in.palette.y, in.axis.y, in.rnd.y, n0);
    let s2 = terrain_sample(in.uv2, d2x, d2y, in.palette.z, in.axis.z, in.rnd.z, n0);
    // Height blend (FS HQ): each corner's height remapped and weighted by its
    // barycentric; the highest wins over a softness of bary · sharpness; back
    // to plain barycentrics as the texture gets small on screen.
    let b0 = tp.p[in.palette.x * 4u + 1u];
    let b1 = tp.p[in.palette.y * 4u + 1u];
    let b2 = tp.p[in.palette.z * 4u + 1u];
    let h = in.bary * vec3<f32>(
        clamp(s0.height * b0.x + b0.y, 0.0, 1.0),
        clamp(s1.height * b1.x + b1.y, 0.0, 1.0),
        clamp(s2.height * b2.x + b2.y, 0.0, 1.0),
    );
    let soft = max(dot(in.bary, vec3<f32>(b0.z, b1.z, b2.z)), 1e-4);
    var w = clamp((h - vec3<f32>(max(h.y, h.z), max(h.x, h.z), max(h.x, h.y)) + 0.5 * soft) / soft,
        vec3<f32>(0.0), vec3<f32>(1.0));
    w = w / max(w.x + w.y + w.z, 1e-4);
    let far = clamp(sqrt(max(dot(d0x, d0x), dot(d0y, d0y))) * 7.0, 0.0, 1.0);
    w = mix(w, in.bary, far);
    var albedo = s0.albedo * w.x + s1.albedo * w.y + s2.albedo * w.z;
    var n = normalize(s0.n * w.x + s1.n * w.y + s2.n * w.z);
    if (in.palette.w > 0u) {
        // A grass blade: the ground's colour, darker, a little varied per blade.
        albedo = albedo * GRASS_BLADE_SHADE * (0.85 + 0.3 * f32(in.rnd.x) / 255.0);
        n = n0;
    }
    let rough = s0.rough * w.x + s1.rough * w.y + s2.rough * w.z;
    let base = sq(albedo);

    let l = normalize(g.sun_dir.xyz);
    let ndl = max(dot(n, l), 0.0);
    let shadow = select(1.0, sun_shadow(in.world, n0), ndl > 0.0);
    // The light grid's skylight (SmoothCluster FS HQ: LightGridSkylight .x
    // scales SkyAmbient and the sky's share of the ambient cube).
    let sv = light_grid(in.world, n0);
    var light = g.ambient.rgb + g.outdoor.rgb * sv + g.sun_color.rgb * g.sun_dir.w * ndl * shadow + env_cube(n, sv);
    // SmoothCluster VS/FS includes Lamp1 on faces turned away from the sun.
    light += g.lamp1.rgb * max(-dot(n, l), 0.0);
    light += local_lights(in.world, select(n, n0, g.sun_color.w > 0.5));
    var c = base * light;
    // Sun highlight: GGX with F0 0.04 (the FS HQ's sun term).
    let v = normalize(g.cam_pos.xyz - in.world);
    let hv = normalize(v + l);
    let a2 = rough * rough * rough * rough;
    let nh = max(dot(n, hv), 0.001);
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let vh = dot(l, hv);
    let fr = 0.04 + 0.96 * pow(1.0 - clamp(vh, 0.0, 1.0), 5.0);
    let spec = fr * (a2 + a2 * a2) / (dd * dd * (vh * 3.0 + 0.5) * (nh * 0.75 + 0.25));
    c += g.sun_color.rgb * g.sun_dir.w * spec * ndl * shadow;
    if (g.fog.z > 0.5) {
        let d = length(in.world - g.cam_pos.xyz);
        let f = fog_amount(d);
        c = mix(c, fog_rgb_at(in.world), f);
    }
    if (g.sky_rot.z > 3.5) {
        // RUSTUDIO_SHADE_DEBUG=skylight: the sky a point sees.
        c = vec3<f32>(sv);
    }
    return vec4<f32>(c, 1.0);
}

struct SunPass {
    vp: mat4x4<f32>,
};
@group(1) @binding(0) var<uniform> sunp: SunPass;

struct TerrainShadowOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_terrain_shadow(v: TerrainIn) -> TerrainShadowOut {
    var o: TerrainShadowOut;
    o.clip = sunp.vp * vec4<f32>(v.pos, 1.0);
    o.clip.z = max(o.clip.z, 0.0);
    return o;
}

// ------------------------------------------------------------- shadows

// One cascade of the sun's shadow (sun_shadow.rs): depth along the light,
// casters nearer the sun than the cascade's box flattened onto its near
// plane (Studio's Pass_CastShadowsPancaked).

struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_shadow(v: PartIn) -> ShadowOut {
    var o: ShadowOut;
    o.clip = sunp.vp * vec4<f32>(part_world(v), 1.0);
    o.clip.z = max(o.clip.z, 0.0);
    // CastShadow false (render.rs NO_SHADOW_BIT): outside the clip box,
    // so the part casts nothing.
    if (((u32(v.extra.y + 0.5) >> 19u) & 1u) == 1u) {
        o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return o;
}

// ---------------------------------------------------------------- lines

struct LineIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world: vec3<f32>,
};

@vertex
fn vs_line(v: LineIn) -> LineOut {
    var o: LineOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.color = v.color;
    o.world = v.pos;
    return o;
}

struct LineFrag {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

// Flags in the vertex alpha (viewport.rs): a ≥ 2 = no distance fade (a
// WireframeHandleAdornment), a ≥ 4 = always on top (drawn at the nearest
// depth, reverse-Z 1.0).
@fragment
fn fs_line(in: LineOut) -> LineFrag {
    var o: LineFrag;
    var a = in.color.a;
    o.depth = in.clip.z;
    var fade = 1.0;
    if (a >= 3.5) {
        a = a - 4.0;
        o.depth = 1.0;
    } else if (a >= 1.5) {
        a = a - 2.0;
    } else {
        let d = length(in.world - g.cam_pos.xyz);
        fade = 1.0 - clamp((d - 80.0) / 260.0, 0.0, 1.0);
    }
    o.color = vec4<f32>(sq(in.color.rgb), a * fade);
    return o;
}

// ------------------------------------------------------- final image

@group(1) @binding(0) var hdr_tex: texture_2d<f32>;
@group(1) @binding(1) var hdr_sampler: sampler;

struct PostOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_post(@builtin(vertex_index) i: u32) -> PostOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var o: PostOut;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return o;
}

// Bloom (docs/MECHANICS.md "Bloom"), as Studio's pyramid: a linear
// down-sample, mip down-samples, then tent up-samples; the CPU picks the
// pyramid size and level from the view's resolution.
@group(1) @binding(2) var bloom_tex: texture_2d<f32>;

fn bloom_source(hdr: vec3<f32>) -> vec3<f32> {
    // Studio's encoded scene target clamps sqrt(linear * 0.25) to
    // [0, 1] (DefaultUnifiedNeonFS.1); bloom decodes it as 4 * rgb².
    // Our floating-point scene target needs the equivalent range limit.
    let c = clamp(hdr, vec3<f32>(0.0), vec3<f32>(4.0));
    let m = max(c.r, max(c.g, c.b));
    if (m <= g.bloom.x) {
        return vec3<f32>(0.0);
    }
    // DownSampleGjoelLinearFS retains the linear channel ratios when
    // extracting the amount over threshold; taking another square root
    // here washed the recovered Neon emission toward white.
    return (c / (m + 0.001)) * (m - g.bloom.x);
}

// Filter the encoded scene, then decode, as Studio's UNORM scene target does.
fn bloom_scene_sample(uv: vec2<f32>) -> vec3<f32> {
    let dim = vec2<i32>(textureDimensions(hdr_tex));
    let p = uv * vec2<f32>(dim) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(p));
    let f = fract(p);
    var taps: array<vec3<f32>,4>;
    for(var i=0;i<4;i++) {
        let q = clamp(base+vec2<i32>(i&1,i>>1),vec2<i32>(0),dim-vec2<i32>(1));
        taps[i] = sqrt(clamp(textureLoad(hdr_tex,q,0).rgb*g.params.y*0.25,vec3<f32>(0.0),vec3<f32>(1.0)));
    }
    let encoded = mix(mix(taps[0],taps[1],f.x),mix(taps[2],taps[3],f.x),f.y);
    return 4.0*encoded*encoded;
}

// DownSampleGjoel8UVVS + DownSampleGjoelLinearFS: nine bilinear
// samples are decoded and averaged BEFORE thresholding. The source is the
// encoded pre-tone scene (4 * rgb²), represented here directly as linear HDR.
@fragment
fn fs_bloom_bright(in: PostOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(hdr_tex));
    let offsets = array<vec2<f32>, 9>(
        vec2<f32>(0.0), vec2<f32>(0.75, 0.0), vec2<f32>(-0.75, 0.0),
        vec2<f32>(0.0, 0.75), vec2<f32>(0.0, -0.75),
        vec2<f32>(0.70512277, 0.70512277), vec2<f32>(-0.70512277, -0.70512277),
        vec2<f32>(0.70512277, -0.70512277), vec2<f32>(-0.70512277, 0.70512277));
    var scene = vec3<f32>(0.0);
    for (var k = 0; k < 9; k++) {
        // (offsets in the half-resolution target's texels: CB1[0].zw)
        let uv = clamp(in.uv + offsets[k] * texel * 2.0, texel * 0.5, vec2<f32>(1.0) - texel * 0.5);
        scene += bloom_scene_sample(uv);
    }
    return vec4<f32>(bloom_source(scene / 9.0), 1.0);
}

// Linear pyramid levels use the same nine-tap pattern without decode or threshold.
// Every pyramid pass offsets its taps in its render TARGET's texels
// (CB1[0].zw is the target's 1/size: DownSampleGjoel8UVVS ±0.75, UpSampleTentFS
// ±2), i.e. ±1.5 source texels down and ±1 source texel up. Read as source
// texels the glow came out twice as wide per level; in the target's it matches
// Studio's capture of corpus/bloom.rbxlx (863x675) within 3 levels at every
// distance (4-stud ball +12 px: 73 vs 71, +16: 33 vs 32).

@fragment fn fs_bloom_down(in: PostOut) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(hdr_tex));
    var c = textureSampleLevel(hdr_tex, hdr_sampler, in.uv, 0.0).rgb;
    let offsets = array<vec2<f32>,8>(vec2<f32>(0.75,0.0),vec2<f32>(-0.75,0.0),vec2<f32>(0.0,0.75),vec2<f32>(0.0,-0.75),vec2<f32>(0.70512277),vec2<f32>(-0.70512277),vec2<f32>(0.70512277,-0.70512277),vec2<f32>(-0.70512277,0.70512277));
    for(var i=0;i<8;i++){ c += textureSampleLevel(hdr_tex,hdr_sampler,clamp(in.uv+offsets[i]*t*2.0,t*0.5,vec2<f32>(1.0)-t*0.5),0.0).rgb; }
    return vec4<f32>(c/9.0,1.0);
}
@fragment fn fs_bloom_up(in: PostOut) -> @location(0) vec4<f32> {
    let full = vec2<f32>(textureDimensions(bloom_tex));
    let dim = vec2<f32>(textureDimensions(hdr_tex));
    let lod = min(5.999,3.0+max(-1.0,log2(sqrt(full.x*full.y/2073600.0))+max(0.0,log2(g.bloom.y/7.0))));
    let level = round(log2(full.x/dim.x));
    let weight = select(1.0,fract(lod),level >= floor(lod)-0.1);
    let t = 1.0 / dim;
    var c = vec3<f32>(0.0);
    for(var y=-1;y<=1;y++){for(var x=-1;x<=1;x++){
        let w = select(1.0,2.0,x==0)*select(1.0,2.0,y==0);
        let uv = clamp(in.uv+vec2<f32>(f32(x),f32(y))*t,0.5/dim,vec2<f32>(1.0)-0.5/dim);
        c += textureSampleLevel(hdr_tex,hdr_sampler,uv,0.0).rgb*w;
    }}
    return vec4<f32>(c*(weight/16.0),1.0);
}

// Studio BokehFarCoCFS: 45 taps on four rings, radius 0.014 *
// intensity * CoC squared, aspect corrected; reject foreground taps.
const DOF_DISK: array<vec2<f32>, 45> = array<vec2<f32>, 45>(
vec2<f32>(0.22490672767162322998046875, 0.16340430080890655517578125),
    vec2<f32>(-0.085906721651554107666015625, 0.2643937170505523681640625),
    vec2<f32>(-0.27799999713897705078125, 3.4044057403885696227487100973264e-17),
    vec2<f32>(-0.085906721651554107666015625, -0.2643937170505523681640625),
    vec2<f32>(0.22490672767162322998046875, -0.16340430080890655517578125),
    vec2<f32>(0.540000021457672119140625, 0.0),
    vec2<f32>(0.436869204044342041015625, 0.3174040615558624267578125),
    vec2<f32>(0.16686917841434478759765625, 0.5135705471038818359375),
    vec2<f32>(-0.16686917841434478759765625, 0.5135705471038818359375),
    vec2<f32>(-0.436869204044342041015625, 0.3174040615558624267578125),
    vec2<f32>(-0.540000021457672119140625, 6.6128748929759884316731399778178e-17),
    vec2<f32>(-0.436869204044342041015625, -0.3174040615558624267578125),
    vec2<f32>(-0.16686917841434478759765625, -0.5135705471038818359375),
    vec2<f32>(0.16686917841434478759765625, -0.5135705471038818359375),
    vec2<f32>(0.436869204044342041015625, -0.3174040615558624267578125),
    vec2<f32>(0.7651021480560302734375, 0.18858073651790618896484375),
    vec2<f32>(0.58982646465301513671875, 0.522540628910064697265625),
    vec2<f32>(0.279428660869598388671875, 0.7367928028106689453125),
    vec2<f32>(-0.094982899725437164306640625, 0.78225457668304443359375),
    vec2<f32>(-0.4476350247859954833984375, 0.64851129055023193359375),
    vec2<f32>(-0.6977393627166748046875, 0.3662018477916717529296875),
    vec2<f32>(-0.78799998760223388671875, 4.4644126152591019802973182351025e-16),
    vec2<f32>(-0.6977393627166748046875, -0.3662018477916717529296875),
    vec2<f32>(-0.4476350247859954833984375, -0.64851129055023193359375),
    vec2<f32>(-0.094982899725437164306640625, -0.78225457668304443359375),
    vec2<f32>(0.279428660869598388671875, -0.7367928028106689453125),
    vec2<f32>(0.58982646465301513671875, -0.522540628910064697265625),
    vec2<f32>(0.7651021480560302734375, -0.18858073651790618896484375),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(0.93247222900390625, 0.3612416684627532958984375),
    vec2<f32>(0.73900890350341796875, 0.673695623874664306640625),
    vec2<f32>(0.4457383453845977783203125, 0.8951632976531982421875),
    vec2<f32>(0.09226836264133453369140625, 0.995734155178070068359375),
    vec2<f32>(-0.273662984371185302734375, 0.961825668811798095703125),
    vec2<f32>(-0.602634608745574951171875, 0.798017203807830810546875),
    vec2<f32>(-0.850217163562774658203125, 0.52643215656280517578125),
    vec2<f32>(-0.9829730987548828125, 0.18374951183795928955078125),
    vec2<f32>(-0.9829730987548828125, -0.18374951183795928955078125),
    vec2<f32>(-0.850217163562774658203125, -0.52643215656280517578125),
    vec2<f32>(-0.602634608745574951171875, -0.798017203807830810546875),
    vec2<f32>(-0.273662984371185302734375, -0.961825668811798095703125),
    vec2<f32>(0.09226836264133453369140625, -0.995734155178070068359375),
    vec2<f32>(0.4457383453845977783203125, -0.8951632976531982421875),
    vec2<f32>(0.73900890350341796875, -0.673695623874664306640625),
    vec2<f32>(0.93247222900390625, -0.3612416684627532958984375));
fn dof_coc(uv: vec2<f32>) -> f32 {
    let d = fx_distance(uv);
    let far = clamp((d - g.dof.x - g.dof.y) / d, 0.0, 1.0);
    let near = clamp((g.dof.x - g.dof.y - d) / d, 0.0, 1.0);
    return max(select(0.0, far, g.dof.w > 0.0), select(0.0, near, g.dof.z > 0.0));
}
@fragment fn fs_dof_h(in: PostOut) -> @location(0) vec4<f32> {
    let c = tonemap(textureSampleLevel(hdr_tex, hdr_sampler, in.uv, 0.0).rgb * g.params.y);
    return vec4<f32>(c*c, dof_coc(in.uv));
}
@fragment fn fs_dof_v(in: PostOut) -> @location(0) vec4<f32> {
    let center = textureSampleLevel(hdr_tex, hdr_sampler, in.uv, 0.0);
    let coc2 = center.a * center.a;
    let size = vec2<f32>(textureDimensions(hdr_tex));
    let scale = vec2<f32>(size.y / size.x, 1.0) * 0.014;
    let radius = coc2 * max(g.dof.z, g.dof.w);
    var sum = center * coc2;
    var weight = coc2;
    for (var i = 0; i < 45; i++) {
        let offset = DOF_DISK[i] * radius;
        let uv = in.uv + offset * scale;
        let tap = textureSampleLevel(hdr_tex, hdr_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
        let coc = tap.a;
        if (all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)) && coc*coc >= dot(offset,offset)*0.995) {
            sum += tap*coc;
            weight += coc;
        }
    }
    return sum * clamp(1.0 / (weight + 0.001), 0.0, 1.0);
}

@group(2) @binding(0) var fx_depth_ms: texture_depth_multisampled_2d;
@group(2) @binding(1) var fx_unused: texture_2d<f32>;
fn fx_depth(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(fx_depth_ms));
    let p = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(fx_depth_ms, p, 0);
}
fn fx_distance(uv: vec2<f32>) -> f32 {
    let z = g.ssao.z / max(fx_depth(uv), 1e-8);
    let ray = vec2<f32>((uv.x * 2.0 - 1.0) / (2.0 * g.ssao.x), (1.0 - uv.y * 2.0) / (2.0 * g.ssao.y));
    return min(z * sqrt(1.0 + dot(ray, ray)), 500.0);
}

@fragment
fn fs_post(in: PostOut) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(hdr_tex, hdr_sampler, in.uv, 0.0).rgb;
    // Bloom, filtered up from half resolution.
    if (g.cc_tint.w > 0.0) {
        c += textureSampleLevel(bloom_tex, hdr_sampler, in.uv, 0.0).rgb * g.cc_tint.w / max(g.params.y, 1e-3);
    }
    c = tonemap(c * g.params.y);
    // Studio's DOF works in squared display colour, after tone mapping.
    if (g.dof.z > 0.0 || g.dof.w > 0.0) {
        let distance = fx_distance(in.uv);
        let far_coc = clamp((distance - (g.dof.x + g.dof.y)) / distance, 0.0, 1.0);
        let near_coc = clamp(((g.dof.x - g.dof.y) - distance) / distance, 0.0, 1.0);
        let intensity = select(g.dof.w, g.dof.z, near_coc > 0.0);
        let coc = max(far_coc, near_coc);
        let amount = clamp(coc * 1.5, 0.0, 1.0);
        if (amount > 0.0 && intensity > 0.0) {
            // TentFilterFS: four bilinear taps at half a source texel.
            let half_texel = vec2<f32>(0.5) / vec2<f32>(textureDimensions(surface_tex));
            let blurred = (textureSampleLevel(surface_tex, hdr_sampler, in.uv-half_texel, 0.0).rgb
                + textureSampleLevel(surface_tex, hdr_sampler, in.uv+half_texel, 0.0).rgb
                + textureSampleLevel(surface_tex, hdr_sampler, in.uv+half_texel*vec2<f32>(1.0,-1.0), 0.0).rgb
                + textureSampleLevel(surface_tex, hdr_sampler, in.uv+half_texel*vec2<f32>(-1.0,1.0), 0.0).rgb) * 0.25;
            c = sqrt(mix(c * c, blurred, amount * amount));
        }
    }
    // ColorCorrectionEffect (display space).
    if (g.cc.w > 0.5) {
        c = c + vec3<f32>(g.cc.x);
        c = (c - vec3<f32>(0.5)) * (1.0 + g.cc.y) + vec3<f32>(0.5);
        let luma = dot(c, vec3<f32>(0.299, 0.587, 0.114));
        c = mix(vec3<f32>(luma), c, 1.0 + g.cc.z);
        c = c * g.cc_tint.rgb;
    }
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

// ------------------------------------------------------------------ SSAO
// Studio's screen-space ambient occlusion (SSAODepthDownFS, SSAOFS,
// SSAOBlurX/YFS, SSAOCompositFS; Studio measurements "Environment light and
// AO"), at half resolution; the result multiplies the HDR scene
// (SSAOApply). g.ssao: x, y half the projection diagonal, z near plane,
// w on.
@group(1) @binding(0) var ssao_depth_ms: texture_depth_multisampled_2d;
@group(1) @binding(1) var ssao_in: texture_2d<f32>;

/// AO depth from the reverse-Z depth buffer: view depth × 0.002 (sky 1).
fn ssao_lin(d: f32) -> f32 {
    if (d <= 0.0) {
        return 1.0;
    }
    return min(g.ssao.z / d * SSAO_DEPTH_SCALE, SSAO_DEPTH_MAX);
}

fn ssao_load(uv: vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(ssao_in));
    let p = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    return textureLoad(ssao_in, p, 0);
}

// Full → half resolution: the nearest of each 2×2.
@fragment
fn fs_ssao_down(in: PostOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(ssao_depth_ms)) - vec2<i32>(1);
    let p = vec2<i32>(in.pos.xy) * 2;
    var m = 1.0;
    for (var j = 0; j < 2; j++) {
        for (var i = 0; i < 2; i++) {
            m = min(m, ssao_lin(textureLoad(ssao_depth_ms, min(p + vec2<i32>(i, j), dims), 0)));
        }
    }
    return vec4<f32>(m, 0.0, 0.0, 1.0);
}

@fragment
fn fs_ssao(in: PostOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(ssao_in));
    let texel = 1.0 / dims;
    let d = ssao_load(in.uv).x;
    if (d > 0.99 || g.ssao.w < 0.5) {
        return vec4<f32>(1.0, d, 0.0, 1.0);
    }
    // Studio rotates the pattern by a 4×4 noise tile (randMap); here a
    // rotation from the pixel's place in its 4×4 tile.
    let q = vec2<u32>(in.pos.xy) % vec2<u32>(4u);
    let ang = f32(q.x + q.y * 4u) * 0.3926991 + 0.2;
    let rot = mat2x2<f32>(vec2<f32>(cos(ang), sin(ang)), vec2<f32>(-sin(ang), cos(ang)));
    let radius = clamp(g.ssao.xy * (SSAO_RADIUS_K / d), texel * SSAO_RADIUS_MIN_TEXELS, texel * SSAO_RADIUS_MAX_TEXELS);
    var sum = 1.0;
    var cnt = 2.0;
    for (var i = 0u; i < SSAO_SAMPLES; i++) {
        let fi = f32(i);
        let t = (fi + 1.0) * 0.1;
        let a = fi * 0.7853981;
        let bias = (fi + 1.0) * 0.01;
        let off = (rot * (vec2<f32>(cos(a), sin(a)) * t)) * radius;
        let s = vec2<f32>(ssao_load(in.uv + off).x, ssao_load(in.uv - off).x);
        let v = (s - vec2<f32>(d)) * (SSAO_DEPTH_GAIN / sqrt(1.0 - t * t)) + vec2<f32>(0.5);
        let w = clamp(min(v.x, v.y) + 0.75, 0.0, 1.0);
        cnt += 2.0 * w;
        sum += (clamp(v.x + bias, 0.0, 1.0) + clamp(v.y + bias, 0.0, 1.0)) * w;
    }
    return vec4<f32>(clamp(sum / cnt * 2.0, 0.0, 1.0), d, 0.0, 1.0);
}

fn ssao_blur(uv: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(ssao_in));
    let c = ssao_load(uv);
    let th = clamp(c.y * SSAO_BLUR_DEPTH, SSAO_BLUR_DEPTH_MIN, SSAO_BLUR_DEPTH_MAX);
    var acc = 0.0;
    var ws = 0.0;
    for (var i = -3; i <= 3; i++) {
        let fi = f32(i);
        let s = ssao_load(uv + dir * texel * fi);
        let w = exp(-fi * fi * SSAO_BLUR_K) * select(0.0, 1.0, abs(s.y - c.y) < th);
        acc += s.x * w;
        ws += w;
    }
    return vec4<f32>(acc / max(ws, 1e-6), c.y, 0.0, 1.0);
}

@fragment
fn fs_ssao_blur_x(in: PostOut) -> @location(0) vec4<f32> {
    return ssao_blur(in.uv, vec2<f32>(1.0, 0.0));
}

@fragment
fn fs_ssao_blur_y(in: PostOut) -> @location(0) vec4<f32> {
    return ssao_blur(in.uv, vec2<f32>(0.0, 1.0));
}

// The occlusion at full resolution (SSAOCompositFS): the half-resolution
// value, or where its depth isn't this pixel's, the mean of the neighbours
// 2 pixels away that match; faded out by Lighting's linear fog; multiplies
// the scene (blend Zero, Src).
@fragment
fn fs_ssao_apply(in: PostOut) -> @location(0) vec4<f32> {
    let full = vec2<f32>(textureDimensions(ssao_depth_ms));
    let dims = vec2<i32>(full) - vec2<i32>(1);
    let p = clamp(vec2<i32>(in.pos.xy), vec2<i32>(0), dims);
    let raw = textureLoad(ssao_depth_ms, p, 0);
    let d = ssao_lin(raw);
    let o = 2.0 / full;
    let c = ssao_load(in.uv);
    let n = array<vec4<f32>, 4>(
        ssao_load(in.uv + vec2<f32>(o.x, 0.0)),
        ssao_load(in.uv - vec2<f32>(o.x, 0.0)),
        ssao_load(in.uv + vec2<f32>(0.0, o.y)),
        ssao_load(in.uv - vec2<f32>(0.0, o.y)),
    );
    var m = 0.0;
    var s = 0.0;
    for (var i = 0; i < 4; i++) {
        let k = select(0.0, 1.0, abs(n[i].y - d) < SSAO_UPSAMPLE_DEPTH);
        m += k;
        s += n[i].x * k;
    }
    var ao = c.x;
    if (abs(c.y - d) * m > 0.00004) {
        ao = s / m;
    }
    // LinearizeDepthFS's y: Lighting's linear fog at the pixel (Params1.zw
    // = FogEnd, 1 / (FogEnd − FogStart)), the radial distance as Studio
    // estimates it.
    var y = 0.0;
    if (g.fog.z > 0.5 && g.fog.z < 1.5 && raw > 0.0) {
        let uvc = in.uv - vec2<f32>(0.5);
        let dist = sqrt(dot(uvc, uvc) * 4.0 + 1.0) * (g.ssao.z / raw);
        y = clamp(1.0 - (g.fog.y - dist) / max(g.fog.y - g.fog.x, 1e-3), 0.0, 1.0);
    }
    let a = (1.0 - y) * ao + y;
    return vec4<f32>(a, a, a, 1.0);
}


// Resolve the nearest covered sample, retaining reversed-Z for reconstruction.
@fragment
fn fs_glass_depth(in: PostOut) -> @location(0) f32 {
    let pixel = vec2<i32>(in.pos.xy);
    var z = 0.0;
    for (var i = 0; i < 4; i++) { z = max(z, textureLoad(ssao_depth_ms, pixel, i)); }
    return z;
}
