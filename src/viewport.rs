//! The 3D viewport tab: rendering, camera input, picking, selection boxes and
//! the move / scale / rotate gizmos.

use egui::{Color32, PointerButton, Pos2, Rect, Sense, Stroke, Vec2};
use glam::{Mat3, Vec3};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::ustr;

use crate::app::{Editor, Tool};
use crate::math::{snap, Xform};
use crate::mech;
use crate::render::{Frame, LineVertex};
use crate::scene::{self, ScenePart};
use crate::theme;

pub const AXIS_COLORS: [Color32; 3] = [
    Color32::from_rgb(232, 76, 61),
    Color32::from_rgb(98, 204, 92),
    Color32::from_rgb(66, 136, 240),
];
const HOVER_COLOR: Color32 = Color32::from_rgb(255, 214, 92);

/// A gizmo handle: which axis, which end, which tool.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handle {
    pub tool: Tool,
    pub axis: usize,
    pub sign: f32,
}

#[derive(Debug, Clone)]
pub struct GizmoDrag {
    pub handle: Handle,
    origin: Vec3,
    axes: Mat3,
    start_param: f32,
    last_angle: f32,
    /// Accumulated screen-space turn in radians (unwrapped).
    turned: f32,
    center_px: Pos2,
    parts: Vec<(Ref, Xform, Vec3)>,
    /// The applied amount (studs or degrees) for the status readout.
    pub amount: f32,
}

/// Fly speed multiplier after `held` seconds of holding movement keys
/// (Studio's per-tick step over its base step).
pub fn fly_boost_at(held: f32) -> f32 {
    let ticks = held * mech::FLY_TICK_HZ;
    if ticks <= mech::FLY_RAMP_AFTER_TICKS {
        return 1.0;
    }
    (ticks / mech::FLY_RAMP_TICKS_PER_STUD).min(mech::FLY_STEP_CAP) / mech::FLY_BASE_STEP
}

/// World-space axis-aligned bounds of a part.
pub fn part_aabb(p: &ScenePart) -> (Vec3, Vec3) {
    let h = p.size * 0.5;
    let r = p.xf.rot;
    let ext = Vec3::new(
        r.x_axis.x.abs() * h.x + r.y_axis.x.abs() * h.y + r.z_axis.x.abs() * h.z,
        r.x_axis.y.abs() * h.x + r.y_axis.y.abs() * h.y + r.z_axis.y.abs() * h.z,
        r.x_axis.z.abs() * h.x + r.y_axis.z.abs() * h.y + r.z_axis.z.abs() * h.z,
    );
    (p.xf.pos - ext, p.xf.pos + ext)
}

fn box_corners(xf: &Xform, size: Vec3) -> [Vec3; 8] {
    let h = size * 0.5;
    let mut out = [Vec3::ZERO; 8];
    for (i, o) in out.iter_mut().enumerate() {
        let l = Vec3::new(
            if i & 1 == 0 { -h.x } else { h.x },
            if i & 2 == 0 { -h.y } else { h.y },
            if i & 4 == 0 { -h.z } else { h.z },
        );
        *o = xf.pos + xf.rot * l;
    }
    out
}

const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

/// A right-drag camera look in progress: the cursor is hidden and grabbed.
#[derive(Debug, Clone)]
pub struct LookGrab {
    /// Where the cursor was when the drag began; it is put back on release.
    pub start: Pos2,
    pub confined: bool,
    frames: u32,
    /// The cursor was moved for the grab (LockCenter puts it in the middle):
    /// it goes back to `start` on release, as for a confined grab.
    moved: bool,
    /// Consecutive frames the pointer still moved while supposedly locked.
    moving: u32,
}

/// Sum of raw (unaccelerated, unclamped) mouse motion this frame.
fn raw_mouse_delta(ctx: &egui::Context) -> Option<Vec2> {
    ctx.input(|i| {
        let mut sum = Vec2::ZERO;
        let mut any = false;
        for e in &i.raw.events {
            if let egui::Event::MouseMoved(d) = e {
                sum += *d;
                any = true;
            }
        }
        any.then_some(sum)
    })
}

impl Editor {
    fn cursor_cmd(&mut self, ctx: &egui::Context, cmd: egui::ViewportCommand, label: &str) {
        self.cursor_log.push(label.to_string());
        if self.cursor_log.len() > 64 {
            self.cursor_log.remove(0);
        }
        ctx.send_viewport_cmd(cmd);
    }

    /// Right-drag look: grabs and hides the cursor while the button is held
    /// (Locked, falling back to Confined when the platform can't lock), reads
    /// raw mouse motion, and puts the cursor back where it was on release.
    /// Returns the look delta in pixels for this frame.
    pub fn update_look_grab(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        pointer_delta: Vec2,
    ) -> Option<Vec2> {
        use egui::viewport::CursorGrab;
        use egui::ViewportCommand as V;
        let secondary_down = ctx.input(|i| i.pointer.secondary_down());
        if resp.drag_started_by(PointerButton::Secondary) && self.look_grab.is_none() {
            let start = ctx
                .input(|i| i.pointer.press_origin())
                .or(resp.hover_pos())
                .unwrap_or(resp.rect.center());
            let confined = self.grab_use_confined;
            self.look_grab = Some(LookGrab {
                moved: false,
                start,
                confined,
                frames: 0,
                moving: 0,
            });
            let mode = if confined {
                CursorGrab::Confined
            } else {
                CursorGrab::Locked
            };
            self.cursor_cmd(
                ctx,
                V::CursorGrab(mode),
                if confined {
                    "grab:confined"
                } else {
                    "grab:locked"
                },
            );
            self.cursor_cmd(ctx, V::CursorVisible(false), "hide");
        }
        let Some(grab) = self.look_grab.as_mut() else {
            return None;
        };
        if !secondary_down {
            let start = grab.start;
            self.look_grab = None;
            self.cursor_cmd(ctx, V::CursorPosition(start), "restore-position");
            self.cursor_cmd(ctx, V::CursorGrab(CursorGrab::None), "grab:none");
            self.cursor_cmd(ctx, V::CursorVisible(true), "show");
            return None;
        }
        grab.frames += 1;
        // A locked cursor never moves. If it still does after a couple of
        // frames, locking isn't supported here: confine it instead.
        if !grab.confined && grab.frames > 3 {
            let moved = ctx
                .input(|i| i.pointer.latest_pos())
                .is_some_and(|p| (p - grab.start).length() > 24.0);
            if moved {
                grab.confined = true;
                self.grab_use_confined = true;
                self.cursor_cmd(
                    ctx,
                    V::CursorGrab(CursorGrab::Confined),
                    "grab:confined-fallback",
                );
            }
        }
        Some(raw_mouse_delta(ctx).unwrap_or(pointer_delta))
    }

    /// Mouse over the game view while playing: motion and wheel go to the
    /// game (InputChanged / PointerAction); the cursor is grabbed while the
    /// game's UserInputService.MouseBehavior isn't Default (LockCurrentPosition
    /// during a right-drag orbit, LockCenter for shift-lock and first person),
    /// as Roblox does, and comes back where it was.
    pub fn update_game_mouse(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        pointer_delta: Vec2,
        notches: f32,
    ) {
        use egui::viewport::CursorGrab;
        use egui::ViewportCommand as V;
        let Some((behavior, icon, menu)) = self
            .play
            .as_ref()
            .map(|s| (s.mouse_behavior, s.mouse_icon, s.game.menu_open))
        else {
            return;
        };
        // The in-game menu frees the cursor (SettingsHub's Modal button and
        // ForceShow icon override): no grab, the arrow shows, no mouse-look.
        // MouseBehavior itself keeps the game's value and applies again on
        // Resume.
        let (behavior, icon) = if menu { (0, true) } else { (behavior, icon) };
        // A shown GuiButton.Modal lets go of the cursor too, unless the
        // right button is down (gui_modal.rs; asked only while the game
        // wants a lock).
        let behavior = if behavior != 0
            && !ctx.input(|i| i.pointer.secondary_down())
            && self.play.as_ref().is_some_and(|s| s.showing == crate::runtime::Side::Client)
            && crate::gui_modal::modal_shown(&self.doc, &self.gui.layout)
        {
            0
        } else {
            behavior
        };
        if menu {
            self.free_cursor_for_menu(ctx);
        } else if let Some(s) = self.play.as_mut() {
            s.game.menu.cursor_freed = false;
        }
        // LockCenter holds whatever the pointer is over, as Roblox does; it
        // only needs the pointer inside the game view to begin. A window that
        // lost focus has lost its grab too, so take it again on return.
        let (focused, inside) = ctx.input(|i| {
            (
                i.focused,
                i.pointer.latest_pos().is_some_and(|p| resp.rect.contains(p)),
            )
        });
        if !focused {
            // Ask for a real lock again on return; a confine fallback found
            // earlier is re-detected if the platform still can't lock.
            self.grab_use_confined = false;
            if let Some(g) = self.game_grab.take() {
                self.cursor_cmd(ctx, V::CursorGrab(CursorGrab::None), "game-grab:unfocused");
                if g.confined || g.moved {
                    self.cursor_cmd(ctx, V::CursorPosition(g.start), "game-grab:restore");
                }
            }
        }
        let begin = if behavior == 1 { inside } else { resp.hovered() };
        let want = focused && behavior != 0 && (begin || self.game_grab.is_some());
        if want && self.game_grab.is_none() {
            let start = ctx
                .input(|i| i.pointer.latest_pos())
                .or(resp.hover_pos())
                .unwrap_or(resp.rect.center());
            let confined = self.grab_use_confined;
            self.game_grab = Some(LookGrab {
                moved: false,
                start,
                confined,
                frames: 0,
                moving: 0,
            });
            self.cursor_cmd(
                ctx,
                V::CursorGrab(if confined {
                    CursorGrab::Confined
                } else {
                    CursorGrab::Locked
                }),
                "game-grab",
            );
        }
        if !want {
            // (an open menu already let go: free_cursor_for_menu)
            if let Some(g) = self.game_grab.take() {
                self.cursor_cmd(ctx, V::CursorGrab(CursorGrab::None), "game-grab:none");
                if g.confined || g.moved {
                    self.cursor_cmd(ctx, V::CursorPosition(g.start), "game-grab:restore");
                }
            }
        }
        let grabbed = self.game_grab.is_some();
        let mut fallback = false;
        if let Some(g) = self.game_grab.as_mut() {
            g.frames += 1;
            // A locked cursor never moves. If it keeps moving for several
            // frames in a row, locking isn't supported here: confine it
            // instead. One jump isn't enough: coming back to the window
            // (alt-tab) reports the pointer's travel as a single big delta,
            // and taking that for "can't lock" left the cursor free.
            if !g.confined && g.frames > 3 {
                g.moving = if pointer_delta.length() > 24.0 { g.moving + 1 } else { 0 };
                if g.moving >= 3 {
                    g.confined = true;
                    fallback = true;
                }
            }
            // LockCenter keeps the cursor in the middle of the view.
            if behavior == 1 && (g.confined || g.frames == 1) {
                g.moved = true;
                ctx.send_viewport_cmd(V::CursorPosition(resp.rect.center()));
            } else if g.confined && behavior == 2 {
                ctx.send_viewport_cmd(V::CursorPosition(g.start));
            }
        }
        if fallback {
            self.grab_use_confined = true;
            self.cursor_cmd(
                ctx,
                V::CursorGrab(CursorGrab::Confined),
                "game-grab:confined-fallback",
            );
        }
        let hidden = !icon;
        if hidden != self.game_cursor_hidden {
            self.game_cursor_hidden = hidden;
            self.cursor_cmd(
                ctx,
                V::CursorVisible(!hidden),
                if hidden {
                    "game-cursor:hide"
                } else {
                    "game-cursor:show"
                },
            );
        }
        // Roblox reports mouse motion deltas only while the cursor is locked;
        // otherwise InputChanged carries the new position with a zero Delta.
        let d = if grabbed {
            raw_mouse_delta(ctx).unwrap_or(pointer_delta)
        } else {
            Vec2::ZERO
        };
        let moved = resp.hovered() && pointer_delta != Vec2::ZERO;
        if menu {
            return;
        }
        if let Some(s) = self.play.as_mut() {
            s.mouse_delta += glam::Vec2::new(d.x, d.y);
            s.mouse_moved |= moved || d != Vec2::ZERO;
            if resp.hovered() || grabbed {
                s.wheel += notches * mech::WHEEL_INPUT_PER_NOTCH;
            }
        }
    }

    /// While the in-game menu is open nothing holds the cursor: SettingsHub
    /// frees it (Modal + ForceShow) whatever the game's MouseBehavior. On the
    /// frame it opens the window's grab is let go and the arrow shown even
    /// if no grab is on record (a right-drag look, or a grab the platform
    /// kept), so hovering any page can't find the cursor held; after that a
    /// grab of either kind is dropped the moment one appears.
    fn free_cursor_for_menu(&mut self, ctx: &egui::Context) {
        use egui::viewport::CursorGrab;
        use egui::ViewportCommand as V;
        let opened = self.play.as_mut().is_some_and(|s| !std::mem::replace(&mut s.game.menu.cursor_freed, true));
        let held = self.game_grab.take().is_some() | self.look_grab.take().is_some();
        if opened || held {
            // (the cursor stays where the lock held it, as Roblox's does)
            self.cursor_cmd(ctx, V::CursorGrab(CursorGrab::None), "game-grab:menu");
        }
        if opened {
            self.game_cursor_hidden = false;
            self.cursor_cmd(ctx, V::CursorVisible(true), "game-cursor:show");
        }
    }

    /// Lets go of a game cursor grab (after Stop).
    pub fn release_game_mouse(&mut self, ctx: &egui::Context) {
        use egui::viewport::CursorGrab;
        use egui::ViewportCommand as V;
        if self.game_grab.take().is_some() {
            self.cursor_cmd(ctx, V::CursorGrab(CursorGrab::None), "game-grab:none");
        }
        if std::mem::take(&mut self.game_cursor_hidden) {
            self.cursor_cmd(ctx, V::CursorVisible(true), "game-cursor:show");
        }
    }

    /// Fly speed multiplier over the base step, Studio's rule:
    /// the base step until 120 ticks held, then ticks / 240 studs, capped at 15
    /// (×30 base, reached after about 25 s).
    pub fn fly_boost(&self) -> f32 {
        fly_boost_at(self.fly_held)
    }

    /// Distance along a ray to the nearest visible part (locked ones count).
    pub fn ray_distance(&self, ray: &crate::math::Ray) -> Option<f32> {
        self.scene
            .parts
            .iter()
            .zip(&self.scene.bounds)
            .filter(|(sp, b)| {
                let oc = b.truncate() - ray.origin;
                let along = oc.dot(ray.dir);
                sp.draw.alpha > 0.02
                    && oc.length_squared() - along * along <= b.w * b.w
                    && along >= -b.w
            })
            .filter_map(|(sp, _)| ray.hit_obb(&sp.xf, sp.size))
            .filter(|t| *t > 0.0)
            .min_by(f32::total_cmp)
    }

    /// Swaps placeholders for loaded mesh geometry where the mesh is local.
    /// Swaps placeholders for loaded meshes, adds mesh textures, classic
    /// clothing on character body parts, and Decal / Texture quads.
    /// Hands the renderer the parts that changed since last frame, with
    /// loaded meshes, textures, classic clothing and Decal / Texture quads
    /// resolved. Parts whose assets are still loading are redone when an
    /// asset arrives.
    pub fn sync_draws(&mut self) {
        use crate::assets::{clothing_role, composite_over, template_box, ClothingLayer};
        use crate::render::{PartDraw, TexMode};
        let changes = self.scene.take_changes();
        if changes.full || !changes.changed.is_empty() || !changes.removed.is_empty() {
            self.lighting.lightgrid_dirty = true;
            self.lighting.lightocc_dirty = true;
        } else if !changes.moved.is_empty() {
            // Moving parts (characters, enemies, physics) refresh the grid
            // on a slower clock: a rebuild gathers every part around it.
            self.lighting.lightgrid_moves = true;
            self.lighting.lightocc_dirty = true;
        }
        let regen = self.asset_gen != self.draws_asset_gen;
        if regen {
            self.lighting.lightgrid_dirty = true;
        }
        if !changes.full
            && changes.changed.is_empty()
            && changes.removed.is_empty()
            && changes.moved.is_empty()
            && !regen
        {
            return;
        }
        self.draws_asset_gen = self.asset_gen;
        let doc_dir = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        let Editor {
            renderer,
            render_state,
            meshes,
            textures,
            assets,
            clothing_meshes,
            scene,
            draw_cache,
            ..
        } = self;
        let (Some(renderer), Some(rs)) = (renderer.as_mut(), render_state.as_ref()) else {
            return;
        };
        let (device, queue) = (rs.device.clone(), rs.queue.clone());
        let dir = doc_dir.as_deref();
        let use2022 = scene::use_2022_materials(&self.doc);
        if changes.full {
            renderer.clear_items();
            *draw_cache = DrawCache::default();
        }
        for r in &changes.removed {
            renderer.remove_item(*r);
            draw_cache.forget(*r);
        }
        // Moved parts: carry their resolved draws along rigidly.
        for r in &changes.moved {
            let (Some(sp), Some(old)) = (scene.get(*r), draw_cache.xf.get(r).copied()) else {
                continue;
            };
            let Some(draws) = renderer.item_draws(*r) else {
                continue;
            };
            // A CFrame written with the value it already had.
            if sp.xf.pos.abs_diff_eq(old.pos, 1e-5) && sp.xf.rot.abs_diff_eq(old.rot, 1e-6) {
                continue;
            }
            let inv = old.rot.transpose();
            let moved: Vec<PartDraw> = draws
                .iter()
                .map(|d| {
                    let local_pos = inv * (d.xf.pos - old.pos);
                    let local_rot = inv * d.xf.rot;
                    PartDraw {
                        xf: Xform {
                            pos: sp.xf.pos + sp.xf.rot * local_pos,
                            rot: sp.xf.rot * local_rot,
                        },
                        ..*d
                    }
                })
                .collect();
            draw_cache.xf.insert(*r, sp.xf);
            renderer.set_item(*r, moved);
        }
        let mut todo: Vec<Ref> = if changes.full {
            scene.parts.iter().map(|p| p.r).collect()
        } else {
            changes.changed
        };
        if regen {
            todo.extend(draw_cache.waiting.drain());
        }
        let mut seen = std::collections::HashSet::new();
        todo.retain(|r| seen.insert(*r));
        for r in todo {
            let Some(sp) = scene.get(r) else { continue };
            // Layered garments are drawn with the body parts they fit.
            if crate::wrap::is_layered_handle(&self.doc, r) {
                renderer.set_item(r, Vec::new());
                continue;
            }
            let mut d = sp.draw;
            let mut extra = Vec::new();
            let mut waiting = false;
            // A file mesh with no mesh id (an empty SpecialMesh FileMesh or
            // MeshPart): nothing to draw, as in Studio (Place B's striped
            // ground cover, round 23).
            if d.placeholder && sp.mesh_asset.is_none() {
                d.alpha = 0.0;
                d.placeholder = false;
            }
            if let Some(asset) = &sp.mesh_asset {
                waiting = true;
                let sd = crate::bench::scope("sd_mesh_get");
                let got = meshes.get(&asset.id, dir, assets, &mut |p, n, u, c, planes| {
                    renderer.add_mesh_projected(&device, p, n, u, c, planes)
                });
                drop(sd);
                if let Some(m) = got {
                    waiting = false;
                    d.custom_mesh = Some(m.index);
                    d.placeholder = false;
                    if { static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *TRACE.get_or_init(|| std::env::var_os("RUSTUDIO_MESH_DENSITY_TRACE").is_some()) } && self.doc.class_of(sp.r).is_some_and(|c| c == "MeshPart") {
                        static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
                        if SEEN.get_or_init(Default::default).lock().unwrap().insert(format!("{:?}", sp.r)) {
                            eprintln!("MESHPART_SOURCE {:?} Size={:?} MeshSize={:?} InitialSize={:?} bounds_extent={:?} uv={}", sp.r, d.size, self.doc.prop(sp.r,"MeshSize"), self.doc.prop(sp.r,"InitialSize"), m.extent, if sp.texture_id.is_some() { "authored" } else { "built-in stud projection" });
                        }
                    }
                    // Studio: source mesh units
                    // scale by current Size / stored InitialSize (MeshSize).
                    // The upload normalized by extent, so restore that factor.
                    if asset.fit_to_part && self.doc.class_of(sp.r).is_some_and(|c| c == "MeshPart") {
                        if let Some(rbx_dom_weak::types::Variant::Vector3(v)) = self.doc.prop(sp.r, "InitialSize") {
                            let initial = glam::Vec3::new(v.x, v.y, v.z);
                            if initial.is_finite() && initial.min_element() > 1e-4 {
                                d.size *= m.extent / initial;
                            }
                        }
                    }
                    if !asset.fit_to_part {
                        d.size = m.extent * asset.scale;
                        d.xf.pos += d.xf.rot * asset.offset;
                    }
                    // A MeshPart with Bones: its skinned mesh, deformed by
                    // the Bones' current frames (skin.rs).
                    let _skin_scope = crate::bench::scope("sd_skin");
                    if asset.fit_to_part && crate::skin::has_bones(&self.doc, sp.r) {
                        if let crate::assets::Fetch::Ready(bytes) = assets.content(&asset.id, dir) {
                            if let Some(base) = crate::skin::base(&asset.id, &bytes) {
                                if let Some(idx) = crate::skin::skinned_mesh(&self.doc, sp.r, &base, &mut |slot, p, n, u, c| {
                                    renderer.set_mesh(&device, &queue, slot, p, n, u, c)
                                }) {
                                    d.custom_mesh = Some(idx);
                                }
                            }
                        }
                    }
                } else {
                    // No mesh yet (downloading) or none to have (Roblox refused
                    // the download, or it doesn't parse): Studio draws such a
                    // part as nothing until its mesh is in (tools/fidelity
                    // place_b), not a striped box. A pending one is redone when
                    // it arrives.
                    if matches!(meshes.state(&asset.id), Some(Err(_))) {
                        waiting = false;
                    }
                    d.alpha = 0.0;
                    d.placeholder = false;
                }
            }
            let sd_tex = crate::bench::scope("sd_texture");
            if let Some(tid) = &sp.texture_id {
                match textures.image(tid, dir, assets, &mut |img| {
                    renderer.add_texture(&device, &queue, img.width, img.height, &img.pixels)
                }) {
                    Some(t) => {
                        d.texture = Some(t.index);
                        d.tex_mode = sp.tex_kind;
                        if let Some(ids) = crate::scene::surface_maps(&self.doc, r) {
                            let mut ids = ids.to_vec();
                            ids.push(sp.emissive.as_ref().map(|(id, _)| id.clone()));
                            let mut maps = Vec::new();
                            for id in &ids {
                                maps.push(id.as_ref().and_then(|id| textures.image(id, dir, assets, &mut |img| {
                                    renderer.add_texture(&device, &queue, img.width, img.height, &img.pixels)
                                })));
                            }
                            waiting |= ids.iter().zip(&maps).any(|(id, im)| id.is_some() && im.is_none());
                            let key = format!("surface|{tid}|{ids:?}");
                            let (w, h) = (t.image.width.max(2), t.image.height.max(2));
                            if !waiting {
                                // The packed normal/roughness/metalness map is
                                // built only when the composite isn't cached
                                // (it was built per part: a full-size pixel
                                // loop for every SurfaceAppearance part).
                                if let Some(index) = textures.composite(&key, || Some((*t.image).clone()), &mut |im| {
                                    let mut packed = vec![0; (w * h * 4) as usize];
                                    for y in 0..h { for x in 0..w {
                                        let at = ((y * w + x) * 4) as usize;
                                        packed[at..at+4].copy_from_slice(&[128,128,255,0]);
                                        for (k, im) in maps.iter().take(3).enumerate() { if let Some(im) = im {
                                            let im = &im.image;
                                            let q = (((y * im.height / h) * im.width + x * im.width / w) * 4) as usize;
                                            if k == 0 { packed[at..at+2].copy_from_slice(&im.pixels[q..q+2]); }
                                            else { packed[at+1+k] = im.pixels[q]; }
                                        }}
                                    }}
                                    renderer.add_texture_surface(&device, &queue, im.width, im.height, &im.pixels,
                                        maps[3].as_ref().map(|m| (m.image.width, m.image.height, m.image.pixels.as_slice())), Some((w,h,&packed)))
                                }) { d.texture = Some(index); }
                                if let Some((_, strength)) = &sp.emissive { d.emissive = *strength; }
                            }
                        }
                        // An emissive SurfaceAppearance: the same image with its mask.
                        if let Some((mask_id, strength)) = sp.emissive.as_ref().filter(|_| crate::scene::surface_maps(&self.doc, r).is_none()) {
                            match textures.image(mask_id, dir, assets, &mut |img| {
                                renderer.add_texture(
                                    &device,
                                    &queue,
                                    img.width,
                                    img.height,
                                    &img.pixels,
                                )
                            }) {
                                Some(m) => {
                                    let key = format!("emissive|{tid}|{mask_id}");
                                    let mask = m.image.clone();
                                    if let Some(i) = textures.composite(
                                        &key,
                                        || Some((*t.image).clone()),
                                        &mut |im| {
                                            renderer.add_texture_emissive(
                                                &device,
                                                &queue,
                                                im.width,
                                                im.height,
                                                &im.pixels,
                                                Some((mask.width, mask.height, &mask.pixels)),
                                            )
                                        },
                                    ) {
                                        d.texture = Some(i);
                                        d.emissive = *strength;
                                    }
                                }
                                None => waiting = true,
                            }
                        }
                    }
                    None => waiting = true,
                }
            }
            drop(sd_tex);
            let _sd_rest = crate::bench::scope("sd_rest");
            // A mesh body part keeps its mesh under clothing, its UVs
            // re-projected onto the template (assets::body_template_uvs):
            // its own UVs are for Roblox's composited clothing atlas.
            let has_mesh = sp.mesh_asset.is_some() && d.custom_mesh.is_some();
            if let Some((name, shirt, pants)) = &sp.clothing {
                if let Some(role) = clothing_role(name) {
                    let box_mesh = *clothing_meshes.entry(name.clone()).or_insert_with(|| {
                        let (p, n, u) = template_box(&role);
                        renderer.add_mesh(&device, &p, &n, &u)
                    });
                    let mut mesh = (!has_mesh).then_some(box_mesh);
                    if let (true, Some(asset)) = (has_mesh, &sp.mesh_asset) {
                        let key = format!("body-clothing|{name}|{}", asset.id);
                        mesh = match clothing_meshes.get(&key) {
                            Some(i) => Some(*i),
                            None => match assets.content(&asset.id, dir) {
                                crate::assets::Fetch::Ready(bytes) => {
                                    let made = crate::assets::mesh_for_upload(&bytes).and_then(|(p, n, _)| {
                                        let uv = crate::assets::body_template_uvs(name, &p)?;
                                        Some(renderer.add_mesh(&device, &p, &n, &uv))
                                    });
                                    // Unreadable mesh: the template box.
                                    let i = made.unwrap_or(box_mesh);
                                    clothing_meshes.insert(key, i);
                                    Some(i)
                                }
                                _ => {
                                    waiting = true;
                                    None
                                }
                            },
                        };
                    }
                    let mut img = |id: &Option<String>| {
                        id.as_ref().and_then(|id| {
                            textures.image(id, dir, assets, &mut |im| {
                                renderer
                                    .add_texture(&device, &queue, im.width, im.height, &im.pixels)
                            })
                        })
                    };
                    let (s, p) = (img(shirt), img(pants));
                    let tex = match role.layer {
                        ClothingLayer::Shirt => s.as_ref().map(|t| t.index),
                        ClothingLayer::Pants => p.as_ref().map(|t| t.index),
                        ClothingLayer::Torso => {
                            let key = format!("torso|{shirt:?}|{pants:?}");
                            textures.composite(
                                &key,
                                || {
                                    composite_over(
                                        p.as_ref().map(|t| &*t.image),
                                        s.as_ref().map(|t| &*t.image),
                                    )
                                },
                                &mut |im| {
                                    renderer.add_texture(
                                        &device, &queue, im.width, im.height, &im.pixels,
                                    )
                                },
                            )
                        }
                    };
                    if tex.is_none() {
                        waiting = true;
                    }
                    if let Some(tex) = tex {
                        if let Some(mesh) = mesh {
                            d.custom_mesh = Some(mesh);
                        }
                        d.texture = Some(tex);
                        d.tex_mode = TexMode::Over;
                        d.placeholder = false;
                    }
                }
            }
            for dec in &sp.decals {
                let Some(t) = textures.image(&dec.id, dir, assets, &mut |img| {
                    renderer.add_texture(&device, &queue, img.width, img.height, &img.pixels)
                }) else {
                    waiting = true;
                    continue;
                };
                let (axis, sign) = match dec.face {
                    0 => (0, 1.0),
                    1 => (1, 1.0),
                    2 => (2, 1.0),
                    3 => (0, -1.0),
                    4 => (1, -1.0),
                    _ => (2, -1.0),
                };
                // Project onto the actual front-facing mesh triangles rather than
                // drawing a closed slab in front of the head.
                if dec.tiling.is_none() {
                    if let Some(asset) = &sp.mesh_asset {
                        if let Some(tris) = crate::assets::mesh_triangles(&asset.id) {
                            let key = format!("decal-projection|{}|{}", asset.id, dec.face);
                            let index = *clothing_meshes.entry(key).or_insert_with(|| {
                                let mut positions = Vec::new();
                                let mut normals = Vec::new();
                                let mut uvs = Vec::new();
                                for tri in tris.iter() {
                                    let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
                                    if normal[axis] * sign <= 0.01 { continue; }
                                    for v in tri {
                                        positions.push(*v + normal * 0.0005);
                                        normals.push(normal);
                                        let uv = match dec.face {
                                            0 => glam::Vec2::new(0.5 - v.z, 0.5 - v.y),
                                            1 => glam::Vec2::new(v.x + 0.5, v.z + 0.5),
                                            2 => glam::Vec2::new(0.5 - v.x, 0.5 - v.y),
                                            3 => glam::Vec2::new(v.z + 0.5, 0.5 - v.y),
                                            4 => glam::Vec2::new(v.x + 0.5, 0.5 - v.z),
                                            _ => glam::Vec2::new(v.x + 0.5, 0.5 - v.y),
                                        };
                                        uvs.push(uv);
                                    }
                                }
                                renderer.add_mesh(&device, &positions, &normals, &uvs)
                            });
                            extra.push(PartDraw {
                                custom_mesh: Some(index), texture: Some(t.index),
                                tex_mode: TexMode::Decal, color: dec.color,
                                alpha: ((1.0 - dec.transparency) * if dec.base_surface { d.alpha } else { 1.0 }).min(0.998),
                                material: 0.0, material_id: 256, mat_layer: -1,
                                surfaces: 0, cast_shadow: false, uvt: PartDraw::NO_UVT,
                                emissive: [0.0; 3], reflectance: 0.0,
                                ..d
                            });
                            continue;
                        }
                    }
                }
                let s = sp.decal_size;
                let mut n = Vec3::ZERO;
                n[axis] = sign;
                let (w, h) = match axis {
                    0 => (s.z, s.y),
                    1 => (s.x, s.z),
                    _ => (s.x, s.y),
                };
                let uvt = match dec.tiling {
                    Some([su, sv, ou, ov]) => [
                        w / su.max(0.01),
                        h / sv.max(0.01),
                        ou / su.max(0.01),
                        ov / sv.max(0.01),
                    ],
                    None => PartDraw::NO_UVT,
                };
                // A decal has one surface. A thin closed Block draws the
                // same image on both sides, visibly doubling it at oblique
                // angles because the decal pass does not write depth.
                let key = format!("decal-plane|{}", dec.face);
                let index = *clothing_meshes.entry(key).or_insert_with(|| {
                    let (u, v) = match dec.face {
                        0 => (-Vec3::Z, -Vec3::Y),
                        1 => (Vec3::X, Vec3::Z),
                        2 => (-Vec3::X, -Vec3::Y),
                        3 => (Vec3::Z, -Vec3::Y),
                        4 => (Vec3::X, -Vec3::Z),
                        _ => (Vec3::X, -Vec3::Y),
                    };
                    let points = [(-u-v)*0.5, (u-v)*0.5, (u+v)*0.5, (-u+v)*0.5];
                    let ids = [0, 1, 2, 0, 2, 3];
                    let uv = [glam::Vec2::ZERO, glam::Vec2::X, glam::Vec2::ONE, glam::Vec2::Y];
                    renderer.add_mesh(&device, &ids.map(|i| points[i]), &[n; 6], &ids.map(|i| uv[i]))
                });
                let xf = sp.xf;
                extra.push(PartDraw {
                    xf: Xform {
                        pos: xf.pos + xf.rot * (n * (s[axis] * 0.5 + 0.012)),
                        rot: xf.rot,
                    },
                    size: s,
                    color: dec.color,
                    alpha: ((1.0 - dec.transparency) * if dec.base_surface { sp.draw.alpha } else { 1.0 }).min(0.998),
                    mesh: crate::render::MeshKind::Block,
                    material: 0.0,
                    placeholder: false,
                    hover: false,
                    reflectance: 0.0,
                    custom_mesh: Some(index),
                    texture: Some(t.index),
                    tex_mode: TexMode::Decal,
                    uvt,
                    emissive: [0.0; 3],
                    tex_albedo: 1.0,
                    surfaces: 0,
                    material_id: 256,
                    mat_layer: -1,
                    cast_shadow: false,
                });
            }
            // Layered clothing fitted onto this body part (wrap.rs).
            let is_target = self.doc.dom.get_by_ref(r).is_some_and(|i| {
                i.children()
                    .iter()
                    .any(|c| self.doc.class_of(*c).is_some_and(|k| k == "WrapTarget"))
            });
            if is_target {
                let model = self
                    .doc
                    .dom
                    .get_by_ref(r)
                    .map(|i| i.parent())
                    .unwrap_or_else(Ref::none);
                match crate::wrap::fitted(&self.doc, model) {
                    crate::wrap::Lookup::Ready(fit) => {
                        for (i, piece) in fit.pieces.iter().enumerate().filter(|(_, p)| p.part == r)
                        {
                            let key = (std::sync::Arc::as_ptr(&fit) as usize, i);
                            let (mesh, centre, extent) =
                                *self.wrap_meshes.entry(key).or_insert_with(|| {
                                    let (lo, hi) = piece.positions.iter().fold(
                                        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                                        |(a, b), p| (a.min(*p), b.max(*p)),
                                    );
                                    let centre = (lo + hi) * 0.5;
                                    let extent = (hi - lo).max(Vec3::splat(1e-4));
                                    let unit: Vec<Vec3> = piece
                                        .positions
                                        .iter()
                                        .map(|p| (*p - centre) / extent)
                                        .collect();
                                    let normals: Vec<Vec3> = piece
                                        .normals
                                        .iter()
                                        .map(|n| (*n * extent).normalize_or_zero())
                                        .collect();
                                    (
                                        renderer.add_mesh(&device, &unit, &normals, &piece.uvs),
                                        centre,
                                        extent,
                                    )
                                });
                            let Some(hsp) = scene.get(piece.handle) else {
                                continue;
                            };
                            let mut g = hsp.draw;
                            g.xf = Xform {
                                pos: sp.xf.pos + sp.xf.rot * centre,
                                rot: sp.xf.rot,
                            };
                            g.size = extent;
                            g.custom_mesh = Some(mesh);
                            g.placeholder = false;
                            g.surfaces = 0;
                            g.mat_layer = -1;
                            if let Some(tid) = &hsp.texture_id {
                                match textures.image(tid, dir, assets, &mut |img| {
                                    renderer.add_texture(
                                        &device,
                                        &queue,
                                        img.width,
                                        img.height,
                                        &img.pixels,
                                    )
                                }) {
                                    Some(t) => {
                                        g.texture = Some(t.index);
                                        g.tex_mode = hsp.tex_kind;
                                    }
                                    None => waiting = true,
                                }
                            }
                            extra.push(g);
                        }
                    }
                    crate::wrap::Lookup::Pending => waiting = true,
                    crate::wrap::Lookup::Failed(e) => {
                        if self.wrap_warned.insert(e.clone()) {
                            textures
                                .messages
                                .push(format!("Layered clothing couldn't be fitted: {e}"));
                        }
                    }
                    crate::wrap::Lookup::None => {}
                }
            }
            // Material texture packs (matpack.rs) on untextured parts.
            if d.texture.is_none() && !d.placeholder && !sp.variant && d.material < 0.5 {
                if let Some(e) = crate::matpack::entry(d.material_id, use2022) {
                    match crate::matpack::maps(e) {
                        crate::matpack::Lookup::Ready(m) => {
                            d.mat_layer = renderer.material_layer(&device, &queue, &m);
                            // The maps carry the material's colour now.
                            let fb = crate::matpack::fallback_albedo(d.material_id, use2022);
                            for c in 0..3 {
                                if fb[c] > 0.0 {
                                    d.color[c] /= fb[c];
                                }
                            }
                            d.tex_albedo = 1.0;
                            // Box faces: one repeat per the material's studs
                            // per tile (Studio's uvScale). Untextured MeshParts use studs too.
                            let c = crate::matpack::consts(d.material_id, use2022);
                            let union_projection = sp.mesh_asset.as_ref().is_some_and(|a| a.id.starts_with("rbxtemp://rustudio/union/"));
                            let meshpart_projection = d.custom_mesh.is_some() && self.doc.class_of(sp.r).is_some_and(|c| c == "MeshPart");
                            let stud_projection = union_projection || meshpart_projection;
                            let k = if d.custom_mesh.is_some() && !stud_projection {
                                m.tiling
                            } else {
                                m.tiling / c[5]
                            };
                            // Negative w selects shared mesh stud-space projection in the vertex shader.
                            d.uvt = [k, k, 0.0, if meshpart_projection { -2.0 } else if union_projection { -1.0 } else { 0.0 }];
                            if meshpart_projection && std::env::var_os("RUSTUDIO_MESH_DENSITY_TRACE").is_some() {
                                static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
                                if SEEN.get_or_init(Default::default).lock().unwrap().insert(format!("{:?}", sp.r)) {
                                    eprintln!("MESHPART_DENSITY {:?} drawn_extent={:?} MeshSize={:?} InitialSize={:?} uv=stud-projection repeats_per_stud={} map_width={} texels_per_stud={}", sp.r, d.size, self.doc.prop(sp.r,"MeshSize"), self.doc.prop(sp.r,"InitialSize"), k, crate::matpack::LAYER_SIZE, k * crate::matpack::LAYER_SIZE as f32);
                                }
                            }
                            if union_projection && std::env::var_os("RUSTUDIO_UNION_DENSITY_TRACE").is_some() {
                                static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
                                let key = format!("{:?}:{:?}", sp.r, d.size);
                                if SEEN.get_or_init(Default::default).lock().unwrap().insert(key) {
                                    eprintln!("UNION_UV {:?} Size={:?} actual_uvt={:?} pack_tiling={} studs_per_tile={}", sp.r, d.size, d.uvt, m.tiling, c[5]);
                                }
                            }
                            // The detail normal pack, drawn once it's loaded.
                            let mut detail = -1.0;
                            if e.detail != 0 && c[1] * c[2] != 0.0 {
                                match crate::matpack::pack_maps(e.detail, 0xffff_ffff) {
                                    crate::matpack::Lookup::Ready(dm) => {
                                        detail = renderer.material_layer(&device, &queue, &dm) as f32;
                                    }
                                    crate::matpack::Lookup::Pending => waiting = true,
                                    crate::matpack::Lookup::Unavailable(_) => {}
                                }
                            }
                            renderer.set_material_params(
                                &queue,
                                d.mat_layer,
                                [[detail, c[1], c[2], c[0]], [c[3], c[4], 0.0, 0.0]],
                            );
                        }
                        crate::matpack::Lookup::Pending => waiting = true,
                        crate::matpack::Lookup::Unavailable(_) => {}
                    }
                }
            }
            let flags = [
                (sp.mesh_asset.is_some() && d.custom_mesh.is_some()) as u32,
                matches!(
                    d.tex_mode,
                    TexMode::Over | TexMode::Opaque | TexMode::OpaqueTinted | TexMode::Cutout
                ) as u32,
                extra.len() as u32,
                (sp.clothing.is_some() && d.texture.is_some()) as u32,
            ];
            draw_cache.set(r, sp.xf, flags, waiting);
            let mut all = Vec::with_capacity(1 + extra.len());
            all.push(d);
            all.extend(extra);
            renderer.set_item(r, all);
        }
        self.draw_stats = self.draw_cache.totals();
        let msgs: Vec<String> = std::mem::take(&mut self.meshes.messages)
            .into_iter()
            .chain(std::mem::take(&mut self.textures.messages))
            .collect();
        for msg in msgs {
            self.warn(msg);
        }
    }

    /// Meshes Workspace.Terrain's voxels (SmoothGrid): the solid surface by
    /// 32³ chunk with Studio's terrain shading (only the chunks an edit
    /// changed, and their neighbours, are re-meshed: round 21), the water as
    /// one see-through mesh.
    fn sync_terrain(&mut self) {
        let terrain = self
            .doc
            .workspace()
            .and_then(|ws| self.doc.find_child(ws, "Terrain"));
        // A Terrain Editor stroke hands its voxels and touched chunks over
        // (no decode, no full compare).
        let brushed = self.terrain_brushed.take();
        let (voxels, brushed) = match brushed {
            Some((v, dirty, water)) => (Some(v), Some((dirty, water))),
            None => match crate::terrain::Voxels::from_dom(&self.doc.dom) {
                Some(Ok(v)) => (Some(v), None),
                Some(Err(e)) => {
                    self.warn(format!("Terrain couldn't be read: {e}"));
                    return;
                }
                None => (None, None),
            },
        };
        let (Some(renderer), Some(rs)) = (self.renderer.as_mut(), self.render_state.as_ref())
        else {
            return;
        };
        let device = rs.device.clone();
        let (Some(terrain), Some(voxels)) = (terrain, voxels) else {
            renderer.clear_terrain();
            if let Some((_, old, _)) = self.terrain_drawn.take() {
                renderer.remove_item(old);
            }
            self.terrain_tris = 0;
            return;
        };
        let _p = crate::bench::scope("terrain_remesh");
        // Another place (or Terrain): start over.
        let same = self
            .terrain_drawn
            .as_ref()
            .is_some_and(|(id, r, _)| *id == self.doc.id && *r == terrain);
        if !same {
            renderer.clear_terrain();
            if let Some((_, old, _)) = self.terrain_drawn.take() {
                renderer.remove_item(old);
            }
            self.terrain_grass_chunks.clear();
            self.terrain_grass_built.clear();
        }
        let old = self.terrain_drawn.take().map(|(_, _, v)| v);
        let use2022 = scene::use_2022_materials(&self.doc);
        crate::terrain::set_use2022(use2022);
        let cube = |m: u8| crate::terrain_mat::material(m, use2022).is_some_and(|t| t.cube);
        let dirty = match &brushed {
            Some((d, _)) => d.clone(),
            None => crate::terrain::dirty_chunks(old.as_ref(), &voxels),
        };
        for key in &dirty {
            let verts = crate::terrain::chunk_triangles(&voxels, *key, &cube);
            if crate::terrain::chunk_has_grass_top(&verts) {
                self.terrain_grass_chunks.insert(*key);
            } else {
                self.terrain_grass_chunks.remove(key);
            }
            // Its blades are made again (sync_terrain_grass).
            if self.terrain_grass_built.remove(key) {
                renderer.set_terrain_grass(&device, *key, &[]);
            }
            renderer.set_terrain_chunk(&device, *key, &verts);
        }
        // Water: one mesh, rebuilt when the water or a chunk beside it changed.
        let water_dirty = match &brushed {
            Some((_, water)) => *water || dirty.iter().any(|k| voxels.has_water_near(*k)),
            None => crate::terrain::water_changed(old.as_ref(), &voxels),
        } || dirty.iter().any(|k| voxels.has_water_near(*k));
        // The Terrain's water look (round 21): WaterColor, WaterTransparency,
        // WaterReflectance (fs_part kind 5).
        let num = |name: &str, d: f32| match self.doc.prop(terrain, name) {
            Some(Variant::Float32(f)) => f,
            Some(Variant::Float64(f)) => f as f32,
            _ => d,
        };
        let water_rgb = match self.doc.prop(terrain, "WaterColor") {
            Some(Variant::Color3(c)) => [c.r, c.g, c.b].map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8),
            Some(Variant::Color3uint8(c)) => [c.r, c.g, c.b],
            _ => mech::WATER_COLOR_DEFAULT,
        };
        let transparency = num("WaterTransparency", mech::WATER_TRANSPARENCY_DEFAULT).clamp(0.0, 1.0);
        let reflectance = num("WaterReflectance", mech::WATER_REFLECTANCE_DEFAULT).clamp(0.0, 1.0);
        self.water_waves.wave_size = num("WaterWaveSize", mech::WATER_WAVE_SIZE_DEFAULT).max(0.0);
        self.water_waves.wave_speed = num("WaterWaveSpeed", mech::WATER_WAVE_SPEED_DEFAULT).max(0.0);
        self.water_waves.transparency = transparency;
        let water_dirty = water_dirty
            || self.terrain_water_look != Some((water_rgb, transparency, reflectance));
        self.terrain_water_look = Some((water_rgb, transparency, reflectance));
        if water_dirty {
            renderer.remove_item(terrain);
            let mut draws = Vec::new();
            for m in crate::terrain::build_water_meshes(&voxels) {
                if m.indices.is_empty() {
                    continue;
                }
                // Fit the mesh into the unit box the renderer instances.
                let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for p in &m.positions {
                    lo = lo.min(Vec3::from(*p));
                    hi = hi.max(Vec3::from(*p));
                }
                let center = (lo + hi) * 0.5;
                let extent = (hi - lo).max(Vec3::splat(0.01));
                let mut pos = Vec::with_capacity(m.indices.len());
                let mut nrm = Vec::with_capacity(m.indices.len());
                for i in &m.indices {
                    pos.push((Vec3::from(m.positions[*i as usize]) - center) / extent);
                    nrm.push(Vec3::from(m.normals[*i as usize]) * extent);
                }
                let uvs = vec![glam::Vec2::ZERO; pos.len()];
                let mesh = renderer.add_mesh(&device, &pos, &nrm, &uvs);
                let rgb = water_rgb;
                draws.push(crate::render::PartDraw {
                    xf: Xform {
                        pos: center,
                        rot: Mat3::IDENTITY,
                    },
                    size: extent,
                    color: [
                        scene::srgb_to_linear(rgb[0]),
                        scene::srgb_to_linear(rgb[1]),
                        scene::srgb_to_linear(rgb[2]),
                    ],
                    // Blended (under 0.999, the opaque pass); fs_part kind 5
                    // sets the real alpha. Water casts no shadow, as in Studio.
                    alpha: 0.99,
                    mesh: crate::render::MeshKind::Block,
                    material: 5.0,
                    placeholder: false,
                    hover: false,
                    reflectance: 0.0,
                    custom_mesh: Some(mesh),
                    texture: None,
                    tex_mode: crate::render::TexMode::None,
                    uvt: crate::render::PartDraw::NO_UVT,
                    emissive: [reflectance, transparency, 0.0],
                    tex_albedo: 1.0,
                    surfaces: 0,
                    material_id: 256,
                    mat_layer: -1,
                    cast_shadow: false,
                });
            }
            renderer.set_item(terrain, draws);
        }
        self.terrain_tris = renderer.terrain_triangles();
        self.terrain_present = crate::terrain::materials_present(&voxels);
        self.terrain_params_dirty = true;
        if !same {
            self.info(format!(
                "Terrain: {} voxels, {} triangles",
                voxels.count(),
                self.terrain_tris
            ));
        }
        if !same || !dirty.is_empty() {
            self.lighting.lightgrid_dirty = true;
            self.lighting.lightgrid_terrain = None;
        }
        self.terrain_drawn = Some((self.doc.id, terrain, voxels));
    }

    /// Terrain.Decoration grass on the Grass-topped chunks near the camera:
    /// one chunk's blades made per frame, far ones dropped (round 21).
    fn sync_terrain_grass(&mut self) {
        let Some((_, terrain, voxels)) = self.terrain_drawn.as_ref() else {
            return;
        };
        let terrain = *terrain;
        let on = matches!(self.doc.prop(terrain, "Decoration"), Some(Variant::Bool(true)));
        let length = match self.doc.prop(terrain, "GrassLength") {
            Some(Variant::Float32(f)) => f,
            _ => mech::GRASS_LENGTH_DEFAULT,
        };
        let (Some(renderer), Some(rs)) = (self.renderer.as_mut(), self.render_state.as_ref())
        else {
            return;
        };
        let device = rs.device.clone();
        if !on || (length - self.terrain_grass_length).abs() > 1e-4 {
            for k in self.terrain_grass_built.drain() {
                renderer.set_terrain_grass(&device, k, &[]);
            }
            self.terrain_grass_length = length;
            if !on {
                return;
            }
        }
        let chunk = crate::terrain::VOXEL * 32.0;
        let near = |k: &[i32; 3], r: f32| {
            let lo = Vec3::new(k[0] as f32, k[1] as f32, k[2] as f32) * chunk;
            let d = (self.cam.pos - self.cam.pos.clamp(lo, lo + Vec3::splat(chunk))).length();
            d <= r
        };
        let far: Vec<[i32; 3]> = self
            .terrain_grass_built
            .iter()
            .filter(|k| !near(k, mech::GRASS_RADIUS * 1.25))
            .copied()
            .collect();
        for k in far {
            self.terrain_grass_built.remove(&k);
            renderer.set_terrain_grass(&device, k, &[]);
        }
        let next = self
            .terrain_grass_chunks
            .iter()
            .filter(|k| !self.terrain_grass_built.contains(*k) && near(k, mech::GRASS_RADIUS))
            .min_by_key(|k| {
                let c = Vec3::new(k[0] as f32 + 0.5, k[1] as f32 + 0.5, k[2] as f32 + 0.5) * chunk;
                (c - self.cam.pos).length() as i64
            })
            .copied();
        if let Some(k) = next {
            let blades = crate::terrain::grass_blades(voxels, k, length);
            renderer.set_terrain_grass(&device, k, &blades);
            self.terrain_grass_built.insert(k);
        }
    }

    /// The terrain shader's per-material parameters (textures, tiling,
    /// blending, MaterialColors recolour): rebuilt after a terrain change and
    /// while texture packs are still arriving.
    fn sync_terrain_params(&mut self) {
        let generation = crate::matpack::generation();
        if !self.terrain_params_dirty && generation == self.terrain_params_gen {
            return;
        }
        let Some((_, terrain, _)) = self.terrain_drawn.as_ref() else {
            return;
        };
        let terrain = *terrain;
        let colors = match self.doc.prop(terrain, "MaterialColors") {
            Some(Variant::BinaryString(b)) if AsRef::<[u8]>::as_ref(&b).len() >= 69 => {
                Some(AsRef::<[u8]>::as_ref(&b).to_vec())
            }
            _ => None,
        };
        let use2022 = scene::use_2022_materials(&self.doc);
        let (Some(renderer), Some(rs)) = (self.renderer.as_mut(), self.render_state.as_ref())
        else {
            return;
        };
        let (device, queue) = (rs.device.clone(), rs.queue.clone());
        let mut params = vec![[0.0f32; 4]; crate::render::TERRAIN_PALETTE * 4];
        let mut waiting = false;
        for m in 0..23u8 {
            let Some(t) = crate::terrain_mat::material(m, use2022) else {
                continue;
            };
            let rgb = match &colors {
                Some(c) if m >= 2 && (m as usize) * 3 + 3 <= c.len() => {
                    let i = m as usize * 3;
                    [c[i], c[i + 1], c[i + 2]]
                }
                _ if m >= 2 => t.default_color,
                _ => crate::terrain::material_color(m),
            };
            let unit = |c: [u8; 3]| c.map(|x| x as f32 / 255.0);
            // At the default MaterialColor the texture shows as it is (Studio's
            // grey Concrete and Slate pads); another colour turns its luma and
            // chroma from the default's to it.
            let [k, cs, sn] = crate::terrain_mat::recolor(unit(t.default_color), unit(rgb));
            for face in 0..3u8 {
                let i = crate::terrain::palette(m, face) as usize * 4;
                let mut layer = -1.0f32;
                let tex = t.faces[face as usize].clone().unwrap_or_default();
                if self.terrain_present[m as usize] && tex.pack != 0 {
                    match crate::matpack::pack_maps(tex.pack, 0xff80_8080) {
                        crate::matpack::Lookup::Ready(maps) => {
                            layer = renderer.material_layer(&device, &queue, &maps) as f32;
                        }
                        crate::matpack::Lookup::Pending => waiting = true,
                        crate::matpack::Lookup::Unavailable(_) => {}
                    }
                }
                let range = (tex.blend_max - tex.blend_min).abs().max(1e-3);
                params[i] = [tex.tiling, tex.detiling, tex.rotation, layer];
                params[i + 1] = [1.0 / range, -tex.blend_min / range, tex.blend_scale.max(1e-3), 0.0];
                params[i + 2] = [k, cs, sn, 0.0];
                // The average albedo (sRGB) a material shows without its pack.
                // (Measured at the default MaterialColors only: a place's own
                // colour recolours avgAlbedo, as place-c-terrain's custom colours
                // match Studio better that way.)
                let fallback = (!use2022 && rgb == t.default_color)
                    .then(|| crate::mech::TERRAIN_FALLBACK_ALBEDO.iter().find(|f| f.0 == m).map(|f| f.1))
                    .flatten();
                let avg = if m >= 2 { unit(fallback.unwrap_or(t.avg_albedo)) } else { unit(rgb) };
                params[i + 3] = [avg[0], avg[1], avg[2], 0.0];
            }
        }
        renderer.set_terrain_params(&queue, &params);
        self.terrain_params_dirty = waiting;
        self.terrain_params_gen = generation;
    }

    /// The part (resolved to its top-level Model unless Alt is held) under a point.
    pub fn pick(&self, rect: Rect, p: Pos2, raw: bool) -> Option<Ref> {
        let ray = self.cam.ray(rect, p);
        let mut best: Option<(f32, Ref)> = None;
        for (sp, b) in self.scene.parts.iter().zip(&self.scene.bounds) {
            // Cheap bounding-sphere test first.
            let oc = b.truncate() - ray.origin;
            let along = oc.dot(ray.dir);
            if oc.length_squared() - along * along > b.w * b.w || along < -b.w {
                continue;
            }
            if sp.draw.alpha < 0.02 || sp.locked {
                continue;
            }
            if let Some(t) = ray.hit_obb(&sp.xf, sp.size) {
                if best.is_none_or(|(bt, _)| t < bt) {
                    best = Some((t, sp.r));
                }
            }
        }
        let r = best?.1;
        Some(if raw { r } else { self.selection_target(r) })
    }

    /// Clicking a part inside a Model selects the outermost Model below Workspace.
    fn selection_target(&self, r: Ref) -> Ref {
        let stop = self.doc.default_parent();
        let mut target = r;
        let mut cur = r;
        loop {
            let Some(parent) = self.doc.dom.get_by_ref(cur).map(|i| i.parent()) else {
                break;
            };
            if parent.is_none() || parent == stop || parent == self.doc.dom.root_ref() {
                break;
            }
            if self.doc.class_of(parent).is_some_and(|c| c == "Model") {
                target = parent;
            }
            cur = parent;
        }
        target
    }

    fn gizmo_frame(&self) -> Option<(Vec3, Mat3, Vec3)> {
        let parts = self.selected_parts();
        if parts.is_empty() {
            return None;
        }
        if parts.len() == 1 || self.local_space {
            let p = self.scene.get(parts[0])?;
            if self.local_space || self.tool == Tool::Scale {
                return Some((p.xf.pos, p.xf.rot, p.size * 0.5));
            }
        }
        let (mn, mx) = self.selection_bounds()?;
        Some(((mn + mx) * 0.5, Mat3::IDENTITY, (mx - mn) * 0.5))
    }

    /// Screen-space handle shapes for the active tool.
    fn handles(&self, rect: Rect) -> Vec<(Handle, Vec<Pos2>)> {
        let mut out = Vec::new();
        let tool = self.tool;
        if tool == Tool::Select {
            return out;
        }
        let Some((origin, axes, half)) = self.gizmo_frame() else {
            return out;
        };
        let wpp = self.cam.world_per_px(rect, origin);
        for axis in 0..3 {
            let dir = axes.col(axis);
            match tool {
                Tool::Move | Tool::Scale => {
                    for sign in [1.0f32, -1.0] {
                        let base = origin + dir * sign * (half[axis] + 6.0 * wpp);
                        let len = if tool == Tool::Move {
                            mech::GIZMO_ARROW_PX * 0.55
                        } else {
                            24.0
                        };
                        let tip = base + dir * sign * len * wpp;
                        if let (Some(a), Some(b)) =
                            (self.cam.project(rect, base), self.cam.project(rect, tip))
                        {
                            out.push((Handle { tool, axis, sign }, vec![a, b]));
                        }
                    }
                }
                Tool::Rotate => {
                    let radius = mech::GIZMO_RING_PX * wpp;
                    let (u, v) = (axes.col((axis + 1) % 3), axes.col((axis + 2) % 3));
                    let pts: Vec<Pos2> = (0..=64)
                        .filter_map(|i| {
                            let a = i as f32 / 64.0 * std::f32::consts::TAU;
                            self.cam
                                .project(rect, origin + (u * a.cos() + v * a.sin()) * radius)
                        })
                        .collect();
                    if pts.len() > 2 {
                        out.push((
                            Handle {
                                tool,
                                axis,
                                sign: 1.0,
                            },
                            pts,
                        ));
                    }
                }
                Tool::Select => {}
            }
        }
        out
    }

    fn handle_at(&self, handles: &[(Handle, Vec<Pos2>)], p: Pos2) -> Option<Handle> {
        let mut best: Option<(f32, Handle)> = None;
        for (h, pts) in handles {
            let d = match h.tool {
                Tool::Scale => (pts[1] - p).length() - 4.0,
                _ => pts
                    .windows(2)
                    .map(|w| dist_to_segment(p, w[0], w[1]))
                    .fold(f32::MAX, f32::min),
            };
            if d < mech::GIZMO_HIT_PX && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, *h));
            }
        }
        best.map(|b| b.1)
    }

    fn start_drag(&mut self, rect: Rect, handle: Handle, p: Pos2) {
        let Some((origin, axes, _)) = self.gizmo_frame() else {
            return;
        };
        let parts: Vec<(Ref, Xform, Vec3)> = self
            .selected_parts()
            .into_iter()
            .filter_map(|r| self.scene.get(r).map(|sp| (r, sp.xf, sp.size)))
            .collect();
        let ray = self.cam.ray(rect, p);
        let dir = axes.col(handle.axis) * handle.sign;
        let start_param = ray.closest_on_line(origin, dir).unwrap_or(0.0);
        let center_px = self.cam.project(rect, origin).unwrap_or(p);
        let last_angle = (p.y - center_px.y).atan2(p.x - center_px.x);
        self.drag = Some(GizmoDrag {
            handle,
            origin,
            axes,
            start_param,
            last_angle,
            turned: 0.0,
            center_px,
            parts,
            amount: 0.0,
        });
    }

    fn update_drag(&mut self, rect: Rect, p: Pos2) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let h = drag.handle;
        let dir = drag.axes.col(h.axis) * h.sign;
        let ray = self.cam.ray(rect, p);
        let mut updates: Vec<(Ref, Xform, Vec3)> = Vec::new();
        match h.tool {
            Tool::Move => {
                let Some(t) = ray.closest_on_line(drag.origin, dir) else {
                    return;
                };
                let d = snap(
                    t - drag.start_param,
                    if self.snap_move { self.move_inc } else { 0.0 },
                );
                drag.amount = d;
                for (r, xf, size) in &drag.parts {
                    updates.push((
                        *r,
                        Xform {
                            pos: xf.pos + dir * d,
                            rot: xf.rot,
                        },
                        *size,
                    ));
                }
            }
            Tool::Scale => {
                let Some(t) = ray.closest_on_line(drag.origin, dir) else {
                    return;
                };
                let d = snap(
                    t - drag.start_param,
                    if self.snap_move { self.move_inc } else { 0.0 },
                );
                drag.amount = d;
                for (r, xf, size) in &drag.parts {
                    // Grow the part's own axis closest to the dragged direction, one side only.
                    let k = (0..3)
                        .max_by(|a, b| {
                            xf.rot
                                .col(*a)
                                .dot(dir)
                                .abs()
                                .total_cmp(&xf.rot.col(*b).dot(dir).abs())
                        })
                        .unwrap_or(0);
                    let along = xf.rot.col(k) * xf.rot.col(k).dot(dir).signum();
                    let mut s = *size;
                    s[k] = (size[k] + d).max(mech::MIN_PART_SIZE);
                    let grow = s[k] - size[k];
                    updates.push((
                        *r,
                        Xform {
                            pos: xf.pos + along * grow * 0.5,
                            rot: xf.rot,
                        },
                        s,
                    ));
                }
            }
            Tool::Rotate => {
                let a = (p.y - drag.center_px.y).atan2(p.x - drag.center_px.x);
                let pi = std::f32::consts::PI;
                drag.turned += (a - drag.last_angle + pi).rem_euclid(2.0 * pi) - pi;
                drag.last_angle = a;
                let axis = drag.axes.col(h.axis);
                // Screen y points down, so a clockwise screen turn is a
                // counter-clockwise turn about an axis that faces the camera.
                let facing = axis.dot(self.cam.pos - drag.origin) > 0.0;
                let mut deg = drag.turned.to_degrees();
                if facing {
                    deg = -deg;
                }
                let deg = snap(
                    deg,
                    if self.snap_rotate {
                        self.rotate_inc
                    } else {
                        0.0
                    },
                );
                drag.amount = deg;
                let rot = Mat3::from_axis_angle(axis, deg.to_radians());
                for (r, xf, size) in &drag.parts {
                    let nx = Xform {
                        pos: drag.origin + rot * (xf.pos - drag.origin),
                        rot: rot * xf.rot,
                    }
                    .orthonormalized();
                    updates.push((*r, nx, *size));
                }
            }
            Tool::Select => {}
        }
        for (r, xf, size) in updates {
            self.doc
                .set_prop_live(r, ustr("CFrame"), Variant::CFrame(xf.to_cframe()));
            if h.tool == Tool::Scale {
                self.doc
                    .set_prop_live(r, ustr("Size"), Variant::Vector3(crate::math::rv3(size)));
            }
        }
    }

    fn finish_drag(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        let mut changes = Vec::new();
        for (r, xf, size) in &drag.parts {
            let old_cf = Some(Variant::CFrame(xf.to_cframe()));
            if let Some(new_cf) = self
                .doc
                .dom
                .get_by_ref(*r)
                .and_then(|i| i.properties.get(&ustr("CFrame")).cloned())
            {
                changes.push((*r, ustr("CFrame"), old_cf, new_cf));
            }
            if drag.handle.tool == Tool::Scale {
                if let Some(new_size) = self
                    .doc
                    .dom
                    .get_by_ref(*r)
                    .and_then(|i| i.properties.get(&ustr("Size")).cloned())
                {
                    changes.push((
                        *r,
                        ustr("Size"),
                        Some(Variant::Vector3(crate::math::rv3(*size))),
                        new_size,
                    ));
                }
            }
        }
        let label = match drag.handle.tool {
            Tool::Move => "Move",
            Tool::Scale => "Resize",
            Tool::Rotate => "Rotate",
            Tool::Select => "Edit",
        };
        self.doc.record_props(label, changes);
    }

    pub fn viewport_ui(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        self.viewport_rect = rect;
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let ctx = ui.ctx().clone();
        let dt = ctx.input(|i| i.stable_dt).min(0.1);

        // ---------------------------------------------------------- camera
        let (scroll, alt, ctrl, shift, delta) = ctx.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.modifiers.alt,
                i.modifiers.command,
                i.modifiers.shift,
                i.pointer.delta(),
            )
        });
        let _ = scroll;
        // While playing (the client's view), the mouse and keys belong to the
        // game: its camera scripts turn and zoom the view.
        let driving = self.play_game_input();
        // Raw wheel notches (not egui's smoothed scroll) so each click moves at once.
        let notches: f32 = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Point => delta.y / mech::WHEEL_POINTS_PER_NOTCH,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                    }),
                    _ => None,
                })
                .sum()
        });
        if !driving && (self.game_grab.is_some() || self.game_cursor_hidden) {
            self.release_game_mouse(&ctx);
        }
        if driving {
            self.update_game_mouse(&ctx, &resp, delta, notches);
        } else if resp.hovered() && notches != 0.0 {
            if let Some(p) = resp.hover_pos() {
                let ray = self.cam.ray(rect, p);
                let hit = self.ray_distance(&ray);
                self.cam.zoom_toward(ray.dir, notches, shift);
                self.last_zoom = Some((ray.dir, hit));
            }
        }
        if !driving {
            let look = self.update_look_grab(&ctx, &resp, delta);
            if let Some(d) = look {
                self.cam.rotate(d.x, d.y);
            }
            if resp.dragged_by(PointerButton::Middle) {
                self.cam.pan(delta.x, delta.y);
            }
            if alt && resp.dragged_by(PointerButton::Primary) && self.drag.is_none() {
                self.cam.orbit(delta.x, delta.y);
            }
        }
        // WASD / Q / E fly the camera wherever the mouse is (Studio), except
        // while a text field or the script editor has the keyboard, a game
        // TextBox is focused, the game has the input (Play), or Ctrl is held.
        let keyboard_ok = !driving
            && !ctx.egui_wants_keyboard_input()
            && self.gui.focus.is_none()
            && self.ui.insert_popup.is_none()
            && self.explorer.renaming.is_none()
            && !ctrl;
        let dt = self.fixed_dt.unwrap_or(dt);
        let mut moving = false;
        if keyboard_ok {
            let mut dir = Vec3::ZERO;
            use egui::Key;
            if self.phys.down(Key::W) {
                dir.z += 1.0;
            }
            if self.phys.down(Key::S) {
                dir.z -= 1.0;
            }
            if self.phys.down(Key::D) {
                dir.x += 1.0;
            }
            if self.phys.down(Key::A) {
                dir.x -= 1.0;
            }
            if self.phys.down(Key::E) {
                dir.y += 1.0;
            }
            if self.phys.down(Key::Q) {
                dir.y -= 1.0;
            }
            if dir != Vec3::ZERO {
                moving = true;
                self.cam.fly(dir, dt, shift, self.fly_boost());
                ctx.request_repaint();
            }
        }
        // Holding movement keys ramps the speed up. Studio clears its hold
        // counter as soon as no movement key is down, and while Shift is held
        //.
        if moving && !shift {
            self.fly_held += dt;
            self.fly_idle = 0.0;
        } else {
            self.fly_idle += dt;
            self.fly_held = 0.0;
        }

        // ---------------------------------------------------------- render
        {
            let _s = crate::bench::scope("vp_scene_sync");
            self.scene.sync(&mut self.doc);
        }
        let pointer = resp.hover_pos();
        let handles = self.handles(rect);
        let hover_handle = if self.drag.is_none() {
            pointer.and_then(|p| self.handle_at(&handles, p))
        } else {
            None
        };
        // (No hover outline while playing the game, as in Studio.)
        let in_play = self
            .play
            .as_ref()
            .is_some_and(|p| p.mode == crate::runtime::RunMode::Play);
        self.hover = if hover_handle.is_none()
            && self.drag.is_none()
            && self.marquee.is_none()
            && !in_play
        {
            pointer.and_then(|p| self.pick(rect, p, alt))
        } else {
            None
        };

        {
            let _s = crate::bench::scope("vp_sync_draws");
            self.sync_draws();
        }
        if std::mem::take(&mut self.scene.terrain_dirty) {
            self.sync_terrain();
        }
        self.sync_terrain_params();
        self.sync_terrain_grass();
        // Hover: the hovered part (or model's parts) drawn again, highlighted.
        let mut overlay = Vec::new();
        if let (Some(h), Some(rend)) = (self.hover, self.renderer.as_ref()) {
            for r in std::iter::once(h).chain(self.doc.dom.descendants_of(h).map(|i| i.referent()))
            {
                if let Some(d) = rend.item_draws(r).and_then(|d| d.first()) {
                    overlay.push(*d);
                }
            }
        }
        let mut lines: Vec<LineVertex> = Vec::new();
        // (RUSTUDIO_NO_GRID hides it for lighting comparisons with Studio;
        // Studio's Play view has none, tools/fidelity demo shots.)
        if self.show_grid
            && !self.is_playing()
            && self.doc.kind == crate::doc::DocKind::Place
            && std::env::var_os("RUSTUDIO_NO_GRID").is_none()
        {
            scene::grid_lines(&mut lines, self.ui.grid_size);
        }
        // WireframeHandleAdornments (domains/world_misc.rs).
        let (wire_lines, wire_texts) = crate::runtime::domains::world::misc::wire_geometry(&self.doc);
        lines.extend(wire_lines.into_iter().map(|(pos, color)| LineVertex { pos, color }));
        let ppp = ctx.pixels_per_point();
        let mut size = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        // (a fixed render size: the camera takes its aspect)
        let rrect = match self.view_render_size {
            Some(s) => {
                size = s;
                Rect::from_min_size(rect.min, egui::vec2(s[0] as f32, s[1] as f32) / ppp)
            }
            None => rect,
        };
        let (globals, lights, shadows, sun_cascades) = {
            let _l = crate::bench::scope("lighting");
            self.frame_lighting(rrect)
        };
        let vp_render = crate::bench::scope("vp_render");
        let tex = match (self.renderer.as_mut(), self.render_state.as_ref()) {
            (Some(r), Some(rs)) => r.render(
                rs,
                size,
                &Frame {
                    globals,
                    cull_view_proj: self.cam.view_proj(rrect),
                    fov_y: self.cam.fov_deg.to_radians(),
                    lights: &lights,
                    shadows,
                    sun_cascades,
                    overlay: &overlay,
                    lines: &lines,
                    sky: self.cam.ortho.is_none(),
                    water: crate::water::WaterLook {
                        time: self.harness.started.elapsed().as_secs_f32(),
                        ..self.water_waves
                    },
                },
            ),
            _ => None,
        };
        drop(vp_render);
        let painter = ui.painter_at(rect);
        // The 3D view behind the GUI (UIBlur samples it).
        crate::gui::share().scene = tex.map(|id| (id, rect));
        match tex {
            Some(id) => {
                painter.image(
                    id,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                self.hud.viewport_drawn = true;
            }
            None => {
                painter.rect_filled(rect, 0.0, theme::BG);
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "3D viewport unavailable (no wgpu device)",
                    egui::FontId::proportional(16.0),
                    theme::TEXT_DIM,
                );
            }
        }

        // Wireframe text: monospace, baseline-left on the point (Studio).
        for (p, text, size, c) in wire_texts {
            if let Some(at) = self.cam.project(rect, p) {
                let col = Color32::from_rgba_unmultiplied(
                    (c[0].clamp(0.0, 1.0) * 255.0) as u8,
                    (c[1].clamp(0.0, 1.0) * 255.0) as u8,
                    (c[2].clamp(0.0, 1.0) * 255.0) as u8,
                    (c[3] * 255.0) as u8,
                );
                painter.text(at, egui::Align2::LEFT_BOTTOM, text, egui::FontId::monospace(size), col);
            }
        }

        // GUIs over the 3D view: ScreenGuis (the player's while playing,
        // StarterGui's while editing), SurfaceGuis and BillboardGuis.
        // Effects share these batches with world GUIs. Clear every frame,
        // including when Show UI / RUSTUDIO_NO_GUI skips Gui::draw; otherwise
        // live particles and beams accumulate all previous frames' geometry.
        self.gui.world_batches.clear();
        let playing = self.play.is_some();
        // View > Show UI.
        let show_gui = self.ui.show_gui;
        let gui_took = {
            let doc_dir = self
                .doc
                .path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf());
            let Editor {
                gui,
                doc,
                cam,
                assets,
                play,
                game_grab,
                ..
            } = self;
            // (RUSTUDIO_NO_GUI hides every GUI, to tell GUI from 3D in tests.)
            if std::env::var_os("RUSTUDIO_NO_GUI").is_none() && show_gui {
                let _g = crate::bench::scope("gui_draw");
                gui.sync_roots(doc);
                gui.draw(
                    &ctx,
                    &painter,
                    crate::gui::DrawIn {
                        render_state: self.render_state.as_ref(),
                        doc,
                        cam,
                        rect,
                        store: assets,
                        doc_dir: doc_dir.as_deref(),
                        playing,
                    },
                );
                // The game's cursor image over the view (not while grabbed:
                // the cursor is hidden then).
                let image = play
                    .as_ref()
                    .filter(|s| s.mouse_icon && game_grab.is_none() && !s.game.menu_open)
                    .and_then(|s| s.cursor_image.clone());
                if let (Some(id), Some(at)) = (
                    image,
                    ctx.input(|i| i.pointer.latest_pos()).filter(|p| rect.contains(*p)),
                ) {
                    if gui.draw_cursor(&ctx, &painter, assets, doc_dir.as_deref(), &id, at) {
                        ctx.set_cursor_icon(egui::CursorIcon::None);
                    }
                }
            }
            // (The open in-game menu covers the game's GUIs: they get no input.)
            if playing && !play.as_ref().is_some_and(|s| s.game.menu_open) {
                let took = gui.input(&ctx, &resp, rect);
                gui.keyboard(&ctx, doc);
                gui.navigate(&ctx, doc);
                took
            } else {
                false
            }
        };
        // ParticleEmitters, Beams and Trails: triangles in the same pass, after
        // the GUIs (round 25).
        // (RUSTUDIO_NO_FX leaves them out, to measure their cost.)
        if self.renderer.is_some() && std::env::var_os("RUSTUDIO_NO_FX").is_none() {
            let doc_dir = self.doc.path.as_ref().and_then(|p| p.parent()).map(|p| p.to_path_buf());
            let Editor { gui, doc, cam, assets, effects, fixed_dt, play, scene, lighting, .. } = self;
            // (round 26: see-through parts dim the effects behind them; the
            // server's :Emit reaches the client's view)
            let glass = if effects.is_empty() { Vec::new() } else { effects.glass(scene, doc.revision, cam) };
            let server = play.as_ref().filter(|s| s.showing == crate::runtime::Side::Client).and_then(|s| {
                let id = s.other_doc_id()?;
                Some((id, &s.map.s2c))
            });
            let s2c = server.map(|(_, m)| move |r: Ref| m.get(&r).copied());
            let live = effects.frame(
                crate::effects::FrameIn {
                    doc,
                    cam,
                    rect,
                    dt: fixed_dt.unwrap_or(dt),
                    texture: &mut |id| gui.texture(&ctx, assets, doc_dir.as_deref(), id).map(|t| t.0),
                    light: &lighting.fx,
                    glass: &glass,
                    server: server.zip(s2c.as_ref()).map(|((id, _), f)| (id, f as &dyn Fn(Ref) -> Option<Ref>)),
                },
            );
            if live {
                ctx.request_repaint();
            }
        }
        // SurfaceGuis / BillboardGuis behind the world stay behind it (round 23).
        if let (Some(r), Some(rs)) = (self.renderer.as_mut(), self.render_state.as_ref()) {
            let _g = crate::bench::scope("gui_world_pass");
            // The same tone curve rule as the 3D view (lighting::tone_curve_on).
            let curve = crate::lighting::tone_curve_on(&self.doc).then_some(mech::TONE_CURVE);
            r.draw_world_gui(rs, &self.gui.world_batches, self.effects.drawn(), curve);
        }
        if let Some(s) = self
            .play
            .as_mut()
            .filter(|s| s.showing == crate::runtime::Side::Client)
        {
            s.game.gui_events.append(&mut self.gui.events);
            // (handed over only when it changed: the game keeps the last one)
            if self.gui.take_layout_change() {
                s.game.gui_layout = self.gui.layout.clone();
            }
        } else {
            self.gui.events.clear();
        }

        // Playing: clicks, keys and the hotbar go to the game.
        let in_game = self.game_viewport(ui, rect, &resp, gui_took);

        // The Terrain Editor's brush has the left button while one is picked.
        let brushing = !in_game && !alt && self.terrain_brush_ui(&ctx, rect, &resp, pointer, &painter);

        // -------------------------------------------------- select & gizmo
        let primary_drag = resp.dragged_by(PointerButton::Primary) && !alt && !in_game && !brushing;
        if resp.drag_started_by(PointerButton::Primary) && !alt && !in_game && !brushing {
            let start = ctx.input(|i| i.pointer.press_origin()).or(pointer);
            if let Some(p) = start {
                if let Some(h) = self.handle_at(&handles, p) {
                    self.start_drag(rect, h, p);
                } else if matches!(self.tool, Tool::Select | Tool::Move)
                    && self.start_part_drag(rect, p, ctrl || shift)
                {
                    // Studio: pressing on a part and dragging moves it (src/part_drag.rs).
                } else {
                    self.marquee = Some(p);
                    self.marquee_base = if ctrl || shift { self.doc.selection.clone() } else { Vec::new() };
                }
            }
        }
        if primary_drag {
            if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
                if self.drag.is_some() {
                    self.update_drag(rect, p);
                } else if self.part_drag.is_some() {
                    self.update_part_drag(rect, p);
                } else if let Some(start) = self.marquee {
                    // Studio highlights what the box holds while it is dragged.
                    // From 3 px of mouse travel (DragSelector.updateDrag).
                    if (p - start).length() >= mech::DRAG_SELECT_MIN_PX {
                        self.doc.selection = self.marquee_pick(rect, Rect::from_two_pos(start, p));
                    }
                }
            }
        }
        if resp.drag_stopped_by(PointerButton::Primary)
            || (!ctx.input(|i| i.pointer.primary_down())
                && (self.drag.is_some() || self.marquee.is_some() || self.part_drag.is_some()))
        {
            if self.drag.is_some() {
                self.finish_drag();
            }
            if self.part_drag.is_some() {
                self.finish_part_drag();
            }
            if let Some(start) = self.marquee.take() {
                if let Some(end) = ctx.input(|i| i.pointer.interact_pos()) {
                    self.marquee_select(rect, Rect::from_two_pos(start, end));
                }
            }
        }
        if resp.clicked_by(PointerButton::Primary) && !alt && !in_game && !brushing {
            if let Some(p) = pointer {
                // A GUI object under the cursor is picked first, as in Studio.
                let hit = self
                    .gui
                    .object_at(p)
                    .or_else(|| self.pick(rect, p, ctx.input(|i| i.modifiers.alt)));
                match (hit, ctrl || shift) {
                    (Some(r), true) => {
                        if let Some(pos) = self.doc.selection.iter().position(|s| *s == r) {
                            self.doc.selection.remove(pos);
                        } else {
                            self.doc.selection.push(r);
                        }
                    }
                    (Some(r), false) => self.select(vec![r]),
                    (None, false) => self.doc.selection.clear(),
                    (None, true) => {}
                }
            }
        }

        // ------------------------------------------------------- overlays
        self.scene.sync(&mut self.doc);
        self.draw_selection(&painter, rect);
        if let Some(h) = self.hover {
            for r in std::iter::once(h).chain(self.doc.dom.descendants_of(h).map(|i| i.referent()))
            {
                if let Some(sp) = self.scene.get(r) {
                    self.draw_box(
                        &painter,
                        rect,
                        &sp.xf,
                        sp.size,
                        Stroke::new(1.0, theme::HOVER_3D.gamma_multiply(0.8)),
                    );
                }
            }
        }
        let handles = self.handles(rect);
        // A grab point per handle: arrow/handle tips, and 45° along each ring
        // (where no other ring crosses it).
        self.gizmo_points = handles
            .iter()
            .map(|(h, pts)| {
                (
                    *h,
                    if h.tool == Tool::Rotate {
                        pts[pts.len() / 8]
                    } else {
                        pts[pts.len() - 1]
                    },
                )
            })
            .collect();
        for (h, pts) in &handles {
            let active =
                self.drag.as_ref().is_some_and(|d| d.handle == *h) || hover_handle == Some(*h);
            let c = if active {
                HOVER_COLOR
            } else {
                AXIS_COLORS[h.axis]
            };
            match h.tool {
                Tool::Move => {
                    painter.line_segment([pts[0], pts[1]], Stroke::new(3.0, c));
                    let d = (pts[1] - pts[0]).normalized();
                    let n = Vec2::new(-d.y, d.x);
                    painter.add(egui::Shape::convex_polygon(
                        vec![pts[1] + d * 12.0, pts[1] + n * 6.0, pts[1] - n * 6.0],
                        c,
                        Stroke::NONE,
                    ));
                }
                Tool::Scale => {
                    painter.line_segment([pts[0], pts[1]], Stroke::new(2.0, c));
                    painter.circle_filled(pts[1], 7.0, c);
                }
                Tool::Rotate => {
                    painter.add(egui::Shape::line(
                        pts.clone(),
                        Stroke::new(if active { 3.5 } else { 2.5 }, c),
                    ));
                }
                Tool::Select => {}
            }
        }
        if let (Some(start), Some(end)) = (self.marquee, pointer) {
            if (end - start).length() > mech::DRAG_SELECT_MIN_PX {
                let r = Rect::from_two_pos(start, end);
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(90, 170, 255, 30));
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.0, Color32::from_rgb(90, 170, 255)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        // Studio's Play view shows the game alone: no tool readout, axis
        // compass or border (tools/fidelity side-by-side shots).
        if !self.is_playing() {
            self.draw_viewport_hud(&painter, rect);
        }
    }

    pub(crate) fn draw_box(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        xf: &Xform,
        size: Vec3,
        stroke: Stroke,
    ) {
        let corners = box_corners(xf, size);
        let pts: Vec<Option<Pos2>> = corners.iter().map(|c| self.cam.project(rect, *c)).collect();
        for (a, b) in BOX_EDGES {
            if let (Some(pa), Some(pb)) = (pts[a], pts[b]) {
                painter.line_segment([pa, pb], stroke);
            }
        }
    }

    fn draw_selection(&self, painter: &egui::Painter, rect: Rect) {
        let stroke = Stroke::new(1.6, theme::SELECT);
        for r in self.selected_parts() {
            if let Some(sp) = self.scene.get(r) {
                self.draw_box(painter, rect, &sp.xf, sp.size, stroke);
            }
        }
        // Models also get a dimmer box around everything they contain.
        for &r in &self.doc.selection {
            if self.scene.get(r).is_none()
                && self
                    .doc
                    .dom
                    .descendants_of(r)
                    .any(|d| self.scene.get(d.referent()).is_some())
            {
                let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for d in self.doc.dom.descendants_of(r) {
                    if let Some(sp) = self.scene.get(d.referent()) {
                        let (a, b) = part_aabb(sp);
                        mn = mn.min(a);
                        mx = mx.max(b);
                    }
                }
                let xf = Xform {
                    pos: (mn + mx) * 0.5,
                    rot: Mat3::IDENTITY,
                };
                self.draw_box(
                    painter,
                    rect,
                    &xf,
                    mx - mn,
                    Stroke::new(1.0, theme::SELECT.gamma_multiply(0.6)),
                );
            }
        }
    }

    /// Releasing the box keeps what it held (with Shift / Ctrl, added to
    /// the selection it started from).
    fn marquee_select(&mut self, rect: Rect, area: Rect) {
        if area.size().length() < mech::DRAG_SELECT_MIN_PX {
            return;
        }
        let picked = self.marquee_pick(rect, area);
        self.select(picked);
    }

    /// The box's selection, as Studio's DraggerSchemaCore.beginBoxSelect and
    /// DragSelector.updateDrag / getSelectionFrustum: every unlocked part
    /// stands for its selectable (its top Model, else itself), and a
    /// selectable is in the box when its centre — a Model's bounding box
    /// centre, a part's position — is inside the four planes through the
    /// camera and the box's corners (the same as projecting inside the box,
    /// in front of the camera). Added to the base selection (Shift / Ctrl).
    fn marquee_pick(&self, rect: Rect, area: Rect) -> Vec<Ref> {
        let mut picked: Vec<Ref> = self.marquee_base.clone();
        // selectable -> (centre if a part, else the box of its parts)
        let mut order: Vec<Ref> = Vec::new();
        let mut centres: std::collections::HashMap<Ref, Result<Vec3, (Vec3, Vec3)>> = std::collections::HashMap::new();
        for sp in &self.scene.parts {
            if matches!(self.doc.prop(sp.r, "Locked"), Some(Variant::Bool(true))) {
                continue;
            }
            let t = self.selection_target(sp.r);
            if t == sp.r {
                order.push(t);
                centres.insert(t, Ok(sp.xf.pos));
                continue;
            }
            let m = sp.xf.rot;
            let half = (m.x_axis.abs() * sp.size.x + m.y_axis.abs() * sp.size.y + m.z_axis.abs() * sp.size.z) * 0.5;
            let (lo, hi) = (sp.xf.pos - half, sp.xf.pos + half);
            match centres.get_mut(&t) {
                Some(Err((a, b))) => {
                    *a = a.min(lo);
                    *b = b.max(hi);
                }
                _ => {
                    order.push(t);
                    centres.insert(t, Err((lo, hi)));
                }
            }
        }
        for t in order {
            let c = match centres[&t] {
                Ok(c) => c,
                Err((a, b)) => (a + b) * 0.5,
            };
            if self.cam.project(rect, c).is_some_and(|p| area.contains(p)) && !picked.contains(&t) {
                picked.push(t);
            }
        }
        picked
    }

    fn draw_viewport_hud(&self, painter: &egui::Painter, rect: Rect) {
        // Tool / drag readout, top-left.
        let mut text = format!(
            "{}  ·  {}",
            self.tool.label(),
            if self.local_space { "Local" } else { "World" }
        );
        if let Some(d) = &self.drag {
            let unit = if d.handle.tool == Tool::Rotate {
                "°"
            } else {
                " studs"
            };
            text = format!("{}: {:+.2}{unit}", d.handle.tool.label(), d.amount);
        }
        let font = egui::FontId::proportional(13.0);
        let galley = painter.layout_no_wrap(text, font, theme::TEXT);
        let pos = rect.min + Vec2::new(10.0, 10.0);
        painter.rect_filled(
            Rect::from_min_size(pos - Vec2::splat(5.0), galley.size() + Vec2::splat(10.0)),
            3.0,
            Color32::from_black_alpha(140),
        );
        painter.galley(pos, galley, theme::TEXT);

        // Axis compass, bottom-left.
        let c = Pos2::new(rect.min.x + 42.0, rect.max.y - 42.0);
        painter.circle_filled(c, 30.0, Color32::from_black_alpha(90));
        let view = self.cam.view();
        let mut axes: Vec<(usize, Vec3)> = (0..3)
            .map(|i| (i, view.transform_vector3(Vec3::AXES[i])))
            .collect();
        axes.sort_by(|a, b| a.1.z.total_cmp(&b.1.z));
        for (i, v) in axes {
            let end = c + Vec2::new(v.x, -v.y) * 24.0;
            painter.line_segment([c, end], Stroke::new(2.0, AXIS_COLORS[i]));
            painter.text(
                end,
                egui::Align2::CENTER_CENTER,
                ["X", "Y", "Z"][i],
                egui::FontId::proportional(11.0),
                AXIS_COLORS[i],
            );
        }
    }
}

impl LightingState {
    /// The skybox faces are still downloading or loading (the sky light in
    /// use is the previous one's until they arrive).
    pub fn sky_pending(&self) -> bool {
        self.sky_failed || self.sky_job.is_some()
    }

    /// Rebuild the light grid next frame, whatever the time since the last.
    pub fn refresh_lightgrid_now(&mut self) {
        self.lightgrid_dirty = true;
        self.lightgrid_at = None;
    }

    /// Parts or terrain changed since the light grid on the GPU was built.
    pub fn lightgrid_pending(&self) -> bool {
        self.lightgrid_dirty || self.lightgrid_moves || self.lightgrid_job.is_some()
    }

}

/// Lighting the viewport keeps between frames.
#[derive(Default)]
pub struct LightingState {
    pub lights: crate::lighting::LightCache,
    /// Skybox face ids in use ("" = none yet) and whether one failed to load.
    sky_key: String,
    sky_failed: bool,
    /// The sky being prepared on a worker (for that face key).
    #[allow(clippy::type_complexity)]
    sky_job: Option<(String, std::sync::mpsc::Receiver<Option<(u32, Vec<u8>, [Vec3; 6])>>)>,
    /// Studio's studs.dds was looked for (loaded when found).
    studs_tried: bool,
    /// Light grid (lightgrid.rs): where it is (chunks, Studio's camera
    /// follow), the grid last asked for (first voxel, document) and the one
    /// on the GPU, last rebuild, parts or terrain changed since, the
    /// terrain's occupancy for the current place of the grid, and the
    /// worker filling the next.
    lightgrid_place: Option<[i32; 3]>,
    lightgrid_key: Option<([i32; 3], u64)>,
    lightgrid_sun: [i32; 3],
    lightgrid_shown: Option<[i32; 3]>,
    lightgrid_at: Option<std::time::Instant>,
    lightgrid_dirty: bool,
    /// Only parts moved since (refreshed every LIGHTGRID_MOVES_REFRESH_S).
    lightgrid_moves: bool,
    /// Hull bounds per (mesh id, CollisionFidelity) at unit size.
    lightgrid_hull_boxes: std::collections::HashMap<(String, u32), Vec<(Vec3, Vec3)>>,
    lightgrid_terrain: Option<([i32; 3], Option<std::sync::Arc<Vec<u8>>>)>,
    lightgrid_lights: Vec<crate::render::LightRaw>,
    lightgrid_job: Option<std::sync::mpsc::Receiver<crate::lightgrid::LightGrid>>,
    /// Parts blocking local lights (lightocc.rs): the grid last asked for
    /// (first voxel, studs per voxel, document) and the one on the GPU,
    /// last rebuild, parts changed since, and the worker filling the next.
    lightocc_origin: Option<([i32; 3], f32, u64)>,
    lightocc_shown: Option<([i32; 3], f32)>,
    lightocc_at: Option<std::time::Instant>,
    lightocc_dirty: bool,
    lightocc_job: Option<std::sync::mpsc::Receiver<crate::lightocc::OccGrid>>,
    /// Lights drawn last frame (for the self-test and bench).
    pub drawn_lights: usize,
    /// Skybox irradiance per axis (+X, −X, +Y, −Y, +Z, −Z).
    pub sky_irr: [Vec3; 6],
    /// Studio's indoor environment (textures/sky/indoor512): its irradiance
    /// per axis, and whether its faces were looked for.
    pub indoor_irr: [Vec3; 6],
    indoor_tried: bool,
    /// The light particles, beams and trails take (round 26, effects.rs):
    /// this frame's terms, local lights, roof map and light grid.
    pub fx: crate::effects::SceneLight,
}

impl Editor {
    /// Studio's indoor environment cube (PrefilteredEnvIndoorTexture) and its
    /// irradiance (AmbientCube[6..11] before EnvironmentDiffuseScale), from
    /// PlatformContent textures/sky/indoor512_*.tex, once. Same face order and
    /// mirroring as the skybox (sync_sky).
    fn sync_indoor(&mut self) {
        if self.lighting.indoor_tried {
            return;
        }
        let (Some(renderer), Some(rs)) = (self.renderer.as_mut(), self.render_state.as_ref()) else {
            return;
        };
        self.lighting.indoor_tried = true;
        let mut imgs = Vec::new();
        for f in ["bk", "dn", "ft", "lf", "rt", "up"] {
            let id = format!("{}{f}.tex", mech::ENV_INDOOR_FACES);
            let Some(img) = crate::assets::resolve(&id, None)
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| crate::texture::decode(&b).ok())
            else {
                return;
            };
            imgs.push(img);
        }
        let size = imgs.iter().map(|i| i.width.min(i.height)).min().unwrap_or(1).clamp(1, 2048);
        let fit = |img: &crate::texture::Rgba| -> Vec<u8> {
            if img.width == size && img.height == size {
                return img.pixels.clone();
            }
            let Some(src) = image::RgbaImage::from_raw(img.width, img.height, img.pixels.clone()) else {
                return vec![0; (size * size * 4) as usize];
            };
            image::imageops::resize(&src, size, size, image::imageops::FilterType::Triangle).into_raw()
        };
        let flip = |mut px: Vec<u8>| -> Vec<u8> {
            let s = size as usize;
            for row in px.chunks_mut(s * 4) {
                for x in 0..s / 2 {
                    for c in 0..4 {
                        row.swap(x * 4 + c, (s - 1 - x) * 4 + c);
                    }
                }
            }
            px
        };
        let cube = [
            flip(fit(&imgs[3])),
            flip(fit(&imgs[4])),
            flip(fit(&imgs[5])),
            flip(fit(&imgs[1])),
            flip(fit(&imgs[0])),
            flip(fit(&imgs[2])),
        ];
        renderer.set_indoor(&rs.device, &rs.queue, size, &cube);
        self.lighting.indoor_irr = crate::lighting::sky_irradiance(&cube, size);
    }

    /// The skybox: the Sky's six faces (or Studio's default sky), uploaded
    /// when they change.
    fn sync_sky(&mut self) {
        // A prepared sky from the worker (decode, resize, flips and the
        // irradiance took ~145 ms on the main thread when Place A swapped skies).
        if let Some((key, rx)) = &self.lighting.sky_job {
            match rx.try_recv() {
                Ok(done) => {
                    let key = key.clone();
                    self.lighting.sky_job = None;
                    if key == self.lighting.sky_key {
                        self.apply_sky(done);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.lighting.sky_job = None,
            }
        }
        let faces = crate::lighting::sky_faces(&self.doc);
        let orient = crate::lighting::sky_orientation(&self.doc);
        let key = format!("{}|{:?}", faces.join("|"), orient.to_array());
        if key == self.lighting.sky_key && !self.lighting.sky_failed {
            return;
        }
        let doc_dir = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        // Every face's bytes first: nothing is decoded while one downloads.
        let mut bytes = Vec::new();
        for id in &faces {
            match self.assets.content(id, doc_dir.as_deref()) {
                crate::assets::Fetch::Ready(b) => bytes.push(b),
                // Still downloading: try again next frame.
                crate::assets::Fetch::Pending => {
                    self.lighting.sky_failed = true;
                    return;
                }
                crate::assets::Fetch::Unavailable(_) => break,
            }
        }
        self.lighting.sky_key = key.clone();
        self.lighting.sky_failed = false;
        if bytes.len() != 6 {
            self.lighting.sky_job = None;
            self.apply_sky(None);
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(prepare_sky(&bytes, orient));
        });
        self.lighting.sky_job = Some((key, rx));
    }

    /// Puts a prepared sky (or, with None, the gradient sky) on the GPU.
    fn apply_sky(&mut self, sky: Option<(u32, Vec<u8>, [Vec3; 6])>) {
        let (Some(renderer), Some(rs)) = (self.renderer.as_mut(), self.render_state.as_ref())
        else {
            return;
        };
        let Some((size, cube, irr)) = sky else {
            renderer.set_sky(&rs.device, &rs.queue, 0, None);
            // The gradient sky: its colours, squared.
            let top = crate::lighting::sq(mech::SKY_GRADIENT_TOP);
            let hor = crate::lighting::sq(mech::SKY_GRADIENT_HORIZON);
            let side = (top + hor) * 0.5;
            self.lighting.sky_irr = [side, side, top, hor * 0.5, side, side];
            return;
        };
        renderer.set_sky(&rs.device, &rs.queue, size, Some(&cube));
        self.lighting.sky_irr = irr;
        let up = self.lighting.sky_irr[2];
        self.info(format!(
            "Sky: {size}px faces, light from above {:.3} {:.3} {:.3}",
            up.x, up.y, up.z
        ));
    }

    /// Keeps Studio's light grid (lightgrid.rs) around the camera: it moves
    /// as Studio's does (a chunk of hysteresis), and is rebuilt when it
    /// moved or parts / terrain changed (at most every LIGHTGRID_REFRESH_S):
    /// the parts in it and the terrain are gathered here, voxelized and lit
    /// on a worker, and uploaded when done; the GPU keeps the last one (with
    /// its own origin) meanwhile. Returns its globals entry (w 0 = none yet).
    fn sync_lightgrid(&mut self, lights: &[crate::render::LightRaw], sun: Vec3, shadows: bool) -> [f32; 4] {
        let lights = if crate::lighting::soft_style(&self.doc) { lights } else { &[] };
        let st = &mut self.lighting;
        if bytemuck::cast_slice::<_, u8>(&st.lightgrid_lights) != bytemuck::cast_slice::<_, u8>(lights) {
            st.lightgrid_lights = lights.to_vec();
            st.lightgrid_dirty = true;
        }
        if let Some(rx) = &st.lightgrid_job {
            match rx.try_recv() {
                Ok(grid) => {
                    if let Ok(points) = std::env::var("RUSTUDIO_LIGHTGRID_DEBUG") {
                        for point in points.split(';') {
                            let p: Vec<f32> = point.split(',').filter_map(|x| x.parse().ok()).collect();
                            if p.len() != 3 { continue; }
                            let pos = Vec3::new(p[0], p[1], p[2]);
                            let v = (pos * 0.25).floor().as_ivec3().to_array();
                            eprintln!("LIGHTGRID camera={:?} origin={:?} pos={:?} sky={:?}", self.cam.pos, grid.origin, pos, grid.sky_at(v));
                        }
                    }
                    if let (Some(renderer), Some(rs)) = (self.renderer.as_mut(), self.render_state.as_ref()) {
                        renderer.set_lightgrid(&rs.queue, &grid.data);
                        renderer.set_localgrid(&rs.queue, &grid.local);
                        st.lightgrid_shown = Some(grid.origin);
                        st.fx.grid = Some(std::sync::Arc::new(grid));
                    }
                    st.lightgrid_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => st.lightgrid_job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let place = crate::lightgrid::follow(st.lightgrid_place, self.cam.pos);
        st.lightgrid_place = Some(place);
        let ch = crate::lightgrid::CHUNK;
        let origin = [place[0] * ch[0] as i32, place[1] * ch[1] as i32, place[2] * ch[2] as i32];
        // The sun channel follows the sun: rebuilt when it turns (as Studio
        // re-dirties its chunks' sun flag), quantised so a moving clock
        // doesn't rebuild every frame. No sun (GlobalShadows off, or no
        // light): the channel stays 255.
        let sun_key = if shadows { (sun * 512.0).round().as_ivec3().to_array() } else { [0; 3] };
        if st.lightgrid_sun != sun_key {
            st.lightgrid_sun = sun_key;
            st.lightgrid_dirty = true;
        }
        let key = (origin, self.doc.id);
        let moved = st.lightgrid_key != Some(key);
        let since = st.lightgrid_at.map(|t| t.elapsed().as_secs_f32());
        let stale = (st.lightgrid_dirty && since.is_none_or(|t| t >= mech::LIGHTGRID_REFRESH_S))
            || (st.lightgrid_moves && since.is_none_or(|t| t >= mech::LIGHTGRID_MOVES_REFRESH_S));
        if (moved || stale) && st.lightgrid_job.is_none() {
            let _b = crate::bench::scope("lightgrid");
            // The terrain's share, read again only when the grid moved or
            // the terrain changed.
            if st.lightgrid_terrain.as_ref().is_none_or(|(o, _)| *o != origin) {
                let plane = self
                    .terrain_drawn
                    .as_ref()
                    .and_then(|(_, _, v)| crate::lightgrid::terrain_plane(v, origin))
                    .map(std::sync::Arc::new);
                st.lightgrid_terrain = Some((origin, plane));
            }
            let terrain = st.lightgrid_terrain.as_ref().and_then(|(_, p)| p.clone());
            let [nx, ny, nz] = crate::lightgrid::dims();
            let lo = Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32) * 4.0;
            let hi = lo + Vec3::new(nx as f32, ny as f32, nz as f32) * 4.0;
            let mut pending_meshes = false;
            let hull_boxes = &mut st.lightgrid_hull_boxes;
            let prims: Vec<_> = self
                .scene
                .parts
                .iter()
                .zip(&self.scene.bounds)
                .filter(|(_, b)| {
                    b.x + b.w >= lo.x
                        && b.y + b.w >= lo.y
                        && b.z + b.w >= lo.z
                        && b.x - b.w <= hi.x
                        && b.y - b.w <= hi.y
                        && b.z - b.w <= hi.z
                })
                .filter_map(|(sp, _)| {
                    let p = sp.light?;
                    let mut boxes = None;
                    if p.kind == crate::lightgrid::Kind::Mesh {
                        let fid = crate::scene::enum_u32(self.doc.prop(sp.r, "CollisionFidelity"))
                            .unwrap_or(crate::collision::FIDELITY_DEFAULT);
                        if fid != crate::collision::FIDELITY_BOX {
                            if let Some(mesh) = &sp.mesh_asset {
                                let scale = |unit: &Vec<(Vec3, Vec3)>| {
                                    // Hull points scale per axis by Size, so their
                                    // unit-size bounds do too (Size is positive).
                                    unit.iter().map(|(lo, hi)| (*lo * sp.size, *hi * sp.size)).collect::<Vec<_>>()
                                };
                                if let Some(unit) = hull_boxes.get(&(mesh.id.clone(), fid)) {
                                    boxes = Some(scale(unit));
                                // Don't poison the physics cache with a missing download.
                                } else if self.meshes.state(&mesh.id).is_some_and(|s| s.is_ok()) {
                                    match crate::collision::hulls(&mesh.id, fid) {
                                        crate::collision::Lookup::Ready(hulls) => {
                                            let unit: Vec<(Vec3, Vec3)> = hulls.iter().filter_map(|h| {
                                                let first = *h.first()?;
                                                Some(h.iter().fold((first, first), |(lo, hi), v| (lo.min(*v), hi.max(*v))))
                                            }).collect();
                                            boxes = Some(scale(&unit));
                                            hull_boxes.insert((mesh.id.clone(), fid), unit);
                                        }
                                        crate::collision::Lookup::Pending => pending_meshes = true,
                                        crate::collision::Lookup::Unavailable => {}
                                    }
                                }
                            }
                        }
                    }
                    Some((p, boxes))
                })
                .collect();
            let local_lights = lights.to_vec();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut occ = crate::lightgrid::Occupancy::new(origin);
                for (p, boxes) in &prims {
                    occ.add_prim_boxes(p, boxes.as_deref());
                }
                if let Some(t) = terrain {
                    occ.add_plane(&t);
                }
                let mut grid = crate::lightgrid::skylight(&occ);
                if shadows {
                    crate::lightgrid::sunlight(&occ, &mut grid, sun);
                }
                grid.local = crate::lightgrid::local_lights(&occ, &local_lights);
                let _ = tx.send(grid);
            });
            st.lightgrid_job = Some(rx);
            st.lightgrid_key = Some(key);
            st.lightgrid_at = Some(std::time::Instant::now());
            st.lightgrid_dirty = pending_meshes;
            st.lightgrid_moves = false;
        }
        match st.lightgrid_shown {
            Some(o) => [o[0] as f32, o[1] as f32, o[2] as f32, mech::LIGHTGRID_NORMAL_OFFSET],
            None => [0.0; 4],
        }
    }

    /// Rebuilds the grid of what blocks local lights (lightocc.rs) when the
    /// camera moved it on, the LightingStyle changed its voxels, or parts
    /// changed (at most every LIGHTOCC_REFRESH_S): the parts in it are
    /// gathered here and the grid is filled on a worker; the GPU keeps the
    /// last one (with its own origin) until the new one arrives. Returns its
    /// globals entry (w 0 = no grid yet, or no shadowed light to block).
    fn sync_lightocc(&mut self, lights: &[crate::render::LightRaw]) -> [f32; 4] {
        if crate::lighting::soft_style(&self.doc) || !lights.iter().any(|l| l.extra[2] > 0.5) {
            return [0.0; 4];
        }
        let st = &mut self.lighting;
        if let Some(rx) = &st.lightocc_job {
            match rx.try_recv() {
                Ok(grid) => {
                    if let (Some(renderer), Some(rs)) = (self.renderer.as_mut(), self.render_state.as_ref()) {
                        renderer.set_lightocc(&rs.queue, &grid.data);
                        st.lightocc_shown = Some((grid.origin, grid.voxel));
                    }
                    st.lightocc_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => st.lightocc_job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let voxel = crate::lightocc::voxel_size(crate::lighting::soft_style(&self.doc));
        let origin = crate::lightocc::origin_for(self.cam.pos, self.cam.forward(), voxel);
        let key = (origin, voxel, self.doc.id);
        let moved = st.lightocc_origin != Some(key);
        let stale = st.lightocc_dirty
            && st
                .lightocc_at
                .is_none_or(|t| t.elapsed().as_secs_f32() >= mech::LIGHTOCC_REFRESH_S);
        if (moved || stale) && st.lightocc_job.is_none() {
            let _b = crate::bench::scope("lightocc");
            let hosts = st.lights.hosts(&mut self.doc);
            let (lo, hi) = crate::lightocc::Gathered::bounds(origin, voxel);
            let mut solids = Vec::new();
            for (sp, b) in self.scene.parts.iter().zip(&self.scene.bounds) {
                if sp.draw.alpha < mech::LIGHTOCC_MIN_ALPHA
                    || b.x + b.w < lo.x
                    || b.y + b.w < lo.y
                    || b.z + b.w < lo.z
                    || b.x - b.w > hi.x
                    || b.y - b.w > hi.y
                    || b.z - b.w > hi.z
                    || hosts.contains(&sp.r)
                    // MeshParts and unions don't block: their box is no guide
                    // to their shape (Place A's chapel went dark behind its meshes).
                    || sp.mesh_asset.is_some()
                    || sp.draw.custom_mesh.is_some()
                {
                    continue;
                }
                solids.push(crate::lightocc::Solid {
                    pos: sp.draw.xf.pos,
                    rot: sp.draw.xf.rot,
                    half: sp.draw.size * 0.5,
                    fill: crate::lightocc::fill_of(sp.draw.mesh),
                });
            }
            let gathered = crate::lightocc::Gathered { origin, voxel, solids };
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::lightocc::fill(gathered));
            });
            st.lightocc_job = Some(rx);
            st.lightocc_origin = Some(key);
            st.lightocc_at = Some(std::time::Instant::now());
            st.lightocc_dirty = false;
        }
        match st.lightocc_shown {
            Some((o, v)) => [o[0] as f32, o[1] as f32, o[2] as f32, v],
            None => [0.0; 4],
        }
    }

    /// Studio's legacy surface atlas, read once from the Studio install
    /// (PlatformContent/pc/textures/studs.dds, read only). Without it surfaces
    /// draw smooth.
    fn sync_studs(&mut self) {
        if std::mem::replace(&mut self.lighting.studs_tried, true) {
            return;
        }
        let Some(dir) = crate::default_scripts::studio_dir() else {
            return;
        };
        let path = dir.join("PlatformContent/pc/textures/studs.dds");
        let img = match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| crate::texture::decode(&b))
        {
            Ok(img) => img,
            Err(e) => {
                self.warn(format!(
                    "Legacy surfaces drawn smooth: {}: {e}",
                    path.display()
                ));
                return;
            }
        };
        if let (Some(renderer), Some(rs)) = (self.renderer.as_mut(), self.render_state.as_ref()) {
            renderer.set_studs(&rs.device, &rs.queue, img.width, img.height, &img.pixels);
        }
    }

    /// This frame's lighting: shader constants, local lights, and whether
    /// to draw the sun's shadow map.
    pub fn frame_lighting(
        &mut self,
        rect: Rect,
    ) -> (crate::render::Globals, Vec<crate::render::LightRaw>, bool, [crate::sun_shadow::Cascade; crate::sun_shadow::CASCADES]) {
        {
            let _s = crate::bench::scope("lt_sky");
            self.sync_sky();
        }
        {
            let _s = crate::bench::scope("lt_indoor");
            self.sync_indoor();
        }
        self.sync_studs();
        let planes = crate::render::frustum(self.cam.view_proj(rect));
        let lt_gather = crate::bench::scope("lt_gather");
        let lights = self.lighting.lights.gather(&mut self.doc, &self.cam, &|c, r| {
            crate::render::sphere_visible(&planes, c, r)
        });
        drop(lt_gather);
        self.lighting.drawn_lights = lights.len();
        let skybox = self.renderer.as_ref().is_some_and(|r| r.sky_size > 0);
        let mut fl = crate::lighting::globals(
            &self.doc,
            &self.cam,
            rect,
            lights.len(),
            skybox,
            &self.lighting.sky_irr,
            &self.lighting.indoor_irr,
        );
        fl.globals.lightgrid = self.sync_lightgrid(&lights, fl.sun_true, fl.shadows);
        fl.globals.lightocc = {
            let _s = crate::bench::scope("lt_occ");
            self.sync_lightocc(&lights)
        };

        {
            // The effects' light (effects::SceneLight): the same terms.
            let g = &fl.globals;
            let v3 = |a: [f32; 4]| Vec3::new(a[0], a[1], a[2]);
            let fx = &mut self.lighting.fx;
            fx.ambient = v3(g.ambient);
            fx.outdoor = v3(g.outdoor);
            // (at night the light is the moon, which lights particles
            // brighter than parts: effects::SceneLight::sun)
            let moon = g.fog[3] < 0.5;
            fx.sun = v3(g.sun_color)
                * g.sun_dir[3]
                * if moon { mech::FX_MOON_PER_BRIGHTNESS / mech::MOON_IRRADIANCE_PER_BRIGHTNESS } else { 1.0 };
            fx.soft = g.sun_color[3] > 0.5;
            fx.env = g.env.iter().map(|e| v3(*e)).sum::<Vec3>() / 6.0 * v3(g.sky_tint);
            fx.lights.clone_from(&lights);
        }
        // The camera in the Terrain's water: Studio fogs the view toward the
        // lit WaterColor (fog.z 2 = the underwater curve, docs/MECHANICS.md
        // "Underwater"); the shader lights the colour.
        if let Some((_, _, v)) = self.terrain_drawn.as_ref() {
            if v.water_at(self.cam.pos.to_array()) {
                let rgb = self.terrain_water_look.map(|l| l.0).unwrap_or(mech::WATER_COLOR_DEFAULT);
                fl.globals.fog[0] = mech::UNDERWATER_FOG[0];
                fl.globals.fog[1] = mech::UNDERWATER_FOG[1];
                fl.globals.fog[2] = 2.0;
                fl.globals.fog_color = crate::lighting::sq(rgb).extend(1.0).to_array();
            }
        }
        // The sun's shadow cascades (sun_shadow.rs), around this camera.
        let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
        let tan_y = (self.cam.fov_deg.to_radians() * 0.5).tan();
        let cascades = crate::sun_shadow::cascades(
            self.cam.pos,
            self.cam.forward(),
            [tan_y * aspect, tan_y],
            fl.sun,
            fl.shadow_softness,
        );
        (fl.globals.sun_casc, fl.globals.sun_sphere, fl.globals.sun_rect) = crate::sun_shadow::uniforms(&cascades);
        (fl.globals, lights, fl.shadows, cascades)
    }
}

/// What the viewport last handed the renderer for each part.
#[derive(Default)]
pub struct DrawCache {
    /// The part's CFrame when its draws were made (moves are applied relative to it).
    pub xf: std::collections::HashMap<Ref, Xform>,
    /// Parts with a mesh or texture still loading.
    pub waiting: std::collections::HashSet<Ref>,
    /// Per part: (loaded mesh, textured, decals, clothed) for the self-test.
    flags: std::collections::HashMap<Ref, [u32; 4]>,
    totals: [usize; 4],
}

impl DrawCache {
    fn set(&mut self, r: Ref, xf: Xform, flags: [u32; 4], waiting: bool) {
        if let Some(old) = self.flags.insert(r, flags) {
            for i in 0..4 {
                self.totals[i] -= old[i] as usize;
            }
        }
        for i in 0..4 {
            self.totals[i] += flags[i] as usize;
        }
        self.xf.insert(r, xf);
        if waiting {
            self.waiting.insert(r);
        } else {
            self.waiting.remove(&r);
        }
    }

    fn forget(&mut self, r: Ref) {
        if let Some(old) = self.flags.remove(&r) {
            for i in 0..4 {
                self.totals[i] -= old[i] as usize;
            }
        }
        self.xf.remove(&r);
        self.waiting.remove(&r);
    }

    pub fn totals(&self) -> [usize; 4] {
        self.totals
    }
}

/// Decodes the six skybox faces (Bk, Dn, Ft, Lf, Rt, Up) into the cube
/// rustudio draws, with its irradiance; None if any face doesn't decode.
fn prepare_sky(bytes: &[std::sync::Arc<Vec<u8>>], orient: Vec3) -> Option<(u32, Vec<u8>, [Vec3; 6])> {
    let mut imgs = Vec::new();
    for b in bytes {
        imgs.push(crate::texture::decode(b).ok()?);
    }
        // All faces square at the smallest face's size.
        let size = imgs
            .iter()
            .map(|i| i.width.min(i.height))
            .min()
            .unwrap_or(1)
            .clamp(1, 2048);
        let fit = |img: &crate::texture::Rgba| -> Vec<u8> {
            if img.width == size && img.height == size {
                return img.pixels.clone();
            }
            let Some(src) = image::RgbaImage::from_raw(img.width, img.height, img.pixels.clone())
            else {
                return vec![255; (size * size * 4) as usize];
            };
            image::imageops::resize(&src, size, size, image::imageops::FilterType::Triangle)
                .into_raw()
        };
        // Sky order Bk, Dn, Ft, Lf, Rt, Up → cube +X Lf, −X Rt, +Y Up, −Y Dn, +Z Bk, −Z Ft.
        // Round 26: Studio's sky is this cube mirrored left to right (x → −x):
        // looking down its SkyboxDn shows flipped side to side, looking at
        // SkyboxFt too (tools/fidelity skydown), and at 45° between faces it
        // matches with Lf and Rt swapped as well. Unmirrored, the Dn face's
        // brighter edge met the side faces at 45° down as a lighter band
        // (terrain T9). So each face is flipped horizontally, Rt goes to −X
        // and Lf to +X.
        let flip = |mut px: Vec<u8>| -> Vec<u8> {
            let s = size as usize;
            for row in px.chunks_mut(s * 4) {
                for x in 0..s / 2 {
                    for c in 0..4 {
                        row.swap(x * 4 + c, (s - 1 - x) * 4 + c);
                    }
                }
            }
            px
        };
        // The top face turned 90° clockwise and the bottom 90° counter-clockwise
        // (2026-11-05, user: both were a quarter turn off Studio's).
        let rot_cw = |px: Vec<u8>| -> Vec<u8> {
            let s = size as usize;
            let mut out = vec![0u8; px.len()];
            for y in 0..s {
                for x in 0..s {
                    let (nx, ny) = (s - 1 - y, x);
                    out[(ny * s + nx) * 4..(ny * s + nx) * 4 + 4].copy_from_slice(&px[(y * s + x) * 4..(y * s + x) * 4 + 4]);
                }
            }
            out
        };
        let rot_ccw = |px: Vec<u8>| -> Vec<u8> {
            let s = size as usize;
            let mut out = vec![0u8; px.len()];
            for y in 0..s {
                for x in 0..s {
                    let (nx, ny) = (y, s - 1 - x);
                    out[(ny * s + nx) * 4..(ny * s + nx) * 4 + 4].copy_from_slice(&px[(y * s + x) * 4..(y * s + x) * 4 + 4]);
                }
            }
            out
        };
        let cube = [
            flip(fit(&imgs[3])),
            flip(fit(&imgs[4])),
            rot_cw(flip(fit(&imgs[5]))),
            rot_ccw(flip(fit(&imgs[1]))),
            flip(fit(&imgs[0])),
            flip(fit(&imgs[2])),
        ];
        let cube = orient_sky(cube, size, orient);
        let irr = crate::lighting::sky_irradiance(&cube, size);
    Some((size, crate::render::cube_mips(size, &cube), irr))
}

/// Turns a sky cube (+X, −X, +Y, −Y, +Z, −Z faces, wgpu's layout) by
/// Sky.SkyboxOrientation (degrees), as Studio does: each texel toward d takes
/// the unturned sky toward R⁻¹·d, R = Rz·Rx·Ry (Roblox's axis rotations),
/// bilinear within a face. Quarter turns land on texel centres, so they are
/// an exact reshuffle. Done here once, so the drawn sky, its reflections and
/// its irradiance all turn together.
fn orient_sky(cube: [Vec<u8>; 6], size: u32, orient: Vec3) -> [Vec<u8>; 6] {
    if orient == Vec3::ZERO {
        return cube;
    }
    // Measured (docs/MECHANICS.md "Skybox orientation"): one axis at a time
    // turns the sky by that axis's Roblox rotation; (30, 60, 20) matches only
    // with Z·X·Y (Y·X·Z, CFrame.fromOrientation's order, is 50 levels off).
    let r = glam::Mat3::from_rotation_z(orient.z.to_radians())
        * glam::Mat3::from_rotation_x(orient.x.to_radians())
        * glam::Mat3::from_rotation_y(orient.y.to_radians());
    let inv = r.transpose();
    let s = size as usize;
    let fetch = |f: usize, x: usize, y: usize| -> [f32; 4] {
        let o = (y * s + x) * 4;
        let p = &cube[f][o..o + 4];
        [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]
    };
    let sample = |d: Vec3| -> [u8; 4] {
        let a = d.abs();
        // Direction → face and (u, v) in −1..1: the inverse of
        // lighting::sky_irradiance's texel → direction table.
        let (f, u, v) = if a.x >= a.y && a.x >= a.z {
            if d.x > 0.0 { (0, -d.z / a.x, -d.y / a.x) } else { (1, d.z / a.x, -d.y / a.x) }
        } else if a.y >= a.z {
            if d.y > 0.0 { (2, d.x / a.y, d.z / a.y) } else { (3, d.x / a.y, -d.z / a.y) }
        } else if d.z > 0.0 {
            (4, d.x / a.z, -d.y / a.z)
        } else {
            (5, -d.x / a.z, -d.y / a.z)
        };
        let fx = ((u + 1.0) * 0.5 * size as f32 - 0.5).clamp(0.0, size as f32 - 1.0);
        let fy = ((v + 1.0) * 0.5 * size as f32 - 0.5).clamp(0.0, size as f32 - 1.0);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(s - 1), (y0 + 1).min(s - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let (p00, p10, p01, p11) = (fetch(f, x0, y0), fetch(f, x1, y0), fetch(f, x0, y1), fetch(f, x1, y1));
        let mut out = [0u8; 4];
        for c in 0..4 {
            let top = p00[c] + (p10[c] - p00[c]) * tx;
            let bot = p01[c] + (p11[c] - p01[c]) * tx;
            out[c] = (top + (bot - top) * ty).round().clamp(0.0, 255.0) as u8;
        }
        out
    };
    std::array::from_fn(|f| {
        let mut px = vec![0u8; s * s * 4];
        for y in 0..s {
            for x in 0..s {
                let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                let d = match f {
                    0 => Vec3::new(1.0, -v, -u),
                    1 => Vec3::new(-1.0, -v, u),
                    2 => Vec3::new(u, 1.0, v),
                    3 => Vec3::new(u, -1.0, -v),
                    4 => Vec3::new(u, -v, 1.0),
                    _ => Vec3::new(-u, -v, -1.0),
                };
                let o = (y * s + x) * 4;
                px[o..o + 4].copy_from_slice(&sample(inv * d));
            }
        }
        px
    })
}
