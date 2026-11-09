//! Self-test step for MeshPart collision: two anchored L-shaped MeshParts,
//! one CollisionFidelity Box and one PreciseConvexDecomposition, with a ball
//! dropped into each one's empty corner. The box keeps the ball on top; the
//! decomposition lets it fall into the corner, as Roblox's would.

use glam::Vec3;
use rbx_dom_weak::types::{CFrame, Color3uint8, ContentId, Enum, Vector3};
use rbx_dom_weak::InstanceBuilder;

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::{new_part, Document};

const MESH: &str = "rbxtemp://rustudio/selftest/l-prism.obj";
/// Part size: the L stands 8 studs tall; its empty corner is x, y in 0..4
/// from the part's centre, open at the top.
const SIZE: f32 = 8.0;

impl SelfTest {
    /// Phases 72..=73.
    pub(super) fn tick_collision(&mut self, f: u64, ed: &mut Editor) -> bool {
        use crate::collision::{self as col, Lookup};
        if self.r4.phase == 72 {
            crate::assets::register_generated(MESH, col::l_prism_obj().into_bytes());
            let mut doc = Document::new_place();
            let ws = doc.workspace().expect("workspace");
            for (name, x, fid) in [
                ("BoxL", -10.0f32, col::FIDELITY_BOX),
                ("PreciseL", 10.0, col::FIDELITY_PRECISE),
            ] {
                let b = InstanceBuilder::new("MeshPart")
                    .with_name(name)
                    .with_property("Anchored", true)
                    .with_property("Size", Vector3::new(SIZE, SIZE, SIZE))
                    .with_property(
                        "CFrame",
                        CFrame::new(
                            Vector3::new(x, SIZE * 0.5 + 0.01, 0.0),
                            CFrame::identity().orientation,
                        ),
                    )
                    .with_property("Color", Color3uint8::new(160, 160, 170))
                    .with_property("MeshId", ContentId::from(MESH))
                    .with_property("CollisionFidelity", Enum::from_u32(fid));
                doc.insert(ws, b);
                let ball = new_part(
                    &format!("{name}Ball"),
                    1,
                    Vec3::new(x + 2.0, SIZE + 1.5, 0.0),
                    Vec3::splat(2.0),
                    [220, 60, 60],
                )
                .with_property("Anchored", false);
                doc.insert(ws, ball);
            }
            ed.set_document(doc);
            self.r6 = super::R6Test {
                started: f,
                ..Default::default()
            };
            self.r4.phase = 73;
            return false;
        }
        // Wait for the decomposition (background threads), then simulate.
        let t = f.saturating_sub(self.r6.started);
        if t == 1 {
            let _ = col::hulls(MESH, col::FIDELITY_PRECISE);
        }
        if !ed.is_playing() {
            let ready = matches!(col::hulls(MESH, col::FIDELITY_PRECISE), Lookup::Ready(_));
            if ready || t > 3000 {
                ed.start_play(crate::runtime::RunMode::Run);
                self.r6.started = f;
            }
            return false;
        }
        if t < 180 {
            return false;
        }
        let y = |ed: &Editor, n: &str| {
            ed.doc
                .workspace()
                .and_then(|ws| ed.doc.find_named(ws, n))
                .and_then(|r| crate::scene::xform_of(&ed.doc, r))
                .map(|x| x.pos.y)
                .unwrap_or(f32::NAN)
        };
        let (on_box, on_precise) = (y(ed, "BoxLBall"), y(ed, "PreciseLBall"));
        let pieces = match col::hulls(MESH, col::FIDELITY_PRECISE) {
            Lookup::Ready(h) => h.len(),
            _ => 0,
        };
        ed.stop_play();
        // Box: the ball rests on the top (8 + radius 1); decomposition: on the
        // corner's floor at the part's mid-height (4 + 1).
        let ok = (on_box - 9.0).abs() < 0.6 && (on_precise - 5.0).abs() < 0.6 && pieces >= 2;
        self.steps.push(Step {
            step: "mesh-collision",
            passed: ok,
            detail: format!(
                "anchored L-shaped MeshParts (8 studs): a 2-stud block dropped into the empty corner rests at y {on_box:.2} on CollisionFidelity Box (want 9: on the box's top) and at y {on_precise:.2} on PreciseConvexDecomposition (want 5: inside the corner; {pieces} convex pieces)"
            ),
        });
        self.r4.phase = 74;
        false
    }
}
