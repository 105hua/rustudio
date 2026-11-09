//! Layered clothing: Accessories whose Handle has a WrapLayer, fitted onto
//! the character's WrapTarget cages.
//!
//! - Each body MeshPart's WrapTarget names its outer cage mesh, placed by
//!   CageOrigin in the part and stretched like the part's mesh (Size over the
//!   mesh's own extent).
//! - A WrapLayer names the garment's inner cage (ReferenceMeshContent, at
//!   ReferenceOrigin) and outer cage (CageMeshContent, at CageOrigin).
//!   Cages are cut from Roblox's one cage template, so a garment's inner cage
//!   vertex and a body cage vertex are the same point of the template when
//!   their UVs are equal: that pairs them.
//! - Layers go on in Order (lowest first): the first fits the body's cages,
//!   each next one the previous layer's fitted outer cage.
//! - Every garment vertex is bound to its nearest inner-cage vertices
//!   (inverse-square distance weights, 4 of them); fitted, it moves with
//!   them, and its height above the inner cage is multiplied by Puffiness.
//! - The fitted garment is cut by body part (each triangle goes with the
//!   part its vertices' cage points belong to) and drawn with those parts,
//!   so it follows their animation.
//!
//! Fitting runs on a background thread once per set of garments; the
//! cutting into parts uses the pose the character has when it starts.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use glam::{Vec2, Vec3};
use rbx_dom_weak::types::{Ref, Variant};

use crate::doc::Document;
use crate::math::Xform;
use crate::scene::{content_string, f32_of, vec3_of, xform_of};

/// A garment piece drawn with one body part, in that part's space.
#[derive(Clone)]
pub struct Piece {
    pub part: Ref,
    /// The garment's Handle (its texture and colour).
    pub handle: Ref,
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
}

/// Everything fitted on one character.
pub struct Fitted {
    pub pieces: Vec<Piece>,
    /// Garment vertices that found their inner cage point on the target by
    /// UV, out of all inner-cage vertices (per layer, in Order).
    pub matched: Vec<(usize, usize)>,
}

struct Target {
    part: Ref,
    xf: Xform,
    size: Vec3,
    mesh: String,
    cage: String,
    origin: Xform,
}

struct Layer {
    handle: Ref,
    xf: Xform,
    size: Vec3,
    mesh: String,
    inner: String,
    inner_origin: Xform,
    outer: String,
    outer_origin: Xform,
    order: i32,
    puffiness: f32,
}

struct Job {
    targets: Vec<Target>,
    layers: Vec<Layer>,
}

#[derive(Clone)]
enum State {
    Pending,
    Ready(Arc<Fitted>),
    Failed(String),
}

fn states() -> &'static Mutex<HashMap<String, State>> {
    static S: OnceLock<Mutex<HashMap<String, State>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Changes whenever a fitting finished.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

pub enum Lookup {
    /// No layered clothing on this character.
    None,
    Pending,
    Ready(Arc<Fitted>),
    Failed(String),
}

fn child_of_class(doc: &Document, parent: Ref, class: &str) -> Option<Ref> {
    doc.dom
        .get_by_ref(parent)?
        .children()
        .iter()
        .copied()
        .find(|c| doc.class_of(*c).is_some_and(|k| k == class))
}

fn cf(doc: &Document, r: Ref, key: &str) -> Xform {
    match doc.prop(r, key) {
        Some(Variant::CFrame(c)) => Xform::from_cframe(&c),
        _ => Xform {
            pos: Vec3::ZERO,
            rot: glam::Mat3::IDENTITY,
        },
    }
}

fn content(doc: &Document, r: Ref, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| content_string(doc.prop(r, k)))
        .filter(|s| !s.is_empty())
}

/// Whether a part is a layered garment's Handle (drawn through its
/// character's fitting, not on its own).
pub fn is_layered_handle(doc: &Document, part: Ref) -> bool {
    child_of_class(doc, part, "WrapLayer")
        .is_some_and(|w| !matches!(doc.prop(w, "Enabled"), Some(Variant::Bool(false))))
        && doc
            .dom
            .get_by_ref(part)
            .and_then(|p| doc.dom.get_by_ref(p.parent()))
            .and_then(|acc| doc.dom.get_by_ref(acc.parent()))
            .is_some_and(|m| {
                m.children()
                    .iter()
                    .any(|c| doc.class_of(*c).is_some_and(|k| k == "Humanoid"))
            })
}

/// The fitted layered clothing of the character `model` (queued on first ask).
pub fn fitted(doc: &Document, model: Ref) -> Lookup {
    let Some(m) = doc.dom.get_by_ref(model) else {
        return Lookup::None;
    };
    let mut targets = Vec::new();
    let mut layers = Vec::new();
    for c in m.children() {
        let Some(class) = doc.class_of(*c) else {
            continue;
        };
        if class == "MeshPart" {
            if let Some(w) = child_of_class(doc, *c, "WrapTarget") {
                let (Some(mesh), Some(cage), Some(xf)) = (
                    content(doc, *c, &["MeshContent", "MeshId"]),
                    content(doc, w, &["CageMeshContent", "CageMeshId"]),
                    xform_of(doc, *c),
                ) else {
                    continue;
                };
                targets.push(Target {
                    part: *c,
                    xf,
                    size: vec3_of(doc.prop(*c, "Size")).unwrap_or(Vec3::ONE),
                    mesh,
                    cage,
                    origin: cf(doc, w, "CageOrigin"),
                });
            }
        } else if class == "Accessory" {
            let Some(h) = crate::default_scripts::child_named(doc, *c, "Handle") else {
                continue;
            };
            if !is_layered_handle(doc, h) {
                continue;
            }
            let Some(w) = child_of_class(doc, h, "WrapLayer") else {
                continue;
            };
            let (Some(mesh), Some(inner), Some(outer), Some(xf)) = (
                content(doc, h, &["MeshContent", "MeshId"]),
                content(doc, w, &["ReferenceMeshContent", "ReferenceMeshId"]),
                content(doc, w, &["CageMeshContent", "CageMeshId"]),
                xform_of(doc, h),
            ) else {
                continue;
            };
            layers.push(Layer {
                handle: h,
                xf,
                size: vec3_of(doc.prop(h, "Size")).unwrap_or(Vec3::ONE),
                mesh,
                inner,
                inner_origin: cf(doc, w, "ReferenceOrigin"),
                outer,
                outer_origin: cf(doc, w, "CageOrigin"),
                order: match doc.prop(w, "Order") {
                    Some(Variant::Int32(o)) => o,
                    Some(Variant::Int64(o)) => o as i32,
                    _ => 1,
                },
                // Studio clamps Puffiness to [-1, 1] in its setter (:
                // maxss .rdata  = -1, minss  = 1); default 1
                // (FInt WrapPuffinessDefaultPercent = 100).
                puffiness: f32_of(doc.prop(w, "Puffiness"))
                    .unwrap_or(crate::mech::WRAP_PUFFINESS_DEFAULT)
                    .clamp(
                        crate::mech::WRAP_PUFFINESS_MIN,
                        crate::mech::WRAP_PUFFINESS_MAX,
                    ),
            });
        }
    }
    if layers.is_empty() || targets.is_empty() {
        return Lookup::None;
    }
    layers.sort_by_key(|l| l.order);
    let key = format!(
        "{model:?}|{}|{}",
        targets
            .iter()
            .map(|t| format!("{:?}:{}:{}", t.part, t.mesh, t.cage))
            .collect::<Vec<_>>()
            .join(","),
        layers
            .iter()
            .map(|l| format!(
                "{:?}:{}:{}:{}:{}:{}",
                l.handle, l.mesh, l.inner, l.outer, l.order, l.puffiness
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut s = states().lock().unwrap_or_else(|e| e.into_inner());
    match s.get(&key) {
        Some(State::Ready(f)) => return Lookup::Ready(f.clone()),
        Some(State::Pending) => return Lookup::Pending,
        Some(State::Failed(e)) => return Lookup::Failed(e.clone()),
        None => {}
    }
    s.insert(key.clone(), State::Pending);
    drop(s);
    let job = Job { targets, layers };
    std::thread::Builder::new()
        .name("layered clothing".into())
        .spawn(move || {
            let st = match fit(&job) {
                Ok(f) => State::Ready(Arc::new(f)),
                Err(e) => State::Failed(e),
            };
            states()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key, st);
            GENERATION.fetch_add(1, Ordering::Relaxed);
        })
        .ok();
    Lookup::Pending
}

fn mesh(id: &str) -> Result<crate::mesh::MeshData, String> {
    let b = crate::assets::content_blocking(id)?;
    crate::mesh::parse(&b).map_err(|e| format!("{id}: {e}"))
}

/// A cage's distinct vertices (position, uv), welded from the triangle list.
fn cage_points(m: &crate::mesh::MeshData) -> Vec<(Vec3, Vec2)> {
    let mut seen = HashMap::new();
    let mut out = Vec::new();
    for (p, uv) in m.positions.iter().zip(&m.uvs) {
        let k = (
            (p.x * 1e4) as i64,
            (p.y * 1e4) as i64,
            (p.z * 1e4) as i64,
            (uv.x * 1e5) as i64,
            (uv.y * 1e5) as i64,
        );
        if seen.insert(k, ()).is_none() {
            out.push((*p, *uv));
        }
    }
    out
}

/// Mesh-space point → world, the way a MeshPart stretches its mesh: the
/// part's Size over the render mesh's own extent (`fit`), then `origin` in
/// the part, then the part.
fn place(xf: &Xform, origin: &Xform, fit: Vec3, p: Vec3) -> Vec3 {
    xf.apply(origin.apply(p * fit))
}

fn fit_scale(size: Vec3, m: &crate::mesh::MeshData) -> Vec3 {
    size / (m.max - m.min).max(Vec3::splat(1e-4))
}

/// UV key for pairing template vertices.
fn uv_key(uv: Vec2) -> (i64, i64) {
    ((uv.x * 1e4).round() as i64, (uv.y * 1e4).round() as i64)
}

/// A cage the next layer fits onto: world positions, uvs, owning part.
struct Surface {
    points: Vec<Vec3>,
    uvs: Vec<Vec2>,
    parts: Vec<usize>,
}

fn nearest_k(points: &[Vec3], p: Vec3, k: usize) -> Vec<(usize, f32)> {
    let mut best: Vec<(usize, f32)> = Vec::with_capacity(k + 1);
    for (i, q) in points.iter().enumerate() {
        let d = q.distance_squared(p);
        if best.len() < k || d < best[best.len() - 1].1 {
            let at = best.partition_point(|b| b.1 <= d);
            best.insert(at, (i, d));
            best.truncate(k);
        }
    }
    best
}

fn fit(job: &Job) -> Result<Fitted, String> {
    // Body cages in world space.
    let mut surface = Surface {
        points: Vec::new(),
        uvs: Vec::new(),
        parts: Vec::new(),
    };
    let parts: Vec<(Ref, Xform)> = job.targets.iter().map(|t| (t.part, t.xf)).collect();
    for (ti, t) in job.targets.iter().enumerate() {
        let render = mesh(&t.mesh)?;
        let cage = mesh(&t.cage)?;
        let s = fit_scale(t.size, &render);
        for (p, uv) in cage_points(&cage) {
            surface.points.push(place(&t.xf, &t.origin, s, p));
            surface.uvs.push(uv);
            surface.parts.push(ti);
        }
    }
    let mut pieces = Vec::new();
    let mut matched = Vec::new();
    for l in &job.layers {
        let render = mesh(&l.mesh)?;
        let inner = mesh(&l.inner)?;
        let outer = mesh(&l.outer)?;
        let s = fit_scale(l.size, &render);
        let by_uv: HashMap<(i64, i64), usize> = surface
            .uvs
            .iter()
            .enumerate()
            .map(|(i, uv)| (uv_key(*uv), i))
            .collect();
        // Inner cage: where it is, and where its template point is on the target.
        let inner_pts = cage_points(&inner);
        let mut rest = Vec::with_capacity(inner_pts.len());
        let mut goal = Vec::with_capacity(inner_pts.len());
        let mut owner = Vec::with_capacity(inner_pts.len());
        let mut hits = 0;
        for (p, uv) in &inner_pts {
            let at = place(&l.xf, &l.inner_origin, s, *p);
            let j = match by_uv.get(&uv_key(*uv)) {
                Some(j) => {
                    hits += 1;
                    *j
                }
                // Not on the template: the nearest target point by UV.
                None => surface
                    .uvs
                    .iter()
                    .enumerate()
                    .min_by(|a, b| {
                        a.1.distance_squared(*uv)
                            .total_cmp(&b.1.distance_squared(*uv))
                    })
                    .map(|(j, _)| j)
                    .unwrap_or(0),
            };
            rest.push(at);
            goal.push(surface.points.get(j).copied().unwrap_or(at));
            owner.push(surface.parts.get(j).copied().unwrap_or(0));
        }
        matched.push((hits, inner_pts.len()));
        if rest.is_empty() {
            continue;
        }
        // Fit a point bound to the inner cage.
        let fit_point = |p: Vec3| -> (Vec3, usize) {
            let near = nearest_k(&rest, p, 4);
            let ws: Vec<f32> = near.iter().map(|(_, d)| 1.0 / (d + 1e-6)).collect();
            let total: f32 = ws.iter().sum();
            let (mut base, mut to) = (Vec3::ZERO, Vec3::ZERO);
            for ((i, _), w) in near.iter().zip(&ws) {
                base += rest[*i] * (w / total);
                to += goal[*i] * (w / total);
            }
            (to + (p - base) * l.puffiness, owner[near[0].0])
        };
        // The garment itself, cut by body part.
        let mut by_part: HashMap<usize, Piece> = HashMap::new();
        for tri in 0..render.positions.len() / 3 {
            let mut fitted = [Vec3::ZERO; 3];
            let mut owners = [0usize; 3];
            for k in 0..3 {
                let i = tri * 3 + k;
                // The render mesh is drawn centred on its bounds.
                let p =
                    l.xf.apply((render.positions[i] - (render.min + render.max) * 0.5) * s);
                (fitted[k], owners[k]) = fit_point(p);
            }
            let o = if owners[1] == owners[2] {
                owners[1]
            } else {
                owners[0]
            };
            let (part, pxf) = parts[o];
            let inv = pxf.inverse();
            let piece = by_part.entry(o).or_insert_with(|| Piece {
                part,
                handle: l.handle,
                positions: Vec::new(),
                normals: Vec::new(),
                uvs: Vec::new(),
            });
            for k in 0..3 {
                let i = tri * 3 + k;
                piece.positions.push(inv.apply(fitted[k]));
                piece
                    .normals
                    .push(inv.rot * (l.xf.rot * render.normals.get(i).copied().unwrap_or(Vec3::Y)));
                piece
                    .uvs
                    .push(render.uvs.get(i).copied().unwrap_or_default());
            }
        }
        pieces.extend(by_part.into_values());
        // This layer's outer cage, fitted, is what the next layer wears.
        let mut next = Surface {
            points: Vec::new(),
            uvs: Vec::new(),
            parts: Vec::new(),
        };
        for (p, uv) in cage_points(&outer) {
            let (q, o) = fit_point(place(&l.xf, &l.outer_origin, s, p));
            next.points.push(q);
            next.uvs.push(uv);
            next.parts.push(o);
        }
        if !next.points.is_empty() {
            surface = next;
        }
    }
    Ok(Fitted { pieces, matched })
}
