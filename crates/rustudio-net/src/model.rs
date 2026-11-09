//! The live CloudEdit DataModel a client builds from the replicator stream.
//!
//! [`LiveModel`] holds an `rbx_dom_weak::WeakDom` (the same tree type the app
//! edits) and applies [`RepOp`]s to it. It resolves parents that arrive
//! before their children by parking orphans until the parent exists. This is
//! the read-only target for Phase 1: feed it the stream and hand the `WeakDom`
//! to the app for the Explorer and the 3D view.

use crate::replicator::{PropValue, RefMap, RepOp};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use std::collections::HashMap;

pub struct LiveModel {
    dom: WeakDom,
    refs: RefMap,
    /// Children whose parent referent hasn't arrived yet.
    pending: HashMap<u64, Vec<PendingInstance>>,
    /// Server referent of the DataModel root, once known.
    root_server_id: Option<u64>,
    pending_refs: HashMap<u64, Vec<(Ref, String)>>,
    pending_ref_targets: HashMap<(Ref, String), u64>,
    /// Union geometry from the 0xa5 channel, by reference key.
    net_assets: HashMap<u64, rbx_types::NetAssetRef>,
    /// Unions whose holder names a key not arrived yet.
    waiting_assets: HashMap<u64, Vec<Ref>>,
    waiting_asset_targets: HashMap<Ref, u64>,
    /// Unions given their geometry since the last `take_resolved`.
    resolved: Vec<Ref>,
}

struct PendingInstance {
    id: u64,
    class: String,
    props: Vec<(String, PropValue)>,
}

impl Default for LiveModel {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveModel {
    pub fn new() -> Self {
        // The root stands in for the server's DataModel.
        let dom = WeakDom::new(InstanceBuilder::new("DataModel").with_name("game"));
        LiveModel {
            dom,
            refs: RefMap::new(),
            pending: HashMap::new(),
            root_server_id: None,
            pending_refs: HashMap::new(),
            pending_ref_targets: HashMap::new(),
            net_assets: HashMap::new(),
            waiting_assets: HashMap::new(),
            waiting_asset_targets: HashMap::new(),
            resolved: Vec::new(),
        }
    }

    pub fn dom(&self) -> &WeakDom {
        &self.dom
    }
    pub fn dom_mut(&mut self) -> &mut WeakDom {
        &mut self.dom
    }
    pub fn server_id(&self, reference: Ref) -> Option<u64> {
        self.refs.server_id(reference)
    }
    /// Bind a locally created DOM instance before its server echo arrives.
    pub fn bind_local(&mut self, id: u64, reference: Ref) {
        self.refs.insert(id, reference);
    }
    /// Unions that got their geometry since the last call (for change marks).
    pub fn take_resolved(&mut self) -> Vec<Ref> {
        std::mem::take(&mut self.resolved)
    }

    pub fn reference(&self, id: u64) -> Option<Ref> {
        self.refs.get(id)
    }
    pub fn instance_count(&self) -> usize {
        self.refs.len()
    }

    /// Apply one op. Unknown/undecoded ops are ignored safely.
    pub fn apply(&mut self, op: RepOp) {
        match op {
            RepOp::JoinDataBegin => {}
            RepOp::NewInstance {
                id,
                class,
                parent,
                props,
            } => self.new_instance(id, class, parent, props),
            RepOp::ChangeProperty { id, name, value } => {
                if let Some(r) = self.refs.get(id) {
                    self.apply_props(r, &[(name, value)]);
                }
            }
            RepOp::DeleteInstance { id } => {
                if let Some(r) = self.refs.get(id) {
                    if r != self.dom.root_ref() && self.dom.get_by_ref(r).is_some() {
                        self.dom.destroy(r);
                    }
                }
            }
            RepOp::Reparent { id, parent } => {
                if let (Some(r), Some(p)) = (
                    self.refs.get(id),
                    if parent == 0 {
                        Some(self.dom.root_ref())
                    } else {
                        self.refs.get(parent)
                    },
                ) {
                    if r != self.dom.root_ref() && r != p {
                        self.dom.transfer_within(r, p);
                    }
                }
            }
            RepOp::NetAsset { key, data } => {
                let asset = rbx_types::NetAssetRef::new(data);
                for r in self.waiting_assets.remove(&key).unwrap_or_default() {
                    if self.waiting_asset_targets.get(&r) != Some(&key) { continue; }
                    self.waiting_asset_targets.remove(&r);
                    if let Some(inst) = self.dom.get_by_ref_mut(r) {
                        inst.properties.insert("SolidMeshHolder".into(), Variant::NetAssetRef(asset.clone()));
                        self.resolved.push(r);
                    }
                }
                self.net_assets.insert(key, asset);
            }
            RepOp::Presence { .. } | RepOp::Event { .. } | RepOp::Unhandled { .. } => {}
        }
    }

    fn new_instance(
        &mut self,
        id: u64,
        class: String,
        parent: Option<u64>,
        props: Vec<(String, PropValue)>,
    ) {
        if let Some(reference) = self
            .refs
            .get(id)
            .filter(|r| self.dom.get_by_ref(*r).is_some())
        {
            self.apply_props(reference, &props);
            if let Some(parent) = parent
                .and_then(|p| self.refs.get(p))
                .filter(|p| self.dom.get_by_ref(*p).is_some())
            {
                if reference != self.dom.root_ref() && reference != parent {
                    self.dom.transfer_within(reference, parent);
                }
            }
            return;
        }
        // The first parent-less instance is the DataModel root itself.
        let parent_ref = match parent {
            None => {
                if self.root_server_id.is_none() {
                    self.root_server_id = Some(id);
                    self.refs.insert(id, self.dom.root_ref());
                    self.resolve_pending_refs(id, self.dom.root_ref());
                    self.apply_props(self.dom.root_ref(), &props);
                    self.drain_pending(id);
                    return;
                }
                self.dom.root_ref()
            }
            Some(pid) => match self.refs.get(pid) {
                Some(r) => r,
                // The wire never sends the DataModel itself: services arrive
                // with its referent as their parent (13 in the captured join).
                // Workspace's parent is always the DataModel, so bind it there.
                None if class == "Workspace" && self.root_server_id.is_none() => {
                    self.root_server_id = Some(pid);
                    self.refs.insert(pid, self.dom.root_ref());
                    self.drain_pending(pid);
                    self.dom.root_ref()
                }
                None => {
                    // Parent not here yet: park it.
                    self.pending
                        .entry(pid)
                        .or_default()
                        .push(PendingInstance { id, class, props });
                    return;
                }
            },
        };
        let r = self.insert_child(parent_ref, id, &class, &props);
        self.drain_pending_into(id, r);
    }

    fn insert_child(
        &mut self,
        parent: Ref,
        id: u64,
        class: &str,
        props: &[(String, PropValue)],
    ) -> Ref {
        // Replication omits Name when it matches the native constructor default.
        // Native defaults can differ from the class name used by InstanceBuilder.
        let default_name = match class {
            "BindableEvent" => "Event",
            "BindableFunction" => "Function",
            "WedgePart" => "Wedge",
            "CornerWedgePart" => "CornerWedge",
            "UnionOperation" => "Union",
            _ => class,
        };
        let mut builder = InstanceBuilder::new(class).with_name(default_name);
        // An explicit wire Name always wins, including an empty name.
        if let Some((_, PropValue::String(name))) = props.iter().find(|(k, _)| k == "Name") {
            builder = builder.with_name(name.clone());
        }
        let r = self.dom.insert(parent, builder);
        self.refs.insert(id, r);
        self.resolve_pending_refs(id, r);
        self.apply_props(r, props);
        r
    }

    fn apply_props(&mut self, r: Ref, props: &[(String, PropValue)]) {
        let mesh_part = self.dom.get_by_ref(r).is_some_and(|i| i.class.as_str() == "MeshPart");
        for (wire_name, value) in props {
            let name = match wire_name.as_str() {
                // Only MeshPart calls its texture TextureID; ParticleEmitter,
                // Beam, Trail, Decal and SpecialMesh read TextureContent
                // (renaming it for every class lost every emitter's texture).
                "TextureContent" if !mesh_part => "TextureContent",
                "size" => "Size",
                "shape" => "Shape",
                "formFactorRaw" => "FormFactor",
                "Color3uint8" => "Color",
                "MaterialVariantSerialized" => "MaterialVariant",
                "CollisionGroupReplicate" => "CollisionGroup",
                "AttributesReplicate" => "Attributes",
                "RenderFidelityReplicate" => "RenderFidelity",
                "MeshContent" if mesh_part => "MeshId",
                "TextureContent" => "TextureID",
                _ => wire_name.as_str(),
            };
            let name = &name.to_owned();
            if let PropValue::Ref(id) = value {
                // A later update supersedes any unresolved earlier reference.
                self.pending_ref_targets.remove(&(r, name.clone()));
                let resolved = id.and_then(|id| self.refs.get(id));
                if let Some(id) = id.filter(|_| resolved.is_none()) {
                    self.pending_refs
                        .entry(id)
                        .or_default()
                        .push((r, name.clone()));
                    self.pending_ref_targets.insert((r, name.clone()), id);
                }
                if let Some(inst) = self.dom.get_by_ref_mut(r) {
                    inst.properties.insert(
                        name.clone().into(),
                        Variant::Ref(resolved.unwrap_or_else(Ref::none)),
                    );
                }
            } else if let Some(inst) = self.dom.get_by_ref_mut(r) {
                if name == "Name" {
                    if let PropValue::String(value) = value {
                        inst.name = value.clone();
                        continue;
                    }
                }
                if name == "SolidMeshHolder" {
                    self.waiting_asset_targets.remove(&r);
                }
                if let Some(value) = to_variant(value, name) {
                    // A union's geometry reference: its contents if they've
                    // arrived on the 0xa5 channel, else wait for them.
                    if let (Variant::BinaryString(b), true) = (&value, name == "SolidMeshHolder") {
                        if let Some(key) = crate::replicator::holder_key(b.as_ref()) {
                            match self.net_assets.get(&key) {
                                Some(asset) => {
                                    inst.properties.insert(name.clone().into(), Variant::NetAssetRef(asset.clone()));
                                }
                                None => {
                                    inst.properties.remove(&rbx_dom_weak::ustr("SolidMeshHolder"));
                                    self.waiting_asset_targets.insert(r, key);
                                    self.waiting_assets.entry(key).or_default().push(r);
                                }
                            }
                            continue;
                        }
                    }
                    inst.properties.insert(name.clone().into(), value);
                }
            }
        }
    }

    fn resolve_pending_refs(&mut self, id: u64, referent: Ref) {
        if let Some(waiting) = self.pending_refs.remove(&id) {
            for (target, name) in waiting {
                if self.pending_ref_targets.get(&(target, name.clone())) != Some(&id) {
                    continue;
                }
                self.pending_ref_targets.remove(&(target, name.clone()));
                if let Some(inst) = self.dom.get_by_ref_mut(target) {
                    inst.properties.insert(name.into(), Variant::Ref(referent));
                }
            }
        }
    }

    /// A parent `id` now exists at `parent_ref`: attach any parked children.
    fn drain_pending_into(&mut self, id: u64, parent_ref: Ref) {
        if let Some(children) = self.pending.remove(&id) {
            for c in children {
                let r = self.insert_child(parent_ref, c.id, &c.class, &c.props);
                self.drain_pending_into(c.id, r);
            }
        }
    }
    fn drain_pending(&mut self, id: u64) {
        let r = self.refs.get(id).unwrap_or(self.dom.root_ref());
        self.drain_pending_into(id, r);
    }
}

/// The wire value for an editor value whose wire form isn't one of the plain
/// `PropValue`s: the inverse of [`typed_variant`] (plus Content, BrickColor,
/// OptionalCFrame and Tags), so these edits reach a Team Create server. The
/// writer picks the bytes' framing from the server's schema type.
pub fn typed_value(v: &Variant) -> Option<PropValue> {
    let mut b = Vec::new();
    let mut f = |x: f32| b.extend_from_slice(&x.to_le_bytes());
    let typed = |ty: u8, bytes: Vec<u8>| Some(PropValue::Typed { ty, bytes });
    match v {
        Variant::UDim(u) => {
            f(u.scale);
            b.extend_from_slice(&u.offset.to_le_bytes());
            typed(0x0d, b)
        }
        Variant::UDim2(u) => {
            for d in [u.x, u.y] {
                b.extend_from_slice(&d.scale.to_le_bytes());
                b.extend_from_slice(&d.offset.to_le_bytes());
            }
            typed(0x0e, b)
        }
        Variant::Vector3int16(v) => {
            for c in [v.x, v.y, v.z] {
                b.extend_from_slice(&c.to_le_bytes());
            }
            typed(0x19, b)
        }
        Variant::NumberSequence(s) => {
            for k in &s.keypoints {
                f(k.time);
                f(k.value);
                f(k.envelope);
            }
            typed(0x23, b)
        }
        Variant::ColorSequence(s) => {
            for k in &s.keypoints {
                f(k.time);
                f(k.color.r);
                f(k.color.g);
                f(k.color.b);
                f(0.0);
            }
            typed(0x26, b)
        }
        Variant::NumberRange(r) => {
            f(r.min);
            f(r.max);
            typed(0x25, b)
        }
        Variant::Rect(r) => {
            f(r.min.x);
            f(r.min.y);
            f(r.max.x);
            f(r.max.y);
            typed(0x28, b)
        }
        Variant::PhysicalProperties(rbx_types::PhysicalProperties::Default) => typed(0x29, b),
        Variant::PhysicalProperties(rbx_types::PhysicalProperties::Custom(c)) => {
            for x in [c.density(), c.friction(), c.elasticity(), c.friction_weight(), c.elasticity_weight(), c.acoustic_absorption()] {
                f(x);
            }
            typed(0x29, b)
        }
        Variant::Font(font) => {
            b.extend_from_slice(&font.weight.as_u16().to_le_bytes());
            b.push(font.style.as_u8());
            b.extend_from_slice(&(font.family.len() as u32).to_le_bytes());
            b.extend_from_slice(font.family.as_bytes());
            let face = font.cached_face_id.as_deref().unwrap_or("");
            b.extend_from_slice(&(face.len() as u32).to_le_bytes());
            b.extend_from_slice(face.as_bytes());
            typed(0x36, b)
        }
        Variant::Content(c) => match c.value() {
            rbx_types::ContentType::Uri(u) => Some(PropValue::String(u.clone())),
            rbx_types::ContentType::Object(_) => None,
            _ => Some(PropValue::String(String::new())),
        },
        Variant::ContentId(c) => Some(PropValue::String(c.as_str().to_string())),
        Variant::BrickColor(c) => Some(PropValue::Int(i64::from(*c as u16))),
        Variant::OptionalCFrame(None) => Some(PropValue::Raw(Vec::new())),
        Variant::OptionalCFrame(Some(cf)) => Some(PropValue::CFrame {
            pos: [cf.position.x, cf.position.y, cf.position.z],
            rot: [
                cf.orientation.x.x, cf.orientation.x.y, cf.orientation.x.z,
                cf.orientation.y.x, cf.orientation.y.y, cf.orientation.y.z,
                cf.orientation.z.x, cf.orientation.z.y, cf.orientation.z.z,
            ],
        }),
        Variant::Tags(t) => Some(PropValue::SharedString(t.encode())),
        _ => None,
    }
}

/// The Roblox value for a `PropValue::Typed` payload (docs/PROTOCOL.md "Property
/// value encodings"). Before this, these types were kept raw and dropped, so
/// live instances fell back to class defaults: every ParticleEmitter's
/// Size / Transparency / Color / Lifetime / Speed, every GUI's UDim2, Fonts
/// and CustomPhysicalProperties.
fn typed_variant(ty: u8, b: &[u8]) -> Option<Variant> {
    use rbx_types as t;
    let f = |i: usize| -> Option<f32> { Some(f32::from_le_bytes(b.get(i * 4..i * 4 + 4)?.try_into().ok()?)) };
    let i32_at = |o: usize| -> Option<i32> { Some(i32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?)) };
    Some(match ty {
        0x0d => Variant::UDim(t::UDim::new(f(0)?, i32_at(4)?)),
        0x0e => Variant::UDim2(t::UDim2::new(
            t::UDim::new(f(0)?, i32_at(4)?),
            t::UDim::new(f(2)?, i32_at(12)?),
        )),
        0x19 => {
            let s = |o: usize| -> Option<i16> { Some(i16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?)) };
            Variant::Vector3int16(t::Vector3int16::new(s(0)?, s(2)?, s(4)?))
        }
        0x23 => Variant::NumberSequence(t::NumberSequence {
            keypoints: b
                .chunks_exact(12)
                .map(|k| {
                    let g = |i: usize| f32::from_le_bytes(k[i * 4..i * 4 + 4].try_into().unwrap());
                    t::NumberSequenceKeypoint::new(g(0), g(1), g(2))
                })
                .collect(),
        }),
        0x26 => Variant::ColorSequence(t::ColorSequence {
            keypoints: b
                .chunks_exact(20)
                .map(|k| {
                    let g = |i: usize| f32::from_le_bytes(k[i * 4..i * 4 + 4].try_into().unwrap());
                    t::ColorSequenceKeypoint::new(g(0), t::Color3::new(g(1), g(2), g(3)))
                })
                .collect(),
        }),
        0x25 => Variant::NumberRange(t::NumberRange::new(f(0)?, f(1)?)),
        0x28 => Variant::Rect(t::Rect::new(t::Vector2::new(f(0)?, f(1)?), t::Vector2::new(f(2)?, f(3)?))),
        0x29 => match b.len() {
            0 => Variant::PhysicalProperties(t::PhysicalProperties::Default),
            _ => Variant::PhysicalProperties(t::PhysicalProperties::Custom(
                t::CustomPhysicalProperties::new(
                    f(0)?, f(1)?, f(2)?, f(3)?, f(4)?,
                    // AcousticAbsorption: its own field when sent (flag bit 1), else Roblox's default 1.
                    if b.len() >= 24 { f(5)? } else { 1.0 },
                ),
            )),
        },
        0x36 => {
            let weight = u16::from_le_bytes(b.get(0..2)?.try_into().ok()?);
            let style = *b.get(2)?;
            let n = u32::from_le_bytes(b.get(3..7)?.try_into().ok()?) as usize;
            let family = std::str::from_utf8(b.get(7..7 + n)?).ok()?;
            let o = 7 + n;
            let m = u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize;
            let face = std::str::from_utf8(b.get(o + 4..o + 4 + m)?).ok()?;
            let mut font = t::Font::new(
                family,
                t::FontWeight::from_u16(weight).unwrap_or_default(),
                t::FontStyle::from_u8(style).unwrap_or_default(),
            );
            if !face.is_empty() {
                font.cached_face_id = Some(face.to_string());
            }
            Variant::Font(font)
        }
        // SolidMeshHolder: the net asset references (the geometry arrives
        // on the 0xa5 channel); kept raw for the live session to resolve.
        0x3c => Variant::BinaryString(b.to_vec().into()),
        _ => return None,
    })
}

fn to_variant(v: &PropValue, name: &str) -> Option<Variant> {
    // The wire interns Tags as a SharedString; the DOM/file type is Tags.
    if name == "Tags" {
        let bytes = match v {
            PropValue::String(text) => text.as_bytes(),
            PropValue::BinaryString(bytes) | PropValue::SharedString(bytes)
            | PropValue::CachedSharedString { bytes, .. } => bytes.as_slice(),
            _ => return None,
        };
        return Some(Variant::Tags(rbx_types::Tags::decode(bytes).ok()?));
    }
    if name.ends_with("Content") || matches!(name, "MeshId" | "TextureID" | "HSRAssetId") {
        if let PropValue::String(uri) = v {
            return Some(Variant::Content(if uri.is_empty() {
                rbx_types::Content::default()
            } else {
                rbx_types::Content::from_uri(uri.clone())
            }));
        }
    }
    if name=="Attributes" {
        let bytes=match v {PropValue::String(text)=>text.as_bytes(),PropValue::BinaryString(bytes)=>bytes.as_slice(),_=>return None};
        let attributes=if bytes.is_empty() {rbx_types::Attributes::new()} else {rbx_types::Attributes::from_reader(bytes).ok()?};
        return Some(Variant::Attributes(attributes));
    }
    if let PropValue::Typed { ty, bytes } = v {
        return typed_variant(*ty, bytes);
    }
    let variant = match v {
        PropValue::Bool(b) => Variant::Bool(*b),
        PropValue::Int(i) => i32::try_from(*i).map(Variant::Int32).unwrap_or(Variant::Int64(*i)),
        PropValue::Int64(i) => Variant::Int64(*i),
        PropValue::Float(f) => Variant::Float64(*f),
        PropValue::Float32(f) => Variant::Float32(*f),
        PropValue::String(s) => Variant::String(s.clone()),
        PropValue::BinaryString(bytes)=>Variant::BinaryString(bytes.clone().into()),
        PropValue::SharedString(bytes) | PropValue::CachedSharedString { bytes, .. }
            if name == "Source" =>
        {
            Variant::String(String::from_utf8(bytes.clone()).ok()?)
        }
        PropValue::SharedString(bytes) | PropValue::CachedSharedString { bytes, .. } => {
            Variant::SharedString(rbx_types::SharedString::new(bytes.clone()))
        }
        PropValue::Enum(e) => Variant::Enum(rbx_types::Enum::from_u32(*e)),
        PropValue::Vector2([x, y]) => Variant::Vector2(rbx_types::Vector2::new(*x, *y)),
        PropValue::Vector3([x, y, z]) => Variant::Vector3(rbx_types::Vector3::new(*x, *y, *z)),
        PropValue::Color3([r, g, b]) => Variant::Color3(rbx_types::Color3::new(*r, *g, *b)),
        PropValue::Color3uint8([r, g, b]) => {
            Variant::Color3uint8(rbx_types::Color3uint8::new(*r, *g, *b))
        }
        PropValue::CFrame { pos, rot } => Variant::CFrame(rbx_types::CFrame::new(
            rbx_types::Vector3::new(pos[0], pos[1], pos[2]),
            rbx_types::Matrix3::new(
                rbx_types::Vector3::new(rot[0], rot[1], rot[2]),
                rbx_types::Vector3::new(rot[3], rot[4], rot[5]),
                rbx_types::Vector3::new(rot[6], rot[7], rot[8]),
            ),
        )),
        PropValue::Ref(_) | PropValue::Raw(_) | PropValue::Typed { .. } | PropValue::SharedHash(_) | PropValue::Array(_) | PropValue::Dictionary(_) => return None,
    };
    Some(match (name, variant) {
        ("WorldPivotData", Variant::CFrame(cf)) => Variant::OptionalCFrame(Some(cf)),
        (_, value) => value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_names_use_constructor_defaults_and_explicit_names_win() {
        let mut m = LiveModel::new();
        for (class, expected) in [("BindableEvent", "Event"), ("BindableFunction", "Function"), ("WedgePart", "Wedge"), ("CornerWedgePart", "CornerWedge"), ("UnionOperation", "Union"), ("Part", "Part"), ("LocalScript", "LocalScript")] {
            let r = m.insert_child(m.dom.root_ref(), 1, class, &[]);
            assert_eq!(m.dom.get_by_ref(r).unwrap().name, expected);
            let r = m.insert_child(m.dom.root_ref(), 2, class, &[("Name".into(), PropValue::String("".into()))]);
            assert_eq!(m.dom.get_by_ref(r).unwrap().name, "");
        }
    }

    #[test]
    fn local_create_echo_updates_the_existing_instance() {
        let mut m = LiveModel::new();
        m.apply(RepOp::NewInstance {
            id: 1,
            class: "DataModel".into(),
            parent: None,
            props: vec![],
        });
        let r = m.dom.insert(
            m.dom.root_ref(),
            InstanceBuilder::new("Folder").with_name("Local"),
        );
        let id = (12u64 << 32) | 1;
        m.bind_local(id, r);
        m.apply(RepOp::NewInstance {
            id,
            class: "Folder".into(),
            parent: Some(1),
            props: vec![("Name".into(), PropValue::String("Echo".into()))],
        });
        assert_eq!(m.dom.root().children(), &[r]);
        assert_eq!(m.dom.get_by_ref(r).unwrap().name, "Echo");
        m.bind_local(id + 1, r);
        assert!(m.reference(id).is_none());
        m.apply(RepOp::DeleteInstance { id });
        assert!(m.dom.get_by_ref(r).is_some());
    }

    #[test]
    fn builds_tree_from_ops_including_out_of_order() {
        let mut m = LiveModel::new();
        // Root DataModel (no parent).
        m.apply(RepOp::NewInstance {
            id: 1,
            class: "DataModel".into(),
            parent: None,
            props: vec![],
        });
        // Child arrives BEFORE its parent (Part before Workspace).
        m.apply(RepOp::NewInstance {
            id: 3,
            class: "Part".into(),
            parent: Some(2),
            props: vec![("Name".into(), PropValue::String("Brick".into()))],
        });
        // Now the parent (Workspace under root).
        m.apply(RepOp::NewInstance {
            id: 2,
            class: "Workspace".into(),
            parent: Some(1),
            props: vec![("Name".into(), PropValue::String("Workspace".into()))],
        });
        // The orphan should now be attached.
        assert_eq!(m.instance_count(), 3);

        let root = m.dom().root_ref();
        let ws = m.dom().get_by_ref(root).unwrap().children()[0];
        let ws_inst = m.dom().get_by_ref(ws).unwrap();
        assert_eq!(ws_inst.class, "Workspace");
        assert_eq!(ws_inst.children().len(), 1);
        let part = m.dom().get_by_ref(ws_inst.children()[0]).unwrap();
        assert_eq!(part.class, "Part");
        assert_eq!(part.name, "Brick");
    }

    #[test]
    fn shared_sources_and_forward_object_references_are_preserved() {
        let mut model = LiveModel::new();
        model.apply(RepOp::NewInstance {
            id: 1,
            class: "DataModel".into(),
            parent: None,
            props: vec![],
        });
        model.apply(RepOp::NewInstance {
            id: 2,
            class: "LocalScript".into(),
            parent: Some(1),
            props: vec![
                (
                    "Source".into(),
                    PropValue::SharedString(b"return 42".to_vec()),
                ),
                ("Object".into(), PropValue::Ref(Some(3))),
            ],
        });
        model.apply(RepOp::NewInstance {
            id: 3,
            class: "Part".into(),
            parent: Some(1),
            props: vec![],
        });
        let script = model.refs.get(2).unwrap();
        let part = model.refs.get(3).unwrap();
        let inst = model.dom.get_by_ref(script).unwrap();
        assert_eq!(
            inst.properties.get(&"Source".into()),
            Some(&Variant::String("return 42".into()))
        );
        assert_eq!(
            inst.properties.get(&"Object".into()),
            Some(&Variant::Ref(part))
        );
        model.apply(RepOp::ChangeProperty {
            id: 2,
            name: "Object".into(),
            value: PropValue::Ref(Some(4)),
        });
        model.apply(RepOp::ChangeProperty {
            id: 2,
            name: "Object".into(),
            value: PropValue::Ref(None),
        });
        model.apply(RepOp::NewInstance {
            id: 4,
            class: "Part".into(),
            parent: Some(1),
            props: vec![],
        });
        assert_eq!(
            model
                .dom
                .get_by_ref(script)
                .unwrap()
                .properties
                .get(&"Object".into()),
            Some(&Variant::Ref(Ref::none()))
        );
    }

    #[test]
    fn property_change_and_delete() {
        let mut m = LiveModel::new();
        m.apply(RepOp::NewInstance {
            id: 1,
            class: "DataModel".into(),
            parent: None,
            props: vec![],
        });
        m.apply(RepOp::NewInstance {
            id: 2,
            class: "Part".into(),
            parent: Some(1),
            props: vec![("Name".into(), PropValue::String("P".into()))],
        });
        m.apply(RepOp::ChangeProperty {
            id: 2,
            name: "Anchored".into(),
            value: PropValue::Bool(true),
        });
        let root = m.dom().root_ref();
        let part = m.dom().get_by_ref(root).unwrap().children()[0];
        let anchored = m
            .dom()
            .get_by_ref(part)
            .unwrap()
            .properties
            .iter()
            .find(|(k, _)| k.as_str() == "Anchored")
            .map(|(_, v)| v.clone());
        assert!(matches!(anchored, Some(Variant::Bool(true))));
        m.apply(RepOp::DeleteInstance { id: 2 });
        assert_eq!(m.dom().get_by_ref(root).unwrap().children().len(), 0);
    }
}
