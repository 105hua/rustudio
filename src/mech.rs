//! Every number that shapes how the editor feels. Mirrored, with sources, in
//! docs/MECHANICS.md and mechanics.json; keep the three in sync.

// Camera
pub const FOV_DEG: f32 = 70.0;
/// Studio's default R15 Play character's body part sizes (offline Studio,
/// default HumanoidDescription; tools/fidelity capture of the control place,
/// 3 decimals): Roblox scales its body parts this way, not 1:1 with the file.
pub const R15_DEFAULT_PART_SIZES: &[(&str, [f32; 3])] = &[
    ("Head", [1.159, 1.182, 1.161]),
    ("UpperTorso", [1.943, 1.698, 1.004]),
    ("LowerTorso", [1.991, 0.401, 1.004]),
    ("LeftUpperArm", [1.001, 1.242, 1.002]),
    ("RightUpperArm", [1.001, 1.242, 1.002]),
    ("LeftLowerArm", [1.001, 1.118, 1.002]),
    ("RightLowerArm", [1.001, 1.118, 1.002]),
    ("LeftHand", [0.984, 0.316, 1.028]),
    ("RightHand", [0.984, 0.316, 1.028]),
    ("LeftUpperLeg", [0.993, 1.363, 0.973]),
    ("RightUpperLeg", [0.993, 1.363, 0.973]),
    ("LeftLowerLeg", [0.993, 1.301, 0.973]),
    ("RightLowerLeg", [0.993, 1.301, 0.973]),
    ("LeftFoot", [1.009, 0.312, 1.001]),
    ("RightFoot", [1.009, 0.312, 1.001]),
];
pub const MATERIAL_AIR: u32 = 1792; // Enum.Material.Air (Humanoid.FloorMaterial in the air)
pub const CORE_GUI_ALL: u32 = 4; // Enum.CoreGuiType.All: StarterGui::setCoreGuiEnabled branches on 4; the All entry key is the constant 4
pub const CORE_GUI_TYPE_COUNT: u32 = 10; // setCoreGuiEnabled loops types 0..9
pub const PLACE_A_GATE_MIN_MOVE: f32 = 2.0; // self-test Place A match: studs the character must cover while the harness holds W (it walked 13 in 3 s; a downed player crawls)
pub const MOON_PHASE: f32 = 0.75; // Lighting:GetMoonPhase: Roblox's moon has no cycle; the phase is a constant (estimated)
pub const SHAPECAST_MAX_SIZE: f32 = 512.0; // FInt ShapecastMaxSize: sphere diameter, block / part side length
pub const SHAPECAST_MAX_DISTANCE: f32 = 1024.0; // FInt ShapecastMaxDistance: the cast direction's length
pub const DEFAULT_GLOW_INTENSITY: f32 = 1.0; // Studio glows over-bright pixels (Neon) without a BloomEffect; BloomEffect's own defaults (Intensity 1, Size 24, Threshold 2) are the starting point, checked against the control shots (estimated)
pub const DEFAULT_GLOW_SIZE_PX: f32 = 24.0; // (BloomEffect.Size default)
pub const DEFAULT_GLOW_THRESHOLD: f32 = 2.0; // (BloomEffect.Threshold default)
pub const BLOOM_SIGMAS_PX: [f32; 2] = [3.28, 8.72]; // Studio's glow at Size 24 is two Gaussians of these widths (screen pixels, the same for a ball at 30 and 60 studs) mixed by BLOOM_WIDE_SHARE: fitted to tools/fidelity bloom (Neon balls of 1, 2, 4 studs at 15–60 studs, 863×675 view), RMS 1.06 levels on R and G out to 32 px (round 19)
pub const BLOOM_WIDE_SHARE: f32 = 0.533; // (the same fit)
pub const BLOOM_GAIN: f32 = 1.49; // halo strength per unit over Threshold: a Neon part (4× its colour) glows at 2.98 of its colour value next to it at Intensity 1 (the same fit)
pub const BLOOM_MAX_RADIUS: i32 = 96; // half-resolution taps each side at most (3 σ of the wide Gaussian at Size 56 is 61)
pub const TECHNOLOGY_SOFT: [u32; 4] = [0, 1, 2, 3]; // Enum.Technology Legacy, Voxel, Compatibility, ShadowMap: places saved without LightingStyle draw Soft in Studio, as do places saving neither (tools/fidelity: Place A saves neither and reads Soft; Voxel is Studio's default Technology; round 19: Place B's og.rbxl saves ShadowMap and no LightingStyle and reads Soft)
pub const TECHNOLOGY_FUTURE: u32 = 4; // Enum.Technology.Future: PrioritizeLightingQuality on when not saved (control place reads true; Place A Voxel reads false)
pub const TERRAIN_BRUSH_INTERVAL_S: f32 = 1.0 / 30.0; // Terrain Editor: one brush application per this while the button is held (estimated, not measured in Studio)
pub const UNDERWATER_FOG: [f32; 2] = [1.066, 0.00137]; // camera in terrain water: visibility = clamp(2^(a − k·d) − 1, 0, 1) toward the lit WaterColor (Studio's exp2 fog form, fs FogParams); fitted to Place B's lake (F4, place-b-terrain 22 s: hit distances 63–698 studs, linear-light blue channel, < 0.02 off): clear to 48 studs, gone by 778
pub const RANDOM_PCG_INC: u64 = 0x69; // Random: PCG32 (multiplier 0x5851f42d4c957f2d) with increment 105, seeded like pcg32_srandom; NextNumber = (second draw << 32 | first) × 2^-64, NextNumber(a, b) = min(1 − t) + max·t clamped; matches Studio's sequences for seeds 0, 1, 1001, 1005, 123456789
pub const TECHNOLOGY_NO_TONE_CURVE: [u32; 1] = [2]; // Enum.Technology Compatibility only (Studio migrates it on load to Voxel + a ColorGradingEffect TonemapperPreset Retro, roughly sqrt). Measured 2026-11-08 in an offline Studio (~/.cache/rustudio/lightov/tone/REPORT.md): Legacy, Voxel, ShadowMap and Future all draw the Default curve, bit-identical (RMS 1.03 levels on TONE_CURVE); round 21 had Legacy/Voxel curve-free, which only offset the sRGB texel decode fixed in 3f3dffc
pub const TECHNOLOGY_QUALITY: [u32; 2] = [3, 4]; // Enum.Technology ShadowMap, Future: PrioritizeLightingQuality on when not saved (control place, Future, reads true; Place B, ShadowMap, reads true; Place A, Voxel, false)
pub const MOVER_HOLDS_CHARACTER_FORCE: f32 = 2000.0; // a Vector LinearVelocity on the HumanoidRootPart with at least this MaxForce carries the character (an R15 weighs ~10 × 196.2 = 1962 force units: below it gravity wins); estimated, round 19 (Place C's ship: 1e7)
/// Services every place has in Studio even when its file doesn't list them
/// (made on load). Checked against offline Studio by tools/fidelity (the
/// structural dump of a place whose file holds only three services).
pub const ALWAYS_SERVICES: &[&str] = &[
    "Workspace",
    "Players",
    "Lighting",
    "MaterialService",
    "ReplicatedFirst",
    "ReplicatedStorage",
    "ServerScriptService",
    "ServerStorage",
    "StarterGui",
    "StarterPack",
    "StarterPlayer",
    "Teams",
    "SoundService",
    "TextChatService",
];
pub const NEAR: f32 = 0.1;
pub const FAR: f32 = 10_000.0;
pub const LOOK_DEG_PER_PX: f32 = 0.25;
// Studio's fly rule, read from Studio: each camera tick moves
// FLY_BASE_STEP × CAMERA_SPEED studs; after
// FLY_RAMP_AFTER_TICKS ticks of holding (no Shift) the step becomes
// min(ticks / FLY_RAMP_TICKS_PER_STUD, FLY_STEP_CAP). Studio ticks once per
// rendered frame; rustudio uses the rate that matches the offline Studio's measurement.
pub const CAMERA_SPEED: f32 = 1.5; // "Camera Speed" default,
pub const FLY_BASE_STEP: f32 = 0.5;
pub const FLY_RAMP_AFTER_TICKS: f32 = 120.0;
pub const FLY_RAMP_TICKS_PER_STUD: f32 = 240.0;
pub const FLY_STEP_CAP: f32 = 15.0;
pub const FLY_TICK_HZ: f32 = 142.7; // camera ticks per second (offline Studio: 107 studs/s ÷ 0.75 studs/tick)
pub const FLY_SPEED: f32 = FLY_BASE_STEP * CAMERA_SPEED * FLY_TICK_HZ; // 107 studs/s base
pub const FLY_SHIFT_FACTOR: f32 = 0.2; // "Camera Shift Speed" default,
pub const WHEEL_STEP: f32 = 15.0; // studs per notch: "Camera Mouse Wheel Speed" default,
pub const WHEEL_ZOOM_TO_MOUSE: bool = true; // "Camera Zoom to Mouse Position" default,
pub const PAN_FRACTION_PER_PX: f32 = 0.0016; // of the focus distance
pub const FOCUS_DISTANCE_FACTOR: f32 = 1.6; // F frames the selection at radius * this
pub const START_CAMERA_POS: [f32; 3] = [0.0, 20.0, 40.0];
pub const START_CAMERA_PITCH_DEG: f32 = -24.0;

// Tools and snapping
pub const MOVE_SNAP: f32 = 1.0; // studs; the Scale tool snaps to the same increment
pub const ROTATE_SNAP_DEG: f32 = 45.0; // Studio Home toolbar rotate increment
pub const MIN_PART_SIZE: f32 = 0.001;
pub const GIZMO_ARROW_PX: f32 = 90.0;
pub const GIZMO_RING_PX: f32 = 80.0;
pub const GIZMO_HIT_PX: f32 = 10.0;
/// Box select starts once the mouse is 3 px from the press
/// (DraggerFramework DragSelector.updateDrag: `(location - start).Magnitude < 3`).
pub const DRAG_SELECT_MIN_PX: f32 = 3.0;

// New objects and the starter place
pub const NEW_PART_SIZE: [f32; 3] = [4.0, 1.2, 2.0]; // Studio Part.Size default
pub const NEW_PART_COLOR: [u8; 3] = [163, 162, 165];
pub const BASEPLATE_SIZE: [f32; 3] = [512.0, 20.0, 512.0];
pub const BASEPLATE_COLOR: [u8; 3] = [99, 95, 98];
pub const GRID_SPACING: f32 = 4.0;
pub const GRID_EXTENT: f32 = 256.0;

// Lighting defaults (used when the place's Lighting service lacks a value)
pub const CLOCK_TIME: f32 = 14.0;
pub const LIGHT_BRIGHTNESS: f32 = 2.0;
pub const LIGHTING_BRIGHTNESS_DEFAULT: f32 = 1.0; // Lighting.Brightness in a place file that doesn't set it: Studio reads 1 (the reflection database says 1.981), round 21 terrain capture
pub const GEOGRAPHIC_LATITUDE: f32 = 41.733;
pub const AMBIENT: [u8; 3] = [70, 70, 70];
pub const OUTDOOR_AMBIENT: [u8; 3] = [128, 128, 128];

// Lighting model (measured in offline Studio, Future; see docs/MECHANICS.md "Lighting")
pub const SUN_AXIAL_TILT_DEG: f32 = 23.5; // GetSunDirection = (cos θ cos φ, sin θ cos φ, sin φ), θ = (ClockTime − 6)·15°, φ = latitude − 23.5° (5 clock times, error < 0.001)
pub const SUN_IRRADIANCE_PER_BRIGHTNESS: f32 = 0.5; // sun = 0.5 × Brightness × N·L in linear (gamma-2.0) light, white at every hour
pub const SKY_DAY_SUN_Y: f32 = 0.1; // sky at full brightness once the sun is this high (estimated)
pub const SKY_NIGHT_SUN_Y: f32 = -0.1; // sky dark (stars) once the sun is this low (estimated)
pub const SKY_NIGHT_TINT: [f32; 3] = [0.051, 0.032, 0.049]; // dusk sky / day sky in linear light (before the tone curve), ClockTime 20.3: sky (10,14,27)/(17,18,31) vs (134,194,211)/(161,200,207); its light on a white face matches (0.048, 0.031, 0.046)
pub const SKY_NIGHT_TINT_SUN_Y: f32 = -0.54; // sun height of that measurement
pub const SKY_DARK_SUN_Y: f32 = -0.95; // sky black (00:00 at latitude 41.7: sky (0,0,0))
pub const SKY_GRADIENT_TOP: [u8; 3] = [129, 195, 225]; // fallback sky (no skybox): Studio's displayed sky top (134,194,211) through the inverse tone curve
pub const SKY_GRADIENT_HORIZON: [u8; 3] = [154, 204, 217]; // likewise for the horizon (161,200,207)
pub const MOON_IRRADIANCE_PER_BRIGHTNESS: f32 = 0.03; // night: the moon (−sun direction) lights at this × Brightness × N·L, tinted by ColorShift_Top like the sun (round 26, atmos calibration n0: the floor's top (56,55,91) under Brightness 2.6 needs 0.024–0.044 by channel; walls facing the horizon unchanged)
pub const MOON_FADE_SUN_Y: f32 = 0.1; // the moon's light fades in as the sun sinks from the horizon to this far below (estimated)
// Atmosphere (round 26, tools/fidelity atmos calibration: docs/MECHANICS.md "Round 26: sky and lighting")
pub const ATMOS_BAND_WEIGHTS: [f32; 3] = [0.208, 0.527, 0.760]; // the haze band at the horizon = Color² × these (before the tone curve), the same by day and night: Color (92,102,138) → (24,65,131) and (200,60,60) → (91,26,36) on screen, both within 1 level
pub const ATMOS_HAZE_AT_1_2: f32 = 1.05; // band strength at the horizon for Haze 1.2 (Place D's); 1 = the band colour in full
pub const ATMOS_HAZE_POW: f32 = 3.6; // band strength ∝ Haze^this, capped at 1 (Haze 3 fills the sky to 25°: 0.96 there)
pub const ATMOS_BAND_DEG: f32 = 3.09; // band strength ÷ (1 + (elevation / this)^ATMOS_BAND_POW): Haze 1.2 at 1, 2, 4, 8, 15, 25, 35, 50° (0.90, 0.67, 0.37, 0.18, 0.085, 0.042, 0.021, 0.010)
pub const ATMOS_BAND_POW: f32 = 1.61;
pub const ATMOS_BAND_FLOOR: f32 = 0.0275; // the band's share everywhere, even at Haze 0 (night sky (1,3,6) instead of (1,2,2))
pub const ATMOS_FOG_K: f32 = 0.149; // objects fade toward the sky behind them: visibility = e^(−k·d), k = this × Density^ATMOS_FOG_POW per stud; fitted before the tone curve to walls at 25–1600 studs: k 0.0001 / 0.00095 / 0.0028 / 0.0185 at Density 0.15 / 0.3 / 0.395 / 0.6 (Offset 0 / 0.2 / 0.6, Haze and Decay changed nothing)
pub const ATMOS_FOG_POW: f32 = 4.2;
// Atmosphere from Studio's own code; these replace the
// round 26 fits above in the sky (AdvSkyFS) and the part fog.
pub const ATMOS_HAZE_THRESHOLD: f32 = 1.98; // Atmosphere change: A = clamp(Haze − this, ATMOS_A_MIN, 1)
pub const ATMOS_A_MIN: f32 = 0.02;
pub const ATMOS_B_HAZE: f32 = 0.5; // B = Haze × this + (Haze ≠ 0 ? ATMOS_B_ON : 0); AdvSkyFS exponent fogOffset.y = B
pub const ATMOS_B_ON: f32 = 0.6;
pub const ATMOS_T_EPS: f32 = 0.001; // AdvSkyFS: t = exp2(fogOffset.x / (this + max(dir.y, 1e-5)^fogOffset.y)), fogOffset.x = −A / ln 2
pub const ATMOS_DECAY_LIN: f32 = 4.0; // glare exponent per channel = exp2((1 − Decay)² + this·(1 − Decay)) − 1
pub const ATMOS_FOG_DENSITY_K: f32 = 0.1; // part fog with an Atmosphere: FogParams.z = −0.14427·clamp(Density,0,1)^4, i.e. visibility = e^(−this·Density⁴·studs)
pub const ATMOS_FOG_DENSITY_POW: f32 = 4.0;
pub const ATMOS_FOG_MIP: f32 = 5.0; // fog colour = the sky cube along the view ray at mip max(Offset, visibility) × this (DefaultUnified*FS)
pub const ATMOS_ENV_CUBE_LEVELS: f32 = 8.0; // mips of Studio's prefiltered sky cube the fog reads (estimated: 128 px); rustudio's skybox has more, so the fog lod is offset by the difference
pub const ENV_PREFILTER_SIZE: u32 = 128; // the face size of that cube (2^(ATMOS_ENV_CUBE_LEVELS − 1)): render::cube_mips GGX-filters the sky's levels below it (Studio's FastConvolutionPS)
pub const ENV_PREFILTER_SAMPLES: usize = 64; // GGX importance samples per texel of those levels (each reads the box chain at its solid angle)
pub const ATMOS_HAZE_K: f32 = 2.65; // haze = skybox irradiance(dir) × Color² × mean irradiance × this (structure from AdvSkyFS +; this factor is Lighting's members, not yet read: measured on two Colors, day and night alike: 2.93-3.12 from the faces in Python, 2.65 once rustudio draws it (its haze read 6 % over Studio at 2.95))
pub const TONE_CURVE: [f32; 5] = [6.0402, 0.0064675, 2.41386, 0.656391, 1.157207]; // Default tone curve a..e: y = x(ax+b)/(x(ax+c)+d)·e, fitted to 16 Studio screenshots, RMS 0.38 levels
pub const LIGHT_INTENSITY: f32 = 1.06; // local light = Color² × Brightness × 1.06 × falloff × N·L (fit over 5 configurations)
pub const LIGHT_MAX_RANGE: f32 = 120.0; // Range setters clamp to [0, 120]: SpotLight, PointLight/SurfaceLight
pub const LIGHT_MAX_ANGLE: f32 = 180.0; // SpotLight.Angle and SurfaceLight.Angle clamp to [0, 180]
pub const CLOCK_TIME_MAX: f32 = 24.0; // ClockTime: clamp to [0, 24], × 3600, truncated to whole seconds, stored as µs (× 1 000 000)
pub const SECONDS_PER_HOUR: f32 = 3600.0;
pub const ENV_SCALE_MAX: f32 = 1.0; // EnvironmentDiffuseScale clamps to [0, 1]
pub const SHADOW_SOFTNESS: f32 = 0.2; // Lighting.ShadowSoftness when a place saves none (the API's default)
pub const SHADOW_SOFTNESS_MAX: f32 = 1.0; // ShadowSoftness clamps to [0, 1]

/// What Studio's property setters do to a number before storing it, read
/// as Studio does. None = stored as given (e.g.
/// Lighting.Brightness and ExposureCompensation
/// store unclamped).
pub fn setter_clamp(class: &str, prop: &str, v: f32) -> Option<f32> {
    let light = matches!(class, "PointLight" | "SpotLight" | "SurfaceLight");
    Some(match (class, prop) {
        (_, "Range") if light => v.clamp(0.0, LIGHT_MAX_RANGE),
        ("SpotLight" | "SurfaceLight", "Angle") => v.clamp(0.0, LIGHT_MAX_ANGLE),
        ("Lighting", "ClockTime") => clock_time(v),
        ("Lighting", "EnvironmentDiffuseScale" | "EnvironmentSpecularScale") => {
            v.clamp(0.0, ENV_SCALE_MAX)
        }
        ("Lighting", "ShadowSoftness") => v.clamp(0.0, SHADOW_SOFTNESS_MAX),
        _ => return None,
    })
}

/// ClockTime as Studio keeps it: clamped to [0, 24] and cut to whole seconds.
pub fn clock_time(v: f32) -> f32 {
    (v.clamp(0.0, CLOCK_TIME_MAX) * SECONDS_PER_HOUR).trunc() / SECONDS_PER_HOUR
}
pub const MAX_LIGHTS: usize = 256; // nearest lights drawn per frame
pub const SHADOW_MAP_TEXELS: u32 = 2048; // the depth map the sun's cascades are drawn into one at a time: the largest tile (SUN_CASCADE_TILES)
pub const SHADOW_LOD_SCALE: f32 = 2.0; // casters can be this much smaller on screen than drawn parts before they're skipped
pub const ENV_SPEC_ROUGHNESS: f32 = 0.5; // the roughness of every material for the sky reflection and the sun highlight (Studio: 0.089 + 0.911 × a per-material vertex value, VARYING4.w; the table isn't read yet; estimated from Place B Newer's N1 road)
pub const ENV_SPEC_LOD_SPAN: f32 = 5.0; // the sky reflection reads the skybox at mip (coarsest − this × (1 − roughness)); Studio's part FS reads its prefiltered cube at roughness × 5 (ShadowMap part FS)
pub const ENV_DIFFUSE_GAIN: f32 = 1.0; // EnvironmentDiffuseScale × skybox irradiance × this (calibrated against Studio)
pub const TEXTURE_ANISOTROPY: u16 = 8; // part textures: mipmapped, trilinear, 8× anisotropic, no mip bias. Studio binds material maps with filter 2 and anisotropy 0 = the device default, which a running offline Studio holds at 8; Vulkan's sampler has mipLodBias 0
pub const LIGHT_TEST_TOLERANCE: u8 = 8; // self-test: lit colours within this many levels (0-255) of Studio's
// Light grid skylight (lightgrid.rs; Studio measurements "Light grid
// (skylight)"): read from Studio and a running Studio.
pub const LIGHTGRID_CHUNKS: [usize; 3] = [8, 4, 8]; // chunks (32×16×32 voxels of 4 studs) in Studio's grid: 1024×256×1024 studs
pub const LIGHTGRID_NORMAL_OFFSET: f32 = 6.0; // studs out along the normal the shaders read the grid (DefaultUnified*VS, SmoothClusterVS: pos + normal × 6)
pub const LIGHTGRID_BORDER: f32 = 0.4921875; // the shaders take the grid only within this share of its size from its centre, per axis, and open sky beyond (CB0 LightConfig3, read from memory: 0.5 − 1/128)
pub const LIGHTGRID_MOVES_REFRESH_S: f32 = 1.5; // ...and after parts only moved (characters, enemies) at most this often
pub const LIGHTGRID_REFRESH_S: f32 = 0.25; // rebuild after a part or terrain edit at most this often (Studio updates the touched chunks over a few frames)
// Local lights blocked by parts (round 26, lightocc.rs; corpus lightwall-soft
// / lightwall-real, docs/MECHANICS.md "Round 26"). Studio: Shadows on blocks them
// (Future: shadow maps, hard; Soft: its 4-stud lighting voxels, which a
// closed room keeps the light in but an open wall lets it round); Shadows
// off goes through everything.
pub const LIGHTOCC_CELLS_XZ: usize = 128; // voxels per side
pub const LIGHTOCC_CELLS_Y: usize = 48; // voxels tall
pub const LIGHTOCC_VOXEL_SOFT: f32 = 4.0; // studs per voxel, LightingStyle Soft (Studio's lighting voxels)
pub const LIGHTOCC_VOXEL_REAL: f32 = 2.0; // studs per voxel, Realistic (finer for its shadow-mapped lights; estimated)
pub const LIGHTOCC_AHEAD: f32 = 0.6; // grid centre this many half-widths ahead of the camera (estimated: Place B Newer N4's lights are 250 studs ahead)
pub const LIGHTOCC_SNAP: i32 = 8; // the grid moves in steps of this many voxels
pub const LIGHTOCC_REFRESH_S: f32 = 0.5; // rebuild at most this often while parts move
pub const LIGHTOCC_MIN_ALPHA: f32 = 0.5; // parts more see-through than this don't block light (as the roof map; estimated)
pub const LIGHTOCC_DENSITY: f32 = 20.0; // per stud of solid: light let through = exp(-this × studs; estimated: lightwall-real's 0.2-stud wall leaves the floor at the ambient level)
pub const LIGHTOCC_START_VOXELS: f32 = 1.0; // the march starts this many voxels out along the normal (past the surface's own part)
pub const LIGHTOCC_LIGHT_SKIP_VOXELS: f32 = 0.5; // and stops this far short of the light (its fitting)
pub const LIGHTOCC_MAX_STEPS: f32 = 24.0; // samples per light at most (one per voxel up to this)
// See-through Neon (round 26, corpus lightwall-soft / -real: cyan (60, 220,
// 255) panels at Transparency 0.5 / 0.86 / three stacked 0.86 over a grey
// floor, the same in Soft and Realistic): Studio adds none of the weak
// channel (red stays the floor's 31) and the strong ones evenly (green =
// blue: 231 / 114 / 175), and hides nothing behind: as if each channel's
// glow (colour² × 4) over the cut were added × (1 − Transparency).
pub const NEON_SEE_THROUGH_CUT: f32 = 1.0; // fitted (rustudio 200 / 121 / 164)
pub const NEON_SEE_THROUGH_MAX: f32 = 1.2; // at most this much glow per channel (fitted)

// Undo
pub const UNDO_LIMIT: usize = 500;
pub const UNDO_MERGE_MS: u64 = 700;

// Harness
pub const SCREENSHOT_FRAMES: u64 = 120;

// Scripts (Luau runtime)
/// A resume running longer than this errors "Script timeout: exhausted
/// allowed execution time". Studio's setting
/// Studio.ScriptTimeoutLength, ("Lua Debugger");
/// its GlobalSettings_13.xml holds 10. (Was an estimated 3 s.)
pub const SCRIPT_TIMEOUT_S: f32 = 10.0;
pub const MAX_DISPATCH_ROUNDS: usize = 64; // chained signal fires handled per frame

// Physics
pub const GRAVITY: f32 = 196.2; // studs/s², Workspace.Gravity default
pub const PART_DENSITY: f32 = 0.7; // Plastic, g/cm³ (PhysicalProperties)
pub const PART_FRICTION: f32 = 0.3; // Plastic
pub const PART_ELASTICITY: f32 = 0.5; // Plastic
pub const PHYSICS_HZ: f32 = 60.0; // fixed step rate
pub const STUDIO_PHYSICS_HZ: f32 = 240.0; // Studio's physics step rate; a character in the air moves by its velocity before gravity each step (tools/fidelity jumpApex: 7.269 at JumpPower 50 = 53²/2g + 53/480)
pub const PHYSICS_SUBSTEPS: usize = 4; // 240 Hz solver
pub const FALLEN_PARTS_HEIGHT: f32 = -500.0; // Workspace.FallenPartsDestroyHeight default

// Character (Play)
pub const WALK_SPEED: f32 = 16.0; // Humanoid.WalkSpeed default
pub const JUMP_HEIGHT: f32 = 7.2; // Humanoid.JumpHeight default
// Round 22: Studio's Humanoid floor finder and controller.
pub const FLOOR_RAY_SPREAD: f32 = 0.4; // floor rays start at ±0.4 × the root's size; 0.4 × size.y below its centre
pub const FLOOR_RAY_MIN_SIZE: f32 = 0.05; // root sizes count at least this
pub const FLOOR_RAYS_ALONG: [f32; 5] = [0.0, 1.0, -1.0, 2.0, -2.0]; // first pass: 5 rays along the root's Z at these × 0.4 × size.z
pub const FLOOR_RAYS_CORNERS: [(f32, f32); 4] = [(1.0, -1.0), (1.0, 1.0), (-1.0, -1.0), (-1.0, 1.0)]; // second pass when none hit: the corners (x, z) × 0.4 × size
pub const FLOOR_RAY_EXTRA: f32 = 0.3; // ray length += 0.3 × (HipHeight + root size.y): FInt ExtraFloorRaycastDistanceHipHeightPercent = 30
pub const FLOOR_RAY_HIP_HELD: f32 = 1.5; // ray length += HipHeight × 1.5 while it has a floor
pub const FLOOR_RAY_HIP_FREE: f32 = 1.1; // … × 1.1 without one
pub const HUMANOID_ALTITUDE_GAIN: f32 = 39.0; // below floor + HipHeight + half the root the root rises at 39 × the gap studs/s (fitted to Studio's step-up and landing traces, round 22 movement_probe: 39.5 / 39 / 42 at gaps 0.06 / 0.03 / 0.01); above it gravity alone brings it down
pub const HUMANOID_BRAKE_BASE: f32 = 0.6; // Fitted base: movement-r6 slippery Studio accelerates/brakes at 7.343 studs/s²; 2.0 overestimated it by 1.4. With no move input on a floor the horizontal velocity closes on the floor's at most BASE + PER × μ studs/s², μ = (f·w + 0.3) / (w + 1) with the character's Plastic (fitted, round 22 movement_probe friction lanes: Studio 8.8 at μ 0.003, 32 at 0.013, 121 at 0.053, a frame or two at ≥ 0.15)
pub const HUMANOID_BRAKE_PER_FRICTION: f32 = 2270.0; // (same fit; residuals under 1 studs/s²)
pub const HUMANOID_CEILING_BOUNCE: f32 = 0.5; // the head hitting a ceiling sends the root back down at this × its speed (fitted: Studio's ceiling jump lands 0.11 s after the jump; Plastic elasticity 0.5)
pub const HUMANOID_WALL_BOUNCE_FRICTION: f32 = 0.8; // on a floor, a body that walked into a wall bounces off it and its slide along it loses this × the wall's friction mix × the speed pushed into it (Plastic: 0.24; fitted, round 25: Studio's wall_slide averages 8.62 instead of 11.31 studs/s at 45°, both rigs; round 22's 0.122 was fitted to wall_slide's peaks). Held flush (pressed in the air, then landing) it loses the full Coulomb mix: Plastic 0.3, R6 wall_jump_diag after landing 2.87 = 7.16 − 0.3 × 14.31, corner 7.92 = 11.31 × 0.7 once settled (round 26: R15's faster ~5 there was the wall's end, not friction). The body also turns flush to the wall
pub const HUMANOID_BOX_CHAMFER: f32 = 0.3; // the body box's vertical edges are cut this far (at most 0.6 × its smaller half-width): Studio's box pivots round a wall's end and gets free ~0.3 studs sooner (movement-wall wall_jump_diag: R6 starts leaving at x 8.64, a square box at 9.0; R15 ~8.6) and turns away (yaw +0.17 R6, +0.30 R15); the cut edge's slanted normal does both (estimated, round 26)
pub const HUMANOID_JUMP_WALL_HOLD_S: f32 = 0.03; // jumping while walking into a wall, Studio stays Jumping this long, its step re-applying the take-off speed (Jumping adds mass × (1.06 × GetJumpVelocity − v·up) each step until that is ≤ 1e-6, which the wall's friction keeps from happening): movement-wall wall_jump_diag Jumping lasts 0.034–0.048 s in 6 of 6 runs over 3 captures, a free jump 0.007; rustudio's own samples land a frame late, so the measured 0.04 rose too far (fitted, round 26: diagonal rise 8.31 R6 / 8.16 R15 for Studio's 8.0–8.4 / 8.7–9.1)
pub const HUMANOID_JUMP_WALL_SLIDE_MIN: f32 = 0.2; // … only when the walk also slides along the wall by more than this × its length: straight in (wall_jump) Studio's Jumping lasts one step (R6 3 of 3 captures, R15 1 of 3; the other two 0.042), at 26.6° (wall_jump_diag, 0.45 of the walk along it) 0.034–0.048 (estimated threshold, round 26)
pub const HUMANOID_WALL_AIR_GRIP: f32 = 32.0; // after a jump into a wall its friction brakes the rise by this (studs/s²) on top of gravity (fitted, round 26: straight in Studio rises 6.55–6.76 for a free 7.27 (rustudio 6.54); the first frames read 20–90, noisy; then the body has drifted off the wall and falls at g)
pub const HUMANOID_WALL_AIR_GRIP_S: f32 = 0.1; // … for this long after the take-off (the same lanes: gone by ~0.1 s)
pub const HUMANOID_WALL_AIR_PUSH: f32 = 0.30; // in the air the slide along a wall gains this × the speed pushed into it instead (R15; fitted, movement-wall wall_jump_diag: Studio 11.44 studs/s along the wall for 7.16 + 14.31 into it, 0.30; round 25 had one 0.28 for both rigs, round 26's slow start after a jump made the plateau count)
pub const HUMANOID_WALL_AIR_PUSH_R6: f32 = 0.22; // … R6 (fitted to where it lands, x 7.24 / 7.30 / 7.93 over three captures; its plateau reads 10.5–10.8, 0.24–0.26, but 0.25 lands it ~0.3 studs late). Measured per rig: R15 collides with four parts (root, Upper/LowerTorso, Head; a deeper 1.16-stud box), R6 with two (Torso, Head), and its contacts differ (round 26)
pub const HUMANOID_WALL_AIR_ACCEL: f32 = 70.0; // … reached at this many studs/s² (fitted, round 26: movement-wall wall_jump_diag ramps 5 → 10.6 within ~0.08 s once the grip below ends, R6 ~75, R15 ~65; round 25's 50 started at once and averaged the slow start in)
pub const HUMANOID_WALL_GRIP_SLIDE_ACCEL: f32 = 20.0; // … while the wall's grip brakes the rise after a jump into it (HUMANOID_WALL_AIR_GRIP_S) the slide speeds up only this fast, and not at all while Jumping is held (the same lanes: R6 2.89 held, then 3.2 / 3.8 / 4.3 / 4.7 / 5.05 at 0.02 s steps; R15 3.34 held, then 3.9 → 5.2) (fitted, round 26)
pub const HUMANOID_WALL_LAND_DECEL: f32 = 50.0; // landing from that slide, the floor's slower one is reached at this many studs/s² (round 25: R15 11.5 → 8.2 → 5.9 → 3.8 at 0.05 s steps; R6 is faster, 10.4 → 2.5 in ~0.08 s)
pub const HUMANOID_WALL_NORMAL_Y: f32 = 0.7; // a body-box contact whose normal is flatter than this is a wall (rustudio's own)
pub const HUMANOID_BOX_SKIN: f32 = 0.02; // the body box's contact offset (rustudio's own)
pub const HUMANOID_BOX_CLIMB_DEG: f32 = 5.0; // the body box itself never climbs slopes steeper than this: the floor rays do the climbing (rustudio's own)
pub const LANDED_TO_RUNNING_S: f32 = 0.05; // Studio's Landed → Running (movement_probe: 0.05 s in jump_stand, jump_run, ledge, ceiling_jump)
pub const SWIM_TILT_RATE: f32 = 6.0; // a swimming Humanoid's root tips forward to flat at this rate /s (Studio: flat within ~0.75 s of swimming forward, movement_probe water; the rate is estimated)
pub const WATER_DRAG_PER_PART: f32 = 13.4; // water drag /s on a swimming Humanoid = this × its parts / its mass; against SWIM_GAIN 35 it settles at 35 / (35 + drag) × WalkSpeed (fitted, round 22 movement_probe water: Studio R15 16 parts, mass 14.26 → 11.12 studs/s at 16; R6 7 parts, 12.6 → 13.26; this predicts 11.14 and 13.20)
pub const CHARACTER_RADIUS: f32 = 1.0;
pub const CHARACTER_HALF_HEIGHT: f32 = 1.5; // capsule: 5 studs tall
pub const CHARACTER_HOVER: f32 = 0.05; // the character controller keeps its capsule this far off what it stands on; the rig is lowered by it so the feet touch (Studio: root rests HipHeight + half the root part above the floor exactly, tools/fidelity restingHeight)
pub const RESPAWN_TIME: f32 = 5.0; // Players.RespawnTime default
pub const CHARACTER_STEP_HEIGHT: f32 = 1.1; // ledges the character walks up (estimated)
pub const MAX_SLOPE_DEG: f32 = 89.0; // Humanoid.MaxSlopeAngle default
pub const PLAYER_NAME: &str = "Player"; // offline Studio names the Play player "Player" (seen in an offline Studio)
pub const NO_SPAWN_HEIGHT: f32 = 10.0; // capsule centre when there is no SpawnLocation (estimated)
/// StarterPlayer property → the Player property each joining player gets (Roblox docs, StarterPlayer).
pub const STARTER_PLAYER_TO_PLAYER: &[(&str, &str)] = &[
    ("CameraMaxZoomDistance", "CameraMaxZoomDistance"),
    ("CameraMinZoomDistance", "CameraMinZoomDistance"),
    ("CameraMode", "CameraMode"),
    ("DevCameraOcclusionMode", "DevCameraOcclusionMode"),
    ("DevComputerCameraMovementMode", "DevComputerCameraMode"),
    ("DevComputerMovementMode", "DevComputerMovementMode"),
    ("DevTouchCameraMovementMode", "DevTouchCameraMode"),
    ("DevTouchMovementMode", "DevTouchMovementMode"),
    ("EnableMouseLockOption", "DevEnableMouseLock"),
    ("AutoJumpEnabled", "AutoJumpEnabled"),
    ("HealthDisplayDistance", "HealthDisplayDistance"),
    ("NameDisplayDistance", "NameDisplayDistance"),
];
pub const JUMP_TAKEOFF_FACTOR: f32 = 1.06; // Humanoid jump: commanded speed (JumpPower, or √(2gh)/1.06) leaves at 1.06×; an offline Studio: 7.27 studs at JumpPower 50, 28.86 at 100
pub const JUMP_SPEED_MAX: f32 = 1000.0; // JumpHeight's commanded speed is clamped to [0, 1000]
pub const WHEEL_INPUT_PER_NOTCH: f32 = 0.5; // the game's wheel input (InputObject.Position.Z, PointerAction) per notch: Studio's camera zoomed 12.5 → 23.375 → 14.917 → 19.146 studs for one Qt wheel event of −3 / +2 / −1 notches (offline Studio), the ZoomController step for 0.5 per notch
pub const TURN_RATE: f32 = 7.0; // Humanoid turning toward its walking direction: the angle left shrinks as exp(-7 t) (fitted to Studio's turn in an offline Studio: 1.48 rad left → 0.93 / 0.38 / 0.12 after 0.10 / 0.22 / 0.35 s)
pub const CAMERA_TYPE_CUSTOM: u32 = 5; // Enum.CameraType.Custom: a joining client's camera
pub const CHARACTER_SKIN: [u8; 3] = [245, 205, 48]; // head and arms: Studio's offline Play character (offline Studio: Head / RightUpperArm.Color)
pub const CHARACTER_SHIRT: [u8; 3] = [13, 105, 172]; // torso (offline Studio: UpperTorso / LowerTorso.Color)
pub const CHARACTER_PANTS: [u8; 3] = [164, 189, 71]; // legs (offline Studio: RightUpperLeg / LeftLowerLeg.Color)
pub const WAIT_MIN_S: f64 = 0.0333; // legacy wait(): shortest wait (Roblox docs: about 1/30 s)
pub const WHEEL_POINTS_PER_NOTCH: f32 = 40.0; // egui's line scroll size
pub const R15_HIP_HEIGHT: f32 = 2.0; // Humanoid.HipHeight of Studio's default R15 body (content/avatar/characterR15DynamicHeadV2.rbxm)
#[cfg_attr(not(test), allow(dead_code))] // checked by rig::hip_tests
pub const R15_PLAY_HIP_HEIGHT: f32 = 2.192423; // HipHeight of Studio's own Play character with default scales (tools/fidelity physics capture, control place); the body file's legs scaled to R15_DEFAULT_PART_SIZES give the same
pub const WRAP_PUFFINESS_MIN: f32 = -1.0; // WrapLayer.Puffiness: maxss with
pub const WRAP_PUFFINESS_MAX: f32 = 1.0; //... then minss with
pub const WRAP_PUFFINESS_DEFAULT: f32 = 1.0; // FInt WrapPuffinessDefaultPercent = 100 / 100
pub const COREMESH_V2_SIZE_BYTES: usize = 4; // COREMESH v2: u32 compressed size first; a chunk under 4 bytes is "too small"
pub const DRACO_ATTRIBUTES: usize = 5; // Draco attributes Studio requires by unique id 0..4
pub const DEFAULT_RIG_R15: bool = true; // default character rig when nothing else says (StarterPlayer default)
pub const JUMP_POWER: f32 = 50.0; // Humanoid.JumpPower default (used when UseJumpPower)
#[allow(dead_code)] // used by library.luau __rs_seat (same number)
pub const SEAT_WELD_MIN_ROOT_HEIGHT: f32 = 0.05; // R6 SeatWeld C1.Y = -(max(root.Size.Y, 0.05) × 0.5 + 0.5); R15 from the rig attachments (library.luau __rs_seat)
pub const PROJECT_DEBOUNCE_MS: u64 = 150; // a project folder's disk changes are applied once the folder has been quiet this long (well inside the 1 s promise)
pub const GAMEPAD_SCAN_S: f32 = 1.0; // how often Play looks for plugged / unplugged gamepads (rustudio's own polling)
pub const SEAT_COOLDOWN_S: f32 = 1.0; // after leaving a seat the humanoid doesn't sit again for this long (estimated; Roblox debounces re-sitting)
pub const CLIMB_SPEED_FACTOR: f32 = 0.7; // Climbing update: vertical speed = |move velocity| × 0.7, no sideways velocity
pub const CLIMB_MIN_INPUT: f32 = 0.1; // a move speed under 0.1 climbs at CLIMB_CREEP instead
pub const CLIMB_CREEP: f32 = 0.01;
pub const CLIMB_DOWN_DOT: f32 = -0.2; // a move direction whose dot with the root's flat look is under -0.2 climbs down
pub const CLIMB_REACH: f32 = 0.9; // a TrussPart this far past the capsule's side, in the walking direction, is climbed (estimated)
pub const SWIM_FORCE_MAX: f32 = 10000.0; // Swimming: the swim force's per-axis limit, × the Humanoid's scale³
pub const SWIM_FORCE_UP_MAX: f32 = 1000.0; // the same, upward
pub const SWIM_GAIN: f32 = 35.0; // Swimming: force = 35 × mass × (desired − current velocity): the velocity closes on the move vector at 35 /s
pub const SWIM_FLAT_RATIO: f32 = 2.0; // a move whose horizontal part exceeds 2 × its vertical swims flat at the full speed
pub const SWIM_FLOAT_SPEED: f32 = 1.63; // studs/s up while submerged, R15: the humanoid floats to the surface (Studio: 1.63 while swimming forward, round 22 movement_probe water)
pub const SWIM_FLOAT_SPEED_R6: f32 = 0.446; // the same for R6 (Studio: 0.446 every frame of the water lane's 3 s swim, round 23 movement_probe water; a buoyancy model with R15's fit predicts ~2, so this is measured per rig)
pub const SWIM_SURFACE_GAP: f32 = 0.19; // the float fades to 0 at the water's surface − this (root centre, studs) …
pub const SWIM_SURFACE_FADE: f32 = 0.27; // … over this depth (Studio R15 in an 8-deep pool: 1.63 at 7.21, 1.28 at 7.60, 0.26 at 7.77, round 23 movement_probe water; was a step at the capsule centre + 1.5, which stopped at 7.19)
pub const SWIM_ENTER_WATER: f32 = 0.5; // water occupancy at the root that starts Swimming
pub const FALLING_DOWN_S: f32 = 3.0; // RBX:HUMAN:FallingDown: timer = 3.0 (0x40400000); every state's timer starts at 0 otherwise
pub const RAGDOLL_TIMER_S: f32 = 5.0; // RBX::HUMAN::Ragdoll: timer = FInt RagdollDefaultTimerTenthSecond / 10
pub const RAGDOLL_EARLY_EXIT_S: f32 = 2.0; // Ragdoll exit check: with the humanoid's jump byte, ends once 5.0 − 2.0 s remain
pub const RAGDOLL_SLOW_ENOUGH: f32 = 1.0; // ends when the root's |linear velocity|² and |angular velocity|² are both under (FInt RagdollSlowEnoughToNotTenth 10 / 10)²
pub const GETTING_UP_S: f32 = 0.0; // RBX::HUMAN::GettingUp has no timed exit: it ends once upright; the controller's character is always upright, so one step
pub const ROOT_PART_FALLBACK_R15: &str = "UpperTorso"; // Humanoid RootPart / Torso getter: R15 slot, else (index 1, "UpperTorso")
pub const ROOT_PART_FALLBACK_R6: &str = "Torso"; // same getter, R6: slot, else (index 6, "Torso")
pub const ATTRIBUTE_NAME_MAX: usize = 100; // Instance:SetAttribute name check: at most 0x64 bytes, [A-Za-z0-9_./-] only, not empty
pub const BENCH_WARMUP_S: f32 = 3.0; // frames ignored after load / Play start before measuring
pub const PERF_SECONDS: f32 = 10.0; // --bench default measured time
pub const SELFTEST_PERF_SECONDS: f32 = 5.0; // the self-test's perf step (round 20: it runs after the 74 s Place A child and the run has 90 s in all)
pub const JOURNAL_LIMIT: usize = 1_000_000; // change-journal entries kept before readers must rebuild
pub const RENDER_CELL: f32 = 128.0; // studs per side of a static-batch culling cell
pub const LOD_MIN_PX: f32 = 1.0; // parts whose bounding radius covers fewer pixels are skipped
pub const STATIC_SETTLE_S: f32 = 0.5; // new parts join the static batches after this long without more new parts
pub const PENDING_MAX: usize = 4096; // ...or as soon as this many are waiting
pub const DYNAMIC_SETTLE_S: f32 = 5.0; // a per-frame part this long without changes rejoins the static batches
pub const STATIC_REBUILD_MIN_S: f32 = 3.0; // static rebuilds (a full re-sort + upload) at most this often
pub const DEPTH_BIAS_STEP: f32 = 4e-7; // per-part relative depth nudge (16 steps) against coplanar flicker
pub const MAX_PHYSICS_STEPS: usize = 3; // physics steps per frame at most (slow frames slow the simulation instead of piling up)
pub const PHYSICS_LENGTH_UNIT: f32 = 5.0; // rapier length scale in studs (sleep thresholds, contact tolerances) (estimated)
pub const MAX_FRAME_DT: f32 = 1.0 / 15.0; // Play's game step per frame at most (Studio 0.746: PreAnimation dt and DistributedGameTime stop at 0.0667 s however long the frame; waits keep wall time)
pub const SCRIPT_FRAME_BUDGET_MS: f64 = 6.0; // per side: waiting scripts past this resume next frame (estimated)
pub const BENCH_TURN_DEG_PER_S: f32 = 36.0; // edit-mode benchmark camera turn rate
pub const BODY_MOVER_P_SCALE: f32 = 1.0; // legacy BodyMover P/D read as force (torque) per stud (radian) and per stud/s (rad/s) (estimated)
pub const ALIGN_DAMPING_RATIO: f32 = 1.0; // AlignPosition/AlignOrientation: damped spring with natural frequency = Responsiveness (estimated)
pub const CONSTRAINT_RIGID_FRACTION: f32 = 1.0; // LinearVelocity/AngularVelocity and RigidityEnabled aligns close this fraction of the error per step (estimated)
pub const TERRAIN_COLLIDER_CELL: f32 = 64.0; // studs per side of one terrain triangle-mesh collider (estimated)
pub const TERRAIN_FRICTION: f32 = 0.4; // terrain collider friction, one value for all materials (estimated)
pub const TERRAIN_ELASTICITY: f32 = 0.1; // terrain collider elasticity, one value for all materials (estimated)
pub const CLICK_DISTANCE: f32 = 32.0; // ClickDetector.MaxActivationDistance default (Roblox docs)
pub const MOVE_TO_REACHED: f32 = 1.0; // Humanoid:MoveTo counts as reached within this many studs (estimated)
pub const MOVE_TO_TIMEOUT_S: f32 = 8.0; // Humanoid:MoveTo gives up after 8 s (Roblox docs)
pub const EXPLOSION_PUSH: f32 = 0.0004; // speed change per (BlastPressure / mass) for unanchored parts hit (estimated)
pub const EXPLOSION_MAX_SPEED: f32 = 150.0; // cap on that speed change, studs/s (estimated)
pub const EXPLOSION_FLASH_S: f32 = 0.6; // how long the explosion flash shows (estimated)
pub const CHAT_SHOW_S: f32 = 30.0; // chat lines stay over the viewport this long (estimated)
pub const JOURNAL_READER_LAG: usize = 200_000; // journal entries a reader may fall behind before it must rebuild
pub const WATER_COLOR_DEFAULT: [u8; 3] = [12, 84, 92]; // Terrain.WaterColor default
pub const WATER_TRANSPARENCY_DEFAULT: f32 = 0.3; // Terrain.WaterTransparency default
pub const WATER_REFLECTANCE_DEFAULT: f32 = 1.0; // Terrain.WaterReflectance default
// Round 24 water (water.rs, after Studio's SmoothWater FS / VS HQ). The
// shader's forms follow Studio's; these rates and scales
// feed its CB3 constants, which the engine sets, and are estimated.
pub const WATER_WAVE_SIZE_DEFAULT: f32 = 0.15; // Terrain.WaterWaveSize default
pub const WATER_WAVE_SPEED_DEFAULT: f32 = 10.0; // Terrain.WaterWaveSpeed default
pub const WATER_FRAMES_PER_SPEED: f32 = 0.8; // normal-map frames / s per WaterWaveSpeed (estimated: 8 / s at the default 10)
pub const WATER_WAVE_NUMBER: f32 = 0.2; // the VS sine waves' wave number, rad / stud (CB3[0].x, estimated)
pub const WATER_WAVE_PHASE_PER_SPEED: f32 = 0.1; // their phase, rad / s per WaterWaveSpeed (CB3[0].y, estimated)
pub const WATER_WAVE_AMPLITUDE: f32 = 1.0; // studs per WaterWaveSize (CB3[0].z, estimated)
pub const WATER_WAVE_FADE: f32 = 1.0 / 800.0; // waves fade out by this many studs of view depth (CB0[28].y, estimated)
pub const WATER_DEPTH_SCALE: f32 = 0.0003; // how fast the bottom fades into WaterColor per stud of water (to Studio's 500-stud depth cap), × (1 − T) / (T + WATER_DEPTH_SOFT) (CB3[2].x, fitted to place-b-water W1/W2: 0.05 → MAE 25.5 / 20.6, 0.005 → 20.5 / 17.8, this → 17.1 / 15.7; Studio shows the bottom through 50+ studs at T 0.3)
pub const WATER_DEPTH_SOFT: f32 = 0.05;
pub const GRASS_BLADES_PER_STUD2: f32 = 4.0; // Terrain.Decoration blades per square stud of Grass top (estimated from an offline Studio's grass hill)
pub const GRASS_HEIGHT_PER_LENGTH: f32 = 2.5; // blade height in studs per unit of Terrain.GrassLength (estimated)
pub const GRASS_BLADE_HALF_WIDTH: f32 = 0.1; // studs (estimated)
pub const GRASS_LENGTH_DEFAULT: f32 = 0.7; // Terrain.GrassLength when the place doesn't set it
pub const GRASS_RADIUS: f32 = 260.0; // chunks within this many studs of the camera get their blades (they have sunk away by 250)
pub const TERRAIN_TOP_NORMAL_Y: f32 = 0.5; // a terrain corner shows its material's top texture from this normal Y up (bottom from minus it down), else the side (estimated)
pub const TERRAIN_OCCUPANCY_SCALE: f32 = 255.0; // occupancy byte = round(share × 255), read (byte + 1) / 256: ReadVoxels after FillBlock / FillBall in an offline Studio (round 21, terrain_probe.luau)
/// The flat colour (sRGB) an offline Studio draws a pre-2022 terrain
/// material in without its texture pack, where it isn't the pack's
/// avgAlbedo (FStringTerrainColorTable): grey materials come out greener
/// and less blue. Measured round 26 (tools/fidelity terraincolor, flat
/// slabs lit by Ambient 128 alone: Studio's linear colour over rustudio's,
/// times avgAlbedo); the rest, and any material a place recolours with
/// Terrain.MaterialColors, keep avgAlbedo.
pub const TERRAIN_FALLBACK_ALBEDO: &[(u8, [u8; 3])] = &[
    (3, [103, 102, 86]),   // Slate (avgAlbedo 96, 102, 96)
    (4, [122, 131, 112]),  // Concrete (125, 122, 115)
    (6, [156, 138, 98]),   // Sand (148, 131, 101)
    (8, [72, 91, 79]),     // Rock (80, 84, 90)
    (10, [184, 188, 209]), // Snow (187, 194, 215)
    (12, [79, 84, 56]),    // Mud (89, 78, 66)
    (13, [46, 52, 34]),    // Basalt (46, 45, 42)
    (14, [92, 79, 50]),    // Ground (87, 81, 56)
    (16, [114, 121, 104]), // Asphalt (114, 117, 110)
    (22, [99, 106, 85]),   // Pavement (99, 100, 95)
];
pub const PERF_PLACE: &str = "/tmp/rustudio-place-a/place-a-latest.rbxl"; // main test place: the latest Place A build saved from offline Studio (opened read-only)
pub const PERF_PLACE_FALLBACK: &str = "/home/user/games/place-b/og.rbxl"; // secondary test place (opened read-only)
pub const PERF_MIN_FPS: f32 = 60.0; // perf check target in Play (person's request)
pub const SNAP_ON_BY_DEFAULT: bool = false; // Studio opens with move and rotate snapping unticked
pub const TEXT_WIDTH_PER_SIZE: f32 = 0.5; // TextBounds before the text is first drawn: width per character, as a fraction of TextSize (estimated)

// ParticleEmitters, Beams and Trails (effects.rs, round 25). The shapes of
// the formulas are Studio's (ParticleCustomVS / FS, TrailFSLight); these are
// the caps and scales the CPU side chooses.
pub const FX_MAX_DIST: f32 = 1024.0; // effects farther from the camera aren't simulated or drawn (estimated; keeps Place B's far emitters free)
pub const FX_PARTICLES_PER_EMITTER: usize = 10_000; // live particles one emitter may hold (a guard, estimated: Studio showed no per-emitter cap; effects-lanes' Rate 1000 screen held ~2700 at 6 s and ~5700 at 12 s, still rising)
pub const FX_MAX_RATE: f32 = 500.0; // particles per second one emitter makes at most (measured: effects-lanes, Rate 1000 covers its screen as ~470 / s would, 2700 at 6 s and 5700 at 12 s, while Rate 100 makes 100 / s; FRMParticleBudget* flags, values not found)
pub const FX_TRAIL_AGE_LEAD: f32 = 0.12; // s: Studio reads a trail's Color as if each point were this much older, and drops points this much early (measured: effects-lanes, trails at 6 studs/s with Lifetime 1: the white → green ramp fits to ±2 levels with 0.12 s, and the trail is 0.7 studs short)
pub const FX_TRAIL_FADE_LEAD: f32 = 0.06; // s: the same for Transparency (measured: effects-lanes' white 0 → 1 trail fits to ±2 levels over its first two thirds with 0.06 s; 0 reads 8–15 levels bright, 0.12 6–29 dark)
pub const FX_LIGHT_TOWARD_CAMERA: f32 = 6.0; // studs: a particle's light is read this far toward the camera (ParticleLightingVS: centre + normalize(camera − centre) × 6)
pub const FX_GLASS_MAX_DIST: f32 = 256.0; // studs: see-through parts farther than this don't dim effects (CPU budget, estimated)
pub const FX_GLASS_MAX: usize = 512; // the nearest see-through parts kept for that per frame (CPU budget)
pub const FX_GLASS_MIN_ALPHA: f32 = 0.02; // see-through parts more opaque than this dim the effects behind them (round 26: the world GUI pass runs after them)
pub const FX_PARTICLES_MAX: usize = 20_000; // particles drawn per frame across all emitters (CPU budget, nearest emitters first)
pub const FX_MIN_PIXELS: f32 = 0.35; // a particle narrower than this on screen is skipped (it can't cover a pixel centre: the pass has no MSAA)
pub const FX_PARTICLE_HALF_SIZE: f32 = 1.0; // a particle quad's half-width per stud of Size: Size is its radius (measured: effects place, a Size 4 SquareParticle is 71 px wide in Studio, 35 px with Size as the full width)
pub const FX_BEAM_MAX_SEGMENTS: i32 = 1000; // Beam.Segments is clamped to 1..1000 (Roblox's documented range)
pub const FX_TRAIL_MAX_POINTS: usize = 2000; // trail points kept per Trail (estimated cap; MinLength 0.1 at 60 fps needs ~60 / s of Lifetime)
pub const FX_DT_MAX: f32 = 0.1; // simulation step cap per frame (s): a hitch doesn't fling particles
pub const FX_MOON_PER_BRIGHTNESS: f32 = 0.115; // night: the moon lights a LightInfluence 1 particle at this × Lighting.Brightness, no N·L (measured: calibration o/n, offline Studio at ClockTime 0, Ambient 0.2, OutdoorAmbient 0.3: a white particle's light reads 0.308 at Brightness 2 and 0.07 at Brightness 0, inverting the tone curve; parts take MOON_IRRADIANCE_PER_BRIGHTNESS)
pub const FX_LOCAL_SLOPE: f32 = 0.055; // a local light's light on effects falls this much per stud per unit of Brightness × Color², straight down to 0 at Range + FX_LOCAL_REACH, no N·L (measured: calibration q, offline Studio, PointLights Range 8 / 16 / 30 at Brightness 1 and 4: a white LightInfluence 1 particle reads 0.050–0.055 per stud at Range 8, 0.05–0.07 at 16, 0.04–0.056 at 30; Studio lights effects from its voxel light grid)
pub const FX_LOCAL_REACH: f32 = 4.0; // studs past Range a local light still reaches effects (measured: calibration q, the light reaches 0 at Range + 4 for Range 8 and 16, + 2.5 for Range 30)
pub const FX_LIGHT_CAP: f32 = 1.0; // the luminance an effect's ambient + local light is held to before the sun is added (ParticleLightingFS min(…, cap); measured: calibration q, white particles read 0.99 near a Range 30 PointLight; an orange light's red channel reads 1.58 at luminance 0.85)
pub const FX_LIGHT_CLAMP: f32 = 4.5; // an effect's linear light is held to this (ParticleCustomFS sqrt(clamp(lin × exposure))) before it joins the scene's light, so Studio's bloom sees no more of it (measured: calibration b, white squares glow the same at Brightness 4, 10 and 100: the halo 1.8–3.5 studs out reads 8.8 / 10.0 / 12.4 levels; rustudio unclamped 8.9 / 28.7 / 96.3, clamped 9.1 / 11.0 / 10.9)

// GUI (gui.rs)
pub const GUI_TOP_INSET: f32 = 58.0; // GuiService:GetGuiInset() in Studio's Play (offline Studio, 863x675 view): ScreenGuis without IgnoreGuiInset start this far down
pub const GUI_FONT_EM_PER_TEXT_SIZE: f32 = 1.04; // font em = TextSize × this / (line height per em): a line is TextSize tall; the 1.04 matches TextService widths ("START GAME" in Oswald 14/18/24/32 → 46/58/78/104 px in an offline Studio)
pub const GUI_BUTTON_HOVER_DARKEN: f32 = 0.3; // AutoButtonColor: background darkened this much under the mouse (estimated)
pub const GUI_BUTTON_PRESS_DARKEN: f32 = 0.5; // ... and while pressed (estimated)
pub const GUI_SELECTION_RGB: [u8; 3] = [0, 162, 255]; // GuiService.SelectedObject outline (estimated)
pub const GUI_VIEWPORT_MAX_TRIS: usize = 3000; // ViewportFrame meshes are thinned to about this many triangles
pub const GUI_SURFACE_LIFT: f32 = 0.01; // SurfaceGuis sit this far off the face (studs, estimated)
pub const GUI_SURFACE_AFFINE_ERR_PX: f32 = 0.5; // SurfaceGui triangles whose colour / texture varies are split until screen-space (affine) interpolation is within this many pixels of perspective-correct; flat-colour ones never (projection keeps their edges exact). Round 20: the old fixed 40 px split made 2.1M triangles per Place B frame
pub const GUI_SCROLL_PX_PER_POINT: f32 = 1.0; // ScrollingFrame: CanvasPosition pixels per pixel of the scroll event
pub const GUI_SCROLL_PX_PER_NOTCH: f32 = 100.0; // FInt ScrollWheelDeltaAmount default
pub const QUERY_GRID_CELL: f32 = 16.0; // studs: spatial grid cell for Raycast / GetPartBoundsInBox / GetPartsInPart
pub const QUERY_GRID_MAX_CELLS: usize = 512; // parts spanning more cells than this are always tested
pub const QUERY_GRID_SCAN_CELLS: usize = 4096; // queries covering more cells than this scan every part
pub const TELEPORT_MIN_STUDS: f32 = 0.5; // a script moving the HumanoidRootPart further than this teleports the character (estimated)

/// How much each Material darkens a part's colour in Studio's Future
/// lighting (linear, per channel), measured on white cubes in offline Studio
/// against SmoothPlastic: top and front faces agree within 0.01
/// (/tmp/rs-light/matcal.json; docs/MECHANICS.md "Lighting"). Plastic,
/// SmoothPlastic, Neon, Glass and the rest: 1. Now only a cross-check:
/// parts use Studio's own table colours (matpack::fallback_albedo), which
/// these measurements match within 0.013.
#[cfg(test)]
pub fn material_albedo(material: u32) -> [f32; 3] {
    match material {
        512 => [0.785, 0.785, 0.785],  // Wood
        528 => [0.682, 0.661, 0.622],  // WoodPlanks
        784 => [0.6, 0.6, 0.6],        // Marble
        788 => [0.359, 0.359, 0.352],  // Basalt
        800 => [0.562, 0.562, 0.562],  // Slate
        804 => [0.087, 0.095, 0.101],  // CrackedLava
        816 => [0.661, 0.661, 0.661],  // Concrete
        820 => [0.838, 0.832, 0.804],  // Limestone
        832 => [0.195, 0.195, 0.195],  // Granite
        836 => [0.305, 0.305, 0.314],  // Pavement
        848 => [0.631, 0.609, 0.609],  // Brick
        864 => [0.661, 0.661, 0.661],  // Pebble
        880 => [0.682, 0.609, 0.527],  // Cobblestone
        896 => [0.357, 0.357, 0.357],  // Rock
        912 => [0.455, 0.44, 0.431],   // Sandstone
        1040 => [0.381, 0.216, 0.14],  // CorrodedMetal
        1056 => [0.434, 0.434, 0.434], // DiamondPlate
        1072 => [0.992, 0.992, 0.992], // Foil
        1088 => [0.6, 0.6, 0.6],       // Metal
        1280 => [0.411, 0.411, 0.381], // Grass
        1284 => [0.222, 0.208, 0.195], // LeafyGrass
        1296 => [0.737, 0.737, 0.737], // Sand
        1312 => [0.169, 0.165, 0.16],  // Fabric
        1328 => [0.725, 0.725, 0.725], // Snow
        1344 => [0.258, 0.258, 0.258], // Mud
        1360 => [0.198, 0.198, 0.191], // Ground
        1376 => [0.229, 0.229, 0.229], // Asphalt
        1392 => [0.737, 0.737, 0.742], // Salt
        1536 => [0.992, 0.992, 0.992], // Ice
        1552 => [0.772, 0.799, 0.799], // Glacier
        2304 => [0.431, 0.431, 0.431], // Cardboard
        2305 => [0.641, 0.641, 0.641], // Carpet
        2306 => [0.868, 0.868, 0.868], // CeramicTiles
        2307 => [0.357, 0.357, 0.357], // ClayRoofTiles
        2308 => [0.495, 0.498, 0.446], // RoofShingles
        2309 => [0.767, 0.767, 0.767], // Leather
        2310 => [0.737, 0.737, 0.737], // Plaster
        2311 => [0.661, 0.661, 0.661], // Rubber
        _ => [1.0, 1.0, 1.0],
    }
}
// Lighting's time-of-day sky levels (2026-11-05, read from a running Studio's
// Lighting members (c0), (c1), (k) at 1/8 h steps and five
// EnvironmentDiffuseScale values, an offline Studio 0.746; Studio measurements "Environment
// light and AO"). Keyed on ClockTime only (latitude 0 / 41.7 / 70 alike).
// Each row: hour, c0 at s=0 (rgb), c0 at s=1, c1 at s=0, c1 at s=1, k0, k1;
// linear between rows (wrapping at 24 h), c0 / c1 = lerp(s=0, s=1, s) and
// k = max(k0, s × k1) (exact to 6e-8 at every sample). c0² / c1² are the
// shaders' SkyGradientTop / Bottom; k drives the sun / moon (Lamp0, Lamp1).
pub const SKY_LEVEL_COLUMNS: usize = 15;
pub const SKY_LEVELS: [f32; 255] = [
    0.0, 0.0, 0.0, 0.0, 0.2, 0.2, 0.2, 0.0, 0.0, 0.0, 0.1, 0.1, 0.1, 0.2, 0.35,
    3.0, 0.0, 0.0, 0.0, 0.125, 0.125, 0.125, 0.0, 0.0, 0.0, 0.2, 0.2, 0.2, 0.14, 0.35,
    4.0, 0.0, 0.0, 0.0, 0.1, 0.1, 0.1, 0.21, 0.21, 0.28, 0.21, 0.21, 0.28, 0.12, 0.35,
    5.0, 0.07, 0.07, 0.1, 0.17, 0.17, 0.2, 0.336667, 0.27, 0.293333, 0.336667, 0.27, 0.293333, 0.1, 0.35,
    5.5, 0.2, 0.15, 0.01, 0.3, 0.25, 0.11, 0.4, 0.3, 0.3, 0.4, 0.3, 0.3, 0.05, 0.35,
    6.0, 0.2, 0.15, 0.01, 0.3, 0.25, 0.11, 0.3, 0.2, 0.3, 0.3, 0.2, 0.3, 0.0, 0.35,
    6.125, 0.4, 0.3625, 0.2575, 0.475, 0.4375, 0.3325, 0.475, 0.4, 0.475, 0.475, 0.4, 0.475, 0.6, 0.6,
    6.5, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.75, 0.75,
    17.5, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.75, 0.75,
    17.653846, 0.815385, 0.753846, 0.707693, 0.846154, 0.784616, 0.738462, 0.815385, 0.784616, 0.753846, 0.815385, 0.784616, 0.753846, 0.35, 0.35,
    17.75, 0.7, 0.6, 0.525, 0.75, 0.65, 0.575, 0.7, 0.65, 0.6, 0.7, 0.65, 0.6, 0.1, 0.35,
    18.0, 0.4, 0.2, 0.05, 0.5, 0.3, 0.15, 0.4, 0.3, 0.2, 0.4, 0.3, 0.2, 0.1, 0.35,
    18.333333, 0.0, 0.0, 0.0, 0.1, 0.1, 0.1, 0.3, 0.2, 0.3, 0.3, 0.2, 0.3, 0.1, 0.35,
    18.375, 0.0, 0.0, 0.0, 0.100735, 0.100735, 0.100735, 0.3, 0.2, 0.3, 0.3, 0.2, 0.3, 0.1, 0.35,
    18.5, 0.0, 0.0, 0.0, 0.102941, 0.102941, 0.102941, 0.3, 0.2, 0.3, 0.3, 0.2, 0.3, 0.1, 0.35,
    20.0, 0.0, 0.0, 0.0, 0.129412, 0.129412, 0.129412, 0.3, 0.2, 0.3, 0.3, 0.2, 0.3, 0.127273, 0.35,
    21.0, 0.0, 0.0, 0.0, 0.147059, 0.147059, 0.147059, 0.0, 0.0, 0.0, 0.2, 0.2, 0.2, 0.145455, 0.35,
];
// Part lighting from Studio's HQ part shaders (DefaultUnifiedFlatOpaqueFS /
// TiledFS variant 3) and the cube build.
pub const ENV_ROUGH_BASE: f32 = 0.089; // shader roughness = this + ENV_ROUGH_SCALE × the material's (SpecularMap .y / vertex value)
pub const ENV_ROUGH_SCALE: f32 = 0.911;
pub const ENV_SPEC_MIP_PER_ROUGHNESS: f32 = 5.0; // prefiltered env cube read at mip roughness × this
pub const ENV_REFLECT_UP_GAIN: f32 = 1.588235; // spec env × mix(SkyGradientBottom, Top, clamp(R.y × this, 0, 1))
pub const ENV_INDOOR_FACES: &str = "rbxasset://textures/sky/indoor512_"; // + bk/dn/ft/lf/rt/up .tex: the indoor environment (PrefilteredEnvIndoorTexture; its irradiance × EnvironmentDiffuseScale is AmbientCube[6..11])
pub const ENV_SPEC_FADE: f32 = 0.0008; // the sky reflection fades out with distance: s × clamp(1 − studs × this) (CB0[28].y, RefractionBias_FadeDistance_GlowFactor, read from a running Studio's constant buffer)
// Screen-space ambient occlusion, as Studio's SSAO passes (
// SSAODepthDownFS, SSAOFS, SSAOBlurX/YFS, SSAOCompositFS; LinearizeDepthFS;
// setup). On by default in Studio's edit view (calibration: a white
// block on a white floor lit by Ambient alone reads 125 at the contact against
// 140; by the sun alone 177 against 190: it darkens all light).
pub const SSAO_DEPTH_SCALE: f32 = 0.002; // depth buffer for AO = view depth × this, at most SSAO_DEPTH_MAX; sky = 1 (LinearizeDepthFS)
pub const SSAO_DEPTH_MAX: f32 = 0.998;
pub const SSAO_RADIUS_K: f32 = 0.004; // sample radius (uv) = 0.5 × projection diagonal × this ÷ AO depth: 2 studs in the world
pub const SSAO_RADIUS_MIN_TEXELS: f32 = 10.0; // the radius clamped to this .. SSAO_RADIUS_MAX_TEXELS half-resolution texels
pub const SSAO_RADIUS_MAX_TEXELS: f32 = 100.0;
pub const SSAO_SAMPLES: u32 = 8; // sample pairs, at 45° steps, at (i + 1) × 0.1 of the radius
pub const SSAO_DEPTH_GAIN: f32 = 125.0; // occlusion = depth difference × this ÷ sqrt(1 − t²) + 0.5, i.e. 4 studs
pub const SSAO_BLUR_K: f32 = 0.2222222; // blur weights exp(−i² × this), i = −3..3, both axes
pub const SSAO_BLUR_DEPTH: f32 = 0.064; // blur taps count while |Δdepth| < clamp(depth × this, SSAO_BLUR_DEPTH_MIN, SSAO_BLUR_DEPTH_MAX)
pub const SSAO_BLUR_DEPTH_MIN: f32 = 0.00008;
pub const SSAO_BLUR_DEPTH_MAX: f32 = 0.008;
pub const SSAO_UPSAMPLE_DEPTH: f32 = 0.0008; // composite: neighbours within this AO depth of the pixel's (SSAOCompositFS)
// The sun / moon (Lamp0) and the back light (Lamp1) as Studio builds them
//.
pub const LAMP0_GAIN: f32 = 0.9; // Lamp0 = this × Brightness × mix(k², k², ColorShift_Top²); k = the sky level (SKY_LEVELS)
pub const LAMP1_GAIN: f32 = 0.36; // Lamp1 = this × Brightness × mix(k², 0, ColorShift_Bottom²): lights faces turned away from Lamp0
pub const LAMP_COLOR_MEAN: f32 = 0.3333; // the colour mix's channel mean factor
pub const LAMP_MOON_BELOW_SUN_Y: f32 = -0.3; // Lamp0 comes from the moon (−sun) once the sun is lower than this

// Local light shadows (Future / Realistic), as Studio (2026-11-05;
// Studio measurements "Local light shadows"): its tiled part shading and a
// running Studio's light / shadow buffers.
pub const LOCAL_LIGHT_DIST: f32 = 580.0; // Globals.localLightDist: lights within this of the camera are shadow-mapped (beyond: Studio's voxel light map)
pub const HYBRID_LERP_SLOPE: f32 = 0.05; // Globals.hybridLerpSlope: the hand-off from shadowed to voxel light over 1/this studs
pub const EVSM_POS_EXP: f32 = 5.54; // Globals.evsmPosExp
pub const EVSM_NEG_EXP: f32 = 5.54; // Globals.evsmNegExp
pub const EVSM_SHADOW_BIAS: f32 = 0.0075; // Globals.shadowBias: minimum variance = (exp × bias × e^(±exp·z))²
pub const EVSM_BLEED_CUT: f32 = 0.2; // light-bleed reduction: clamp((p − 0.2) × 1.25) (the shader's constants)
pub const LSHADOW_TAN_HALF_FOV: f32 = 1.053_774; // each cube face's projection scale 0.94896 = 1/tan(46.5°): 93° faces
pub const LSHADOW_NEAR: f32 = 0.1; // face near plane (far = Range)
pub const LSHADOW_ATLAS: u32 = 2048; // rustudio's atlas side (Studio packs faces at 3/32 × 1/20 of its atlas; its size isn't read yet)
pub const LSHADOW_FACE: u32 = 192; // pixels per face (estimated: Studio's ratios fit 192 on a 2048-wide atlas)
pub const LSHADOW_MAX_LIGHTS: usize = 16; // shadowed lights at once: (2048/192)² faces / 6 (Studio has 128 slots and shrinks faces as its atlas fills)
// The sun's shadow cascades (sun_shadow.rs; docs/MECHANICS.md "Sun shadow cascades").
pub const SUN_CASCADE_SPLITS: [f32; 5] = [1e-5, 20.0, 45.0, 140.0, 600.0]; // view depths the four cascades span
pub const SUN_CASCADE_TILES: [u32; 4] = [384, 768, 1024, 2048]; // texels across each cascade's tile (a running Studio's atlas matrices)
pub const SUN_CASCADE_SNAP: f32 = 8.0; // cascades 1..3 sit on a light-space grid of 2 × radius / this
pub const SUN_CASCADE_PANCAKE: f32 = 64.0; // studs toward the sun a cascade's depth reaches beyond its sphere (a running Studio's matrices are 2 × radius + 64 deep)
pub const SUN_CASCADE_CASTER_DEPTH: f32 = 2000.0; // studs toward the sun casters are gathered from
pub const SUN_CASCADE_UP_FLIP: f32 = 0.7; // light space's up turns from +Y to +Z when the light's |y| reaches this
pub const SHADOW_SOFTNESS_BLUR: [f32; 2] = [1.71, 0.05]; // the blur's reach in studs = ShadowSoftness × [0] + [1] (as Studio); also what a cascade's sphere gives up for the shader's test
pub const SUN_SHADOW_ATLAS: [u32; 2] = [3200, 2048]; // rustudio's atlas for the four tiles (Studio shares a 4096 × 7680 one with the local lights)
pub const SUN_CASCADE_ORIGINS: [[u32; 2]; 4] = [[2816, 1024], [2048, 1024], [2048, 0], [0, 0]]; // each tile's corner in that atlas
pub const SUN_DIST: f32 = 580.0; // Globals.sunDist: the sun's map hands off to the voxel sun from here, over 1 / HYBRID_LERP_SLOPE studs
pub const LIGHT_INNER: f32 = 0.71; // light record m1 = Range × 1000 + this: full brightness to this × Range (all lights read)

// Part materials, from Studio's own code and memory (2026-11-05;
// Studio measurements "Part materials"). Per material (Enum.Material order of
// matpack's tables): Studio's PartMaterialConsts (the part shaders' CB2),
// read from its material records in a running offline Studio
// (: [table][43] × 0x130 bytes, shader type, the
// 80-byte constants; filled per table, looked up) — and the
// box faces' studs per texture repeat, measured from the per-instance uvScale
// (CB1 row 5 of DefaultUnified*VS: uv = TEXCOORD0 × uvScale + uvOffset) of
// sentinel-sized parts in a clean place with Use2022Materials off and on.
// Columns: textureTiling (Tiled FS: uv × this), normalDetailTiling,
// normalDetailScale (detail normal = NormalDetailMap(uv × tiling).xy × scale,
// added to the normal's xy), noiseScale (Detiled FS), detiled (shader type
// 21: DefaultUnifiedDetiledFS), studs per tile.
pub const PART_MATERIAL_PRE2022: [[f32; 6]; 43] = [
    [10.4, 1.0, 0.5, 0.0, 0.0, 8.0], // Plastic (type 0)
    [0.0, 1.0, 0.0, 0.0, 0.0, 8.0], // SmoothPlastic (type 0)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Snow (type 21)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Sandstone (type 21)
    [0.0, 1.0, 0.0, 0.0, 0.0, 8.0], // Neon (type 2)
    [1.0, 7.0, 0.6, 0.0, 0.0, 5.0], // Wood (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // WoodPlanks (type 6)
    [1.0, 0.0, 0.0, 0.0, 0.0, 10.0], // Marble (type 6)
    [1.0, 5.0, 1.0, 0.0, 0.0, 10.0], // Slate (type 7)
    [1.0, 10.0, 1.0, 0.0, 0.0, 20.0 / 3.0], // Concrete (type 7)
    [1.0, 0.0, 0.0, 0.0, 0.0, 10.0], // Granite (type 7)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Brick (type 6)
    [1.0, 6.0, 1.5, 0.0, 0.0, 10.0], // Pebble (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 10.0], // CorrodedMetal (type 7)
    [1.0, 0.0, 0.0, 0.0, 0.0, 5.0], // DiamondPlate (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 10.0], // Foil (type 8)
    [1.0, 0.0, 0.0, 0.0, 0.0, 5.0], // Metal (type 4)
    [0.65, 0.0, 0.0, 0.0, 0.0, 20.0 / 3.0], // Grass (type 6)
    [1.0, 0.0, 0.0, 0.0, 0.0, 10.0], // Sand (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 20.0 / 3.0], // Fabric (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 10.0], // Ice (type 8)
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0 / 0.0675], // Cobblestone (type 6)
    [1.0, 0.0, 0.0, 0.0, 0.0, 4.0], // Glass (type 10)
    [1.0, 0.0, 0.0, 0.0, 0.0, 4.0], // ForceField (type 12)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Mud (type 21)
    [1.0, 0.0, 0.0, 0.04, 1.0, 8.0], // Limestone (type 21)
    [1.0, 15.0, 0.15, 0.05, 1.0, 8.0], // Basalt (type 21)
    [1.0, 0.0, 0.0, 0.05, 1.0, 8.0], // Ground (type 21)
    [1.0, 0.0, 0.0, 0.05, 1.0, 8.0], // LeafyGrass (type 21)
    [1.0, 0.0, 0.0, 0.045, 1.0, 8.0], // Rock (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Pavement (type 4)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Glacier (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // CrackedLava (type 9)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Salt (type 7)
    [1.0, 7.0, 0.6, 0.03, 1.0, 8.0], // Asphalt (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Cardboard (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Carpet (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // CeramicTiles (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // ClayRoofTiles (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // RoofShingles (type 4)
    [1.0, 0.0, 0.0, 0.0, 1.0, 8.0], // Leather (type 21)
    [1.0, 0.0, 0.0, 0.0, 1.0, 8.0], // Plaster (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Rubber (type 4)
];
pub const PART_MATERIAL_2022: [[f32; 6]; 43] = [
    [1.0, 1.0, 0.5, 0.0, 0.0, 8.0], // Plastic (type 0)
    [0.0, 1.0, 0.0, 0.0, 0.0, 8.0], // SmoothPlastic (type 0)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Snow (type 21)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Sandstone (type 21)
    [0.0, 1.0, 0.0, 0.0, 0.0, 8.0], // Neon (type 2)
    [1.0, 7.0, 0.1, 0.03, 1.0, 8.0], // Wood (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // WoodPlanks (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Marble (type 4)
    [1.0, 5.0, 1.0, 0.03, 1.0, 8.0], // Slate (type 21)
    [1.0, 10.0, 0.1, 0.03, 1.0, 8.0], // Concrete (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Granite (type 7)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Brick (type 4)
    [1.0, 6.0, 1.5, 0.0, 0.0, 8.0], // Pebble (type 7)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // CorrodedMetal (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // DiamondPlate (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Foil (type 8)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Metal (type 4)
    [1.0, 0.0, 0.0, 0.02, 1.0, 8.0], // Grass (type 21)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Sand (type 21)
    [1.0, 0.1, 0.1, 0.0, 0.0, 8.0], // Fabric (type 7)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Ice (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Cobblestone (type 7)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Glass (type 10)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // ForceField (type 12)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Mud (type 21)
    [1.0, 0.0, 0.0, 0.04, 1.0, 8.0], // Limestone (type 21)
    [1.0, 15.0, 0.15, 0.05, 1.0, 8.0], // Basalt (type 21)
    [1.0, 0.0, 0.0, 0.05, 1.0, 8.0], // Ground (type 21)
    [1.0, 0.0, 0.0, 0.05, 1.0, 8.0], // LeafyGrass (type 21)
    [1.0, 0.0, 0.0, 0.045, 1.0, 8.0], // Rock (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Pavement (type 4)
    [1.0, 0.0, 0.0, 0.03, 1.0, 8.0], // Glacier (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // CrackedLava (type 9)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Salt (type 7)
    [1.0, 7.0, 0.6, 0.03, 1.0, 8.0], // Asphalt (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Cardboard (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Carpet (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // CeramicTiles (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // ClayRoofTiles (type 4)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // RoofShingles (type 4)
    [1.0, 0.0, 0.0, 0.0, 1.0, 8.0], // Leather (type 21)
    [1.0, 0.0, 0.0, 0.0, 1.0, 8.0], // Plaster (type 21)
    [1.0, 0.0, 0.0, 0.0, 0.0, 8.0], // Rubber (type 4)
];
pub const DETILE_LEVELS: f32 = 8.0; // Detiled FS: k = noise(uv × noiseScale).x × this; copies at uv + sin((3, 7) × floor(k)) and floor(k) + 1, mixed by fract(k)
pub const DETILE_NOISE: &str = "rbxasset://textures/noise.dds"; // its noise

// Studio 0.746: (index, Use2022Materials), copied by
// to type and PartMaterialConsts. Both 2022
// texture resolutions use this same constructor. Columns: shader type, then
// the 20 floats in PartMaterialConsts.
pub const PART_MATERIAL_FULL_PRE2022: [[f32; 21]; 43] = [
    [0.0, 10.4, 0.6, 0.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Plastic
    [0.0, 0.0, 0.4, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0], // SmoothPlastic
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Snow
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Sandstone
    [2.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Neon
    [4.0, 1.0, 1.0, 0.0, 7.0, 0.6, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Wood
    [6.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // WoodPlanks
    [6.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Marble
    [7.0, 1.0, 1.0, 0.0, 5.0, 1.0, 0.25, 1.0, 1.0, 0.75, 0.0, 0.0, 0.0, 4.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Slate
    [7.0, 1.0, 1.0, 0.0, 10.0, 1.0, 0.25, 1.0, 1.0, 0.75, 0.0, 0.0, 0.0, 4.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Concrete
    [7.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.25, 1.0, 1.0, 0.6, 0.0, 0.0, 0.0, 2.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Granite
    [6.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Brick
    [4.0, 1.0, 1.0, 0.0, 6.0, 1.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Pebble
    [7.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 0.6, 0.0, 0.0, 0.0, 2.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // CorrodedMetal
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // DiamondPlate
    [8.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.25, 1.0, 0.0, 0.75, 0.0, 0.0, 1.0, 4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Foil
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Metal
    [6.0, 0.65, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Grass
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Sand
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Fabric
    [8.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.25, 0.0, 0.0, 0.75, 0.0, 1.0, 1.0, 4.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Ice
    [6.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Cobblestone
    [10.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.25, 0.0, 0.0, 0.75, 0.0, 1.0, 1.0, 4.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Glass
    [12.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.25, 0.0, 0.0, 0.75, 0.0, 1.0, 1.0, 4.0, 1.0, 0.0, 0.0, 0.0, 0.0], // ForceField
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Mud
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.04, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Limestone
    [21.0, 1.0, 1.0, 0.0, 15.0, 0.15, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.05, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Basalt
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.05, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Ground
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.05, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // LeafyGrass
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.045, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Rock
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Pavement
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Glacier
    [9.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.6, 1.0, 1.0, 0.6, 0.0, 0.0, 0.0, 2.5, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 0.0], // CrackedLava
    [7.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.6, 1.0, 1.0, 0.6, 0.0, 0.0, 0.0, 2.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Salt
    [21.0, 1.0, 1.0, 0.0, 7.0, 0.6, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Asphalt
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Cardboard
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Carpet
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // CeramicTiles
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // ClayRoofTiles
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // RoofShingles
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Leather
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Plaster
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Rubber
];
pub const PART_MATERIAL_FULL_2022: [[f32; 21]; 43] = [
    [0.0, 1.0, 0.6, 0.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Plastic
    [0.0, 0.0, 0.4, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0], // SmoothPlastic
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Snow
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Sandstone
    [2.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Neon
    [21.0, 1.0, 1.0, 0.0, 7.0, 0.1, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Wood
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // WoodPlanks
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Marble
    [21.0, 1.0, 1.0, 0.0, 5.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Slate
    [21.0, 1.0, 1.0, 0.0, 10.0, 0.1, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Concrete
    [7.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.25, 1.0, 1.0, 0.6, 0.0, 0.0, 0.0, 2.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Granite
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Brick
    [7.0, 1.0, 1.0, 0.0, 6.0, 1.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Pebble
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // CorrodedMetal
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // DiamondPlate
    [8.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.25, 1.0, 0.0, 0.75, 0.0, 0.0, 1.0, 4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Foil
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Metal
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.02, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Grass
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Sand
    [7.0, 1.0, 1.0, 0.0, 0.1, 0.1, 0.6, 1.0, 1.0, 0.6, 0.0, 0.0, 0.0, 2.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Fabric
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Ice
    [7.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.6, 1.0, 1.0, 0.6, 0.0, 0.0, 0.0, 2.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Cobblestone
    [10.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Glass
    [12.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // ForceField
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Mud
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.04, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Limestone
    [21.0, 1.0, 1.0, 0.0, 15.0, 0.15, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.05, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Basalt
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.05, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Ground
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.05, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // LeafyGrass
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.045, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Rock
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Pavement
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Glacier
    [9.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.6, 1.0, 1.0, 0.6, 0.0, 0.0, 0.0, 2.5, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 0.0], // CrackedLava
    [7.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.6, 1.0, 1.0, 0.6, 0.0, 0.0, 0.0, 2.5, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Salt
    [21.0, 1.0, 1.0, 0.0, 7.0, 0.6, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.03, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Asphalt
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Cardboard
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Carpet
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // CeramicTiles
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // ClayRoofTiles
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // RoofShingles
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Leather
    [21.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Plaster
    [4.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], // Rubber
];
pub const WANG_INDEX: &str = "rbxasset://textures/wangIndex.dds";
pub const WANG_CELLS: f32 = 4.0; // WangFS: uv*4, lookup nearest at floor(uv*4) modulo 128; RG index *255/256
pub const MATERIAL_DIFFUSE_FADE_DEPTH: f32 = 300.0; // all LQ and HQ HideDiffuse/HideAll variants
pub const MATERIAL_LQ_DIFFUSE_CUTOFF: f32 = 0.6; // LQ hardcodes this instead of farDiffuseCutoff

// ; indexed by pointer FNV bits 3..8 / 9..14.
pub const MATERIAL_UV_RANDOM: [f32; 64] = [
    0.8273230195045471, 0.526485025882721, 0.9162539839744568, 0.22542600333690643,
    0.969743013381958, 0.06947600096464157, 0.42392799258232117, 0.794975996017456,
    0.6293249726295471, 0.7071089744567871, 0.9970800280570984, 0.3712979853153229,
    0.5371220111846924, 0.6921870112419128, 0.30915001034736633, 0.20761099457740784,
    0.3472990095615387, 0.6802690029144287, 0.854915976524353, 0.32154199481010437,
    0.8480449914932251, 0.8254410028457642, 0.6998199820518494, 0.365231990814209,
    0.391757994890213, 0.9821310043334961, 0.9611260294914246, 0.8547160029411316,
    0.8471310138702393, 0.42798200249671936, 0.9016919732093811, 0.7058209776878357,
    0.23047299683094025, 0.36752399802207947, 0.32439300417900085, 0.7975710034370422,
    0.04854999855160713, 0.24529600143432617, 0.8050749897956848, 0.48486700654029846,
    0.7406690120697021, 0.37504398822784424, 0.792493999004364, 0.27567800879478455,
    0.9159319996833801, 0.37617799639701843, 0.6258789896965027, 0.6489499807357788,
    0.21351100504398346, 0.4549669921398163, 0.17218099534511566, 0.7345719933509827,
    0.3393700122833252, 0.28091299533843994, 0.053203001618385315, 0.0018820000113919377,
    0.8266639709472656, 0.5510219931602478, 0.833666980266571, 0.9876109957695007,
    0.1083500012755394, 0.569212019443512, 0.9478449821472168, 0.20134299993515015,
];
pub const MATERIAL_UV_FNV_OFFSET: u64 = 0xcbf29ce484222325;
pub const MATERIAL_UV_FNV_PRIME: u64 = 0x100000001b3;

// Studio Running/Freefall force solvers.
// Velocity error gain getter ->.
pub const HUMANOID_VELOCITY_GAIN: f32 = 150.0;
// Freefall horizontal acceleration cap.
pub const HUMANOID_AIR_ACCEL: f32 = 143.0;

// Instance collector (src/runtime.rs Runtime::collect, src/play.rs collect_garbage).
pub const GC_INTERVAL: f64 = 0.5; // seconds of Play between collector passes
pub const GC_BUDGET: usize = 20_000; // unparented instances looked at per pass and side
pub const GC_CYCLE_PASSES: usize = 2; // collector passes a Luau GC cycle is stepped over (each steps heap / this KB; keeps weak instance values clearing on quiet scripts)
pub const GC_STEP_MIN_KB: usize = 256; // least Luau GC work per collector pass
pub const GC_STEP_MAX_KB: usize = 16 * 1024; // most Luau GC work per collector pass (bounds the pass's time on huge heaps)
pub const GC_FRAME_MAX_KB: f64 = 200.0; // most Luau GC work in one frame: Studio's FInt LuaGcMaxKb (200; its per-frame LuauGc job clamps to it)
pub const GC_IDLE_CYCLE_S: f64 = 8.0; // seconds a background Luau GC cycle is spread over while no new unparented instance shows up (only ones scripts still hold)
pub const GC_BUSY_ORPHANS: usize = 64; // unparented instances arriving (or freed) between two collector passes from which the Luau GC is stepped at the fast rate
pub const HUMANOID_COYOTE_S: f32 = 0.1; // a jump still takes off this long after walking off a ledge (Studio via an offline Studio, 2026-11-09: always within 0.04 s, sometimes to 0.10 s, never from 0.11 s, at WalkSpeed 16 and 8; Running turns Freefall ~0.11 s after the floor goes)
