//! Visible distance constraints. Rope centerline follows the recovered Studio
//! catenary; cross-section shading remains a camera-facing strip.
use super::*;

/// Studio: 64 uniform horizontal catenary intervals.
/// trims vertical ends when horizontal span / length <.05.
fn rope_points(a: Vec3, b: Vec3, length: f32) -> Vec<Vec3> {
    let distance = a.distance(b);
    if !length.is_finite() || length <= distance { return vec![a, b]; }
    let horizontal = Vec3::new(b.x-a.x, 0.0, b.z-a.z);
    let h = horizontal.length();
    let dy = (b.y-a.y).abs();
    let mut drop0 = 0.0;
    let mut drop1 = 0.0;
    if h <= (length * length - dy * dy).sqrt() * 0.05 {
        if h <= (length-dy)*0.05 {
            drop1 = ((length-dy)-h/0.05)*0.5;
            drop0 = drop1+dy;
        } else {
            drop0 = ((dy+length)-h*h/((length-dy)*0.0025))*0.5;
        }
        if a.y < b.y { std::mem::swap(&mut drop0, &mut drop1); }
    }
    let start = a-Vec3::Y*drop0;
    let end = b-Vec3::Y*drop1;
    let remaining = length-drop0-drop1;
    let mut points = vec![a];
    if drop0 > 0.0 { points.push(start); }
    if remaining <= 0.05 || h < 1e-6 {
        points.push((start+end)*0.5);
    } else {
        // Solve sinh(u)/u = sqrt(L²-dy²)/h. Studio uses a
        // three-step approximation; bisection is stable here.
        let v = (end.y-start.y) as f64;
        let l = remaining as f64;
        let h64 = h as f64;
        let ratio = ((l*l-v*v).max(h64*h64)).sqrt()/h64;
        let (mut lo, mut hi) = (0.0f64, 30.0f64);
        for _ in 0..60 {
            let u = (lo+hi)*0.5;
            if u.sinh()/u < ratio { lo=u; } else { hi=u; }
        }
        let scale = h64/(lo+hi).max(1e-9);
        let x0 = h64*0.5-scale*(v/l).atanh();
        let base = (x0/scale).cosh();
        for i in 1..=64 {
            let t = i as f32/64.0;
            let y = scale*(((h64*t as f64-x0)/scale).cosh()-base);
            points.push(start+horizontal*t+Vec3::Y*y as f32);
        }
    }
    if drop1 > 0.0 || points.len() < 3 { points.push(b); }
    else if let Some(last) = points.last_mut() { *last = b; }
    points
}

pub(super) fn draw(d: &Document, r: Ref, view: &View) -> Option<Draw> {
    // Visible is independent of Enabled: disabling physics retains geometry.
    if !flag(d, r, "Visible", false) { return None; }
    let a = attachment_world(d, reference(d, r, "Attachment0")?)?.pos;
    let b = attachment_world(d, reference(d, r, "Attachment1")?)?.pos;
    let class = d.class_of(r)?;
    let points = match class.as_str() {
        "RopeConstraint" => rope_points(a, b, num(d, r, "Length", 5.0)),
        "SpringConstraint" => {
            let axis = (b - a).normalize_or_zero();
            if axis == Vec3::ZERO { return None; }
            let u = axis.any_orthonormal_vector();
            let v = axis.cross(u);
            let coils = num(d, r, "Coils", 8.0).clamp(0.0, 100.0);
            let radius = num(d, r, "Radius", 0.4).max(0.0);
            let n = (coils * 16.0).ceil().max(16.0) as usize;
            (0..=n).map(|i| {
                let t = i as f32 / n as f32;
                if i == 0 { a } else if i == n { b } else {
                    let angle = t * coils * std::f32::consts::TAU;
                    a.lerp(b, t) + radius * (u * angle.cos() + v * angle.sin())
                }
            }).collect()
        }
        _ => vec![a, b],
    };
    let half = num(d, r, "Thickness", 0.1).max(0.0) * 0.5;
    if half == 0.0 { return None; }
    let rows: Vec<_> = points.iter().enumerate().map(|(i, &p)| {
        let prev = points[i.saturating_sub(1)];
        let next = points[(i + 1).min(points.len() - 1)];
        let tangent = (next - prev).normalize_or_zero();
        let side = tangent.cross(view.pos - p).normalize_or_zero();
        let side = if side == Vec3::ZERO { view.right } else { side };
        (p, side * half, i as f32 / (points.len() - 1) as f32, 0.0)
    }).collect();
    let rgb = match d.prop(r, "Color") {
        Some(Variant::BrickColor(c)) => Vec3::from_array(crate::scene::brick_rgb(c).map(|x| x as f32 / 255.0)),
        _ => Vec3::splat(0.64),
    };
    strip(&rows, &CSeq(vec![(0.0, rgb)]), &Seq::read(None, 0.0), 0.0,
        &Look { influence: 1.0, brightness: 1.0, emission: 0.0, alpha: 1.0 },
        0.0, false, &[], 0.0, view, (TextureId::Managed(0), false))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slack_preserves_endpoints_and_length() {
        for b in [Vec3::X * 10.0, Vec3::Y * 10.0, Vec3::new(5.0, 8.0, 3.0)] {
            let points = rope_points(Vec3::ZERO, b, 15.0);
            assert_eq!(points[0], Vec3::ZERO);
            assert_eq!(*points.last().unwrap(), b);
            let arc: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
            assert!((arc - 15.0).abs() < 0.01, "{arc}");
        }
        assert_eq!(rope_points(Vec3::ZERO, Vec3::X * 10.0, 5.0).len(), 2);
    }
}
