//! GUI drawing and input: ScreenGui, SurfaceGui and BillboardGui with their
//! GuiObjects (Frame, TextLabel/Button/Box, ImageLabel/Button, ScrollingFrame,
//! CanvasGroup, ViewportFrame) and modifiers (UIListLayout, UIGridLayout,
//! UIPadding, UIScale, UIAspectRatioConstraint, UISizeConstraint,
//! UITextSizeConstraint, UICorner, UIGradient, UIStroke), following the public
//! Roblox docs and measured against Roblox Studio (offline Studio; see docs/MECHANICS.md).
//!
//! Every GUI is laid out in its own canvas (pixels), turned into egui shapes,
//! tessellated, clipped in canvas space, and only then mapped to the screen:
//! straight for ScreenGuis, through the 3D camera for SurfaceGuis (so a
//! tilted panel is drawn in perspective) and BillboardGuis. Fonts are Studio's
//! own font files from the content folder (fonts/families/*.json).
//!
//! gui_walk.rs lays out and draws one GUI; gui_input.rs places GUIs on the
//! screen and handles the mouse and keyboard.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, FontFamily, Pos2, Rect, TextureHandle};
use glam::Vec3;
use rbx_dom_weak::types::{Ref, Variant};

use crate::assets::{AssetStore, Fetch};
use crate::camera::Camera;
use crate::doc::{self, Document};
use crate::scene::{enum_u32, f32_of};

#[path = "gui_input.rs"]
mod input;
#[path = "gui_walk.rs"]
mod walk;
#[path = "gui_fx.rs"]
mod fx;
#[path = "gui_adorn.rs"]
mod adorn;

/// Where a GuiObject ended up (the values scripts read as AbsolutePosition /
/// AbsoluteSize / AbsoluteRotation), plus its text's size (TextBounds).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GuiBox {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub rot: f32,
    pub text: Option<[f32; 2]>,
    /// A ScrollingFrame's canvas (CanvasSize / AutomaticCanvasSize), pixels.
    pub canvas: Option<[f32; 2]>,
}

/// What the viewport's GUI and the client's scripts tell each other between
/// frames (src/domains/gui_input.rs): which TextBox has the keyboard, what
/// is hovered / pressed (GuiObject.GuiState), which images have loaded
/// (ImageLabel.IsLoaded), and the focus changes each side asked for. One
/// play client per process, so one of these.
#[derive(Default)]
pub struct GuiShare {
    /// The TextBox typing goes to (both sides keep this up to date).
    pub focus: Option<Ref>,
    /// Scripts' CaptureFocus (Some) / ReleaseFocus (None) for the viewport.
    pub focus_req: Vec<Option<Ref>>,
    /// The player's clicks / Enter / Esc changing focus, for the scripts to
    /// fire Focused / FocusLost in Studio's order: (box, gained, enter
    /// pressed, the mouse button and GUI position that caused it).
    pub focus_events: Vec<(Ref, bool, bool, Option<(u8, f32, f32)>)>,
    pub hovered: HashSet<Ref>,
    pub pressed: Option<Ref>,
    /// Image content ids decoded for drawing.
    pub loaded: HashSet<String>,
    /// Mouse button presses / releases over the view this frame, in order:
    /// (button, down, a GUI object took it).
    pub mouse_edges: Vec<(u8, bool, bool)>,
    /// Mouse buttons held over the view (1, 2, 3).
    pub buttons: [bool; 3],
    /// Mouse drags of Draggable objects / UIDragDetectors for the scripts:
    /// (object, 0 begin / 1 move / 2 end, GUI x, y).
    pub drags: Vec<(Ref, u8, f32, f32)>,
    /// The 3D view's texture and where it was drawn this frame (UIBlur).
    pub scene: Option<(egui::TextureId, Rect)>,
    /// ParticleEmitter2D:Emit(n) / Clear() from the scripts (n = 0: clear).
    pub p2d: Vec<(Ref, u32)>,
    /// GuiService's inspect menu (game_menu.rs draws it): None closed.
    pub inspect: Option<InspectView>,
    /// The player closed it (back arrow / Esc) since the scripts last looked.
    pub inspect_closed: bool,
    /// UIPageLayout scroll: (page it leaves, page it goes to, tween alpha).
    pub pages: HashMap<Ref, (Ref, Ref, f32)>,
}

/// What the inspect menu shows: "<name>'s Avatar", the Robux total of
/// what's for sale, and the worn items (name, price or None off sale);
/// `loading` until the lookups come back.
#[derive(Clone, Debug, Default)]
pub struct InspectView {
    pub title: String,
    pub robux: i64,
    pub items: Vec<(String, Option<i64>)>,
    pub loading: bool,
    pub scroll: f32,
}

pub fn share() -> std::sync::MutexGuard<'static, GuiShare> {
    static S: std::sync::OnceLock<std::sync::Mutex<GuiShare>> = std::sync::OnceLock::new();
    S.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Something for the game to hear about (fired on the client).
#[derive(Debug, Clone)]
pub enum GuiEvent {
    /// Instance, event name, arguments.
    Fire(Ref, &'static str, Vec<GuiArg>),
    /// A TextBox's text typed by the player.
    SetText(Ref, String),
    /// GuiService.SelectedObject moved by keyboard navigation.
    Select(Option<Ref>),
    /// ScrollingFrame.CanvasPosition moved by the wheel (pixels).
    Scroll(Ref, [f32; 2]),
}

#[derive(Debug, Clone)]
pub enum GuiArg {
    /// An InputObject: mouse button (1/2, 0 = none), began, position.
    Input {
        button: u8,
        began: bool,
        x: f32,
        y: f32,
    },
    #[allow(dead_code)]
    Bool(bool),
    Num(f64),
}

/// An interactive object, in screen space, in draw order.
#[derive(Debug, Clone)]
struct Hit {
    r: Ref,
    quad: [Pos2; 4],
    clip: Option<[Pos2; 4]>,
    button: bool,
    textbox: bool,
    /// Sinks clicks (Active, buttons, text boxes).
    sink: bool,
    scrolling: bool,
    drag: bool,
    painted: bool,
}

enum Img {
    Ready(TextureHandle, [usize; 2]),
    Failed,
}

/// GUI state kept between frames (textures, fonts, hover, focus).
#[derive(Default)]
pub struct Gui {
    /// SurfaceGui / BillboardGui triangles (AlwaysOnTop false) for the
    /// renderer's depth-tested pass, by egui texture (round 23).
    pub world_batches: Vec<crate::world_gui::GuiBatch>,
    vf_gpu: Option<crate::gui_viewport::Gpu>,
    vf_targets: HashMap<Ref, crate::gui_viewport::Target>,
    images: HashMap<String, Img>,
    /// EditableImage textures (runtime/editable.rs) by id without the
    /// generation: the generation shown (updated in place when it moves on).
    editable_gens: HashMap<String, u64>,
    /// Font file → egui family name, once added to egui.
    fonts: HashMap<String, String>,
    fonts_failed: HashSet<String>,
    /// egui font size per Roblox TextSize, by family.
    em_ratio: HashMap<String, f32>,
    /// TextScaled sizes: (text, family, box w, box h, wrapped) → size.
    scaled: HashMap<(String, String, i32, i32, bool), f32>,
    /// ViewportFrame meshes: fit mode + content id → centered triangles.
    vf_meshes: HashMap<String, Option<Arc<Vec<[(Vec3, glam::Vec2, Vec3); 3]>>>>,
    hits: Vec<Hit>,
    /// Layout of everything drawn last frame. Kept from frame to frame: a
    /// GUI walked again replaces its own entries (`layout_from`), one no
    /// longer drawn takes them away; `layout_changed` tells the game.
    pub layout: HashMap<Ref, GuiBox>,
    layout_from: HashMap<Ref, Vec<Ref>>,
    layout_roots: Vec<Ref>,
    layout_changed: bool,
    hovered: HashSet<Ref>,
    pressed: Option<(Ref, u8)>,
    /// The object being dragged with the left button.
    dragging: Option<Ref>,
    /// ParticleEmitter2D particles (gui_fx.rs).
    p2d: HashMap<Ref, fx::Emitter>,
    p2d_clock: Option<std::time::Instant>,
    p2d_dt: f32,
    pub focus: Option<Ref>,
    pub events: Vec<GuiEvent>,
    /// The last button clicked (the self-test reads it).
    pub last_click: Option<Ref>,
    /// Shapes drawn last frame (the self-test counts them).
    pub drawn: usize,
    /// SurfaceGuis / BillboardGuis under Workspace, in tree order (kept by
    /// `sync_roots`), for the document `roots_doc`.
    world_roots: Vec<Ref>,
    world_roots_set: HashSet<Ref>,
    /// Handle adornments under Workspace (gui_adorn.rs).
    adorn_roots: HashSet<Ref>,
    /// Adornments under the LocalPlayer's PlayerGui (`adorn_pg_of`), kept
    /// from the journal; their tree order once asked for (None: stale).
    adorn_pg: HashSet<Ref>,
    adorn_pg_of: Option<Ref>,
    adorn_pg_order: Option<Vec<Ref>>,
    roots_doc: u64,
    /// face_file per (family, weight, italic), with when it was looked up.
    faces: HashMap<(String, u32, bool), (Option<String>, std::time::Instant)>,
    /// Each GUI root's layout from its last walk (for frames it is skipped).
    root_boxes: HashMap<Ref, Vec<(Ref, GuiBox)>>,
    /// ScreenGuis drawn as they were last frame (gui_input.rs `RootCache`).
    cache: HashMap<Ref, input::RootCache>,
    /// What a cached ScreenGui's walk read → that ScreenGui: instances whose
    /// properties it read (`cache_owner`) and whose children it listed
    /// (`cache_kids`); per ScreenGui, both lists, to drop them again. A
    /// hidden menu's own changes (Place E animates its hidden Index menu every
    /// frame) touch nothing a walk read, so they leave the cache alone.
    /// (`cache_owner`: read instance → its GUI and the object whose walk
    /// read it.)
    cache_owner: rustc_hash::FxHashMap<Ref, (Ref, Ref, u32)>,
    /// Bumped per `cache_own`: which walk last read an instance.
    own_gen: u32,
    cache_kids: rustc_hash::FxHashMap<Ref, Ref>,
    cache_owned: HashMap<Ref, (Vec<(Ref, Ref)>, Vec<Ref>)>,
    /// The current walk's reads (see `cache_owner`): (instance, reader).
    walk_reads: Vec<(Ref, Ref)>,
    walk_kids: Vec<Ref>,
    /// Each drawn GuiObject's output from its last walk (gui_walk.rs
    /// `Memo`), replayed while its inputs are the same and no mark touched
    /// it or anything under it: Studio's per-node LayoutDirty /
    /// ViewportDirty bits.
    memo: rustc_hash::FxHashMap<Ref, walk::Memo>,
    memo_frames: u32,
    /// RUSTUDIO_GUI_CACHE_VERIFY's second walk: no memo is used.
    memo_off: bool,
    /// This frame's objects drawn from memos / walked (bench).
    memo_hits: u64,
    memo_walks: u64,
    /// What the cached outputs were drawn with besides the document.
    cache_with: input::CacheWith,
    /// Set while walking when something drawn depends on more than the
    /// document (an image still loading, a caret, particles, the 3D view
    /// behind a UIBlur…): that GUI is walked again next frame.
    no_cache: Option<&'static str>,
    /// RUSTUDIO_GUI_CACHE_WHY: the mark that dropped a kept GUI.
    cache_why: HashMap<Ref, String>,
    /// ViewportFrames the current walk drew.
    walk_vfs: Vec<Ref>,
}

/// Everything a draw needs from the editor.
pub struct DrawIn<'a> {
    pub render_state: Option<&'a egui_wgpu::RenderState>,
    pub doc: &'a Document,
    pub cam: &'a Camera,
    pub rect: Rect,
    pub store: &'a mut AssetStore,
    pub doc_dir: Option<&'a std::path::Path>,
    /// Play (the LocalPlayer's PlayerGui) or edit (StarterGui).
    pub playing: bool,
}

// ------------------------------------------------------------------ helpers

fn udim2(d: &Document, r: Ref, name: &str) -> [f32; 4] {
    match d.prop(r, name) {
        Some(Variant::UDim2(u)) => [u.x.scale, u.x.offset as f32, u.y.scale, u.y.offset as f32],
        _ => [0.0; 4],
    }
}

fn udim(d: &Document, r: Ref, name: &str) -> [f32; 2] {
    match d.prop(r, name) {
        Some(Variant::UDim(u)) => [u.scale, u.offset as f32],
        _ => [0.0; 2],
    }
}

fn vec2(d: &Document, r: Ref, name: &str, default: [f32; 2]) -> [f32; 2] {
    match d.prop(r, name) {
        Some(Variant::Vector2(v)) => [v.x, v.y],
        _ => default,
    }
}

fn color(d: &Document, r: Ref, name: &str, default: [f32; 3]) -> [f32; 3] {
    match d.prop(r, name) {
        Some(Variant::Color3(c)) => [c.r, c.g, c.b],
        Some(Variant::Color3uint8(c)) => {
            [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0]
        }
        _ => default,
    }
}

fn num(d: &Document, r: Ref, name: &str, default: f32) -> f32 {
    f32_of(d.prop(r, name)).unwrap_or(default)
}

fn boolp(d: &Document, r: Ref, name: &str, default: bool) -> bool {
    match d.prop(r, name) {
        Some(Variant::Bool(b)) => b,
        _ => default,
    }
}

fn text_prop(d: &Document, r: Ref, name: &str) -> String {
    match d.prop(r, name) {
        Some(Variant::String(s)) => s,
        Some(Variant::ContentId(c)) => c.as_str().to_string(),
        Some(Variant::Content(c)) => c.as_uri().unwrap_or("").to_string(),
        _ => String::new(),
    }
}

fn rgba(c: [f32; 3], alpha: f32) -> Color32 {
    let to = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(to(c[0]), to(c[1]), to(c[2]), to(alpha))
}

fn children(d: &Document, r: Ref) -> Vec<Ref> {
    d.dom
        .get_by_ref(r)
        .map(|i| i.children().to_vec())
        .unwrap_or_default()
}

/// The first enabled child of a class (UI modifiers).
fn child_of_class(d: &Document, r: Ref, class: &str) -> Option<Ref> {
    children(d, r)
        .into_iter()
        .find(|c| d.class_of(*c).is_some_and(|k| k == class) && boolp(d, *c, "Enabled", true))
}

fn name_of(d: &Document, r: Ref) -> String {
    d.dom
        .get_by_ref(r)
        .map(|i| i.name.clone())
        .unwrap_or_default()
}

/// Gradient colour and transparency at t (0..1) along a UIGradient.
fn sample_gradient(d: &Document, g: Ref, t: f32) -> ([f32; 3], f32) {
    let t = t.clamp(0.0, 1.0);
    let c = match d.prop(g, "Color") {
        Some(Variant::ColorSequence(seq)) if !seq.keypoints.is_empty() => {
            let k = &seq.keypoints;
            let mut out = [k[0].color.r, k[0].color.g, k[0].color.b];
            for w in k.windows(2) {
                if t >= w[0].time && t <= w[1].time {
                    let f = if w[1].time > w[0].time {
                        (t - w[0].time) / (w[1].time - w[0].time)
                    } else {
                        0.0
                    };
                    out = [
                        w[0].color.r + (w[1].color.r - w[0].color.r) * f,
                        w[0].color.g + (w[1].color.g - w[0].color.g) * f,
                        w[0].color.b + (w[1].color.b - w[0].color.b) * f,
                    ];
                }
            }
            if t > k[k.len() - 1].time {
                let l = &k[k.len() - 1].color;
                out = [l.r, l.g, l.b];
            }
            out
        }
        _ => [1.0, 1.0, 1.0],
    };
    let a = match d.prop(g, "Transparency") {
        Some(Variant::NumberSequence(seq)) if !seq.keypoints.is_empty() => {
            let k = &seq.keypoints;
            let mut out = k[0].value;
            for w in k.windows(2) {
                if t >= w[0].time && t <= w[1].time {
                    let f = if w[1].time > w[0].time {
                        (t - w[0].time) / (w[1].time - w[0].time)
                    } else {
                        0.0
                    };
                    out = w[0].value + (w[1].value - w[0].value) * f;
                }
            }
            if t > k[k.len() - 1].time {
                out = k[k.len() - 1].value;
            }
            out
        }
        _ => 0.0,
    };
    (c, a)
}

/// A UIGradient over a rect: point → (colour, transparency).
struct Grad<'a> {
    d: &'a Document,
    g: Ref,
    rect: Rect,
    dir: egui::Vec2,
    offset: [f32; 2],
}

impl Grad<'_> {
    fn new(d: &Document, g: Ref, rect: Rect) -> Grad<'_> {
        let rot = num(d, g, "Rotation", 0.0).to_radians();
        Grad {
            d,
            g,
            rect,
            dir: egui::vec2(rot.cos(), rot.sin()),
            offset: vec2(d, g, "Offset", [0.0, 0.0]),
        }
    }
    fn at(&self, p: Pos2) -> ([f32; 3], f32) {
        // Position in the unit square around the centre, minus Offset, along the direction.
        let u = (p.x - self.rect.center().x) / self.rect.width().max(1e-3) - self.offset[0];
        let v = (p.y - self.rect.center().y) / self.rect.height().max(1e-3) - self.offset[1];
        sample_gradient(self.d, self.g, u * self.dir.x + v * self.dir.y + 0.5)
    }
}

/// A rounded rectangle's outline (clockwise from the top-left corner).
fn rounded(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius
        .min(rect.width() * 0.5)
        .min(rect.height() * 0.5)
        .max(0.0);
    if r < 0.5 {
        return vec![
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
    }
    let n = ((r * 0.6) as usize).clamp(3, 12);
    let mut pts = Vec::with_capacity(4 * (n + 1));
    for (c, start) in [
        (Pos2::new(rect.left() + r, rect.top() + r), 180.0f32),
        (Pos2::new(rect.right() - r, rect.top() + r), 270.0),
        (Pos2::new(rect.right() - r, rect.bottom() - r), 0.0),
        (Pos2::new(rect.left() + r, rect.bottom() - r), 90.0),
    ] {
        for k in 0..=n {
            let a = (start + 90.0 * k as f32 / n as f32).to_radians();
            pts.push(Pos2::new(c.x + r * a.cos(), c.y + r * a.sin()));
        }
    }
    pts
}

/// A filled convex polygon as a mesh, coloured per point.
fn fill_mesh(pts: &[Pos2], col: &dyn Fn(Pos2) -> Color32) -> Mesh {
    let mut m = Mesh::default();
    if pts.len() < 3 {
        return m;
    }
    let c =
        (pts.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / pts.len() as f32).to_pos2();
    m.vertices.push(Vertex {
        pos: c,
        uv: egui::epaint::WHITE_UV,
        color: col(c),
    });
    for p in pts {
        m.vertices.push(Vertex {
            pos: *p,
            uv: egui::epaint::WHITE_UV,
            color: col(*p),
        });
    }
    let n = pts.len() as u32;
    for i in 0..n {
        m.indices.extend_from_slice(&[0, 1 + i, 1 + (i + 1) % n]);
    }
    m
}

/// A quad split into a grid (so gradients and perspective look right).
/// A textured quad with rounded corners (an image under a UICorner): the
/// rounded outline as a fan, each point's UV placed by its position.
fn rounded_quad_mesh(
    rect: Rect,
    uv: Rect,
    tex: egui::TextureId,
    col: &dyn Fn(Pos2) -> Color32,
    radius: f32,
) -> Mesh {
    let mut m = fill_mesh(&rounded(rect, radius), col);
    m.texture_id = tex;
    for v in &mut m.vertices {
        let fx = (v.pos.x - rect.left()) / rect.width().max(1e-6);
        let fy = (v.pos.y - rect.top()) / rect.height().max(1e-6);
        v.uv = Pos2::new(uv.left() + uv.width() * fx, uv.top() + uv.height() * fy);
    }
    m
}

fn quad_mesh(
    rect: Rect,
    uv: Rect,
    tex: egui::TextureId,
    col: &dyn Fn(Pos2) -> Color32,
    steps: usize,
) -> Mesh {
    let mut m = Mesh::with_texture(tex);
    let n = steps.max(1);
    for j in 0..=n {
        for i in 0..=n {
            let (fx, fy) = (i as f32 / n as f32, j as f32 / n as f32);
            let p = Pos2::new(
                rect.left() + rect.width() * fx,
                rect.top() + rect.height() * fy,
            );
            let t = Pos2::new(uv.left() + uv.width() * fx, uv.top() + uv.height() * fy);
            m.vertices.push(Vertex {
                pos: p,
                uv: t,
                color: col(p),
            });
        }
    }
    let w = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * w + i;
            m.indices
                .extend_from_slice(&[a, a + 1, a + w, a + 1, a + w + 1, a + w]);
        }
    }
    m
}

fn lerp_vertex(a: &Vertex, b: &Vertex, t: f32) -> Vertex {
    let c = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Vertex {
        pos: a.pos + (b.pos - a.pos) * t,
        uv: a.uv + (b.uv - a.uv) * t,
        color: Color32::from_rgba_premultiplied(
            c(a.color.r(), b.color.r()),
            c(a.color.g(), b.color.g()),
            c(a.color.b(), b.color.b()),
            c(a.color.a(), b.color.a()),
        ),
    }
}

/// Clips a mesh's triangles to a rect (Sutherland–Hodgman).
fn clip_mesh(m: &Mesh, clip: Rect) -> Mesh {
    if m.vertices.iter().all(|v| clip.contains(v.pos)) {
        return m.clone();
    }
    let mut out = Mesh::with_texture(m.texture_id);
    for tri in m.indices.chunks_exact(3) {
        let mut poly: Vec<Vertex> = tri
            .iter()
            .filter_map(|i| m.vertices.get(*i as usize).copied())
            .collect();
        if poly.len() < 3 {
            continue;
        }
        for (axis, val, ge) in [
            (0, clip.left(), true),
            (0, clip.right(), false),
            (1, clip.top(), true),
            (1, clip.bottom(), false),
        ] {
            if poly.is_empty() {
                break;
            }
            let get = |v: &Vertex| if axis == 0 { v.pos.x } else { v.pos.y };
            let ins = |v: &Vertex| if ge { get(v) >= val } else { get(v) <= val };
            let mut next = Vec::with_capacity(poly.len() + 2);
            for k in 0..poly.len() {
                let a = &poly[k];
                let b = &poly[(k + 1) % poly.len()];
                if ins(a) {
                    next.push(*a);
                }
                if ins(a) != ins(b) {
                    next.push(lerp_vertex(a, b, (val - get(a)) / (get(b) - get(a))));
                }
            }
            poly = next;
        }
        if poly.len() >= 3 {
            let base = out.vertices.len() as u32;
            let n = poly.len() as u32;
            out.vertices.extend(poly);
            for k in 1..n - 1 {
                out.indices
                    .extend_from_slice(&[base, base + k, base + k + 1]);
            }
        }
    }
    out
}

/// Splits a SurfaceGui mesh (canvas space) where perspective would bend
/// its colours or texture: `screen` gives a canvas point's screen position
/// and clip-space w. Projection keeps straight edges straight, so a triangle
/// with one colour and one uv is exact as it is; any other is split until
/// screen-space interpolation is within `err` pixels of perspective-correct
/// (about edge × (w_max / w_min − 1) / 4), at most 5 times. Pieces that
/// fall wholly past one edge of `view` (screen) are dropped. A mesh whose
/// whole screen extent passes that test (no triangle in it can fail it)
/// comes back as it is.
fn subdivide(
    m: Mesh,
    err: f32,
    view: Rect,
    screen: &dyn Fn(Pos2) -> Option<(Pos2, f32)>,
) -> Mesh {
    {
        let (mut b, mut wlo, mut whi) = (Rect::NOTHING, f32::INFINITY, 0.0f32);
        let mut known = true;
        for v in &m.vertices {
            match screen(v.pos) {
                Some((p, w)) => {
                    b.extend_with(p);
                    wlo = wlo.min(w);
                    whi = whi.max(w);
                }
                None => {
                    known = false;
                    break;
                }
            }
        }
        // (Behind the camera in part: the caller drops the mesh anyway.)
        if !known || m.vertices.is_empty() || b.size().length() * (whi / wlo - 1.0) * 0.25 <= err {
            return m;
        }
    }
    let flat = |a: &Vertex, b: &Vertex, c: &Vertex| {
        a.uv == b.uv && a.uv == c.uv && a.color == b.color && a.color == c.color
    };
    let tri_flat = |t: &[u32]| match (
        m.vertices.get(t[0] as usize),
        m.vertices.get(t[1] as usize),
        m.vertices.get(t[2] as usize),
    ) {
        (Some(a), Some(b), Some(c)) => flat(a, b, c),
        _ => true,
    };
    if m.indices.chunks_exact(3).all(tri_flat) {
        return m;
    }
    let mut out = Mesh::with_texture(m.texture_id);
    let mut stack: Vec<([Vertex; 3], u8)> = Vec::new();
    for tri in m.indices.chunks_exact(3) {
        let (Some(a), Some(b), Some(c)) = (
            m.vertices.get(tri[0] as usize),
            m.vertices.get(tri[1] as usize),
            m.vertices.get(tri[2] as usize),
        ) else {
            continue;
        };
        stack.push(([*a, *b, *c], 0));
    }
    while let Some(([a, b, c], depth)) = stack.pop() {
        let on_screen = (screen(a.pos), screen(b.pos), screen(c.pos));
        if let (Some((pa, _)), Some((pb, _)), Some((pc, _))) = on_screen {
            let past = |f: &dyn Fn(Pos2) -> bool| f(pa) && f(pb) && f(pc);
            if past(&|p| p.x < view.min.x)
                || past(&|p| p.x > view.max.x)
                || past(&|p| p.y < view.min.y)
                || past(&|p| p.y > view.max.y)
            {
                continue;
            }
        }
        let done = depth >= 5
            || flat(&a, &b, &c)
            || match on_screen {
                (Some((pa, wa)), Some((pb, wb)), Some((pc, wc))) => {
                    let long = (pa - pb).length().max((pb - pc).length()).max((pc - pa).length());
                    let ratio = wa.max(wb).max(wc) / wa.min(wb).min(wc);
                    long * (ratio - 1.0) * 0.25 <= err
                }
                // Behind the camera: the caller drops the mesh anyway.
                _ => true,
            };
        if done {
            let base = out.vertices.len() as u32;
            out.vertices.extend_from_slice(&[a, b, c]);
            out.indices.extend_from_slice(&[base, base + 1, base + 2]);
            continue;
        }
        let (ab, bc, ca) = (
            lerp_vertex(&a, &b, 0.5),
            lerp_vertex(&b, &c, 0.5),
            lerp_vertex(&c, &a, 0.5),
        );
        for t in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            stack.push((t, depth + 1));
        }
    }
    out
}

fn point_in_quad(q: &[Pos2; 4], p: Pos2) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let c = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if c.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

// ------------------------------------------------------------------ fonts

/// Enum.Font → (family file, weight, italic), as Font.fromEnum maps them
/// (Gotham is Montserrat in current Studio's content folder).
fn legacy_font(name: &str) -> (String, u32, bool) {
    let f = |fam: &str, w: u32, i: bool| (format!("rbxasset://fonts/families/{fam}.json"), w, i);
    match name {
        "Legacy" => f("LegacyArial", 400, false),
        "Arial" => f("Arimo", 400, false),
        "ArialBold" | "ArimoBold" => f("Arimo", 700, false),
        "SourceSans" => f("SourceSansPro", 400, false),
        "SourceSansBold" => f("SourceSansPro", 700, false),
        "SourceSansSemibold" => f("SourceSansPro", 600, false),
        "SourceSansLight" => f("SourceSansPro", 300, false),
        "SourceSansItalic" => f("SourceSansPro", 400, true),
        "Bodoni" => f("AccanthisADFStd", 400, false),
        "Garamond" => f("Guru", 400, false),
        "Cartoon" => f("ComicNeueAngular", 400, false),
        "Code" => f("Inconsolata", 400, false),
        "Highway" => f("HighwayGothic", 400, false),
        "SciFi" => f("Zekton", 400, false),
        "Arcade" => f("PressStart2P", 400, false),
        "Fantasy" => f("Balthazar", 400, false),
        "Antique" => f("RomanAntique", 400, false),
        "Gotham" => f("Montserrat", 400, false),
        "GothamMedium" | "GothamSemibold" => f("Montserrat", 500, false),
        "GothamBold" => f("Montserrat", 700, false),
        "GothamBlack" => f("Montserrat", 900, false),
        "BuilderSans" => f("BuilderSans", 400, false),
        "BuilderSansMedium" => f("BuilderSans", 500, false),
        "BuilderSansBold" => f("BuilderSans", 700, false),
        "BuilderSansExtraBold" => f("BuilderSans", 800, false),
        other => f(other, 400, false),
    }
}

fn weight_of(w: rbx_dom_weak::types::FontWeight) -> u32 {
    use rbx_dom_weak::types::FontWeight as W;
    match w {
        W::Thin => 100,
        W::ExtraLight => 200,
        W::Light => 300,
        W::Regular => 400,
        W::Medium => 500,
        W::SemiBold => 600,
        W::Bold => 700,
        W::ExtraBold => 800,
        W::Heavy => 900,
        _ => 400,
    }
}

/// A text object's font: family file, weight, italic. FontFace wins when set;
/// a script that set the legacy Font enum gets that one.
fn font_of(d: &Document, r: Ref) -> (String, u32, bool) {
    let inst = d.dom.get_by_ref(r);
    let has = |n: &str| inst.is_some_and(|i| i.properties.contains_key(&rbx_dom_weak::ustr(n)));
    let face = || match d.prop(r, "FontFace") {
        Some(Variant::Font(f)) => Some((
            f.family.clone(),
            weight_of(f.weight),
            matches!(f.style, rbx_dom_weak::types::FontStyle::Italic),
        )),
        _ => None,
    };
    let legacy = || {
        let v = enum_u32(d.prop(r, "Font"))?;
        let name = doc::db()
            .enums
            .get("Font")?
            .items
            .iter()
            .find(|(_, val)| **val == v)
            .map(|(n, _)| n.to_string())?;
        Some(legacy_font(&name))
    };
    if has("FontFace") {
        if let Some(f) = face() {
            return f;
        }
    }
    if has("Font") {
        if let Some(f) = legacy() {
            return f;
        }
    }
    face()
        .or_else(legacy)
        .unwrap_or_else(|| legacy_font("SourceSans"))
}

/// The face file for a family/weight/style: the exact face if its file is
/// here, else the nearest weight (same style first) that is.
fn face_file(store: &mut AssetStore, family: &str, weight: u32, italic: bool) -> Option<String> {
    static FAMILIES: std::sync::Mutex<Option<HashMap<String, Vec<(u32, bool, String)>>>> =
        std::sync::Mutex::new(None);
    let mut g = FAMILIES.lock().ok()?;
    let map = g.get_or_insert_with(HashMap::new);
    if !map.contains_key(family) {
        let faces = match store.content(family, None) {
            Fetch::Ready(b) => serde_json::from_slice::<serde_json::Value>(&b)
                .ok()
                .and_then(|j| {
                    Some(
                        j.get("faces")?
                            .as_array()?
                            .iter()
                            .filter_map(|f| {
                                Some((
                                    f.get("weight")?.as_u64()? as u32,
                                    f.get("style")?.as_str()? == "italic",
                                    f.get("assetId")?.as_str()?.to_string(),
                                ))
                            })
                            .collect::<Vec<_>>(),
                    )
                }),
            _ => None,
        }
        .unwrap_or_default();
        map.insert(family.to_string(), faces);
    }
    let available = |id: &str| -> bool {
        if id.starts_with("rbxasset://") {
            crate::assets::resolve(id, None).is_some_and(|p| p.is_file())
        } else {
            crate::assets::asset_ref(id).is_some_and(|a| {
                crate::assets::local_asset_path(a.id).is_some()
                    || crate::assets::cache_path(a).is_file()
            })
        }
    };
    let mut best: Option<(u32, &String)> = None;
    for (w, it, id) in map.get(family)? {
        if !available(id) {
            continue;
        }
        let cost = w.abs_diff(weight) + if *it != italic { 1000 } else { 0 };
        if best.is_none_or(|(c, _)| cost < c) {
            best = Some((cost, id));
        }
    }
    best.map(|(_, id)| id.clone())
}

/// Fonts for glyphs a game font lacks, in order: Studio's Builder Sans and
/// Source Sans Pro, its Noto UI script faces (content/fonts), then broad
/// symbol fonts installed on the system (Noto Sans Symbols 1 / 2, DejaVu Sans).
pub(crate) fn fallback_fonts() -> &'static [&'static [u8]] {
    static FONTS: std::sync::OnceLock<Vec<&'static [u8]>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let content = crate::assets::content_dir().join("fonts");
        let mut files: Vec<std::path::PathBuf> = ["BuilderSans-Regular.otf", "SourceSansPro-Regular.ttf"]
            .iter()
            .map(|f| content.join(f))
            .collect();
        let mut scripts: Vec<std::path::PathBuf> = std::fs::read_dir(&content)
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("NotoSans") || n.starts_with("NotoNaskh"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        scripts.sort();
        files.extend(scripts);
        files.extend(
            [
                "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
                "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            ]
            .map(std::path::PathBuf::from),
        );
        files
            .iter()
            .filter_map(|p| std::fs::read(p).ok())
            .map(|b| &*Box::leak(b.into_boxed_slice()))
            .collect()
    })
}

impl Gui {
    /// The egui family for a font (Studio's file for the nearest available
    /// face); egui's default font until the file has been added (next frame).
    fn family(
        &mut self,
        ctx: &egui::Context,
        store: &mut AssetStore,
        font: &(String, u32, bool),
    ) -> FontFamily {
        // face_file stats every face's file: once per face, not per text
        // object per frame (60 µs each, most of Place B's GUI walk). A face
        // not here yet is looked for again a second later.
        let key = font.clone();
        let path = match self.faces.get(&key) {
            Some((p, at)) if p.is_some() || at.elapsed().as_secs_f32() < 1.0 => p.clone(),
            _ => {
                let p = face_file(store, &font.0, font.1, font.2);
                self.faces.insert(key, (p.clone(), std::time::Instant::now()));
                p
            }
        };
        let Some(path) = path else {
            // (looked for again in a second)
            self.no_cache = Some("font face missing");
            return FontFamily::Proportional;
        };
        if let Some(name) = self.fonts.get(&path) {
            let fam = FontFamily::Name(name.clone().into());
            let ready = ctx.fonts(|f| f.definitions().families.contains_key(&fam));
            if !ready { self.no_cache = Some("font not ready"); }
            return if ready { fam } else { FontFamily::Proportional };
        }
        if self.fonts_failed.contains(&path) {
            return FontFamily::Proportional;
        }
        let bytes = match store.content(&path, None) {
            Fetch::Ready(b) => b,
            Fetch::Pending => {
                self.no_cache = Some("font loading");
                return FontFamily::Proportional;
            }
            Fetch::Unavailable(_) => {
                self.fonts_failed.insert(path);
                return FontFamily::Proportional;
            }
        };
        let name = format!("rbx:{path}");
        let family = FontFamily::Name(name.clone().into());
        ctx.add_font(egui::epaint::text::FontInsert::new(
            &name,
            egui::FontData::from_owned(bytes.to_vec()),
            vec![egui::epaint::text::InsertFontFamily {
                family: family.clone(),
                priority: egui::epaint::text::FontPriority::Highest,
            }],
        ));
        // Glyphs the face lacks ("▶" in Sarpanch) come from the fallback
        // fonts after it, as Roblox's text falls back, not a missing-glyph box.
        // (egui adds a font name once, so each family gets its own names for
        // the shared, never-freed bytes.)
        for (i, bytes) in fallback_fonts().iter().enumerate() {
            ctx.add_font(egui::epaint::text::FontInsert::new(
                &format!("{name}+fallback{i}"),
                egui::FontData::from_static(bytes),
                vec![egui::epaint::text::InsertFontFamily {
                    family: family.clone(),
                    priority: egui::epaint::text::FontPriority::Lowest,
                }],
            ));
        }
        self.fonts.insert(path, name);
        // (the face is used from the next frame)
        self.no_cache = Some("font added");
        FontFamily::Proportional
    }

    /// A game's mouse cursor image (UserInputService.MouseIcon / Mouse.Icon)
    /// drawn centred on the pointer at its own pixel size, as Roblox does.
    /// False while the image isn't loaded (the system cursor stays then).
    pub fn draw_cursor(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        store: &mut AssetStore,
        doc_dir: Option<&std::path::Path>,
        id: &str,
        at: Pos2,
    ) -> bool {
        let Some((tex, size)) = self.texture(ctx, store, doc_dir, id) else {
            return false;
        };
        let s = egui::vec2(size[0] as f32, size[1] as f32);
        painter.image(
            tex,
            Rect::from_center_size(at, s),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        true
    }

    /// Whether an image went through `texture` (GUI images and, since
    /// round 26, effect textures): loaded, failed, or never asked for (None).
    pub fn image_state(&self, id: &str) -> Option<Result<(), String>> {
        match self.images.get(id)? {
            Img::Ready(..) => Some(Ok(())),
            Img::Failed => Some(Err("not decoded or unavailable".into())),
        }
    }

    /// An image as an egui texture (and its pixel size), loaded on first use.
    pub(crate) fn texture(
        &mut self,
        ctx: &egui::Context,
        store: &mut AssetStore,
        doc_dir: Option<&std::path::Path>,
        id: &str,
    ) -> Option<(egui::TextureId, [usize; 2])> {
        if let Some((base, gen)) = crate::runtime::editable::image_key(id) {
            // (EditableImage pixels change without a document mark)
            self.no_cache = Some("editable image");
            if self.editable_gens.get(base).is_some_and(|h| *h >= gen) {
                if let Some(Img::Ready(h, s)) = self.images.get(base) {
                    return Some((h.id(), *s));
                }
            }
            let (now, img) = crate::runtime::editable::image_pixels(id)?;
            let size = [img.width as usize, img.height as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, &img.pixels);
            let opts = egui::TextureOptions::NEAREST;
            let out = match self.images.get_mut(base) {
                Some(Img::Ready(h, s)) => {
                    h.set(color, opts);
                    *s = size;
                    (h.id(), size)
                }
                _ => {
                    let h = ctx.load_texture(format!("gui:{base}"), color, opts);
                    let out = (h.id(), size);
                    self.images.insert(base.to_string(), Img::Ready(h, size));
                    out
                }
            };
            self.editable_gens.insert(base.to_string(), now.max(gen));
            return Some(out);
        }
        if let Some(i) = self.images.get(id) {
            return match i {
                Img::Ready(h, s) => Some((h.id(), *s)),
                Img::Failed => None,
            };
        }
        let mut bytes = match store.content(id, doc_dir) {
            Fetch::Ready(b) => b,
            Fetch::Pending => {
                self.no_cache = Some("image loading");
                return None;
            }
            Fetch::Unavailable(_) => {
                self.images.insert(id.to_string(), Img::Failed);
                return None;
            }
        };
        // A Decal asset (XML) points at its image.
        if bytes.starts_with(b"<roblox") {
            let inner = Document::decode(&bytes).ok().and_then(|dom| {
                dom.descendants().find_map(|i| {
                    match i.properties.get(&rbx_dom_weak::ustr("Texture")) {
                        Some(Variant::ContentId(c)) => Some(c.as_str().to_string()),
                        Some(Variant::Content(c)) => c.as_uri().map(|s| s.to_string()),
                        _ => None,
                    }
                })
            });
            match inner.map(|i| store.content(&i, doc_dir)) {
                Some(Fetch::Ready(b)) => bytes = b,
                Some(Fetch::Pending) => {
                    self.no_cache = Some("decal image loading");
                    return None;
                }
                _ => {
                    self.images.insert(id.to_string(), Img::Failed);
                    return None;
                }
            }
        }
        match crate::texture::decode(&bytes) {
            Ok(img) => {
                let size = [img.width as usize, img.height as usize];
                let h = ctx.load_texture(
                    format!("gui:{id}"),
                    egui::ColorImage::from_rgba_unmultiplied(size, &img.pixels),
                    egui::TextureOptions::LINEAR,
                );
                let out = (h.id(), size);
                self.images.insert(id.to_string(), Img::Ready(h, size));
                share().loaded.insert(id.to_string());
                Some(out)
            }
            Err(_) => {
                self.images.insert(id.to_string(), Img::Failed);
                None
            }
        }
    }

    /// Authored UVs with centered positions; fitted meshes use unit-box positions,
    /// file SpecialMeshes preserve their authored dimensions before Scale.
    fn vf_mesh(
        &mut self,
        store: &mut AssetStore,
        doc_dir: Option<&std::path::Path>,
        id: &str,
        fit_to_part: bool,
    ) -> Option<Arc<Vec<[(Vec3, glam::Vec2, Vec3); 3]>>> {
        let cache_key = format!("{fit_to_part}|{id}");
        if let Some(m) = self.vf_meshes.get(&cache_key) {
            return m.clone();
        }
        let loaded = match store.content(id, doc_dir) {
            Fetch::Ready(b) => crate::mesh::parse(&b).ok().map(|m| {
                let ext = if fit_to_part { (m.max - m.min).max(Vec3::splat(1e-4)) } else { Vec3::ONE };
                let c = (m.max + m.min) * 0.5;
                let n = m.positions.len() / 3;
                Arc::new(
                    (0..n)
                        .map(|t| {
                            [
                                ((m.positions[t * 3] - c) / ext, m.uvs.get(t * 3).copied().unwrap_or_default(), m.normals.get(t * 3).copied().unwrap_or(Vec3::Y) * ext),
                                ((m.positions[t * 3 + 1] - c) / ext, m.uvs.get(t * 3 + 1).copied().unwrap_or_default(), m.normals.get(t * 3 + 1).copied().unwrap_or(Vec3::Y) * ext),
                                ((m.positions[t * 3 + 2] - c) / ext, m.uvs.get(t * 3 + 2).copied().unwrap_or_default(), m.normals.get(t * 3 + 2).copied().unwrap_or(Vec3::Y) * ext),
                            ]
                        })
                        .collect::<Vec<_>>(),
                )
            }),
            Fetch::Pending => return None,
            Fetch::Unavailable(_) => None,
        };
        self.vf_meshes.insert(cache_key, loaded.clone());
        loaded
    }
}

// ------------------------------------------------------------------ rich text

/// One run of rich text.
struct Run {
    text: String,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    color: Option<[f32; 3]>,
    size: Option<f32>,
    transparency: Option<f32>,
}

fn parse_color(s: &str) -> Option<[f32; 3]> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let v = u32::from_str_radix(h, 16).ok()?;
        return Some([
            ((v >> 16) & 255) as f32 / 255.0,
            ((v >> 8) & 255) as f32 / 255.0,
            (v & 255) as f32 / 255.0,
        ]);
    }
    let inner = s.strip_prefix("rgb(").and_then(|x| x.strip_suffix(')'))?;
    let v: Vec<f32> = inner
        .split(',')
        .filter_map(|x| x.trim().parse().ok())
        .collect();
    (v.len() == 3).then(|| [v[0] / 255.0, v[1] / 255.0, v[2] / 255.0])
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let i = tag.find(&format!("{name}="))?;
    let rest = &tag[i + name.len() + 1..];
    let q = rest.chars().next()?;
    if q == '"' || q == '\'' {
        let end = rest[1..].find(q)?;
        Some(rest[1..1 + end].to_string())
    } else {
        Some(
            rest.split_whitespace()
                .next()?
                .trim_end_matches('>')
                .to_string(),
        )
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Roblox RichText: <b> <i> <u> <s> <font color size transparency weight>
/// <br/> <uppercase>/<uc> <smallcaps>/<sc> (as uppercase) and the XML escapes.
fn rich_runs(src: &str) -> Vec<Run> {
    #[derive(Clone, Default)]
    struct St {
        b: bool,
        i: bool,
        u: bool,
        s: bool,
        upper: bool,
        color: Option<[f32; 3]>,
        size: Option<f32>,
        tr: Option<f32>,
    }
    let mut stack: Vec<(String, St)> = Vec::new();
    let mut st = St::default();
    let mut out = Vec::new();
    let mut rest = src;
    let push = |out: &mut Vec<Run>, text: &str, st: &St| {
        if text.is_empty() {
            return;
        }
        let t = unescape(text);
        out.push(Run {
            text: if st.upper { t.to_uppercase() } else { t },
            bold: st.b,
            italic: st.i,
            underline: st.u,
            strike: st.s,
            color: st.color,
            size: st.size,
            transparency: st.tr,
        });
    };
    while let Some(lt) = rest.find('<') {
        push(&mut out, &rest[..lt], &st);
        let Some(gt) = rest[lt..].find('>') else {
            push(&mut out, &rest[lt..], &st);
            return out;
        };
        let tag = &rest[lt + 1..lt + gt];
        rest = &rest[lt + gt + 1..];
        let lower = tag.trim().to_ascii_lowercase();
        if lower == "br" || lower == "br/" || lower == "br /" {
            push(&mut out, "\n", &st);
            continue;
        }
        if let Some(name) = lower.strip_prefix('/') {
            if let Some(pos) = stack.iter().rposition(|(n, _)| n == name.trim()) {
                st = stack[pos].1.clone();
                stack.truncate(pos);
            }
            continue;
        }
        let name = lower
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string();
        let prev = st.clone();
        match name.as_str() {
            "b" => st.b = true,
            "i" => st.i = true,
            "u" => st.u = true,
            "s" => st.s = true,
            "uppercase" | "uc" | "smallcaps" | "sc" => st.upper = true,
            "font" => {
                if let Some(c) = attr(tag, "color").and_then(|c| parse_color(&c)) {
                    st.color = Some(c);
                }
                if let Some(s) = attr(tag, "size").and_then(|s| s.parse().ok()) {
                    st.size = Some(s);
                }
                if let Some(t) = attr(tag, "transparency").and_then(|s| s.parse().ok()) {
                    st.tr = Some(t);
                }
                if attr(tag, "weight").is_some_and(|w| {
                    w.eq_ignore_ascii_case("bold") || w.parse::<u32>().is_ok_and(|n| n >= 600)
                }) {
                    st.b = true;
                }
            }
            "stroke" | "mark" | "comment" => {}
            _ => {
                // Not a tag Roblox knows: shown as text.
                push(&mut out, &format!("<{tag}>"), &st);
                continue;
            }
        }
        if !tag.trim_end().ends_with('/') {
            stack.push((name, prev));
        }
    }
    push(&mut out, rest, &st);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_text_runs() {
        let runs = rich_runs(
            "a <b>bold</b> <font color=\"#FF0000\" size=\"20\">red</font><br/>x &lt;y&gt;",
        );
        let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["a ", "bold", " ", "red", "\n", "x <y>"]);
        assert!(runs[1].bold);
        assert_eq!(runs[3].color, Some([1.0, 0.0, 0.0]));
        assert_eq!(runs[3].size, Some(20.0));
    }

    #[test]
    fn clip_keeps_inside() {
        let m = quad_mesh(
            Rect::from_min_max(Pos2::new(-10.0, -10.0), Pos2::new(10.0, 10.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            egui::TextureId::default(),
            &|_| Color32::WHITE,
            1,
        );
        let c = clip_mesh(&m, Rect::from_min_max(Pos2::ZERO, Pos2::new(5.0, 5.0)));
        assert!(c.vertices.iter().all(|v| v.pos.x >= -1e-4
            && v.pos.x <= 5.0001
            && v.pos.y >= -1e-4
            && v.pos.y <= 5.0001));
        assert!(!c.indices.is_empty());
    }

    #[test]
    fn point_in_turned_quad() {
        let q = [
            Pos2::new(0.0, 5.0),
            Pos2::new(5.0, 0.0),
            Pos2::new(10.0, 5.0),
            Pos2::new(5.0, 10.0),
        ];
        assert!(point_in_quad(&q, Pos2::new(5.0, 5.0)));
        assert!(!point_in_quad(&q, Pos2::new(0.5, 0.5)));
    }
}

#[cfg(test)]
mod calibrate {
    /// Prints egui's width for Roblox's measured strings (TextService:GetTextSize
    /// in Studio: "START GAME" Oswald 14 → 46×14, 18 → 58×18, 24 → 78×24, 32 → 104×32).
    #[test]
    fn oswald_width() {
        let path = crate::assets::content_dir().join("fonts/Oswald-Regular.ttf");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let ctx = egui::Context::default();
        let mut defs = egui::FontDefinitions::default();
        defs.font_data.insert(
            "oswald".into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        defs.families.insert(
            egui::FontFamily::Name("oswald".into()),
            vec!["oswald".into()],
        );
        ctx.set_fonts(defs);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for s in [14.0f32, 18.0, 24.0, 32.0] {
                let g = ui.ctx().fonts_mut(|f| {
                    f.layout_job(egui::text::LayoutJob::simple_singleline(
                        "START GAME".into(),
                        egui::FontId::new(s, egui::FontFamily::Name("oswald".into())),
                        egui::Color32::WHITE,
                    ))
                });
                println!("CAL oswald {s}: {:.2} x {:.2}", g.size().x, g.size().y);
            }
        });
        output.textures_delta.clear();
    }
}

/// What RichText leaves of a string once its tags are gone (ContentText).
pub fn content_text(src: &str) -> String {
    rich_runs(src).into_iter().map(|r| r.text).collect()
}
