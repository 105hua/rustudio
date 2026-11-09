//! EditableImage:DrawImageProjected / SampleImageProjected: a decal
//! projected along a box onto an EditableMesh's UV layout, and the reverse
//! (the mesh's texture seen through the box). Parameter checks and errors
//! are Studio's (strings in Studio; measured: a 1×1 box over
//! a 2×2 quad mapped to an 8×8 image paints its middle 4×4 texels).

use glam::{Vec2, Vec3};

use super::*;

struct Box3 {
    pos: Vec3,
    right: Vec3,
    up: Vec3,
    dir: Vec3,
    size: Vec3,
}

fn need(t: &Value, param: &str, key: &str, who: &str) -> mlua::Result<Value> {
    let v = match t {
        Value::Table(t) => t.get::<Value>(key)?,
        _ => Value::Nil,
    };
    if v.is_nil() {
        // Studio words both calls' checks as DrawImageProjected's.
        let _ = who;
        return Err(rt_err(format!("EditableImage::DrawImageProjected Param {param}, is missing {key}")));
    }
    Ok(v)
}

fn projection(t: &Value, who: &str) -> mlua::Result<Box3> {
    let vec = |k: &str| -> mlua::Result<Vec3> {
        let v = need(t, "ProjectionParams", k, who)?;
        opt_ud::<LVector3>(&v).map(|v| v.0).ok_or_else(|| rt_err(format!("EditableImage::DrawImageProjected Param ProjectionParams, is missing {k}")))
    };
    let dir = vec("Direction")?;
    let pos = vec("Position")?;
    let size = vec("Size")?;
    let up = vec("Up")?;
    if !size.is_finite() {
        return Err(rt_err("Projection size cannot be Nan or Inf in any dimension."));
    }
    if size.cmplt(Vec3::ZERO).any() {
        return Err(rt_err("Projection size cannot be negative in any dimension."));
    }
    if size.cmpeq(Vec3::ZERO).any() {
        return Err(rt_err("Projection size cannot be 0 in any dimension."));
    }
    if !up.is_finite() {
        return Err(rt_err("Projection Up Vector is NaN or Inf"));
    }
    if !dir.is_finite() {
        return Err(rt_err("Projection Direction Vector is NaN or Inf"));
    }
    if up.length_squared() == 0.0 {
        return Err(rt_err("Projection Up Vector cannot be zero"));
    }
    if dir.length_squared() == 0.0 {
        return Err(rt_err("Projection Direction Vector cannot be zero"));
    }
    let dir = dir.normalize();
    let right = dir.cross(up.normalize());
    if right.length_squared() < 1e-10 {
        return Err(rt_err("Projection Up and Direction Vectors cannot be parallel"));
    }
    let right = right.normalize();
    let up = right.cross(dir);
    Ok(Box3 { pos, right, up, dir, size })
}

struct Brush {
    color_mode: u32,
    alpha_mode: u32,
    intensity: f32,
    cos_fade: f32,
}

fn brush(t: &Value, who: &str, decal: bool) -> mlua::Result<(Brush, Option<Ref>)> {
    let d = if decal {
        let v = need(t, "BrushConfig", "Decal", who)?;
        Some(opt_inst(&v).ok_or_else(|| rt_err(format!("{who}: Decal is nil")))?)
    } else {
        None
    };
    let color = enum_val(&need(t, "BrushConfig", "ColorBlendType", who)?).unwrap_or(1);
    let alpha = enum_val(&need(t, "BrushConfig", "AlphaBlendType", who)?).unwrap_or(1);
    let intensity = if decal { num(&need(t, "BrushConfig", "BlendIntensity", who)?).unwrap_or(1.0) as f32 } else { 1.0 };
    let fade = num(&need(t, "BrushConfig", "FadeAngle", who)?).unwrap_or(90.0) as f32;
    if decal && color == 7 {
        return Err(rt_err("EditableImage:DrawImageProjected: Enum.ImageCombineType.Subtract is not yet supported"));
    }
    Ok((Brush { color_mode: color, alpha_mode: alpha, intensity: intensity.clamp(0.0, 1.0), cos_fade: fade.clamp(0.0, 180.0).to_radians().cos() }, d))
}

/// One texel blended with the alpha mode (LockCanvasAlpha keeps the
/// canvas alpha, LockCanvasColor keeps its colour).
fn put(dst: &mut [u8], src: [f32; 4], b: &Brush) {
    let before = [dst[0], dst[1], dst[2], dst[3]];
    blend(dst, src, b.color_mode);
    match b.alpha_mode {
        2 => dst[3] = before[3],
        3 => dst[..3].copy_from_slice(&before[..3]),
        _ => {}
    }
}

fn sample(px: &[u8], w: u32, h: u32, u: f32, v: f32) -> [f32; 4] {
    let x = ((u * w as f32).floor() as i64).clamp(0, w as i64 - 1);
    let y = ((v * h as f32).floor() as i64).clamp(0, h as i64 - 1);
    let o = ((y * w as i64 + x) * 4) as usize;
    [px[o] as f32 / 255.0, px[o + 1] as f32 / 255.0, px[o + 2] as f32 / 255.0, px[o + 3] as f32 / 255.0]
}

/// (positions, uvs) per triangle of the mesh.
fn mesh_tris(m: &Mesh) -> Vec<([Vec3; 3], [Vec2; 3])> {
    m.tris()
        .into_iter()
        .filter_map(|(_, f, k)| {
            let p = [m.pos(f.v[k[0]])?, m.pos(f.v[k[1]])?, m.pos(f.v[k[2]])?];
            let uv = [k[0], k[1], k[2]].map(|i| f.uv.get(i).and_then(|u| m.uvs.get(*u).copied()).unwrap_or(Vec2::ZERO));
            Some((p, uv))
        })
        .collect()
}

fn mesh_of(v: &Value, who: &str) -> mlua::Result<Mesh> {
    let r = opt_inst(v).ok_or_else(|| rt_err(format!("{who}: EditableMesh is nil")))?;
    with(|s| s.meshes.get(&r).map(|m| lock(m).clone()))
        .ok_or_else(|| rt_err(format!("EditableImage:{who}: projectionSource must be an EditableMesh or WrapTextureTransfer")))
}

pub fn draw_projected(lua: &Lua, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let who = "DrawImageProjected";
    let bx = projection(&arg(args, 2), who)?;
    let (b, decal) = brush(&arg(args, 3), who, true)?;
    let m = mesh_of(&arg(args, 1), who)?;
    let decal = decal.expect("decal");
    let (dw, dh, dpx) = with(|s| s.images.get(&decal).map(|i| (i.w, i.h, i.px.clone()))).ok_or_else(|| rt_err(format!("{who}: Decal is nil")))?;
    let tris = mesh_tris(&m);
    edit_pixels(this, |w, h, px| {
        let scale = Vec2::new(w as f32, h as f32);
        for (p, uv) in &tris {
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            if n.dot(-bx.dir) < b.cos_fade - 1e-6 {
                continue;
            }
            let t = uv.map(|u| u * scale);
            let lo = t[0].min(t[1]).min(t[2]).floor().max(Vec2::ZERO);
            let hi = t[0].max(t[1]).max(t[2]).ceil().min(scale);
            let area = (t[1] - t[0]).perp_dot(t[2] - t[0]);
            if area.abs() < 1e-9 {
                continue;
            }
            for y in lo.y as i64..hi.y as i64 {
                for x in lo.x as i64..hi.x as i64 {
                    let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = (t[2] - t[1]).perp_dot(q - t[1]) / area;
                    let w1 = (t[0] - t[2]).perp_dot(q - t[2]) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-6 || w1 < -1e-6 || w2 < -1e-6 {
                        continue;
                    }
                    let world = p[0] * w0 + p[1] * w1 + p[2] * w2;
                    let l = world - bx.pos;
                    let (u, v, d) = (l.dot(bx.right) / bx.size.x + 0.5, 0.5 - l.dot(bx.up) / bx.size.y, l.dot(bx.dir));
                    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) || d.abs() > bx.size.z * 0.5 {
                        continue;
                    }
                    let mut s = sample(&dpx, dw, dh, u, v);
                    s[3] *= b.intensity;
                    let o = ((y * w as i64 + x) * 4) as usize;
                    put(&mut px[o..o + 4], s, &b);
                }
            }
        }
    })?;
    touched(lua, this)?;
    Ok(MultiValue::new())
}

pub fn sample_projected(lua: &Lua, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let who = "SampleImageProjected";
    let bx = projection(&arg(args, 3), who)?;
    let (b, _) = brush(&arg(args, 4), who, false)?;
    let m = mesh_of(&arg(args, 1), who).map_err(|_| rt_err("SampleImageProjected: sourceMesh is nil"))?;
    let src = opt_inst(&arg(args, 2)).ok_or_else(|| rt_err("SampleImageProjected: sourceTexture is nil"))?;
    let (sw, sh, spx) = with(|s| s.images.get(&src).map(|i| (i.w, i.h, i.px.clone()))).ok_or_else(|| rt_err("SampleImageProjected: sourceTexture is nil"))?;
    if sw == 0 || sh == 0 {
        return Err(rt_err("SampleImageProjected: sourceTexture has zero dimensions"));
    }
    let tris = mesh_tris(&m);
    edit_pixels(this, |w, h, px| {
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let u = (x as f32 + 0.5) / w as f32 - 0.5;
                let v = 0.5 - (y as f32 + 0.5) / h as f32;
                let o = bx.pos + bx.right * u * bx.size.x + bx.up * v * bx.size.y - bx.dir * bx.size.z * 0.5;
                let mut best: Option<(f32, Vec2)> = None;
                for (p, uv) in &tris {
                    let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
                    if n.dot(-bx.dir) < b.cos_fade - 1e-6 {
                        continue;
                    }
                    let e1 = p[1] - p[0];
                    let e2 = p[2] - p[0];
                    let pv = bx.dir.cross(e2);
                    let det = e1.dot(pv);
                    if det.abs() < 1e-12 {
                        continue;
                    }
                    let s = o - p[0];
                    let a = s.dot(pv) / det;
                    let q = s.cross(e1);
                    let c = bx.dir.dot(q) / det;
                    let t = e2.dot(q) / det;
                    if a < 0.0 || c < 0.0 || a + c > 1.0 || t < 0.0 || t > bx.size.z {
                        continue;
                    }
                    if best.is_none_or(|bb| t < bb.0) {
                        best = Some((t, uv[0] * (1.0 - a - c) + uv[1] * a + uv[2] * c));
                    }
                }
                if let Some((_, uv)) = best {
                    let s = sample(&spx, sw, sh, uv.x, uv.y);
                    let i = ((y * w as i64 + x) * 4) as usize;
                    put(&mut px[i..i + 4], s, &b);
                }
            }
        }
    })?;
    touched(lua, this)?;
    Ok(MultiValue::new())
}
