//! The fidelity corpus (tools/fidelity): place files both real Studio and
//! rustudio open, so the same moments can be captured in each and compared.
//!
//! - `demo.rbxlx`: rustudio's sample scene (`--demo`) on the default place.
//! - `control.rbxlx`: deterministic geometry, lighting and materials — a
//!   baseplate and spawn, primary-coloured blocks, every basic shape, a row
//!   of material cubes, a neon bar, glass, fixed Lighting (ClockTime 14,
//!   Brightness 2, Future) — for render, lighting, material and physics
//!   comparisons that don't depend on any game script.
//!
//! Place A is the third corpus place; its file is the person's own and is read
//! where it lies (tools/fidelity/corpus.json).

use std::path::Path;

use glam::Vec3;
use rbx_dom_weak::types::{CFrame, Color3, Enum, Vector3};
use rbx_dom_weak::{ustr, InstanceBuilder};

use crate::doc::{new_part, Document};

fn control() -> Document {
    let mut doc = Document::new_place();
    let root = doc.dom.root_ref();
    if let Some(l) = doc.find_child(root, "Lighting") {
        if let Some(i) = doc.dom.get_by_ref_mut(l) {
            let p = &mut i.properties;
            p.insert(ustr("Technology"), Enum::from_u32(4).into());
            p.insert(ustr("ClockTime"), 14.0f32.into());
            p.insert(ustr("Brightness"), 2.0f32.into());
            p.insert(ustr("GlobalShadows"), true.into());
            p.insert(ustr("Ambient"), Color3::new(70.0 / 255.0, 70.0 / 255.0, 70.0 / 255.0).into());
            p.insert(
                ustr("OutdoorAmbient"),
                Color3::new(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0).into(),
            );
            p.insert(ustr("EnvironmentDiffuseScale"), 1.0f32.into());
            p.insert(ustr("EnvironmentSpecularScale"), 1.0f32.into());
        }
    }
    let ms = doc.find_child(root, "MaterialService").unwrap_or_else(|| {
        doc.dom.insert(
            root,
            InstanceBuilder::new("MaterialService").with_name("MaterialService"),
        )
    });
    if let Some(i) = doc.dom.get_by_ref_mut(ms) {
        i.properties.insert(ustr("Use2022Materials"), true.into());
    }
    let ws = doc.workspace().expect("workspace");
    // Primary-coloured blocks and every basic shape, in front of the spawn.
    let shapes: [(&str, u32, [f32; 3], [f32; 3], [u8; 3]); 6] = [
        ("Red", 1, [-12.0, 2.0, -16.0], [4.0, 4.0, 4.0], [255, 0, 0]),
        ("Green", 1, [-6.0, 2.0, -16.0], [4.0, 4.0, 4.0], [0, 255, 0]),
        ("Blue", 1, [0.0, 2.0, -16.0], [4.0, 4.0, 4.0], [0, 0, 255]),
        ("White", 1, [6.0, 2.0, -16.0], [4.0, 4.0, 4.0], [255, 255, 255]),
        ("Ball", 0, [12.0, 2.0, -16.0], [4.0, 4.0, 4.0], [255, 200, 0]),
        ("Cylinder", 2, [18.0, 2.0, -16.0], [4.0, 4.0, 4.0], [0, 200, 200]),
    ];
    for (name, shape, p, s, c) in shapes {
        doc.insert(ws, new_part(name, shape, Vec3::from(p), Vec3::from(s), c));
    }
    doc.insert(
        ws,
        new_part(
            "Wedge",
            1,
            Vec3::new(-18.0, 2.0, -16.0),
            Vec3::splat(4.0),
            [200, 120, 60],
        )
        .with_class("WedgePart"),
    );
    // Material cubes (2022 materials), white so the maps show.
    for (i, mat) in [848u32, 512, 816, 1088, 1280, 800, 1312, 528].iter().enumerate() {
        doc.insert(
            ws,
            new_part(
                &format!("Mat{mat}"),
                1,
                Vec3::new(-21.0 + i as f32 * 6.0, 2.0, -26.0),
                Vec3::splat(4.0),
                [255, 255, 255],
            )
            .with_property("Material", Enum::from_u32(*mat)),
        );
    }
    doc.insert(
        ws,
        new_part("Neon", 1, Vec3::new(0.0, 7.0, -30.0), Vec3::new(24.0, 1.0, 1.0), [255, 80, 200])
            .with_property("Material", Enum::from_u32(288)),
    );
    doc.insert(
        ws,
        new_part("Glass", 1, Vec3::new(0.0, 3.0, -9.0), Vec3::new(8.0, 6.0, 0.5), [180, 220, 255])
            .with_property("Material", Enum::from_u32(1568))
            .with_property("Transparency", 0.5f32),
    );
    // A ledge to fall from and a slope, for the physics facts.
    doc.insert(
        ws,
        new_part("Ledge", 1, Vec3::new(20.0, 5.0, 10.0), Vec3::new(8.0, 10.0, 8.0), [120, 120, 130]),
    );
    doc
}

/// Place A's chapel lobby lighting in a plain room: what its Menu script sets at
/// run time (Shared/NightLighting: ClockTime 07:45, Brightness 0.35, night
/// ambients, ColorShift_Top, close fog, GlobalShadows off) on a place that
/// saves Technology Voxel (drawn Soft), with spot and point lights at Place A's values,
/// so the chapel's brightness can be measured without the game.
fn night() -> Document {
    let mut doc = Document::new_place();
    let root = doc.dom.root_ref();
    let rgb = |r: u8, g: u8, b: u8| Color3::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    if let Some(l) = doc.find_child(root, "Lighting") {
        if let Some(i) = doc.dom.get_by_ref_mut(l) {
            let p = &mut i.properties;
            // Voxel, as Place A saves it (Studio draws it Soft; a place saving no
            // Technology at all stops Studio on a migration dialog).
            p.insert(ustr("Technology"), Enum::from_u32(1).into());
            p.insert(ustr("ClockTime"), 7.75f32.into());
            p.insert(ustr("TimeOfDay"), "07:45:00".to_string().into());
            p.insert(ustr("GeographicLatitude"), 42.0f32.into());
            p.insert(ustr("Brightness"), 0.35f32.into());
            p.insert(ustr("GlobalShadows"), false.into());
            p.insert(ustr("ColorShift_Top"), rgb(140, 175, 230).into());
            p.insert(ustr("ColorShift_Bottom"), rgb(0, 0, 0).into());
            p.insert(ustr("Ambient"), rgb(8, 9, 14).into());
            p.insert(ustr("OutdoorAmbient"), rgb(12, 15, 24).into());
            p.insert(ustr("EnvironmentDiffuseScale"), 0.08f32.into());
            p.insert(ustr("EnvironmentSpecularScale"), 0.3f32.into());
            p.insert(ustr("ExposureCompensation"), 0.0f32.into());
            p.insert(ustr("FogStart"), 40.0f32.into());
            p.insert(ustr("FogEnd"), 400.0f32.into());
            p.insert(ustr("FogColor"), rgb(16, 22, 34).into());
        }
    }
    let ws = doc.workspace().expect("workspace");
    // No spawn ForceField: its bubble would stand in front of the lighting.
    if let Some(sp) = doc.find_child(ws, "SpawnLocation") {
        if let Some(i) = doc.dom.get_by_ref_mut(sp) {
            i.properties.insert(ustr("Duration"), 0i32.into());
        }
    }
    // The room: open toward the spawn (+Z), a roof with a hole over the middle.
    let stone = [150, 145, 140];
    let room: [(&str, [f32; 3], [f32; 3], [u8; 3]); 8] = [
        ("WallL", [-17.0, 8.0, -24.0], [2.0, 16.0, 32.0], stone),
        ("WallR", [17.0, 8.0, -24.0], [2.0, 16.0, 32.0], stone),
        ("WallBack", [0.0, 8.0, -41.0], [36.0, 16.0, 2.0], stone),
        ("RoofFront", [0.0, 17.0, -13.0], [36.0, 2.0, 10.0], stone),
        ("RoofBack", [0.0, 17.0, -35.0], [36.0, 2.0, 10.0], stone),
        ("RoofL", [-11.0, 17.0, -24.0], [14.0, 2.0, 12.0], stone),
        ("RoofR", [11.0, 17.0, -24.0], [14.0, 2.0, 12.0], stone),
        ("Floor", [0.0, 0.1, -24.0], [32.0, 0.2, 32.0], [110, 90, 80]),
    ];
    for (name, p, s, c) in room {
        doc.insert(ws, new_part(name, 1, Vec3::from(p), Vec3::from(s), c));
    }
    let blocks: [(&str, [f32; 3], [f32; 3], [u8; 3]); 4] = [
        ("White", [-8.0, 2.2, -26.0], [4.0, 4.0, 4.0], [255, 255, 255]),
        ("Grey", [0.0, 3.2, -32.0], [4.0, 6.0, 4.0], [128, 128, 128]),
        ("Red", [8.0, 2.2, -26.0], [4.0, 4.0, 4.0], [200, 60, 50]),
        ("Altar", [0.0, 1.7, -22.0], [8.0, 3.0, 3.0], [170, 110, 90]),
    ];
    for (name, p, s, c) in blocks {
        doc.insert(ws, new_part(name, 1, Vec3::from(p), Vec3::from(s), c));
    }
    // Lamps (small parts holding the lights, as Place A's fixtures do).
    let down = rbx_dom_weak::types::Matrix3::new(
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 1.0),
        Vector3::new(0.0, -1.0, 0.0),
    );
    let lamp = |name: &str, p: [f32; 3], light: InstanceBuilder, turned: bool| {
        let mut b = new_part(name, 1, Vec3::from(p), Vec3::splat(0.5), [255, 220, 170]).with_child(light);
        if turned {
            b.add_property(
                "CFrame",
                CFrame::new(Vector3::new(p[0], p[1], p[2]), down),
            );
        }
        b
    };
    let warm = rgb(255, 200, 140);
    doc.insert(
        ws,
        lamp(
            "LampSpotA",
            [-8.0, 14.0, -26.0],
            InstanceBuilder::new("SpotLight")
                .with_property("Brightness", 0.63f32)
                .with_property("Range", 22.08f32)
                .with_property("Angle", 90.0f32)
                .with_property("Face", Enum::from_u32(4))
                .with_property("Color", warm),
            false,
        ),
    );
    doc.insert(
        ws,
        lamp(
            "LampSpotB",
            [8.0, 15.5, -30.0],
            InstanceBuilder::new("SpotLight")
                .with_property("Brightness", 2.0f32)
                .with_property("Range", 32.0f32)
                .with_property("Angle", 60.0f32)
                .with_property("Face", Enum::from_u32(4))
                .with_property("Color", rgb(200, 210, 255)),
            false,
        ),
    );
    doc.insert(
        ws,
        lamp(
            "LampPoint",
            [0.0, 6.0, -36.0],
            InstanceBuilder::new("PointLight")
                .with_property("Brightness", 0.15f32)
                .with_property("Range", 16.0f32)
                .with_property("Color", warm),
            false,
        ),
    );
    doc.insert(
        ws,
        lamp(
            "LampLow",
            [12.0, 4.0, -18.0],
            InstanceBuilder::new("PointLight")
                .with_property("Brightness", 0.63f32)
                .with_property("Range", 9.6f32)
                .with_property("Color", warm),
            true,
        ),
    );
    doc
}

fn demo() -> Document {
    let mut doc = Document::new_place();
    crate::app::build_demo_parts(&mut doc);
    doc
}

/// Writes demo.rbxlx, control.rbxlx and night.rbxlx into `dir` (`--write-corpus DIR`).
pub fn write_corpus(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for (name, doc) in [("demo", demo()), ("control", control()), ("night", night())] {
        let bytes = doc.encode_all(true)?;
        let path = dir.join(format!("{name}.rbxlx"));
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
