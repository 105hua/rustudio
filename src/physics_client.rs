//! The client's own simulation. Studio's client simulates the instances
//! only it has: parts a LocalScript made or cloned. Place E's ClientObjectScripts
//! clone the spawn lantern and its ropes and the JToH rope platform, then
//! unanchor the clones, and PotOfStew clones the Ingredients folder whose
//! orbit ingredients the airstream script ropes to tweened anchors; nothing
//! moved any of them here, so they hung stiff.
//!
//! What the server has stays the server's, even where the client unanchored
//! it: Studio's client never simulates a replicated part (checked in Studio
//! Play: a replicated part unanchored on the client
//! doesn't fall, a client ApplyImpulse on it is ignored, and a replicated
//! part roped to a tweened client anchor stays put).
//!
//! Studio's client keeps a full world of its own: replicated assemblies it
//! doesn't own take their poses from the network (PhysicsReceiver) and only
//! what it owns is integrated. A full copy here (every part of the client
//! DataModel as a collider) measured 1.3–1.5 ms a frame in Place B (50k parts)
//! even with nothing in rapier's step walking all colliders: following ~850
//! animated NPC limbs a frame (0.35 ms) and its ~60 rolling tumbleweeds in a
//! 51k-leaf tree. So the client world holds just the client-local parts plus
//! the replicated ones near a client body: each client body's AABB (grown by
//! its speed) is looked up in the server world's broad phase every few
//! frames (the server world has the same replicated geometry), and the parts
//! found come in as fixed colliders that follow their replicated CFrames
//! (`external`); the rest stay out (`skip`). 0.9 ms in Place B.
use super::*;
use rapier3d::geometry::BoundingVolume;

/// Frames between neighbourhood updates.
const NEAR_EVERY: u64 = 6;
/// A neighbour stays this many frames after leaving every neighbourhood
/// (a body swinging at its edge would otherwise rebuild it on and off).
const NEAR_KEEP: u64 = 30;
/// Studs around a client body that its neighbourhood covers, plus this many
/// seconds of its speed (a body crosses it in more than NEAR_EVERY frames).
const NEAR_MARGIN: f32 = 6.0;
const NEAR_LEAD_S: f32 = 0.5;

/// The client DataModel's simulation (built on first use).
#[derive(Default)]
pub struct ClientSim {
    physics: Option<Physics>,
    /// Replicated parts in the client world now (as fixed followers), with
    /// the frame each was last inside a client body's neighbourhood.
    near: HashMap<Ref, u64>,
    frame: u64,
}

impl ClientSim {
    pub fn physics(&self) -> Option<&Physics> {
        self.physics.as_ref()
    }

    /// Whether this world has bodies of its own to step.
    pub fn has_bodies(&self) -> bool {
        self.physics.as_ref().is_some_and(|p| !p.asms.is_empty())
    }

    /// The motion of the parts this world simulates (the client's own:
    /// not replicated, not following a replicated assembly): part →
    /// (linear velocity, angular velocity, assembly centre of mass). Client
    /// scripts read these (AssemblyLinearVelocity / AssemblyAngularVelocity),
    /// the replicated parts' come from the server, as in Studio.
    pub fn own_motion(&self, c2s: &HashMap<Ref, Ref>) -> HashMap<Ref, (Vec3, Vec3, Vec3)> {
        let mut out = HashMap::new();
        let Some(p) = self.physics.as_ref() else { return out };
        for a in p.asms.values() {
            let Some(b) = p.world.bodies.get(a.body) else { continue };
            if !b.is_dynamic() {
                continue;
            }
            let m = (gv(b.linvel()), gv(b.angvel()), gv(b.center_of_mass()));
            for r in &a.parts {
                if !p.external.contains(r) && !c2s.contains_key(r) {
                    out.insert(*r, m);
                }
            }
        }
        out
    }

    /// One client frame: syncs the client DataModel's changes, keeps the
    /// replicated parts near its own bodies in its world, and steps.
    /// `vel`: velocities client scripts set (they only reach the client's
    /// own parts, as in Studio).
    pub fn step(
        &mut self,
        server: &Physics,
        cd: &mut Document,
        s2c: &HashMap<Ref, Ref>,
        c2s: &HashMap<Ref, Ref>,
        vel: &[(Ref, Vec3)],
        dt: f32,
    ) -> StepResult {
        // A/B switch for perf checks.
        if std::env::var_os("RUSTUDIO_NO_CLIENT_PHYSICS").is_some() {
            return StepResult::default();
        }
        self.frame += 1;
        let p = self.physics.get_or_insert_with(|| {
            // Everything replicated stays out until a client body comes near.
            let skip: HashSet<Ref> = match cd.workspace() {
                Some(ws) => cd
                    .dom
                    .descendants_of(ws)
                    .filter(|i| doc::is_a(&i.class, "BasePart") && c2s.contains_key(&i.referent()))
                    .map(|i| i.referent())
                    .collect(),
                None => HashSet::new(),
            };
            let mut p = Physics::new_skipping(cd, skip);
            p.groups = server.groups.clone();
            p.fresh = Some(Vec::new());
            p.follower_eps = (FOLLOWER_EPS_POS, FOLLOWER_EPS_ROT);
            p
        });
        p.char_group = server.char_group.clone();
        let sc = crate::bench::scope("pc_sync");
        p.sync(cd);
        drop(sc);
        let _ex = crate::bench::scope("pc_near");
        let mut requeue: Vec<Ref> = Vec::new();
        // A new body of the client's gets its neighbourhood before it moves
        // (arriving later, a neighbour can appear inside it and kick it).
        let mut new_body = false;
        // Parts built since last frame: replicated ones not near go back out;
        // a client part welded to a replicated one is the server's assembly.
        let fresh = p.fresh.replace(Vec::new()).unwrap_or_default();
        for r in fresh {
            let Some(pb) = p.parts.get(&r) else { continue };
            if pb.asm.is_some() && !c2s.contains_key(&r) {
                new_body = true;
            }
            if c2s.contains_key(&r) {
                if !self.near.contains_key(&r) {
                    p.skip.insert(r);
                    requeue.push(r);
                } else if p.external.insert(r) {
                    requeue.push(r);
                }
            } else if pb.asm.is_some() && !p.external.contains(&r) {
                let mut group = vec![r];
                let mut seen: HashSet<Ref> = HashSet::from([r]);
                let mut i = 0;
                while i < group.len() {
                    let n: Vec<Ref> = p.neighbours(group[i]).collect();
                    for x in n {
                        if seen.insert(x) {
                            group.push(x);
                        }
                    }
                    i += 1;
                }
                if group.iter().any(|x| c2s.contains_key(x)) {
                    for x in group {
                        if !c2s.contains_key(&x) && p.external.insert(x) {
                            requeue.push(x);
                        }
                    }
                }
            }
        }
        // The neighbourhood of the client's own bodies.
        if self.frame % NEAR_EVERY == 1 || !requeue.is_empty() || new_body {
            let mut regions: Vec<rp::Aabb> = Vec::new();
            for a in p.asms.values() {
                let Some(b) = p.world.bodies.get(a.body) else { continue };
                let mut bb: Option<rp::Aabb> = None;
                for r in &a.parts {
                    if let Some(c) = p.parts.get(r).and_then(|pb| p.world.colliders.get(pb.collider)) {
                        let x = c.compute_aabb();
                        bb = Some(match bb {
                            Some(mut m) => {
                                m.merge(&x);
                                m
                            }
                            None => x,
                        });
                    }
                }
                if let Some(bb) = bb {
                    let lead = gv(b.linvel()).length() * NEAR_LEAD_S;
                    regions.push(bb.loosened(NEAR_MARGIN + lead));
                }
            }
            let mut now: HashSet<Ref> = HashSet::new();
            if !regions.is_empty() {
                let qp = server.world.query_pipeline();
                for reg in &regions {
                    for (h, _) in qp.intersect_aabb_conservative(*reg) {
                        if let Some(c) = server.by_collider.get(&h).and_then(|s| s2c.get(s)) {
                            now.insert(*c);
                        }
                    }
                }
                // The character isn't in the server's world (it moves it
                // itself): its parts by their client poses.
                for s in &server.skip {
                    let Some(c) = s2c.get(s) else { continue };
                    let Some(xf) = xform_of(cd, *c) else { continue };
                    let pt = rv(xf.pos);
                    if regions.iter().any(|reg| reg.loosened(4.0).contains_local_point(pt)) {
                        now.insert(*c);
                    }
                }
            }
            let frame = self.frame;
            for r in &now {
                if self.near.insert(*r, frame).is_none() {
                    p.skip.remove(r);
                    p.external.insert(*r);
                    requeue.push(*r);
                }
            }
            self.near.retain(|r, seen| {
                if frame - *seen <= NEAR_KEEP {
                    return true;
                }
                p.skip.insert(*r);
                p.external.remove(r);
                requeue.push(*r);
                false
            });
        }
        if !requeue.is_empty() {
            p.queued.extend(requeue);
            p.sync(cd);
            // What that sync built is already sorted.
            if let Some(f) = p.fresh.as_mut() {
                f.retain(|r| !(c2s.contains_key(r) && self.near.contains_key(r)));
            }
        }
        drop(_ex);
        for (r, v) in vel {
            if !p.external.contains(r) && !c2s.contains_key(r) {
                p.set_velocity(*r, *v);
            }
        }
        // Nothing of its own to move: skip the solver (the usual case).
        if p.asms.is_empty() {
            return StepResult::default();
        }
        let _sc = crate::bench::scope("pc_step");
        p.step(cd, dt)
    }
}

