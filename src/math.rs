//! Conversions between Roblox value types and glam, plus ray tests.

use glam::{EulerRot, Mat3, Vec3};
use rbx_dom_weak::types::{CFrame, Matrix3, Vector3};

pub fn v3(v: Vector3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

pub fn rv3(v: Vec3) -> Vector3 {
    Vector3::new(v.x, v.y, v.z)
}

/// A rigid transform: position plus an orthonormal rotation (columns are the
/// part's right, up and back vectors, like Roblox's CFrame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xform {
    pub pos: Vec3,
    pub rot: Mat3,
}

impl Xform {
    /// A point in this transform's space → world.
    pub fn apply(&self, p: Vec3) -> Vec3 {
        self.pos + self.rot * p
    }

    /// The inverse transform (world → this space).
    pub fn inverse(&self) -> Xform {
        let rt = self.rot.transpose();
        Xform {
            pos: -(rt * self.pos),
            rot: rt,
        }
    }

    pub fn from_cframe(cf: &CFrame) -> Self {
        // rbx_types stores the matrix as rows.
        let m = &cf.orientation;
        let rot = Mat3::from_cols(
            Vec3::new(m.x.x, m.y.x, m.z.x),
            Vec3::new(m.x.y, m.y.y, m.z.y),
            Vec3::new(m.x.z, m.y.z, m.z.z),
        );
        Xform {
            pos: v3(cf.position),
            rot,
        }
    }

    pub fn to_cframe(&self) -> CFrame {
        let (c0, c1, c2) = (self.rot.x_axis, self.rot.y_axis, self.rot.z_axis);
        CFrame::new(
            rv3(self.pos),
            Matrix3::new(
                Vector3::new(c0.x, c1.x, c2.x),
                Vector3::new(c0.y, c1.y, c2.y),
                Vector3::new(c0.z, c1.z, c2.z),
            ),
        )
    }

    /// Orientation in degrees, Roblox order (applied Y, then X, then Z).
    pub fn orientation_deg(&self) -> Vec3 {
        let (y, x, z) = self.rot.to_euler(EulerRot::YXZ);
        Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
    }

    pub fn from_orientation_deg(pos: Vec3, deg: Vec3) -> Self {
        let rot = Mat3::from_euler(
            EulerRot::YXZ,
            deg.y.to_radians(),
            deg.x.to_radians(),
            deg.z.to_radians(),
        );
        Xform { pos, rot }
    }

    /// Re-orthonormalises the rotation (drag accumulation drifts slightly).
    pub fn orthonormalized(mut self) -> Self {
        let x = self.rot.x_axis.normalize_or(Vec3::X);
        let y = (self.rot.y_axis - x * x.dot(self.rot.y_axis)).normalize_or(Vec3::Y);
        // Keep a mirrored matrix mirrored (its faces wind the other way).
        let z = x.cross(y)
            * if self.rot.determinant() < 0.0 {
                -1.0
            } else {
                1.0
            };
        self.rot = Mat3::from_cols(x, y, z);
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }

    /// Distance along the ray to an oriented box, if it is hit.
    /// Like hit_obb, but a ray that starts inside the box doesn't hit it
    /// (Roblox's WorldRoot:Raycast rule).
    pub fn enter_obb(&self, xf: &Xform, size: Vec3) -> Option<f32> {
        let local = xf.rot.transpose() * (self.origin - xf.pos);
        if local.abs().cmple(size * 0.5).all() {
            return None;
        }
        self.hit_obb(xf, size)
    }

    pub fn hit_obb(&self, xf: &Xform, size: Vec3) -> Option<f32> {
        let inv = xf.rot.transpose();
        let o = inv * (self.origin - xf.pos);
        let d = inv * self.dir;
        let half = size * 0.5;
        let mut tmin = f32::NEG_INFINITY;
        let mut tmax = f32::INFINITY;
        for i in 0..3 {
            if d[i].abs() < 1e-8 {
                if o[i] < -half[i] || o[i] > half[i] {
                    return None;
                }
            } else {
                let t1 = (-half[i] - o[i]) / d[i];
                let t2 = (half[i] - o[i]) / d[i];
                tmin = tmin.max(t1.min(t2));
                tmax = tmax.min(t1.max(t2));
            }
        }
        if tmax >= tmin.max(0.0) {
            Some(tmin.max(0.0))
        } else {
            None
        }
    }

    pub fn hit_plane(&self, point: Vec3, normal: Vec3) -> Option<f32> {
        let denom = normal.dot(self.dir);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (point - self.origin).dot(normal) / denom;
        (t >= 0.0).then_some(t)
    }

    /// Parameter `s` of the point on the line `p + s*axis` closest to this ray.
    pub fn closest_on_line(&self, p: Vec3, axis: Vec3) -> Option<f32> {
        let w0 = p - self.origin;
        let a = axis.dot(axis);
        let b = axis.dot(self.dir);
        let c = self.dir.dot(self.dir);
        let d = axis.dot(w0);
        let e = self.dir.dot(w0);
        let denom = a * c - b * b;
        if denom.abs() < 1e-6 {
            return None;
        }
        Some((b * e - c * d) / denom)
    }
}

pub fn snap(v: f32, inc: f32) -> f32 {
    if inc > 0.0 {
        (v / inc).round() * inc
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cframe_roundtrip() {
        let xf = Xform::from_orientation_deg(Vec3::new(1.0, 2.0, 3.0), Vec3::new(10.0, 20.0, 30.0));
        let back = Xform::from_cframe(&xf.to_cframe());
        assert!((back.pos - xf.pos).length() < 1e-5);
        let o = back.orientation_deg();
        assert!((o - Vec3::new(10.0, 20.0, 30.0)).length() < 1e-3, "{o:?}");
    }

    #[test]
    fn ray_hits_box() {
        let r = Ray {
            origin: Vec3::new(0.0, 0.0, 10.0),
            dir: Vec3::NEG_Z,
        };
        let t = r
            .hit_obb(
                &Xform {
                    pos: Vec3::ZERO,
                    rot: Mat3::IDENTITY,
                },
                Vec3::splat(2.0),
            )
            .unwrap();
        assert!((t - 9.0).abs() < 1e-5);
    }
}
