//! Client replication messages, indexed by the negotiated server schema.
use crate::handshake::{put_varint, put_varstr, zigzag};
use crate::replicator::{PropValue, Schema};

/// Encode one property change using the class's actual schema index.
/// Unsupported value types are rejected before any bytes are sent.
pub fn property(
    schema: &Schema,
    id: u64,
    class: &str,
    name: &str,
    value: &PropValue,
) -> Result<Vec<u8>, String> {
    // The wire renames apply per class (a Part's Color is Color3uint8, a
    // light's is Color; a Part's Size is size, a GUI's is Size): the renamed
    // property is used when the class has it, else the editor's name.
    let find = |name: &str| {
        schema.global_props.iter().enumerate().find_map(|(global, &(ci, pi))| {
            let cls = &schema.classes[ci as usize];
            let prop = &cls.props[pi as usize];
            (cls.name == class && prop.name == name).then_some((global, prop.ty))
        })
    };
    let (global, ty) = find(wire_name(name))
        .or_else(|| find(name))
        .ok_or_else(|| format!("{class}.{name} is not replicated by this server"))?;

    let global = u16::try_from(global).map_err(|_| "Property index exceeds the protocol range")?;
    let mut out = vec![0x83, 3];
    put_reference(&mut out, id);
    out.extend_from_slice(&global.to_le_bytes());
    out.push(0); // Client-origin version byte (verified against Studio c2s).
    let value = prepare_value(ty, value)?;
    encode_value(&mut out, ty, &value)?;
    append_shared(&mut out, &value);
    out.push(0); // End of item list.
    Ok(out)
}

/// Invoke a replicated event using the server's negotiated argument types.
pub fn event(schema: &Schema, id: u64, class: &str, name: &str, args: &[PropValue]) -> Result<Vec<u8>, String> {
    let (global, descriptor)=schema.global_events.iter().enumerate().find_map(|(global,&(ci,ei))| {
        let cls=&schema.classes[ci as usize];let event=&cls.events[ei as usize];
        (cls.name==class && event.name==name).then_some((global,event))
    }).ok_or_else(|| format!("{class}.{name} is not replicated by this server"))?;
    if args.len()!=descriptor.arg_types.len() {return Err(format!("{class}.{name} argument count"));}
    let mut out=vec![0x83,7];put_reference(&mut out,id);
    out.extend_from_slice(&u16::try_from(global).map_err(|_| "event index range")?.to_le_bytes());
    for (value,ty) in args.iter().zip(&descriptor.arg_types) {encode_value(&mut out,*ty,value)?;}
    out.push(0);Ok(out)
}

fn encode_tagged(out: &mut Vec<u8>, value: &PropValue) -> Result<(), String> {
    let ty=match value {
        PropValue::Bool(_)=>0x09,PropValue::Int(_) | PropValue::Int64(_)=>0x2c,PropValue::Float(_)=>0x0c,PropValue::Float32(_)=>0x0b,
        PropValue::String(_)=>0x02,PropValue::Array(_)=>0x1e,PropValue::Dictionary(_)=>0x1f,
        PropValue::Ref(_)=>0x1c,PropValue::Enum(_)=>0x07,
        PropValue::Vector3(_)=>0x16,PropValue::Vector2(_)=>0x15,PropValue::CFrame {..}=>0x1a,
        _=>return Err("Unsupported event container value".into()),
    };
    out.push(ty);encode_value(out,ty,value)
}

pub fn wire_name(name: &str) -> &str {
    match name {
        "Attributes"=>"AttributesReplicate",
        "Size" => "size",
        "Shape" => "shape",
        "FormFactor" => "formFactorRaw",
        "Color" => "Color3uint8",
        "MaterialVariant" => "MaterialVariantSerialized",
        "CollisionGroup" => "CollisionGroupReplicate",
        "RenderFidelity" => "RenderFidelityReplicate",
        "MeshId" => "MeshContent",
        "TextureID" => "TextureContent",
        other => other,
    }
}

fn prepare_value(ty: u8, value: &PropValue) -> Result<PropValue, String> {
    use blake2::digest::{Update,VariableOutput};
    let bytes=match (ty,value) {
        (0x06,PropValue::String(source))=>Some(source.as_bytes()),
        (0x06 | 0x2e,PropValue::SharedString(bytes))=>Some(bytes.as_slice()),
        _=>None,
    };
    if let Some(bytes)=bytes {
        let mut hash=[0;16];let mut state=blake2::Blake2bVar::new(16).map_err(|_| "shared hash size")?;
        state.update(bytes);state.finalize_variable(&mut hash).map_err(|_| "shared hash output")?;
        Ok(PropValue::CachedSharedString {hash,bytes:bytes.to_vec()})
    } else {Ok(value.clone())}
}

fn append_shared(out: &mut Vec<u8>, value: &PropValue) {
    if let PropValue::CachedSharedString {hash,bytes}=value {
        out.extend_from_slice(hash);out.push(0);put_varstr(out,bytes);
    }
}

fn encode_value(out: &mut Vec<u8>, ty: u8, value: &PropValue) -> Result<(), String> {
    if let PropValue::Typed { bytes, .. } = value {
        return encode_value(out, ty, &PropValue::Raw(bytes.clone()));
    }
    if let PropValue::Float32(v) = value {
        return encode_value(out, ty, &PropValue::Float(f64::from(*v)));
    }
    match (ty, value) {
        (0x1d | 0x1e, PropValue::Array(values)) => {
            put_varint(out,values.len() as u64);
            for value in values {encode_tagged(out,value)?;}
        }
        (0x1f, PropValue::Dictionary(values)) => {
            put_varint(out,values.len() as u64);
            for (key,value) in values {put_varstr(out,key.as_bytes());encode_tagged(out,value)?;}
        }
        (0x09, PropValue::Bool(v)) => out.push(u8::from(*v)),
        (0x0a | 0x2c, PropValue::Int(v) | PropValue::Int64(v)) => put_varint(out, zigzag(*v)),
        (0x07, PropValue::Enum(v)) => put_varint(out, u64::from(*v)),
        (0x0b, PropValue::Float(v)) => out.extend_from_slice(&(*v as f32).to_le_bytes()),
        (0x0c, PropValue::Float(v)) => out.extend_from_slice(&v.to_le_bytes()),
        (0x02 | 0x21, PropValue::BinaryString(v)) => put_varstr(out,v),
        (0x02 | 0x21, PropValue::String(v)) => put_varstr(out, v.as_bytes()),
        (0x31, PropValue::String(v)) if v.len() < 128 => {
            out.push(v.len() as u8);
            out.extend_from_slice(v.as_bytes());
        }
        (0x14, PropValue::Color3uint8(v)) => out.extend_from_slice(v),
        (0x14, PropValue::Color3(v)) => {
            for c in v {
                out.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        (0x13, PropValue::Color3uint8(v)) => {
            for c in v {
                out.extend_from_slice(&(f32::from(*c) / 255.0).to_le_bytes());
            }
        }
        (0x3d, PropValue::String(uri)) => {
            if uri.is_empty() {
                out.push(0);
            } else {
                out.push(1);
                put_varstr(out, uri.as_bytes());
            }
        }
        (0x13, PropValue::Color3(v)) | (0x16, PropValue::Vector3(v)) => {
            for f in v {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        (0x15, PropValue::Vector2(v)) => {
            for f in v {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        (0x1a, PropValue::CFrame { pos, rot }) => {
            for f in pos {
                out.extend_from_slice(&f.to_le_bytes());
            }
            out.push(0); // Full matrix, avoiding compressed-rotation assumptions.
            for f in rot {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        (0x1c, PropValue::Ref(reference)) => match reference {
            Some(id) => {
                put_reference(out, *id);
            }
            None => out.push(0),
        },
        (0x06 | 0x2e, PropValue::CachedSharedString { hash, .. } | PropValue::SharedHash(hash)) => {
            out.extend_from_slice(hash)
        }
        (0x08, PropValue::Raw(bytes)) => {
            put_varint(out, bytes.len() as u64);
            out.extend_from_slice(bytes);
        }
        (0x12, PropValue::Int(v)) => out.extend_from_slice(
            &u16::try_from(*v)
                .map_err(|_| "u16 property range")?
                .to_le_bytes(),
        ),
        (0x22, PropValue::Int(v)) => put_varint(
            out,
            u64::try_from(*v).map_err(|_| "unsigned property range")?,
        ),
        (0x23 | 0x26, PropValue::Raw(bytes)) => {
            let width = if ty == 0x23 { 12 } else { 20 };
            if bytes.len() % width != 0 {
                return Err("sequence byte length".into());
            }
            out.extend_from_slice(&((bytes.len() / width) as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        (0x29, PropValue::Raw(bytes)) => {
            out.push(match bytes.len() {
                0 => 0,
                20 => 1,
                24 => 3,
                _ => return Err("physical properties byte length".into()),
            });
            out.extend_from_slice(bytes);
        }
        (0x32, PropValue::Raw(bytes)) if bytes.is_empty() => out.push(0),
        (0x32, PropValue::CFrame { pos, rot }) => {
            out.push(0x80);
            for f in rot {
                out.extend_from_slice(&f.to_le_bytes());
            }
            for f in pos {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        (
            0x0d | 0x0e | 0x19 | 0x1d..=0x1f | 0x25 | 0x28 | 0x34 | 0x36 | 0x37 | 0x3c | 0x44,
            PropValue::Raw(bytes),
        ) => out.extend_from_slice(bytes),
        _ => return Err(format!("Wire value type {ty:#x} is not supported yet")),
    }
    Ok(())
}

/// NewObjectItem with schema-indexed properties and an empty early group.
pub fn create(
    schema: &Schema,
    id: u64,
    class: &str,
    parent: u64,
    props: &[(String, PropValue)],
) -> Result<Vec<u8>, String> {
    let (ci, cls) = schema
        .classes
        .iter()
        .enumerate()
        .find(|(_, c)| c.name == class)
        .ok_or_else(|| format!("Class {class} is not replicated by this server"))?;
    let ci = u16::try_from(ci).map_err(|_| "Class index exceeds protocol range")?;
    let mut out = vec![0x83, 2];
    put_reference(&mut out, id);
    out.extend_from_slice(&ci.to_le_bytes());
    let mut encoded=Vec::new();
    for (name,value) in props {
        let find=|n: &str| cls.props.iter().enumerate().find(|(_,p)| p.name==n);
        let (pi,prop)=find(wire_name(name)).or_else(|| find(name))
            .ok_or_else(|| format!("{class}.{name} is not replicated by this server"))?;
        if pi>=255 {return Err("Property index exceeds protocol range".into());}
        encoded.push((pi as u8,prop.ty,prepare_value(prop.ty,value)?));
    }
    // Protected source is serialized in NewObject's early group.
    for &(pi,ty,ref value) in encoded.iter().filter(|(_,ty,_)| *ty==0x06) {
        out.push(pi);encode_value(&mut out,ty,value)?;
    }
    out.push(0xff);
    for &(pi,ty,ref value) in encoded.iter().filter(|(_,ty,_)| *ty!=0x06) {
        out.push(pi);encode_value(&mut out,ty,value)?;
    }
    out.push(0xff);
    for (_,_,value) in encoded.iter().filter(|(_,ty,_)| *ty==0x06).chain(encoded.iter().filter(|(_,ty,_)| *ty!=0x06)) {
        append_shared(&mut out,value);
    }
    put_reference(&mut out, parent);
    out.push(0);
    Ok(out)
}

pub fn delete(id: u64) -> Vec<u8> {
    let mut out = vec![0x83, 1];
    put_reference(&mut out, id);
    out.push(0);
    out
}

/// Parent uses the reserved global descriptor after all ordinary properties.
/// Verified against a cross-account Studio reparent message.
pub fn reparent(schema: &Schema, id: u64, parent: u64) -> Result<Vec<u8>, String> {
    let descriptor =
        u16::try_from(schema.global_props.len()).map_err(|_| "Parent descriptor range")?;
    let mut out = vec![0x83, 3];
    put_reference(&mut out, id);
    out.extend_from_slice(&descriptor.to_le_bytes());
    out.push(0);
    put_reference(&mut out, parent);
    out.push(0);
    Ok(out)
}

fn put_reference(out: &mut Vec<u8>, id: u64) {
    let scope = if id <= u32::MAX as u64 { 1 } else { id >> 32 };
    put_varint(out, scope);
    out.extend_from_slice(&(id as u32).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replicator::{Decoder, RepOp, SchemaClass, SchemaProp};

    /// Every editor value type the live session sends survives the wire:
    /// the editor value → wire value → frame → decoder → model value.
    #[test]
    fn struct_values_round_trip_through_the_wire() {
        use rbx_types::{self as t, Variant as V};
        let cases: Vec<(&str, &str, u8, V)> = vec![
            ("Frame", "Size", 0x0e, V::UDim2(t::UDim2::new(t::UDim::new(0.5, -10), t::UDim::new(0.25, 40)))),
            ("UIPadding", "PaddingLeft", 0x0d, V::UDim(t::UDim::new(0.1, 8))),
            ("ImageLabel", "SliceCenter", 0x28, V::Rect(t::Rect::new(t::Vector2::new(10.0, 12.0), t::Vector2::new(90.0, 88.0)))),
            ("ParticleEmitter", "Size", 0x23, V::NumberSequence(t::NumberSequence { keypoints: vec![
                t::NumberSequenceKeypoint::new(0.0, 1.0, 0.0), t::NumberSequenceKeypoint::new(1.0, 3.5, 0.5)] })),
            ("ParticleEmitter", "Color", 0x26, V::ColorSequence(t::ColorSequence { keypoints: vec![
                t::ColorSequenceKeypoint::new(0.0, t::Color3::new(1.0, 0.0, 0.0)), t::ColorSequenceKeypoint::new(1.0, t::Color3::new(0.0, 0.0, 1.0))] })),
            ("ParticleEmitter", "Lifetime", 0x25, V::NumberRange(t::NumberRange::new(1.0, 2.5))),
            ("Part", "CustomPhysicalProperties", 0x29, V::PhysicalProperties(t::PhysicalProperties::Custom(t::CustomPhysicalProperties::new(0.7, 0.3, 0.5, 1.0, 1.0, 0.2)))),
            ("Part", "CustomPhysicalProperties", 0x29, V::PhysicalProperties(t::PhysicalProperties::Default)),
            ("TextLabel", "FontFace", 0x36, V::Font(t::Font::new("rbxasset://fonts/families/GothamSSm.json", t::FontWeight::Bold, t::FontStyle::Italic))),
            ("Decal", "TextureContent", 0x3d, V::Content(t::Content::from_uri("rbxassetid://123"))),
            ("Decal", "TextureContent", 0x3d, V::Content(t::Content::default())),
            ("PointLight", "Color", 0x13, V::Color3(t::Color3::new(1.0, 0.5, 0.25))),
            ("TerrainRegion", "ExtentsMin", 0x19, V::Vector3int16(t::Vector3int16::new(-4, 8, 300))),
        ];
        for (class, name, ty, value) in cases {
            let schema = Schema {
                classes: vec![SchemaClass { name: class.into(), props: vec![SchemaProp { name: name.into(), ty }], events: vec![] }],
                global_props: vec![(0, 0)],
                ..Default::default()
            };
            let wire = crate::model::typed_value(&value)
                .or_else(|| match &value { V::Color3(c) => Some(PropValue::Color3([c.r, c.g, c.b])), _ => None })
                .unwrap_or_else(|| panic!("{class}.{name}: no wire value"));
            let mut dec = Decoder::new();
            dec.schema = Some(schema.clone());
            let mut model = crate::model::LiveModel::new();
            model.apply(RepOp::NewInstance { id: 1, class: "DataModel".into(), parent: None, props: vec![] });
            let frame = create(&schema, 7, class, 1, &[]).unwrap();
            for op in dec.decode_frame(frame[0], &frame[1..]) { model.apply(op); }
            let frame = property(&schema, 7, class, name, &wire).unwrap_or_else(|e| panic!("{class}.{name}: {e}"));
            for op in dec.decode_frame(frame[0], &frame[1..]) { model.apply(op); }
            assert!(dec.errors.is_empty(), "{class}.{name}: {:?}", dec.errors);
            let inst = model.dom().get_by_ref(model.reference(7).unwrap()).unwrap();
            let got = inst.properties.iter().find(|(k, _)| k.as_str() == name).map(|(_, v)| v.clone());
            assert_eq!(got.as_ref(), Some(&value), "{class}.{name}");
        }
    }

    /// A Part's Color goes as Color3uint8 bytes; a GUI's Size keeps its name.
    #[test]
    fn renames_apply_only_where_the_class_has_them() {
        let schema = Schema {
            classes: vec![
                SchemaClass { name: "Part".into(), props: vec![SchemaProp { name: "Color3uint8".into(), ty: 0x14 }, SchemaProp { name: "size".into(), ty: 0x16 }], events: vec![] },
                SchemaClass { name: "Frame".into(), props: vec![SchemaProp { name: "Size".into(), ty: 0x0e }], events: vec![] },
            ],
            global_props: vec![(0, 0), (0, 1), (1, 0)],
            ..Default::default()
        };
        let frame = property(&schema, 7, "Part", "Color", &PropValue::Color3([1.0, 0.5, 0.0])).unwrap();
        assert!(frame.windows(3).any(|w| w == [255, 128, 0]));
        assert!(property(&schema, 7, "Part", "Size", &PropValue::Vector3([1.0, 2.0, 3.0])).is_ok());
        assert!(property(&schema, 8, "Frame", "Size", &crate::model::typed_value(&rbx_types::Variant::UDim2(rbx_types::UDim2::new(rbx_types::UDim::new(1.0, 0), rbx_types::UDim::new(0.0, 50)))).unwrap()).is_ok());
    }
    #[test]
    fn binary_attributes_survive_wire_and_become_editable_attributes() {
        let attributes=rbx_types::Attributes::new().with("RustudioSyncProbe",true).with("non_ascii",rbx_types::BinaryString::from("🦊".as_bytes())).with("value",123.456f64);
        let mut bytes=Vec::new();attributes.to_writer(&mut bytes).unwrap();
        assert!(String::from_utf8(bytes.clone()).is_err());
        let schema=Schema {classes:vec![SchemaClass {name:"ModuleScript".into(),props:vec![SchemaProp {name:"AttributesReplicate".into(),ty:2}],events:vec![]}],global_props:vec![(0,0)],..Default::default()};
        let frame=create(&schema,7,"ModuleScript",1,&[("Attributes".into(),PropValue::BinaryString(bytes))]).unwrap();
        let mut decoder=Decoder::new();decoder.schema=Some(schema);
        let mut model=crate::model::LiveModel::new();
        model.apply(RepOp::NewInstance {id:1,class:"DataModel".into(),parent:None,props:vec![]});
        for op in decoder.decode_frame(frame[0],&frame[1..]) {model.apply(op);}
        assert!(decoder.errors.is_empty());
        let script=model.dom().get_by_ref(model.reference(7).unwrap()).unwrap();
        assert_eq!(script.properties.get(&"Attributes".into()),Some(&rbx_types::Variant::Attributes(attributes)));
    }

    #[test]
    fn source_hash_matches_studio_and_inline_record_is_consumed() {
        let schema=Schema {classes:vec![SchemaClass {name:"ModuleScript".into(),props:vec![SchemaProp{name:"Source".into(),ty:6}],events:vec![]}],global_props:vec![(0,0)],..Default::default()};
        let source="-- rustudio synchronization probe\nreturn 2\n";
        let frame=property(&schema,(5u64<<32)|7,"ModuleScript","Source",&PropValue::String(source.into())).unwrap();
        let hash=[0x69,0xac,0xbd,0x1f,0x4b,0x55,0x3f,0xe1,0xe1,0x33,0x24,0xeb,0xcd,0x80,0x88,0xbf];
        assert_eq!(&frame[10..26],&hash);
        let mut dec=Decoder::new();dec.schema=Some(schema.clone());
        let ops=dec.decode_frame(frame[0],&frame[1..]);
        assert!(dec.errors.is_empty(),"{:?}",dec.errors);
        assert!(matches!(&ops[0],RepOp::ChangeProperty {value:PropValue::CachedSharedString {hash:h,bytes},..} if h==&hash && bytes==source.as_bytes()));
        let frame=create(&schema,(5u64<<32)|8,"ModuleScript",1,&[("Source".into(),PropValue::String(source.into()))]).unwrap();
        let ops=dec.decode_frame(frame[0],&frame[1..]);
        assert!(dec.errors.is_empty(),"{:?}",dec.errors);
        assert!(matches!(&ops[0],RepOp::NewInstance {props,..} if matches!(&props[0].1,PropValue::CachedSharedString {hash:h,bytes} if h==&hash && bytes==source.as_bytes())));
    }

    #[test]
    fn publish_event_preserves_typed_options_and_response_metadata() {
        use crate::replicator::SchemaEvent;
        let schema=Schema {classes:vec![SchemaClass{name:"TeamCreatePublishService".into(),props:vec![],events:vec![
            SchemaEvent{name:"TeamCreatePlaceUploadRequest".into(),arg_types:vec![0x1f]},
            SchemaEvent{name:"TeamCreatePlaceUploadResponse".into(),arg_types:vec![9,0x1f]},
        ]}],global_events:vec![(0,0),(0,1)],..Default::default()};
        let options=PropValue::Dictionary(vec![("publish".into(),PropValue::Bool(false)),("upload_to_edge".into(),PropValue::Bool(false))]);
        let frame=event(&schema,341653,"TeamCreatePublishService","TeamCreatePlaceUploadRequest",&[options.clone()]).unwrap();
        assert!(frame.windows(7).any(|w| w==b"publish"));
        let mut dec=Decoder::new();dec.schema=Some(schema.clone());
        let ops=dec.decode_frame(frame[0],&frame[1..]);
        assert!(dec.errors.is_empty());
        assert!(matches!(&ops[0],RepOp::Event {args,..} if args==&vec![options]));
        let metadata=PropValue::Dictionary(vec![("version_number".into(),PropValue::Int64(456))]);
        let frame=event(&schema,341653,"TeamCreatePublishService","TeamCreatePlaceUploadResponse",&[PropValue::Bool(true),metadata.clone()]).unwrap();
        let ops=dec.decode_frame(frame[0],&frame[1..]);
        assert!(matches!(&ops[0],RepOp::Event {args,..} if args==&vec![PropValue::Bool(true),metadata]));
        assert!(event(&schema,1,"TeamCreatePublishService","TeamCreatePlaceUploadRequest",&[]).is_err());
    }

    #[test]
    fn parent_uses_the_reserved_descriptor() {
        let schema = Schema {
            global_props: vec![(0, 0); 12259],
            ..Default::default()
        };
        let frame = reparent(&schema, (5u64 << 32) | 0xed2ea, (5u64 << 32) | 0xecef4).unwrap();
        assert_eq!(&frame[7..10], &[0xe3, 0x2f, 0]);
        let mut dec = Decoder::new();
        dec.schema = Some(schema);
        let ops = dec.decode_frame(frame[0], &frame[1..]);
        assert!(dec.errors.is_empty());
        assert!(
            matches!(ops[0],RepOp::Reparent {id,parent} if id==(5u64<<32)|0xed2ea && parent==(5u64<<32)|0xecef4)
        );
    }

    #[test]
    fn create_and_delete_preserve_properties_and_scoped_references() {
        let schema = Schema {
            classes: vec![SchemaClass {
                name: "Part".into(),
                props: vec![
                    SchemaProp {
                        name: "Name".into(),
                        ty: 0x31,
                    },
                    SchemaProp {
                        name: "size".into(),
                        ty: 0x16,
                    },
                    SchemaProp {
                        name: "UniqueId".into(),
                        ty: 0x34,
                    },
                ],
                events: vec![],
            }],
            ..Default::default()
        };
        let id = (12u64 << 32) | 1;
        let parent = (5u64 << 32) | 7;
        let props = vec![
            ("Name".into(), PropValue::String("Kept".into())),
            ("size".into(), PropValue::Vector3([2., 3., 4.])),
            ("UniqueId".into(), PropValue::Raw(vec![42; 16])),
        ];
        let frame = create(&schema, id, "Part", parent, &props).unwrap();
        let mut dec = Decoder::new();
        dec.schema = Some(schema);
        let ops = dec.decode_frame(frame[0], &frame[1..]);
        assert!(dec.errors.is_empty(), "{:?}", dec.errors);
        assert!(
            matches!(&ops[0],RepOp::NewInstance{id:actual,parent:Some(p),props:actual_props,..}
            if *actual==id && *p==parent && actual_props==&props)
        );
        let frame = delete(id);
        assert!(
            matches!(dec.decode_frame(frame[0],&frame[1..])[0],RepOp::DeleteInstance{id:actual} if actual==id)
        );
    }

    #[test]
    fn property_update_round_trips_through_negotiated_schema() {
        let schema = Schema {
            classes: vec![SchemaClass {
                name: "Part".into(),
                props: vec![
                    SchemaProp {
                        name: "size".into(),
                        ty: 0x16,
                    },
                    SchemaProp {
                        name: "Name".into(),
                        ty: 0x31,
                    },
                ],
                events: vec![],
            }],
            global_props: vec![(0, 0), (0, 1)],
            ..Default::default()
        };
        for (name, value) in [
            ("Size", PropValue::Vector3([3.0, 4.0, 5.0])),
            ("Name", PropValue::String("Synced".into())),
        ] {
            let id = (42u64 << 32) | 77;
            let frame = property(&schema, id, "Part", name, &value).unwrap();
            let mut decoder = Decoder::new();
            decoder.schema = Some(schema.clone());
            let ops = decoder.decode_frame(frame[0], &frame[1..]);
            assert!(decoder.errors.is_empty());
            assert!(
                matches!(&ops[0], RepOp::ChangeProperty { id: actual_id, value: actual, .. } if *actual_id == id && actual == &value)
            );
        }
        assert!(property(
            &schema,
            77,
            "Part",
            "Source",
            &PropValue::String("x".into())
        )
        .is_err());
    }
}
