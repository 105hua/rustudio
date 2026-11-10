//! Placing GUIs on the screen (ScreenGui, SurfaceGui, BillboardGui), painting
//! them, and the mouse / keyboard: hover, clicks, text boxes, scrolling and
//! GuiService selection.

use egui::epaint::{TessellationOptions, Tessellator};
use egui::Shape;
use glam::Mat3;

use super::walk::{Aff, Down, Walk};
use super::*;
use crate::math::Xform;
use crate::mech;
use crate::scene::{vec3_of, xform_of};

/// The GUI's debugging switches (read once).
#[derive(Clone, Copy)]
enum DebugEnv {
    /// RUSTUDIO_GUI_CACHE_WHY: say why each GUI was walked.
    Why,
    /// RUSTUDIO_GUI_CACHE_VERIFY: walk kept GUIs anyway (and once more
    /// without memos) and compare.
    Verify,
    /// RUSTUDIO_GUI_LAYOUT_VERIFY: compare the kept layout with a fresh one.
    LayoutVerify,
    /// RUSTUDIO_GUI_BLACK_BG: ScreenGuis over black (GUI-only captures).
    BlackBg,
}

fn debug_env(e: DebugEnv) -> bool {
    static ON: std::sync::OnceLock<[bool; 4]> = std::sync::OnceLock::new();
    ON.get_or_init(|| {
        ["RUSTUDIO_GUI_CACHE_WHY", "RUSTUDIO_GUI_CACHE_VERIFY", "RUSTUDIO_GUI_LAYOUT_VERIFY", "RUSTUDIO_GUI_BLACK_BG"]
            .map(|n| std::env::var_os(n).is_some())
    })[e as usize]
}

/// The LocalPlayer's PlayerGui (where play-mode ScreenGuis live).
pub(super) fn local_player_gui(d: &Document) -> Option<Ref> {
    d.find_child(d.dom.root_ref(), "Players").and_then(|p| match d.prop(p, "LocalPlayer") {
        Some(Variant::Ref(lp)) if d.exists(lp) => d.find_child(lp, "PlayerGui"),
        _ => None,
    })
}

/// A ScreenGui's output from the frame it was last walked: its triangles
/// on the screen, what can be clicked, and its Stats counts. Drawn again as
/// it is until a journal mark touches its tree or something it was drawn
/// with changes (hover, focus, selection, the font atlas, the view).
///
/// Studio does the same: each GuiBase2d's LayoutNode keeps dirty bits
/// (LayoutDirty, ViewportDirty, MarkedForViewportUpdate, ChildSort,
/// IntrinsicSize, ContentSize…), layout runs only for dirty nodes, and the
/// vertices sit in a retained GUI cluster per layer collector that is
/// refilled only when marked (the frame stats time UILayout,
/// FillGuiVertices and Render2d apart).
pub(super) struct RootCache {
    key: [u32; 12],
    shapes: Vec<Shape>,
    hits: Vec<Hit>,
    /// Stats.UI2D: draw calls, triangles.
    ui2d: (u64, u64),
    vfs: Vec<Ref>,
    /// SurfaceGuis / BillboardGuis: the canvas triangles (with their
    /// bounds for the view test) and hit boxes, mapped again each frame.
    citems: Vec<(Rect, Mesh)>,
    lhits: Vec<walk::LocalHit>,
}

/// The state outside the document that cached GUIs were drawn with.
#[derive(Default)]
pub(super) struct CacheWith {
    hovered: HashSet<Ref>,
    pressed: Option<Ref>,
    focus: Option<Ref>,
    selected: Option<Ref>,
    atlas: [usize; 2],
    fill: f32,
    /// Font files and families egui knows: a font added rebuilds egui's
    /// atlas from scratch (glyphs move) at the next frame.
    fonts: (usize, usize),
    ppp: f32,
}

/// What a GUI root is drawn onto.
#[derive(Clone, Copy)]
enum Target {
    /// Screen: canvas (0,0) at this point.
    Screen(Pos2),
    /// A part's face: canvas corners top-left, top-right, bottom-left (world).
    Surface { tl: Vec3, tr: Vec3, bl: Vec3 },
    /// Billboard: canvas centred on this screen point (its world anchor).
    Billboard(Pos2, Vec3),
}

struct Root {
    /// Drawn over the world (AlwaysOnTop, ScreenGuis), not depth-tested.
    on_top: bool,
    /// LightInfluence / Brightness: the colour factor (gamma space).
    light: f32,
    order: i64,
    dist: f32,
    gui: Ref,
    target: Target,
    canvas: [f32; 2],
    abs_offset: [f32; 2],
}

impl Gui {
    /// Keeps the list of SurfaceGuis / BillboardGuis under Workspace from the
    /// document's journal: scanning Place B's 116k instances for them was
    /// 8 ms a frame. Call before `draw`.
    pub fn sync_roots(&mut self, d: &mut Document) {
        let _p = crate::bench::scope("gui_roots");
        let marks = d.marks_for(doc::Reader::Gui);
        self.vf_dirty(d, &marks);
        self.cache_marks(d, &marks);
        let ws = d.workspace();
        let layer = |d: &Document, r: Ref| {
            d.dom
                .get_by_ref(r)
                .is_some_and(|i| i.class == "SurfaceGui" || i.class == "BillboardGui")
                && ws.is_some_and(|w| d.is_ancestor(w, r))
        };
        // Adornments under the LocalPlayer's PlayerGui come from the same
        // journal (scanning that subtree every frame was 4 ms in Place E); a new
        // PlayerGui (LocalPlayer set, respawn) is scanned once.
        let pg = local_player_gui(d);
        let pg_new = pg != self.adorn_pg_of || d.id != self.roots_doc;
        if pg_new || !matches!(marks, doc::Marks::Some(_)) {
            self.adorn_pg_of = pg;
            self.adorn_pg = match pg {
                Some(pg) => d
                    .subtree(pg)
                    .into_iter()
                    .filter(|r| d.class_of(*r).is_some_and(|c| super::adorn::is_adornment(&c)))
                    .collect(),
                None => HashSet::new(),
            };
            self.adorn_pg_order = None;
        }
        match marks {
            doc::Marks::Some(marks) if d.id == self.roots_doc => {
                for m in marks {
                    let doc::Mark::Tree(t) = m else { continue };
                    for r in d.subtree(t) {
                        let adorn = d.dom.get_by_ref(r).is_some_and(|i| {
                            doc::is_a(&i.class, "HandleAdornment") || doc::is_a(&i.class, "HandlesBase")
                        });
                        if adorn && ws.is_some_and(|w| d.is_ancestor(w, r)) {
                            self.adorn_roots.insert(r);
                        } else {
                            self.adorn_roots.remove(&r);
                        }
                        let in_pg = adorn && pg.is_some_and(|p| d.is_ancestor(p, r));
                        let changed = if in_pg { self.adorn_pg.insert(r) } else { self.adorn_pg.remove(&r) };
                        if changed {
                            self.adorn_pg_order = None;
                        }
                        let keep = layer(d, r);
                        let has = self.world_roots_set.contains(&r);
                        if keep && !has {
                            self.world_roots_set.insert(r);
                            self.world_roots.push(r);
                        } else if !keep && has {
                            self.world_roots_set.remove(&r);
                            self.world_roots.retain(|x| *x != r);
                            self.root_boxes.remove(&r);
                        }
                    }
                }
            }
            _ => {
                self.roots_doc = d.id;
                self.world_roots = match ws {
                    Some(ws) => d
                        .dom
                        .descendants_of(ws)
                        .filter(|i| i.class == "SurfaceGui" || i.class == "BillboardGui")
                        .map(|i| i.referent())
                        .collect(),
                    None => Vec::new(),
                };
                self.world_roots_set = self.world_roots.iter().copied().collect();
                self.adorn_roots = match ws {
                    Some(ws) => d
                        .dom
                        .descendants_of(ws)
                        .filter(|i| doc::is_a(&i.class, "HandleAdornment") || doc::is_a(&i.class, "HandlesBase"))
                        .map(|i| i.referent())
                        .collect(),
                    None => HashSet::new(),
                };
                self.root_boxes.clear();
                self.layout.clear();
                self.layout_from.clear();
                self.layout_changed = true;
            }
        }
    }

    /// Drops the kept output of every ScreenGui a mark touches: a property of
    /// something its walk read, or an instance added, moved or removed where
    /// that walk listed the children (or that it read itself).
    fn cache_marks(&mut self, d: &Document, marks: &doc::Marks) {
        if self.cache_owner.is_empty() {
            return;
        }
        let marks = match marks {
            doc::Marks::Some(m) if d.id == self.roots_doc => m,
            _ => {
                self.cache.clear();
                self.memo.clear();
                return;
            }
        };
        let why_on = debug_env(DebugEnv::Why);
        for m in marks {
            let (r, read, listed) = match *m {
                doc::Mark::Prop(r, _) => (r, self.cache_owner.get(&r).map(|o| (o.0, o.1)), None),
                doc::Mark::Tree(r) => {
                    let parent = d.dom.get_by_ref(r).map(|i| i.parent());
                    (r, self.cache_owner.get(&r).map(|o| (o.0, o.1)), parent.and_then(|p| self.cache_kids.get(&p).map(|g| (*g, p))))
                }
            };
            // The objects whose memos it touches: itself and up, the one that
            // read it and up (it may be gone, or read from elsewhere: a
            // ViewportFrame's camera), the parent that listed it and up.
            if let Some((g, by)) = read {
                self.memo_up(d, r, g);
                self.memo_up(d, by, g);
            }
            if let Some((g, p)) = listed {
                self.memo_up(d, p, g);
            }
            let hit = read.map(|x| x.0).or(listed.map(|x| x.0));
            if let Some(g) = hit {
                if self.cache.remove(&g).is_some() && why_on {
                    let what = match *m {
                        doc::Mark::Prop(_, n) => n.to_string(),
                        doc::Mark::Tree(_) => "<tree>".to_string(),
                    };
                    let class = d.class_of(r).map(|c| c.to_string()).unwrap_or_default();
                    self.cache_why.insert(g, format!("mark {class} {} {what}", name_of(d, r)));
                }
            }
        }
    }

    /// RUSTUDIO_GUI_CACHE_VERIFY: GUI `g` walked again (`shapes` / `hits` on
    /// the screen, `citems` / `lhits` in its canvas) against what was kept.
    #[allow(clippy::too_many_arguments)]
    fn cache_verify(
        &self,
        d: &Document,
        g: Ref,
        k: &RootCache,
        screen: bool,
        shapes: &[Shape],
        hits: &[Hit],
        citems: &[(Rect, Mesh)],
        lhits: &[walk::LocalHit],
    ) {
        let same_mesh = |a: &Mesh, b: &Mesh| {
            a.texture_id == b.texture_id
                && a.indices == b.indices
                && a.vertices.len() == b.vertices.len()
                && a.vertices.iter().zip(&b.vertices).all(|(x, y)| x.pos == y.pos && x.uv == y.uv && x.color == y.color)
        };
        let same_shape = |a: &Shape, b: &Shape| match (a, b) {
            (Shape::Mesh(a), Shape::Mesh(b)) => same_mesh(a, b),
            _ => false,
        };
        let same_hit = |a: &Hit, b: &Hit| a.r == b.r && a.quad == b.quad && a.clip == b.clip && a.sink == b.sink;
        let same_lhit = |a: &walk::LocalHit, b: &walk::LocalHit| a.r == b.r && a.rect == b.rect && a.clip == b.clip && a.sink == b.sink;
        let (n_shape, n_hit) = if screen {
            (
                (k.shapes.len() != shapes.len()).then_some(usize::MAX).or_else(|| k.shapes.iter().zip(shapes).position(|(a, b)| !same_shape(a, b))),
                (k.hits.len() != hits.len()).then_some(usize::MAX).or_else(|| k.hits.iter().zip(hits).position(|(a, b)| !same_hit(a, b))),
            )
        } else {
            (
                (k.citems.len() != citems.len()).then_some(usize::MAX).or_else(|| k.citems.iter().zip(citems).position(|(a, b)| a.0 != b.0 || !same_mesh(&a.1, &b.1))),
                (k.lhits.len() != lhits.len()).then_some(usize::MAX).or_else(|| k.lhits.iter().zip(lhits).position(|(a, b)| !same_lhit(a, b))),
            )
        };
        let same = n_shape.is_none() && n_hit.is_none();
        crate::bench::add(if same { "gui_verify_same" } else { "gui_verify_differ" }, 0.0, 1);
        if !same {
            eprintln!(
                "[guiverify] {} differs from its kept output: first differing triangles {:?}, hit {:?} (usize::MAX: counts differ; {} / {} screen shapes, {} / {} canvas items)",
                name_of(d, g),
                n_shape,
                n_hit,
                k.shapes.len(),
                shapes.len(),
                k.citems.len(),
                citems.len(),
            );
        }
    }

    /// Whether `layout` changed since this was last asked (the viewport
    /// hands it to the game only then).
    pub fn take_layout_change(&mut self) -> bool {
        std::mem::take(&mut self.layout_changed)
    }

    /// Keeps what ScreenGui `g`'s walk read (`walk_reads`, `walk_kids`) for
    /// `cache_marks`, in place of what its last walk read.
    fn cache_own(&mut self, g: Ref) {
        self.own_gen = self.own_gen.wrapping_add(1);
        let gen = self.own_gen;
        let mut reads = std::mem::take(&mut self.walk_reads);
        let kids = std::mem::take(&mut self.walk_kids);
        reads.push((g, g));
        for (x, by) in &reads {
            self.cache_owner.insert(*x, (g, *by, gen));
        }
        let old = self.cache_owned.remove(&g);
        if let Some((old_reads, old_kids)) = &old {
            // What this walk no longer read (under something hidden now, or
            // gone): its entry goes, and so do the memos of it and of what
            // read it, which nothing would keep up to date while unread.
            for (x, by) in old_reads {
                if self.cache_owner.get(x).is_some_and(|o| o.0 == g && o.2 != gen) {
                    self.cache_owner.remove(x);
                    self.memo.remove(x);
                    self.memo.remove(by);
                }
            }
            for x in old_kids {
                if self.cache_kids.get(x) == Some(&g) {
                    self.cache_kids.remove(x);
                }
            }
        }
        for x in &kids {
            self.cache_kids.insert(*x, g);
        }
        self.cache_owned.insert(g, (reads, kids));
    }

    /// Drops the memos of `from` and every object above it, up to GUI `g`.
    fn memo_up(&mut self, d: &Document, from: Ref, g: Ref) {
        let mut c = from;
        while c.is_some() && c != g {
            self.memo.remove(&c);
            match d.dom.get_by_ref(c) {
                Some(i) => c = i.parent(),
                None => break,
            }
        }
    }

    /// The outside state cached GUIs were drawn with changed: drop those it
    /// touches (hover / press / focus / selection), or all (font atlas).
    fn cache_check(&mut self, ctx: &egui::Context, d: &Document) {
        let ppp = ctx.pixels_per_point();
        let (atlas, fill, fonts) = ctx.fonts(|f| {
            let defs = f.definitions();
            (f.font_image_size(), f.font_atlas_fill_ratio(), (defs.font_data.len(), defs.families.len()))
        });
        let selected = d
            .find_child(d.dom.root_ref(), "GuiService")
            .and_then(|gs| match d.prop(gs, "SelectedObject") {
                Some(Variant::Ref(r)) if d.exists(r) => Some(r),
                _ => None,
            });
        let now = CacheWith {
            hovered: self.hovered.clone(),
            pressed: self.pressed.map(|p| p.0),
            focus: self.focus,
            selected,
            atlas,
            fill,
            fonts,
            ppp,
        };
        let was = std::mem::replace(&mut self.cache_with, now);
        // Memos of objects that are gone (now and then).
        self.memo_frames += 1;
        if self.memo_frames % 600 == 0 {
            self.memo.retain(|r, _| d.exists(*r));
        }
        // (egui's atlas grew, or started over: glyph UVs moved)
        if atlas != was.atlas || fill < was.fill || fonts != was.fonts || ppp != was.ppp {
            self.memo.clear();
            self.cache.clear();
            return;
        }
        let now = &self.cache_with;
        let touched: Vec<Ref> = now
            .hovered
            .symmetric_difference(&was.hovered)
            .copied()
            .chain([now.pressed, was.pressed].into_iter().flatten().filter(|_| now.pressed != was.pressed))
            .chain([now.focus, was.focus].into_iter().flatten().filter(|_| now.focus != was.focus))
            .chain([now.selected, was.selected].into_iter().flatten().filter(|_| now.selected != was.selected))
            .collect();
        for r in touched {
            if let Some((g, _, _)) = self.cache_owner.get(&r).copied() {
                self.cache.remove(&g);
                self.memo_up(d, r, g);
            }
        }
    }

    /// ViewportFrames draw again only when what they show changed: a mark on
    /// the frame or anything under it, on its camera, or a part it drew
    /// being taken away.
    fn vf_dirty(&mut self, d: &Document, marks: &doc::Marks) {
        if self.vf_targets.is_empty() {
            return;
        }
        let marks = match marks {
            doc::Marks::Some(m) if d.id == self.roots_doc => m,
            _ => {
                for t in self.vf_targets.values_mut() {
                    t.dirty = true;
                }
                return;
            }
        };
        for m in marks {
            let (r, tree) = match *m {
                doc::Mark::Prop(r, _) => (r, false),
                doc::Mark::Tree(r) => (r, true),
            };
            let mut cur = r;
            while let Some(i) = d.dom.get_by_ref(cur) {
                if let Some(t) = self.vf_targets.get_mut(&cur) {
                    t.dirty = true;
                    break;
                }
                cur = i.parent();
            }
            for t in self.vf_targets.values_mut() {
                if !t.dirty && t.cam == Some(r) {
                    t.dirty = true;
                }
            }
            if tree && self.vf_targets.values().any(|t| !t.dirty && !t.parts.is_empty()) {
                for x in d.subtree(r) {
                    for t in self.vf_targets.values_mut() {
                        if t.parts.contains(&x) {
                            t.dirty = true;
                        }
                    }
                }
            }
        }
    }

    /// Draws every GUI in view and remembers what can be clicked.
    pub fn draw(&mut self, ctx: &egui::Context, painter: &egui::Painter, mut inp: DrawIn<'_>) {
        if let Some(rs) = inp.render_state {
            self.vf_targets.retain(|r, target| {
                let keep = inp.doc.exists(*r) && target.used.elapsed().as_secs() < 30;
                if !keep { rs.renderer.write().free_texture(&target.id); }
                keep
            });
        }
        self.hits.clear();
        self.world_batches.clear();
        self.cache_check(ctx, inp.doc);
        self.fx_frame(inp.doc);
        if inp.playing {
            let _a = crate::bench::scope("gui_adorn");
            self.draw_adornments(inp.doc, inp.cam, inp.rect, painter);
        }
        let pre = crate::bench::scope("gui_pre");
        let d = inp.doc;
        let root = d.dom.root_ref();
        let rect = inp.rect;
        let mut roots: Vec<Root> = Vec::new();

        // ScreenGuis: the LocalPlayer's PlayerGui while playing; StarterGui
        // (ShowDevelopmentGui) while editing, as Studio shows them.
        let screen_parent = if inp.playing {
            d.find_child(root, "Players")
                .and_then(|p| match d.prop(p, "LocalPlayer") {
                    Some(Variant::Ref(lp)) if d.exists(lp) => d.find_child(lp, "PlayerGui"),
                    _ => None,
                })
        } else {
            d.find_child(root, "StarterGui")
                .filter(|sg| boolp(d, *sg, "ShowDevelopmentGui", true))
        };
        let mut layered: Vec<Ref> = Vec::new();
        if let Some(pg) = screen_parent {
            for g in children(d, pg) {
                let Some(class) = d.class_of(g) else { continue };
                if !boolp(d, g, "Enabled", true) {
                    continue;
                }
                match class.as_str() {
                    "ScreenGui" => {
                        // IgnoreGuiInset, or ScreenInsets None / DeviceSafeInsets: the whole view.
                        let ignore = boolp(d, g, "IgnoreGuiInset", false)
                            || matches!(enum_u32(d.prop(g, "ScreenInsets")), Some(0) | Some(1));
                        let inset = if ignore { 0.0 } else { mech::GUI_TOP_INSET };
                        roots.push(Root {
                            on_top: true,
                            light: 1.0,
                            order: 1_000_000 + num(d, g, "DisplayOrder", 0.0) as i64,
                            dist: 0.0,
                            gui: g,
                            target: Target::Screen(Pos2::new(rect.left(), rect.top() + inset)),
                            canvas: [rect.width(), rect.height() - inset],
                            abs_offset: [0.0, inset - mech::GUI_TOP_INSET],
                        });
                        // BillboardGuis / SurfaceGuis kept in a ScreenGui (the
                        // proximity prompts' ProximityPrompts gui) draw at
                        // their Adornee.
                        for c in children(d, g) {
                            if matches!(d.class_of(c).as_deref(), Some("BillboardGui" | "SurfaceGui"))
                                && boolp(d, c, "Enabled", true)
                                && matches!(d.prop(c, "Adornee"), Some(Variant::Ref(a)) if d.exists(a))
                            {
                                layered.push(c);
                            }
                        }
                    }
                    "SurfaceGui" | "BillboardGui" => layered.push(g),
                    _ => {}
                }
            }
        }
        layered.extend(
            self.world_roots
                .iter()
                .copied()
                .filter(|g| d.exists(*g) && boolp(d, *g, "Enabled", true)),
        );
        drop(pre);
        let place = crate::bench::scope("gui_place");
        for g in layered {
            if let Some((target, canvas, dist)) = place_3d(d, inp.cam, rect, g) {
                let on_top = boolp(d, g, "AlwaysOnTop", false);
                let normal = match target {
                    Target::Surface { tl, tr, bl } => (bl - tl).cross(tr - tl).normalize_or_zero(),
                    _ => -inp.cam.forward(),
                };
                roots.push(Root {
                    on_top,
                    light: world_gui_light(d, g, normal),
                    order: if on_top { 500_000 } else { 0 } + num(d, g, "ZOffset", 0.0) as i64,
                    dist,
                    gui: g,
                    target,
                    canvas,
                    abs_offset: [0.0, 0.0],
                });
            }
        }
        // Back to front: 3D GUIs (far first, AlwaysOnTop after), then ScreenGuis by DisplayOrder.
        roots.sort_by(|a, b| a.order.cmp(&b.order).then(b.dist.total_cmp(&a.dist)));
        drop(place);

        // (No prepared discs: they only speed up small circles.)
        let atlas = ctx.fonts(|f| f.font_image_size());
        let view_proj = inp.cam.view_proj(rect);
        // The renderer's reverse-Z projection: the depth the world GUI pass tests.
        let view_proj_rz = inp.cam.view_proj_reverse_z(rect);
        let mut tess = Tessellator::new(
            ctx.pixels_per_point(),
            TessellationOptions::default(),
            atlas,
            Vec::new(),
        );
        let mut out: Vec<Shape> = Vec::new();
        let mut drawn_roots: Vec<Ref> = Vec::with_capacity(roots.len());
        // (RUSTUDIO_GUI_CACHE_WHY: say why each GUI was walked)
        let why_on = debug_env(DebugEnv::Why);
        for root in roots {
            let g = root.gui;
            let class = d.class_of(g).map(|c| c.to_string()).unwrap_or_default();
            // ZIndexBehavior: Global 0, Sibling 1.
            let global_z = enum_u32(d.prop(g, "ZIndexBehavior")).unwrap_or(0) == 0;
            let canvas_rect =
                Rect::from_min_size(Pos2::ZERO, egui::vec2(root.canvas[0], root.canvas[1]));
            let clip =
                if class == "ScreenGui" || boolp(d, g, "ClipsDescendants", class == "SurfaceGui") {
                    canvas_rect
                } else {
                    Rect::EVERYTHING
                };
            let root_box = GuiBox {
                pos: root.abs_offset,
                size: root.canvas,
                rot: 0.0,
                text: None,
                canvas: None,
            };
            if self.layout.insert(g, root_box) != Some(root_box) {
                self.layout_changed = true;
            }
            drawn_roots.push(g);
            // A clipped SurfaceGui that can't colour a pixel of the view
            // (off to the side, or too far to cover a pixel centre) isn't
            // walked: scripts keep reading its last layout (its canvas
            // doesn't depend on the camera).
            if let Target::Surface { tl, tr, bl } = root.target {
                if clip != Rect::EVERYTHING && !surface_in_view(&view_proj, rect, ctx.pixels_per_point(), tl, tr, bl) {
                    // (its entries from its last walk stay in `layout`)
                    crate::bench::add("gui_unseen", 0.0, 1);
                    continue;
                }
            }
            // A ScreenGui nothing touched since it was walked is drawn from
            // what that walk made.
            // A GUI nothing touched since it was walked is drawn from what
            // that walk made: a ScreenGui's screen triangles as they are, a
            // SurfaceGui's / BillboardGui's canvas triangles projected again.
            let screen_root = matches!(root.target, Target::Screen(_));
            let key = {
                let (ox, oy, kind) = match root.target {
                    Target::Screen(o) => (o.x, o.y, 0.0),
                    Target::Surface { .. } => (0.0, 0.0, 1.0),
                    Target::Billboard(..) => (0.0, 0.0, 2.0),
                };
                [
                    ox, oy, root.canvas[0], root.canvas[1], root.abs_offset[1],
                    clip.min.x, clip.min.y, clip.max.x, clip.max.y, ctx.pixels_per_point(),
                    global_z as u32 as f32, kind,
                ]
                .map(f32::to_bits)
            };
            // (RUSTUDIO_GUI_CACHE_VERIFY: walk it anyway and compare)
            let verify = debug_env(DebugEnv::Verify);
            let mut kept: Option<RootCache> = None;
            let mut reuse: Option<RootCache> = None;
            if let Some(c) = self.cache.get(&g).filter(|c| c.key == key) {
                let vfs_ok = c.vfs.iter().all(|v| self.vf_targets.get(v).is_some_and(|t| !t.dirty));
                if vfs_ok {
                    let now = std::time::Instant::now();
                    for v in &c.vfs {
                        if let Some(t) = self.vf_targets.get_mut(v) {
                            t.used = now;
                        }
                    }
                }
                if vfs_ok && verify {
                    kept = self.cache.remove(&g);
                } else if vfs_ok && screen_root {
                    let c = self.cache.get(&g).unwrap();
                    out.extend(c.shapes.iter().cloned());
                    self.hits.extend(c.hits.iter().cloned());
                    crate::runtime::domains::services::perf::draw(crate::runtime::domains::services::perf::UI2D, c.ui2d.0, c.ui2d.1);
                    crate::bench::add("gui_cached", 0.0, 1);
                    continue;
                } else if vfs_ok {
                    reuse = self.cache.remove(&g);
                    crate::bench::add("gui_cached", 0.0, 1);
                }
            }
            let (target, canvas) = (root.target, root.canvas);
            // World GUIs go to the renderer's pass (depth-tested, lit and
            // tone-mapped with the scene, as Studio's); AlwaysOnTop ones and
            // ScreenGuis to egui over the view (Studio draws AlwaysOnTop
            // colours as they are: tools/fidelity gui-depth).
            let world = !matches!(target, Target::Screen(_)) && !root.on_top;
            let light = root.light;
            // A canvas point's reverse-Z depth (SurfaceGui: its world point;
            // BillboardGui: its anchor's).
            // (and its distance from the camera, for fog).
            let depth_of = |p: Pos2| -> (f32, f32) {
                let w = match target {
                    Target::Surface { tl, tr, bl } => {
                        let (u, v) = (p.x / canvas[0].max(1.0), p.y / canvas[1].max(1.0));
                        tl + (tr - tl) * u + (bl - tl) * v
                    }
                    Target::Billboard(_, at) => at,
                    Target::Screen(_) => return (1.0, 0.0),
                };
                let c = view_proj_rz * w.extend(1.0);
                let dist = (w - inp.cam.pos).length();
                (if c.w <= 1e-6 { 1.0 } else { c.z / c.w }, dist)
            };
            // A world point's screen position and clip-space w.
            let project_w = |world: Vec3| -> Option<(Pos2, f32)> {
                let clip = view_proj * world.extend(1.0);
                if clip.w <= 1e-4 {
                    return None;
                }
                let ndc = clip.truncate() / clip.w;
                Some((
                    Pos2::new(
                        rect.min.x + (ndc.x + 1.0) * 0.5 * rect.width(),
                        rect.min.y + (1.0 - ndc.y) * 0.5 * rect.height(),
                    ),
                    clip.w,
                ))
            };
            let project = |world: Vec3| project_w(world).map(|(p, _)| p);
            // SurfaceGuis: a canvas point's screen position and w.
            let surface = |p: Pos2| -> Option<(Pos2, f32)> {
                let Target::Surface { tl, tr, bl } = target else {
                    return None;
                };
                let (u, v) = (p.x / canvas[0].max(1.0), p.y / canvas[1].max(1.0));
                project_w(tl + (tr - tl) * u + (bl - tl) * v)
            };
            let map = |p: Pos2| -> Option<Pos2> {
                match target {
                    Target::Screen(o) => Some(o + p.to_vec2()),
                    Target::Billboard(c, _) => Some(Pos2::new(
                        c.x - canvas[0] * 0.5 + p.x,
                        c.y - canvas[1] * 0.5 + p.y,
                    )),
                    Target::Surface { tl, tr, bl } => {
                        let (u, v) = (p.x / canvas[0].max(1.0), p.y / canvas[1].max(1.0));
                        project(tl + (tr - tl) * u + (bl - tl) * v)
                    }
                }
            };
            let perspective = matches!(target, Target::Surface { .. });
            let ppp = ctx.pixels_per_point();
            // Whether an item can colour any pixel of the view, in two
            // halves: its bounds (turned, clipped) in the canvas, kept with
            // its triangles (None: nothing to draw)...
            let canvas_bounds = |it: &walk::Item| -> Option<Rect> {
                // (+ the tessellator's feathering, 1 / ppp canvas units)
                let b = it.shape.visual_bounding_rect();
                if !b.is_positive() {
                    return None;
                }
                let b = b.expand(1.0 / ppp);
                let c = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()]
                    .map(|p| it.aff.apply(p));
                let mut b = Rect::from_points(&c);
                if it.clip != Rect::EVERYTHING {
                    b = b.intersect(it.clip);
                    if !b.is_positive() {
                        return None;
                    }
                }
                Some(b)
            };
            // ... and, each frame, those bounds on screen must overlap the
            // view and hold a pixel centre (the GUI pass has no MSAA, so a
            // triangle with none draws nothing). Unknown (behind the
            // camera) counts as yes.
            let on_screen = |b: Rect| -> bool {
                let mut s = Rect::NOTHING;
                for p in [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()] {
                    match map(p) {
                        Some(q) => s.extend_with(q),
                        None => return true,
                    }
                }
                let s = s.intersect(rect);
                // A pixel centre (i + 0.5) / ppp inside on both axes.
                let holds = |lo: f32, hi: f32| (hi * ppp - 0.5).floor() >= (lo * ppp - 0.5).ceil();
                s.is_positive() && holds(s.min.x, s.max.x) && holds(s.min.y, s.max.y)
            };
            let (out_from, hits_from) = (out.len(), self.hits.len());
            let (mut t_tess, mut t_clip, mut t_split, mut t_map, mut n_in, mut n_out) =
                (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0u64, 0u64);
            // The walk and the canvas triangles (or the kept ones).
            let (citems, lhits) = match reuse.as_mut() {
                Some(c) => (std::mem::take(&mut c.citems), std::mem::take(&mut c.lhits)),
                None => {
                    self.cache.remove(&g);
                    self.no_cache = None;
                    self.walk_vfs.clear();
                    self.walk_reads.clear();
                    self.walk_kids.clear();
                    // Its old entries make way for what this walk writes.
                    for r in self.layout_from.remove(&g).unwrap_or_default() {
                        if r != g {
                            self.layout.remove(&r);
                        }
                    }
                    let walk_scope = crate::bench::scope("gui_walk");
                    let mut w = Walk {
                        gui: self,
                        ctx,
                        input: &mut inp,
                        items: Vec::new(),
                        hits: Vec::new(),
                        seq: 0,
                        boxes: Vec::new(),
                        global_z,
                        abs_offset: root.abs_offset,
                        screen: match root.target {
                            Target::Screen(p) => Some(p),
                            _ => None,
                        },
                        cur: Ref::none(),
                        rec: Vec::new(),
                        replaying: 0,
                    };
                    w.children(
                        g,
                        canvas_rect,
                        Down {
                            aff: Aff::ID,
                            clip,
                            alpha: 1.0,
                            tint: [1.0, 1.0, 1.0],
                            scale: 1.0,
                            z: 0,
                        },
                    );
                    drop(walk_scope);
                    let _mesh_scope = crate::bench::scope("gui_mesh");
                    let mut items = std::mem::take(&mut w.items);
                    let hits = std::mem::take(&mut w.hits);
                    let boxes = std::mem::take(&mut w.boxes);
                    if verify {
                        // The same walk without memos must give the same.
                        let saved = (self.no_cache, self.walk_reads.len(), self.walk_kids.len(), self.walk_vfs.len());
                        self.memo_off = true;
                        let mut w = Walk {
                            gui: self,
                            ctx,
                            input: &mut inp,
                            items: Vec::new(),
                            hits: Vec::new(),
                            seq: 0,
                            boxes: Vec::new(),
                            global_z,
                            abs_offset: root.abs_offset,
                            screen: match root.target {
                                Target::Screen(p) => Some(p),
                                _ => None,
                            },
                            cur: Ref::none(),
                            rec: Vec::new(),
                            replaying: 0,
                        };
                        w.children(g, canvas_rect, Down { aff: Aff::ID, clip, alpha: 1.0, tint: [1.0, 1.0, 1.0], scale: 1.0, z: 0 });
                        let (items2, hits2, boxes2) = (std::mem::take(&mut w.items), std::mem::take(&mut w.hits), std::mem::take(&mut w.boxes));
                        self.memo_off = false;
                        self.no_cache = saved.0;
                        self.walk_reads.truncate(saved.1);
                        self.walk_kids.truncate(saved.2);
                        self.walk_vfs.truncate(saved.3);
                        let tess_of = |tess: &mut Tessellator, it: &walk::Item| {
                            let mut m = Mesh::default();
                            tess.tessellate_shape(it.shape.clone(), &mut m);
                            (m.vertices.iter().map(|v| (v.pos, v.uv, v.color)).collect::<Vec<_>>(), m.indices, m.texture_id)
                        };
                        let mut bad: Option<String> = None;
                        if items.len() != items2.len() {
                            bad = Some(format!("{} / {} items", items.len(), items2.len()));
                        } else {
                            for (i, (a, b)) in items.iter().zip(&items2).enumerate() {
                                if a.z != b.z || a.seq != b.seq || a.clip != b.clip || !a.aff.same(&b.aff) || tess_of(&mut tess, a) != tess_of(&mut tess, b) {
                                    let path = |r: Ref| {
                                        let mut n = Vec::new();
                                        let mut c = r;
                                        while let Some(i) = d.dom.get_by_ref(c) {
                                            if c == g || n.len() > 8 {
                                                break;
                                            }
                                            n.push(i.name.clone());
                                            c = i.parent();
                                        }
                                        n.reverse();
                                        n.join(".")
                                    };
                                    let (ta, tb) = (tess_of(&mut tess, a), tess_of(&mut tess, b));
                                    bad = Some(format!(
                                        "item {i} by {} / {}: z {} {} seq {} {} clip {:?} {:?} verts {:?} / {:?}",
                                        path(a.by), path(b.by), a.z, b.z, a.seq, b.seq, a.clip, b.clip,
                                        &ta.0[..ta.0.len().min(4)], &tb.0[..tb.0.len().min(4)]
                                    ));
                                    break;
                                }
                            }
                        }
                        let same_hit = |a: &walk::LocalHit, b: &walk::LocalHit| a.r == b.r && a.rect == b.rect && a.clip == b.clip && a.sink == b.sink && a.button == b.button && a.painted == b.painted && a.aff.same(&b.aff);
                        if bad.is_none() && (hits.len() != hits2.len() || !hits.iter().zip(&hits2).all(|(a, b)| same_hit(a, b))) {
                            bad = Some(format!("hits {} / {}", hits.len(), hits2.len()));
                        }
                        if bad.is_none() && boxes != boxes2 {
                            bad = Some(format!("boxes {} / {}", boxes.len(), boxes2.len()));
                        }
                        crate::bench::add(if bad.is_none() { "gui_verify_memo_same" } else { "gui_verify_memo_differ" }, 0.0, 1);
                        if let Some(b) = bad {
                            eprintln!("[guiverify] {}: memo walk differs from a full walk: {b}", name_of(d, g));
                        }
                    }
                    self.layout_from.insert(g, boxes.iter().map(|b| b.0).collect());
                    if self.root_boxes.get(&g) != Some(&boxes) {
                        self.layout_changed = true;
                    }
                    self.root_boxes.insert(g, boxes);
                    // (what it read: for the marks that drop kept output)
                    self.cache_own(g);
                    if global_z {
                        items.sort_by_key(|i| (i.z, i.seq));
                    }
                    let mut citems: Vec<(Rect, Mesh)> = Vec::with_capacity(items.len());
                    for it in &items {
                        let t0 = std::time::Instant::now();
                        let kept = it.tess.get_or_init(|| {
                            let b = canvas_bounds(it)?;
                            let mut mesh = Mesh::default();
                            tess.tessellate_shape(it.shape.clone(), &mut mesh);
                            if mesh.is_empty() {
                                return None;
                            }
                            if !it.aff.is_id() {
                                for v in &mut mesh.vertices {
                                    v.pos = it.aff.apply(v.pos);
                                }
                            }
                            if it.clip != Rect::EVERYTHING {
                                mesh = clip_mesh(&mesh, it.clip);
                            }
                            Some((b, mesh))
                        });
                        t_tess += t0.elapsed().as_secs_f64();
                        let Some((b, mesh)) = kept else { continue };
                        // (a ScreenGui's view test needs no camera: done now)
                        if screen_root && !on_screen(*b) {
                            continue;
                        }
                        citems.push((*b, mesh.clone()));
                    }
                    (citems, hits)
                }
            };
            let mut ui2d = (0u64, 0u64);
            let mut ui2d_tex: Option<egui::TextureId> = None;
            for (b, mesh) in &citems {
                if !screen_root && !on_screen(*b) {
                    continue;
                }
                let t2 = std::time::Instant::now();
                let mut mesh = mesh.clone();
                n_in += mesh.indices.len() as u64 / 3;
                if perspective {
                    mesh = subdivide(mesh, mech::GUI_SURFACE_AFFINE_ERR_PX, rect, &surface);
                }
                n_out += mesh.indices.len() as u64 / 3;
                let t3 = std::time::Instant::now();
                t_split += (t3 - t2).as_secs_f64();
                let mut ok = !mesh.is_empty();
                let mut depths = Vec::new();
                for v in &mut mesh.vertices {
                    if world {
                        depths.push(depth_of(v.pos));
                    }
                    match map(v.pos) {
                        Some(p) => v.pos = p,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                t_map += t3.elapsed().as_secs_f64();
                if ok && world {
                    // Into the renderer's pass (NDC over the view), in order.
                    let verts: Vec<crate::world_gui::GuiVertex> = mesh
                        .vertices
                        .iter()
                        .zip(&depths)
                        .map(|(v, d)| crate::world_gui::GuiVertex {
                            ndc: [
                                (v.pos.x - rect.min.x) / rect.width().max(1.0) * 2.0 - 1.0,
                                1.0 - (v.pos.y - rect.min.y) / rect.height().max(1.0) * 2.0,
                            ],
                            depth: d.0,
                            uv: [v.uv.x, v.uv.y],
                            color: v.color.to_array(),
                            light: [light; 3],
                            dist: d.1,
                            clip_w: 0.0,
                            alpha_mul: 1.0,
                            ..Default::default()
                        })
                        .collect();
                    crate::world_gui::push(&mut self.world_batches, mesh.texture_id, verts, &mesh.indices);
                } else if ok {
                    // Stats.UI2D*: a draw call per texture change (egui batches the rest).
                    let new_tex = ui2d_tex.replace(mesh.texture_id) != Some(mesh.texture_id);
                    crate::runtime::domains::services::perf::draw(crate::runtime::domains::services::perf::UI2D, new_tex as u64, (mesh.indices.len() / 3) as u64);
                    ui2d.0 += new_tex as u64;
                    ui2d.1 += (mesh.indices.len() / 3) as u64;
                    out.push(Shape::Mesh(mesh.into()));
                }
            }
            crate::bench::add("gui_m_tess", t_tess * 1e3, 1);
            crate::bench::add("gui_m_clip", t_clip * 1e3, 1);
            crate::bench::add("gui_m_split", t_split * 1e3, 1);
            crate::bench::add("gui_m_map", t_map * 1e3, 1);
            crate::bench::add(if perspective { "gui_tris_surface_in" } else { "gui_tris_flat" }, 0.0, n_in);
            crate::bench::add("gui_tris_out", 0.0, n_out);
            for h in &lhits {
                let corners = [
                    h.rect.left_top(),
                    h.rect.right_top(),
                    h.rect.right_bottom(),
                    h.rect.left_bottom(),
                ]
                .map(|p| h.aff.apply(p));
                let q: Option<Vec<Pos2>> = corners.iter().map(|p| map(*p)).collect();
                let Some(q) = q else { continue };
                let clip = h.clip.and_then(|c| {
                    let cq: Option<Vec<Pos2>> = [
                        c.left_top(),
                        c.right_top(),
                        c.right_bottom(),
                        c.left_bottom(),
                    ]
                    .iter()
                    .map(|p| map(*p))
                    .collect();
                    cq.map(|v| [v[0], v[1], v[2], v[3]])
                });
                self.hits.push(Hit {
                    r: h.r,
                    quad: [q[0], q[1], q[2], q[3]],
                    clip,
                    button: h.button,
                    textbox: h.textbox,
                    sink: h.sink,
                    scrolling: h.scrolling,
                    drag: h.drag,
                    painted: h.painted,
                });
            }
            if let Some(mut c) = reuse {
                // (drawn from what was kept: keep it)
                c.citems = citems;
                c.lhits = lhits;
                self.cache.insert(g, c);
                continue;
            }
            if let Some(k) = kept.take() {
                self.cache_verify(d, g, &k, screen_root, &out[out_from..], &self.hits[hits_from..], &citems, &lhits);
            }
            if self.no_cache.is_none() {
                let (shapes, hits) = if screen_root {
                    (out[out_from..].to_vec(), self.hits[hits_from..].to_vec())
                } else {
                    (Vec::new(), Vec::new())
                };
                let (citems, lhits) = if screen_root { (Vec::new(), Vec::new()) } else { (citems, lhits) };
                self.cache.insert(
                    g,
                    RootCache {
                        key,
                        shapes,
                        hits,
                        ui2d,
                        vfs: std::mem::take(&mut self.walk_vfs),
                        citems,
                        lhits,
                    },
                );
            } else {
                crate::bench::add("gui_uncached", 0.0, 1);
            }
            if why_on {
                eprintln!(
                    "[guiwhy] {} walked: {}",
                    name_of(d, g),
                    match (self.no_cache, self.cache_why.remove(&g)) {
                        (Some(v), _) => format!("kept out: {v}"),
                        (_, Some(w)) => w,
                        _ => "new or view changed".to_string(),
                    }
                );
            }
        }
        // GUIs drawn last frame and not now: their entries go.
        let was: HashSet<Ref> = std::mem::take(&mut self.layout_roots).into_iter().collect();
        let now: HashSet<Ref> = drawn_roots.iter().copied().collect();
        for g in was.difference(&now) {
            for r in self.layout_from.remove(g).unwrap_or_default() {
                self.layout.remove(&r);
            }
            self.layout.remove(g);
            self.layout_changed = true;
        }
        if debug_env(DebugEnv::Verify) || debug_env(DebugEnv::LayoutVerify) {
            // The layout as a fresh frame would write it.
            let mut fresh: HashMap<Ref, GuiBox> = HashMap::new();
            for g in &drawn_roots {
                for (r, b) in self.root_boxes.get(g).into_iter().flatten() {
                    fresh.insert(*r, *b);
                }
            }
            for g in &drawn_roots {
                if let Some(b) = self.layout.get(g) {
                    fresh.insert(*g, *b);
                }
            }
            let same = fresh == self.layout;
            crate::bench::add(if same { "gui_verify_layout_same" } else { "gui_verify_layout_differ" }, 0.0, 1);
            if !same {
                eprintln!("[guiverify] layout: {} kept, {} fresh", self.layout.len(), fresh.len());
            }
        }
        self.layout_roots = drawn_roots;
        crate::bench::add("gui_memo_hit", 0.0, std::mem::take(&mut self.memo_hits));
        crate::bench::add("gui_memo_walk", 0.0, std::mem::take(&mut self.memo_walks));
        self.drawn = out.len();
        let _paint = crate::bench::scope("gui_paint");
        let clipped = painter.with_clip_rect(rect);
        // (RUSTUDIO_GUI_BLACK_BG: ScreenGuis over black, to compare GUI
        // captures without the 3D view's frame-to-frame differences)
        if debug_env(DebugEnv::BlackBg) {
            clipped.rect_filled(rect, 0.0, Color32::BLACK);
        }
        for s in out {
            clipped.add(s);
        }
    }

    /// Pointer over the viewport: hover, clicks and the wheel. Queues events;
    /// returns true when the GUI took the input (no 3D picking or clicks).
    pub fn input(&mut self, ctx: &egui::Context, resp: &egui::Response, rect: Rect) -> bool {
        let pointer = resp.hover_pos().filter(|p| rect.contains(*p));
        let under: Vec<Hit> = match pointer {
            Some(p) => self
                .hits
                .iter()
                .filter(|h| {
                    point_in_quad(&h.quad, p) && h.clip.is_none_or(|c| point_in_quad(&c, p))
                })
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        let top = under.iter().rev().find(|h| h.sink).cloned();
        // GUI coordinates: from the view's top-left, below the top bar inset (as InputObject.Position).
        let gui_xy = |p: Pos2| (p.x - rect.left(), p.y - rect.top() - mech::GUI_TOP_INSET);
        let now: HashSet<Ref> = under.iter().map(|h| h.r).collect();
        let moved = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
        if let Some(p) = pointer {
            let (x, y) = gui_xy(p);
            for h in &under {
                if !self.hovered.contains(&h.r) {
                    self.events.push(GuiEvent::Fire(
                        h.r,
                        "MouseEnter",
                        vec![GuiArg::Num(x as f64), GuiArg::Num(y as f64)],
                    ));
                } else if moved {
                    self.events.push(GuiEvent::Fire(
                        h.r,
                        "MouseMoved",
                        vec![GuiArg::Num(x as f64), GuiArg::Num(y as f64)],
                    ));
                    self.events.push(GuiEvent::Fire(
                        h.r,
                        "InputChanged",
                        vec![GuiArg::Input { button: 0, began: false, x, y }],
                    ));
                }
            }
        }
        let (lx, ly) = pointer.map(gui_xy).unwrap_or((0.0, 0.0));
        if moved {
            if let (Some(d), Some(p)) = (self.dragging, pointer) {
                let (x, y) = gui_xy(p);
                super::share().drags.push((d, 1, x, y));
            }
        }
        for r in self.hovered.difference(&now) {
            self.events.push(GuiEvent::Fire(
                *r,
                "MouseLeave",
                vec![GuiArg::Num(lx as f64), GuiArg::Num(ly as f64)],
            ));
        }
        self.hovered = now;
        {
            let mut sh = super::share();
            sh.hovered = self.hovered.clone();
            sh.buttons = ctx.input(|i| {
                [
                    i.pointer.primary_down(),
                    i.pointer.secondary_down(),
                    i.pointer.middle_down(),
                ]
            });
        }

        let mut took = top.is_some() && (resp.clicked() || resp.secondary_clicked());
        let presses: Vec<(egui::PointerButton, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::PointerButton {
                        button, pressed, ..
                    } => Some((*button, *pressed)),
                    _ => None,
                })
                .collect()
        });
        for (button, pressed) in presses {
            let b = match button {
                egui::PointerButton::Primary => 1u8,
                egui::PointerButton::Secondary => 2,
                _ => continue,
            };
            let Some(p) = pointer else { continue };
            {
                let mut sh = super::share();
                sh.mouse_edges.push((b, pressed, top.is_some()));
                if sh.mouse_edges.len() > 64 {
                    sh.mouse_edges.remove(0);
                }
            }
            let (x, y) = gui_xy(p);
            let input = GuiArg::Input {
                button: b,
                began: pressed,
                x,
                y,
            };
            if pressed {
                // Clicking anywhere else ends text editing.
                if let Some(f) = self
                    .focus
                    .filter(|f| top.as_ref().is_none_or(|t| t.r != *f))
                {
                    // The scripts fire FocusLost (src/domains/gui_input.rs).
                    let mut sh = super::share();
                    sh.focus_events.push((f, false, false, Some((b, x, y))));
                    sh.focus = None;
                    self.focus = None;
                }
                for h in &under {
                    self.events
                        .push(GuiEvent::Fire(h.r, "InputBegan", vec![input.clone()]));
                }
                // A left press starts dragging the top-most draggable object
                // that nothing in front of it took.
                if b == 1 {
                    let top_i = under.iter().rposition(|h| h.sink);
                    let drag = under
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(i, h)| h.drag && top_i.is_none_or(|t| *i >= t))
                        .map(|(_, h)| h.r);
                    if let Some(d) = drag {
                        self.dragging = Some(d);
                        super::share().drags.push((d, 0, x, y));
                        took = true;
                    }
                }
                if let Some(t) = &top {
                    took = true;
                    self.pressed = Some((t.r, b));
                    super::share().pressed = Some(t.r);
                    if t.button {
                        self.events.push(GuiEvent::Fire(
                            t.r,
                            if b == 1 {
                                "MouseButton1Down"
                            } else {
                                "MouseButton2Down"
                            },
                            vec![GuiArg::Num(x as f64), GuiArg::Num(y as f64)],
                        ));
                    }
                    if t.textbox && b == 1 && self.focus != Some(t.r) {
                        self.focus = Some(t.r);
                        let mut sh = super::share();
                        sh.focus_events.push((t.r, true, false, Some((b, x, y))));
                        sh.focus = Some(t.r);
                    }
                }
            } else {
                if b == 1 {
                    if let Some(d) = self.dragging.take() {
                        super::share().drags.push((d, 2, x, y));
                    }
                }
                for h in &under {
                    self.events
                        .push(GuiEvent::Fire(h.r, "InputEnded", vec![input.clone()]));
                }
                if let Some((pr, pb)) = self.pressed.take() {
                    super::share().pressed = None;
                    if pb != b {
                        continue;
                    }
                    took = true;
                    if self.hits.iter().any(|h| h.r == pr && h.button) {
                        self.events.push(GuiEvent::Fire(
                            pr,
                            if b == 1 {
                                "MouseButton1Up"
                            } else {
                                "MouseButton2Up"
                            },
                            vec![GuiArg::Num(x as f64), GuiArg::Num(y as f64)],
                        ));
                        if top.as_ref().is_some_and(|t| t.r == pr) {
                            self.events.push(GuiEvent::Fire(
                                pr,
                                if b == 1 {
                                    "MouseButton1Click"
                                } else {
                                    "MouseButton2Click"
                                },
                                vec![],
                            ));
                            if b == 1 {
                                self.events.push(GuiEvent::Fire(
                                    pr,
                                    "Activated",
                                    vec![input.clone(), GuiArg::Num(1.0)],
                                ));
                            }
                            self.last_click = Some(pr);
                        }
                    }
                }
            }
        }
        // The wheel over a ScrollingFrame scrolls it: ScrollWheelDeltaAmount
        // (100) pixels per notch.
        let wheel: f32 = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(
                        mech::GUI_SCROLL_PX_PER_NOTCH
                            * match unit {
                                egui::MouseWheelUnit::Line => delta.y,
                                egui::MouseWheelUnit::Point => {
                                    delta.y / mech::WHEEL_POINTS_PER_NOTCH
                                }
                                egui::MouseWheelUnit::Page => delta.y * 3.0,
                            },
                    ),
                    _ => None,
                })
                .sum()
        });
        if wheel != 0.0 && resp.hovered() {
            // Every object under the pointer hears the notch (MouseWheelForward
            // / Backward and InputChanged with a MouseWheel InputObject).
            if let Some(p) = pointer {
                let (x, y) = gui_xy(p);
                let fwd = wheel > 0.0;
                for h in &under {
                    self.events.push(GuiEvent::Fire(
                        h.r,
                        if fwd { "MouseWheelForward" } else { "MouseWheelBackward" },
                        vec![GuiArg::Num(x as f64), GuiArg::Num(y as f64)],
                    ));
                    self.events.push(GuiEvent::Fire(
                        h.r,
                        "InputChanged",
                        vec![GuiArg::Input { button: if fwd { 3 } else { 4 }, began: false, x, y }],
                    ));
                }
            }
            if let Some(sf) = under.iter().rev().find(|h| h.scrolling) {
                self.events.push(GuiEvent::Scroll(sf.r, [0.0, -wheel]));
                took = true;
            }
        }
        took
    }

    /// Typing into the focused TextBox; Enter / Escape end it. True while a
    /// TextBox has the keyboard.
    pub fn keyboard(&mut self, ctx: &egui::Context, doc: &Document) -> bool {
        // Scripts' TextBox:CaptureFocus / ReleaseFocus (they fired the events).
        for req in std::mem::take(&mut super::share().focus_req) {
            self.focus = req;
        }
        let Some(f) = self.focus else { return false };
        if !doc.exists(f) {
            self.focus = None;
            super::share().focus = None;
            return false;
        }
        let mut text = text_prop(doc, f, "Text");
        let before = text.clone();
        let mut done: Option<bool> = None;
        // (The raw events: nothing else in the editor may have used them up.)
        ctx.input(|i| {
            for e in &i.raw.events {
                match e {
                    egui::Event::Text(t) => text.push_str(t),
                    egui::Event::Key {
                        key: egui::Key::Backspace,
                        pressed: true,
                        ..
                    } => {
                        text.pop();
                    }
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        ..
                    } => done = Some(true),
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    } => done = Some(false),
                    _ => {}
                }
            }
        });
        if text != before {
            self.events.push(GuiEvent::SetText(f, text));
        }
        if let Some(enter) = done {
            // Enter / Esc: the scripts fire FocusLost with the key's InputObject.
            let mut sh = super::share();
            sh.focus_events.push((f, false, enter, Some((0, 0.0, 0.0))));
            sh.focus = None;
            self.focus = None;
        }
        true
    }

    /// Keyboard navigation of GuiService.SelectedObject: arrows move to the
    /// NextSelection* object or the nearest Selectable one that way; Enter
    /// activates it.
    pub fn navigate(&mut self, ctx: &egui::Context, doc: &Document) {
        let Some(gs) = doc.find_child(doc.dom.root_ref(), "GuiService") else {
            return;
        };
        let sel = match doc.prop(gs, "SelectedObject") {
            Some(Variant::Ref(r)) if doc.exists(r) => r,
            _ => return,
        };
        let (dir, enter) = ctx.input(|i| {
            use egui::Key::*;
            let d = if i.key_pressed(ArrowUp) {
                Some((0.0, -1.0, "NextSelectionUp"))
            } else if i.key_pressed(ArrowDown) {
                Some((0.0, 1.0, "NextSelectionDown"))
            } else if i.key_pressed(ArrowLeft) {
                Some((-1.0, 0.0, "NextSelectionLeft"))
            } else if i.key_pressed(ArrowRight) {
                Some((1.0, 0.0, "NextSelectionRight"))
            } else {
                None
            };
            (d, i.key_pressed(Enter))
        });
        if enter
            && doc
                .class_of(sel)
                .is_some_and(|c| doc::is_a(&c, "GuiButton"))
        {
            self.events
                .push(GuiEvent::Fire(sel, "MouseButton1Click", vec![]));
            self.events.push(GuiEvent::Fire(
                sel,
                "Activated",
                vec![
                    GuiArg::Input {
                        button: 0,
                        began: false,
                        x: 0.0,
                        y: 0.0,
                    },
                    GuiArg::Num(1.0),
                ],
            ));
        }
        let Some((dx, dy, prop)) = dir else { return };
        if let Some(Variant::Ref(n)) = doc.prop(sel, prop) {
            if doc.exists(n) {
                self.events.push(GuiEvent::Select(Some(n)));
                return;
            }
        }
        let Some(from) = self.layout.get(&sel).copied() else {
            return;
        };
        let c0 = (
            from.pos[0] + from.size[0] * 0.5,
            from.pos[1] + from.size[1] * 0.5,
        );
        let mut best: Option<(f32, Ref)> = None;
        for h in &self.hits {
            if h.r == sel {
                continue;
            }
            let selectable = match doc.prop(h.r, "Selectable") {
                Some(Variant::Bool(s)) => s && (h.button || h.textbox),
                _ => h.button || h.textbox,
            };
            let Some(b) = self.layout.get(&h.r) else {
                continue;
            };
            if !selectable {
                continue;
            }
            let c = (
                b.pos[0] + b.size[0] * 0.5 - c0.0,
                b.pos[1] + b.size[1] * 0.5 - c0.1,
            );
            let along = c.0 * dx + c.1 * dy;
            if along <= 1.0 {
                continue;
            }
            let cost = along + (c.0 * dy - c.1 * dx).abs() * 2.0;
            if best.is_none_or(|(bc, _)| cost < bc) {
                best = Some((cost, h.r));
            }
        }
        if let Some((_, r)) = best {
            self.events.push(GuiEvent::Select(Some(r)));
        }
    }

    /// The topmost GuiObject under a screen point (Studio selects it when
    /// clicked while editing). Empty transparent containers (a full-screen
    /// BackgroundTransparency=1 Frame holding a layout) are clicked through.
    pub fn object_at(&self, p: Pos2) -> Option<Ref> {
        self.hits
            .iter()
            .rev()
            .find(|h| {
                h.painted
                    && point_in_quad(&h.quad, p)
                    && h.clip.is_none_or(|c| point_in_quad(&c, p))
            })
            .map(|h| h.r)
    }

    /// Screen centre of a drawn GuiObject (the self-test clicks there).
    pub fn screen_center(&self, r: Ref) -> Option<Pos2> {
        let h = self.hits.iter().find(|h| h.r == r)?;
        Some((h.quad.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / 4.0).to_pos2())
    }
}

/// Where a SurfaceGui / BillboardGui is drawn, its canvas size and distance.
/// Whether a SurfaceGui canvas (corners top-left, top-right, bottom-left)
/// can colour a pixel of the view: on screen it must overlap the view and
/// hold a pixel centre. Behind the camera in part counts as yes.
fn surface_in_view(view_proj: &glam::Mat4, rect: Rect, ppp: f32, tl: Vec3, tr: Vec3, bl: Vec3) -> bool {
    let mut s = Rect::NOTHING;
    for p in [tl, tr, bl, tr + bl - tl] {
        let c = *view_proj * p.extend(1.0);
        if c.w <= 1e-4 {
            return true;
        }
        let ndc = c.truncate() / c.w;
        s.extend_with(Pos2::new(
            rect.min.x + (ndc.x + 1.0) * 0.5 * rect.width(),
            rect.min.y + (1.0 - ndc.y) * 0.5 * rect.height(),
        ));
    }
    // (+ a pixel for the tessellator's feathering)
    let s = s.expand(1.0 / ppp).intersect(rect);
    let holds = |lo: f32, hi: f32| (hi * ppp - 0.5).floor() >= (lo * ppp - 0.5).ceil();
    s.is_positive() && holds(s.min.x, s.max.x) && holds(s.min.y, s.max.y)
}

fn place_3d(d: &Document, cam: &Camera, rect: Rect, g: Ref) -> Option<(Target, [f32; 2], f32)> {
    let class = d.class_of(g)?;
    let adornee = match d.prop(g, "Adornee") {
        Some(Variant::Ref(a)) if d.exists(a) => Some(a),
        _ => None,
    };
    let host = adornee.unwrap_or(d.dom.get_by_ref(g)?.parent());
    let host_class = d.class_of(host)?;
    if class == "SurfaceGui" {
        if !doc::is_a(&host_class, "BasePart") {
            return None;
        }
        let xf = xform_of(d, host)?;
        let h = vec3_of(d.prop(host, "Size")).unwrap_or(Vec3::ONE) * 0.5;
        // Face (NormalId): Right 0, Top 1, Back 2, Left 3, Bottom 4, Front 5 → (normal, canvas right, canvas up).
        let (n, right, up) = match enum_u32(d.prop(g, "Face")).unwrap_or(5) {
            0 => (Vec3::X, -Vec3::Z, Vec3::Y),
            1 => (Vec3::Y, Vec3::X, -Vec3::Z),
            2 => (Vec3::Z, Vec3::X, Vec3::Y),
            3 => (-Vec3::X, Vec3::Z, Vec3::Y),
            4 => (-Vec3::Y, Vec3::X, Vec3::Z),
            _ => (-Vec3::Z, -Vec3::X, Vec3::Y),
        };
        let (wr, hu) = ((right * h).length(), (up * h).length());
        let centre = n * (n * h).length() + n * mech::GUI_SURFACE_LIFT;
        let mid = xf.apply(centre);
        // Only seen from in front of the face.
        if (xf.rot * n).dot(cam.pos - mid) <= 0.0 {
            return None;
        }
        let max = num(d, g, "MaxDistance", 0.0);
        let dist = (mid - cam.pos).length();
        if max > 0.0 && dist > max {
            return None;
        }
        // SizingMode: FixedSize 0 (CanvasSize), PixelsPerStud 1.
        let canvas = if enum_u32(d.prop(g, "SizingMode")).unwrap_or(0) == 1 {
            let pps = num(d, g, "PixelsPerStud", 50.0);
            [wr * 2.0 * pps, hu * 2.0 * pps]
        } else {
            vec2(d, g, "CanvasSize", [800.0, 600.0])
        };
        let tl = xf.apply(centre - right * wr + up * hu);
        let tr = xf.apply(centre + right * wr + up * hu);
        let bl = xf.apply(centre - right * wr - up * hu);
        return Some((Target::Surface { tl, tr, bl }, canvas, dist));
    }
    // BillboardGui: over a part, attachment or model.
    let base = if doc::is_a(&host_class, "BasePart") {
        xform_of(d, host)?.pos
    } else if host_class == "Attachment" {
        let pxf = xform_of(d, d.dom.get_by_ref(host)?.parent())?;
        let cf = match d.prop(host, "CFrame") {
            Some(Variant::CFrame(c)) => Xform::from_cframe(&c),
            _ => Xform {
                pos: Vec3::ZERO,
                rot: Mat3::IDENTITY,
            },
        };
        pxf.apply(cf.pos)
    } else if doc::is_a(&host_class, "Model") {
        let ps: Vec<Vec3> = d
            .dom
            .descendants_of(host)
            .filter(|i| doc::is_a(&i.class, "BasePart"))
            .filter_map(|i| xform_of(d, i.referent()).map(|x| x.pos))
            .collect();
        if ps.is_empty() {
            return None;
        }
        ps.iter().fold(Vec3::ZERO, |a, p| a + *p) / ps.len() as f32
    } else {
        return None;
    };
    let world_off = vec3_of(d.prop(g, "StudsOffsetWorldSpace")).unwrap_or(Vec3::ZERO);
    let cam_off = vec3_of(d.prop(g, "StudsOffset")).unwrap_or(Vec3::ZERO);
    let p = base + world_off + cam.right() * cam_off.x + cam.up() * cam_off.y
        - cam.forward() * cam_off.z;
    let dist = (p - cam.pos).length();
    let max = num(d, g, "MaxDistance", 0.0);
    if max > 0.0 && max.is_finite() && dist > max {
        return None;
    }
    let c = cam.project(rect, p)?;
    let s = udim2(d, g, "Size");
    // Size: scale in studs, offset in pixels.
    let px_per_stud =
        rect.height() / (2.0 * dist.max(0.01) * (cam.fov_deg.to_radians() * 0.5).tan());
    let canvas = [s[0] * px_per_stud + s[1], s[2] * px_per_stud + s[3]];
    if canvas[0] <= 0.0 || canvas[1] <= 0.0 || canvas[0] > 8192.0 || canvas[1] > 8192.0 {
        return None;
    }
    Some((Target::Billboard(c, p), canvas, dist))
}

/// A SurfaceGui / BillboardGui's light (round 23, linear): Brightness where
/// LightInfluence is 0, the light on its face where it is 1, between in
/// between. The light: Ambient, OutdoorAmbient and the sun on the face
/// (0.5 × Lighting.Brightness × N·L), unshadowed and without local lights
/// (an estimate). The world GUI pass then tone-maps it with the scene.
/// (tools/fidelity gui-depth: Studio shows LightInfluence 0 colours through
/// its tone curve: red 255 → 224, Brightness 2 grey 0.4 → 148.)
fn world_gui_light(d: &Document, g: Ref, normal: Vec3) -> f32 {
    let li = num(d, g, "LightInfluence", 1.0).clamp(0.0, 1.0);
    let bright = num(d, g, "Brightness", 1.0).max(0.0);
    if li <= 0.0 {
        return bright;
    }
    bright * (1.0 - li) + scene_light(d, Some(normal)) * li
}

/// The light `world_gui_light` mixes in: Ambient, OutdoorAmbient and the sun
/// on a face with this normal, or, with no normal, the whole sun (Studio's
/// particle and trail shaders light without N·L, round 25).
pub(crate) fn scene_light(d: &Document, normal: Option<Vec3>) -> f32 {
    let root = d.dom.root_ref();
    let lighting = d.find_child(root, "Lighting");
    let get = |k: &str| lighting.and_then(|l| d.prop(l, k));
    let rgb = |k: &str, dflt: [u8; 3]| -> [u8; 3] {
        match get(k) {
            Some(Variant::Color3(c)) => [c.r, c.g, c.b].map(|x| (x.clamp(0.0, 1.0) * 255.0) as u8),
            Some(Variant::Color3uint8(c)) => [c.r, c.g, c.b],
            _ => dflt,
        }
    };
    let luma = |v: Vec3| v.x * 0.299 + v.y * 0.587 + v.z * 0.114;
    let amb = crate::lighting::sq(rgb("Ambient", mech::AMBIENT));
    let out = crate::lighting::sq(rgb("OutdoorAmbient", mech::OUTDOOR_AMBIENT));
    let ambient = luma(amb) + (luma(out) - luma(amb)).max(0.0);
    let clock = f32_of(get("ClockTime")).unwrap_or(mech::CLOCK_TIME);
    let lat = f32_of(get("GeographicLatitude")).unwrap_or(mech::GEOGRAPHIC_LATITUDE);
    let sun = crate::lighting::sun_direction(clock, lat);
    let brightness = f32_of(get("Brightness")).unwrap_or(mech::LIGHT_BRIGHTNESS);
    let sun_light = if sun.y > 0.0 {
        mech::SUN_IRRADIANCE_PER_BRIGHTNESS * brightness * normal.map_or(1.0, |n| n.dot(sun).max(0.0))
    } else {
        0.0
    };
    (ambient + sun_light).max(0.0)
}
