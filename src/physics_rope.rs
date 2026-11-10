//! Unilateral rope impulses, applied at attachment points to preserve swing.
//! The Rapier integration here is an approximation, not a recovered Roblox solver.
use super::*;

impl Physics {
    fn rope_endpoint(&self, doc: &Document, att: Ref) -> Option<(Option<rp::RigidBodyHandle>, Vec3)> {
        let mut part = doc.dom.get_by_ref(att)?.parent();
        while let Some(i) = doc.dom.get_by_ref(part) {
            if doc::is_a(&i.class, "BasePart") { break; }
            part = i.parent();
        }
        let world = crate::attachment::world(doc, att, false)?.pos;
        let local = xform_of(doc, part).map(|xf| xf.rot.transpose() * (world - xf.pos));
        let point = match (local, self.part_xform(doc, part)) {
            (Some(local), Some(xf)) => xf.apply(local),
            _ => world,
        };
        Some((self.body_of(part), point))
    }

    pub(super) fn apply_ropes(&mut self, doc: &Document, dt: f32) {
        if self.ropes.is_empty() { return; }
        let Some(ws) = doc.workspace() else { return; };
        // Drop ropes destroyed or moved out of Workspace since they were seen.
        self.ropes.retain(|&r| {
            let Some(i) = doc.dom.get_by_ref(r) else { return false };
            if i.class != "RopeConstraint" && i.class != "RodConstraint" { return false; }
            let mut p = i.parent();
            while let Some(a) = doc.dom.get_by_ref(p) {
                if p == ws { return true; }
                p = a.parent();
            }
            false
        });
        let ropes: Vec<Ref> = self.ropes.iter().copied().collect();
        for r in ropes {
            if matches!(doc.prop(r, "Enabled"), Some(Variant::Bool(false))) { continue; }
            let get = |key| match doc.prop(r, key) { Some(Variant::Ref(a)) => Some(a), _ => None };
            let Some((h0, p0)) = get("Attachment0").and_then(|a| self.rope_endpoint(doc, a)) else { continue; };
            let Some((h1, p1)) = get("Attachment1").and_then(|a| self.rope_endpoint(doc, a)) else { continue; };
            if h0 == h1 { continue; }
            // A body asleep on its rope has no gravity to hold: leave it
            // asleep (a taut rope's impulse woke it every step, so nothing
            // hanging on a rope ever slept).
            let awake = |h: Option<rp::RigidBodyHandle>| {
                h.and_then(|h| self.world.bodies.get(h)).is_some_and(|b| b.is_dynamic() && !b.is_sleeping())
            };
            if !awake(h0) && !awake(h1) { continue; }
            let delta = p1 - p0;
            let distance = delta.length();
            if distance < 1e-6 { continue; }
            let n = rv(delta / distance);
            let mut inverse_mass = 0.0;
            let mut separating = 0.0;
            let mut gravity_velocity = 0.0;
            for (h, p, sign) in [(h0, p0, -1.0), (h1, p1, 1.0)] {
                if let Some(b) = h.and_then(|h| self.world.bodies.get(h)) {
                    separating += sign * b.velocity_at_point(rv(p)).dot(n);
                    if b.is_dynamic() {
                        gravity_velocity += sign * self.world.gravity.dot(n) * dt;
                        let m = b.mass_properties();
                        let arm = rv(p) - b.center_of_mass();
                        let torque = arm.cross(n);
                        inverse_mass += n.dot(m.effective_inv_mass * n)
                            + torque.dot(m.effective_world_inv_inertia * torque);
                    }
                }
            }
            if inverse_mass <= 0.0 { continue; }
            let current_separating = separating;
            separating += gravity_velocity;
            let length = num(doc, r, "Length", 5.0).max(0.0);
            let error = distance - length;
            // A RodConstraint holds its length both ways (pushes too).
            if doc.class_of(r).is_some_and(|c| c == "RodConstraint") {
                let impulse = (separating + 0.2 * error / dt) / inverse_mass;
                if impulse.is_finite() && impulse != 0.0 {
                    for (h, p, sign) in [(h0, p0, 1.0), (h1, p1, -1.0)] {
                        if let Some(b) = h.and_then(|h| self.world.bodies.get_mut(h)) {
                            b.apply_impulse_at_point(n * (sign * impulse), rv(p), true);
                        }
                    }
                }
                continue;
            }
            // A slack rope permits separation until it reaches its limit.
            let restitution = num(doc, r, "Restitution", 0.0).clamp(0.0, 1.0);
            let desired = if error < 0.0 { -error / dt } else { -0.2 * error / dt };
            let bounce = if error >= -0.001 && separating > 1.0 { restitution * separating } else { 0.0 };
            let mut impulse = ((separating - desired + bounce) / inverse_mass).max(0.0);
            if matches!(doc.prop(r, "WinchEnabled"), Some(Variant::Bool(true))) {
                let response = num(doc, r, "WinchResponsiveness", 5.0).clamp(5.0, 200.0);
                let target = num(doc, r, "WinchTarget", 5.0).max(0.0).min(length);
                let speed = num(doc, r, "WinchSpeed", 2.0).max(0.0);
                let force = num(doc, r, "WinchForce", 10000.0).max(0.0);
                // Critically damped coefficients recovered in constructor;
                // force control integration is an implementation approximation.
                let closing = -current_separating;
                let velocity = (closing + (0.25 * response * response * (distance - target)
                    - response * closing) * dt).clamp(-speed, speed);
                let pull = (velocity + separating).max(0.0);
                impulse = impulse.max((pull / inverse_mass).clamp(0.0, force * dt));
            }
            if impulse <= 0.0 || !impulse.is_finite() { continue; }
            for (h, p, sign) in [(h0, p0, 1.0), (h1, p1, -1.0)] {
                if let Some(b) = h.and_then(|h| self.world.bodies.get_mut(h)) {
                    b.apply_impulse_at_point(n * (sign * impulse), rv(p), true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::{InstanceBuilder, types::{CFrame, Vector3}};
    fn fixture(winch: bool) -> (Document, Physics, Ref) {
        let mut d = Document::new_place();
        let ws = d.workspace().unwrap();
        let anchor = d.dom.insert(ws, InstanceBuilder::new("Part").with_property("Anchored", true)
            .with_property("CFrame", CFrame::new(Vector3::new(0.0, 30.0, 0.0), CFrame::identity().orientation)));
        let bob = d.dom.insert(ws, InstanceBuilder::new("Part").with_property("CanCollide", false)
            .with_property("Size", Vector3::new(1.0, 1.0, 1.0))
            .with_property("CFrame", CFrame::new(Vector3::new(1.0, 20.05, 0.0), CFrame::identity().orientation)));
        let a = d.dom.insert(anchor, InstanceBuilder::new("Attachment"));
        let b = d.dom.insert(bob, InstanceBuilder::new("Attachment"));
        d.dom.insert(ws, InstanceBuilder::new("RopeConstraint").with_property("Attachment0", a)
            .with_property("Attachment1", b).with_property("Length", 10.0f32)
            .with_property("WinchEnabled", winch).with_property("WinchTarget", 5.0f32)
            .with_property("WinchResponsiveness", 50.0f32).with_property("WinchForce", 100000.0f32));
        let p = Physics::new(&mut d);
        (d, p, bob)
    }
    #[test]
    fn pendulum_period() {
        let (mut d, mut p, bob) = fixture(false);
        let dt = 1.0/240.0;
        let mut crossings = Vec::new();
        let mut previous = 1.0;
        for step in 0..2400 {
            p.step(&mut d, dt);
            let x = xform_of(&d, bob).unwrap().pos.x;
            if x < 0.0 && previous >= 0.0 { crossings.push(step as f32*dt); }
            previous=x;
        }
        let expected = std::f32::consts::TAU*(10.0/196.2f32).sqrt();
        assert!(crossings.len()>=3, "{crossings:?}");
        let period = crossings[2]-crossings[1];
        assert!((period-expected).abs()<0.12, "{period} vs {expected}");
    }
    #[test]
    fn winch_lifts_bob() {
        let (mut d, mut p, bob) = fixture(true);
        for _ in 0..600 { p.step(&mut d, 1.0/60.0); }
        let distance = xform_of(&d, bob).unwrap().pos.distance(Vec3::new(0.0,30.0,0.0));
        assert!(distance<6.0, "distance={distance}");
    }
}
