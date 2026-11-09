//! The CloudEdit replicator: the item stream carried over the transport.
//!
//! The wire layout here is decoded from a real, decrypted Team Create join
//! (Studio 0.746, docs/PROTOCOL.md §3b, [verified: capture]); the research decoder
//! it was derived from is `tools/tc/rig/rtcio_*.py`.
//!
//! Layers, outermost first:
//! - [`split_stream`]: a reassembled `rbx-rtcio` QUIC stream is
//!   `06 01 <channel> <u32 BE seq | "ctrl">` then messages framed as
//!   `<u32 BE 0x80000000 | len> <bytes>`.
//! - [`Decoder::decode_frame`]: one message = a Roblox packet id byte plus a
//!   body. `0x97` carries the schema (class / property / event tables) that
//!   every later item indexes into; `0x83` (ID_DATA) carries replicator items:
//!   initial-stream chunks, instance batches, new instances, property changes,
//!   events.

use rbx_dom_weak::types::Ref;
use std::collections::HashMap;

/// Roblox top-level packet ids (the first byte of a message).
pub mod packet_id {
    /// The top-level service table [verified: capture].
    pub const SET_GLOBALS: u8 = 0x81;
    /// Replicator items [verified: capture].
    pub const DATA: u8 = 0x83;
    /// `84 01 00 00 00` once after the schema [verified: capture].
    pub const MARKER: u8 = 0x84;
    /// Flag/protocol sync: u8, u16 LE count, (u16-LE-len key, u16-LE-len value)* [verified: capture].
    pub const PROTOCOL_SYNC: u8 = 0x93;
    /// The replication schema [verified: capture].
    pub const NEW_SCHEMA: u8 = 0x97;
    /// CSG / SolidMesh geometry fragments [inferred].
    pub const SOLID_MESH: u8 = 0xa5;
}

/// The replicator item kinds. Names match the exe's RTTI (`NewObjectItem`,
/// `ChangePropertyItem`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    NewObject,
    DeleteInstance,
    DestroyInstance,
    ChangeProperty,
    ChangeAttribute,
    EventInvocation,
    Marker,
    Ping,
    Tag,
    JoinData,
    Cluster,
    SharedString,
    StreamData,
    InstanceIntent,
    ResolveNewInstanceAtomicTree,
    SharedReferencePropertyChanged,
    Stats,
}

/// A decoded replicator op in rustudio's own terms: what to do to the live
/// DataModel. Instance identity is the server's referent (a wire id); the
/// model maps it to an rbx-dom [`Ref`].
#[derive(Debug, Clone)]
pub enum RepOp {
    /// The initial place stream has begun; following ops build the DataModel.
    JoinDataBegin,
    /// A new instance: server referent, class name, parent referent (None =
    /// a top-level service/DataModel child), and its initial properties.
    NewInstance {
        id: u64,
        class: String,
        parent: Option<u64>,
        props: Vec<(String, PropValue)>,
    },
    /// A property changed on an existing instance.
    ChangeProperty {
        id: u64,
        name: String,
        value: PropValue,
    },
    /// An instance was removed.
    DeleteInstance { id: u64 },
    /// An instance moved under a new parent (item 0x13: post-join batch
    /// roots arrive with a nil parent and are attached this way).
    Reparent { id: u64, parent: u64 },
    /// A collaborator presence update (camera, selection, status).
    Presence { user_id: u64 },
    /// A remote event fired on an instance (`Class.Event` and its args).
    Event {
        id: u64,
        name: String,
        args: Vec<PropValue>,
    },
    /// Anything we recognise the kind of but can't decode the body of yet.
    Unhandled { kind: ItemKind },
    /// A net asset (a union's SolidMeshHolder contents) from the 0xa5
    /// channel, keyed like the holder property's reference (`net_asset_key`).
    NetAsset { key: u64, data: Vec<u8> },
}

/// A property value in transit.
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    Bool(bool),
    Int(i64),
    /// Explicit int64 wire value (0x2c), including values that fit in i32.
    Int64(i64),
    Float(f64),
    Float32(f32),
    String(String),
    /// A string-typed wire value containing arbitrary non-UTF-8 bytes.
    BinaryString(Vec<u8>),
    /// A reference to another instance by server referent.
    Ref(Option<u64>),
    Enum(u32),
    Vector2([f32; 2]),
    Vector3([f32; 3]),
    Color3([f32; 3]),
    Color3uint8([u8; 3]),
    /// Position, then the row-major 3x3 rotation.
    CFrame {
        pos: [f32; 3],
        rot: [f32; 9],
    },
    Array(Vec<PropValue>),
    Dictionary(Vec<(String, PropValue)>),
    /// A 16-byte shared-string hash (ProtectedString sources, SharedString).
    SharedHash([u8; 16]),
    /// Resolved binary contents of a replicated shared/protected string.
    SharedString(Vec<u8>),
    CachedSharedString {
        hash: [u8; 16],
        bytes: Vec<u8>,
    },
    /// Decoded but not modelled further: raw bytes kept for analysis.
    Raw(Vec<u8>),
    /// A value of wire type `ty` whose payload the model converts to a
    /// Roblox type (UDim/UDim2, Vector3int16, NumberSequence, NumberRange,
    /// ColorSequence, PhysicalProperties, Font). `bytes` is laid out as
    /// `Raw` was for the same type (sequences without their count,
    /// PhysicalProperties without its flag byte), so the writer sends it
    /// unchanged.
    Typed { ty: u8, bytes: Vec<u8> },
}

/// A parse failure. `Incomplete` means the input ended mid-item (a record
/// crossing a chunk boundary): retry with more bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Incomplete,
    Bad(String),
}

type R<T> = Result<T, DecodeError>;
type Props = Vec<(String, PropValue)>;

fn bad<T>(msg: impl Into<String>) -> R<T> {
    Err(DecodeError::Bad(msg.into()))
}

/// A byte cursor.
struct Cur<'a> {
    d: &'a [u8],
    o: usize,
}

impl<'a> Cur<'a> {
    fn new(d: &'a [u8]) -> Self {
        Cur { d, o: 0 }
    }
    fn left(&self) -> usize {
        self.d.len() - self.o
    }
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        if self.left() < n {
            return Err(DecodeError::Incomplete);
        }
        self.o += n;
        Ok(&self.d[self.o - n..self.o])
    }
    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }
    fn peek(&self) -> R<u8> {
        self.d.get(self.o).copied().ok_or(DecodeError::Incomplete)
    }
    fn u16(&mut self) -> R<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> R<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32s<const N: usize>(&mut self) -> R<[f32; N]> {
        let mut a = [0f32; N];
        for v in &mut a {
            *v = self.f32()?;
        }
        Ok(a)
    }
    /// Unsigned LEB128.
    fn varint(&mut self) -> R<u64> {
        let (mut v, mut s) = (0u64, 0u32);
        loop {
            let b = self.u8()?;
            if s < 64 {
                v |= u64::from(b & 0x7f) << s;
            }
            s += 7;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
    }
    fn zigzag(&mut self) -> R<i64> {
        let v = self.varint()?;
        Ok((v >> 1) as i64 ^ -((v & 1) as i64))
    }
    fn str_n(&mut self, n: usize) -> R<String> {
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn varstr(&mut self) -> R<String> {
        let n = self.varint()? as usize;
        self.str_n(n)
    }
    /// `01 <u32 LE id>` or `00` (nil).
    fn reference(&mut self) -> R<Option<u64>> {
        // Native serializer  writes a varint scope followed by
        // the u32 instance index. Scope 1 is the server's usual namespace;
        // peer-created instances retain their own scope.
        let scope = self.varint()?;
        if scope == 0 {
            return Ok(None);
        }
        if scope > u32::MAX as u64 {
            return bad("reference scope exceeds u32");
        }
        let index = u64::from(self.u32()?);
        Ok(Some(if scope == 1 {
            index
        } else {
            (scope << 32) | index
        }))
    }
}

/// One property of a schema class.
#[derive(Debug, Clone)]
pub struct SchemaProp {
    pub name: String,
    pub ty: u8,
}

#[derive(Debug, Clone)]
pub struct SchemaEvent {
    pub name: String,
    pub arg_types: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SchemaClass {
    pub name: String,
    pub props: Vec<SchemaProp>,
    pub events: Vec<SchemaEvent>,
}

/// The replication schema (packet 0x97). Items index classes, class-local
/// props, global props and global events through it.
#[derive(Debug, Clone, Default)]
pub struct Schema {
    pub enums: Vec<String>,
    pub classes: Vec<SchemaClass>,
    /// (class index, prop index) by global prop index.
    pub global_props: Vec<(u16, u16)>,
    /// (class index, event index) by global event index.
    pub global_events: Vec<(u16, u16)>,
    pub content_prefixes: Vec<String>,
    /// The common-name dictionary used by Name values (`0x80 | index`).
    pub names: Vec<String>,
}

impl Schema {
    /// Parse the decompressed 0x97 body.
    pub fn parse(d: &[u8]) -> Result<Schema, DecodeError> {
        let mut c = Cur::new(d);
        c.take(9)?;
        let mut s = Schema::default();
        for _ in 0..c.varint()? {
            s.enums.push(c.varstr()?);
            c.u8()?;
        }
        let n_classes = c.varint()?;
        c.varint()?;
        c.varint()?;
        for ci in 0..n_classes {
            let name = c.varstr()?;
            let mut props = Vec::new();
            for pi in 0..c.varint()? {
                let pname = c.varstr()?;
                let ty = c.u8()?;
                match ty {
                    0x07 => {
                        c.take(2)?;
                    }
                    0x3c => {
                        c.take(1)?;
                    }
                    _ => {}
                }
                s.global_props.push((ci as u16, pi as u16));
                props.push(SchemaProp { name: pname, ty });
            }
            c.take(2)?;
            let mut events = Vec::new();
            for ei in 0..c.varint()? {
                let ename = c.varstr()?;
                let mut arg_types = Vec::new();
                for _ in 0..c.varint()? {
                    let a = c.take(3)?;
                    arg_types.push(a[0]);
                }
                s.global_events.push((ci as u16, ei as u16));
                events.push(SchemaEvent {
                    name: ename,
                    arg_types,
                });
            }
            s.classes.push(SchemaClass {
                name,
                props,
                events,
            });
        }
        for _ in 0..c.varint()? {
            s.content_prefixes.push(c.varstr()?);
        }
        for _ in 0..c.varint()? {
            s.names.push(c.varstr()?);
        }
        Ok(s)
    }

    /// Decode one property value of schema type `ty`.
    fn value(&self, ty: u8, c: &mut Cur) -> R<PropValue> {
        self.value_at_depth(ty, c, 0)
    }

    fn value_at_depth(&self, ty: u8, c: &mut Cur, depth: usize) -> R<PropValue> {
        if depth > 32 {
            return bad("nested value depth");
        }
        Ok(match ty {
            0x02 | 0x21 => {
                let n=c.varint()? as usize;
                let bytes=c.take(n)?.to_vec();
                match String::from_utf8(bytes) {Ok(text)=>PropValue::String(text),Err(error)=>PropValue::BinaryString(error.into_bytes())}
            },
            0x06 | 0x2e => PropValue::SharedHash(c.take(16)?.try_into().unwrap()),
            0x07 => PropValue::Enum(c.varint()? as u32),
            0x08 => {
                let n = c.varint()? as usize;
                PropValue::Raw(c.take(n)?.to_vec())
            }
            0x09 => PropValue::Bool(c.u8()? != 0),
            0x0a => PropValue::Int(c.zigzag()?),
            0x2c => PropValue::Int64(c.zigzag()?),
            0x0b => PropValue::Float32(c.f32()?),
            0x0c => PropValue::Float(f64::from_le_bytes(c.take(8)?.try_into().unwrap())),
            0x0d => PropValue::Typed { ty, bytes: c.take(8)?.to_vec() },
            0x0e => PropValue::Typed { ty, bytes: c.take(16)?.to_vec() },
            0x12 => PropValue::Int(i64::from(c.u16()?)),
            0x13 => PropValue::Color3(c.f32s()?),
            0x14 => PropValue::Color3uint8(c.take(3)?.try_into().unwrap()),
            0x15 => PropValue::Vector2(c.f32s()?),
            0x16 => PropValue::Vector3(c.f32s()?),
            0x19 => PropValue::Typed { ty, bytes: c.take(6)?.to_vec() },
            0x1a => {
                let pos = c.f32s()?;
                let rot = rotation(c.u8()?, c)?;
                PropValue::CFrame { pos, rot }
            }
            0x1c => PropValue::Ref(c.reference()?),
            // Captured event containers: count, then type-tagged values;
            // dictionaries additionally prefix each value with a key string.
            0x1d..=0x1f => {
                let count = c.varint()? as usize;
                if count > c.left() { return Err(DecodeError::Incomplete); }
                let mut array = Vec::new();
                let mut dictionary = Vec::new();
                for _ in 0..count {
                    let key = if ty == 0x1f { Some(c.varstr()?) } else {None};
                    let element_ty = c.u8()?;
                    let value=self.value_at_depth(element_ty,c,depth+1)?;
                    if let Some(key)=key {dictionary.push((key,value));} else {array.push(value);}
                }
                if ty==0x1f {PropValue::Dictionary(dictionary)} else {PropValue::Array(array)}
            }
            // Captured Player.User identity: four varints. Keep opaque;
            // the following property is the separately encoded UserId.
            0x44 => {
                let start = c.o;
                for _ in 0..4 {
                    c.varint()?;
                }
                PropValue::Raw(c.d[start..c.o].to_vec())
            }
            0x22 => PropValue::Int(c.varint()? as i64),
            0x23 | 0x26 => {
                let n = c.u32()? as usize;
                if n > 1024 {
                    return bad(format!("sequence of {n}"));
                }
                let w = if ty == 0x23 { 12 } else { 20 };
                PropValue::Typed { ty, bytes: c.take(n * w)?.to_vec() }
            }
            0x25 => PropValue::Typed { ty, bytes: c.take(8)?.to_vec() },
            // Rect (ImageLabel/ImageButton.SliceCenter): min x, y, max x, y.
            0x28 => PropValue::Typed { ty, bytes: c.take(16)?.to_vec() },
            0x29 => {
                let fl = c.u8()?;
                let n = if fl == 0 {
                    0
                } else if fl & 2 != 0 {
                    24
                } else {
                    20
                };
                PropValue::Typed { ty, bytes: c.take(n)?.to_vec() }
            }
            0x31 => {
                let b = c.peek()?;
                if b & 0x80 != 0 {
                    c.u8()?;
                    match self.names.get(usize::from(b & 0x7f)) {
                        Some(n) => PropValue::String(n.clone()),
                        None => return bad(format!("name dictionary index {}", b & 0x7f)),
                    }
                } else {
                    PropValue::String(c.varstr()?)
                }
            }
            0x32 => {
                let b = c.u8()?;
                if b == 0 {
                    PropValue::Raw(Vec::new())
                } else {
                    let rot = rotation(b & 0x7f, c)?;
                    let pos = c.f32s()?;
                    PropValue::CFrame { pos, rot }
                }
            }
            0x34 => PropValue::Raw(c.take(16)?.to_vec()),
            0x36 => {
                let start = c.o;
                c.take(3)?;
                let n = c.u32()? as usize;
                c.take(n)?;
                let n2 = c.u32()? as usize;
                c.take(n2)?;
                PropValue::Typed { ty, bytes: c.d[start..c.o].to_vec() }
            }
            0x37 => PropValue::Raw(c.take(8)?.to_vec()),
            0x3c => {
                let start = c.o;
                let n = c.u32()?;
                if n > 64 {
                    return bad(format!("solid mesh count {n}"));
                }
                for _ in 0..n {
                    c.take(3)?;
                    c.varint()?;
                    c.take(8)?;
                }
                PropValue::Typed { ty, bytes: c.d[start..c.o].to_vec() }
            }
            0x3d => match c.u8()? {
                0 => PropValue::String(String::new()),
                1 => PropValue::String(c.varstr()?),
                // Object (Content.fromObject: an EditableImage /
                // EditableMesh): an instance reference, as type 0x1c. Those
                // objects never replicate, so the property stays at its
                // default; an unknown kind here used to stop the whole live
                // session (a live Place A session, 2026-11-07).
                2 => {
                    let start = c.o;
                    c.reference()?;
                    PropValue::Raw(c.d[start..c.o].to_vec())
                }
                k => return bad(format!("content kind {k:#x}")),
            },
            t => return bad(format!("value type {t:#x}")),
        })
    }
}

/// A CFrame rotation: id 0 = nine f32 follow, else a basic (axis-aligned)
/// rotation id as in the binary place format.
fn rotation(id: u8, c: &mut Cur) -> R<[f32; 9]> {
    if id == 0 {
        return c.f32s();
    }
    match rbx_types::Matrix3::from_basic_rotation_id(id) {
        Ok(m) => Ok([
            m.x.x, m.x.y, m.x.z, m.y.x, m.y.y, m.y.z, m.z.x, m.z.y, m.z.z,
        ]),
        Err(_) => bad(format!("rotation id {id:#x}")),
    }
}

/// A net asset reference as the SolidMeshHolder property and the 0xa5 begin
/// message both write it: `u32 1, u8 1, u16 0, u8 tag, varint id`; the key
/// packs the tag and id.
fn net_asset_key(c: &mut Cur) -> R<u64> {
    c.take(4)?;
    c.take(3)?;
    let tag = c.u8()? as u64;
    let id = c.varint()?;
    Ok((tag << 56) | (id & 0x00ff_ffff_ffff_ffff))
}

/// The key of the first reference in a SolidMeshHolder property's bytes.
pub fn holder_key(bytes: &[u8]) -> Option<u64> {
    let mut c = Cur::new(bytes);
    net_asset_key(&mut c).ok()
}

// A local resource limit, not a claimed protocol maximum.
const MAX_NET_ASSET_BYTES: usize = 64 * 1024 * 1024;

/// A raw LZ4 block (no frame), decoded until the input ends.
pub fn lz4_block(src: &[u8]) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(src.len().min(4096));
    let mut i = 0usize;
    let len = |i: &mut usize, mut n: usize| -> Option<usize> {
        if n == 15 {
            loop {
                let v = *src.get(*i)? as usize;
                *i += 1;
                n += v;
                if v != 255 {
                    break;
                }
            }
        }
        Some(n)
    };
    while i < src.len() {
        let tok = src[i];
        i += 1;
        let ll = len(&mut i, (tok >> 4) as usize)?;
        if out.len().checked_add(ll)? > MAX_NET_ASSET_BYTES { return None; }
        out.extend_from_slice(src.get(i..i.checked_add(ll)?)?);
        i += ll;
        if i >= src.len() {
            break;
        }
        let off = u16::from_le_bytes([*src.get(i)?, *src.get(i + 1)?]) as usize;
        i += 2;
        if off == 0 || off > out.len() {
            return None;
        }
        let ml = len(&mut i, (tok & 15) as usize)? + 4;
        if out.len().checked_add(ml)? > MAX_NET_ASSET_BYTES { return None; }
        let start = out.len() - off;
        for k in 0..ml {
            let b = out[start + k];
            out.push(b);
        }
    }
    Some(out)
}

/// Split a reassembled `rbx-rtcio` stream into channel, tag and messages.
pub fn split_stream(b: &[u8]) -> Option<(u8, [u8; 4], Vec<&[u8]>)> {
    if b.len() < 7 || b[0] != 0x06 || b[1] != 0x01 {
        return None;
    }
    let mut msgs = Vec::new();
    let mut o = 7;
    while o + 4 <= b.len() {
        let w = u32::from_be_bytes(b[o..o + 4].try_into().unwrap());
        let n = (w & 0x7fff_ffff) as usize;
        if w >> 31 == 0 || o + 4 + n > b.len() {
            break;
        }
        msgs.push(&b[o + 4..o + 4 + n]);
        o += 4 + n;
    }
    Some((b[2], b[3..7].try_into().unwrap(), msgs))
}

fn unzstd(d: &[u8], size: usize) -> R<Vec<u8>> {
    let out = zstd::bulk::decompress(d, size.max(1))
        .map_err(|e| DecodeError::Bad(format!("zstd: {e}")))?;
    if out.len() != size {
        return bad("zstd size mismatch");
    }
    Ok(out)
}

/// SET_GLOBALS assigns the client's GUID namespace after seven flags.
/// Native client parser: , , ...
/// The second flag enables an optional terrain block, not supported here.
pub fn assigned_peer_scope(body: &[u8]) -> Result<u32, String> {
    let mut c = Cur::new(body);
    c.u8().map_err(|e| format!("{e:?}"))?;
    if c.u8().map_err(|e| format!("{e:?}"))? != 0 {
        return Err("terrain globals block".into());
    }
    c.take(5).map_err(|e| format!("{e:?}"))?;
    let scope = c.varint().map_err(|e| format!("{e:?}"))?;
    if scope <= 1 || scope > u32::MAX as u64 {
        return Err("invalid client namespace".into());
    }
    Ok(scope as u32)
}

/// The stateful decoder for one replication stream (server → client).
#[derive(Default)]
pub struct Decoder {
    pub schema: Option<Schema>,
    /// Shared strings by hash (script sources and other blobs).
    pub shared: HashMap<[u8; 16], Vec<u8>>,
    /// Undecoded tails of the initial-stream stages (records cross chunks).
    stage_buf: HashMap<u8, Vec<u8>>,
    began: bool,
    /// Items or values the decoder could not parse (kept for diagnostics).
    pub errors: Vec<String>,
    /// The 0xa5 transfer in progress: its key, compressed length, chunks.
    net_asset: Option<(u64, usize, u16, Vec<u8>)>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode one message (`packet_id` = its first byte, `body` = the rest).
    pub fn decode_frame(&mut self, packet_id: u8, body: &[u8]) -> Vec<RepOp> {
        let mut ops = Vec::new();
        let r = match packet_id {
            packet_id::NEW_SCHEMA => self.schema_msg(body),
            packet_id::DATA => self.data_msg(body, &mut ops),
            packet_id::SOLID_MESH => self.net_asset_msg(body, &mut ops),
            _ => Ok(()),
        };
        if let Err(e) = r {
            self.errors.push(format!("packet {packet_id:#x}: {e:?}"));
        }
        for op in &mut ops {
            match op {
                RepOp::NewInstance { props, .. } => {
                    for (_, value) in props {
                        self.resolve_shared(value);
                    }
                }
                RepOp::ChangeProperty { value, .. } => self.resolve_shared(value),
                _ => {}
            }
        }
        ops
    }

    /// The 0xa5 channel (2026-11-05, from the Place B [DEV] join capture): each
    /// transfer is `02 fe 04 <ref> 00 <varint compressed length>` (begin; ref as
    /// in the SolidMeshHolder property), then `01 fe <u16 LE chunk index>
    /// <≤512 bytes>` chunks, then `03 fe <varint>` (end); `06 01 <varint>`
    /// carries no geometry. The joined chunks are a varint of the decompressed
    /// size, then an LZ4 block: the holder blob without its `03 01` prefix
    /// (`<u32 size> … <u32 size> SolidMesh …`).
    fn net_asset_msg(&mut self, body: &[u8], ops: &mut Vec<RepOp>) -> R<()> {
        let mut c = Cur::new(body);
        match c.u8()? {
            2 => {
                c.u8()?; // fe
                c.u8()?; // 04
                let key = net_asset_key(&mut c)?;
                c.u8()?; // 00 (where the holder property has its u32 3)
                let len = c.varint()? as usize;
                if len > MAX_NET_ASSET_BYTES {
                    self.net_asset = None;
                    return bad("net asset compressed size exceeds limit");
                }
                self.net_asset = Some((key, len, 0, Vec::new()));
            }
            1 => {
                c.u8()?;
                let index = c.u16()?;
                if let Some((_, len, next, buf)) = &mut self.net_asset {
                    let chunk = &body[c.o..];
                    if index != *next || chunk.len() > 512 || buf.len() + chunk.len() > *len {
                        self.net_asset = None;
                        return bad("net asset chunk index or size mismatch");
                    }
                    *next = next.wrapping_add(1);
                    buf.extend_from_slice(chunk);
                }
            }
            3 => {
                if let Some((key, len, _, buf)) = self.net_asset.take() {
                    if buf.len() != len {
                        return bad("net asset truncated transfer");
                    }
                    let data = &buf[..];
                    // A varint of the decompressed size, then the LZ4 block.
                    let mut h = Cur::new(data);
                    let size = usize::try_from(h.varint()?).map_err(|_| DecodeError::Bad("net asset size overflow".into()))?;
                    if size > MAX_NET_ASSET_BYTES {
                        return bad("net asset decoded size exceeds limit");
                    }
                    match lz4_block(&data[h.o..]).filter(|o| o.len() == size) {
                        Some(mut out) => {
                            let mut holder = vec![3u8, 1];
                            holder.append(&mut out);
                            ops.push(RepOp::NetAsset { key, data: holder });
                        }
                        None => self.errors.push(format!("net asset {key}: LZ4 data didn't decode")),
                    }
                }
            }
            // `06 01 <varint>`: no geometry (3 in the Place B join; release or ack, not decoded).
            6 => {}
            k => return bad(format!("net asset message kind {k}")),
        }
        Ok(())
    }

    fn resolve_shared(&self, value: &mut PropValue) {
        if let PropValue::SharedHash(hash) = value {
            if let Some(bytes) = self.shared.get(hash) {
                *value = PropValue::CachedSharedString {
                    hash: *hash,
                    bytes: bytes.clone(),
                };
            }
        }
    }

    fn schema_msg(&mut self, body: &[u8]) -> R<()> {
        let mut c = Cur::new(body);
        let clen = c.u32()? as usize;
        let size = c.u32()? as usize;
        let d = unzstd(c.take(clen)?, size)?;
        self.schema = Some(Schema::parse(&d)?);
        Ok(())
    }

    fn data_msg(&mut self, body: &[u8], ops: &mut Vec<RepOp>) -> R<()> {
        let mut c = Cur::new(body);
        while c.left() > 0 && c.peek()? != 0 {
            self.item(&mut c, ops)?;
        }
        Ok(())
    }

    fn schema(&self) -> R<&Schema> {
        self.schema
            .as_ref()
            .ok_or(DecodeError::Bad("no schema yet".into()))
    }

    fn item(&mut self, c: &mut Cur, ops: &mut Vec<RepOp>) -> R<()> {
        let kind = c.u8()?;
        match kind {
            // initial-stream chunk: stage, clen, size, zstd, [u32 u8 unless stage 1]
            0x16 => {
                let stage = c.u8()?;
                let clen = c.u32()? as usize;
                let size = c.u32()? as usize;
                let d = unzstd(c.take(clen)?, size)?;
                if stage != 1 {
                    // JoinDataItemV2 writes a u32, a varint pair count, then
                    // that many (u32-varint, u32-varint) metadata pairs.
                    // Serializer: ... Empty tables
                    // looked like a fixed five-byte trailer in early captures.
                    c.u32()?;
                    let count = c.varint()? as usize;
                    if count > c.left() / 2 {
                        return Err(DecodeError::Incomplete);
                    }
                    for _ in 0..count {
                        c.varint()?;
                        c.varint()?;
                    }
                }
                if !self.began {
                    self.began = true;
                    ops.push(RepOp::JoinDataBegin);
                }
                if stage == 1 {
                    self.shared_strings(&d)?;
                } else {
                    let mut buf = self.stage_buf.remove(&stage).unwrap_or_default();
                    buf.extend_from_slice(&d);
                    let r = self.records(&buf, None, ops);
                    let used = match &r {
                        Ok(n) => *n,
                        // keep the stream aligned for the next chunk: drop this chunk's bytes
                        Err(_) => buf.len(),
                    };
                    buf.drain(..used);
                    self.stage_buf.insert(stage, buf);
                    r.map_err(|e| DecodeError::Bad(format!("stage {stage}: {e:?}")))?;
                }
            }
            // post-join instance batch: count, optional compressed records,
            // shared-string entries, then compact region markers.
            0x0d => {
                let count = c.u32()? as usize;
                if count != 0 {
                    let clen = c.u32()? as usize;
                    let size = c.u32()? as usize;
                    let d = unzstd(c.take(clen)?, size)?;
                    let used = self
                        .records(&d, Some(count), ops)
                        .map_err(|e| DecodeError::Bad(format!("instance batch: {e:?}")))?;
                    self.shared_strings(&d[used..])?;
                }
                // StreamDataItem writes zero or more compact region markers
                // after the records. Empty batches omit the compressed block.
                // Native serializer  / .
                loop {
                    let flags = c.u8()?;
                    match flags & 7 {
                        0 => match c.u8()? {
                            0 => {
                                c.take(3)?;
                            }
                            1 => {
                                c.take(12)?;
                            }
                            _ => return bad("stream region coordinate mode"),
                        },
                        1..=4 => {}
                        _ => return bad("stream region flags"),
                    }
                    if flags & 8 == 0 {
                        break;
                    }
                }
            }
            // DeleteInstanceItem: observed when a peer destroys its probe.
            0x01 => {
                if let Some(id) = c.reference()? {
                    ops.push(RepOp::DeleteInstance { id });
                }
            }
            // new instance: ref, class, ff, props, ff, parent
            0x02 => {
                let id = c.reference()?;
                let ci = c.u16()?;
                // NewObject carries an early property group (e.g. LocalScript
                // Source), then the ordinary properties. Player's early group
                // is empty (ff); it is not a universal fixed marker.
                let (_, mut early_props) = self.props(ci, c)?;
                let (class, props) = self.props(ci, c)?;
                early_props.extend(props);
                for (_, value) in &early_props {
                    if let PropValue::SharedHash(expected) = value {
                        let hash: [u8; 16] = c.take(16)?.try_into().unwrap();
                        if &hash != expected {
                            return bad("new-object shared-string hash");
                        }
                        if c.u8()? != 0 {
                            return bad("new-object shared-string encoding");
                        }
                        let len = c.varint()? as usize;
                        self.shared.insert(hash, c.take(len)?.to_vec());
                    }
                }
                let props = early_props;
                let parent = c.reference()?;
                if let Some(id) = id {
                    ops.push(RepOp::NewInstance {
                        id,
                        class,
                        parent,
                        props,
                    });
                }
            }
            // property change: ref, u16 global prop, u8, value
            0x03 => {
                let id = c.reference()?;
                let gi = usize::from(c.u16()?);
                c.u8()?;
                let s = self.schema()?;
                // Parent is a reserved reference descriptor immediately after
                // the ordinary property dictionary (not a class-local prop).
                if gi == s.global_props.len() {
                    let parent = c.reference()?.unwrap_or(0);
                    if let Some(id) = id {
                        ops.push(RepOp::Reparent { id, parent });
                    }
                    return Ok(());
                }
                let &(ci, pi) = s
                    .global_props
                    .get(gi)
                    .ok_or(DecodeError::Bad(format!("global prop {gi}")))?;
                let cls = &s.classes[usize::from(ci)];
                let p = &cls.props[usize::from(pi)];
                let value = s.value(p.ty, c)?;
                let name = p.name.clone();
                let collaborator = cls.name == "Collaborator";
                if let PropValue::SharedHash(expected) = &value {
                    let hash:[u8;16]=c.take(16)?.try_into().unwrap();
                    if &hash!=expected {return bad("property shared-string hash");}
                    if c.u8()?!=0 {return bad("property shared-string encoding");}
                    let len=c.varint()? as usize;
                    self.shared.insert(hash,c.take(len)?.to_vec());
                }
                if let Some(id) = id {
                    if collaborator {ops.push(RepOp::Presence { user_id: id });}
                    ops.push(RepOp::ChangeProperty {id,name,value});
                }
            }
            // event: ref, u16 global event, args
            0x07 => {
                let id = c.reference()?;
                let gi = usize::from(c.u16()?);
                let s = self.schema()?;
                let &(ci, ei) = s
                    .global_events
                    .get(gi)
                    .ok_or(DecodeError::Bad(format!("global event {gi}")))?;
                let cls = &s.classes[usize::from(ci)];
                let e = &cls.events[usize::from(ei)];
                let mut args = Vec::new();
                for &t in &e.arg_types {
                    args.push(s.value(t, c)?);
                }
                ops.push(RepOp::Event {
                    id: id.unwrap_or(0),
                    name: format!("{}.{}", cls.name, e.name),
                    args,
                });
            }
            // Marker id, not a count: capture marker 13 precedes nine items.
            0x10 => {
                c.u32()?;
            }
            // (child, new parent) [verified: capture: every nil-parent root of
            // the 0x0d batches is reparented by one of these]
            0x13 => {
                let child = c.reference()?;
                let parent = c.reference()?;
                if let (Some(id), Some(parent)) = (child, parent) {
                    ops.push(RepOp::Reparent { id, parent });
                }
            }
            // StreamDataInfoItem, serializer : compact region
            // coordinate, then two u32 fields. Studio 0.746 capabilities do
            // not enable the optional third u32 (capability bit 56).
            0x14 => {
                match c.u8()? {
                    0 => {
                        c.take(3)?;
                    }
                    1 => {
                        c.take(12)?;
                    }
                    _ => return bad("stream-info coordinate mode"),
                }
                c.take(8)?;
                ops.push(RepOp::Unhandled {
                    kind: ItemKind::StreamData,
                });
            }
            0x23 => {
                c.u32()?;
            }
            // Fixed-size captured ping bodies may precede other items.
            0x05 | 0x06 => {
                c.take(if kind == 0x05 { 26 } else { 46 })?;
                ops.push(RepOp::Unhandled {
                    kind: ItemKind::Ping,
                });
            }
            k => return bad(format!("item {k:#x} at {}", c.o)),
        }
        Ok(())
    }

    fn shared_strings(&mut self, d: &[u8]) -> R<()> {
        let mut c = Cur::new(d);
        while c.left() > 0 {
            let h: [u8; 16] = c.take(16)?.try_into().unwrap();
            c.u8()?;
            let n = c.varint()? as usize;
            self.shared.insert(h, c.take(n)?.to_vec());
        }
        Ok(())
    }

    /// Class-local props until `ff`.
    fn props(&self, ci: u16, c: &mut Cur) -> R<(String, Props)> {
        let s = self.schema()?;
        let cls = s
            .classes
            .get(usize::from(ci))
            .ok_or(DecodeError::Bad(format!("class {ci}")))?;
        let mut props = Vec::new();
        loop {
            let pi = c.u8()?;
            if pi == 0xff {
                break;
            }
            let p = cls
                .props
                .get(usize::from(pi))
                .ok_or(DecodeError::Bad(format!("{}: prop {pi}", cls.name)))?;
            let v = s.value(p.ty, c)?;
            props.push((p.name.clone(), v));
        }
        Ok((cls.name.clone(), props))
    }

    /// Parse complete `01 <u32 id> <u16 class> props ff <parent>` records
    /// (at most `max`); returns the bytes consumed (a trailing partial record
    /// is left).
    fn records(&self, d: &[u8], max: Option<usize>, ops: &mut Vec<RepOp>) -> R<usize> {
        let mut c = Cur::new(d);
        let mut done = 0;
        let mut n = 0;
        while c.left() > 0 && max.is_none_or(|m| n < m) {
            let r = (|| {
                let id = c
                    .reference()?
                    .ok_or_else(|| DecodeError::Bad("nil instance record".into()))?;
                let ci = c.u16()?;
                let (class, props) = self.props(ci, &mut c)?;
                let parent = c.reference()?;
                Ok(RepOp::NewInstance {
                    id,
                    class,
                    parent,
                    props,
                })
            })();
            match r {
                Ok(op) => {
                    ops.push(op);
                    done = c.o;
                    n += 1;
                }
                Err(DecodeError::Incomplete) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(done)
    }
}

/// Maps the server's instance referents to rbx-dom [`Ref`]s as instances
/// arrive. Pending parents (a child seen before its parent) are not resolved
/// here; the model holds them.
#[derive(Default)]
pub struct RefMap {
    fwd: std::collections::HashMap<u64, Ref>,
    reverse: std::collections::HashMap<Ref, u64>,
}

impl RefMap {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn get(&self, server_id: u64) -> Option<Ref> {
        self.fwd.get(&server_id).copied()
    }
    pub fn insert(&mut self, server_id: u64, r: Ref) {
        if let Some(old_id) = self.reverse.insert(r, server_id) {
            self.fwd.remove(&old_id);
        }
        if let Some(old_ref) = self.fwd.insert(server_id, r) {
            if old_ref != r {
                self.reverse.remove(&old_ref);
            }
        }
    }
    pub fn server_id(&self, reference: Ref) -> Option<u64> {
        self.reverse.get(&reference).copied()
    }
    pub fn len(&self) -> usize {
        self.fwd.len()
    }
    pub fn is_empty(&self) -> bool {
        self.fwd.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globals_assign_the_client_namespace_after_flags() {
        assert_eq!(
            assigned_peer_scope(&[1, 0, 0, 0, 0, 0, 1, 0xac, 2, 0, 0, 0]).unwrap(),
            300
        );
        assert!(assigned_peer_scope(&[1, 1, 0, 0, 0, 0, 1, 5]).is_err());
        assert!(assigned_peer_scope(&[1, 0, 0, 0, 0, 0, 1, 1]).is_err());
    }

    #[test]
    fn peer_scoped_delete_preserves_following_marker() {
        let mut dec = decoder();
        let body = [1, 5, 0xf5, 0xce, 0x0e, 0, 0x10, 123, 0, 0, 0, 0];
        let ops = dec.decode_frame(0x83, &body);
        assert!(dec.errors.is_empty(), "{:?}", dec.errors);
        assert!(matches!(ops[0], RepOp::DeleteInstance { id } if id == (5u64 << 32) | 0xecef5));
    }

    #[test]
    fn empty_stream_batch_regions_preserve_next_item() {
        let mut dec = decoder();
        // Zero records, full region coordinates, then a compact delta region.
        let mut body = vec![0x0d, 0, 0, 0, 0, 8, 1];
        for v in [-64i32, -64, -64] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.push(3);
        body.push(0x10);
        body.extend_from_slice(&123u32.to_le_bytes());
        body.push(0);
        dec.decode_frame(0x83, &body);
        assert!(dec.errors.is_empty(), "{:?}", dec.errors);
    }

    fn varstr(out: &mut Vec<u8>, s: &str) {
        out.push(s.len() as u8);
        out.extend_from_slice(s.as_bytes());
    }

    /// A tiny schema in the 0x97 wire layout: enum `Material`; classes
    /// `Collaborator` (Status enum, Username string) and `Part`
    /// (CFrame, Name, Anchored); event `Part.Touched(Instance)`; name dict
    /// ["Baseplate"].
    fn schema_body() -> Vec<u8> {
        let mut d = vec![0x03, 0x01, 0, 0, 0, 0, 0, 0, 0];
        d.push(1);
        varstr(&mut d, "Material");
        d.push(11);
        d.extend_from_slice(&[2, 5, 1]);
        varstr(&mut d, "Collaborator");
        d.push(2);
        varstr(&mut d, "Status");
        d.extend_from_slice(&[0x07, 0x00, 0x00]);
        varstr(&mut d, "Username");
        d.push(0x02);
        d.extend_from_slice(&[0xff, 0xff, 0]);
        varstr(&mut d, "Part");
        d.push(3);
        varstr(&mut d, "CFrame");
        d.push(0x1a);
        varstr(&mut d, "Name");
        d.push(0x31);
        varstr(&mut d, "Anchored");
        d.push(0x09);
        d.extend_from_slice(&[0xff, 0xff, 1]);
        varstr(&mut d, "Touched");
        d.extend_from_slice(&[1, 0x1c, 0xff, 0xff]);
        d.push(1);
        varstr(&mut d, "rbxassetid://");
        d.push(1);
        varstr(&mut d, "Baseplate");
        d
    }

    fn zmsg(prefix: &[u8], d: &[u8], counts: bool) -> Vec<u8> {
        let z = zstd::bulk::compress(d, 3).unwrap();
        let mut m = prefix.to_vec();
        if counts {
            m.extend_from_slice(&(z.len() as u32).to_le_bytes());
            m.extend_from_slice(&(d.len() as u32).to_le_bytes());
        }
        m.extend_from_slice(&z);
        m
    }

    fn decoder() -> Decoder {
        let mut dec = Decoder::new();
        let m = zmsg(&[], &schema_body(), true);
        assert!(dec.decode_frame(0x97, &m).is_empty());
        assert!(dec.errors.is_empty(), "{:?}", dec.errors);
        dec
    }

    #[test]
    fn parses_schema_and_global_indices() {
        let dec = decoder();
        let s = dec.schema.as_ref().unwrap();
        assert_eq!(s.classes.len(), 2);
        assert_eq!(s.global_props.len(), 5);
        assert_eq!(s.global_props[3], (1, 1)); // Part.Name
        assert_eq!(s.global_events, vec![(1, 0)]);
        assert_eq!(s.names, vec!["Baseplate"]);
    }

    #[test]
    fn splits_rtcio_stream() {
        let mut b = vec![0x06, 0x01, 0x04, 0, 0, 0, 1];
        b.extend_from_slice(&[0x80, 0, 0, 2, 0x84, 0x01]);
        b.extend_from_slice(&[0x80, 0, 0, 1, 0x93]);
        let (ch, tag, msgs) = split_stream(&b).unwrap();
        assert_eq!((ch, tag), (4, [0, 0, 0, 1]));
        assert_eq!(msgs, vec![&[0x84u8, 0x01][..], &[0x93u8][..]]);
    }

    /// Instance records split across two 0x16 chunks (as on the wire), with
    /// a full-matrix CFrame, a dictionary Name and a plain Name.
    #[test]
    fn initial_stream_records_cross_chunks() {
        let mut dec = decoder();
        let mut recs = vec![1, 0x80, 0x65, 0, 0, 1, 0]; // id 0x6580, class Part
        recs.push(0); // CFrame
        for f in [1.0f32, 2.0, 3.0] {
            recs.extend_from_slice(&f.to_le_bytes());
        }
        recs.push(0); // full rotation follows
        for f in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            recs.extend_from_slice(&f.to_le_bytes());
        }
        recs.extend_from_slice(&[1, 0x80, 2, 1, 0xff, 1, 13, 0, 0, 0]); // Name=dict[0], Anchored, parent 13
        recs.extend_from_slice(&[1, 0x81, 0x65, 0, 0, 1, 0, 1, 3]);
        recs.extend_from_slice(b"Pad");
        recs.extend_from_slice(&[0xff, 1, 0x80, 0x65, 0, 0]);
        let (a, b) = recs.split_at(30);
        let mut ops = Vec::new();
        for part in [a, b] {
            let mut m = zmsg(&[0x16, 0x04], part, true);
            m.extend_from_slice(&932u32.to_le_bytes());
            // A real populated JoinDataItemV2 metadata trailer. Both chunks
            // must leave the following item boundary intact.
            for value in [2, 91032, 197, 83668, 602] {
                crate::handshake::put_varint(&mut m, value);
            }
            m.push(0);
            ops.extend(dec.decode_frame(0x83, &m));
        }
        assert!(dec.errors.is_empty(), "{:?}", dec.errors);
        let news: Vec<_> = ops
            .iter()
            .filter_map(|o| match o {
                RepOp::NewInstance {
                    id,
                    class,
                    parent,
                    props,
                } => Some((*id, class.clone(), *parent, props.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(news.len(), 2);
        assert_eq!(news[0].0, 0x6580);
        assert_eq!(news[0].2, Some(13));
        assert_eq!(
            news[0].3[1],
            ("Name".into(), PropValue::String("Baseplate".into()))
        );
        assert_eq!(news[0].3[2], ("Anchored".into(), PropValue::Bool(true)));
        match &news[0].3[0].1 {
            PropValue::CFrame { pos, rot } => {
                assert_eq!(*pos, [1.0, 2.0, 3.0]);
                assert_eq!(rot[4], 1.0);
            }
            v => panic!("{v:?}"),
        }
        assert_eq!(
            news[1].3[0],
            ("Name".into(), PropValue::String("Pad".into()))
        );
        assert_eq!(news[1].2, Some(0x6580));
    }

    /// The Collaborator.Status change from the capture, re-indexed to the
    /// test schema: `03 <ref> <u16 global prop> 01 <enum>`, then an event.
    #[test]
    fn property_change_event_and_batch() {
        let mut dec = decoder();
        let mut m = vec![0x03, 0x01, 0x96, 0x38, 0x05, 0x00, 0x00, 0x00, 0x01, 0x01];
        m.extend_from_slice(&[
            0x07, 0x01, 0x80, 0x65, 0, 0, 0x00, 0x00, 0x01, 0x81, 0x65, 0, 0,
        ]);
        // a batch holding one new instance: Collaborator with Username "testuser1"
        m.extend_from_slice(&[
            0x10, 1, 0, 0, 0, 0x02, 0x01, 0x96, 0x38, 0x05, 0x00, 0x00, 0x00, 0xff, 0x01, 9,
        ]);
        m.extend_from_slice(b"testuser1");
        m.extend_from_slice(&[0xff, 0x01, 0x94, 0x36, 0x05, 0x00]);
        m.extend_from_slice(&[0x13, 0x01, 0x81, 0x65, 0, 0, 0x01, 0x80, 0x65, 0, 0]); // reparent
        m.push(0x00);
        let ops = dec.decode_frame(0x83, &m);
        assert!(dec.errors.is_empty(), "{:?}", dec.errors);
        assert!(matches!(ops[0], RepOp::Presence { user_id: 0x053896 }));
        assert!(
            matches!(&ops[1], RepOp::ChangeProperty { id: 0x053896, name, value: PropValue::Enum(1) } if name == "Status")
        );
        assert!(
            matches!(&ops[2], RepOp::Event { id: 0x6580, name, args } if name == "Part.Touched" && args == &vec![PropValue::Ref(Some(0x6581))])
        );
        assert!(
            matches!(&ops[3], RepOp::NewInstance { id: 0x053896, class, parent: Some(0x053694), props }
            if class == "Collaborator" && props[0].1 == PropValue::String("testuser1".into()))
        );
        assert!(matches!(
            ops[4],
            RepOp::Reparent {
                id: 0x6581,
                parent: 0x6580
            }
        ));
    }

    #[test]
    fn new_object_early_source_group_includes_inline_shared_string() {
        let mut dec = decoder();
        dec.schema.as_mut().unwrap().classes[1]
            .props
            .push(SchemaProp {
                name: "Source".into(),
                ty: 0x06,
            });
        let hash = [0x55; 16];
        let mut body = vec![2, 1, 7, 0, 0, 0, 1, 0, 3];
        body.extend_from_slice(&hash);
        body.extend_from_slice(&[0xff, 1, 4]);
        body.extend_from_slice(b"Code");
        body.push(0xff);
        body.extend_from_slice(&hash);
        body.extend_from_slice(&[0, 3]);
        body.extend_from_slice(b"abc");
        body.extend_from_slice(&[0, 0]); // nil parent, end of items
        let ops = dec.decode_frame(0x83, &body);
        assert!(dec.errors.is_empty(), "{:?}", dec.errors);
        assert!(matches!(&ops[0], RepOp::NewInstance { id: 7, props, .. }
            if props.contains(&("Source".into(), PropValue::CachedSharedString {hash: [0x55; 16], bytes: b"abc".to_vec()}))
                && props.contains(&("Name".into(), PropValue::String("Code".into())))));
    }

    #[test]
    fn stream_region_metadata_does_not_consume_following_marker() {
        let mut dec = decoder();
        let ops = dec.decode_frame(
            0x83,
            &[
                0x14, 0, 0, 0, 0, 63, 0, 1, 0, 0, 0, 0, 0, 0x23, 1, 0, 0, 0, 0,
            ],
        );
        assert!(dec.errors.is_empty(), "{:?}", dec.errors);
        assert!(matches!(
            ops[0],
            RepOp::Unhandled {
                kind: ItemKind::StreamData
            }
        ));
    }

    #[test]
    fn content_object_kind_is_consumed() {
        let s = Schema::default();
        // kind 2, reference scope 1, index 0x1234, then a following byte.
        let d = [2u8, 1, 0x34, 0x12, 0, 0, 0xaa];
        let mut c = Cur::new(&d);
        assert!(matches!(s.value(0x3d, &mut c), Ok(PropValue::Raw(_))));
        assert_eq!(c.o, 6);
        // nil object
        let d = [2u8, 0];
        let mut c = Cur::new(&d);
        assert!(s.value(0x3d, &mut c).is_ok());
        assert_eq!(c.o, 2);
    }

    #[test]
    fn bad_input_is_reported_not_panicking() {
        let mut dec = decoder();
        assert!(dec.decode_frame(0x83, &[0x7e, 0x00]).is_empty());
        assert_eq!(dec.errors.len(), 1);
        let mut dec = Decoder::new();
        dec.decode_frame(0x83, &[0x03, 0x01, 0, 0, 0, 0, 0, 0, 1, 1, 0]);
        assert!(dec.errors[0].contains("no schema"));
    }
}
