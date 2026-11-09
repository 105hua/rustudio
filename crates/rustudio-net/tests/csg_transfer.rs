//! Smallest complete a5 transfer in the recorded 0.746 join. Geometry only:
//! three frames, no instance names, account data, tickets or stream keys.
use blake2::{Blake2b, Digest, digest::consts::U16};
use rustudio_net::{model::LiveModel, replicator::{split_stream, Decoder, PropValue, RepOp}};
use rbx_dom_weak::types::Variant;

fn frames() -> Vec<&'static [u8]> {
    split_stream(include_bytes!("fixtures/csg-solidmesh.rtcio")).unwrap().2
}

fn asset() -> (u64, Vec<u8>) {
    let mut decoder = Decoder::new();
    let mut result = None;
    for frame in frames() {
        for op in decoder.decode_frame(frame[0], &frame[1..]) {
            if let RepOp::NetAsset { key, data } = op { result = Some((key, data)); }
        }
    }
    assert!(decoder.errors.is_empty(), "{:?}", decoder.errors);
    result.unwrap()
}

#[test]
fn captured_transfer_has_exact_holder_bytes() {
    let (key, data) = asset();
    assert_eq!(key, (0xf0u64 << 56) | 1410);
    assert_eq!(data.len(), 379);
    assert_eq!(&data[..14], &[3,1,109,1,0,0,0,0,0,0,109,1,0,0]);
    assert_eq!(&data[14..27], b"SolidMesh\0\0\0\0");
    let hash = Blake2b::<U16>::digest(&data);
    assert_eq!(format!("{hash:x}"), "47b98fb04896c60867735c080666284a");
}

#[test]
fn incomplete_or_wrong_index_transfer_cannot_install_geometry() {
    let frames = frames();
    for wrong_index in [false, true] {
        let mut d = Decoder::new();
        d.decode_frame(0xa5, &frames[0][1..]);
        let mut chunk = frames[1][1..].to_vec();
        if wrong_index { chunk[2] = 1; } else { chunk.pop(); }
        assert!(d.decode_frame(0xa5, &chunk).is_empty());
        assert!(d.decode_frame(0xa5, &frames[2][1..]).is_empty());
        assert!(!d.errors.is_empty());
    }
}

#[test]
fn union_holder_and_tags_round_trip_to_rbxl_in_either_arrival_order() {
    let (key, data) = asset();
    // Single reference from the captured begin frame, followed by holder flags.
    let mut reference = frames()[0][4..14].to_vec();
    reference.extend_from_slice(&[3,0,0,0,0,0,0,0]);
    for early_asset in [false, true] {
        let mut m = LiveModel::new();
        m.apply(RepOp::NewInstance { id: 1, class: "DataModel".into(), parent: None, props: vec![] });
        if early_asset { m.apply(RepOp::NetAsset {key, data: data.clone()}); }
        m.apply(RepOp::NewInstance {
            id: 2, class: "UnionOperation".into(), parent: Some(1),
            props: vec![("SolidMeshHolder".into(), PropValue::Typed {ty: 0x3c, bytes: reference.clone()}),
                        ("Tags".into(), PropValue::SharedString(b"Geometry\0Captured".to_vec())),
                        ("TriangleCount".into(), PropValue::Int(12)),
                        ("SourceAssetId".into(), PropValue::Int64(0)),
                        ("Transparency".into(), PropValue::Float32(0.25))],
        });
        if !early_asset { m.apply(RepOp::NetAsset {key, data: data.clone()}); }
        let r = m.reference(2).unwrap();
        let mut bytes = Vec::new();
        rbx_binary::to_writer(&mut bytes, m.dom(), &[r]).unwrap();
        let loaded = rbx_binary::from_reader(bytes.as_slice()).unwrap();
        let inst = loaded.get_by_ref(loaded.root().children()[0]).unwrap();
        match &inst.properties[&rbx_dom_weak::ustr("SolidMeshHolder")] {
            Variant::NetAssetRef(h) => assert_eq!(h.data(), data.as_slice()),
            other => panic!("unexpected holder {other:?}"),
        }
        assert!(matches!(&inst.properties[&rbx_dom_weak::ustr("Tags")], Variant::Tags(t) if t.len() == 2));
    }
}

#[test]
fn superseded_pending_holder_does_not_install_stale_geometry() {
    let (key, data) = asset();
    let mut reference = frames()[0][4..14].to_vec();
    reference.extend_from_slice(&[3,0,0,0,0,0,0,0]);
    let mut m = LiveModel::new();
    m.apply(RepOp::NewInstance { id: 1, class: "DataModel".into(), parent: None, props: vec![] });
    m.apply(RepOp::NewInstance { id: 2, class: "UnionOperation".into(), parent: Some(1),
        props: vec![("SolidMeshHolder".into(), PropValue::Typed {ty: 0x3c, bytes: reference.clone()})] });
    reference[8] += 1; // a distinct asset id with the same varint width
    m.apply(RepOp::ChangeProperty { id: 2, name: "SolidMeshHolder".into(),
        value: PropValue::Typed {ty: 0x3c, bytes: reference} });
    m.apply(RepOp::NetAsset { key, data });
    assert!(m.take_resolved().is_empty());
    assert!(!m.dom().get_by_ref(m.reference(2).unwrap()).unwrap().properties.contains_key(&rbx_dom_weak::ustr("SolidMeshHolder")));
}
