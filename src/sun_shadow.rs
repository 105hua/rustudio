//! The sun's shadow map as Studio draws it (Realistic / ShadowMap / Future;
//! docs/MECHANICS.md "Sun shadow cascades"): four orthographic cascades of
//! EVSM moments in one atlas. Each cascade's depth is drawn, warped and
//! blurred across (ShadowEvsmHBlurD), then blurred down into the atlas
//! (ShadowEvsmVBlurL); the part and beam shaders pick a cascade by the
//! distance to its sphere and test with Chebyshev on both warps.
//!
//! Measured from a running Studio: its cascade spheres (and sphere radius −
//! blur for the shader), blur width and level, blur passes and their kernel
//! weights, Globals and matrices.

use crate::mech;
use egui_wgpu::wgpu;
use glam::{DVec3, Mat4, Vec3, Vec4};

pub const CASCADES: usize = 4;
/// Uniform bytes per pass (a dynamic offset each).
const PASS_STRIDE: u64 = 256;
const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Kernel tables by blur level − 1 (levels 1..24). Across: the centre weight
/// and one per texel either side (9 floats a row; Studio's horizontal EVSM
/// blur). Down: the centre weight and one per bilinear tap pair at these
/// texel offsets (Studio's vertical EVSM blur).
const EVSM_H_WEIGHTS: [[f32; 9]; 24] = [
    [0.666666687, 0.166666672, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.375, 0.25, 0.0625, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.312805474, 0.234604105, 0.0938416421, 0.0151515203, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.276816607, 0.221453294, 0.11072664, 0.0294117592, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.246575341, 0.205479443, 0.117416829, 0.0440313108, 0.0097847404, 0.0, 0.0, 0.0, 0.0],
    [0.227027029, 0.194594592, 0.121621624, 0.0540540516, 0.0162162203, 0.0, 0.0, 0.0, 0.0],
    [0.209887147, 0.183651254, 0.122434162, 0.0612170808, 0.0222607609, 0.00549317989, 0.0, 0.0, 0.0],
    [0.197277129, 0.175357446, 0.122750208, 0.0669546574, 0.0278977808, 0.00840133987, 0.0, 0.0, 0.0],
    [0.187015116, 0.168313593, 0.122409888, 0.0714057684, 0.0329565108, 0.0114066703, 0.0, 0.0, 0.0],
    [0.176652253, 0.160592958, 0.120444722, 0.0741198286, 0.0370599106, 0.0148239704, 0.00463249022, 0.0, 0.0],
    [0.168921813, 0.154844984, 0.11911153, 0.076571703, 0.0408382416, 0.0178667307, 0.00630589994, 0.0, 0.0],
    [0.162252873, 0.149771884, 0.117677912, 0.0784519389, 0.0441292115, 0.0207666904, 0.00807593018, 0.0, 0.0],
    [0.156444594, 0.14526999, 0.116215989, 0.0798984915, 0.0469991118, 0.0234995596, 0.00989454985, 0.0, 0.0],
    [0.150020525, 0.140019163, 0.113765568, 0.0803051069, 0.049075339, 0.0258291308, 0.0116231097, 0.00437232014, 0.0],
    [0.145248607, 0.136170566, 0.112140462, 0.0809903294, 0.0511517897, 0.0281334799, 0.0133969001, 0.00539217982, 0.0],
    [0.140973747, 0.132681176, 0.110567637, 0.0814708918, 0.0529560782, 0.0302606206, 0.0151303103, 0.00644642022, 0.0],
    [0.137124404, 0.129506379, 0.109058008, 0.0817935094, 0.0545290001, 0.0322216786, 0.0168113094, 0.00751789985, 0.0],
    [0.133642241, 0.126608446, 0.10761717, 0.0819940269, 0.0559050217, 0.0340291411, 0.0184324495, 0.00859261956, 0.0],
    [0.129244164, 0.122781947, 0.105241671, 0.0813231096, 0.0565726012, 0.0353578702, 0.0198004097, 0.00990020018, 0.00440008985],
    [0.126181602, 0.120172963, 0.103785731, 0.081223622, 0.0575333983, 0.0368213691, 0.0212430991, 0.0110149402, 0.00511407992],
    [0.123362191, 0.117754817, 0.102395497, 0.0810630992, 0.0583654307, 0.0381620117, 0.0226145294, 0.01211493, 0.00584858004],
    [0.120758623, 0.115508243, 0.101069711, 0.0808557719, 0.0590869114, 0.0393912718, 0.0239161309, 0.0131951096, 0.00659755012],
    [0.118347518, 0.113416366, 0.0998064131, 0.0806128681, 0.0597132407, 0.0405196995, 0.0251501594, 0.0142517602, 0.00735573983],
    [0.116108812, 0.111464463, 0.0986031666, 0.0803433284, 0.0602574907, 0.0415568911, 0.0263193697, 0.01528221, 0.00811867975],
];
const EVSM_V_WEIGHTS: [[f32; 5]; 24] = [
    [0.666666687, 0.166666672, 0.0, 0.0, 0.0],
    [0.375, 0.3125, 0.0, 0.0, 0.0],
    [0.322580636, 0.338709682, 0.0, 0.0, 0.0],
    [0.294117659, 0.352941185, 0.0, 0.0, 0.0],
    [0.246575341, 0.322896272, 0.0538160503, 0.0, 0.0],
    [0.227027029, 0.31621623, 0.0702702701, 0.0, 0.0],
    [0.212218657, 0.309485525, 0.0844051391, 0.0, 0.0],
    [0.200648561, 0.303202271, 0.096473448, 0.0, 0.0],
    [0.191381156, 0.297510713, 0.106798723, 0.0, 0.0],
    [0.176652253, 0.281037688, 0.111179739, 0.0194564499, 0.0],
    [0.168921813, 0.273956507, 0.117409937, 0.0241726302, 0.0],
    [0.162252873, 0.267449796, 0.122581147, 0.0288426206, 0.0],
    [0.156444594, 0.261485994, 0.126897603, 0.0333941095, 0.0],
    [0.151343986, 0.256023556, 0.130521819, 0.0377826318, 0.0],
    [0.146832094, 0.251018077, 0.133582726, 0.0419831388, 0.0],
    [0.142815024, 0.246425942, 0.136182755, 0.0459837914, 0.0],
    [0.139217645, 0.242206156, 0.13840352, 0.0497815013, 0.0],
    [0.135979071, 0.238321215, 0.140310317, 0.0533789285, 0.0],
    [0.129244164, 0.228023633, 0.137895703, 0.0551582798, 0.0143002998],
    [0.126181602, 0.223958686, 0.138757005, 0.0580644682, 0.0161290206],
    [0.123362191, 0.220150322, 0.139428526, 0.0607765391, 0.01796351],
    [0.120758623, 0.216577947, 0.139942676, 0.0633073971, 0.0197926592],
    [0.118347518, 0.213222787, 0.140326098, 0.0656698495, 0.0216074996],
    [0.116108812, 0.21006763, 0.140600815, 0.067876257, 0.0234008897],
];
const EVSM_V_OFFSETS: [[f32; 4]; 24] = [
    [1.0, 0.0, 0.0, 0.0],
    [1.20000005, 0.0, 0.0, 0.0],
    [1.28571427, 0.0, 0.0, 0.0],
    [1.33333337, 0.0, 0.0, 0.0],
    [1.36363637, 3.18181825, 0.0, 0.0],
    [1.38461542, 3.23076916, 0.0, 0.0],
    [1.39999998, 3.26666665, 0.0, 0.0],
    [1.41176474, 3.29411769, 0.0, 0.0],
    [1.42105258, 3.31578946, 0.0, 0.0],
    [1.42857146, 3.33333325, 5.23809528, 0.0],
    [1.43478262, 3.347826, 5.2608695, 0.0],
    [1.44000006, 3.3599999, 5.28000021, 0.0],
    [1.44444442, 3.37037039, 5.29629612, 0.0],
    [1.4482758, 3.37931037, 5.3103447, 0.0],
    [1.45161295, 3.38709688, 5.32258081, 0.0],
    [1.4545455, 3.3939395, 5.33333349, 0.0],
    [1.45714283, 3.4000001, 5.34285736, 0.0],
    [1.45945942, 3.40540552, 5.35135126, 0.0],
    [1.46153843, 3.41025639, 5.35897446, 7.30769253],
    [1.46341467, 3.41463423, 5.36585379, 7.31707335],
    [1.46511626, 3.41860461, 5.3720932, 7.32558155],
    [1.4666667, 3.42222214, 5.37777758, 7.33333349],
    [1.46808505, 3.42553186, 5.38297892, 7.34042549],
    [1.46938777, 3.42857146, 5.38775492, 7.34693861],
];

/// One cascade: an orthographic box around a sphere, looking down the light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cascade {
    pub centre: Vec3,
    /// The sphere's radius: half the box's width and height.
    pub radius: f32,
    /// The radius the shaders test against (radius − the blur's reach).
    pub test_radius: f32,
    /// Blur level (1..24): the kernel's reach in texels.
    pub level: u32,
    /// Texels across its square tile, and the tile's corner in the atlas.
    pub tile: u32,
    pub origin: [u32; 2],
    /// Light space: x, y of the tile (y up) and the way the light travels.
    pub right: Vec3,
    pub up: Vec3,
    pub travel: Vec3,
}

/// Light space as Studio's look-at builds it: up is +Y, or +Z
/// once the light is steeper than |y| ≥ 0.7.
fn light_basis(travel: Vec3) -> (Vec3, Vec3) {
    let up = if travel.y.abs() >= mech::SUN_CASCADE_UP_FLIP { Vec3::Z } else { Vec3::Y };
    let right = travel.cross(up).normalize_or(Vec3::X);
    (right, right.cross(travel))
}

/// The four cascades for a camera (position, forward, tan of half the
/// horizontal / vertical field of view), the direction to the sun and
/// Lighting.ShadowSoftness.
///
/// As Studio: cascade i spans view depths SPLITS[i]..SPLITS[i+1]; its
/// sphere sits at the far plane's centre and reaches the farthest of the
/// slice's corners. Cascade 0's is then moved to start at the
/// camera; the others snap to a grid of radius / 4 in light space,
/// so they stay put while the camera moves a little.
pub fn cascades(cam: Vec3, forward: Vec3, tan_half: [f32; 2], sun: Vec3, softness: f32) -> [Cascade; CASCADES] {
    let travel = -sun.normalize_or(Vec3::Y);
    let (right, up) = light_basis(travel);
    let blur = softness * mech::SHADOW_SOFTNESS_BLUR[0] + mech::SHADOW_SOFTNESS_BLUR[1];
    let hyp = (tan_half[0] * tan_half[0] + tan_half[1] * tan_half[1]).sqrt();
    let origins = mech::SUN_CASCADE_ORIGINS;
    std::array::from_fn(|i| {
        let (near, far) = (mech::SUN_CASCADE_SPLITS[i], mech::SUN_CASCADE_SPLITS[i + 1]);
        let to_far = far * hyp;
        let to_near = ((near * hyp).powi(2) + (far - near).powi(2)).sqrt();
        let radius = to_far.max(to_near);
        let centre = if i == 0 {
            cam + forward * (radius - blur)
        } else {
            let c = (cam + forward * far).as_dvec3();
            let step = (2.0 / mech::SUN_CASCADE_SNAP * radius) as f64;
            let snap = |axis: Vec3| {
                let a = axis.as_dvec3();
                a * ((c.dot(a) / step).round() * step)
            };
            let s: DVec3 = snap(right) + snap(up) + snap(travel);
            s.as_vec3()
        };
        let tile = mech::SUN_CASCADE_TILES[i];
        // The blur's reach in texels, the blur width through
        // the cascade's projection (1 / radius) times its tile, rounded.
        let level = ((tile as f32 * blur / radius + 0.5) as u32).clamp(1, EVSM_H_WEIGHTS.len() as u32);
        Cascade { centre, radius, test_radius: radius - blur, level, tile, origin: origins[i], right, up, travel }
    })
}

impl Cascade {
    fn matrix(&self, toward_sun: f32, sx: f32, sy: f32, ox: f32, oy: f32) -> Mat4 {
        let r = self.radius;
        let depth = 2.0 * r + toward_sun;
        let x = self.right * (sx / r);
        let y = self.up * (sy / r);
        let z = self.travel / depth;
        let c = self.centre;
        Mat4::from_cols(
            Vec4::new(x.x, y.x, z.x, 0.0),
            Vec4::new(x.y, y.y, z.y, 0.0),
            Vec4::new(x.z, y.z, z.z, 0.0),
            Vec4::new(ox - x.dot(c), oy - y.dot(c), (r + toward_sun) / depth - z.dot(c), 1.0),
        )
    }

    /// World to clip space: the box reaches SUN_CASCADE_PANCAKE studs
    /// toward the sun beyond the sphere (depth 0) and the sphere's far side
    /// is depth 1. Casters nearer the sun are flattened onto depth 0.
    pub fn view_proj(&self) -> Mat4 {
        self.matrix(mech::SUN_CASCADE_PANCAKE, 1.0, 1.0, 0.0, 0.0)
    }

    /// The same box reaching SUN_CASCADE_CASTER_DEPTH toward the sun: what
    /// can cast into the cascade.
    pub fn caster_view_proj(&self) -> Mat4 {
        self.matrix(mech::SUN_CASCADE_CASTER_DEPTH, 1.0, 1.0, 0.0, 0.0)
    }

    /// World to the tile's own (u, v, depth), 0..1 with v down.
    pub fn local(&self) -> Mat4 {
        self.matrix(mech::SUN_CASCADE_PANCAKE, 0.5, -0.5, 0.5, 0.5)
    }

    /// The tile in the atlas: corner and size in UV.
    pub fn rect(&self) -> [f32; 4] {
        let [w, h] = mech::SUN_SHADOW_ATLAS.map(|v| v as f32);
        let t = self.tile as f32;
        [self.origin[0] as f32 / w, self.origin[1] as f32 / h, t / w, t / h]
    }
}

/// What the shaders read (Globals / the beams' constants).
pub fn uniforms(c: &[Cascade; CASCADES]) -> ([[[f32; 4]; 4]; CASCADES], [[f32; 4]; CASCADES], [[f32; 4]; CASCADES]) {
    (
        c.map(|c| c.local().to_cols_array_2d()),
        c.map(|c| c.centre.extend(c.test_radius * c.test_radius).to_array()),
        c.map(|c| c.rect()),
    )
}

const BLUR_WGSL: &str = r#"
struct SunPass {
    vp: mat4x4<f32>,
    rect: vec4<f32>,            // x tile texels, zw the tile's corner in the atlas
    hw: array<vec4<f32>, 3>,    // across: centre weight, then one per texel out
    vw: array<vec4<f32>, 2>,    // down: centre weight, then one per tap pair
    vo: vec4<f32>,              // down: the tap pairs' texel offsets
};
@group(0) @binding(0) var<uniform> u: SunPass;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(1) @binding(0) var moments_tex: texture_2d<f32>;
@group(1) @binding(1) var moments_sampler: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i32(i & 1u) * 4 - 1), f32(i32(i >> 1u) * 4 - 1), 0.0, 1.0);
}

fn moments(px: vec2<i32>) -> vec4<f32> {
    let hi = i32(u.rect.x) - 1;
    let z = 2.0 * textureLoad(depth_tex, clamp(px, vec2<i32>(0), vec2<i32>(hi)), 0) - 1.0;
    let ep = exp(EVSM_POS_EXP * z);
    let en = -exp(-EVSM_NEG_EXP * z);
    return vec4<f32>(ep, ep * ep, en, en * en);
}

// ShadowEvsmHBlurDFS_N: depth to moments, blurred across.
@fragment
fn fs_h(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(floor(p.xy));
    var sum = moments(px) * u.hw[0].x;
    for (var i = 1; i < 9; i++) {
        let w = u.hw[i / 4][i % 4];
        if (w <= 0.0) { break; }
        sum += (moments(px + vec2<i32>(i, 0)) + moments(px - vec2<i32>(i, 0))) * w;
    }
    return sum;
}

// ShadowEvsmVBlurLFS_N: blurred down with bilinear tap pairs, into the atlas.
@fragment
fn fs_v(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(moments_tex));
    let at = p.xy - u.rect.zw;
    let lo = 0.5;
    let hi = u.rect.x - 0.5;
    var sum = textureSampleLevel(moments_tex, moments_sampler, at / size, 0.0) * u.vw[0].x;
    for (var i = 1; i < 5; i++) {
        let w = u.vw[i / 4][i % 4];
        if (w <= 0.0) { break; }
        let o = u.vo[i - 1];
        let a = vec2<f32>(at.x, clamp(at.y + o, lo, hi)) / size;
        let b = vec2<f32>(at.x, clamp(at.y - o, lo, hi)) / size;
        sum += (textureSampleLevel(moments_tex, moments_sampler, a, 0.0) + textureSampleLevel(moments_tex, moments_sampler, b, 0.0)) * w;
    }
    return sum;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PassRaw {
    vp: [[f32; 4]; 4],
    rect: [f32; 4],
    hw: [f32; 12],
    vw: [f32; 8],
    vo: [f32; 4],
}

/// What a cascade's tile was last drawn from.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    cascade: Cascade,
    scene: u64,
}

/// The atlas, its passes and which tiles are current.
pub struct SunShadows {
    pub atlas_view: wgpu::TextureView,
    temp_view: wgpu::TextureView,
    pass_buf: wgpu::Buffer,
    /// Group 1 of the depth pipelines: the cascade's SunPass.
    pub pass_layout: wgpu::BindGroupLayout,
    pass_bg: wgpu::BindGroup,
    h_bg: wgpu::BindGroup,
    v_bg: wgpu::BindGroup,
    h_pipe: wgpu::RenderPipeline,
    v_pipe: wgpu::RenderPipeline,
    drawn: [Option<Drawn>; CASCADES],
    frame: u64,
}

impl SunShadows {
    /// `depth`: the square depth map the cascades are drawn into one at a
    /// time (at least the largest tile across).
    pub fn new(device: &wgpu::Device, depth: &wgpu::TextureView) -> Self {
        let tex = |label: &str, [w, h]: [u32; 2]| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: ATLAS_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let side = mech::SUN_CASCADE_TILES.iter().copied().max().unwrap_or(1);
        let atlas_view = tex("sun shadow atlas", mech::SUN_SHADOW_ATLAS);
        let temp_view = tex("sun shadow blur", [side, side]);
        let pass_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sun shadow passes"),
            size: CASCADES as u64 * PASS_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pass_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<PassRaw>() as u64),
            },
            count: None,
        };
        let pass_binding = wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &pass_buf,
                offset: 0,
                size: wgpu::BufferSize::new(std::mem::size_of::<PassRaw>() as u64),
            }),
        };
        let pass_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun shadow pass"),
            entries: &[pass_entry],
        });
        let pass_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun shadow pass"),
            layout: &pass_layout,
            entries: &[pass_binding.clone()],
        });
        let h_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun shadow blur across"),
            entries: &[
                pass_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let h_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun shadow blur across"),
            layout: &h_layout,
            entries: &[pass_binding, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) }],
        });
        let v_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun shadow blur down"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sun shadow blur"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let v_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun shadow blur down"),
            layout: &v_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&temp_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sun shadow blur"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "const EVSM_POS_EXP: f32 = {:?};\nconst EVSM_NEG_EXP: f32 = {:?};\n{BLUR_WGSL}",
                    mech::EVSM_POS_EXP,
                    mech::EVSM_NEG_EXP
                )
                .into(),
            ),
        });
        let pipe = |label: &str, entry: &str, layouts: &[Option<&wgpu::BindGroupLayout>]| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(ATLAS_FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let h_pipe = pipe("sun shadow blur across", "fs_h", &[Some(&h_layout)]);
        let v_pipe = pipe("sun shadow blur down", "fs_v", &[Some(&pass_layout), Some(&v_layout)]);
        SunShadows { atlas_view, temp_view, pass_buf, pass_layout, pass_bg, h_bg, v_bg, h_pipe, v_pipe, drawn: [None; CASCADES], frame: 0 }
    }

    /// The cascades to redraw this frame. Cascade 0 follows the camera, so
    /// always; the others when they moved or the scene changed (`scene`
    /// counts changes of what stands still). With moving casters
    /// (`moving`) they also come round in turn: one of them a frame,
    /// cascade 1 every 2nd, 2 every 4th, 3 every 8th.
    pub fn plan(&mut self, cascades: &[Cascade; CASCADES], scene: u64, moving: bool) -> [bool; CASCADES] {
        self.frame += 1;
        let f = self.frame;
        std::array::from_fn(|i| {
            let now = Drawn { cascade: cascades[i], scene };
            let turn = moving && [true, f % 2 == 0, f % 4 == 1, f % 8 == 3][i];
            let due = i == 0 || turn || self.drawn[i] != Some(now);
            if due {
                self.drawn[i] = Some(now);
            }
            due
        })
    }

    /// Nothing drawn is current any more (the depth map was lost / shadows off).
    pub fn forget(&mut self) {
        self.drawn = [None; CASCADES];
    }

    /// Uploads the passes' constants; before any of them is encoded.
    pub fn upload(&self, queue: &wgpu::Queue, cascades: &[Cascade; CASCADES]) {
        let mut data = vec![0u8; (CASCADES as u64 * PASS_STRIDE) as usize];
        for (i, c) in cascades.iter().enumerate() {
            let k = (c.level as usize).clamp(1, EVSM_H_WEIGHTS.len()) - 1;
            let mut raw = PassRaw {
                vp: c.view_proj().to_cols_array_2d(),
                rect: [c.tile as f32, 0.0, c.origin[0] as f32, c.origin[1] as f32],
                hw: [0.0; 12],
                vw: [0.0; 8],
                vo: EVSM_V_OFFSETS[k],
            };
            raw.hw[..9].copy_from_slice(&EVSM_H_WEIGHTS[k]);
            raw.vw[..5].copy_from_slice(&EVSM_V_WEIGHTS[k]);
            let o = i * PASS_STRIDE as usize;
            data[o..o + std::mem::size_of::<PassRaw>()].copy_from_slice(bytemuck::bytes_of(&raw));
        }
        queue.write_buffer(&self.pass_buf, 0, &data);
    }

    /// Binds cascade `i`'s constants as group 1 of a depth pass.
    pub fn bind(&self, pass: &mut wgpu::RenderPass, i: usize) {
        pass.set_bind_group(1, &self.pass_bg, &[(i as u64 * PASS_STRIDE) as u32]);
    }

    /// Cascade `i`'s depth (in the depth map's corner) to blurred moments
    /// in its atlas tile.
    pub fn blur(&self, encoder: &mut wgpu::CommandEncoder, c: &Cascade, i: usize) {
        let offset = [(i as u64 * PASS_STRIDE) as u32];
        let t = c.tile as f32;
        fn begin<'a>(encoder: &'a mut wgpu::CommandEncoder, view: &wgpu::TextureView) -> wgpu::RenderPass<'a> {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sun shadow blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
        }
        {
            let mut pass = begin(encoder, &self.temp_view);
            pass.set_viewport(0.0, 0.0, t, t, 0.0, 1.0);
            pass.set_scissor_rect(0, 0, c.tile, c.tile);
            pass.set_pipeline(&self.h_pipe);
            pass.set_bind_group(0, &self.h_bg, &offset);
            pass.draw(0..3, 0..1);
        }
        let mut pass = begin(encoder, &self.atlas_view);
        pass.set_viewport(c.origin[0] as f32, c.origin[1] as f32, t, t, 0.0, 1.0);
        pass.set_scissor_rect(c.origin[0], c.origin[1], c.tile, c.tile);
        pass.set_pipeline(&self.v_pipe);
        pass.set_bind_group(0, &self.pass_bg, &offset);
        pass.set_bind_group(1, &self.v_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A running Studio (a test place, ShadowSoftness 0.2): Globals
    /// spheres of radius 22.322 / 50.716 / 158.609 / 681.042, blur kernels
    /// of levels 7 / 6 / 3 / 1, cascade matrices 109.43 / 166.22 / 382.00 /
    /// 1426.87 studs deep with the sphere centre at depth 0.7924 in the first.
    #[test]
    fn cascades_match_studio() {
        // far corner at 1.13571 × the far plane's distance, as that view had
        let ty = 35f32.to_radians().tan();
        let tx = (1.13571f32 * 1.13571 - ty * ty).sqrt();
        let sun = Vec3::new(-0.474_896, 0.822_544, 0.312_882);
        let cam = Vec3::new(40.0, 150.0, 300.0);
        let c = cascades(cam, Vec3::new(0.9, -0.3, 0.1).normalize(), [tx, ty], sun, 0.2);
        let want_r = [22.322, 50.716, 158.609, 681.042];
        let want_depth = [109.43, 166.22, 382.00, 1426.87];
        for i in 0..CASCADES {
            assert!((c[i].test_radius - want_r[i]).abs() < 0.03, "radius {i}: {}", c[i].test_radius);
            let m = c[i].local();
            let depth = 1.0 / m.row(2).truncate().length();
            assert!((depth - want_depth[i]).abs() < 0.06, "depth {i}: {depth}");
            let at = m.project_point3(c[i].centre);
            assert!((at.x - 0.5).abs() < 1e-3 && (at.y - 0.5).abs() < 1e-3, "centre {i}: {at}");
        }
        assert_eq!(c.map(|c| c.level), [7, 6, 3, 1]);
        assert!((c[0].local().project_point3(c[0].centre).z - 0.79243).abs() < 2e-4);
        // Studio's light space for that sun: tile x along (−0.866, −0.5, 0),
        // tile y (down) along (−0.156, 0.271, −0.95).
        assert!((c[0].right - Vec3::new(-0.866, -0.5, 0.0)).length() < 2e-3);
        assert!((c[0].up - Vec3::new(0.156, -0.271, 0.95)).length() < 2e-3);
        // cascade 0 starts at the camera; the others sit on their grid
        assert!(((c[0].centre - cam).length() - c[0].test_radius).abs() < 1e-3);
        for c in &c[1..] {
            let step = c.radius / 4.0;
            for axis in [c.right, c.up, c.travel] {
                let k = c.centre.dot(axis) / step;
                assert!((k - k.round()).abs() < 2e-3, "snap {k}");
            }
        }
    }

    #[test]
    fn kernels_sum_to_one() {
        for k in 0..EVSM_H_WEIGHTS.len() {
            let h = EVSM_H_WEIGHTS[k][0] + 2.0 * EVSM_H_WEIGHTS[k][1..].iter().sum::<f32>();
            let v = EVSM_V_WEIGHTS[k][0] + 2.0 * EVSM_V_WEIGHTS[k][1..].iter().sum::<f32>();
            assert!((h - 1.0).abs() < 1e-5 && (v - 1.0).abs() < 1e-5, "level {}", k + 1);
        }
    }
}
