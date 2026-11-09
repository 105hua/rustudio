//! Write a local rendering fixture from the credential-free captured a5 transfer.
//! No network. Usage: tc_csg_fixture /private/path/union.rbxl
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use rbx_types::{CFrame, Matrix3, NetAssetRef, Vector3};
use rustudio_net::replicator::{split_stream, Decoder, RepOp};

fn main() {
    let path = std::env::args().nth(1).expect("usage: tc_csg_fixture OUT.rbxl");
    let (_, _, frames) = split_stream(include_bytes!("../tests/fixtures/csg-solidmesh.rtcio")).unwrap();
    let mut decoder = Decoder::new();
    let data = frames.into_iter().flat_map(|f| decoder.decode_frame(f[0], &f[1..]))
        .find_map(|op| match op {RepOp::NetAsset {data,..} => Some(data), _ => None}).unwrap();
    assert!(decoder.errors.is_empty());
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let workspace = dom.insert(dom.root_ref(), InstanceBuilder::new("Workspace"));
    dom.insert(workspace, InstanceBuilder::new("UnionOperation")
        .with_name("Captured a5 SolidMesh")
        .with_property("SolidMeshHolder", NetAssetRef::new(data))
        .with_property("Anchored", true)
        .with_property("UsePartColor", true)
        .with_property("Size", Vector3::new(10.0, 10.0, 10.0))
        .with_property("CFrame", CFrame::new(Vector3::new(0.0, 5.0, 0.0), Matrix3::identity())));
    rbx_binary::to_writer(std::io::BufWriter::new(std::fs::File::create(path).unwrap()), &dom, &[workspace]).unwrap();
}
