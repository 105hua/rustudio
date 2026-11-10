//! The open document: a Roblox DOM plus undo/redo, selection and the edit
//! operations every panel and tool goes through.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rbx_dom_weak::types::{CFrame, Color3uint8, Enum, Ref, Variant, Vector3};
use rbx_dom_weak::{ustr, InstanceBuilder, Ustr, WeakDom};
use rbx_reflection::ClassTag;

use crate::mech;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// `.rbxl` / `.rbxlx`: the root is the DataModel.
    Place,
    /// `.rbxm` / `.rbxmx`: the root's children are the model's top-level items.
    Model,
}

#[derive(Debug, Clone)]
pub enum Change {
    Prop {
        r: Ref,
        name: Ustr,
        old: Option<Variant>,
        new: Option<Variant>,
    },
    Name {
        r: Ref,
        old: String,
        new: String,
    },
    Parent {
        r: Ref,
        old: Ref,
        new: Ref,
    },
    /// Forward: the instance lives in the DOM under `parent`.
    Create {
        r: Ref,
        parent: Ref,
    },
    /// Forward: the instance lives in limbo.
    Remove {
        r: Ref,
        parent: Ref,
    },
}

/// One entry in a document's change journal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    /// A property (or the Name) of an instance changed.
    Prop(Ref, Ustr),
    /// An instance was created, reparented, removed or destroyed: its whole
    /// subtree must be re-checked.
    Tree(Ref),
}

/// Who reads a journal (each keeps its own position).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    Scene = 0,
    Physics = 1,
    /// Script world queries (raycasts, region queries).
    Query = 2,
    /// A project folder's file write-back (src/project.rs).
    Project = 3,
    /// The client's own joints (src/client_joints.rs).
    ClientJoints = 4,
    /// The SurfaceGuis / BillboardGuis under Workspace (src/gui_input.rs).
    Gui = 5,
    /// FindFirstChild's name index (src/runtime.rs).
    Children = 6,
    /// The lights under Workspace (src/lighting.rs).
    Lights = 7,
    /// The client DataModel's anchored parts its LocalScripts move
    /// (src/physics.rs follow_client).
    ClientPhysics = 8,
    /// ParticleEmitters, Beams and Trails under Workspace (src/effects.rs).
    Effects = 9,
    /// Outgoing Team Create edits.
    TeamCreate = 10,
    /// CollectionService's tag index (src/runtime_ext.rs GetTagged).
    Tags = 11,
    /// Sounds, audio / video instances and Wires (src/domains/media.rs).
    Media = 12,
    /// Motors and IKControls (src/domains/anim.rs).
    Anim = 13,
    /// GUI boxes scripts listen to (runtime_ext::fire_layout_changes).
    Layout = 14,
}

/// What a reader gets back: everything changed (rebuild), or the new marks.
pub enum Marks {
    All,
    Some(Vec<Mark>),
}

/// Change journal: lets the scene and physics update only what changed.
struct Journal {
    /// Sequence number of `marks[0]`.
    base: u64,
    marks: Vec<Mark>,
    /// Next sequence number each reader wants; None = hasn't read yet.
    read: [Option<u64>; 15],
    /// The joint graph's position (src/domains/world_part.rs), read through
    /// `&self`: 0 = not reading, else the next sequence number + 1.
    graph_pos: std::sync::atomic::AtomicU64,
}

static NEXT_DOC_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct Command {
    pub label: String,
    pub changes: Vec<Change>,
    merge_key: Option<String>,
    at: Instant,
    /// Selection when the command was made; undo restores it.
    sel_before: Vec<Ref>,
    /// Selection right before it was undone; redo restores it.
    sel_after: Vec<Ref>,
}

pub struct Document {
    pub dom: WeakDom,
    /// Removed instances wait here so undo can bring them back with the same refs.
    limbo: WeakDom,
    pub kind: DocKind,
    pub path: Option<PathBuf>,
    pub title: String,
    /// The place on Roblox this document is (opened from Roblox, round 27):
    /// Ctrl+S saves a new version there, Play reports its ids.
    pub cloud: Option<rustudio_account::CloudOrigin>,
    undo: Vec<Command>,
    redo: Vec<Command>,
    /// Bumped on every change; the renderer and panels cache against it.
    pub revision: u64,
    saved_revision: u64,
    pub selection: Vec<Ref>,
    /// Unique per document (a play copy gets a new one).
    pub id: u64,
    /// Bumped when instances are added, moved, removed or renamed.
    pub tree_revision: u64,
    journal: Journal,
}

/// Studio's default for a property missing from a place file, where it
/// isn't the reflection database's: Lighting.Brightness reads 1 in Studio and
/// 1.981 from the database (round 21, terrain corpus place: its sun was twice
/// Studio's).
fn studio_default(class: &str, name: &str) -> Option<Variant> {
    match (class, name) {
        ("Lighting", "Brightness") => Some(Variant::Float32(crate::mech::LIGHTING_BRIGHTNESS_DEFAULT)),
        // On in Studio; the reflection database's default is false, which
        // left a new place (and any file that leaves it out) without shadows.
        ("Lighting", "GlobalShadows") => Some(Variant::Bool(true)),
        // Studio's UICorner default; the reflection database has none, and
        // files saved at the default leave it out (Place B's index UI).
        ("UICorner", "CornerRadius") => Some(Variant::UDim(rbx_dom_weak::types::UDim::new(0.0, 8))),
        _ => None,
    }
}

/// The instance-independent part of `Document::prop`'s fallback chain.
struct PropPlan {
    stored_as: Option<Ustr>,
    twin: Option<Ustr>,
    /// Studio's or the reflection database's default (or the empty id of a
    /// content property): returned before the XML twin is looked at.
    fixed: Option<Variant>,
    xml: Ustr,
    xml_default: Option<Variant>,
    upper: Option<String>,
}

/// None when the class isn't in the reflection database (prop gives None).
fn prop_plan(class: Ustr, key: Ustr) -> Option<std::rc::Rc<PropPlan>> {
    thread_local! {
        static PLANS: std::cell::RefCell<rustc_hash::FxHashMap<(Ustr, Ustr), Option<std::rc::Rc<PropPlan>>>> =
            Default::default();
    }
    if let Some(p) = PLANS.with(|m| m.borrow().get(&(class, key)).cloned()) {
        return p;
    }
    let name = key.as_str();
    let plan = (|| {
        let db = db();
        let c = db.classes.get(class.as_str())?;
        let stored_as = db.superclasses_iter(c).find_map(|c| match &c.properties.get(name)?.kind {
            rbx_reflection::PropertyKind::Canonical {
                serialization: rbx_reflection::PropertySerialization::SerializesAs(alt),
            } => Some(alt.to_string()),
            _ => None,
        });
        let twin = content_twin(name);
        let fixed = studio_default(class.as_str(), name)
            .or_else(|| {
                db.find_default_property(c, name)
                    .or_else(|| stored_as.as_deref().and_then(|alt| db.find_default_property(c, alt)))
                    .cloned()
            })
            .or_else(|| {
                // A content property the class has but nothing set: the
                // empty id (MeshPart.TextureID reads "" in Roblox, not nil).
                (twin.is_some() && db.superclasses_iter(c).any(|c| c.properties.contains_key(name)))
                    .then(|| Variant::ContentId(rbx_dom_weak::types::ContentId::from(String::new())))
            });
        let xml = format!("{name}_XML");
        let xml_default = db.find_default_property(c, &xml).cloned();
        let mut chars = name.chars();
        let upper = match chars.next() {
            Some(ch) if ch.is_ascii_lowercase() => Some(ch.to_ascii_uppercase().to_string() + chars.as_str()),
            _ => None,
        };
        Some(std::rc::Rc::new(PropPlan {
            stored_as: stored_as.map(|s| ustr(&s)),
            twin: twin.map(ustr),
            fixed,
            xml: ustr(&xml),
            xml_default,
            upper,
        }))
    })();
    PLANS.with(|m| m.borrow_mut().insert((class, key), plan.clone()));
    plan
}

pub fn db() -> &'static rbx_reflection::ReflectionDatabase<'static> {
    rbx_reflection_database::get().unwrap_or_else(|_| rbx_reflection_database::get_bundled())
}

/// `Instance:IsA` against the reflection database.
pub fn is_a(class: &str, base: &str) -> bool {
    if class == base {
        return true;
    }
    // Memoized: engine code asks this per part per frame in places, and
    // every script property read or method call asks it many times (each
    // engine domain checks the class). Keyed by the strings themselves,
    // base first: interning both names for the key (two locked, hashed
    // lookups in the global string table) was the single largest cost of
    // a property read from Luau (Ustr::from: 10-14 % of a Play frame).
    thread_local! {
        static CACHE: std::cell::RefCell<rustc_hash::FxHashMap<Box<str>, rustc_hash::FxHashMap<Box<str>, bool>>> =
            Default::default();
    }
    if let Some(v) = CACHE.with(|c| c.borrow().get(base).and_then(|m| m.get(class).copied())) {
        return v;
    }
    let db = db();
    let v = match db.classes.get(class) {
        Some(c) => db.superclasses_iter(c).any(|d| d.name == base),
        // Classes newer than the reflection database (UIBlur,
        // ParticleEmitter2D…): the API dump's hierarchy.
        None => crate::api::get().is_some_and(|a| a.is_a(class, base)),
    };
    CACHE.with(|c| c.borrow_mut().entry(base.into()).or_default().insert(class.into(), v));
    v
}

pub fn is_service(class: &str) -> bool {
    db().classes
        .get(class)
        .is_some_and(|c| c.tags.contains(&ClassTag::Service))
}

pub fn is_creatable(class: &str) -> bool {
    is_script_creatable(class) && db().classes.get(class).is_some_and(|c| !c.tags.contains(&ClassTag::Deprecated))
}

/// What `Instance.new` accepts: Studio creates deprecated classes too
/// (FunctionalTest, Hint, Message); only the Insert Object list hides them.
pub fn is_script_creatable(class: &str) -> bool {
    match db().classes.get(class) {
        Some(c) => !c.tags.contains(&ClassTag::NotCreatable) && !c.tags.contains(&ClassTag::Service),
        // Newer than the reflection database: as the API dump says.
        None => crate::api::get()
            .and_then(|a| a.classes.get(class))
            .is_some_and(|c| c.creatable && !c.service),
    }
}

fn limbo_dom() -> WeakDom {
    WeakDom::new(InstanceBuilder::new("Folder").with_name("Limbo"))
}

pub fn new_part(
    name: &str,
    shape: u32,
    pos: glam::Vec3,
    size: glam::Vec3,
    color: [u8; 3],
) -> InstanceBuilder {
    let mut b = InstanceBuilder::new("Part")
        .with_name(name)
        .with_property("Anchored", true)
        .with_property("Size", Vector3::new(size.x, size.y, size.z))
        .with_property(
            "CFrame",
            CFrame::new(
                Vector3::new(pos.x, pos.y, pos.z),
                CFrame::identity().orientation,
            ),
        )
        .with_property("Color", Color3uint8::new(color[0], color[1], color[2]));
    // Block is the default shape; wedge classes have no Shape property at all.
    if shape != 1 {
        b.add_property("Shape", Enum::from_u32(shape));
    }
    b
}

impl Document {
    fn from_dom(dom: WeakDom, kind: DocKind, path: Option<PathBuf>, title: String) -> Self {
        Document {
            dom,
            limbo: limbo_dom(),
            kind,
            path,
            title,
            cloud: None,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 1,
            saved_revision: 1,
            selection: Vec::new(),
            id: NEXT_DOC_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            tree_revision: 0,
            journal: Journal {
                base: 0,
                marks: Vec::new(),
                read: [None; 15],
                graph_pos: std::sync::atomic::AtomicU64::new(0),
            },
        }
    }

    // -------------------------------------------------------------- journal

    /// Notes a property change (and bumps the revision).
    pub fn mark_prop(&mut self, r: Ref, name: Ustr) {
        self.revision += 1;
        if name == "Name" {
            self.tree_revision += 1;
        }
        self.push_mark(Mark::Prop(r, name));
    }

    /// Notes that `r`'s subtree was created, moved or removed.
    pub fn mark_tree(&mut self, r: Ref) {
        self.revision += 1;
        self.tree_revision += 1;
        self.push_mark(Mark::Tree(r));
    }

    /// Everything may have changed: readers rebuild.
    pub fn mark_all(&mut self) {
        self.revision += 1;
        self.tree_revision += 1;
        let j = &mut self.journal;
        j.base += j.marks.len() as u64 + 1;
        j.marks.clear();
    }

    fn push_mark(&mut self, m: Mark) {
        let j = &mut self.journal;
        if j.marks.len() >= mech::JOURNAL_LIMIT {
            // Nobody is reading fast enough: drop it and let readers rebuild.
            self.mark_all();
            return;
        }
        j.marks.push(m);
    }

    /// The marks `reader` hasn't seen yet (All on its first call, or when it
    /// fell too far behind). Marks every reader has seen are dropped.
    pub fn marks_for(&mut self, reader: Reader) -> Marks {
        let j = &mut self.journal;
        let end = j.base + j.marks.len() as u64;
        let slot = reader as usize;
        let out = match j.read[slot] {
            Some(pos) if pos >= j.base => Marks::Some(j.marks[(pos - j.base) as usize..].to_vec()),
            _ => Marks::All,
        };
        j.read[slot] = Some(end);
        // A reader that stopped reading doesn't hold the journal back: it
        // rebuilds from scratch if it comes back.
        for r in j.read.iter_mut() {
            if r.is_some_and(|p| end - p > mech::JOURNAL_READER_LAG as u64) {
                *r = None;
            }
        }
        use std::sync::atomic::Ordering::Relaxed;
        let gp = j.graph_pos.load(Relaxed);
        if gp > 0 && end - (gp - 1) > mech::JOURNAL_READER_LAG as u64 {
            j.graph_pos.store(0, Relaxed);
        }
        let gp = j.graph_pos.load(Relaxed).checked_sub(1);
        let keep_from = j.read.iter().flatten().copied().chain(gp).min().unwrap_or(end);
        if keep_from > j.base {
            let n = ((keep_from - j.base) as usize).min(j.marks.len());
            j.marks.drain(..n);
            j.base += n as u64;
        }
        out
    }

    /// The marks since the joint graph last looked (All the first time or
    /// after it fell behind). Works through `&self` so property reads can
    /// keep the graph current without a mutable document.
    pub fn graph_marks(&self) -> Marks {
        use std::sync::atomic::Ordering::Relaxed;
        let j = &self.journal;
        let end = j.base + j.marks.len() as u64;
        let out = match j.graph_pos.load(Relaxed).checked_sub(1) {
            Some(pos) if pos >= j.base => Marks::Some(j.marks[(pos - j.base) as usize..].to_vec()),
            _ => Marks::All,
        };
        j.graph_pos.store(end + 1, Relaxed);
        out
    }

    /// `r` and its descendants, whether in the DOM or removed (in limbo).
    pub fn subtree(&self, r: Ref) -> Vec<Ref> {
        let dom = if self.dom.get_by_ref(r).is_some() {
            &self.dom
        } else {
            &self.limbo
        };
        if dom.get_by_ref(r).is_none() {
            return vec![r];
        }
        dom.descendants_of(r).map(|i| i.referent()).collect()
    }

    /// A fresh place like the starter "Baseplate" template.
    pub fn new_place() -> Self {
        let bp = mech::BASEPLATE_SIZE;
        let workspace = InstanceBuilder::new("Workspace")
            .with_name("Workspace")
            .with_child(InstanceBuilder::new("Camera").with_name("Camera"))
            .with_child(
                new_part(
                    "Baseplate",
                    1,
                    glam::Vec3::new(0.0, -bp[1] / 2.0, 0.0),
                    glam::Vec3::from(bp),
                    mech::BASEPLATE_COLOR,
                )
                .with_property("Locked", true),
            )
            .with_child(
                InstanceBuilder::new("SpawnLocation")
                    .with_name("SpawnLocation")
                    .with_property("Anchored", true)
                    .with_property("Size", Vector3::new(12.0, 1.0, 12.0))
                    .with_property(
                        "CFrame",
                        CFrame::new(Vector3::new(0.0, 0.5, 0.0), CFrame::identity().orientation),
                    )
                    .with_property("Color", Color3uint8::new(163, 162, 165)),
            );
        let lighting = InstanceBuilder::new("Lighting")
            .with_name("Lighting")
            .with_property("ClockTime", mech::CLOCK_TIME)
            .with_property("Brightness", mech::LIGHT_BRIGHTNESS)
            // Studio's Baseplate template is lit by Future (Realistic).
            .with_property(
                "Technology",
                rbx_dom_weak::types::Enum::from_u32(mech::TECHNOLOGY_FUTURE),
            );
        let player = InstanceBuilder::new("StarterPlayer")
            .with_name("StarterPlayer")
            .with_child(
                InstanceBuilder::new("StarterPlayerScripts").with_name("StarterPlayerScripts"),
            )
            .with_child(
                InstanceBuilder::new("StarterCharacterScripts")
                    .with_name("StarterCharacterScripts"),
            );
        let mut root = InstanceBuilder::new("DataModel").with_name("Game");
        root.add_child(workspace);
        root.add_child(InstanceBuilder::new("Players").with_name("Players"));
        root.add_child(lighting);
        for class in [
            "ReplicatedFirst",
            "ReplicatedStorage",
            "ServerScriptService",
            "ServerStorage",
            "StarterGui",
            "StarterPack",
        ] {
            root.add_child(InstanceBuilder::new(class).with_name(class));
        }
        root.add_child(player);
        for class in ["Teams", "SoundService", "TextChatService"] {
            root.add_child(InstanceBuilder::new(class).with_name(class));
        }
        let mut doc = Self::from_dom(
            WeakDom::new(root),
            DocKind::Place,
            None,
            "Untitled place".into(),
        );
        // What every Studio place has (Terrain, the Camera, the services),
        // the same as an opened file gets.
        doc.ensure_services();
        doc
    }

    /// A throwaway document (used while the real one is lent to Lua).
    /// Wraps a DOM built for a play session (the client's copy).
    pub fn from_play_dom(dom: WeakDom, title: String) -> Self {
        Self::from_dom(dom, DocKind::Place, None, title)
    }

    pub fn empty() -> Self {
        Self::from_dom(
            WeakDom::new(InstanceBuilder::new("DataModel")),
            DocKind::Place,
            None,
            String::new(),
        )
    }

    /// A deep copy of the whole DataModel for a play session (new refs; Ref
    /// properties inside the copy point at the copied instances).
    pub fn clone_for_play(&self, title: String) -> Self {
        let root = self.dom.root();
        let mut builder = InstanceBuilder::new(root.class).with_name(root.name.clone());
        for (k, v) in &root.properties {
            builder.add_property(*k, v.clone());
        }
        let mut dom = WeakDom::new(builder);
        let kids = root.children().to_vec();
        let copies = self.dom.clone_multiple_into_external(&kids, &mut dom);
        let new_root = dom.root_ref();
        for c in copies {
            dom.transfer_within(c, new_root);
        }
        let mut doc = Self::from_dom(dom, DocKind::Place, self.path.clone(), title);
        doc.cloud = self.cloud.clone();
        doc
    }

    pub fn kind_for(path: &Path) -> Option<(DocKind, bool)> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "rbxl" => Some((DocKind::Place, false)),
            "rbxlx" => Some((DocKind::Place, true)),
            "rbxm" => Some((DocKind::Model, false)),
            "rbxmx" => Some((DocKind::Model, true)),
            _ => None,
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<WeakDom, String> {
        let xml = bytes.starts_with(b"<roblox") && !bytes.starts_with(b"<roblox!");
        if xml {
            rbx_xml::from_reader_default(Cursor::new(bytes)).map_err(|e| e.to_string())
        } else {
            rbx_binary::from_reader(Cursor::new(bytes)).map_err(|e| e.to_string())
        }
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let (kind, _) = Self::kind_for(path)
            .ok_or("unsupported file type (use .rbxl, .rbxlx, .rbxm or .rbxmx)")?;
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let dom = Self::decode(&bytes)?;
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut doc = Self::from_dom(dom, kind, Some(path.to_path_buf()), title);
        doc.ensure_services();
        Ok(doc)
    }

    pub fn from_place_bytes(bytes: &[u8], title: String) -> Result<Self, String> {
        let mut doc = Self::from_dom(Self::decode(bytes)?, DocKind::Place, None, title);
        doc.ensure_services();
        Ok(doc)
    }

    /// Every Roblox DataModel has these services, whatever the file lists
    /// (a place file may hold only a few, e.g. ReplicatedStorage,
    /// ServerScriptService and StarterPlayer): Studio makes the missing ones
    /// when it loads the place, and Workspace always has a Terrain and a
    /// Camera that is its CurrentCamera. Returns the services added.
    pub fn ensure_services(&mut self) -> Vec<&'static str> {
        if self.kind != DocKind::Place {
            return Vec::new();
        }
        let root = self.dom.root_ref();
        let mut added = Vec::new();
        for svc in mech::ALWAYS_SERVICES {
            if self.find_child(root, svc).is_none() {
                self.dom
                    .insert(root, InstanceBuilder::new(*svc).with_name(*svc));
                added.push(*svc);
            }
        }
        if let Some(sp) = self.find_child(root, "StarterPlayer") {
            for svc in ["StarterPlayerScripts", "StarterCharacterScripts"] {
                if self.find_child(sp, svc).is_none() {
                    self.dom.insert(sp, InstanceBuilder::new(svc).with_name(svc));
                }
            }
        }
        if let Some(ws) = self.workspace() {
            if self.find_child(ws, "Terrain").is_none() {
                self.dom
                    .insert(ws, InstanceBuilder::new("Terrain").with_name("Terrain"));
            }
            let current = match self.prop(ws, "CurrentCamera") {
                Some(Variant::Ref(c)) if self.exists(c) => Some(c),
                _ => None,
            };
            if current.is_none() {
                let cam = self.find_child(ws, "Camera").unwrap_or_else(|| {
                    self.dom.insert(
                        ws,
                        InstanceBuilder::new("Camera")
                            .with_name("Camera")
                            .with_property("FieldOfView", mech::FOV_DEG),
                    )
                });
                if let Some(i) = self.dom.get_by_ref_mut(ws) {
                    i.properties
                        .insert(rbx_dom_weak::ustr("CurrentCamera"), Variant::Ref(cam));
                }
            }
        }
        // A place saved without LightingStyle: Studio takes it from
        // Technology (tools/fidelity: Place A, which saves neither, reads Soft
        // with PrioritizeLightingQuality false; the control place, Future,
        // reads Realistic with it true; Place B, ShadowMap, Soft with it true).
        if let Some(l) = self.find_child(root, "Lighting") {
            if let Some(i) = self.dom.get_by_ref_mut(l) {
                let key = |k: &str| rbx_dom_weak::ustr(k);
                let tech = crate::scene::enum_u32(i.properties.get(&key("Technology")).cloned());
                if !i.properties.contains_key(&key("LightingStyle")) {
                    // Neither saved (Place A: its chapel is built at run time)
                    // reads Soft too: Studio's default.
                    let t = tech.unwrap_or(mech::TECHNOLOGY_SOFT[1]);
                    let soft = mech::TECHNOLOGY_SOFT.contains(&t);
                    i.properties.insert(
                        key("LightingStyle"),
                        Variant::Enum(rbx_dom_weak::types::Enum::from_u32(u32::from(soft))),
                    );
                    i.properties
                        .entry(key("PrioritizeLightingQuality"))
                        .or_insert(Variant::Bool(mech::TECHNOLOGY_QUALITY.contains(&t)));
                }
            }
        }
        added
    }

    pub fn encode(&self, refs: &[Ref], xml: bool) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        if xml {
            rbx_xml::to_writer_default(&mut out, &self.dom, refs).map_err(|e| e.to_string())?;
        } else {
            rbx_binary::to_writer(&mut out, &self.dom, refs).map_err(|e| e.to_string())?;
        }
        Ok(out)
    }

    /// The whole document in its file format (place or model).
    pub fn encode_all(&self, xml: bool) -> Result<Vec<u8>, String> {
        let top = self.dom.root().children().to_vec();
        self.encode(&top, xml)
    }

    pub fn save_to(&mut self, path: &Path) -> Result<(), String> {
        let (kind, xml) = Self::kind_for(path).ok_or("save as .rbxl, .rbxlx, .rbxm or .rbxmx")?;
        let bytes = self.encode_all(xml)?;
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        self.kind = kind;
        self.path = Some(path.to_path_buf());
        self.title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.saved_revision = self.revision;
        Ok(())
    }

    /// Writes only the given instances as a model file.
    pub fn save_selection_to(&self, refs: &[Ref], path: &Path) -> Result<(), String> {
        let xml = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("rbxmx"));
        let bytes = self.encode(&self.top_level(refs), xml)?;
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// Marks the document saved as of `revision` (a save that ran in the
    /// background: edits made since stay unsaved).
    pub fn mark_saved_at(&mut self, revision: u64) {
        self.saved_revision = revision;
    }

    // ---------------------------------------------------------------- queries

    pub fn find_child(&self, parent: Ref, class: &str) -> Option<Ref> {
        let p = self.dom.get_by_ref(parent)?;
        p.children()
            .iter()
            .copied()
            .find(|c| self.dom.get_by_ref(*c).is_some_and(|i| i.class == class))
    }

    /// First descendant of `ancestor` with this name.
    pub fn find_named(&self, ancestor: Ref, name: &str) -> Option<Ref> {
        // (A ref from the other DataModel, or a removed one: nothing here.)
        self.dom.get_by_ref(ancestor)?;
        self.dom
            .descendants_of(ancestor)
            .find(|i| i.referent() != ancestor && i.name == name)
            .map(|i| i.referent())
    }

    pub fn workspace(&self) -> Option<Ref> {
        self.find_child(self.dom.root_ref(), "Workspace")
    }

    /// Where new objects go: Workspace for places, the root for models.
    pub fn default_parent(&self) -> Ref {
        match self.kind {
            DocKind::Place => self.workspace().unwrap_or_else(|| self.dom.root_ref()),
            DocKind::Model => self.dom.root_ref(),
        }
    }

    pub fn exists(&self, r: Ref) -> bool {
        self.dom.get_by_ref(r).is_some()
    }

    pub fn is_ancestor(&self, ancestor: Ref, of: Ref) -> bool {
        let mut cur = self.dom.get_by_ref(of).map(|i| i.parent());
        while let Some(p) = cur {
            if p.is_none() {
                return false;
            }
            if p == ancestor {
                return true;
            }
            cur = self.dom.get_by_ref(p).map(|i| i.parent());
        }
        false
    }

    /// Drops refs whose ancestor is also in the list (for copy/move/delete).
    pub fn top_level(&self, refs: &[Ref]) -> Vec<Ref> {
        refs.iter()
            .copied()
            .filter(|r| self.exists(*r) && !refs.iter().any(|a| a != r && self.is_ancestor(*a, *r)))
            .collect()
    }

    pub fn class_of(&self, r: Ref) -> Option<Ustr> {
        self.dom.get_by_ref(r).map(|i| i.class)
    }

    /// Reads a property, falling back to the class default.
    pub fn prop(&self, r: Ref, name: &str) -> Option<Variant> {
        self.prop_key(r, ustr(name))
    }

    /// `prop` by an already interned name (the runtime's member routes).
    pub fn prop_key(&self, r: Ref, key: rbx_dom_weak::Ustr) -> Option<Variant> {
        let name = key.as_str();
        let inst = self.dom.get_by_ref(r)?;
        if let Some(v) = inst.properties.get(&key) {
            return Some(v.clone());
        }
        // What doesn't depend on the instance (aliases, class defaults) is
        // worked out once per (class, name): the reflection walks it takes
        // were ~50 µs a part when a 50k-part map was cloned in.
        let plan = prop_plan(inst.class, key)?;
        if let Some(alt) = plan.stored_as {
            if let Some(v) = inst.properties.get(&alt) {
                return Some(v.clone());
            }
        }
        // Newer files keep asset ids in Content twins (MeshId → MeshContent …);
        // a saved twin wins over any class default.
        match plan.twin.and_then(|t| inst.properties.get(&t)) {
            Some(Variant::Content(c)) => {
                if let Some(uri) = c.as_uri() {
                    return Some(Variant::ContentId(rbx_dom_weak::types::ContentId::from(
                        uri.to_string(),
                    )));
                }
                // An EditableImage / EditableMesh (runtime/editable.rs): its
                // current generated id; this instance redraws on its writes.
                if let Some(uri) = c
                    .as_object()
                    .and_then(|o| crate::runtime::editable::content_uri(o, Some((r, name))))
                {
                    return Some(Variant::ContentId(rbx_dom_weak::types::ContentId::from(uri)));
                }
            }
            // Team Create's live stream gives Content twins as plain strings.
            Some(Variant::String(uri)) => {
                return Some(Variant::ContentId(rbx_dom_weak::types::ContentId::from(
                    uri.clone(),
                )));
            }
            _ => {}
        }
        if let Some(v) = &plan.fixed {
            return Some(v.clone());
        }
        // Not serialised itself: the value lives in its XML twin
        // (Humanoid.Health → Health_XML) or, for deprecated lowercase names,
        // in the current property (maxHealth → MaxHealth).
        if let Some(v) = inst.properties.get(&plan.xml) {
            return Some(v.clone());
        }
        if let Some(v) = &plan.xml_default {
            return Some(v.clone());
        }
        match &plan.upper {
            Some(u) => self.prop(r, u),
            None => None,
        }
    }

    fn can_edit_structure(&self, r: Ref) -> bool {
        r != self.dom.root_ref() && self.class_of(r).is_some_and(|c| !is_service(&c))
    }

    // ----------------------------------------------------------- undo / redo

    fn mark_change(&mut self, c: &Change) {
        match c {
            Change::Prop { r, name, .. } => self.mark_prop(*r, *name),
            Change::Name { r, .. } => self.mark_prop(*r, ustr("Name")),
            Change::Parent { r, .. } | Change::Create { r, .. } | Change::Remove { r, .. } => {
                self.mark_tree(*r)
            }
        }
    }

    fn apply(&mut self, c: &Change, forward: bool) {
        self.apply_inner(c, forward);
        self.mark_change(c);
    }

    fn apply_inner(&mut self, c: &Change, forward: bool) {
        match c {
            Change::Prop { r, name, old, new } => {
                let v = if forward { new } else { old };
                if let Some(inst) = self.dom.get_by_ref_mut(*r) {
                    match v {
                        Some(v) => {
                            inst.properties.insert(*name, v.clone());
                        }
                        None => {
                            inst.properties.remove(name);
                        }
                    }
                }
            }
            Change::Name { r, old, new } => {
                if let Some(inst) = self.dom.get_by_ref_mut(*r) {
                    inst.name = if forward { new.clone() } else { old.clone() };
                }
            }
            Change::Parent { r, old, new } => {
                let to = if forward { *new } else { *old };
                if self.exists(*r) && self.exists(to) {
                    self.dom.transfer_within(*r, to);
                }
            }
            Change::Create { r, parent } | Change::Remove { r, parent } => {
                let into_dom = matches!(c, Change::Create { .. }) == forward;
                if into_dom {
                    if self.limbo.get_by_ref(*r).is_some() && self.exists(*parent) {
                        self.limbo.transfer(*r, &mut self.dom, *parent);
                    }
                } else if self.exists(*r) {
                    let lr = self.limbo.root_ref();
                    self.dom.transfer(*r, &mut self.limbo, lr);
                }
            }
        }
    }

    /// Records an already-applied command. Commands with the same merge key
    /// within a short window fold together (dragging a number, typing).
    fn push(&mut self, label: impl Into<String>, changes: Vec<Change>, merge_key: Option<String>) {
        if changes.is_empty() {
            return;
        }
        for c in &changes {
            self.mark_change(c);
        }
        self.redo.clear();
        let now = Instant::now();
        if let (Some(key), Some(last)) = (&merge_key, self.undo.last_mut()) {
            if last.merge_key.as_ref() == Some(key)
                && now.duration_since(last.at) < Duration::from_millis(mech::UNDO_MERGE_MS)
            {
                for c in changes {
                    if let Change::Prop { r, name, new, .. } = &c {
                        if let Some(Change::Prop { new: n, .. }) = last.changes.iter_mut().find(|e| matches!(e, Change::Prop { r: r2, name: n2, .. } if r2 == r && n2 == name)) {
                            *n = new.clone();
                            continue;
                        }
                    }
                    last.changes.push(c);
                }
                last.at = now;
                return;
            }
        }
        let sel_before = self.selection.clone();
        self.undo.push(Command {
            label: label.into(),
            changes,
            merge_key,
            at: now,
            sel_before,
            sel_after: Vec::new(),
        });
        if self.undo.len() > mech::UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    /// How many undo steps there are.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn can_undo(&self) -> Option<&str> {
        self.undo.last().map(|c| c.label.as_str())
    }

    pub fn can_redo(&self) -> Option<&str> {
        self.redo.last().map(|c| c.label.as_str())
    }

    pub fn undo(&mut self) -> Option<String> {
        let mut cmd = self.undo.pop()?;
        cmd.sel_after = self.selection.clone();
        for c in cmd.changes.iter().rev() {
            self.apply(c, false);
        }
        self.revision += 1;
        self.selection = cmd.sel_before.clone();
        self.prune_selection();
        let label = cmd.label.clone();
        self.redo.push(cmd);
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let mut cmd = self.redo.pop()?;
        for c in cmd.changes.iter() {
            self.apply(c, true);
        }
        self.revision += 1;
        self.selection = cmd.sel_after.clone();
        self.prune_selection();
        let label = cmd.label.clone();
        cmd.merge_key = None;
        self.undo.push(cmd);
        Some(label)
    }

    /// Forgets undo history and marks the document unchanged.
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.saved_revision = self.revision;
    }

    fn prune_selection(&mut self) {
        let sel = std::mem::take(&mut self.selection);
        self.selection = sel
            .into_iter()
            .filter(|r| self.dom.get_by_ref(*r).is_some())
            .collect();
    }

    // ---------------------------------------------------------------- edits

    /// Sets properties on many instances as one undo step.
    pub fn set_props(
        &mut self,
        label: &str,
        edits: Vec<(Ref, Ustr, Variant)>,
        merge_key: Option<String>,
    ) {
        let mut changes = Vec::new();
        for (r, name, value) in edits {
            let Some(inst) = self.dom.get_by_ref_mut(r) else {
                continue;
            };
            let old = inst.properties.insert(name, value.clone());
            if old.as_ref() != Some(&value) {
                changes.push(Change::Prop {
                    r,
                    name,
                    old,
                    new: Some(value),
                });
            }
        }
        self.push(label, changes, merge_key);
    }

    /// Records property values that were already changed live (gizmo drags).
    pub fn record_props(
        &mut self,
        label: &str,
        changes: Vec<(Ref, Ustr, Option<Variant>, Variant)>,
    ) {
        let changes = changes
            .into_iter()
            .filter(|(_, _, old, new)| old.as_ref() != Some(new))
            .map(|(r, name, old, new)| Change::Prop {
                r,
                name,
                old,
                new: Some(new),
            })
            .collect();
        self.push(label, changes, None);
    }

    /// Changes a property without recording undo (live preview while dragging).
    pub fn set_prop_live(&mut self, r: Ref, name: Ustr, value: Variant) {
        if let Some(inst) = self.dom.get_by_ref_mut(r) {
            inst.properties.insert(name, value);
            self.mark_prop(r, name);
        }
    }

    pub fn rename(&mut self, r: Ref, name: &str) {
        let Some(inst) = self.dom.get_by_ref_mut(r) else {
            return;
        };
        if inst.name == name {
            return;
        }
        let old = std::mem::replace(&mut inst.name, name.to_string());
        self.push(
            format!("Rename to {name}"),
            vec![Change::Name {
                r,
                old,
                new: name.into(),
            }],
            None,
        );
    }

    /// Moves instances under a new parent. Refuses cycles, services and the root.
    pub fn reparent(&mut self, refs: &[Ref], new_parent: Ref) -> usize {
        if !self.exists(new_parent) {
            return 0;
        }
        let mut changes = Vec::new();
        for r in self.top_level(refs) {
            if r == new_parent || self.is_ancestor(r, new_parent) || !self.can_edit_structure(r) {
                continue;
            }
            let old = self
                .dom
                .get_by_ref(r)
                .map(|i| i.parent())
                .unwrap_or_else(Ref::none);
            if old == new_parent {
                continue;
            }
            self.dom.transfer_within(r, new_parent);
            changes.push(Change::Parent {
                r,
                old,
                new: new_parent,
            });
        }
        let n = changes.len();
        self.push("Reparent", changes, None);
        n
    }

    pub fn insert(&mut self, parent: Ref, builder: InstanceBuilder) -> Ref {
        let parent = if self.exists(parent) {
            parent
        } else {
            self.default_parent()
        };
        let r = self.dom.insert(parent, builder);
        let class = self.class_of(r).map(|c| c.to_string()).unwrap_or_default();
        self.push(
            format!("Insert {class}"),
            vec![Change::Create { r, parent }],
            None,
        );
        r
    }

    pub fn remove(&mut self, refs: &[Ref]) -> usize {
        let mut changes = Vec::new();
        for r in self.top_level(refs) {
            if !self.can_edit_structure(r) {
                continue;
            }
            let parent = self
                .dom
                .get_by_ref(r)
                .map(|i| i.parent())
                .unwrap_or_else(Ref::none);
            let lr = self.limbo.root_ref();
            self.dom.transfer(r, &mut self.limbo, lr);
            changes.push(Change::Remove { r, parent });
        }
        let n = changes.len();
        self.push("Delete", changes, None);
        self.prune_selection();
        n
    }

    /// Serialises instances into clipboard bytes (binary model format).
    pub fn copy(&self, refs: &[Ref]) -> Option<Vec<u8>> {
        let top: Vec<Ref> = self
            .top_level(refs)
            .into_iter()
            .filter(|r| self.can_edit_structure(*r))
            .collect();
        if top.is_empty() {
            return None;
        }
        self.encode(&top, false).ok()
    }

    /// Inserts clipboard bytes under `parent`; returns the new top-level refs.
    pub fn paste(&mut self, bytes: &[u8], parent: Ref, label: &str) -> Vec<Ref> {
        let Ok(mut tmp) = Self::decode(bytes) else {
            return Vec::new();
        };
        let parent = if self.exists(parent) {
            parent
        } else {
            self.default_parent()
        };
        let roots = tmp.root().children().to_vec();
        let mut changes = Vec::new();
        for r in &roots {
            tmp.transfer(*r, &mut self.dom, parent);
            changes.push(Change::Create { r: *r, parent });
        }
        self.push(label, changes, None);
        roots
    }

    /// Duplicates each instance next to the original.
    pub fn duplicate(&mut self, refs: &[Ref]) -> Vec<Ref> {
        let mut out = Vec::new();
        let mut changes = Vec::new();
        for r in self.top_level(refs) {
            if !self.can_edit_structure(r) {
                continue;
            }
            let parent = self
                .dom
                .get_by_ref(r)
                .map(|i| i.parent())
                .unwrap_or_else(Ref::none);
            let Ok(bytes) = self.encode(&[r], false) else {
                continue;
            };
            let Ok(mut tmp) = Self::decode(&bytes) else {
                continue;
            };
            for nr in tmp.root().children().to_vec() {
                tmp.transfer(nr, &mut self.dom, parent);
                changes.push(Change::Create { r: nr, parent });
                out.push(nr);
            }
        }
        self.push("Duplicate", changes, None);
        out
    }

    /// Wraps the instances in a new Model under their first common parent.
    pub fn group(&mut self, refs: &[Ref]) -> Option<Ref> {
        self.group_as(refs, "Model")
    }

    /// Groups into a new `class` (Model, or Folder: Studio's Group as Folder).
    pub fn group_as(&mut self, refs: &[Ref], class: &str) -> Option<Ref> {
        let top: Vec<Ref> = self
            .top_level(refs)
            .into_iter()
            .filter(|r| self.can_edit_structure(*r))
            .collect();
        let first = *top.first()?;
        let parent = self.dom.get_by_ref(first)?.parent();
        let model = self
            .dom
            .insert(parent, InstanceBuilder::new(class).with_name(class));
        let mut changes = vec![Change::Create { r: model, parent }];
        for r in top {
            let old = self
                .dom
                .get_by_ref(r)
                .map(|i| i.parent())
                .unwrap_or_else(Ref::none);
            self.dom.transfer_within(r, model);
            changes.push(Change::Parent { r, old, new: model });
        }
        self.push("Group", changes, None);
        Some(model)
    }

    /// Moves a Model/Folder's children up and removes it.
    pub fn ungroup(&mut self, refs: &[Ref]) -> Vec<Ref> {
        let mut out = Vec::new();
        let mut changes = Vec::new();
        for r in self.top_level(refs) {
            let Some(class) = self.class_of(r) else {
                continue;
            };
            if !(class == "Model" || class == "Folder") || !self.can_edit_structure(r) {
                continue;
            }
            let parent = self
                .dom
                .get_by_ref(r)
                .map(|i| i.parent())
                .unwrap_or_else(Ref::none);
            for c in self
                .dom
                .get_by_ref(r)
                .map(|i| i.children().to_vec())
                .unwrap_or_default()
            {
                self.dom.transfer_within(c, parent);
                changes.push(Change::Parent {
                    r: c,
                    old: r,
                    new: parent,
                });
                out.push(c);
            }
            let lr = self.limbo.root_ref();
            self.dom.transfer(r, &mut self.limbo, lr);
            changes.push(Change::Remove { r, parent });
        }
        self.push("Ungroup", changes, None);
        out
    }

    /// Counts instances of a class (IsA) in the whole document.
    /// Sources of scripts whose full name contains `pattern`.
    /// (full name, source) of every script (development: --dump-scripts).
    pub fn all_script_sources(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for i in self.dom.descendants() {
            if !matches!(i.class.as_str(), "Script" | "LocalScript" | "ModuleScript") {
                continue;
            }
            let mut path = Vec::new();
            let mut cur = i.referent();
            while let Some(x) = self.dom.get_by_ref(cur) {
                if x.parent().is_none() {
                    break;
                }
                path.push(x.name.replace(['/', '\\'], "_"));
                cur = x.parent();
            }
            path.reverse();
            if let Some(Variant::String(src)) = i.properties.get(&ustr("Source")) {
                out.push((path.join("."), src.clone()));
            }
        }
        out
    }

    pub fn script_sources(&self, pattern: &str) -> String {
        let mut out = String::new();
        for i in self.dom.descendants() {
            if !matches!(i.class.as_str(), "Script" | "LocalScript" | "ModuleScript") {
                continue;
            }
            let mut path = Vec::new();
            let mut cur = i.referent();
            while let Some(x) = self.dom.get_by_ref(cur) {
                if x.parent().is_none() {
                    break;
                }
                path.push(x.name.clone());
                cur = x.parent();
            }
            path.reverse();
            let full = path.join(".");
            if !full.contains(pattern) {
                continue;
            }
            let src = match i.properties.get(&ustr("Source")) {
                Some(Variant::String(s)) => s.clone(),
                _ => String::new(),
            };
            out.push_str(&format!("==== {full} ({})\n", i.class));
            for (n, line) in src.lines().enumerate().take(60) {
                out.push_str(&format!("{:4} {line}\n", n + 1));
            }
        }
        out
    }

    /// A text summary of the place: instance counts by class and part flags.
    pub fn stats(&self) -> String {
        let mut by_class: std::collections::HashMap<String, usize> = Default::default();
        let (mut parts, mut anchored, mut collide_off, mut in_ws) = (0, 0, 0, 0);
        let ws = self.workspace();
        for i in self.dom.descendants() {
            *by_class.entry(i.class.to_string()).or_default() += 1;
            if is_a(&i.class, "BasePart") {
                parts += 1;
                if matches!(
                    self.prop(i.referent(), "Anchored"),
                    Some(Variant::Bool(true))
                ) {
                    anchored += 1;
                }
                if matches!(
                    self.prop(i.referent(), "CanCollide"),
                    Some(Variant::Bool(false))
                ) {
                    collide_off += 1;
                }
                if ws.is_some_and(|w| self.is_ancestor(w, i.referent())) {
                    in_ws += 1;
                }
            }
        }
        let mut v: Vec<_> = by_class.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        let mut out = format!("parts {parts} (in Workspace {in_ws}, anchored {anchored}, CanCollide off {collide_off})\n");
        for (c, n) in v.iter().take(60) {
            out.push_str(&format!("{n:>8} {c}\n"));
        }
        out
    }

    pub fn count_class(&self, base: &str) -> usize {
        self.dom
            .descendants()
            .filter(|i| is_a(&i.class, base))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_insert_delete() {
        let mut d = Document::new_place();
        let ws = d.workspace().unwrap();
        let n0 = d.count_class("BasePart");
        let p = d.insert(
            ws,
            new_part("P", 1, glam::Vec3::ZERO, glam::Vec3::ONE, [1, 2, 3]),
        );
        assert_eq!(d.count_class("BasePart"), n0 + 1);
        d.remove(&[p]);
        assert_eq!(d.count_class("BasePart"), n0);
        d.undo();
        assert!(d.exists(p));
        d.undo();
        assert!(!d.exists(p));
        d.redo();
        assert!(d.exists(p));
        let dup = d.duplicate(&[p]);
        assert_eq!(dup.len(), 1);
        let bytes = d.encode_all(false).unwrap();
        let back = Document::from_place_bytes(&bytes, "t".into()).unwrap();
        assert_eq!(back.count_class("BasePart"), d.count_class("BasePart"));
        let xml = d.encode_all(true).unwrap();
        let back = Document::from_place_bytes(&xml, "t".into()).unwrap();
        assert_eq!(back.count_class("BasePart"), d.count_class("BasePart"));
    }
}

/// The Content twin newer files keep a legacy asset-id property in
/// (MeshId → MeshContent …). Roblox treats the pair as one property: setting
/// either changes both.
pub fn content_twin(name: &str) -> Option<&'static str> {
    Some(match name {
        "MeshId" => "MeshContent",
        "TextureID" | "TextureId" | "Texture" => "TextureContent",
        "ShirtTemplate" => "ShirtTemplateContent",
        "PantsTemplate" => "PantsTemplateContent",
        "Graphic" => "GraphicContent",
        "Image" => "ImageContent",
        "ColorMap" => "ColorMapContent",
        "NormalMap" => "NormalMapContent",
        "MetalnessMap" => "MetalnessMapContent",
        "RoughnessMap" => "RoughnessMapContent",
        "SoundId" => "AudioContent",
        "AnimationId" => "AnimationContent",
        // Sky (2026-11-05: Place B saves only these; rustudio
        // drew the default sky instead of its own).
        "SkyboxBk" => "SkyboxBackContent",
        "SkyboxDn" => "SkyboxDownContent",
        "SkyboxFt" => "SkyboxFrontContent",
        "SkyboxLf" => "SkyboxLeftContent",
        "SkyboxRt" => "SkyboxRightContent",
        "SkyboxUp" => "SkyboxUpContent",
        "SunTextureId" => "SunTextureContent",
        "MoonTextureId" => "MoonTextureContent",
        _ => return None,
    })
}

/// The legacy names whose Content twin is `twin` (TextureContent → Texture,
/// TextureID, TextureId).
pub fn content_legacy_names(twin: &str) -> Vec<&'static str> {
    const LEGACY: [&str; 22] = [
        "MeshId", "TextureID", "TextureId", "Texture", "ShirtTemplate", "PantsTemplate", "Graphic",
        "Image", "ColorMap", "NormalMap", "MetalnessMap", "RoughnessMap", "SoundId", "AnimationId",
        "SkyboxBk", "SkyboxDn", "SkyboxFt", "SkyboxLf", "SkyboxRt", "SkyboxUp", "SunTextureId",
        "MoonTextureId",
    ];
    LEGACY.into_iter().filter(|l| content_twin(l) == Some(twin)).collect()
}

/// The instances of some classes, kept current from a document's change
/// journal instead of walking the whole DataModel: a full scan only the first
/// time (or when the reader fell behind), then just the subtrees that were
/// added, moved or removed. Full scans list in DOM order; later finds are
/// appended.
pub struct ClassIndex {
    reader: Reader,
    doc_id: u64,
    list: Vec<Ref>,
    set: std::collections::HashSet<Ref>,
}

impl ClassIndex {
    pub fn new(reader: Reader) -> Self {
        ClassIndex { reader, doc_id: 0, list: Vec::new(), set: Default::default() }
    }

    pub fn list(&self) -> &[Ref] {
        &self.list
    }

    /// Brings the index up to date with `d`; `want` picks classes. Returns
    /// whether the membership changed.
    pub fn update(&mut self, d: &mut Document, want: impl Fn(&str) -> bool) -> bool {
        let marks = d.marks_for(self.reader);
        let marks = match marks {
            Marks::Some(m) if self.doc_id == d.id => m,
            _ => {
                let list: Vec<Ref> = d
                    .dom
                    .descendants()
                    .filter(|i| want(i.class.as_str()))
                    .map(|i| i.referent())
                    .collect();
                let changed = list != self.list || self.doc_id != d.id;
                self.doc_id = d.id;
                self.set = list.iter().copied().collect();
                self.list = list;
                return changed;
            }
        };
        let mut changed = false;
        let mut removed = false;
        let mut tree = false;
        for m in marks {
            let Mark::Tree(t) = m else { continue };
            tree = true;
            for r in d.subtree(t) {
                let live = d.dom.get_by_ref(r).is_some_and(|i| want(i.class.as_str()));
                if live {
                    if self.set.insert(r) {
                        self.list.push(r);
                        changed = true;
                    }
                } else if self.set.remove(&r) {
                    removed = true;
                }
            }
        }
        // Instances freed outright (no longer in the DOM or in limbo) don't
        // come back through `subtree`: drop whatever has left the DOM.
        if removed || tree {
            let before = self.list.len();
            self.list.retain(|r| self.set.contains(r) && d.dom.get_by_ref(*r).is_some());
            if removed || self.list.len() != before {
                self.set = self.list.iter().copied().collect();
                changed = true;
            }
        }
        changed
    }
}
