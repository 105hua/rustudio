//! DragDetector drags in Play: the mouse (play_game's pointer and button
//! events) drives domains/world_drag.rs on the server, and the dragging
//! client fires its own DragStart / DragContinue / DragEnd first, with the
//! same arguments (Studio 0.705: the client's events come before the
//! server's). DragContinue goes out once per cursor move.

use std::cell::RefCell;

use super::*;
use crate::runtime::domains::world::drag;

#[derive(Clone, Copy)]
struct Active {
    det: Ref,
    /// The server took the drag (permission); the client fires either way.
    server: bool,
    part: Ref,
    last: (Vec3, Vec3),
    view: Xform,
}

thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
}

/// The face normal of `part`'s box nearest the point `p`.
fn face_normal(d: &Document, part: Ref, p: Vec3) -> Vec3 {
    let Some(x) = crate::scene::xform_of(d, part) else { return Vec3::Y };
    let h = crate::scene::vec3_of(d.prop(part, "Size")).unwrap_or(Vec3::ONE) * 0.5;
    let l = x.rot.transpose() * (p - x.pos) / h.max(Vec3::splat(1e-4));
    let a = l.abs();
    if a.x >= a.y && a.x >= a.z {
        x.rot.x_axis * l.x.signum()
    } else if a.y >= a.z {
        x.rot.y_axis * l.y.signum()
    } else {
        x.rot.z_axis * l.z.signum()
    }
}

impl Editor {
    fn drag_c(&self, server: Ref) -> Option<Ref> {
        self.play.as_ref()?.map.s2c.get(&server).copied()
    }

    fn drag_s(&self, side: Side, r: Ref) -> Option<Ref> {
        match side {
            Side::Server => Some(r),
            Side::Client => self.play.as_ref()?.map.c2s.get(&r).copied(),
        }
    }

    /// The client camera's CFrame (the server is in front, the client's
    /// DataModel is `other_doc`).
    fn client_view(&self, fallback: (Vec3, Vec3)) -> Xform {
        let cam = self.play.as_ref().and_then(|s| s.other_doc.as_ref()).and_then(|d| {
            let ws = d.workspace()?;
            let c = match d.prop(ws, "CurrentCamera")? {
                Variant::Ref(c) if c.is_some() => c,
                _ => return None,
            };
            crate::scene::xform_of(d, c)
        });
        cam.unwrap_or_else(|| crate::ltypes::look_at(fallback.0, fallback.0 + fallback.1, Vec3::Y))
    }

    fn mode_key_down(&self) -> bool {
        self.play
            .as_ref()
            .and_then(|s| s.client.as_ref())
            .and_then(|c| c.world.as_ref())
            .is_some_and(|w| w.keys_down.contains("LeftControl"))
    }

    /// Fires a drag event on the client copy of `det`.
    fn drag_client_event(&mut self, det: Ref, ev: &str, ray: &crate::ltypes::LRay, view: Xform, hit: Option<(Xform, Ref)>, mode: bool) {
        let (Some(cd), Some(cp)) = (self.drag_c(det), self.play.as_ref().and_then(|s| self.drag_c(s.player))) else {
            return;
        };
        let hit = hit.and_then(|(f, p)| self.drag_c(p).map(|cp| (f, cp)));
        let ev = ev.to_string();
        let ray = ray.clone();
        self.with_side(Side::Client, |rt, _, _| {
            if let Ok(args) = drag::event_args(&rt.lua, &ev, cp, &ray, view, hit, mode) {
                rt.fire(cd, &ev, args);
            }
        });
    }

    /// Mouse button 1 presses, moves and releases for DragDetectors.
    pub(super) fn drag_detectors(&mut self, buttons: &[(u8, bool)]) {
        let Some((player, pointer)) = self.play.as_ref().map(|s| (s.player, s.game.last_pointer)) else {
            ACTIVE.with(|a| *a.borrow_mut() = None);
            return;
        };
        let pressed = buttons.iter().any(|(b, d)| *b == 1 && *d);
        let released = buttons.iter().any(|(b, d)| *b == 1 && !*d);
        let mode = self.mode_key_down();
        let active = ACTIVE.with(|a| *a.borrow());
        // A drag the server dropped (detector destroyed, session restarted).
        if let Some(a) = active {
            if !self.doc.exists(a.det) {
                ACTIVE.with(|x| *x.borrow_mut() = None);
            }
        }
        let active = ACTIVE.with(|a| *a.borrow());
        match (active, pointer) {
            (None, Some(p)) if pressed => {
                let Some((hit_ref, hit)) = p.hit else { return };
                let Some(part) = self.drag_s(p.side, hit_ref) else { return };
                let Some(det) = drag::detector_for(&self.doc, part) else { return };
                let reach = crate::scene::f32_of(self.doc.prop(det, "MaxActivationDistance")).unwrap_or(32.0);
                if let (Some(at), Some(x)) = (self.character_pos_for_drag(), crate::scene::xform_of(&self.doc, part)) {
                    if (x.pos - at).length() > reach {
                        return;
                    }
                }
                let normal = face_normal(&self.doc, part, hit);
                let view = self.client_view((p.origin, p.dir));
                let ray = drag::cursor_ray(p.origin, p.dir);
                // The client fires its events whatever the server's permission
                // says (Studio: a refused drag still fires on the client).
                self.drag_client_event(det, "DragStart", &ray, view, Some((drag::hit_frame(hit, normal), part)), mode);
                let started = self
                    .with_side(Side::Server, |rt, _, _| drag::start(&rt.lua, part, player, &ray, view, hit, normal, mode).ok().flatten())
                    .flatten();
                ACTIVE.with(|a| *a.borrow_mut() = Some(Active { det, server: started.is_some(), part, last: (p.origin, p.dir), view }));
            }
            (Some(a), p) => {
                if let Some(p) = p {
                    let view = self.client_view((p.origin, p.dir));
                    let moved = (p.origin - a.last.0).length() > 1e-5
                        || (p.dir.normalize_or_zero() - a.last.1.normalize_or_zero()).length() > 1e-6
                        || (view.pos - a.view.pos).length() > 1e-5;
                    if moved {
                        let ray = drag::cursor_ray(p.origin, p.dir);
                        self.drag_client_event(a.det, "DragContinue", &ray, view, None, mode);
                        if a.server {
                            self.with_side(Side::Server, |rt, _, _| drag::motion(&rt.lua, &ray, view, mode));
                        }
                        ACTIVE.with(|x| *x.borrow_mut() = Some(Active { last: (p.origin, p.dir), view, ..a }));
                    }
                }
                if released {
                    let ray = drag::cursor_ray(a.last.0, a.last.1);
                    self.drag_client_event(a.det, "DragEnd", &ray, a.view, None, mode);
                    if a.server {
                        self.with_side(Side::Server, |rt, _, _| drag::end(&rt.lua));
                    }
                    ACTIVE.with(|x| *x.borrow_mut() = None);
                    let _ = a.part;
                }
            }
            _ => {}
        }
    }

    /// The player's character position (for MaxActivationDistance).
    fn character_pos_for_drag(&self) -> Option<Vec3> {
        self.play.as_ref()?.character.as_ref().filter(|c| !c.dead).map(|c| c.body.pos)
    }
}
