//! Numerical attachment transform regressions, independent of rendering.
use super::*;
use rbx_dom_weak::InstanceBuilder;
fn cf(p: Vec3, r: Mat3) -> Variant { Variant::CFrame(Xform {pos:p,rot:r}.to_cframe()) }
#[test]
fn nested_bones_and_terrain() {
    let mut d = Document::new_place();
    let ws = d.find_child(d.dom.root_ref(), "Workspace").unwrap();
    let p = d.insert(ws, InstanceBuilder::new("Part").with_property("CFrame", cf(Vec3::new(10.,0.,0.), Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2))));
    let b = d.insert(p, InstanceBuilder::new("Bone").with_property("CFrame", cf(Vec3::X, Mat3::IDENTITY)).with_property("Transform", cf(Vec3::Y, Mat3::IDENTITY)));
    assert!((world(&d,b,false).unwrap().pos-Vec3::new(10.,1.,0.)).length()<1e-5);
    assert!((world(&d,b,true).unwrap().pos-Vec3::new(9.,1.,0.)).length()<1e-5);
    let a = d.insert(b, InstanceBuilder::new("Attachment").with_property("CFrame", cf(Vec3::X, Mat3::IDENTITY)));
    assert!((world(&d,a,false).unwrap().pos-Vec3::new(9.,2.,0.)).length()<1e-5);
    assert!((world(&d,a,true).unwrap().pos-Vec3::new(9.,2.,0.)).length()<1e-5);
    let terrain = d.insert(ws, InstanceBuilder::new("Terrain").with_property("CFrame", cf(Vec3::splat(100.), Mat3::IDENTITY)));
    let a = d.insert(terrain, InstanceBuilder::new("Attachment").with_property("CFrame", cf(Vec3::X, Mat3::IDENTITY)));
    assert_eq!(world(&d,a,false).unwrap().pos, Vec3::X);
    let a = d.insert(ws, InstanceBuilder::new("Attachment"));
    assert_eq!(world(&d,a,true).unwrap().pos, Vec3::ZERO);
}

#[test]
fn bone_light_uses_transformed_frame() {
    let mut d = Document::new_place();
    let ws = d.find_child(d.dom.root_ref(), "Workspace").unwrap();
    let p = d.insert(ws, InstanceBuilder::new("Part").with_property("CFrame", cf(Vec3::new(10.,0.,0.), Mat3::IDENTITY)));
    let b = d.insert(p, InstanceBuilder::new("Bone").with_property("CFrame", cf(Vec3::Y, Mat3::IDENTITY)).with_property("Transform", cf(Vec3::X, Mat3::IDENTITY)));
    d.insert(b, InstanceBuilder::new("PointLight"));
    let mut cache = crate::lighting::LightCache::default();
    assert!(cache.hosts(&mut d).contains(&p));
    let lights = cache.gather(&mut d, &crate::camera::Camera::default(), &|_,_| true);
    assert_eq!(lights.len(), 1);
    assert_eq!(&lights[0].pos_range[..3], &[11.,1.,0.]);
}
