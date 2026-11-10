//! Assemblies a client builds for itself (a game's display models: Place A's
//! lobby crew is a server-made avatar cloned into the client's menu scene,
//! its root anchored and moved by script). Studio's client simulates those;
//! here each frame places their parts from their joints, outward from the
//! anchored part: Part1 = Part0 · C0 · Transform · C1⁻¹ (and the inverse
//! when the anchored side is Part1). Parts the server also has are left
//! alone: server physics and replication own them.

use std::collections::{HashMap, HashSet, VecDeque};

use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::ustr;

use crate::doc::{self, Document};
use crate::math::Xform;

/// Joint classes that hold two parts by C0 / C1.
const JOINTS: [&str; 7] = ["Motor6D", "Weld", "Motor", "ManualWeld", "Snap", "Glue", "AnimationConstraint"];

#[derive(Default)]
pub struct ClientJoints {
    doc_id: u64,
    /// The client-only joints under Workspace, kept up to date from the
    /// document's journal (a full scan of Place B's 116k instances on every
    /// tree change was 3 ms a frame).
    joints: HashSet<Ref>,
    /// Root parts of animated rigs (a Humanoid's HumanoidRootPart, an
    /// AnimationController model's PrimaryPart) the server owns, found
    /// every 60 frames.
    rigs: Vec<Ref>,
    frame: u64,
}

fn is_joint(cd: &Document, r: Ref) -> bool {
    cd.dom
        .get_by_ref(r)
        .is_some_and(|i| JOINTS.contains(&i.class.as_str()))
}

fn compose(a: &Xform, b: &Xform) -> Xform {
    Xform {
        pos: a.apply(b.pos),
        rot: a.rot * b.rot,
    }
}

fn cframe(d: &Document, r: Ref, k: &str) -> Option<Xform> {
    match d.dom.get_by_ref(r)?.properties.get(&ustr(k)) {
        Some(Variant::CFrame(c)) => Some(Xform::from_cframe(c)),
        _ => None,
    }
}

fn part_ref(d: &Document, j: Ref, k: &str) -> Option<Ref> {
    match d.dom.get_by_ref(j)?.properties.get(&ustr(k)) {
        Some(Variant::Ref(r)) if r.is_some() && d.dom.get_by_ref(*r).is_some() => Some(*r),
        _ => None,
    }
}

fn anchored(d: &Document, p: Ref) -> bool {
    matches!(d.prop(p, "Anchored"), Some(Variant::Bool(true)))
}

impl ClientJoints {
    /// Places the client-only parts of `cd` (the client's DataModel) from
    /// their joints. `c2s` maps client instances the server also has.
    pub fn solve(&mut self, cd: &mut Document, c2s: &HashMap<Ref, Ref>) {
        self.pose_rigs(cd, c2s);
        let marks = cd.marks_for(doc::Reader::ClientJoints);
        let ws = cd.find_child(cd.dom.root_ref(), "Workspace");
        let wanted = |cd: &Document, r: Ref| {
            !c2s.contains_key(&r) && is_joint(cd, r) && ws.is_some_and(|w| cd.is_ancestor(w, r))
        };
        match marks {
            doc::Marks::Some(marks) if cd.id == self.doc_id => {
                for m in marks {
                    let doc::Mark::Tree(t) = m else { continue };
                    for r in cd.subtree(t) {
                        if wanted(cd, r) {
                            self.joints.insert(r);
                        } else {
                            self.joints.remove(&r);
                        }
                    }
                }
            }
            _ => {
                self.doc_id = cd.id;
                self.joints = match ws {
                    Some(ws) => cd
                        .dom
                        .descendants_of(ws)
                        .filter(|i| JOINTS.contains(&i.class.as_str()))
                        .map(|i| i.referent())
                        .filter(|r| !c2s.contains_key(r))
                        .collect(),
                    None => HashSet::new(),
                };
            }
        }
        if self.joints.is_empty() {
            return;
        }
        // (joint, part0, part1, C0 · Transform, C1)
        let mut edges: Vec<(Ref, Ref, Xform, Xform)> = Vec::new();
        let mut by_part: HashMap<Ref, Vec<usize>> = HashMap::new();
        for &j in &self.joints {
            if matches!(cd.prop(j, "Enabled"), Some(Variant::Bool(false))) {
                continue;
            }
            let Some((p0, p1, c0, c1)) = crate::rig::joint_frames(cd, j) else {
                continue;
            };
            let is_part = |r: Ref| {
                cd.dom
                    .get_by_ref(r)
                    .is_some_and(|i| doc::is_a(&i.class, "BasePart"))
            };
            if p0 == p1 || !is_part(p0) || !is_part(p1) {
                continue;
            }
            let ident = Xform::from_cframe(&rbx_dom_weak::types::CFrame::identity());
            let t = cframe(cd, j, "Transform").unwrap_or(ident);
            let i = edges.len();
            edges.push((p0, p1, compose(&c0, &t), c1));
            by_part.entry(p0).or_default().push(i);
            by_part.entry(p1).or_default().push(i);
        }
        // Outward from each anchored part; parts with no anchored part in
        // their assembly would fall in Studio and are left where they are.
        let mut placed: HashMap<Ref, Xform> = HashMap::new();
        let mut queue: VecDeque<Ref> = VecDeque::new();
        for &p in by_part.keys() {
            if anchored(cd, p) {
                if let Some(xf) = cframe(cd, p, "CFrame") {
                    placed.insert(p, xf);
                    queue.push_back(p);
                }
            }
        }
        let mut moved: HashSet<Ref> = HashSet::new();
        while let Some(p) = queue.pop_front() {
            let at = placed[&p];
            for &i in by_part.get(&p).map(|v| v.as_slice()).unwrap_or(&[]) {
                let (p0, p1, c0t, c1) = edges[i];
                let (other, xf) = if p == p0 {
                    (p1, compose(&compose(&at, &c0t), &c1.inverse()))
                } else {
                    (p0, compose(&compose(&at, &c1), &c0t.inverse()))
                };
                if placed.contains_key(&other) || anchored(cd, other) {
                    continue;
                }
                placed.insert(other, xf);
                queue.push_back(other);
                if !c2s.contains_key(&other) {
                    moved.insert(other);
                }
            }
        }
        for p in moved {
            let xf = placed[&p];
            let old = cframe(cd, p, "CFrame");
            let same = old.is_some_and(|o| {
                (o.pos - xf.pos).length() < 1e-4 && o.rot.abs_diff_eq(xf.rot, 1e-5)
            });
            if same {
                continue;
            }
            if let Some(inst) = cd.dom.get_by_ref_mut(p) {
                inst.properties
                    .insert(ustr("CFrame"), Variant::CFrame(xf.to_cframe()));
            }
            cd.mark_prop(p, ustr("CFrame"));
        }
    }
}

impl ClientJoints {
    /// Server-owned rigs as the client sees them: each frame its own
    /// simulation poses them from their Motor6D.Transform (what the
    /// Animator wrote, replicated), outward from the root part the server
    /// moves. Studio animates NPCs on every peer; without this a server
    /// NPC's limbs stayed in the rest pose on the client (Place A's enemies).
    fn pose_rigs(&mut self, cd: &mut Document, c2s: &HashMap<Ref, Ref>) {
        if self.frame % 60 == 0 || cd.id != self.doc_id {
            self.rigs = find_rigs(cd);
        }
        self.frame += 1;
        let ident = Xform::from_cframe(&rbx_dom_weak::types::CFrame::identity());
        let mut writes: Vec<(Ref, Xform)> = Vec::new();
        for &root in &self.rigs {
            if !c2s.contains_key(&root) {
                continue;
            }
            let Some(model) = cd.dom.get_by_ref(root).map(|i| i.parent()) else {
                continue;
            };
            // (A root left unparented when its character was destroyed:
            // the rig list is refreshed only every 60 frames.)
            if cd.dom.get_by_ref(model).is_none() {
                continue;
            }
            let Some(at) = cframe(cd, root, "CFrame") else { continue };
            let mut edges: Vec<(Ref, Ref, Xform, Xform)> = Vec::new();
            for i in cd.dom.descendants_of(model) {
                if !JOINTS.contains(&i.class.as_str()) {
                    continue;
                }
                let j = i.referent();
                if matches!(cd.prop(j, "Enabled"), Some(Variant::Bool(false))) {
                    continue;
                }
                let Some((p0, p1, c0, c1)) = crate::rig::joint_frames(cd, j) else {
                    continue;
                };
                let t = cframe(cd, j, "Transform").unwrap_or(ident);
                edges.push((p0, p1, compose(&c0, &t), c1));
            }
            let mut placed: HashMap<Ref, Xform> = HashMap::new();
            placed.insert(root, at);
            let mut queue = VecDeque::from([root]);
            while let Some(p) = queue.pop_front() {
                let at = placed[&p];
                for &(p0, p1, c0t, c1) in &edges {
                    let (other, xf) = if p == p0 {
                        (p1, compose(&compose(&at, &c0t), &c1.inverse()))
                    } else if p == p1 {
                        (p0, compose(&compose(&at, &c1), &c0t.inverse()))
                    } else {
                        continue;
                    };
                    if placed.contains_key(&other) || anchored(cd, other) {
                        continue;
                    }
                    placed.insert(other, xf);
                    queue.push_back(other);
                    writes.push((other, xf));
                }
            }
        }
        for (p, xf) in writes {
            let same = cframe(cd, p, "CFrame").is_some_and(|o| {
                (o.pos - xf.pos).length() < 1e-4 && o.rot.abs_diff_eq(xf.rot, 1e-5)
            });
            if same {
                continue;
            }
            if let Some(inst) = cd.dom.get_by_ref_mut(p) {
                inst.properties.insert(ustr("CFrame"), Variant::CFrame(xf.to_cframe()));
            }
            cd.mark_prop(p, ustr("CFrame"));
        }
    }
}

/// Root parts of the animated rigs under Workspace.
fn find_rigs(cd: &Document) -> Vec<Ref> {
    let Some(ws) = cd.find_child(cd.dom.root_ref(), "Workspace") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in cd.dom.descendants_of(ws) {
        if i.class != "Humanoid" && i.class != "AnimationController" {
            continue;
        }
        let model = i.parent();
        let root = cd.find_named(model, "HumanoidRootPart").filter(|r| {
            cd.dom.get_by_ref(*r).is_some_and(|p| p.parent() == model)
        });
        let root = root.or_else(|| part_ref(cd, model, "PrimaryPart"));
        if let Some(r) = root {
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use rbx_dom_weak::types::{CFrame, Matrix3, Vector3};
    use rbx_dom_weak::InstanceBuilder;

    fn at(x: f32, y: f32, z: f32) -> CFrame {
        CFrame::new(Vector3::new(x, y, z), Matrix3::identity())
    }

    /// An anchored root moved by script carries a client-made limb with it.
    #[test]
    fn limbs_follow_an_anchored_root() {
        let mut d = Document::new_place();
        let root = d.dom.root_ref();
        let ws = d.find_child(root, "Workspace").unwrap();
        let model = d.dom.insert(ws, InstanceBuilder::new("Model"));
        let hrp = d.dom.insert(
            model,
            InstanceBuilder::new("Part")
                .with_property("Anchored", true)
                .with_property("CFrame", at(10.0, 3.0, 9000.0)),
        );
        let torso = d.dom.insert(
            model,
            InstanceBuilder::new("Part").with_property("CFrame", at(0.0, 0.0, 0.0)),
        );
        let arm = d.dom.insert(
            model,
            InstanceBuilder::new("Part").with_property("CFrame", at(0.0, 0.0, 0.0)),
        );
        d.dom.insert(
            torso,
            InstanceBuilder::new("Motor6D")
                .with_property("Part0", hrp)
                .with_property("Part1", torso)
                .with_property("C0", at(0.0, 1.0, 0.0))
                .with_property("C1", at(0.0, 0.0, 0.0)),
        );
        // Joined the other way round: the limb is Part0.
        d.dom.insert(
            arm,
            InstanceBuilder::new("Weld")
                .with_property("Part0", arm)
                .with_property("Part1", torso)
                .with_property("C0", at(0.0, 0.5, 0.0))
                .with_property("C1", at(1.5, 0.0, 0.0)),
        );
        d.mark_all();
        let mut cj = ClientJoints::default();
        cj.solve(&mut d, &HashMap::new());
        let pos = |p| cframe(&d, p, "CFrame").unwrap().pos;
        assert!((pos(torso) - Vec3::new(10.0, 4.0, 9000.0)).length() < 1e-4);
        assert!((pos(arm) - Vec3::new(11.5, 3.5, 9000.0)).length() < 1e-4);
        assert!((pos(hrp) - Vec3::new(10.0, 3.0, 9000.0)).length() < 1e-4);
    }
}
