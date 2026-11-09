//! Resolve Roblox Sound/SoundGroup properties without coupling the mixer to the DOM.
use crate::{
    audio_dsp::{rolloff, Effect, Settings},
    doc::{self, Document},
    math::Xform,
    scene::{self, xform_of},
};
use glam::{Mat3, Vec3};
use rbx_dom_weak::types::{Ref, Variant};
fn number(d: &Document, r: Ref, k: &str, default: f32) -> f32 {
    match d.prop(r, k) {
        Some(Variant::Float32(x)) => x,
        Some(Variant::Float64(x)) => x as f32,
        Some(Variant::Int32(x)) => x as f32,
        _ => default,
    }
}
fn effects(d: &Document, r: Ref) -> Vec<Effect> {
    let mut out = vec![];
    if let Some(i) = d.dom.get_by_ref(r) {
        for child in i.children() {
            let Some(c) = d.class_of(*child) else {
                continue;
            };
            if !c.ends_with("SoundEffect")
                || matches!(d.prop(*child, "Enabled"), Some(Variant::Bool(false)))
            {
                continue;
            }
            let props: &[(&str, f32)] = match c.as_str() {
                "ReverbSoundEffect" => &[
                    ("DecayTime", 1.5),
                    ("Density", 1.),
                    ("Diffusion", 1.),
                    ("WetLevel", -6.),
                    ("DryLevel", -6.),
                ],
                "EchoSoundEffect" => &[
                    ("Delay", 1.),
                    ("Feedback", 0.5),
                    ("WetLevel", -6.),
                    ("DryLevel", 0.),
                ],
                "EqualizerSoundEffect" => &[("LowGain", 0.), ("MidGain", 0.), ("HighGain", 0.)],
                "CompressorSoundEffect" => &[
                    ("Threshold", -40.),
                    ("Ratio", 40.),
                    ("Attack", 0.1),
                    ("Release", 0.1),
                    ("GainMakeup", 0.),
                ],
                "ChorusSoundEffect" | "FlangeSoundEffect" => {
                    &[("Rate", 0.5), ("Depth", 0.15), ("Mix", 0.5)]
                }
                "PitchShiftSoundEffect" => &[("Octave", 1.)],
                "DistortionSoundEffect" => &[("Level", 0.5)],
                "TremoloSoundEffect" => &[("Frequency", 5.), ("Depth", 1.), ("Duty", 0.5)],
                _ => continue,
            };
            out.push((
                number(d, *child, "Priority", 0.),
                Effect {
                    id: key(*child),
                    kind: c.to_string(),
                    p: props
                        .iter()
                        .map(|(k, v)| number(d, *child, k, *v))
                        .collect(),
                },
            ));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, e)| e).collect()
}
fn world(d: &Document, r: Ref, depth: u8) -> Option<Xform> {
    if depth > 32 {
        return None;
    }
    let c = d.class_of(r)?;
    if c == "Terrain" {
        return Some(Xform {
            pos: Vec3::ZERO,
            rot: Mat3::IDENTITY,
        });
    }
    if doc::is_a(&c, "Attachment") {
        let local = match d.prop(r, "CFrame") {
            Some(Variant::CFrame(c)) => Xform::from_cframe(&c),
            _ => Xform {
                pos: Vec3::ZERO,
                rot: Mat3::IDENTITY,
            },
        };
        let parent = d.dom.get_by_ref(r)?.parent();
        let p = world(d, parent, depth + 1).unwrap_or(Xform {
            pos: Vec3::ZERO,
            rot: Mat3::IDENTITY,
        });
        Some(Xform {
            pos: p.pos + p.rot * local.pos,
            rot: p.rot * local.rot,
        })
    } else {
        xform_of(d, r)
    }
}
pub fn settings(
    d: &Document,
    r: Ref,
    cam: Vec3,
    right: Vec3,
    listener: Option<(u32, Option<Xform>, Option<Ref>)>,
) -> (Vec<(u64, Settings)>, Settings) {
    let (cam, right) = match listener {
        Some((1, Some(cf), _)) => (cf.pos, cf.rot.x_axis),
        Some((mode, _, Some(part))) if mode == 2 || mode == 3 => world(d, part, 0)
            .map(|x| (x.pos, if mode == 3 { x.rot.x_axis } else { right }))
            .unwrap_or((cam, right)),
        _ => (cam, right),
    };
    let mut groups = vec![];
    let mut group = match d.prop(r, "SoundGroup") {
        Some(Variant::Ref(g)) => g,
        _ => Ref::none(),
    };
    for _ in 0..32 {
        if d.class_of(group).as_deref() != Some("SoundGroup") {
            break;
        }
        if groups.iter().any(|(id, _)| *id == key(group)) {
            break;
        }
        groups.push((
            key(group),
            Settings {
                gain: number(d, group, "Volume", 1.).clamp(0., 10.),
                pan: None,
                effects: effects(d, group),
            },
        ));
        group = d
            .dom
            .get_by_ref(group)
            .map(|i| i.parent())
            .unwrap_or_else(Ref::none);
    }
    groups.reverse();
    let mut s = Settings {
        effects: effects(d, r),
        ..Settings::default()
    };
    let parent = d
        .dom
        .get_by_ref(r)
        .map(|i| i.parent())
        .unwrap_or_else(Ref::none);
    let c = d.class_of(parent).unwrap_or_default();
    if doc::is_a(&c, "Attachment") || doc::is_a(&c, "BasePart") {
        if let Some(at) = world(d, parent, 0) {
            let delta = at.pos - cam;
            let distance = delta.length();
            let service = d
                .find_child(d.dom.root_ref(), "SoundService")
                .unwrap_or_else(Ref::none);
            let scale = number(d, service, "RolloffScale", 1.);
            s.gain = rolloff(
                scene::enum_u32(d.prop(r, "RollOffMode")).unwrap_or(0),
                distance,
                number(d, r, "RollOffMinDistance", 10.),
                number(d, r, "RollOffMaxDistance", 10000.),
                scale,
            );
            s.pan = Some(delta.normalize_or_zero().dot(right));
        }
    }
    (groups, s)
}

fn key(r: Ref) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    r.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::InstanceBuilder;
    #[test]
    fn audio_terrain_attachment_and_group_graph() {
        let mut d = Document::new_place();
        let root = d.dom.root_ref();
        let ws = d.workspace().unwrap();
        let terrain = d.dom.insert(ws, InstanceBuilder::new("Terrain"));
        let a = d.dom.insert(
            terrain,
            InstanceBuilder::new("Attachment").with_property(
                "CFrame",
                rbx_dom_weak::types::CFrame::new(
                    rbx_dom_weak::types::Vector3::new(60., 0., 0.),
                    rbx_dom_weak::types::Matrix3::identity(),
                ),
            ),
        );
        let master = d.dom.insert(
            root,
            InstanceBuilder::new("SoundGroup").with_property("Volume", 0.5f32),
        );
        let category = d.dom.insert(
            master,
            InstanceBuilder::new("SoundGroup").with_property("Volume", 0.25f32),
        );
        let disabled = d.dom.insert(
            category,
            InstanceBuilder::new("EchoSoundEffect").with_property("Enabled", false),
        );
        let _ = disabled;
        d.dom.insert(
            category,
            InstanceBuilder::new("EqualizerSoundEffect").with_property("Priority", 3i32),
        );
        d.dom.insert(
            category,
            InstanceBuilder::new("ReverbSoundEffect").with_property("Priority", 1i32),
        );
        let sound = d.dom.insert(
            a,
            InstanceBuilder::new("Sound")
                .with_property("SoundGroup", category)
                .with_property("RollOffMinDistance", 6f32)
                .with_property("RollOffMaxDistance", 120f32),
        );
        let (groups, s) = settings(&d, sound, Vec3::ZERO, Vec3::X, None);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].1.gain * groups[1].1.gain, 0.125);
        assert_eq!(groups[1].1.effects.len(), 2);
        assert_eq!(groups[1].1.effects[0].kind, "ReverbSoundEffect");
        assert_eq!(groups[1].1.effects[1].kind, "EqualizerSoundEffect");
        assert!((s.gain - 0.1).abs() < 1e-6);
        assert_eq!(s.pan, Some(1.));
        let (_, s) = settings(
            &d,
            sound,
            Vec3::ZERO,
            Vec3::X,
            Some((
                1,
                Some(Xform {
                    pos: Vec3::new(60., 0., 0.),
                    rot: Mat3::IDENTITY,
                }),
                None,
            )),
        );
        assert_eq!(s.gain, 1.);
    }
}
