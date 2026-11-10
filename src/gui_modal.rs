//! GuiButton.Modal: while a Modal button is shown, the game's mouse lock
//! (MouseBehavior LockCenter / LockCurrentPosition: first person, shift
//! lock) lets go of the cursor, unless the right mouse button is down
//! (Studio's API docs). UserInputService.MouseBehavior keeps the game's
//! value; only the effective lock changes (viewport.rs update_game_mouse).
//!
//! As Studio does it:
//! - the PlayerGui walks its children, keeps the layer collectors and asks
//!   each.
//! - a layer collector answers false when it isn't Enabled; otherwise true
//!   if any Modal button registered with it is shown: with
//!   FFlagOnlyVisibleFramesAllowModal (on by default) by the full test
//!   below, else by the button's own visibility.
//! - shown: the button and its ancestors are visible, its layer collector
//!   exists and is Enabled, its absolute rect has an area, every GuiObject
//!   ancestor up to the collector with ClipsDescendants clips it and leaves
//!   some area, the collector's own clip too, and what's left lies on the
//!   screen.
//!
//! Here the GUI walk's layout (gui.rs `layout`, AbsolutePosition space) gives
//! the rects; an object the walk didn't lay out isn't drawn, so isn't shown.

use std::collections::HashMap;

use rbx_dom_weak::types::{Ref, Variant};

use crate::doc::{self, Document};
use crate::gui::GuiBox;

fn flag(d: &Document, r: Ref, name: &str, default: bool) -> bool {
    match d.prop(r, name) {
        Some(Variant::Bool(b)) => b,
        _ => default,
    }
}

/// (min, max) corners of a laid-out object.
fn rect(layout: &HashMap<Ref, GuiBox>, r: Ref) -> Option<([f32; 2], [f32; 2])> {
    let b = layout.get(&r)?;
    Some((b.pos, [b.pos[0] + b.size[0], b.pos[1] + b.size[1]]))
}

fn clip(a: ([f32; 2], [f32; 2]), b: ([f32; 2], [f32; 2])) -> Option<([f32; 2], [f32; 2])> {
    let lo = [a.0[0].max(b.0[0]), a.0[1].max(b.0[1])];
    let hi = [a.1[0].min(b.1[0]), a.1[1].min(b.1[1])];
    (hi[0] - lo[0] > 0.0 && hi[1] - lo[1] > 0.0).then_some((lo, hi))
}

/// One Modal button under the layer collector `lc`, as Studio judges it.
fn shown(d: &Document, layout: &HashMap<Ref, GuiBox>, lc: Ref, button: Ref) -> bool {
    if !flag(d, button, "Visible", true) {
        return false;
    }
    let Some(mut area) = rect(layout, button).and_then(|r| clip(r, r)) else { return false };
    let mut p = d.dom.get_by_ref(button).map(|i| i.parent()).unwrap_or_else(Ref::none);
    while p != lc {
        let Some(inst) = d.dom.get_by_ref(p) else { return false };
        if doc::is_a(&inst.class, "GuiObject") {
            if !flag(d, p, "Visible", true) {
                return false;
            }
            if flag(d, p, "ClipsDescendants", false) {
                match rect(layout, p).and_then(|r| clip(area, r)) {
                    Some(a) => area = a,
                    None => return false,
                }
            }
        }
        p = inst.parent();
    }
    // The collector's own canvas is the screen it draws on.
    rect(layout, lc).and_then(|r| clip(area, r)).is_some()
}

/// Whether the LocalPlayer's PlayerGui shows a Modal GuiButton now.
pub fn modal_shown(d: &Document, layout: &HashMap<Ref, GuiBox>) -> bool {
    let Some(pg) = d.find_child(d.dom.root_ref(), "Players").and_then(|p| match d.prop(p, "LocalPlayer") {
        Some(Variant::Ref(lp)) if d.exists(lp) => d.find_child(lp, "PlayerGui"),
        _ => None,
    }) else {
        return false;
    };
    let Some(pgi) = d.dom.get_by_ref(pg) else { return false };
    for &lc in pgi.children() {
        let Some(li) = d.dom.get_by_ref(lc) else { continue };
        if !doc::is_a(&li.class, "LayerCollector") || !flag(d, lc, "Enabled", true) {
            continue;
        }
        for x in d.dom.descendants_of(lc) {
            if doc::is_a(&x.class, "GuiButton")
                && matches!(x.properties.get(&rbx_dom_weak::ustr("Modal")), Some(Variant::Bool(true)))
                && shown(d, layout, lc, x.referent())
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::InstanceBuilder;

    fn gb(pos: [f32; 2], size: [f32; 2]) -> GuiBox {
        GuiBox { pos, size, rot: 0.0, text: None, canvas: None }
    }

    /// Studio's rules: Enabled collector, the Visible chain, an area left
    /// after ClipsDescendants, on the collector's screen, Modal set.
    #[test]
    fn modal_button_shown_as_studio_judges_it() {
        let mut d = Document::new_place();
        let root = d.dom.root_ref();
        let players = d.find_child(root, "Players").unwrap_or_else(|| d.insert(root, InstanceBuilder::new("Players")));
        let lp = d.insert(players, InstanceBuilder::new("Player").with_name("P"));
        d.dom.get_by_ref_mut(players).unwrap().properties.insert(rbx_dom_weak::ustr("LocalPlayer"), Variant::Ref(lp));
        let pg = d.insert(lp, InstanceBuilder::new("PlayerGui").with_name("PlayerGui"));
        let sg = d.insert(pg, InstanceBuilder::new("ScreenGui").with_name("G"));
        let frame = d.insert(sg, InstanceBuilder::new("Frame").with_name("F"));
        let b = d.insert(frame, InstanceBuilder::new("TextButton").with_name("B").with_property("Modal", true));
        let mut layout = HashMap::new();
        layout.insert(sg, gb([0.0, 0.0], [800.0, 600.0]));
        layout.insert(frame, gb([100.0, 100.0], [200.0, 200.0]));
        layout.insert(b, gb([120.0, 120.0], [50.0, 20.0]));
        let set = |d: &mut Document, r: Ref, k: &str, v: Variant| {
            d.dom.get_by_ref_mut(r).unwrap().properties.insert(rbx_dom_weak::ustr(k), v);
        };
        assert!(modal_shown(&d, &layout), "a visible Modal button");
        set(&mut d, b, "Modal", Variant::Bool(false));
        assert!(!modal_shown(&d, &layout), "Modal off");
        set(&mut d, b, "Modal", Variant::Bool(true));
        set(&mut d, frame, "Visible", Variant::Bool(false));
        assert!(!modal_shown(&d, &layout), "an invisible ancestor");
        set(&mut d, frame, "Visible", Variant::Bool(true));
        set(&mut d, sg, "Enabled", Variant::Bool(false));
        assert!(!modal_shown(&d, &layout), "a disabled ScreenGui");
        set(&mut d, sg, "Enabled", Variant::Bool(true));
        // Clipped away by a ClipsDescendants frame it lies outside of.
        layout.insert(b, gb([400.0, 120.0], [50.0, 20.0]));
        assert!(modal_shown(&d, &layout), "outside a frame that doesn't clip");
        set(&mut d, frame, "ClipsDescendants", Variant::Bool(true));
        assert!(!modal_shown(&d, &layout), "clipped by its frame");
        set(&mut d, frame, "ClipsDescendants", Variant::Bool(false));
        // Off the screen, or with no size.
        layout.insert(b, gb([900.0, 120.0], [50.0, 20.0]));
        assert!(!modal_shown(&d, &layout), "off screen");
        layout.insert(b, gb([120.0, 120.0], [0.0, 20.0]));
        assert!(!modal_shown(&d, &layout), "no width");
        // Not laid out: not drawn.
        layout.remove(&b);
        assert!(!modal_shown(&d, &layout), "not laid out");
    }
}
