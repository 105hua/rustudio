//! Laying out and drawing one GUI's objects into canvas-space shapes.

use egui::epaint::TextShape;
use egui::text::{LayoutJob, TextFormat};
use egui::{FontId, Shape, Stroke};

use super::*;
use crate::mech;
use crate::scene::{vec3_of, xform_of};

/// 2D affine map (turns of rotated GuiObjects).
#[derive(Clone, Copy, Debug)]
pub(super) struct Aff {
    m: [f32; 4],
    t: [f32; 2],
}

impl Aff {
    pub(super) const ID: Aff = Aff {
        m: [1.0, 0.0, 0.0, 1.0],
        t: [0.0, 0.0],
    };
    pub(super) fn apply(&self, p: Pos2) -> Pos2 {
        Pos2::new(
            self.m[0] * p.x + self.m[1] * p.y + self.t[0],
            self.m[2] * p.x + self.m[3] * p.y + self.t[1],
        )
    }
    /// `self` first, then `o`.
    fn then(&self, o: &Aff) -> Aff {
        let m = [
            o.m[0] * self.m[0] + o.m[1] * self.m[2],
            o.m[0] * self.m[1] + o.m[1] * self.m[3],
            o.m[2] * self.m[0] + o.m[3] * self.m[2],
            o.m[2] * self.m[1] + o.m[3] * self.m[3],
        ];
        let t = o.apply(Pos2::new(self.t[0], self.t[1]));
        Aff { m, t: [t.x, t.y] }
    }
    fn rotate_about(c: Pos2, deg: f32) -> Aff {
        let (s, co) = deg.to_radians().sin_cos();
        Aff {
            m: [co, -s, s, co],
            t: [c.x - co * c.x + s * c.y, c.y - s * c.x - co * c.y],
        }
    }
    pub(super) fn same(&self, o: &Aff) -> bool {
        self.m == o.m && self.t == o.t
    }
    pub(super) fn is_id(&self) -> bool {
        self.m == [1.0, 0.0, 0.0, 1.0] && self.t == [0.0, 0.0]
    }
}

/// One shape in canvas space with its clip, turn and draw order.
#[derive(Clone)]
pub(super) struct Item {
    pub shape: Shape,
    pub clip: Rect,
    pub aff: Aff,
    pub z: i32,
    pub seq: usize,
    /// The object that drew it.
    pub by: Ref,
    /// Its canvas bounds and triangles (turned, clipped) once tessellated;
    /// shared with the memo it is replayed from, so a kept item is
    /// tessellated once (None: draws nothing).
    pub tess: Arc<std::sync::OnceLock<Option<(Rect, Mesh)>>>,
}

/// A clickable / hoverable object found while drawing (canvas space).
#[derive(Clone)]
pub(super) struct LocalHit {
    pub r: Ref,
    pub rect: Rect,
    pub aff: Aff,
    pub clip: Option<Rect>,
    pub button: bool,
    pub textbox: bool,
    pub sink: bool,
    pub scrolling: bool,
    /// Draggable, or holds an enabled UIDragDetector.
    pub drag: bool,
    /// Drew something of its own (background, image, text, stroke…); an
    /// empty transparent container isn't picked by edit-mode clicks.
    pub painted: bool,
}

/// A drawn GuiObject's output from its last walk, in order: runs of its own
/// pieces (shapes, hit boxes, layout boxes, what it read) and its children,
/// each kept in its own `Memo`. Valid while the walk would reach it with the
/// same inputs (`key`) and no journal mark touched it or anything under it
/// (gui_input.rs `memo_up` drops it and every ancestor's, like Studio's
/// LayoutDirty / ViewportDirty bits going up the LayoutNode tree).
pub(super) struct Memo {
    key: [u32; 28],
    segs: Vec<Seg>,
}

pub(super) enum Seg {
    Own {
        items: Vec<Item>,
        hits: Vec<LocalHit>,
        boxes: Vec<(Ref, GuiBox)>,
        reads: Vec<(Ref, Ref)>,
        kids: Vec<Ref>,
    },
    Child { r: Ref, parent: Rect, laid: Option<Rect>, dn: Down },
}

/// A memo being written: where its current run of own pieces starts.
pub(super) struct Rec {
    start: [usize; 5],
    segs: Vec<Seg>,
    no_cache: Option<&'static str>,
    vfs: usize,
}

/// Laying out one GUI root.
pub(super) struct Walk<'a, 'b> {
    pub gui: &'a mut Gui,
    pub ctx: &'a egui::Context,
    pub input: &'a mut DrawIn<'b>,
    pub items: Vec<Item>,
    pub hits: Vec<LocalHit>,
    pub seq: usize,
    /// Every layout written, to keep for frames this GUI isn't walked.
    pub boxes: Vec<(Ref, GuiBox)>,
    pub global_z: bool,
    /// Added to canvas positions for AbsolutePosition (ScreenGui inset).
    pub abs_offset: [f32; 2],
    /// A ScreenGui's canvas origin on screen (UIBlur samples the view there).
    pub screen: Option<Pos2>,
    /// The object being walked (none at the root): who reads what.
    pub cur: Ref,
    /// Memos being written, innermost last; and replay depth.
    pub rec: Vec<Rec>,
    pub replaying: u32,
}

/// Inherited while walking down the tree.
#[derive(Clone, Copy)]
pub(super) struct Down {
    pub aff: Aff,
    pub clip: Rect,
    pub alpha: f32,
    pub tint: [f32; 3],
    /// UIScale so far (offsets, text sizes and strokes grow with it).
    pub scale: f32,
    pub z: i32,
}

impl Walk<'_, '_> {
    fn push(&mut self, shape: Shape, dn: &Down) {
        self.seq += 1;
        self.items.push(Item {
            shape,
            clip: dn.clip,
            aff: dn.aff,
            z: dn.z,
            seq: self.seq,
            by: self.cur,
            tess: Default::default(),
        });
    }

    /// Lays out and draws the GuiObject children of `parent` inside `content`.
    pub(super) fn children(&mut self, parent: Ref, content: Rect, dn: Down) {
        let d = self.input.doc;
        // Folders organize GUI descendants without introducing a layout box.
        // (what was read is kept for the GUI cache: gui_input.rs cache_marks)
        fn gui_children(d: &Document, parent: Ref, out: &mut Vec<Ref>, gui: &mut Gui, by: Ref) {
            gui.walk_kids.push(parent);
            for child in children(d, parent) {
                gui.walk_reads.push((child, by));
                match d.class_of(child).as_deref() {
                    Some("Folder") => gui_children(d, child, out, gui, by),
                    Some(class) if doc::is_a(class, "GuiObject") => out.push(child),
                    _ => {}
                }
            }
        }
        let mut kids = Vec::new();
        gui_children(d, parent, &mut kids, self.gui, self.cur);
        let list = child_of_class(d, parent, "UIListLayout")
            .or_else(|| child_of_class(d, parent, "UIPageLayout"));
        let grid = child_of_class(d, parent, "UIGridLayout");
        let mut placed: HashMap<Ref, Rect> = HashMap::new();
        if let Some(lay) = grid.or(list) {
            let mut order: Vec<Ref> = kids
                .iter()
                .copied()
                .filter(|k| boolp(d, *k, "Visible", true))
                .collect();
            // SortOrder: Name 0, Custom 1, LayoutOrder 2.
            match enum_u32(d.prop(lay, "SortOrder")).unwrap_or(2) {
                0 => order.sort_by_key(|k| name_of(d, *k)),
                2 => order.sort_by_key(|k| num(d, *k, "LayoutOrder", 0.0) as i64),
                _ => {}
            }
            let halign = enum_u32(d.prop(lay, "HorizontalAlignment")).unwrap_or(1); // Center 0, Left 1, Right 2
            let valign = enum_u32(d.prop(lay, "VerticalAlignment")).unwrap_or(1); // Center 0, Top 1, Bottom 2
            let vertical = enum_u32(d.prop(lay, "FillDirection")).unwrap_or(1) == 1;
            if Some(lay) == grid {
                let cell = udim2(d, lay, "CellSize");
                let pad = udim2(d, lay, "CellPadding");
                let cw = cell[0] * content.width() + cell[1] * dn.scale;
                let ch = cell[2] * content.height() + cell[3] * dn.scale;
                let px = pad[0] * content.width() + pad[1] * dn.scale;
                let py = pad[2] * content.height() + pad[3] * dn.scale;
                let max_cells = num(d, lay, "FillDirectionMaxCells", 0.0) as usize;
                let fit = if vertical {
                    ((content.height() + py) / (ch + py).max(1e-3)).floor()
                } else {
                    ((content.width() + px) / (cw + px).max(1e-3)).floor()
                };
                let mut per_line = (fit as usize).max(1);
                if max_cells > 0 {
                    per_line = per_line.min(max_cells);
                }
                let n = order.len();
                let lines = n.div_ceil(per_line).max(1);
                let (cols, rows) = if vertical {
                    (lines, per_line.min(n.max(1)))
                } else {
                    (per_line.min(n.max(1)), lines)
                };
                let total_w = cols as f32 * cw + cols.saturating_sub(1) as f32 * px;
                let total_h = rows as f32 * ch + rows.saturating_sub(1) as f32 * py;
                let x0 = content.left()
                    + match halign {
                        0 => (content.width() - total_w) * 0.5,
                        2 => content.width() - total_w,
                        _ => 0.0,
                    };
                let y0 = content.top()
                    + match valign {
                        0 => (content.height() - total_h) * 0.5,
                        2 => content.height() - total_h,
                        _ => 0.0,
                    };
                for (i, k) in order.iter().enumerate() {
                    let (c, r) = if vertical {
                        (i / per_line, i % per_line)
                    } else {
                        (i % per_line, i / per_line)
                    };
                    placed.insert(
                        *k,
                        Rect::from_min_size(
                            Pos2::new(x0 + c as f32 * (cw + px), y0 + r as f32 * (ch + py)),
                            egui::vec2(cw, ch),
                        ),
                    );
                }
            } else {
                let pad = udim(d, lay, "Padding");
                let gap = pad[0]
                    * if vertical {
                        content.height()
                    } else {
                        content.width()
                    }
                    + pad[1] * dn.scale;
                let sizes: Vec<egui::Vec2> = order
                    .iter()
                    .map(|k| self.own_size(*k, content, dn.scale, 0))
                    .collect();
                let total: f32 = sizes
                    .iter()
                    .map(|s| if vertical { s.y } else { s.x })
                    .sum::<f32>()
                    + gap * order.len().saturating_sub(1) as f32;
                let mut at = if vertical {
                    content.top()
                        + match valign {
                            0 => (content.height() - total) * 0.5,
                            2 => content.height() - total,
                            _ => 0.0,
                        }
                } else {
                    content.left()
                        + match halign {
                            0 => (content.width() - total) * 0.5,
                            2 => content.width() - total,
                            _ => 0.0,
                        }
                };
                for (k, s) in order.iter().zip(sizes) {
                    let r = if vertical {
                        let x = content.left()
                            + match halign {
                                0 => (content.width() - s.x) * 0.5,
                                2 => content.width() - s.x,
                                _ => 0.0,
                            };
                        Rect::from_min_size(Pos2::new(x, at), s)
                    } else {
                        let y = content.top()
                            + match valign {
                                0 => (content.height() - s.y) * 0.5,
                                2 => content.height() - s.y,
                                _ => 0.0,
                            };
                        Rect::from_min_size(Pos2::new(at, y), s)
                    };
                    placed.insert(*k, r);
                    at += if vertical { s.y } else { s.x } + gap;
                }
            }
        }
        // A UIPageLayout shows its current page where the row starts, sliding
        // between pages while it tweens (src/domains/gui_input.luau).
        if let Some(lay) = list.filter(|l| d.class_of(*l).is_some_and(|c| c == "UIPageLayout")) {
            let page = super::share().pages.get(&lay).copied();
            if let Some((from, to, alpha)) = page {
                // (sliding: the page moves without a document mark)
                self.gui.no_cache = Some("page sliding");
                let off = |r: Ref| placed.get(&r).map(|p| p.min - content.min);
                if let (Some(a), Some(b)) = (off(from).or(off(to)), off(to)) {
                    let o = a + (b - a) * alpha;
                    for p in placed.values_mut() {
                        *p = p.translate(-o);
                    }
                }
            }
        }
        // Sibling order: ZIndex, then child order.
        let mut sorted = kids;
        sorted.sort_by_key(|k| num(d, *k, "ZIndex", 1.0) as i32);
        for k in sorted {
            self.object_memo(k, content, placed.get(&k).copied(), dn);
        }
    }

    /// egui font size per Roblox TextSize for a family: Roblox sizes a font so
    /// a line (ascent + descent) is TextSize tall; egui sizes by the em.
    fn em_per_size(&mut self, family: &FontFamily) -> f32 {
        let key = format!("{family:?}");
        if let Some(k) = self.gui.em_ratio.get(&key) {
            return *k;
        }
        let fam = family.clone();
        let line = self
            .ctx
            .fonts_mut(|f| f.row_height(&FontId::new(100.0, fam)))
            / 100.0;
        let k = mech::GUI_FONT_EM_PER_TEXT_SIZE / line.max(0.5);
        self.gui.em_ratio.insert(key, k);
        k
    }

    /// The size of a text object's text, wrapped at `wrap` (AutomaticSize).
    fn measure_text(&mut self, r: Ref, wrap: f32, scale: f32) -> egui::Vec2 {
        let d = self.input.doc;
        let mut text = text_prop(d, r, "Text");
        if text.is_empty() && d.class_of(r).is_some_and(|c| c == "TextBox") {
            text = text_prop(d, r, "PlaceholderText");
        }
        if boolp(d, r, "RichText", false) {
            text = rich_runs(&text).into_iter().map(|run| run.text).collect();
        }
        if text.is_empty() {
            return egui::Vec2::ZERO;
        }
        let font = font_of(d, r);
        let family = self.gui.family(self.ctx, self.input.store, &font);
        let size = num(d, r, "TextSize", 14.0) * scale;
        let k = self.em_per_size(&family);
        let mut f = TextFormat::simple(FontId::new(size * k, family), Color32::WHITE);
        f.line_height = Some(size * num(d, r, "LineHeight", 1.0));
        let mut job = LayoutJob::default();
        job.append(&text, 0.0, f);
        job.wrap.max_width = wrap.max(1.0);
        job.round_output_to_gui = false;
        let g = self.ctx.fonts_mut(|fo| fo.layout_job(job));
        egui::vec2(g.rect.width(), g.rect.height().max(size))
    }

    /// What an AutomaticSize object must hold: its text, or its children
    /// (laid out by a UIListLayout, or by their own positions), plus padding.
    fn content_size(
        &mut self,
        r: Ref,
        size: egui::Vec2,
        scale: f32,
        auto: u32,
        depth: usize,
    ) -> egui::Vec2 {
        let d = self.input.doc;
        let pad = |name: &str, along: f32| {
            let u = child_of_class(d, r, "UIPadding")
                .map(|p| udim(d, p, name))
                .unwrap_or([0.0, 0.0]);
            u[0] * along + u[1] * scale
        };
        let (pl, pr, pt, pb) = (
            pad("PaddingLeft", size.x),
            pad("PaddingRight", size.x),
            pad("PaddingTop", size.y),
            pad("PaddingBottom", size.y),
        );
        let inner = Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2((size.x - pl - pr).max(0.0), (size.y - pt - pb).max(0.0)),
        );
        let mut ext = egui::Vec2::ZERO;
        if d.class_of(r)
            .is_some_and(|c| matches!(c.as_str(), "TextLabel" | "TextButton" | "TextBox"))
        {
            let wrap = if auto & 1 != 0 || !boolp(d, r, "TextWrapped", false) {
                f32::INFINITY
            } else {
                inner.width()
            };
            ext = self.measure_text(r, wrap, scale);
        }
        let kids: Vec<Ref> = children(d, r)
            .into_iter()
            .filter(|c| {
                d.class_of(*c).is_some_and(|k| doc::is_a(&k, "GuiObject"))
                    && boolp(d, *c, "Visible", true)
            })
            .collect();
        if let Some(lay) = child_of_class(d, r, "UIListLayout") {
            let vertical = enum_u32(d.prop(lay, "FillDirection")).unwrap_or(1) == 1;
            let p = udim(d, lay, "Padding");
            let gap =
                p[0] * if vertical {
                    inner.height()
                } else {
                    inner.width()
                } + p[1] * scale;
            let mut along = 0.0f32;
            let mut across = 0.0f32;
            for (i, k) in kids.iter().enumerate() {
                let s = self.own_size(*k, inner, scale, depth + 1);
                along += if vertical { s.y } else { s.x } + if i > 0 { gap } else { 0.0 };
                across = across.max(if vertical { s.x } else { s.y });
            }
            let e = if vertical {
                egui::vec2(across, along)
            } else {
                egui::vec2(along, across)
            };
            ext = ext.max(e);
        } else if let Some(lay) = child_of_class(d, r, "UIGridLayout") {
            // A grid holds its cells (CellSize, not the children's Size), in
            // as many lines as the fill axis fits (as `children` places them).
            let vertical = enum_u32(d.prop(lay, "FillDirection")).unwrap_or(1) == 1;
            let cell = udim2(d, lay, "CellSize");
            let cp = udim2(d, lay, "CellPadding");
            let cw = cell[0] * inner.width() + cell[1] * scale;
            let ch = cell[2] * inner.height() + cell[3] * scale;
            let px = cp[0] * inner.width() + cp[1] * scale;
            let py = cp[2] * inner.height() + cp[3] * scale;
            let n = kids.len();
            if n > 0 {
                let fits = |room: f32, c: f32, p: f32| -> usize {
                    if room.is_finite() && room > 0.0 {
                        (((room + p) / (c + p).max(1e-3)).floor() as usize).max(1)
                    } else {
                        n
                    }
                };
                // The fill axis is the one AutomaticSize doesn't grow.
                let mut per_line = if vertical {
                    if auto & 2 != 0 { n } else { fits(inner.height(), ch, py) }
                } else if auto & 1 != 0 {
                    n
                } else {
                    fits(inner.width(), cw, px)
                };
                let max_cells = num(d, lay, "FillDirectionMaxCells", 0.0) as usize;
                if max_cells > 0 {
                    per_line = per_line.min(max_cells);
                }
                let lines = n.div_ceil(per_line);
                let (cols, rows) = if vertical {
                    (lines, per_line.min(n))
                } else {
                    (per_line.min(n), lines)
                };
                ext = ext.max(egui::vec2(
                    cols as f32 * cw + cols.saturating_sub(1) as f32 * px,
                    rows as f32 * ch + rows.saturating_sub(1) as f32 * py,
                ));
            }
        } else {
            for k in kids {
                let s = self.own_size(k, inner, scale, depth + 1);
                let p = udim2(d, k, "Position");
                let anchor = vec2(d, k, "AnchorPoint", [0.0, 0.0]);
                let at = egui::vec2(
                    p[0] * inner.width() + p[1] * scale,
                    p[2] * inner.height() + p[3] * scale,
                );
                ext = ext.max(at + egui::vec2(s.x * (1.0 - anchor[0]), s.y * (1.0 - anchor[1])));
            }
        }
        egui::vec2(ext.x + pl + pr, ext.y + pt + pb)
    }

    /// A GuiObject's size from Size, AutomaticSize and its constraints (and UIScale).
    fn own_size(&mut self, r: Ref, parent: Rect, scale: f32, depth: usize) -> egui::Vec2 {
        let d = self.input.doc;
        let s = udim2(d, r, "Size");
        let mut size = egui::vec2(
            s[0] * parent.width() + s[1] * scale,
            s[2] * parent.height() + s[3] * scale,
        );
        // AutomaticSize: None 0, X 1, Y 2, XY 3.
        let auto = enum_u32(d.prop(r, "AutomaticSize")).unwrap_or(0);
        if auto != 0 && depth < 8 {
            let c = self.content_size(r, size, scale, auto, depth);
            if auto & 1 != 0 {
                size.x = size.x.max(c.x);
            }
            if auto & 2 != 0 {
                size.y = size.y.max(c.y);
            }
        }
        if let Some(c) = child_of_class(d, r, "UISizeConstraint") {
            let min = vec2(d, c, "MinSize", [0.0, 0.0]);
            let max = vec2(d, c, "MaxSize", [f32::INFINITY, f32::INFINITY]);
            size.x = size.x.clamp(min[0], max[0].max(min[0]));
            size.y = size.y.clamp(min[1], max[1].max(min[1]));
        }
        if let Some(c) = child_of_class(d, r, "UIAspectRatioConstraint") {
            let ratio = num(d, c, "AspectRatio", 1.0).max(1e-3);
            // AspectType: FitWithinMaxSize 0, ScaleWithParentSize 1; DominantAxis: Width 0, Height 1.
            if enum_u32(d.prop(c, "AspectType")) == Some(1) {
                if enum_u32(d.prop(c, "DominantAxis")) == Some(1) {
                    size.x = size.y * ratio;
                } else {
                    size.y = size.x / ratio;
                }
            } else if size.x / size.y.max(1e-3) > ratio {
                size.x = size.y * ratio;
            } else {
                size.y = size.x / ratio;
            }
        }
        if let Some(sc) = child_of_class(d, r, "UIScale") {
            size *= num(d, sc, "Scale", 1.0);
        }
        size.max(egui::Vec2::ZERO)
    }

    /// What a memo depends on besides the document: where and how the walk
    /// reaches the object.
    fn memo_key(&self, parent: Rect, laid: Option<Rect>, dn: &Down) -> [u32; 28] {
        let l = laid.unwrap_or(Rect::NOTHING);
        [
            parent.min.x, parent.min.y, parent.max.x, parent.max.y,
            laid.is_some() as u32 as f32, l.min.x, l.min.y, l.max.x, l.max.y,
            dn.aff.m[0], dn.aff.m[1], dn.aff.m[2], dn.aff.m[3], dn.aff.t[0], dn.aff.t[1],
            dn.clip.min.x, dn.clip.min.y, dn.clip.max.x, dn.clip.max.y,
            dn.alpha, dn.tint[0], dn.tint[1], dn.tint[2], dn.scale, dn.z as f32,
            self.global_z as u32 as f32, self.abs_offset[0], self.abs_offset[1],
        ]
        .map(f32::to_bits)
    }

    fn lens(&self) -> [usize; 5] {
        [self.items.len(), self.hits.len(), self.boxes.len(), self.gui.walk_reads.len(), self.gui.walk_kids.len()]
    }

    /// Ends the innermost memo's current run of own pieces.
    fn close_own(&mut self) {
        let lens = self.lens();
        let Some(top) = self.rec.last_mut() else { return };
        if lens != top.start {
            let s = top.start;
            top.segs.push(Seg::Own {
                items: self.items[s[0]..].to_vec(),
                hits: self.hits[s[1]..].to_vec(),
                boxes: self.boxes[s[2]..].to_vec(),
                reads: self.gui.walk_reads[s[3]..].to_vec(),
                kids: self.gui.walk_kids[s[4]..].to_vec(),
            });
        }
        top.start = lens;
    }

    /// A child GuiObject: from its memo when it has a valid one, else walked
    /// (and its memo written, unless something it drew changes by itself).
    pub(super) fn object_memo(&mut self, k: Ref, parent: Rect, laid: Option<Rect>, dn: Down) {
        if self.replaying == 0 {
            self.close_own();
            if let Some(top) = self.rec.last_mut() {
                top.segs.push(Seg::Child { r: k, parent, laid, dn });
            }
        }
        let key = self.memo_key(parent, laid, &dn);
        if !self.gui.memo_off && self.gui.memo.get(&k).is_some_and(|m| m.key == key) {
            let m = self.gui.memo.remove(&k).unwrap();
            self.replaying += 1;
            for s in &m.segs {
                match s {
                    Seg::Own { items, hits, boxes, reads, kids } => {
                        for it in items {
                            self.seq += 1;
                            let mut it = it.clone();
                            it.seq = self.seq;
                            self.items.push(it);
                        }
                        self.hits.extend(hits.iter().cloned());
                        for (r, b) in boxes {
                            self.gui.layout.insert(*r, *b);
                            self.boxes.push((*r, *b));
                        }
                        self.gui.walk_reads.extend_from_slice(reads);
                        self.gui.walk_kids.extend_from_slice(kids);
                    }
                    Seg::Child { r, parent, laid, dn } => self.object_memo(*r, *parent, *laid, *dn),
                }
            }
            self.replaying -= 1;
            self.gui.memo.insert(k, m);
            self.gui.memo_hits += 1;
        } else {
            let replaying = std::mem::replace(&mut self.replaying, 0);
            let lens = self.lens();
            self.rec.push(Rec { start: lens, segs: Vec::new(), no_cache: self.gui.no_cache.take(), vfs: self.gui.walk_vfs.len() });
            let cur = std::mem::replace(&mut self.cur, k);
            self.object(k, parent, laid, dn);
            self.cur = cur;
            self.close_own();
            let rec = self.rec.pop().unwrap();
            if self.gui.no_cache.is_none() && self.gui.walk_vfs.len() == rec.vfs {
                self.gui.memo.insert(k, Memo { key, segs: rec.segs });
            } else {
                self.gui.memo.remove(&k);
            }
            self.gui.no_cache = rec.no_cache.or(self.gui.no_cache);
            self.replaying = replaying;
            self.gui.memo_walks += 1;
        }
        if self.replaying == 0 {
            let lens = self.lens();
            if let Some(top) = self.rec.last_mut() {
                top.start = lens;
            }
        }
    }

    fn object(&mut self, r: Ref, parent: Rect, laid: Option<Rect>, dn: Down) {
        let d = self.input.doc;
        let Some(class) = d.class_of(r).map(|c| c.to_string()) else {
            return;
        };
        if !boolp(d, r, "Visible", true) {
            return;
        }
        // Drawn: its modifiers (and theirs: a UIStroke's UIGradient) are read.
        self.gui.walk_kids.push(r);
        for c in children(d, r) {
            self.gui.walk_reads.push((c, r));
            if !d.class_of(c).is_some_and(|k| doc::is_a(&k, "GuiObject")) {
                for x in d.subtree(c) {
                    self.gui.walk_reads.push((x, r));
                    self.gui.walk_kids.push(x);
                }
            }
        }
        let size = self.own_size(r, parent, dn.scale, 0);
        let rect = match laid {
            // Layouts own both position and size (UIGridLayout overrides Size).
            Some(l) => l,
            None => {
                let p = udim2(d, r, "Position");
                let anchor = vec2(d, r, "AnchorPoint", [0.0, 0.0]);
                let at = Pos2::new(
                    parent.left() + p[0] * parent.width() + p[1] * dn.scale,
                    parent.top() + p[2] * parent.height() + p[3] * dn.scale,
                );
                Rect::from_min_size(
                    Pos2::new(at.x - anchor[0] * size.x, at.y - anchor[1] * size.y),
                    size,
                )
            }
        };
        let rot = num(d, r, "Rotation", 0.0);
        let mut me = dn;
        me.scale *= child_of_class(d, r, "UIScale")
            .map(|s| num(d, s, "Scale", 1.0))
            .unwrap_or(1.0);
        if self.global_z {
            me.z = num(d, r, "ZIndex", 1.0) as i32;
        }
        if rot.abs() > 1e-3 {
            me.aff = Aff::rotate_about(rect.center(), rot).then(&dn.aff);
        }
        if class == "CanvasGroup" {
            me.alpha *= 1.0 - num(d, r, "GroupTransparency", 0.0);
            let c = color(d, r, "GroupColor3", [1.0, 1.0, 1.0]);
            me.tint = [me.tint[0] * c[0], me.tint[1] * c[1], me.tint[2] * c[2]];
        }
        let mut gb = GuiBox {
            pos: [
                rect.left() + self.abs_offset[0],
                rect.top() + self.abs_offset[1],
            ],
            size: [rect.width(), rect.height()],
            rot,
            text: None,
            canvas: None,
        };

        let items_before = self.items.len();
        // UIBlur: what's behind, blurred, under the object's own background.
        self.blur(r, rect, &me);
        // Background (buttons darken while hovered / pressed: AutoButtonColor).
        let mut corner = child_of_class(d, r, "UICorner").map(|c| {
            let u = udim(d, c, "CornerRadius");
            u[0] * rect.width().min(rect.height()) + u[1] * me.scale
        });
        // Frame.Style RobloxSquare (2) / RobloxRound (3): Studio's
        // blackBkg_square/_round.png, black at 179/255 with an 8 px
        // corner, in place of the frame's own colour and border.
        let frame_style = if class == "Frame" {
            crate::scene::enum_u32(d.prop(r, "Style")).unwrap_or(0)
        } else {
            0
        };
        let styled = matches!(frame_style, 2 | 3);
        if frame_style == 3 {
            corner = Some(8.0 * me.scale);
        }
        let gradient = child_of_class(d, r, "UIGradient");
        let is_button = doc::is_a(&class, "GuiButton");
        let mut bg_t = num(d, r, "BackgroundTransparency", 0.0);
        let mut bg = color(d, r, "BackgroundColor3", [1.0, 1.0, 1.0]);
        if styled {
            bg_t = 1.0 - 179.0 / 255.0;
            bg = [0.0, 0.0, 0.0];
        }
        if is_button && boolp(d, r, "AutoButtonColor", true) {
            let k = if self.gui.pressed.is_some_and(|(p, _)| p == r) {
                mech::GUI_BUTTON_PRESS_DARKEN
            } else if self.gui.hovered.contains(&r) {
                mech::GUI_BUTTON_HOVER_DARKEN
            } else {
                0.0
            };
            bg = [bg[0] * (1.0 - k), bg[1] * (1.0 - k), bg[2] * (1.0 - k)];
        }
        // Frame.Style ChatBlue (1) / ChatGreen (4) / ChatRed (5): Studio's
        // dialog_*.png 9-sliced over the box (gui_fx.rs), not the colour.
        let chat_style = matches!(frame_style, 1 | 4 | 5);
        if chat_style {
            self.chat_frame(frame_style, rect, &me);
            bg_t = 1.0;
        }
        if bg_t < 1.0 && rect.width() > 0.0 && rect.height() > 0.0 {
            let a = (1.0 - bg_t) * me.alpha;
            let tint = me.tint;
            let pts = rounded(rect, corner.unwrap_or(0.0));
            let mesh = match gradient {
                Some(g) => {
                    let gr = Grad::new(d, g, rect);
                    let f = move |p: Pos2| {
                        let (c, t) = gr.at(p);
                        rgba(
                            [
                                bg[0] * c[0] * tint[0],
                                bg[1] * c[1] * tint[1],
                                bg[2] * c[2] * tint[2],
                            ],
                            a * (1.0 - t),
                        )
                    };
                    if corner.is_some() {
                        fill_mesh(&pts, &f)
                    } else {
                        quad_mesh(
                            rect,
                            Rect::from_min_max(egui::epaint::WHITE_UV, egui::epaint::WHITE_UV),
                            egui::TextureId::default(),
                            &f,
                            8,
                        )
                    }
                }
                None => {
                    let c = rgba([bg[0] * tint[0], bg[1] * tint[1], bg[2] * tint[2]], a);
                    fill_mesh(&pts, &|_| c)
                }
            };
            self.push(Shape::Mesh(mesh.into()), &me);
            // BorderSizePixel draws outside the box (BorderMode Outline), not with a UICorner.
            let border = num(d, r, "BorderSizePixel", 0.0) * me.scale;
            if border > 0.0 && corner.is_none() && !styled && !chat_style {
                let bc = rgba(color(d, r, "BorderColor3", [0.0, 0.0, 0.0]), a);
                let o = rect.expand(border);
                for s in [
                    Rect::from_min_max(o.min, Pos2::new(o.max.x, rect.min.y)),
                    Rect::from_min_max(Pos2::new(o.min.x, rect.max.y), o.max),
                    Rect::from_min_max(
                        Pos2::new(o.min.x, rect.min.y),
                        Pos2::new(rect.min.x, rect.max.y),
                    ),
                    Rect::from_min_max(
                        Pos2::new(rect.max.x, rect.min.y),
                        Pos2::new(o.max.x, rect.max.y),
                    ),
                ] {
                    self.push(
                        Shape::Mesh(
                            fill_mesh(
                                &[
                                    s.left_top(),
                                    s.right_top(),
                                    s.right_bottom(),
                                    s.left_bottom(),
                                ],
                                &|_| bc,
                            )
                            .into(),
                        ),
                        &me,
                    );
                }
            }
        }
        let text_class = matches!(class.as_str(), "TextLabel" | "TextButton" | "TextBox");
        // UIStroke: around the box (Border), or around the text (Contextual, on text objects).
        let stroke = child_of_class(d, r, "UIStroke");
        let stroke_on_text = stroke.is_some_and(|s| {
            text_class && enum_u32(d.prop(s, "ApplyStrokeMode")).unwrap_or(0) == 0
        });
        if let Some(s) = stroke.filter(|_| !stroke_on_text) {
            let th = num(d, s, "Thickness", 1.0) * me.scale;
            let st = num(d, s, "Transparency", 0.0);
            if th > 0.0 && st < 1.0 {
                let c = rgba(color(d, s, "Color", [0.0, 0.0, 0.0]), (1.0 - st) * me.alpha);
                let outer = rounded(
                    rect.expand(th * 0.5),
                    corner.map(|c| c + th * 0.5).unwrap_or(0.0),
                );
                self.push(Shape::closed_line(outer, Stroke::new(th, c)), &me);
            }
        }
        if matches!(class.as_str(), "ImageLabel" | "ImageButton") {
            self.image(r, rect, &me, gradient, corner);
        }
        // VideoFrame / VideoDisplay: the video's current frame (media domain).
        if matches!(class.as_str(), "VideoFrame" | "VideoDisplay") {
            self.image(r, rect, &me, gradient, corner);
        }
        if class == "ViewportFrame" {
            self.viewport_frame(r, rect, &me);
        }
        if text_class {
            gb.text = self.text(r, &class, rect, &me, stroke.filter(|_| stroke_on_text));
        }
        // ParticleEmitter2D children, over the object, under its children.
        self.particles(r, rect, &me);
        // GuiService.SelectedObject (keyboard / gamepad selection).
        if self.selected_object() == Some(r) {
            let [cr, cg, cb] = mech::GUI_SELECTION_RGB;
            self.push(
                Shape::closed_line(
                    rounded(rect.expand(3.0), corner.unwrap_or(0.0) + 3.0),
                    Stroke::new(2.0, Color32::from_rgb(cr, cg, cb)),
                ),
                &me,
            );
        }

        let textbox = class == "TextBox";
        let scrolling = class == "ScrollingFrame";
        self.hits.push(LocalHit {
            r,
            rect,
            aff: me.aff,
            clip: (dn.clip != Rect::EVERYTHING).then_some(dn.clip),
            button: is_button,
            textbox,
            sink: is_button || textbox || boolp(d, r, "Active", false),
            scrolling,
            drag: boolp(d, r, "Draggable", false)
                || child_of_class(d, r, "UIDragDetector").is_some_and(|dd| boolp(d, dd, "Enabled", true)),
            painted: self.items.len() > items_before,
        });
        self.gui.layout.insert(r, gb);
        self.boxes.push((r, gb));

        // Children: inside the padding; clipped by ClipsDescendants (not when turned).
        let mut content = rect;
        if let Some(p) = child_of_class(d, r, "UIPadding") {
            let l = udim(d, p, "PaddingLeft");
            let rr = udim(d, p, "PaddingRight");
            let t = udim(d, p, "PaddingTop");
            let b = udim(d, p, "PaddingBottom");
            content = Rect::from_min_max(
                Pos2::new(
                    rect.left() + l[0] * rect.width() + l[1] * me.scale,
                    rect.top() + t[0] * rect.height() + t[1] * me.scale,
                ),
                Pos2::new(
                    rect.right() - rr[0] * rect.width() - rr[1] * me.scale,
                    rect.bottom() - b[0] * rect.height() - b[1] * me.scale,
                ),
            );
        }
        // Chat-styled frames hold their children 17 px in (offline Studio: a
        // child at (-11, -8) of a ChatBlue frame lands at (+6, +9)).
        if chat_style {
            content = content.shrink(17.0 * me.scale);
        }
        if (boolp(d, r, "ClipsDescendants", false) || scrolling) && me.aff.is_id() {
            me.clip = me.clip.intersect(rect);
        }
        if scrolling {
            let cs = udim2(d, r, "CanvasSize");
            let cp = vec2(d, r, "CanvasPosition", [0.0, 0.0]);
            let mut canvas = egui::vec2(
                cs[0] * content.width() + cs[1] * me.scale,
                cs[2] * content.height() + cs[3] * me.scale,
            );
            // AutomaticCanvasSize (None 0, X 1, Y 2, XY 3) grows the canvas
            // to hold the children (Place A's admin panel: CanvasSize 0).
            let acs = enum_u32(d.prop(r, "AutomaticCanvasSize")).unwrap_or(0);
            if acs != 0 {
                let held = self.content_size(r, rect.size(), me.scale, acs, 0)
                    - (rect.size() - content.size());
                if acs & 1 != 0 {
                    canvas.x = canvas.x.max(held.x);
                }
                if acs & 2 != 0 {
                    canvas.y = canvas.y.max(held.y);
                }
            }
            let canvas = canvas.max(content.size());
            // Recorded with the padding around it (the scroll range is this
            // less the frame's size).
            let whole = canvas + (rect.size() - content.size());
            if let Some(b) = self.gui.layout.get_mut(&r) {
                b.canvas = Some([whole.x, whole.y]);
            }
            if let Some((_, b)) = self.boxes.iter_mut().rev().find(|(k, _)| *k == r) {
                b.canvas = Some([whole.x, whole.y]);
            }
            self.children(
                r,
                Rect::from_min_size(
                    Pos2::new(content.left() - cp[0], content.top() - cp[1]),
                    canvas,
                ),
                me,
            );
            let th = num(d, r, "ScrollBarThickness", 12.0) * me.scale;
            let st = num(d, r, "ScrollBarImageTransparency", 0.0);
            if th > 0.0 && st < 1.0 && canvas.y > content.height() + 0.5 {
                let top = content.top() + (cp[1] / canvas.y) * content.height();
                let bar = Rect::from_min_size(
                    Pos2::new(rect.right() - th, top),
                    egui::vec2(th, content.height() * content.height() / canvas.y),
                );
                let c = rgba(
                    color(d, r, "ScrollBarImageColor3", [1.0, 1.0, 1.0]),
                    (1.0 - st) * me.alpha,
                );
                self.push(
                    Shape::Mesh(
                        fill_mesh(
                            &[
                                bar.left_top(),
                                bar.right_top(),
                                bar.right_bottom(),
                                bar.left_bottom(),
                            ],
                            &|_| c,
                        )
                        .into(),
                    ),
                    &me,
                );
            }
        } else {
            self.children(r, content, me);
        }
    }

    fn selected_object(&self) -> Option<Ref> {
        let d = self.input.doc;
        let gs = d.find_child(d.dom.root_ref(), "GuiService")?;
        match d.prop(gs, "SelectedObject") {
            Some(Variant::Ref(r)) if d.exists(r) => Some(r),
            _ => None,
        }
    }

    /// ImageLabel / ImageButton with ImageRect*, ScaleType and UIGradient.
    fn image(&mut self, r: Ref, rect: Rect, me: &Down, gradient: Option<Ref>, corner: Option<f32>) {
        let d = self.input.doc;
        let video = matches!(d.class_of(r).as_deref(), Some("VideoFrame" | "VideoDisplay"));
        let id = if video {
            self.gui.no_cache = Some("video");
            crate::runtime::domains::media::video::surface_uri(r).unwrap_or_default()
        } else {
            text_prop(d, r, "Image")
        };
        let it = num(d, r, if video { "VideoTransparency" } else { "ImageTransparency" }, 0.0);
        if id.is_empty() || it >= 1.0 {
            return;
        }
        let Some((tex, [tw, th])) =
            self.gui
                .texture(self.ctx, self.input.store, self.input.doc_dir, &id)
        else {
            return;
        };
        let ic = color(d, r, if video { "VideoColor3" } else { "ImageColor3" }, [1.0, 1.0, 1.0]);
        let (tw, th) = (tw as f32, th as f32);
        let off = vec2(d, r, if video { "VideoRectOffset" } else { "ImageRectOffset" }, [0.0, 0.0]);
        let sz = vec2(d, r, if video { "VideoRectSize" } else { "ImageRectSize" }, [0.0, 0.0]);
        let src = if sz[0] > 0.0 && sz[1] > 0.0 {
            Rect::from_min_size(Pos2::new(off[0], off[1]), egui::vec2(sz[0], sz[1]))
        } else {
            Rect::from_min_size(Pos2::ZERO, egui::vec2(tw, th))
        };
        let uv_of = |s: Rect| {
            Rect::from_min_max(
                Pos2::new(s.min.x / tw, s.min.y / th),
                Pos2::new(s.max.x / tw, s.max.y / th),
            )
        };
        let alpha = (1.0 - it) * me.alpha;
        let tint = me.tint;
        let grad = gradient.map(|g| Grad::new(d, g, rect));
        let col = move |p: Pos2| match &grad {
            Some(g) => {
                let (c, t) = g.at(p);
                rgba(
                    [
                        ic[0] * c[0] * tint[0],
                        ic[1] * c[1] * tint[1],
                        ic[2] * c[2] * tint[2],
                    ],
                    alpha * (1.0 - t),
                )
            }
            None => rgba([ic[0] * tint[0], ic[1] * tint[1], ic[2] * tint[2]], alpha),
        };
        let steps = if gradient.is_some() { 8 } else { 1 };
        // UICorner rounds the image too (Stretch, Fit, Crop).
        let quad = |dst: Rect, uv: Rect| match corner.filter(|c| *c >= 0.5) {
            Some(c) => rounded_quad_mesh(dst, uv, tex, &col, c),
            None => quad_mesh(dst, uv, tex, &col, steps),
        };
        let mut meshes: Vec<Mesh> = Vec::new();
        // ScaleType: Stretch 0, Slice 1, Tile 2, Fit 3, Crop 4.
        match enum_u32(d.prop(r, "ScaleType")).unwrap_or(0) {
            1 => {
                let sc = match d.prop(r, "SliceCenter") {
                    Some(Variant::Rect(rc)) => Rect::from_min_max(
                        Pos2::new(rc.min.x, rc.min.y),
                        Pos2::new(rc.max.x, rc.max.y),
                    ),
                    _ => Rect::from_min_size(Pos2::ZERO, src.size()),
                };
                let k = num(d, r, "SliceScale", 1.0) * me.scale;
                let xs = [
                    src.left(),
                    src.left() + sc.left(),
                    src.left() + sc.right(),
                    src.right(),
                ];
                let ys = [
                    src.top(),
                    src.top() + sc.top(),
                    src.top() + sc.bottom(),
                    src.bottom(),
                ];
                // A target smaller than its corners shrinks them in
                // proportion (each axis on its own); the middle gets nothing.
                let fit = |a: f32, b: f32, room: f32| {
                    let (a, b) = (a.max(0.0), b.max(0.0));
                    let f = if a + b > room && a + b > 0.0 { room.max(0.0) / (a + b) } else { 1.0 };
                    (a * f, b * f)
                };
                let (dl, dr) = fit(sc.left() * k, (src.width() - sc.right()) * k, rect.width());
                let (dt, db) = fit(sc.top() * k, (src.height() - sc.bottom()) * k, rect.height());
                let dx = [
                    rect.left(),
                    rect.left() + dl,
                    rect.right() - dr,
                    rect.right(),
                ];
                let dy = [
                    rect.top(),
                    rect.top() + dt,
                    rect.bottom() - db,
                    rect.bottom(),
                ];
                for j in 0..3 {
                    for i in 0..3 {
                        let dst = Rect::from_min_max(
                            Pos2::new(dx[i], dy[j]),
                            Pos2::new(dx[i + 1], dy[j + 1]),
                        );
                        if dst.width() > 0.0 && dst.height() > 0.0 {
                            meshes.push(quad_mesh(
                                dst,
                                uv_of(Rect::from_min_max(
                                    Pos2::new(xs[i], ys[j]),
                                    Pos2::new(xs[i + 1], ys[j + 1]),
                                )),
                                tex,
                                &col,
                                steps,
                            ));
                        }
                    }
                }
            }
            2 => {
                let ts = udim2(d, r, "TileSize");
                let tsz = egui::vec2(
                    (ts[0] * rect.width() + ts[1] * me.scale).max(1.0),
                    (ts[2] * rect.height() + ts[3] * me.scale).max(1.0),
                );
                let (nx, ny) = (
                    (rect.width() / tsz.x).ceil() as usize,
                    (rect.height() / tsz.y).ceil() as usize,
                );
                if nx * ny <= 4096 {
                    for j in 0..ny {
                        for i in 0..nx {
                            let dst = Rect::from_min_size(
                                Pos2::new(
                                    rect.left() + i as f32 * tsz.x,
                                    rect.top() + j as f32 * tsz.y,
                                ),
                                tsz,
                            );
                            let vis = dst.intersect(rect);
                            let s = Rect::from_min_max(
                                Pos2::new(
                                    src.left() + (vis.left() - dst.left()) / tsz.x * src.width(),
                                    src.top() + (vis.top() - dst.top()) / tsz.y * src.height(),
                                ),
                                Pos2::new(
                                    src.left() + (vis.right() - dst.left()) / tsz.x * src.width(),
                                    src.top() + (vis.bottom() - dst.top()) / tsz.y * src.height(),
                                ),
                            );
                            meshes.push(quad_mesh(vis, uv_of(s), tex, &col, steps));
                        }
                    }
                }
            }
            3 => {
                let k = (rect.width() / src.width()).min(rect.height() / src.height());
                meshes.push(quad(Rect::from_center_size(rect.center(), src.size() * k), uv_of(src)));
            }
            4 => {
                let k = (rect.width() / src.width()).max(rect.height() / src.height());
                let shown = Rect::from_center_size(
                    src.center(),
                    egui::vec2(rect.width() / k, rect.height() / k),
                );
                meshes.push(quad(rect, uv_of(shown)));
            }
            _ => meshes.push(quad(rect, uv_of(src))),
        }
        for m in meshes {
            self.push(Shape::Mesh(m.into()), me);
        }
    }

    /// Text with its font, size (TextScaled, UITextSizeConstraint), wrapping,
    /// truncation, alignment, rich text and outline. Returns TextBounds.
    fn text(
        &mut self,
        r: Ref,
        class: &str,
        rect: Rect,
        me: &Down,
        ui_stroke: Option<Ref>,
    ) -> Option<[f32; 2]> {
        let d = self.input.doc;
        let mut text = text_prop(d, r, "Text");
        let mut col = color(d, r, "TextColor3", [0.0, 0.0, 0.0]);
        let tt = num(d, r, "TextTransparency", 0.0);
        let focused = self.gui.focus == Some(r);
        if class == "TextBox" && text.is_empty() && !focused {
            text = text_prop(d, r, "PlaceholderText");
            col = color(d, r, "PlaceholderColor3", [0.7, 0.7, 0.7]);
        }
        let font = font_of(d, r);
        let family = self.gui.family(self.ctx, self.input.store, &font);
        let bold_face = font.1 >= 600;
        let rich = boolp(d, r, "RichText", false);
        let wrapped = boolp(d, r, "TextWrapped", false);
        let scaled = boolp(d, r, "TextScaled", false);
        let line_height = num(d, r, "LineHeight", 1.0);
        let tsc = child_of_class(d, r, "UITextSizeConstraint");
        let truncate = enum_u32(d.prop(r, "TextTruncate")).unwrap_or(0) != 0;
        // TextXAlignment: Left 0, Right 1, Center 2. TextYAlignment: Top 0, Center 1, Bottom 2.
        let xa = enum_u32(d.prop(r, "TextXAlignment")).unwrap_or(2);
        let ya = enum_u32(d.prop(r, "TextYAlignment")).unwrap_or(1);
        let tint = me.tint;
        let alpha = me.alpha;
        let ui_scale = me.scale;
        // Roblox sizes a font so a line (ascent + descent) is TextSize tall;
        // egui sizes by the em. This family's line height per em, measured once.
        let em_per_size = self.em_per_size(&family);
        let build = |size: f32, wrap: bool| -> LayoutJob {
            let mut job = LayoutJob::default();
            let runs = if rich {
                rich_runs(&text)
            } else {
                vec![Run {
                    text: text.clone(),
                    bold: false,
                    italic: font.2,
                    underline: false,
                    strike: false,
                    color: None,
                    size: None,
                    transparency: None,
                }]
            };
            for run in runs {
                let rs = run.size.map(|s| s * ui_scale).unwrap_or(size);
                let c = run.color.unwrap_or(col);
                let color = rgba(
                    [c[0] * tint[0], c[1] * tint[1], c[2] * tint[2]],
                    (1.0 - run.transparency.unwrap_or(tt)) * alpha,
                );
                let mut f =
                    TextFormat::simple(FontId::new(rs * em_per_size, family.clone()), color);
                f.line_height = Some(rs * line_height);
                f.italics = run.italic;
                if run.underline {
                    f.underline = Stroke::new((rs / 14.0).max(1.0), color);
                }
                if run.strike {
                    f.strikethrough = Stroke::new((rs / 14.0).max(1.0), color);
                }
                if run.bold && !bold_face {
                    // Faux bold: egui has no synthetic bold, so a little extra spacing.
                    f.extra_letter_spacing = rs * 0.04;
                }
                job.append(&run.text, 0.0, f);
            }
            job.wrap.max_width = if wrap || truncate {
                rect.width().max(1.0)
            } else {
                f32::INFINITY
            };
            if truncate {
                if !wrap {
                    job.wrap.max_rows = 1;
                }
                job.wrap.overflow_character = Some('…');
            }
            job.halign = match xa {
                0 => egui::Align::LEFT,
                1 => egui::Align::RIGHT,
                _ => egui::Align::Center,
            };
            job.round_output_to_gui = false;
            job
        };
        let ctx = self.ctx;
        let layout = |job: LayoutJob| ctx.fonts_mut(|f| f.layout_job(job));
        let mut size = num(d, r, "TextSize", 14.0) * me.scale;
        if scaled {
            let key = (
                text.clone(),
                format!("{family:?}"),
                rect.width() as i32,
                rect.height() as i32,
                wrapped,
            );
            size = match self.gui.scaled.get(&key) {
                Some(s) => *s,
                None => {
                    let max = tsc
                        .map(|c| num(d, c, "MaxTextSize", 100.0))
                        .unwrap_or(100.0)
                        .min(100.0)
                        * me.scale.max(1.0);
                    let (mut lo, mut hi) = (1.0f32, max.max(1.0));
                    for _ in 0..10 {
                        let mid = (lo + hi) * 0.5;
                        let g = layout(build(mid, true));
                        if g.size().x <= rect.width() + 0.5 && g.size().y <= rect.height() + 0.5 {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                    if self.gui.scaled.len() > 4096 {
                        self.gui.scaled.clear();
                    }
                    let s = lo.floor().max(1.0);
                    self.gui.scaled.insert(key, s);
                    s
                }
            };
            if let Some(c) = tsc {
                size = size.max(num(d, c, "MinTextSize", 1.0) * me.scale);
            }
        } else if let Some(c) = tsc {
            size = size.clamp(
                num(d, c, "MinTextSize", 1.0) * me.scale,
                num(d, c, "MaxTextSize", 100.0) * me.scale,
            );
        }
        if text.is_empty() && !focused {
            return Some([0.0, 0.0]);
        }
        let galley = layout(build(size, wrapped || scaled));
        let gs = galley.size();
        // The galley's rows are aligned around x by its halign.
        let x = match xa {
            0 => rect.left(),
            1 => rect.right(),
            _ => rect.center().x,
        };
        let y = match ya {
            0 => rect.top(),
            2 => rect.bottom() - gs.y,
            _ => rect.center().y - gs.y * 0.5,
        };
        let pos = Pos2::new(x, y);
        let mut outline: Option<(f32, Color32)> = None;
        let stroke_t = num(d, r, "TextStrokeTransparency", 1.0);
        if stroke_t < 1.0 {
            outline = Some((
                me.scale.max(1.0),
                rgba(
                    color(d, r, "TextStrokeColor3", [0.0, 0.0, 0.0]),
                    (1.0 - stroke_t) * (1.0 - tt) * me.alpha,
                ),
            ));
        }
        if let Some(s) = ui_stroke {
            let th = num(d, s, "Thickness", 1.0) * me.scale;
            let st = num(d, s, "Transparency", 0.0);
            if th > 0.0 && st < 1.0 {
                outline = Some((
                    th,
                    rgba(
                        color(d, s, "Color", [0.0, 0.0, 0.0]),
                        (1.0 - st) * (1.0 - tt) * me.alpha,
                    ),
                ));
            }
        }
        if let Some((th, c)) = outline {
            for k in 0..8 {
                let a = std::f32::consts::TAU * k as f32 / 8.0;
                let mut ts =
                    TextShape::new(pos + egui::vec2(a.cos(), a.sin()) * th, galley.clone(), c);
                ts.override_text_color = Some(c);
                self.push(Shape::Text(ts), me);
            }
        }
        self.push(
            Shape::Text(TextShape::new(
                pos,
                galley.clone(),
                rgba(col, (1.0 - tt) * me.alpha),
            )),
            me,
        );
        // Caret in the focused TextBox (blinking).
        if focused { self.gui.no_cache = Some("focused textbox"); }
        if focused && (self.ctx.input(|i| i.time) * 2.0) as i64 % 2 == 0 {
            let cx = pos.x + galley.rect.right() + 1.0;
            self.push(
                Shape::line_segment(
                    [
                        Pos2::new(cx, rect.center().y - size * 0.5),
                        Pos2::new(cx, rect.center().y + size * 0.5),
                    ],
                    Stroke::new(1.0, rgba(col, me.alpha)),
                ),
                me,
            );
        }
        Some([galley.rect.width(), galley.rect.height()])
    }

    /// Depth-tested offscreen scene, composited as a single GUI image.
    fn viewport_frame(&mut self, r: Ref, rect: Rect, me: &Down) {
        let d = self.input.doc;
        let cam = match d.prop(r, "CurrentCamera") {
            Some(Variant::Ref(c)) if d.exists(c) => c,
            _ => return,
        };
        let Some(cxf) = xform_of(d, cam) else { return };
        let fov = num(d, cam, "FieldOfView", 70.0).to_radians();
        let amb = color(
            d,
            r,
            "Ambient",
            [200.0 / 255.0, 200.0 / 255.0, 200.0 / 255.0],
        );
        let light = color(
            d,
            r,
            "LightColor",
            [140.0 / 255.0, 140.0 / 255.0, 140.0 / 255.0],
        );
        let mut ldir = vec3_of(d.prop(r, "LightDirection"))
            .unwrap_or(Vec3::splat(-1.0));
        // Studio:
        // near-zero directions reset to the default before shading.
        let length2 = ldir.length_squared();
        let tolerance = if (length2.abs() + 1.0).is_infinite() {
            1.0e-5
        } else {
            (length2.abs() + 1.0) * 1.0e-5
        };
        if !(tolerance < length2.abs()) {
            ldir = Vec3::splat(-1.0);
        }
        let ldir = ldir.normalize_or_zero();
        let it = num(d, r, "ImageTransparency", 0.0);
        let ic = color(d, r, "ImageColor3", [1.0, 1.0, 1.0]);
        let inv = cxf.inverse();
        let f = 1.0 / (fov * 0.5).tan();
        let aspect = rect.width() / rect.height().max(1.0);
        let Some(rs) = self.input.render_state else { return };
        let size = [(rect.width() * self.ctx.pixels_per_point()).ceil().clamp(1.0, 1024.0) as u32,
                    (rect.height() * self.ctx.pixels_per_point()).ceil().clamp(1.0, 1024.0) as u32];
        // (a kept GUI is walked again while its ViewportFrames are dirty;
        // anything under one, and its camera, is read)
        self.gui.walk_vfs.push(r);
        self.gui.walk_reads.push((cam, r));
        for x in d.subtree(r) {
            self.gui.walk_reads.push((x, r));
            self.gui.walk_kids.push(x);
        }
        if let Some(target) = self.gui.vf_targets.get_mut(&r) {
            // (what changes is drawn again at most 15 times a second)
            if target.size == size && (!target.dirty || target.at.elapsed().as_secs_f32() < 1.0 / 15.0) {
                target.used = std::time::Instant::now();
                let mut m = Mesh::with_texture(target.id);
                m.add_rect_with_uv(rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), rgba(std::array::from_fn(|k| ic[k]*me.tint[k]), (1.0-it)*me.alpha));
                let mut inside = *me;
                inside.clip = me.clip.intersect(rect);
                self.push(Shape::Mesh(m.into()), &inside);
                return;
            }
        }
        let _vfs = crate::bench::scope("vf_build");
        let mut vertices = Vec::new();
        // Still loading something it shows: draw it again when it may be in.
        let mut pending = false;
        let mut drawn = std::collections::HashSet::new();
        let mut batches = Vec::new();
        let parts: Vec<Ref> = d
            .dom
            .descendants_of(r)
            .filter(|i| doc::is_a(&i.class, "BasePart"))
            .map(|i| i.referent())
            .take(400)
            .collect();
        for p in parts {
            drawn.insert(p);
            let Some(xf) = xform_of(d, p) else { continue };
            let size = vec3_of(d.prop(p, "Size")).unwrap_or(Vec3::ONE);
            let pt = num(d, p, "Transparency", 0.0);
            if pt >= 1.0 {
                continue;
            }
            let pc = color(d, p, "Color", [0.64, 0.64, 0.64]);
            // Share scene selection: SpecialMesh, CharacterMesh and shipped R6
            // geometry must have the same precedence in a ViewportFrame.
            let Some(sp) = d.dom.get_by_ref(p).and_then(|inst| crate::scene::part_draw(d, inst)) else { continue };
            if sp.draw.placeholder && sp.mesh_asset.is_none() { continue; }
            let mut mesh_uvs = Vec::new();
            let texture = sp.texture_id.as_deref().and_then(|id| {
                let t = self.gui.texture(self.ctx, self.input.store, self.input.doc_dir, id).map(|t| t.0);
                pending |= t.is_none() && !self.gui.images.contains_key(id);
                t
            }).unwrap_or_default();
            let asset = sp.mesh_asset.as_ref();
            let mesh = asset.and_then(|a| self.gui.vf_mesh(self.input.store, self.input.doc_dir, &a.id, a.fit_to_part));
            // Pending/failed file meshes are invisible, just as in the scene.
            if let (Some(a), None) = (asset, &mesh) {
                pending |= !self.gui.vf_meshes.contains_key(&format!("{}|{}", a.fit_to_part, a.id));
                continue;
            }
            let (mesh_size, mesh_xf) = match asset {
                Some(a) if !a.fit_to_part => {
                    let mut xf = sp.draw.xf;
                    xf.pos += xf.rot * a.offset;
                    (a.scale, xf)
                }
                _ => (sp.draw.size, sp.draw.xf),
            };
            let tri_list: Vec<[Vec3; 3]> = match mesh.as_ref()
            {
                Some(m) => m
                    .iter()
                    .map(|t| {
                        mesh_uvs.push([t[0].1, t[1].1, t[2].1]);
                        [
                            mesh_xf.apply(t[0].0 * mesh_size),
                            mesh_xf.apply(t[1].0 * mesh_size),
                            mesh_xf.apply(t[2].0 * mesh_size),
                        ]
                    })
                    .collect(),
                None => {
                    let h = size * 0.5;
                    let c = |x: f32, y: f32, z: f32| xf.apply(Vec3::new(x * h.x, y * h.y, z * h.z));
                    let faces = [
                        [
                            c(1., -1., -1.),
                            c(1., 1., -1.),
                            c(1., 1., 1.),
                            c(1., -1., 1.),
                        ],
                        [
                            c(-1., -1., 1.),
                            c(-1., 1., 1.),
                            c(-1., 1., -1.),
                            c(-1., -1., -1.),
                        ],
                        [
                            c(-1., 1., -1.),
                            c(-1., 1., 1.),
                            c(1., 1., 1.),
                            c(1., 1., -1.),
                        ],
                        [
                            c(-1., -1., 1.),
                            c(-1., -1., -1.),
                            c(1., -1., -1.),
                            c(1., -1., 1.),
                        ],
                        [
                            c(-1., -1., 1.),
                            c(1., -1., 1.),
                            c(1., 1., 1.),
                            c(-1., 1., 1.),
                        ],
                        [
                            c(1., -1., -1.),
                            c(-1., -1., -1.),
                            c(-1., 1., -1.),
                            c(1., 1., -1.),
                        ],
                    ];
                    faces
                        .iter()
                        .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
                        .collect()
                }
            };
            let first = vertices.len() as u32;
            for (tri_index, t) in tri_list.into_iter().enumerate() {
                let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                let uv = mesh_uvs.get(tri_index).copied().unwrap_or([glam::Vec2::ZERO; 3]);
                for i in 0..3 {
                    let normal = mesh.as_ref().map(|m| {
                        (mesh_xf.rot * (m[tri_index][i].2 / mesh_size)).normalize_or_zero()
                    }).unwrap_or(n);
                    let shaded = viewport_diffuse(amb, light, (-ldir).dot(normal), pc, [1.0;3], [1.0;3]);
                    let v = inv.apply(t[i]);
                    let z = -v.z;
                    vertices.push(crate::gui_viewport::Vertex {
                        clip: [v.x*f/aspect, v.y*f, (10000.0*z-500.0)/9999.95, z],
                        uv: uv[i].to_array(), color: [shaded[0], shaded[1], shaded[2], 1.0-pt],
                    });
                }
            }
            batches.push((first..vertices.len() as u32, texture));
        }
        let gpu = self.gui.vf_gpu.get_or_insert_with(|| crate::gui_viewport::Gpu::new(&rs.device));
        let target = self.gui.vf_targets.entry(r).or_insert_with(|| gpu.target(rs, size));
        if target.size != size {
            rs.renderer.write().free_texture(&target.id);
            *target = gpu.target(rs, size);
        }
        gpu.render(rs, target, &vertices, &batches);
        target.used = target.at;
        target.dirty = pending;
        target.cam = Some(cam);
        target.parts = drawn;
        let mut m = Mesh::with_texture(target.id);
        m.add_rect_with_uv(rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), rgba(std::array::from_fn(|k| ic[k]*me.tint[k]), (1.0-it)*me.alpha));
        let mut inside = *me;
        inside.clip = me.clip.intersect(rect);
        self.push(Shape::Mesh(m.into()), &inside);
    }
}

// CPU ViewportFrame diffuse path. Keep image modulation after lighting so
// changing ImageColor3 cannot change the directional response.
fn viewport_diffuse(ambient: [f32; 3], light: [f32; 3], ndotl: f32,
    surface: [f32; 3], image: [f32; 3], tint: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| (ambient[k] + light[k] * ndotl.max(0.0)).min(1.0)
        * surface[k] * image[k] * tint[k])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_lighting_selftest() {
        let ambient = [0.2, 0.3, 0.4];
        let light = [0.6, 0.4, 0.2];
        let dark = viewport_diffuse(ambient, light, -1.0, [1.0; 3], [1.0; 3], [1.0; 3]);
        assert_eq!(dark, ambient);
        let lit = viewport_diffuse(ambient, light, 1.0, [0.5; 3], [0.25; 3], [1.0; 3]);
        for (actual, expected) in lit.into_iter().zip([0.1, 0.0875, 0.075]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        assert_eq!(viewport_diffuse([0.8; 3], [0.6; 3], 1.0,
            [0.5; 3], [1.0; 3], [1.0; 3]), [0.5; 3]);
    }

    #[test]
    fn rotate_about_centre() {
        let a = Aff::rotate_about(Pos2::new(10.0, 10.0), 90.0);
        let p = a.apply(Pos2::new(20.0, 10.0));
        assert!(
            (p.x - 10.0).abs() < 1e-4 && (p.y - 20.0).abs() < 1e-4,
            "{p:?}"
        );
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use rbx_dom_weak::{InstanceBuilder, types::{UDim, UDim2, Enum}};

    #[test]
    fn grid_overrides_authored_tab_width() {
        let mut doc = Document::new_place();
        let root = doc.dom.root_ref();
        let parent = doc.dom.insert(root, InstanceBuilder::new("Frame"));
        let folder = doc.dom.insert(parent, InstanceBuilder::new("Folder"));
        let nested = doc.dom.insert(folder, InstanceBuilder::new("Folder"));
        let control = doc.dom.insert(nested, InstanceBuilder::new("TextButton").with_name("Travel").with_property("LayoutOrder", 100i32));
        let mut tabs = Vec::new();
        for name in ["Sound", "Titles"] {
            tabs.push(doc.dom.insert(parent, InstanceBuilder::new("Frame").with_name(name)
                .with_property("Size", UDim2::new(UDim::new(0.326, 0), UDim::new(1.0, 0)))));
        }
        doc.dom.insert(parent, InstanceBuilder::new("UIGridLayout")
            .with_property("CellSize", UDim2::new(UDim::new(0.49, 0), UDim::new(1.0, 0)))
            .with_property("CellPadding", UDim2::new(UDim::new(0.01, 0), UDim::new(0.0, 0)))
            .with_property("FillDirection", Enum::from_u32(1))
            .with_property("HorizontalAlignment", Enum::from_u32(0)));
        let ctx = egui::Context::default();
        let mut gui = Gui::default();
        let cam = Camera::default();
        let mut store = AssetStore::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 40.0));
        let mut input = DrawIn { render_state: None, doc: &doc, cam: &cam, rect, store: &mut store, doc_dir: None, playing: false };
        let _ = ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx();
            let mut walk = Walk { gui: &mut gui, ctx, input: &mut input, items: Vec::new(), hits: Vec::new(), seq: 0, boxes: Vec::new(), global_z: false, abs_offset: [0.0; 2], screen: None, cur: Ref::none(), rec: Vec::new(), replaying: 0 };
            walk.children(parent, rect, Down { aff: Aff::ID, clip: rect, alpha: 1.0, tint: [1.0; 3], scale: 1.0, z: 0 });
            assert!(walk.boxes.iter().any(|(r, _)| *r == control), "nested Folder control was skipped");
            let boxes: Vec<_> = tabs.iter().map(|r| &walk.boxes.iter().find(|(k, _)| k == r).unwrap().1).collect();
            assert!((boxes[0].size[0] - 490.0).abs() < 0.01);
            assert!((boxes[1].pos[0] - boxes[0].pos[0] - 500.0).abs() < 0.01);
        });
    }
}
