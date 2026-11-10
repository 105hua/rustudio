//! Server → client replication for Play, following Roblox's documented
//! model: everything except ServerStorage and ServerScriptService contents
//! replicates to the client; property changes, creations, reparents and
//! destroys stream over; client changes stay on the client.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{InstanceBuilder, WeakDom};

use crate::doc::Document;
use crate::runtime::{RemoteMsg, RepOp, Runtime};

/// Services whose contents never reach the client.
pub const SERVER_ONLY: &[&str] = &["ServerStorage", "ServerScriptService"];

/// Server ↔ client instance pairs, and the changes on their way over.
#[derive(Default)]
pub struct RepMap {
    pub s2c: HashMap<Ref, Ref>,
    pub c2s: HashMap<Ref, Ref>,
    /// Server changes not yet applied to the client, oldest first.
    queue: VecDeque<Pending>,
    /// Ops enqueued so far (each op's sequence number).
    seq: u64,
    /// Prop / CFrame ops waiting in the queue (a newer one adds nothing:
    /// the value is read from the server when the op is applied).
    queued_props: HashSet<(Ref, String)>,
    /// Server → client remote calls waiting for the replication sent
    /// before them (Roblox keeps remotes and replication in one order).
    held: VecDeque<(u64, RemoteMsg)>,
    /// Client Ref properties whose server target hasn't reached the client
    /// yet: server target -> (client instance, property).
    pending_refs: HashMap<Ref, Vec<(Ref, String)>>,
    /// Targets that arrived since the last flush of `pending_refs`.
    arrived: Vec<Ref>,
    /// Client Animator track slots that changed (crate::anim_replicate).
    anim_dirty: Vec<(Ref, usize)>,
    /// Frame time average (µs) and this frame's replication time.
    frame_ema_us: f64,
    frame_used: Duration,
    frame_items: u32,
    frame_budget: Option<Duration>,
}

/// One unit of replication work.
struct Pending {
    /// The op this came from (an attach carries its op's number).
    seq: u64,
    at: Instant,
    kind: PendingKind,
}

enum PendingKind {
    Op(RepOp),
    /// A big new subtree arriving in pieces: the server children of `s`
    /// from index `next` on are still to come (Studio streams new
    /// instances over several packets, parents first).
    Attach { s: Ref, next: usize },
}
/// Studio's client packet budget (ProcessPacketsJob; its DFInt defaults):
/// ClientPacketHealthyAllocationPercent 15 % of the frame-time average,
/// at least ClientPacketMinMicroseconds 1600, frames counted up to
/// ClientPacketMaxFrameMicroseconds 48000. Once the oldest waiting data is
/// older than ClientPacketMaxDelayMs 100, it catches up: min(packets
/// waiting, MaxProcessPacketsStepsPerCyclic 5) x max(frame / (1/60 s), 1),
/// at most MaxProcessPacketsJobScaling 10, steps of 1/120 s.
const HEALTHY_PERCENT: f64 = 15.0;
const MIN_US: f64 = 1600.0;
const MAX_FRAME_US: f64 = 48_000.0;
const MAX_DELAY: Duration = Duration::from_millis(100);
const STEPS_PER_CYCLIC: f64 = 5.0;
const JOB_SCALING: f64 = 10.0;
const STEP_S: f64 = 1.0 / 120.0;
/// Instances per piece when a big new subtree streams over (about what a
/// few of Studio's packets carry); smaller subtrees arrive whole.
const PIECE: usize = 128;

impl RepMap {
    fn link(&mut self, s: Ref, c: Ref) {
        self.s2c.insert(s, c);
        self.c2s.insert(c, s);
        if self.pending_refs.contains_key(&s) {
            self.arrived.push(s);
        }
    }

    fn unlink_client(&mut self, c: Ref) {
        if let Some(s) = self.c2s.remove(&c) {
            self.s2c.remove(&s);
        }
    }

    /// Starts a frame's replication budget (`dt`: the frame's length).
    pub fn begin_frame(&mut self, dt: f64) {
        let us = (dt * 1e6).clamp(0.0, MAX_FRAME_US);
        self.frame_ema_us = if self.frame_ema_us == 0.0 {
            us
        } else {
            0.9 * self.frame_ema_us + 0.1 * us
        };
        self.frame_used = Duration::ZERO;
        self.frame_items = 0;
        let late = self
            .queue
            .front()
            .is_some_and(|p| p.at.elapsed() > MAX_DELAY);
        let secs = if late {
            let waiting = (self.queue.len() as f64).min(STEPS_PER_CYCLIC);
            (waiting * (dt * 60.0).max(1.0)).min(JOB_SCALING) * STEP_S
        } else {
            (HEALTHY_PERCENT / 100.0 * self.frame_ema_us).max(MIN_US) * 1e-6
        };
        self.frame_budget = Some(Duration::from_secs_f64(secs));
    }

    /// Whether server changes are still waiting to reach the client.
    pub fn backlog(&self) -> usize {
        self.queue.len()
    }

    fn enqueue(&mut self, ops: Vec<RepOp>) {
        let now = Instant::now();
        for op in ops {
            let key = match &op {
                RepOp::Prop(r, n) => Some((*r, n.clone())),
                RepOp::Moved(r) => Some((*r, "CFrame".to_string())),
                _ => None,
            };
            if let Some(k) = key {
                if !self.queued_props.insert(k) {
                    continue;
                }
            }
            self.seq += 1;
            self.queue.push_back(Pending { seq: self.seq, at: now, kind: PendingKind::Op(op) });
        }
    }

    /// The server → client remote calls that may go now: each waits until
    /// the replication sent before it has been applied.
    pub fn gate_remotes(&mut self, msgs: Vec<RemoteMsg>) -> Vec<RemoteMsg> {
        if self.queue.is_empty() && self.held.is_empty() {
            return msgs;
        }
        for m in msgs {
            self.held.push_back((self.seq, m));
        }
        let first_waiting = self.queue.front().map(|p| p.seq);
        let mut out = Vec::new();
        while let Some((seq, _)) = self.held.front() {
            if first_waiting.is_some_and(|w| w <= *seq) {
                break;
            }
            out.push(self.held.pop_front().unwrap().1);
        }
        out
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
/// counterparts. A target that is on its way (replicated, not there yet)
/// is nil until it arrives, as in Studio; one that never replicates stays nil.
fn remap_refs(server: Option<&Document>, dst: &mut WeakDom, created: &[Ref], map: &mut RepMap) {
    for c in created {
        let Some(i) = dst.get_by_ref_mut(*c) else {
            continue;
        };
        for (k, v) in i.properties.iter_mut() {
            if let Variant::Ref(t) = v {
                let target = *t;
                *t = map.s2c.get(&target).copied().unwrap_or_else(Ref::none);
                if t.is_none() && target.is_some() && server.is_some_and(|s| replicated(s, target)) {
                    map.pending_refs.entry(target).or_default().push((*c, k.to_string()));
                }
            }
        }
    }
}

fn remap_value(server: &Document, c: Ref, name: &str, v: Variant, map: &mut RepMap) -> Variant {
    match v {
        Variant::Ref(t) => {
            let ct = map.s2c.get(&t).copied().unwrap_or_else(Ref::none);
            if ct.is_none() && t.is_some() && replicated(server, t) {
                map.pending_refs.entry(t).or_default().push((c, name.to_string()));
            }
            Variant::Ref(ct)
        }
        other => other,
    }
}

/// Sets the Ref properties that were waiting for instances that arrived.
fn flush_refs(client: &Runtime, map: &mut RepMap) {
    while let Some(s) = map.arrived.pop() {
        let Some(waiting) = map.pending_refs.remove(&s) else { continue };
        let Some(ct) = map.s2c.get(&s).copied() else { continue };
        for (c, name) in waiting {
            if map.c2s.contains_key(&c) {
                client.apply_prop(c, &name, Variant::Ref(ct));
            }
        }
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
    remap_refs(None, &mut dom, &created, &mut map);
    let mut doc = Document::from_play_dom(dom, title);
    doc.revision += 1;
    (doc, map)
}

/// Applies server changes to the client, all of them now (engine steps
/// that read the client copy right after, like a character spawn). The
/// client runtime must be entered (its World holds the client document) so
/// client scripts see Changed, ChildAdded and friends. Returns the client
/// instances created.
pub fn apply(server: &Document, client: &Runtime, map: &mut RepMap, ops: Vec<RepOp>) -> Vec<Ref> {
    map.enqueue(ops);
    drain(server, client, map, false)
}

/// Applies server changes within this frame's budget (`RepMap::begin_frame`),
/// as Studio's ProcessPacketsJob does: what doesn't fit waits for the next
/// frame, in order. Fixed-step runs (no wall clock) apply everything.
pub fn apply_budgeted(server: &Document, client: &Runtime, map: &mut RepMap, ops: Vec<RepOp>) -> Vec<Ref> {
    map.enqueue(ops);
    let realtime = client.world_mut().is_some_and(|w| w.realtime);
    drain(server, client, map, realtime && map.frame_budget.is_some())
}

fn drain(server: &Document, client: &Runtime, map: &mut RepMap, budgeted: bool) -> Vec<Ref> {
    let mut created = Vec::new();
    if map.queue.is_empty() {
        return created;
    }
    let start = Instant::now();
    let limit = map.frame_budget.unwrap_or(Duration::MAX);
    let used = map.frame_used;
    // At least one piece a frame, as Studio's job always takes a packet.
    let over = |map: &RepMap| budgeted && map.frame_items > 0 && used + start.elapsed() >= limit;
    let mut seen_props: HashSet<(Ref, String)> = Default::default();
    while let Some(p) = map.queue.front() {
        if over(map) {
            break;
        }
        let p = map.queue.pop_front().unwrap();
        map.frame_items += 1;
        let _sc = crate::bench::scope(match &p.kind {
            PendingKind::Op(RepOp::Parent(_)) => "rep_parent",
            PendingKind::Op(RepOp::Destroy(_)) => "rep_destroy",
            PendingKind::Op(RepOp::Moved(_)) => "rep_moved",
            PendingKind::Op(_) => "rep_prop",
            PendingKind::Attach { .. } => "rep_attach",
        });
        match p.kind {
            PendingKind::Op(op) => {
                if let RepOp::Prop(r, n) = &op {
                    map.queued_props.remove(&(*r, n.clone()));
                } else if let RepOp::Moved(r) = &op {
                    map.queued_props.remove(&(*r, "CFrame".to_string()));
                }
                apply_op(server, client, map, op, p.seq, budgeted, &mut seen_props, &mut created);
            }
            PendingKind::Attach { s, next } => {
                attach(server, client, map, s, next, p.seq, &over, &mut created);
            }
        }
        flush_refs(client, map);
    }
    map.frame_used += start.elapsed();
    crate::anim_replicate::sync(&client.lua, std::mem::take(&mut map.anim_dirty));
    created
}

/// Copies a server subtree to the client and parents it there, whole when
/// it is small or `whole`, else its root now and the rest in pieces.
#[allow(clippy::too_many_arguments)]
fn arrive(
    server: &Document,
    client: &Runtime,
    map: &mut RepMap,
    r: Ref,
    cp: Ref,
    whole: bool,
    seq: u64,
    created: &mut Vec<Ref>,
) -> bool {
    let big = !whole && server.dom.descendants_of(r).nth(PIECE).is_some();
    let mut new_refs = Vec::new();
    let copy = {
        let Some(mut w) = client.world_mut() else {
            return false;
        };
        let copy = if big {
            builder_of(&server.dom, r).map(|b| {
                let c = w.dm().dom.insert(Ref::none(), b);
                map.link(r, c);
                new_refs.push(c);
                c
            })
        } else {
            copy_subtree(&server.dom, r, &mut w.dm().dom, Ref::none(), map, &mut new_refs)
        };
        remap_refs(Some(server), &mut w.dm().dom, &new_refs, map);
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
    if big {
        map.queue.push_front(Pending { seq, at: Instant::now(), kind: PendingKind::Attach { s: r, next: 0 } });
    }
    true
}

/// Brings over the server children of `s` from `next` on (each a whole
/// small subtree, or a big one's root followed by its own pieces, so the
/// client gets parents before children and siblings in order).
#[allow(clippy::too_many_arguments)]
fn attach(
    server: &Document,
    client: &Runtime,
    map: &mut RepMap,
    s: Ref,
    next: usize,
    seq: u64,
    over: &dyn Fn(&RepMap) -> bool,
    created: &mut Vec<Ref>,
) {
    let Some(cs) = map.s2c.get(&s).copied() else { return };
    let kids: Vec<Ref> = match server.dom.get_by_ref(s) {
        Some(i) if replicated(server, s) => i.children().to_vec(),
        _ => return,
    };
    let mut k = next;
    let mut did = false;
    while k < kids.len() {
        let sc = kids[k];
        k += 1;
        if map.s2c.contains_key(&sc) {
            continue;
        }
        if did && over(map) {
            k -= 1;
            break;
        }
        did = true;
        let len = map.queue.len();
        arrive(server, client, map, sc, cs, false, seq, created);
        if map.queue.len() > len {
            // `sc` streams in pieces first; the rest of `s` comes after it.
            let piece = map.queue.pop_front();
            if k < kids.len() {
                map.queue.push_front(Pending { seq, at: Instant::now(), kind: PendingKind::Attach { s, next: k } });
            }
            if let Some(piece) = piece {
                map.queue.push_front(piece);
            }
            return;
        }
    }
    if k < kids.len() {
        map.queue.push_front(Pending { seq, at: Instant::now(), kind: PendingKind::Attach { s, next: k } });
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_op(
    server: &Document,
    client: &Runtime,
    map: &mut RepMap,
    op: RepOp,
    seq: u64,
    budgeted: bool,
    seen_props: &mut HashSet<(Ref, String)>,
    created: &mut Vec<Ref>,
) {
    match op {
        RepOp::Prop(r, name) => {
            if !seen_props.insert((r, name.clone())) {
                return;
            }
            let Some(c) = map.s2c.get(&r).copied() else {
                return;
            };
            let Some(v) = server
                .dom
                .get_by_ref(r)
                .and_then(|i| i.properties.get(&rbx_dom_weak::ustr(&name)).cloned())
            else {
                return;
            };
            crate::runtime::domains::services::perf::net(true, false, crate::runtime::domains::services::perf::prop_bytes(&v));
            // An Animator's track slot: stored without Changed (hidden,
            // NotScriptable), then the client's own tracks follow it.
            if let Some(slot) = crate::anim_replicate::is_slot_prop(&name) {
                if server.class_of(r).is_some_and(|k| k == "Animator") {
                    if let Some(mut w) = client.world_mut() {
                        if let Some(i) = w.dm().dom.get_by_ref_mut(c) {
                            i.properties.insert(rbx_dom_weak::ustr(&name), v);
                        }
                    }
                    map.anim_dirty.push((c, slot));
                    return;
                }
            }
            let v = remap_value(server, c, &name, v, map);
            client.apply_prop(c, &name, v);
        }
        RepOp::Moved(r) => {
            if !seen_props.insert((r, "CFrame".into())) {
                return;
            }
            let (Some(c), Some(v)) = (
                map.s2c.get(&r).copied(),
                server
                    .dom
                    .get_by_ref(r)
                    .and_then(|i| i.properties.get(&rbx_dom_weak::ustr("CFrame")).cloned()),
            ) else {
                return;
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
        RepOp::Destroy(r) => {
            // Studio's DestroyInstance item: the client runs Destroy on its
            // copy (one op, however big the subtree).
            let Some(c) = map.s2c.get(&r).copied() else {
                return;
            };
            crate::runtime::domains::services::perf::net(true, false, 10);
            let subtree: Vec<Ref> = client
                .world_mut()
                .map(|mut w| w.dm().subtree(c))
                .unwrap_or_default();
            client.apply_destroy(c);
            for d in subtree {
                map.unlink_client(d);
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
                    // New to the client: copy it (unparented), then parent it
                    // so events fire. A big one streams over in pieces.
                    arrive(server, client, map, r, cp, !budgeted, seq, created);
                }
                (_, Some(c), _) => {
                    // Left the replicated tree (unparented, moved server-side only).
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::runtime::{RunMode, Side, World};

    /// A server and a client runtime over a fresh place, as Play pairs them.
    fn pair() -> (Runtime, Runtime, RepMap) {
        let sd = Document::new_place();
        let (cd, map) = initial_client(&sd, "c".into());
        let s = Runtime::new().unwrap();
        let mut sw = World::new(sd, RunMode::Run);
        sw.has_peer = true;
        s.enter(sw);
        s.bind_globals().unwrap();
        let c = Runtime::new().unwrap();
        let mut cw = World::new(cd, RunMode::Run);
        cw.side = Side::Client;
        cw.has_peer = true;
        cw.realtime = true;
        c.enter(cw);
        c.bind_globals().unwrap();
        (s, c, map)
    }

    fn ops(s: &Runtime) -> Vec<RepOp> {
        std::mem::take(&mut s.world_mut().unwrap().rep)
    }

    fn sync(s: &Runtime, c: &Runtime, map: &mut RepMap, budgeted: bool) {
        let o = ops(s);
        let w = s.world_mut().unwrap();
        if budgeted {
            apply_budgeted(w.d(), c, map, o);
        } else {
            apply(w.d(), c, map, o);
        }
        drop(w);
        c.dispatch();
    }

    fn names(d: &Document, r: Ref) -> Vec<String> {
        d.dom.descendants_of(r).skip(1).map(|i| i.name.clone()).collect()
    }

    /// A server Destroy reaches the client as one op (Studio's
    /// DestroyInstance item), and the client destroys its copy: Destroying
    /// fires there, every descendant is cut loose and locked.
    #[test]
    fn destroy_replicates_as_one_op() {
        let (s, c, mut map) = pair();
        s.lua.load("local m = Instance.new('Model') m.Name = 'Map'\n\
            for i = 1, 300 do local p = Instance.new('Part') p.Name = 'P' .. i\n\
            local a = Instance.new('Attachment') a.Parent = p p.Parent = m end\n\
            m.Parent = workspace").exec().unwrap();
        sync(&s, &c, &mut map, false);
        c.lua.load("_G.log = {} local m = workspace:FindFirstChild('Map')\n\
            m.Destroying:Connect(function() table.insert(_G.log, 'destroying') end)\n\
            _G.m = m _G.p = m:FindFirstChild('P7')").exec().unwrap();
        s.lua.load("workspace.Map:Destroy()").exec().unwrap();
        let o = ops(&s);
        assert_eq!(o.iter().filter(|x| matches!(x, RepOp::Destroy(_))).count(), 1, "{o:?}");
        assert!(o.len() <= 2, "one Destroy and the root's Parent, not one per descendant: {}", o.len());
        {
            let w = s.world_mut().unwrap();
            apply(w.d(), &c, &mut map, o);
        }
        c.dispatch();
        c.step(1.0 / 60.0);
        let out: Vec<String> = c.lua.load("local p = _G.p\n\
            return table.concat(_G.log, ',') .. '|' .. tostring(_G.m.Parent) .. '|' .. tostring(p.Parent) ..\n\
            '|' .. #_G.m:GetChildren() .. '|' .. #p:GetChildren() .. '|' ..\n\
            tostring(pcall(function() p.Parent = workspace end))").eval::<String>().map(|x| vec![x]).unwrap();
        assert_eq!(out[0], "destroying|nil|nil|0|0|false");
        let w = c.world_mut().unwrap();
        assert!(w.d().find_named(w.d().workspace().unwrap(), "Map").is_none());
        assert_eq!(map.c2s.len(), map.s2c.len());
    }

    /// A big new subtree streams over a budgeted client in pieces, parents
    /// before children and siblings in order, ending identical to the
    /// server's; a Ref to a part that comes later is set when it arrives;
    /// a remote fired after it waits until it is all there.
    #[test]
    fn big_subtree_arrives_in_order_over_frames() {
        let (s, c, mut map) = pair();
        c.lua.load("_G.seen = {} workspace.DescendantAdded:Connect(function(d) table.insert(_G.seen, d.Name) end)").exec().unwrap();
        s.lua.load("local m = Instance.new('Model') m.Name = 'Map'\n\
            for i = 1, 60 do local f = Instance.new('Folder') f.Name = 'F' .. i\n\
              for j = 1, 40 do local p = Instance.new('Part') p.Name = 'P' .. i .. '_' .. j p.Parent = f end\n\
              f.Parent = m end\n\
            m.PrimaryPart = m.F60.P60_40\n\
            m.Parent = workspace").exec().unwrap();
        let o = ops(&s);
        let mut frames = 0;
        {
            let w = s.world_mut().unwrap();
            map.begin_frame(1.0 / 60.0);
            // (A small budget, so the test doesn't depend on machine speed.)
            map.frame_budget = Some(Duration::from_micros(300));
            apply_budgeted(w.d(), &c, &mut map, o);
        }
        c.dispatch();
        assert!(map.backlog() > 0, "2461 instances don't fit a 0.3 ms budget");
        let msg = RemoteMsg { inst: Ref::none(), kind: crate::runtime::RemoteKind::Event, args: vec![] };
        assert!(map.gate_remotes(vec![msg]).is_empty(), "a remote waits for the replication before it");
        while map.backlog() > 0 && frames < 10_000 {
            frames += 1;
            let w = s.world_mut().unwrap();
            map.begin_frame(1.0 / 60.0);
            map.frame_budget = Some(Duration::from_micros(300));
            apply_budgeted(w.d(), &c, &mut map, vec![]);
            drop(w);
            c.dispatch();
        }
        assert!(frames > 1, "{frames}");
        assert_eq!(map.gate_remotes(vec![]).len(), 1, "the remote goes once all of it arrived");
        let (sn, sm) = {
            let w = s.world_mut().unwrap();
            let m = w.d().find_named(w.d().workspace().unwrap(), "Map").unwrap();
            (names(w.d(), m), m)
        };
        let cm = map.s2c[&sm];
        let w = c.world_mut().unwrap();
        let mut cn = names(w.d(), cm);
        let mut sn2 = sn.clone();
        assert_eq!(w.d().dom.get_by_ref(cm).unwrap().children().len(), 60);
        // Same tree (breadth-first names), same child order everywhere.
        assert_eq!(cn.len(), sn2.len());
        assert_eq!(cn, sn2);
        cn.sort();
        sn2.sort();
        assert_eq!(cn, sn2);
        let pp = match w.d().prop(cm, "PrimaryPart") {
            Some(Variant::Ref(r)) => r,
            other => panic!("PrimaryPart {other:?}"),
        };
        assert_eq!(w.d().dom.get_by_ref(pp).map(|i| i.name.as_str()), Some("P60_40"));
        drop(w);
        // DescendantAdded saw each parent before its children.
        let order: String = c.lua.load("local pos = {} for i, n in _G.seen do pos[n] = i end\n\
            for i = 1, 60 do local f = pos['F' .. i] if not f then return 'missing F' .. i end\n\
              for j = 1, 40 do local p = pos['P' .. i .. '_' .. j] if not p or p < f then return 'bad P' .. i .. '_' .. j end end end\n\
            return 'ok ' .. #_G.seen").eval().unwrap();
        assert_eq!(order, "ok 2461");
        let _ = sn;
    }

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
