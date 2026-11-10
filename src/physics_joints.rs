//! Joint constraints between assemblies: HingeConstraint, BallSocketConstraint,
//! PrismaticConstraint, CylindricalConstraint, UniversalConstraint,
//! PlaneConstraint and NoCollisionConstraint become rapier generic impulse
//! joints between the two attachments' assembly bodies (an anchored end hangs
//! on one fixed `ground` body, its frame in world space). SpringConstraint and
//! TorsionSpringConstraint push with impulses every substep (`apply_springs`);
//! RodConstraint is the rope solver run two-sided (physics_rope.rs).
//!
//! The joint frame is each attachment's frame in its body: Roblox's axes
//! are the attachments' (X primary: the hinge / slide axis; Y secondary:
//! the angle's zero), which is also rapier's generic-joint X axis. A
//! constraint doesn't merge assemblies, and its parts still collide (as in
//! Studio), except NoCollisionConstraint's.
//!
//! Before this, only ropes and rigid joints existed: a Hinge / BallSocket
//! bob fell freely (a pendulum fixture).
use super::*;
use rp::{GenericJointBuilder, JointAxesMask as Mask, JointAxis as Axis, MotorModel};

const JOINT_CLASSES: [&str; 9] = [
    "HingeConstraint",
    "BallSocketConstraint",
    "PrismaticConstraint",
    "CylindricalConstraint",
    "UniversalConstraint",
    "PlaneConstraint",
    "NoCollisionConstraint",
    "SpringConstraint",
    "TorsionSpringConstraint",
];

pub(super) fn is_joint_constraint(class: &str) -> bool {
    JOINT_CLASSES.contains(&class)
}

/// A joint made for a constraint: the bodies it joins and what it was built
/// from (rebuilt when either changes).
pub(super) struct LiveJoint {
    handle: rp::ImpulseJointHandle,
    bodies: (rp::RigidBodyHandle, rp::RigidBodyHandle),
    sig: Vec<f32>,
}

/// One end of a constraint: its body (None = fixed: the ground body) and
/// the attachment's frame in that body (world frame for a fixed end).
struct End {
    body: Option<rp::RigidBodyHandle>,
    frame: rp::Pose,
    world: rp::Pose,
}

fn deg(v: f32) -> f32 {
    v.to_radians()
}

/// Roblox's math.huge limits; rapier wants finite numbers.
fn fin(v: f32) -> f32 {
    if v.is_finite() { v.abs() } else { f32::MAX }
}

fn pose_sig(p: &rp::Pose, out: &mut Vec<f32>) {
    let t = p.translation;
    let r = p.rotation;
    out.extend([t.x, t.y, t.z, r.x, r.y, r.z, r.w]);
}

impl Physics {
    fn ground(&mut self) -> rp::RigidBodyHandle {
        if let Some(g) = self.ground.filter(|g| self.world.bodies.get(*g).is_some()) {
            return g;
        }
        let g = self.world.insert_body(rp::RigidBodyBuilder::fixed().build());
        self.ground = Some(g);
        g
    }

    /// An attachment's end: its part's body and its frame there.
    fn joint_end(&self, doc: &Document, att: Ref) -> Option<End> {
        let part = attachment_part(doc, att)?;
        let att_world = crate::attachment::world(doc, att, false)?;
        let part_doc = xform_of(doc, part)?;
        // The attachment in its part's space, from the document (both there).
        let local = pose(&part_doc).inverse() * pose(&att_world);
        let world = pose(&self.part_xform(doc, part).unwrap_or(part_doc)) * local;
        match self.parts.get(&part) {
            Some(pb) if pb.asm.is_some() => Some(End {
                body: self.body_of(part),
                frame: pb.local * local,
                world,
            }),
            // Anchored / not simulated: the ground body, in world space.
            _ => Some(End { body: None, frame: world, world }),
        }
    }

    /// Creates, updates or drops the joints for the joint constraints.
    pub(super) fn apply_joints(&mut self, doc: &Document) {
        if self.joint_cons.is_empty() && self.live_joints.is_empty() {
            return;
        }
        let Some(ws) = doc.workspace() else { return };
        let cons: Vec<Ref> = self.joint_cons.iter().copied().collect();
        let mut keep: HashSet<Ref> = HashSet::new();
        for r in cons {
            let Some(class) = doc.class_of(r) else {
                self.joint_cons.remove(&r);
                continue;
            };
            if !is_joint_constraint(&class) || !doc.is_ancestor(ws, r) {
                self.joint_cons.remove(&r);
                continue;
            }
            if matches!(class.as_str(), "SpringConstraint" | "TorsionSpringConstraint") {
                continue; // apply_springs
            }
            if !bool_prop(doc, r, "Enabled", true) {
                continue;
            }
            let Some((data, b0, b1, sig)) = self.joint_data(doc, r, &class) else { continue };
            keep.insert(r);
            let live = self.live_joints.get(&r).filter(|l| {
                self.world.impulse_joints.get(l.handle).is_some()
            });
            match live {
                Some(l) if l.bodies == (b0, b1) => {
                    if l.sig.len() != sig.len()
                        || l.sig.iter().zip(&sig).any(|(a, b)| (a - b).abs() > 1e-4)
                    {
                        let h = l.handle;
                        if let Some(j) = self.world.impulse_joints.get_mut(h, true) {
                            j.data = data;
                        }
                        if let Some(l) = self.live_joints.get_mut(&r) {
                            l.sig = sig;
                        }
                    }
                }
                _ => {
                    if let Some(old) = self.live_joints.remove(&r) {
                        self.world.impulse_joints.remove(old.handle, true);
                    }
                    let handle = self.world.impulse_joints.insert(b0, b1, data, true);
                    self.live_joints.insert(r, LiveJoint { handle, bodies: (b0, b1), sig });
                }
            }
        }
        let gone: Vec<Ref> = self.live_joints.keys().filter(|r| !keep.contains(r)).copied().collect();
        for r in gone {
            if let Some(old) = self.live_joints.remove(&r) {
                self.world.impulse_joints.remove(old.handle, true);
            }
        }
    }

    /// The rapier joint for a constraint, its bodies and its signature.
    fn joint_data(
        &mut self,
        doc: &Document,
        r: Ref,
        class: &str,
    ) -> Option<(rp::GenericJoint, rp::RigidBodyHandle, rp::RigidBodyHandle, Vec<f32>)> {
        let (e0, e1) = if class == "NoCollisionConstraint" {
            // Part0 / Part1: two bodies that don't touch each other.
            let p0 = joint_parts(doc, r).map(|p| p.0)?;
            let p1 = joint_parts(doc, r).map(|p| p.1)?;
            let b0 = self.body_of(p0);
            let b1 = self.body_of(p1);
            (
                End { body: b0, frame: rp::Pose::IDENTITY, world: rp::Pose::IDENTITY },
                End { body: b1, frame: rp::Pose::IDENTITY, world: rp::Pose::IDENTITY },
            )
        } else {
            let a0 = ref_prop(doc, r, "Attachment0")?;
            let a1 = ref_prop(doc, r, "Attachment1")?;
            (self.joint_end(doc, a0)?, self.joint_end(doc, a1)?)
        };
        // Nothing to hold: both fixed, or one assembly.
        if e0.body.is_none() && e1.body.is_none() || e0.body == e1.body {
            return None;
        }
        let e = |name: &str| enum_u32(doc.prop(r, name));
        let n = |name: &str, d: f32| num(doc, r, name, d);
        let on = |name: &str| bool_prop(doc, r, name, false);
        let mut sig = Vec::new();
        pose_sig(&e0.frame, &mut sig);
        pose_sig(&e1.frame, &mut sig);
        let mut j = match class {
            "HingeConstraint" => GenericJointBuilder::new(Mask::LOCKED_REVOLUTE_AXES),
            "BallSocketConstraint" => GenericJointBuilder::new(Mask::LOCKED_SPHERICAL_AXES),
            "PrismaticConstraint" => GenericJointBuilder::new(Mask::LOCKED_PRISMATIC_AXES),
            "CylindricalConstraint" => GenericJointBuilder::new(
                Mask::LIN_Y | Mask::LIN_Z | Mask::ANG_Y | Mask::ANG_Z,
            ),
            "UniversalConstraint" => GenericJointBuilder::new(Mask::LIN_AXES | Mask::ANG_X),
            "PlaneConstraint" => GenericJointBuilder::new(Mask::LIN_X),
            "NoCollisionConstraint" => GenericJointBuilder::new(Mask::empty()).contacts_enabled(false),
            _ => return None,
        }
        .local_frame1(e0.frame)
        .local_frame2(e1.frame);
        // ActuatorType: None 0, Motor 1, Servo 2.
        let mut actuate = |j: GenericJointBuilder,
                           axis: Axis,
                           kind: Option<u32>,
                           vel: f32,
                           motor_max: f32,
                           target: f32,
                           resp: f32,
                           servo_max: f32,
                           sig: &mut Vec<f32>| {
            sig.extend([kind.unwrap_or(0) as f32, vel, motor_max, target, resp, servo_max]);
            match kind {
                Some(1) => j
                    .motor_model(axis, MotorModel::ForceBased)
                    .motor_velocity(axis, vel, 1e5)
                    .motor_max_force(axis, fin(motor_max)),
                Some(2) => j
                    .motor_model(axis, MotorModel::AccelerationBased)
                    // Critically damped at the responsiveness.
                    .motor_position(axis, target, resp * resp * 0.25, resp)
                    .motor_max_force(axis, fin(servo_max)),
                _ => j,
            }
        };
        match class {
            "HingeConstraint" => {
                if on("LimitsEnabled") {
                    let (lo, hi) = (deg(n("LowerAngle", -45.0)), deg(n("UpperAngle", 45.0)));
                    sig.extend([lo, hi]);
                    j = j.limits(Axis::AngX, [lo.min(hi), hi.max(lo)]);
                }
                j = actuate(
                    j,
                    Axis::AngX,
                    e("ActuatorType"),
                    n("AngularVelocity", 0.0),
                    n("MotorMaxTorque", 0.0),
                    deg(n("TargetAngle", 0.0)),
                    n("AngularResponsiveness", 45.0),
                    n("ServoMaxTorque", 0.0),
                    &mut sig,
                );
            }
            "BallSocketConstraint" => {
                if on("LimitsEnabled") {
                    let a = deg(n("UpperAngle", 45.0)).abs();
                    sig.push(a);
                    j = j.limits(Axis::AngY, [-a, a]).limits(Axis::AngZ, [-a, a]);
                }
                if on("TwistLimitsEnabled") {
                    let (lo, hi) = (deg(n("TwistLowerAngle", -45.0)), deg(n("TwistUpperAngle", 45.0)));
                    sig.extend([lo, hi]);
                    j = j.limits(Axis::AngX, [lo.min(hi), hi.max(lo)]);
                }
                let friction = n("MaxFrictionTorque", 0.0);
                if friction > 0.0 {
                    sig.push(friction);
                    for axis in [Axis::AngX, Axis::AngY, Axis::AngZ] {
                        j = j
                            .motor_model(axis, MotorModel::ForceBased)
                            .motor_velocity(axis, 0.0, 1e5)
                            .motor_max_force(axis, fin(friction));
                    }
                }
            }
            "PrismaticConstraint" | "CylindricalConstraint" => {
                if on("LimitsEnabled") {
                    let (lo, hi) = (n("LowerLimit", 0.0), n("UpperLimit", 5.0));
                    sig.extend([lo, hi]);
                    j = j.limits(Axis::LinX, [lo.min(hi), hi.max(lo)]);
                }
                j = actuate(
                    j,
                    Axis::LinX,
                    e("ActuatorType"),
                    n("Velocity", 0.0),
                    n("MotorMaxForce", 0.0),
                    n("TargetPosition", 0.0),
                    n("LinearResponsiveness", 45.0),
                    n("ServoMaxForce", 0.0),
                    &mut sig,
                );
                if class == "CylindricalConstraint" {
                    if on("AngularLimitsEnabled") {
                        let (lo, hi) = (deg(n("LowerAngle", -45.0)), deg(n("UpperAngle", 45.0)));
                        sig.extend([lo, hi]);
                        j = j.limits(Axis::AngX, [lo.min(hi), hi.max(lo)]);
                    }
                    j = actuate(
                        j,
                        Axis::AngX,
                        e("AngularActuatorType"),
                        n("AngularVelocity", 0.0),
                        n("MotorMaxTorque", 0.0),
                        deg(n("TargetAngle", 0.0)),
                        n("AngularResponsiveness", 45.0),
                        n("ServoMaxTorque", 0.0),
                        &mut sig,
                    );
                }
            }
            "UniversalConstraint" => {
                if on("LimitsEnabled") {
                    let a = deg(n("MaxAngle", 45.0)).abs();
                    sig.push(a);
                    j = j.limits(Axis::AngY, [-a, a]).limits(Axis::AngZ, [-a, a]);
                }
            }
            _ => {}
        }
        let g = if e0.body.is_none() || e1.body.is_none() { Some(self.ground()) } else { None };
        let b0 = e0.body.or(g)?;
        let b1 = e1.body.or(g)?;
        Some((j.build(), b0, b1, sig))
    }

    /// SpringConstraint and TorsionSpringConstraint: forces for one substep.
    pub(super) fn apply_springs(&mut self, doc: &Document, _h: f32) {
        if self.joint_cons.is_empty() {
            return;
        }
        let springs: Vec<Ref> = self
            .joint_cons
            .iter()
            .copied()
            .filter(|r| {
                doc.class_of(*r)
                    .is_some_and(|c| c == "SpringConstraint" || c == "TorsionSpringConstraint")
            })
            .collect();
        for r in springs {
            if !bool_prop(doc, r, "Enabled", true) {
                continue;
            }
            let (Some(a0), Some(a1)) = (ref_prop(doc, r, "Attachment0"), ref_prop(doc, r, "Attachment1")) else {
                continue;
            };
            let (Some(e0), Some(e1)) = (self.joint_end(doc, a0), self.joint_end(doc, a1)) else {
                continue;
            };
            if e0.body == e1.body {
                continue;
            }
            let p0 = e0.world.translation;
            let p1 = e1.world.translation;
            let class = doc.class_of(r).unwrap_or_default();
            let n = |name: &str, d: f32| num(doc, r, name, d);
            let vel = |b: Option<rp::RigidBodyHandle>, p: rp::Vector| {
                b.and_then(|b| self.world.bodies.get(b))
                    .map(|b| b.velocity_at_point(p))
                    .unwrap_or(rp::Vector::ZERO)
            };
            let angvel = |b: Option<rp::RigidBodyHandle>| {
                b.and_then(|b| self.world.bodies.get(b))
                    .map(|b| b.angvel())
                    .unwrap_or(rp::Vector::ZERO)
            };
            if class == "SpringConstraint" {
                let d = p1 - p0;
                let len = d.length();
                if len < 1e-6 {
                    continue;
                }
                let dir = d / len;
                // Stretch pulls the ends together; damping opposes separation.
                let sep = (vel(e1.body, p1) - vel(e0.body, p0)).dot(dir);
                let mut f = n("Stiffness", 0.0) * (len - n("FreeLength", 1.0)) + n("Damping", 0.0) * sep;
                let max = fin(n("MaxForce", f32::INFINITY));
                f = f.clamp(-max, max);
                // A force over the step (cleared by the next apply_movers), so
                // gravity and the spring integrate together.
                let force = dir * f;
                for (b, p, s) in [(e0.body, p0, 1.0), (e1.body, p1, -1.0)] {
                    if let Some(bh) = b {
                        if let Some(b) = self.world.bodies.get_mut(bh) {
                            b.add_force_at_point(force * s, p, true);
                            self.pushed.insert(bh);
                        }
                    }
                }
            } else {
                // TorsionSpring: turns Attachment1's Y back to Attachment0's
                // about their X axes.
                let x0 = e0.world.rotation * rp::Vector::X;
                let y0 = e0.world.rotation * rp::Vector::Y;
                let y1 = e1.world.rotation * rp::Vector::Y;
                let y1p = (y1 - x0 * y1.dot(x0)).normalize_or_zero();
                if y1p == rp::Vector::ZERO {
                    continue;
                }
                let angle = y0.cross(y1p).dot(x0).atan2(y0.dot(y1p));
                let w = (angvel(e1.body) - angvel(e0.body)).dot(x0);
                let mut t = -n("Stiffness", 0.0) * angle - n("Damping", 0.0) * w;
                let max = fin(n("MaxTorque", f32::INFINITY));
                t = t.clamp(-max, max);
                let torque = x0 * t;
                for (b, s) in [(e0.body, -1.0), (e1.body, 1.0)] {
                    if let Some(bh) = b {
                        if let Some(b) = self.world.bodies.get_mut(bh) {
                            b.add_torque(torque * s, true);
                            self.pushed.insert(bh);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::types::{CFrame, Vector3};
    use rbx_dom_weak::InstanceBuilder;

    fn part(name: &str, pos: Vec3, anchored: bool) -> InstanceBuilder {
        InstanceBuilder::new("Part")
            .with_name(name)
            .with_property("Anchored", anchored)
            .with_property("Size", Vector3::new(1.0, 1.0, 1.0))
            .with_property("CFrame", CFrame::new(Vector3::new(pos.x, pos.y, pos.z), CFrame::identity().orientation))
    }

    /// The pendulum fixture: an anchored pivot at (0,300,0), a
    /// 1-stud bob at (0,300,5) whose attachment sits on the pivot.
    fn pendulum(class: &str, props: &[(&str, Variant)]) -> (Document, Ref) {
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let top = doc.dom.insert(ws, part("Top", Vec3::new(0.0, 300.0, 0.0), true));
        let bob = doc.dom.insert(ws, part("Bob", Vec3::new(0.0, 300.0, 5.0), false));
        let a0 = doc.dom.insert(top, InstanceBuilder::new("Attachment"));
        let a1 = doc.dom.insert(bob, InstanceBuilder::new("Attachment").with_property(
            "CFrame",
            CFrame::new(Vector3::new(0.0, 0.0, -5.0), CFrame::identity().orientation),
        ));
        let mut b = InstanceBuilder::new(class)
            .with_property("Attachment0", a0)
            .with_property("Attachment1", a1);
        for (k, v) in props {
            b = b.with_property(*k, v.clone());
        }
        doc.dom.insert(top, b);
        (doc, bob)
    }

    fn run(doc: &mut Document, steps: usize) -> Vec<Vec3> {
        let mut p = Physics::new(doc);
        let mut out = Vec::new();
        for _ in 0..steps {
            p.sync(doc);
            p.step(doc, 1.0 / 60.0);
            out.push(Vec3::ZERO);
        }
        out
    }

    fn pos(doc: &Document, r: Ref) -> Vec3 {
        xform_of(doc, r).unwrap().pos
    }

    /// Studio (an offline Studio's client, 2026-11-09): the hinge bob swings in the YZ
    /// plane about the pivot and never falls away (y ≥ 295, |z| ≤ 5).
    #[test]
    fn hinge_and_ball_socket_pendulums_swing_about_the_pivot() {
        for class in ["HingeConstraint", "BallSocketConstraint"] {
            let (mut doc, bob) = pendulum(class, &[]);
            let mut p = Physics::new(&mut doc);
            let mut low: f32 = 300.0;
            let mut bottom_at = None;
            for i in 0..120 {
                p.sync(&mut doc);
                p.step(&mut doc, 1.0 / 60.0);
                let b = pos(&doc, bob);
                if bottom_at.is_none() && b.z <= 0.0 {
                    bottom_at = Some((i + 1) as f32 / 60.0);
                }
                low = low.min(b.y);
                let r = (b - Vec3::new(0.0, 300.0, 0.0)).length();
                assert!((r - 5.0).abs() < 0.3, "{class}: bob left the pivot: {b:?}");
                if class == "HingeConstraint" {
                    assert!(b.x.abs() < 0.05, "hinge swings in its plane: {b:?}");
                }
            }
            assert!(low < 295.5, "{class} swung down: {low}");
            // A quarter of a 90° swing: T/4 = 1.18·2π√(L/g)/4 ≈ 0.30 s.
            let t = bottom_at.expect("reached the bottom");
            assert!((0.25..=0.36).contains(&t), "{class}: bottom at {t} s");
        }
    }

    #[test]
    fn prismatic_slides_only_along_its_axis_within_limits() {
        // Axis X along world Y (attachments turned): the bob drops to the limit.
        let rot = CFrame::new(Vector3::new(0.0, 0.0, 0.0), rbx_dom_weak::types::Matrix3::new(
            Vector3::new(0.0, 1.0, 0.0), Vector3::new(-1.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 1.0)));
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let top = doc.dom.insert(ws, part("Top", Vec3::new(0.0, 300.0, 0.0), true));
        let bob = doc.dom.insert(ws, part("Bob", Vec3::new(0.0, 300.0, 0.0), false).with_property("CanCollide", false));
        let a0 = doc.dom.insert(top, InstanceBuilder::new("Attachment").with_property("CFrame", rot));
        let a1 = doc.dom.insert(bob, InstanceBuilder::new("Attachment").with_property("CFrame", rot));
        doc.dom.insert(top, InstanceBuilder::new("PrismaticConstraint")
            .with_property("Attachment0", a0).with_property("Attachment1", a1)
            .with_property("LimitsEnabled", true)
            .with_property("LowerLimit", 0.0f32).with_property("UpperLimit", 4.0f32));
        let _ = run(&mut doc, 120);
        let b = pos(&doc, bob);
        assert!(b.x.abs() < 0.05 && b.z.abs() < 0.05, "stays on the axis: {b:?}");
        assert!((b.y - 296.0).abs() < 0.2, "rests on the 4-stud limit: {b:?}");
    }

    #[test]
    fn rod_holds_its_length_and_spring_settles_at_its_load() {
        // A rod from the pivot to the bob's centre swings at its length.
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let top = doc.dom.insert(ws, part("Top", Vec3::new(0.0, 300.0, 0.0), true));
        let bob = doc.dom.insert(ws, part("Bob", Vec3::new(0.0, 300.0, 5.0), false));
        let a0 = doc.dom.insert(top, InstanceBuilder::new("Attachment"));
        let a1 = doc.dom.insert(bob, InstanceBuilder::new("Attachment"));
        doc.dom.insert(top, InstanceBuilder::new("RodConstraint")
            .with_property("Attachment0", a0).with_property("Attachment1", a1)
            .with_property("Length", 5.0f32));
        let mut p = Physics::new(&mut doc);
        for i in 0..90 {
            p.sync(&mut doc);
            p.step(&mut doc, 1.0 / 60.0);
            let b = pos(&doc, bob);
            assert!(((b - Vec3::new(0.0, 300.0, 0.0)).length() - 5.0).abs() < 0.25, "rod at {i}: {b:?}");
        }
        // A spring straight down: stiffness k, free length L → rests at L + m g / k.
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let top = doc.dom.insert(ws, part("Top", Vec3::new(0.0, 300.0, 0.0), true));
        let bob = doc.dom.insert(ws, part("Bob", Vec3::new(0.0, 296.0, 0.0), false));
        let a0 = doc.dom.insert(top, InstanceBuilder::new("Attachment"));
        let a1 = doc.dom.insert(bob, InstanceBuilder::new("Attachment"));
        doc.dom.insert(top, InstanceBuilder::new("SpringConstraint")
            .with_property("Attachment0", a0).with_property("Attachment1", a1)
            .with_property("Stiffness", 200.0f32).with_property("Damping", 20.0f32)
            .with_property("FreeLength", 4.0f32));
        let mut p = Physics::new(&mut doc);
        for _ in 0..600 {
            p.sync(&mut doc);
            p.step(&mut doc, 1.0 / 60.0);
        }
        let mass = p.parts.get(&bob).and_then(|pb| pb.asm).and_then(|id| p.asms.get(&id))
            .and_then(|a| p.world.bodies.get(a.body)).map(|b| b.mass()).unwrap();
        let want = 300.0 - 4.0 - mass * p.gravity / 200.0;
        let b = pos(&doc, bob);
        assert!((b.y - want).abs() < 0.05, "spring rests at {want}: {b:?}");
    }
}
