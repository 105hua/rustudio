//! Editor camera: fly (WASD + right-drag look) and orbit (around a focus point).

use egui::{Pos2, Rect};
use glam::{Mat3, Mat4, Vec3, Vec4};

use crate::math::Ray;
use crate::mech;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    Fly,
    Orbit,
}

#[derive(Debug, Clone)]
pub struct Camera {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_deg: f32,
    /// The point orbit mode circles and zoom aims at.
    pub focus: Vec3,
    pub mode: CameraMode,
    /// A game's Camera.CFrame rotation (Play), roll included; used only
    /// while it still looks where yaw / pitch do, so anything that turns
    /// the camera by yaw / pitch drops it. Without it a view straight
    /// down came out turned 180° (pitch −π/2 has cos < 0 in f32).
    pub basis: Option<Mat3>,
    /// A game Camera's orthographic projection (ProjectionType Orthographic):
    /// OrthographicSize and OrthographicMode (0 Vertical, 1 Horizontal,
    /// 2 Diagonal, 3 MaxAxis), and ViewStretch (Studio with
    /// SupportOrthoCameraApi on: checks/parity/world.md).
    pub ortho: Option<(f32, u32)>,
    pub stretch: f32,
}

impl Default for Camera {
    fn default() -> Self {
        let pos = Vec3::from(mech::START_CAMERA_POS);
        let pitch = mech::START_CAMERA_PITCH_DEG.to_radians();
        let mut c = Camera {
            pos,
            yaw: 0.0,
            pitch,
            fov_deg: mech::FOV_DEG,
            focus: Vec3::ZERO,
            mode: CameraMode::Fly,
            basis: None,
            ortho: None,
            stretch: 1.0,
        };
        c.focus = c.pos + c.forward() * pos.length();
        c
    }
}

impl Camera {
    fn forward_yp(&self) -> Vec3 {
        Vec3::new(
            -self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }

    /// The game camera's rotation while it still agrees with yaw / pitch.
    fn game_basis(&self) -> Option<Mat3> {
        self.basis
            .filter(|b| (-b.z_axis - self.forward_yp()).length() < 1e-3)
    }

    /// Sets the view from a rotation (Camera.CFrame in Play), keeping yaw /
    /// pitch in step for everything that reads them.
    pub fn set_rotation(&mut self, rot: Mat3) {
        let look = -rot.z_axis;
        self.pitch = look.y.clamp(-1.0, 1.0).asin();
        self.yaw = (-look.x).atan2(-look.z);
        self.basis = Some(rot);
    }

    pub fn forward(&self) -> Vec3 {
        match self.game_basis() {
            Some(b) => -b.z_axis,
            None => self.forward_yp(),
        }
    }

    pub fn right(&self) -> Vec3 {
        match self.game_basis() {
            Some(b) => b.x_axis,
            None => self.forward().cross(Vec3::Y).normalize_or(Vec3::X),
        }
    }

    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
    }

    pub fn view(&self) -> Mat4 {
        self.view_at(self.pos)
    }

    fn view_at(&self, pos: Vec3) -> Mat4 {
        match self.game_basis() {
            Some(b) => Mat4::look_to_rh(pos, -b.z_axis, b.y_axis),
            None => Mat4::look_to_rh(pos, self.forward(), Vec3::Y),
        }
    }

    /// The inverse of `view_proj` with the camera at the origin, inverted in
    /// f64: what the shader rebuilds view directions from (the sky, ray_dir).
    /// The camera's translation inside an f32 inverse costs the sky its
    /// direction far from the origin: 0.3° of wobble at 1000 studs, 6° at
    /// 20000, so the skybox jittered as the camera moved.
    pub fn inv_view_proj_rel(&self, rect: Rect) -> Mat4 {
        let proj = self.proj(rect.width() / rect.height().max(1.0));
        (proj.as_dmat4() * self.view_at(Vec3::ZERO).as_dmat4())
            .inverse()
            .as_mat4()
    }

    /// Half the orthographic view's width and height (studs): Studio sizes
    /// the height (Vertical), width (Horizontal), diagonal or larger side
    /// (MaxAxis) by OrthographicSize; ViewStretch widens it.
    fn ortho_half(&self, aspect: f32) -> Option<(f32, f32)> {
        let (size, mode) = self.ortho?;
        let a = aspect.max(0.01);
        let s = size * 0.5;
        let h = match mode {
            1 => s / a,
            2 => s / (1.0 + a * a).sqrt(),
            3 if a >= 1.0 => s / a,
            _ => s,
        };
        Some((h * a * self.stretch.max(1e-3), h))
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        if let Some((w, h)) = self.ortho_half(aspect) {
            // The near plane is the camera's own (nothing behind it draws).
            return Mat4::orthographic_rh(-w, w, -h, h, 0.0, mech::FAR);
        }
        Mat4::perspective_rh(
            self.fov_deg.to_radians(),
            aspect.max(0.01),
            mech::NEAR,
            mech::FAR,
        )
    }

    pub fn view_proj(&self, rect: Rect) -> Mat4 {
        self.proj(rect.width() / rect.height().max(1.0)) * self.view()
    }

    /// What the GPU draws with: reverse-Z (near = 1, infinitely far = 0),
    /// which keeps depth precise far away and stops distant flicker.
    pub fn view_proj_reverse_z(&self, rect: Rect) -> Mat4 {
        let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
        if let Some((w, h)) = self.ortho_half(aspect) {
            // Reverse-Z: the camera plane at 1, FAR at 0.
            return Mat4::orthographic_rh(-w, w, -h, h, mech::FAR, 0.0) * self.view();
        }
        Mat4::perspective_infinite_reverse_rh(self.fov_deg.to_radians(), aspect, mech::NEAR)
            * self.view()
    }

    pub fn focus_distance(&self) -> f32 {
        (self.focus - self.pos).length().max(1.0)
    }

    /// Mouse look (fly) or orbit around the focus point. Dragging right turns
    /// the view right; dragging down tilts it down.
    pub fn rotate(&mut self, dx: f32, dy: f32) {
        let d = mech::LOOK_DEG_PER_PX.to_radians();
        let dist = self.focus_distance();
        // forward = (-sin yaw, _, -cos yaw): turning right (toward +right) needs yaw to decrease.
        self.yaw -= dx * d;
        self.pitch = (self.pitch - dy * d).clamp(-1.55, 1.55);
        match self.mode {
            CameraMode::Fly => self.focus = self.pos + self.forward() * dist,
            CameraMode::Orbit => self.pos = self.focus - self.forward() * dist,
        }
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        let mode = self.mode;
        self.mode = CameraMode::Orbit;
        self.rotate(dx, dy);
        self.mode = mode;
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        let k = self.focus_distance() * mech::PAN_FRACTION_PER_PX;
        let delta = -self.right() * dx * k + self.up() * dy * k;
        self.pos += delta;
        self.focus += delta;
    }

    /// Wheel zoom, Studio's rule (Studio ..):
    /// each notch moves the camera and its focus point together by
    /// "Camera Mouse Wheel Speed" (15) studs, or "Camera Shift Speed" (0.2)
    /// with Shift held, along the cursor ray when "Zoom to Mouse Position"
    /// is on, else straight ahead. No slow-down near what's under the cursor:
    /// the camera can pass through it. The view direction stays.
    pub fn zoom_toward(&mut self, dir: Vec3, notches: f32, shift: bool) {
        let speed = if shift {
            mech::FLY_SHIFT_FACTOR
        } else {
            mech::WHEEL_STEP
        };
        let dir = if mech::WHEEL_ZOOM_TO_MOUSE {
            dir
        } else {
            self.forward()
        };
        let delta = dir * speed * notches;
        self.pos += delta;
        self.focus += delta;
    }

    /// WASD/QE movement in camera space; `dir` is (right, up, forward);
    /// `boost` is the acceleration multiplier from holding the keys.
    pub fn fly(&mut self, dir: Vec3, dt: f32, slow: bool, boost: f32) {
        if dir == Vec3::ZERO {
            return;
        }
        let speed = mech::FLY_SPEED * boost * if slow { mech::FLY_SHIFT_FACTOR } else { 1.0 };
        // Studio (, then ): the key vector (A/D x, Q/E
        // y, W/S z) is normalised, then multiplied by all three columns of
        // the camera's rotation ([camera+0xe8..0x114], the look column
        // negated by the sign mask at ): Q / E go along the
        // camera's own up axis, not the world's.
        let key = dir.normalize_or_zero();
        let delta = (self.right() * key.x + self.up() * key.y + self.forward() * key.z)
            * speed
            * dt;
        self.pos += delta;
        self.focus += delta;
    }

    /// Frames a bounding sphere.
    pub fn frame(&mut self, center: Vec3, radius: f32) {
        let dist = (radius.max(1.0) * mech::FOCUS_DISTANCE_FACTOR)
            / (self.fov_deg.to_radians() * 0.5).tan();
        self.focus = center;
        self.pos = center - self.forward() * dist;
    }

    pub fn ray(&self, rect: Rect, p: Pos2) -> Ray {
        let ndc_x = (p.x - rect.min.x) / rect.width() * 2.0 - 1.0;
        let ndc_y = 1.0 - (p.y - rect.min.y) / rect.height() * 2.0;
        let inv = self.view_proj(rect).inverse();
        let near = inv * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far = inv * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        let near = near.truncate() / near.w;
        let far = far.truncate() / far.w;
        if self.ortho.is_some() {
            // Parallel rays from the camera plane.
            return Ray { origin: near, dir: self.forward() };
        }
        Ray {
            origin: self.pos,
            dir: (far - near).normalize_or(self.forward()),
        }
    }

    pub fn project(&self, rect: Rect, world: Vec3) -> Option<Pos2> {
        let clip = self.view_proj(rect) * world.extend(1.0);
        if clip.w <= 1e-4 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Pos2::new(
            rect.min.x + (ndc.x + 1.0) * 0.5 * rect.width(),
            rect.min.y + (1.0 - ndc.y) * 0.5 * rect.height(),
        ))
    }

    /// World units per screen pixel at a point (to keep gizmos a fixed size).
    pub fn world_per_px(&self, rect: Rect, at: Vec3) -> f32 {
        if let Some((_, h)) = self.ortho_half(rect.width() / rect.height().max(1.0)) {
            return 2.0 * h / rect.height().max(1.0);
        }
        let dist = (at - self.pos).dot(self.forward()).max(0.1);
        2.0 * dist * (self.fov_deg.to_radians() * 0.5).tan() / rect.height().max(1.0)
    }
}
