//! Server → client replication for Play, following Roblox's documented
//! model: everything except ServerStorage and ServerScriptService contents
//! replicates to the client; property changes, creations, reparents and
//! destroys stream over; client changes stay on the client.

use std::collections::HashMap;

use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{InstanceBuilder, WeakDom};

use crate::doc::Document;
use crate::runtime::{RepOp, Runtime};

/// Services whose contents never reach the client.
pub const SERVER_ONLY: &[&str] = &["ServerStorage", "ServerScriptService"];

/// Server ↔ client instance pairs.
#[derive(Default)]
pub struct RepMap {
    pub s2c: HashMap<Ref, Ref>,
    pub c2s: HashMap<Ref, Ref>,
}

impl RepMap {
    fn link(&mut self, s: Ref, c: Ref) {
        self.s2c.insert(s, c);
        self.c2s.insert(c, s);
    }

    fn unlink_client(&mut self, c: Ref) {
        if let Some(s) = self.c2s.remove(&c) {
            self.s2c.remove(&s);
        }
    }
}

/// Whether a server instance is in a replicated part of the tree.
pub fn replicated(server: &Document, r: Ref) -> bool {
    let root = server.dom.root_ref();
    if r == root {
        return true;
    }
    let mut cur = r;
    loop {
        let Some(i) = server.dom.get_by_ref(cur) else {
            return false;
        };
        let parent = i.parent();
        if parent.is_none() {
            return false;
        }
        if parent == root {
            return !SERVER_ONLY.contains(&i.class.as_str());
        }
        cur = parent;
    }
}

fn builder_of(dom: &WeakDom, r: Ref) -> Option<InstanceBuilder> {
    let i = dom.get_by_ref(r)?;
    let mut b = InstanceBuilder::new(i.class).with_name(i.name.clone());
    for (k, v) in &i.properties {
        b.add_property(*k, v.clone());
    }
    Some(b)
}

/// Copies a server subtree into the client DOM (parent `parent`, may be
/// none), recording the pairs. Returns the client root of the copy.
fn copy_subtree(
    src: &WeakDom,
    r: Ref,
    dst: &mut WeakDom,
    parent: Ref,
    map: &mut RepMap,
    created: &mut Vec<Ref>,
) -> Option<Ref> {
    let b = builder_of(src, r)?;
    let c = dst.insert(parent, b);
    map.link(r, c);
    created.push(c);
    // A stack, children pushed last-first: each parent gets its children in
    // the server's order (Roblox's GetChildren is the file's order; Place B
    // Newer's Fast Travel list, LayoutOrder ties, read upside down when the
    // client got them reversed).
    let mut queue: Vec<(Ref, Ref)> = src
        .get_by_ref(r)?
        .children()
        .iter()
        .rev()
        .map(|k| (*k, c))
        .collect();
    while let Some((s, cp)) = queue.pop() {
        let Some(b) = builder_of(src, s) else {
            continue;
        };
        let cc = dst.insert(cp, b);
        map.link(s, cc);
        created.push(cc);
        if let Some(i) = src.get_by_ref(s) {
            queue.extend(i.children().iter().rev().map(|k| (*k, cc)));
        }
    }
    Some(c)
}

/// Points Ref properties of freshly copied client instances at their client
/// counterparts (unknown targets become nil).
fn remap_refs(dst: &mut WeakDom, created: &[Ref], map: &RepMap) {
    for c in created {
        let Some(i) = dst.get_by_ref_mut(*c) else {
            continue;
        };
        for v in i.properties.values_mut() {
            if let Variant::Ref(t) = v {
                *t = map.s2c.get(t).copied().unwrap_or_else(Ref::none);
            }
        }
    }
}

fn remap_value(v: Variant, map: &RepMap) -> Variant {
    match v {
        Variant::Ref(t) => Variant::Ref(map.s2c.get(&t).copied().unwrap_or_else(Ref::none)),
        other => other,
    }
}

/// The client's starting DataModel: a copy of the server's, with the
/// server-only services present but empty.
pub fn initial_client(server: &Document, title: String) -> (Document, RepMap) {
    let mut map = RepMap::default();
    let sroot = server.dom.root();
    let mut root_b = InstanceBuilder::new(sroot.class).with_name(sroot.name.clone());
    for (k, v) in &sroot.properties {
        root_b.add_property(*k, v.clone());
    }
    let mut dom = WeakDom::new(root_b);
    let croot = dom.root_ref();
    map.link(sroot.referent(), croot);
    let mut created = Vec::new();
    for child in sroot.children().to_vec() {
        let class = server
            .dom
            .get_by_ref(child)
            .map(|i| i.class.to_string())
            .unwrap_or_default();
        if SERVER_ONLY.contains(&class.as_str()) {
            if let Some(b) = builder_of(&server.dom, child) {
                let c = dom.insert(croot, b);
                map.link(child, c);
            }
        } else {
            copy_subtree(&server.dom, child, &mut dom, croot, &mut map, &mut created);
        }
    }
    remap_refs(&mut dom, &created, &map);
    let mut doc = Document::from_play_dom(dom, title);
    doc.revision += 1;
    (doc, map)
}

/// Applies server changes to the client. The client runtime must be entered
/// (its World holds the client document) so client scripts see Changed,
/// ChildAdded and friends. Returns the client instances created.
pub fn apply(server: &Document, client: &Runtime, map: &mut RepMap, ops: Vec<RepOp>) -> Vec<Ref> {
    let mut created = Vec::new();
    let mut seen_props: std::collections::HashSet<(Ref, String)> = Default::default();
    for op in ops {
        match op {
            RepOp::Prop(r, name) => {
                if !seen_props.insert((r, name.clone())) {
                    continue;
                }
                let Some(c) = map.s2c.get(&r).copied() else {
                    continue;
                };
                let Some(v) = server
                    .dom
                    .get_by_ref(r)
                    .and_then(|i| i.properties.get(&rbx_dom_weak::ustr(&name)).cloned())
                else {
                    continue;
                };
                crate::runtime::domains::services::perf::net(true, false, crate::runtime::domains::services::perf::prop_bytes(&v));
                client.apply_prop(c, &name, remap_value(v, map));
            }
            RepOp::Moved(r) => {
                if !seen_props.insert((r, "CFrame".into())) {
                    continue;
                }
                let (Some(c), Some(v)) = (
                    map.s2c.get(&r).copied(),
                    server
                        .dom
                        .get_by_ref(r)
                        .and_then(|i| i.properties.get(&rbx_dom_weak::ustr("CFrame")).cloned()),
                ) else {
                    continue;
                };
                // A physics update: position, rotation, velocities.
                crate::runtime::domains::services::perf::net(true, true, 4 + 12 + 6 + 12 + 12);
                if let Some(mut w) = client.world_mut() {
                    let d = w.dm();
                    if let Some(i) = d.dom.get_by_ref_mut(c) {
                        i.properties.insert(rbx_dom_weak::ustr("CFrame"), v);
                    }
                    d.mark_prop(c, rbx_dom_weak::ustr("CFrame"));
                }
            }
            RepOp::Name(r) => {
                if let (Some(c), Some(i)) = (map.s2c.get(&r).copied(), server.dom.get_by_ref(r)) {
                    client.apply_name(c, &i.name);
                    crate::runtime::domains::services::perf::net(true, false, 6 + i.name.len() as u64 + 2);
                }
            }
            RepOp::Parent(r) => {
                let sp = server
                    .dom
                    .get_by_ref(r)
                    .map(|i| i.parent())
                    .unwrap_or_else(Ref::none);
                let visible = server.dom.get_by_ref(r).is_some() && replicated(server, r);
                let cp = map.s2c.get(&sp).copied();
                match (visible, map.s2c.get(&r).copied(), cp) {
                    (true, Some(c), Some(cp)) => {
                        crate::runtime::domains::services::perf::net(true, false, 10);
                        client.apply_parent(c, cp)
                    }
                    (true, None, Some(cp)) => {
                        // New to the client: copy it (unparented), then parent it so events fire.
                        let mut new_refs = Vec::new();
                        let copy = {
                            let Some(mut w) = client.world_mut() else {
                                continue;
                            };
                            let copy = copy_subtree(
                                &server.dom,
                                r,
                                &mut w.dm().dom,
                                Ref::none(),
                                map,
                                &mut new_refs,
                            );
                            remap_refs(&mut w.dm().dom, &new_refs, map);
                            w.structure_changed = true;
                            // A new instance: class, name, parent and every property.
                            let bytes: u64 = new_refs
                                .iter()
                                .filter_map(|c| w.d().dom.get_by_ref(*c))
                                .map(|i| 10 + i.name.len() as u64 + i.properties.values().map(|v| 2 + crate::runtime::domains::services::perf::value_bytes(v)).sum::<u64>())
                                .sum();
                            crate::runtime::domains::services::perf::net(true, false, bytes);
                            copy
                        };
                        if let Some(c) = copy {
                            client.apply_parent(c, cp);
                        }
                        created.extend(new_refs);
                    }
                    (_, Some(c), _) => {
                        // Left the replicated tree (destroyed, unparented, moved server-side only).
                        client.apply_parent(c, Ref::none());
                        // Forget the pairs for the client copy's whole subtree.
                        let subtree: Vec<Ref> = client
                            .world_mut()
                            .map(|mut w| w.dm().subtree(c))
                            .unwrap_or_default();
                        for d in subtree {
                            map.unlink_client(d);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    created
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_only_services_stay_empty() {
        let mut s = Document::new_place();
        let root = s.dom.root_ref();
        let ss = s.find_child(root, "ServerStorage").unwrap();
        s.dom
            .insert(ss, InstanceBuilder::new("Folder").with_name("Secret"));
        let (c, map) = initial_client(&s, "c".into());
        let css = c.find_child(c.dom.root_ref(), "ServerStorage").unwrap();
        assert!(c.dom.get_by_ref(css).unwrap().children().is_empty());
        assert_eq!(c.count_class("BasePart"), s.count_class("BasePart"));
        assert!(map.s2c.contains_key(&s.workspace().unwrap()));
    }

    /// The client's children come in the server's order, at every depth
    /// (round 26: Place B Newer's Fast Travel list was upside down).
    #[test]
    fn client_keeps_child_order() {
        let mut s = Document::new_place();
        let ws = s.workspace().unwrap();
        let names = ["Desert", "Forest", "Snow", "Spawn", "Mushroom"];
        let mut f = InstanceBuilder::new("Folder").with_name("List");
        for n in names {
            f = f.with_child(
                InstanceBuilder::new("Folder")
                    .with_name(n)
                    .with_child(InstanceBuilder::new("Folder").with_name(format!("{n}1")))
                    .with_child(InstanceBuilder::new("Folder").with_name(format!("{n}2"))),
            );
        }
        let list = s.dom.insert(ws, f);
        let (c, map) = initial_client(&s, "c".into());
        let kids = |r: Ref| -> Vec<String> {
            c.dom.get_by_ref(r).unwrap().children().iter()
                .map(|k| c.dom.get_by_ref(*k).unwrap().name.clone())
                .collect()
        };
        let clist = map.s2c[&list];
        assert_eq!(kids(clist), names);
        let first = c.dom.get_by_ref(clist).unwrap().children()[0];
        assert_eq!(kids(first), ["Desert1", "Desert2"]);
    }
}
