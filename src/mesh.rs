//! Mesh geometry loading: Roblox `.mesh` files (versions 1.00 through 7.00)
//! and Wavefront OBJ text, flattened into a de-indexed triangle list.
//!
//! Every read is bounds-checked; malformed or unsupported input yields
//! `Err(String)` rather than a panic. Positions, normals and UVs are
//! extracted; tangents, colors, skinning, bones, subsets and FACS data are
//! skipped. For multi-LOD meshes only the highest-detail LOD is used.
//!
//! UVs follow the image convention: (0,0) is the top-left of the texture and
//! v grows downwards. Binary Roblox meshes already store UVs this way; v1.x
//! text meshes and OBJ files store v upwards and are flipped (`1 - v`).

use glam::{Vec2, Vec3};

/// Triangle geometry ready for upload/rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshData {
    /// Triangle list, 3 entries per triangle (already de-indexed).
    pub positions: Vec<glam::Vec3>,
    pub normals: Vec<glam::Vec3>,
    /// Texture coordinates, one per position; (0,0) is the image's top-left.
    pub uvs: Vec<glam::Vec2>,
    /// Per-corner colours (sRGB 0..1, alpha 1), or empty: unions keep each
    /// child part's colour this way (SolidMesh's fourth array).
    pub colors: Vec<glam::Vec4>,
    /// CSGMDL projection plane per corner; empty for authored UV meshes.
    pub projection_tags: Vec<u8>,
    /// Axis-aligned bounds of the positions.
    pub min: glam::Vec3,
    pub max: glam::Vec3,
    /// Skinning (v4+ bones and per-vertex weights), one entry per corner.
    pub skin: Option<Skin>,
}

/// A mesh bone: its name, parent bone (None at the root) and bind-pose
/// frame in mesh space (rotation columns, position).
#[derive(Debug, Clone, PartialEq)]
pub struct SkinBone {
    pub name: String,
    pub parent: Option<usize>,
    pub rot: glam::Mat3,
    pub pos: Vec3,
}

/// A skinned mesh's bones and, per corner of the triangle list, up to four
/// (bone, weight) pairs (weights sum to 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    pub bones: Vec<SkinBone>,
    pub corner_bones: Vec<[u16; 4]>,
    pub corner_weights: Vec<[f32; 4]>,
    /// The mesh vertex each corner came from (corners of one vertex skin
    /// alike, so a deformer works per vertex and copies to corners).
    pub corner_vertex: Vec<u32>,
}

/// Parses Roblox mesh bytes (versions 1.00, 1.01, 2.00, 3.00, 3.01, 4.00, 4.01,
/// 5.00, and the chunked 6.00/7.00) or Wavefront OBJ text.
/// Returns Err(String) for unsupported versions/encodings (e.g. Draco-compressed
/// v7 geometry) or malformed data; never panics on bad input.
pub fn parse(bytes: &[u8]) -> Result<MeshData, String> {
    if let Some(at) = solid_mesh_start(bytes) {
        parse_solid_mesh(&bytes[at..])
    } else if is_csgmdl(bytes) {
        parse_csgmdl(bytes)
    } else if bytes.starts_with(b"version ") {
        parse_roblox(bytes)
    } else {
        parse_obj(bytes)
    }
}

// ---------------------------------------------------------------------------
// Roblox .mesh
// ---------------------------------------------------------------------------

/// Splits off the `version X.YY` line and dispatches on the version string.
fn parse_roblox(bytes: &[u8]) -> Result<MeshData, String> {
    let nl = bytes
        .iter()
        .position(|&b| b == b'\n')
        .ok_or("mesh: missing newline after version header")?;
    let line = std::str::from_utf8(&bytes[..nl]).map_err(|_| "mesh: version header is not text")?;
    let version = line["version ".len()..].trim();
    let body = &bytes[nl + 1..];
    match version {
        "1.00" => parse_v1(body, 0.5),
        "1.01" => parse_v1(body, 1.0),
        "2.00" => parse_v2(body),
        "3.00" | "3.01" => parse_v3(body),
        "4.00" | "4.01" => parse_v4(body, 24),
        "5.00" => parse_v4(body, 32),
        "6.00" | "7.00" => parse_chunked(body),
        other => Err(format!("mesh: unsupported version {other:?}")),
    }
}

/// Version 1.x: text. Line 1 (after the header) is the face count, the rest is
/// `[x,y,z]` groups: position, normal, uv for each of the 3 corners of each face.
fn parse_v1(body: &[u8], scale: f32) -> Result<MeshData, String> {
    let text = std::str::from_utf8(body).map_err(|_| "mesh v1: body is not UTF-8 text")?;
    let (count_line, rest) = text.split_once('\n').unwrap_or((text, ""));
    let face_count: usize = count_line
        .trim()
        .parse()
        .map_err(|_| format!("mesh v1: bad face count {:?}", count_line.trim()))?;
    let needed = face_count
        .checked_mul(9)
        .ok_or("mesh v1: face count overflow")?;

    // Collect bracketed groups; stop once we have enough so a bogus huge
    // face count can't make us allocate beyond what the text contains.
    let mut groups: Vec<Vec<f32>> = Vec::new();
    let mut s = rest;
    while groups.len() < needed {
        let Some(start) = s.find('[') else { break };
        let after = &s[start + 1..];
        let end = after.find(']').ok_or("mesh v1: unterminated '['")?;
        let vals = after[..end]
            .split(',')
            .map(|p| p.trim().parse::<f32>())
            .collect::<Result<Vec<f32>, _>>()
            .map_err(|_| format!("mesh v1: bad number in [{}]", &after[..end]))?;
        groups.push(vals);
        s = &after[end + 1..];
    }
    if groups.len() < needed {
        return Err(format!(
            "mesh v1: truncated, expected {needed} vector groups, found {}",
            groups.len()
        ));
    }

    let vec3 = |g: &Vec<f32>| -> Result<Vec3, String> {
        match g.as_slice() {
            [x, y, z, ..] => Ok(Vec3::new(*x, *y, *z)),
            _ => Err("mesh v1: vector with fewer than 3 components".into()),
        }
    };
    let mut out = TriBuilder::with_capacity(face_count);
    for face in groups.chunks_exact(9) {
        let mut p = [Vec3::ZERO; 3];
        let mut n = [None; 3];
        let mut uv = [Vec2::ZERO; 3];
        for k in 0..3 {
            p[k] = vec3(&face[k * 3])? * scale;
            n[k] = Some(vec3(&face[k * 3 + 1])?);
            // v1 stores v upwards; flip to the image convention.
            let t = vec3(&face[k * 3 + 2])?;
            uv[k] = Vec2::new(t.x, 1.0 - t.y);
        }
        out.push(p, n, uv);
    }
    Ok(out.finish())
}

/// Version 2.00: u16 header size, u8 vertex size, u8 face size, u32 vertex
/// count, u32 face count; then vertices and faces.
fn parse_v2(body: &[u8]) -> Result<MeshData, String> {
    let mut r = Reader::new(body);
    let header_size = r.u16()? as usize;
    let vertex_size = r.u8()? as usize;
    let face_size = r.u8()? as usize;
    let num_verts = r.u32()? as usize;
    let num_faces = r.u32()? as usize;
    check_header(header_size, 12, body.len())?;

    let (verts, off) = read_vertices(body, header_size, num_verts, vertex_size)?;
    let (faces, _) = read_faces(body, off, num_faces, face_size)?;
    build_indexed(&verts, &faces)
}

/// Version 3.00/3.01: u16 header size, u8 vertex size, u8 face size, u16 LOD
/// entry size, u16 LOD count, u32 vertex count, u32 face count; then vertices,
/// faces, LOD offsets.
fn parse_v3(body: &[u8]) -> Result<MeshData, String> {
    let mut r = Reader::new(body);
    let header_size = r.u16()? as usize;
    let vertex_size = r.u8()? as usize;
    let face_size = r.u8()? as usize;
    let lod_size = r.u16()? as usize;
    let num_lods = r.u16()? as usize;
    let num_verts = r.u32()? as usize;
    let num_faces = r.u32()? as usize;
    check_header(header_size, 16, body.len())?;

    let (verts, off) = read_vertices(body, header_size, num_verts, vertex_size)?;
    let (faces, off) = read_faces(body, off, num_faces, face_size)?;
    let lods = read_lods(body, off, num_lods, lod_size)?;
    build_indexed(&verts, lod0_faces(&faces, &lods))
}

/// Versions 4.00/4.01 (header 24 bytes) and 5.00 (header 32 bytes, adding
/// FACS format/size u32s). Header: u16 size, u16 LOD type, u32 vertex count,
/// u32 face count, u16 LOD count, u16 bone count, u32 bone-name bytes, u16
/// subset count, u8 HQ LOD count, u8 padding, [u32 FACS format, u32 FACS size].
/// Body: 40-byte vertices, skinning (8 bytes/vertex, only if bones > 0), faces,
/// LOD offsets, then bones/names/subsets/FACS (ignored).
fn parse_v4(body: &[u8], min_header: usize) -> Result<MeshData, String> {
    let mut r = Reader::new(body);
    let header_size = r.u16()? as usize;
    let _lod_type = r.u16()?;
    let num_verts = r.u32()? as usize;
    let num_faces = r.u32()? as usize;
    let num_lods = r.u16()? as usize;
    let num_bones = r.u16()? as usize;
    check_header(header_size, min_header, body.len())?;

    let (verts, mut off) = read_vertices(body, header_size, num_verts, 40)?;
    let envelopes = off;
    if num_bones > 0 {
        // Skinning envelopes: u8 bone indices[4], u8 weights[4] per vertex.
        off = span_end(body, off, num_verts, 8, "skinning data")?;
    }
    let (faces, off) = read_faces(body, off, num_faces, 12)?;
    let lods = read_lods(body, off, num_lods, 4)?;
    let lod0 = lod0_faces(&faces, &lods);
    let mut m = build_indexed(&verts, lod0)?;
    if num_bones > 0 && header_size >= 24 {
        // Header (v4+): …, u16 bones at 14, u32 bone-name bytes at 16, u16
        // subsets at 20. Bones, names and subsets follow the LOD offsets.
        let names_len = u32_at(body, 16) as usize;
        let subsets = u16::from_le_bytes([body[20], body[21]]) as usize;
        let bones_at = off + num_lods * 4;
        m.skin = read_skin(body, envelopes, num_verts, bones_at, num_bones, names_len, subsets, lod0);
    }
    Ok(m)
}

/// Skinning from the v4/v5 layout or the v6/v7 SKINNING chunk: bones (60
/// bytes: u32 name offset, u16 parent, u16 LOD parent, f32 culling, 3×3
/// rotation row-major, position), the name table, subsets (72 bytes: faces
/// begin/length, verts begin/length, u32 bone count, u16[26] bone table) and
/// per-vertex envelopes (u8 subset-local bone[4], u8 weight[4] summing 255),
/// laid out per corner of `faces`. None when anything is out of range.
#[allow(clippy::too_many_arguments)]
fn read_skin(
    data: &[u8],
    envelopes: usize,
    num_verts: usize,
    bones_at: usize,
    num_bones: usize,
    names_len: usize,
    num_subsets: usize,
    faces: &[[u32; 3]],
) -> Option<Skin> {
    let names_at = bones_at.checked_add(num_bones.checked_mul(60)?)?;
    let subsets_at = names_at.checked_add(names_len)?;
    let subsets_end = subsets_at.checked_add(num_subsets.checked_mul(72)?)?;
    if subsets_end > data.len() || envelopes.checked_add(num_verts.checked_mul(8)?)? > data.len() {
        return None;
    }
    let names = &data[names_at..subsets_at];
    let mut bones = Vec::with_capacity(num_bones);
    for b in 0..num_bones {
        let rec = &data[bones_at + b * 60..bones_at + b * 60 + 60];
        let name_off = u32_at(rec, 0) as usize;
        let name = names
            .get(name_off..)
            .map(|s| String::from_utf8_lossy(s.split(|c| *c == 0).next().unwrap_or(&[])).into_owned())
            .unwrap_or_default();
        let parent = u16::from_le_bytes([rec[4], rec[5]]);
        let r = |i: usize| f32_at(rec, 12 + i * 4);
        // Row-major R00 R01 R02 R10 … → columns.
        let rot = glam::Mat3::from_cols(
            Vec3::new(r(0), r(3), r(6)),
            Vec3::new(r(1), r(4), r(7)),
            Vec3::new(r(2), r(5), r(8)),
        );
        let pos = Vec3::new(f32_at(rec, 48), f32_at(rec, 52), f32_at(rec, 56));
        bones.push(SkinBone {
            name,
            parent: (parent != 0xFFFF && (parent as usize) < num_bones).then_some(parent as usize),
            rot,
            pos,
        });
    }
    // Vertex → its subset's bone table.
    let mut table: Vec<Option<usize>> = vec![None; num_verts];
    for s in 0..num_subsets {
        let rec = &data[subsets_at + s * 72..subsets_at + s * 72 + 72];
        let begin = u32_at(rec, 8) as usize;
        let len = u32_at(rec, 12) as usize;
        for v in begin..begin.saturating_add(len).min(num_verts) {
            table[v] = Some(s);
        }
    }
    let subset_bone = |s: usize, local: u8| -> u16 {
        let rec = &data[subsets_at + s * 72..subsets_at + s * 72 + 72];
        let count = u32_at(rec, 16) as usize;
        if (local as usize) < count.min(26) {
            u16::from_le_bytes([rec[20 + local as usize * 2], rec[21 + local as usize * 2]])
        } else {
            0xFFFF
        }
    };
    let mut corner_bones = Vec::with_capacity(faces.len() * 3);
    let mut corner_weights = Vec::with_capacity(faces.len() * 3);
    let mut corner_vertex = Vec::with_capacity(faces.len() * 3);
    for f in faces {
        for &v in f {
            corner_vertex.push(v);
            let v = v as usize;
            let e = &data[envelopes + v * 8..envelopes + v * 8 + 8];
            let mut ids = [0u16; 4];
            let mut ws = [0f32; 4];
            let mut sum = 0.0;
            for k in 0..4 {
                let w = e[4 + k] as f32 / 255.0;
                let id = match table.get(v).copied().flatten() {
                    Some(s) => subset_bone(s, e[k]),
                    None => e[k] as u16,
                };
                if w > 0.0 && (id as usize) < num_bones {
                    ids[k] = id;
                    ws[k] = w;
                    sum += w;
                }
            }
            if sum > 0.0 {
                for w in &mut ws {
                    *w /= sum;
                }
            }
            corner_bones.push(ids);
            corner_weights.push(ws);
        }
    }
    Some(Skin { bones, corner_bones, corner_weights, corner_vertex })
}

/// Versions 6.00/7.00: a sequence of chunks, each an 8-byte ASCII type, u32
/// chunk version, u32 data size, then the data. Geometry lives in `COREMESH`
/// (v1: u32 vertex count, 40-byte vertices, u32 face count, u32x3 faces; v2 is
/// Draco-compressed and unsupported). `LODS` v1 holds u16 LOD type, u8 HQ LOD
/// count, u32 LOD count, u32 face offsets. Other chunk types are skipped.
fn parse_chunked(body: &[u8]) -> Result<MeshData, String> {
    let mut r = Reader::new(body);
    let mut core: Option<(Vec<Vertex>, Vec<[u32; 3]>)> = None;
    let mut lods: Vec<u32> = Vec::new();
    let mut skinning: Option<&[u8]> = None;
    while r.pos < body.len() {
        let header = r.take(16).map_err(|_| "mesh: truncated chunk header")?;
        let kind = &header[..8];
        let version = u32_at(header, 8);
        let size = u32_at(header, 12) as usize;
        let name = String::from_utf8_lossy(kind);
        let name = name.trim_end_matches('\0');
        let data = r
            .take(size)
            .map_err(|_| format!("mesh: truncated {name} chunk ({size} bytes declared)"))?;
        match kind {
            b"COREMESH" => match version {
                1 => {
                    if core.is_some() {
                        return Err("mesh: duplicate COREMESH chunk".into());
                    }
                    let mut c = Reader::new(data);
                    let num_verts = c.u32()? as usize;
                    let (verts, off) = read_vertices(data, c.pos, num_verts, 40)?;
                    let mut c = Reader { data, pos: off };
                    let num_faces =
                        c.u32().map_err(|_| "mesh: truncated COREMESH face count")? as usize;
                    let (faces, _) = read_faces(data, c.pos, num_faces, 12)?;
                    core = Some((verts, faces));
                }
                2 => {
                    if core.is_some() {
                        return Err("mesh: duplicate COREMESH chunk".into());
                    }
                    core = Some(decode_draco_core(data)?);
                }
                v => return Err(format!("mesh: unsupported COREMESH chunk version {v}")),
            },
            b"LODS\0\0\0\0" if version == 1 => {
                let mut c = Reader::new(data);
                let _lod_type = c.u16()?;
                let _hq_lods = c.u8()?;
                let num_lods = c.u32()? as usize;
                lods = read_lods(data, c.pos, num_lods, 4)?;
            }
            b"SKINNING" if version == 1 => skinning = Some(data),
            // FACS, HSRAVIS, other LODS versions, unknown types.
            _ => {}
        }
    }
    let (verts, faces) = core.ok_or("mesh: no COREMESH chunk")?;
    let lod0 = lod0_faces(&faces, &lods);
    let mut m = build_indexed(&verts, lod0)?;
    // SKINNING v1: u32 envelope count, envelopes, u32 bone count, bones,
    // u32 name bytes, names, u32 subset count, subsets.
    if let Some(d) = skinning {
        let n_env = u32_at(d, 0) as usize;
        let bones_count_at = 4 + n_env.saturating_mul(8);
        if n_env == verts.len() && bones_count_at + 4 <= d.len() {
            let num_bones = u32_at(d, bones_count_at) as usize;
            let bones_at = bones_count_at + 4;
            let names_count_at = bones_at.saturating_add(num_bones.saturating_mul(60));
            if names_count_at + 4 <= d.len() {
                let names_len = u32_at(d, names_count_at) as usize;
                let subsets_count_at = (names_count_at + 4).saturating_add(names_len);
                if subsets_count_at + 4 <= d.len() && num_bones > 0 {
                    let subsets = u32_at(d, subsets_count_at) as usize;
                    // read_skin wants names then subsets contiguous after the
                    // bones; here each is preceded by its count, so copy.
                    let mut flat = Vec::new();
                    flat.extend_from_slice(&d[4..bones_count_at]);
                    let env = 0;
                    let b_at = flat.len();
                    flat.extend_from_slice(d.get(bones_at..names_count_at).unwrap_or(&[]));
                    flat.extend_from_slice(d.get(names_count_at + 4..subsets_count_at).unwrap_or(&[]));
                    flat.extend_from_slice(d.get(subsets_count_at + 4..(subsets_count_at + 4).saturating_add(subsets.saturating_mul(72)).min(d.len())).unwrap_or(&[]));
                    m.skin = read_skin(&flat, env, n_env, b_at, num_bones, names_len, subsets, lod0);
                }
            }
        }
    }
    Ok(m)
}

/// Validates a declared header size against the minimum the version needs.
fn check_header(declared: usize, min: usize, body_len: usize) -> Result<(), String> {
    if declared < min {
        return Err(format!(
            "mesh: header size {declared} smaller than required {min}"
        ));
    }
    if declared > body_len {
        return Err("mesh: truncated header".into());
    }
    Ok(())
}

/// Returns the end offset of `count * stride` bytes starting at `off`,
/// erroring (without allocating) if that runs past the data.
fn span_end(
    data: &[u8],
    off: usize,
    count: usize,
    stride: usize,
    what: &str,
) -> Result<usize, String> {
    count
        .checked_mul(stride)
        .and_then(|len| off.checked_add(len))
        .filter(|&end| end <= data.len())
        .ok_or_else(|| format!("mesh: truncated {what} ({count} x {stride} bytes at offset {off})"))
}

/// Position, normal and UV of one indexed vertex.
#[derive(Clone, Copy)]
struct Vertex {
    pos: Vec3,
    nrm: Vec3,
    uv: Vec2,
}

/// COREMESH v2: a u32 length, then a Draco bitstream holding the vertices
/// (Position, Normal, TexCoord attributes) and triangles, decoded with
/// draco-core (a pure-Rust decoder of Google's Draco format).
fn decode_draco_core(data: &[u8]) -> Result<(Vec<Vertex>, Vec<[u32; 3]>), String> {
    // Studio: a chunk under 4 bytes "is too small to hold the
    // compressed mesh data size"; a size past the chunk "exceeds" it.
    let head = crate::mech::COREMESH_V2_SIZE_BYTES;
    let mut c = Reader::new(data);
    let len = c.u32().map_err(|_| {
        "mesh: truncated Draco length (COREMESH chunk too small to hold the compressed mesh data size)"
    })? as usize;
    let bytes = data.get(head..head + len).ok_or(
        "mesh: truncated Draco data (Compressed mesh data size exceeds COREMESH chunk size)",
    )?;
    let mut buf = draco_core::decoder_buffer::DecoderBuffer::new(bytes);
    let mut mesh = draco_core::mesh::Mesh::new();
    draco_core::mesh_decoder::MeshDecoder::new()
        .decode(&mut buf, &mut mesh)
        .map_err(|e| format!("mesh: Draco decode failed: {e:?}"))?;
    // Studio's rule (, attributes looked up by unique id through
    //): id 0 position 3 × float32, 1 normal 3 × float32, 2 uv
    // 2 × float32, 3 tangent 4 × uint8, 4 colour 4 × uint8, all required.
    // Values are copied as they are (no flip) into its 40-byte vertex.
    use draco_core::draco_types::DataType as T;
    let n = mesh.num_points();
    let shapes = [
        (3u8, T::Float32),
        (3, T::Float32),
        (2, T::Float32),
        (4, T::Uint8),
        (4, T::Uint8),
    ];
    let mut attrs = Vec::with_capacity(crate::mech::DRACO_ATTRIBUTES);
    for (id, (k, t)) in shapes.iter().enumerate() {
        match mesh.attribute_by_unique_id(id as u32) {
            Some(a) if a.num_components() == *k && a.data_type() == *t => attrs.push(a),
            _ => {
                return Err(
                    "mesh: Decoded Draco mesh is missing or has malformed attributes".into(),
                )
            }
        }
    }
    let pos = attrs[0].read_f32s(n, 3);
    let nrm = attrs[1].read_f32s(n, 3);
    let uv = attrs[2].read_f32s(n, 2);
    let verts = (0..n)
        .map(|i| Vertex {
            pos: Vec3::new(pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2]),
            nrm: Vec3::new(nrm[i * 3], nrm[i * 3 + 1], nrm[i * 3 + 2]),
            uv: Vec2::new(uv[i * 2], uv[i * 2 + 1]),
        })
        .collect();
    let mut faces = Vec::with_capacity(mesh.num_faces());
    for f in 0..mesh.num_faces() {
        let face = mesh
            .face(draco_core::geometry_indices::FaceIndex(f as u32))
            .map(|p| p.0);
        if face.iter().any(|&i| i as usize >= n) {
            return Err("mesh: Decoded Draco attribute value out of range".into());
        }
        faces.push(face);
    }
    Ok((verts, faces))
}

/// Reads position, normal and (when the record is >= 32 bytes) UV from each
/// vertex record of `stride` bytes. UVs are kept as stored (v points down).
fn read_vertices(
    data: &[u8],
    off: usize,
    count: usize,
    stride: usize,
) -> Result<(Vec<Vertex>, usize), String> {
    if stride < 24 {
        return Err(format!("mesh: vertex size {stride} too small (need >= 24)"));
    }
    let end = span_end(data, off, count, stride, "vertices")?;
    let verts = data[off..end]
        .chunks_exact(stride)
        .map(|rec| Vertex {
            pos: Vec3::new(f32_at(rec, 0), f32_at(rec, 4), f32_at(rec, 8)),
            nrm: Vec3::new(f32_at(rec, 12), f32_at(rec, 16), f32_at(rec, 20)),
            uv: if stride >= 32 {
                Vec2::new(f32_at(rec, 24), f32_at(rec, 28))
            } else {
                Vec2::ZERO
            },
        })
        .collect();
    Ok((verts, end))
}

/// Reads triangles (3 x u32 indices) from face records of `stride` bytes.
fn read_faces(
    data: &[u8],
    off: usize,
    count: usize,
    stride: usize,
) -> Result<(Vec<[u32; 3]>, usize), String> {
    if stride < 12 {
        return Err(format!("mesh: face size {stride} too small (need >= 12)"));
    }
    let end = span_end(data, off, count, stride, "faces")?;
    let faces = data[off..end]
        .chunks_exact(stride)
        .map(|rec| [u32_at(rec, 0), u32_at(rec, 4), u32_at(rec, 8)])
        .collect();
    Ok((faces, end))
}

/// Reads LOD face offsets (u32, first 4 bytes of each `stride`-byte entry).
fn read_lods(data: &[u8], off: usize, count: usize, stride: usize) -> Result<Vec<u32>, String> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if stride < 4 {
        return Err(format!(
            "mesh: LOD entry size {stride} too small (need >= 4)"
        ));
    }
    let end = span_end(data, off, count, stride, "LOD offsets")?;
    Ok(data[off..end]
        .chunks_exact(stride)
        .map(|rec| u32_at(rec, 0))
        .collect())
}

/// Picks the highest-detail LOD's faces: `[lods[0], lods[1])` when there are
/// at least two LOD entries and that range is sane, otherwise all faces.
fn lod0_faces<'a>(faces: &'a [[u32; 3]], lods: &[u32]) -> &'a [[u32; 3]] {
    if let [a, b, ..] = *lods {
        let start = (a as usize).min(faces.len());
        let end = (b as usize).min(faces.len());
        if start < end {
            return &faces[start..end];
        }
    }
    faces
}

/// De-indexes faces against vertex arrays; out-of-range indices are an error.
fn build_indexed(verts: &[Vertex], faces: &[[u32; 3]]) -> Result<MeshData, String> {
    let mut out = TriBuilder::with_capacity(faces.len());
    for f in faces {
        let i = f.map(|i| i as usize);
        if let Some(bad) = i.iter().find(|&&i| i >= verts.len()) {
            return Err(format!(
                "mesh: face index {bad} out of range ({} vertices)",
                verts.len()
            ));
        }
        let v = i.map(|i| verts[i]);
        out.push(v.map(|v| v.pos), v.map(|v| Some(v.nrm)), v.map(|v| v.uv));
    }
    Ok(out.finish())
}

/// Bounds-checked little-endian cursor over the binary header.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or("mesh: truncated header")?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().unwrap()))
    }
}

/// Little-endian u32 at `off`; callers guarantee `off + 4 <= rec.len()`.
fn u32_at(rec: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([rec[off], rec[off + 1], rec[off + 2], rec[off + 3]])
}

/// Little-endian f32 at `off`; callers guarantee `off + 4 <= rec.len()`.
fn f32_at(rec: &[u8], off: usize) -> f32 {
    f32::from_bits(u32_at(rec, off))
}

// ---------------------------------------------------------------------------
// SolidMesh (unions: PartOperation.SolidMeshHolder)
// ---------------------------------------------------------------------------

/// Where a `SolidMesh` blob starts: at 0, or after the SolidMeshHolder's
/// header (14 bytes `03 01 <u32 size> … <u32 size>` in the places seen).
fn solid_mesh_start(bytes: &[u8]) -> Option<usize> {
    let head = &bytes[..bytes.len().min(64)];
    head.windows(9).position(|w| w == b"SolidMesh")
}

/// Studio's SolidMesh reader (Studio, Studio measurements
/// "Unions"): `SolidMesh` + u32 version 0, then
/// positions (u64 n, n × vec3), normals (vec3[]), UVs (u64 n, n × 2 f32),
/// per-corner colours (vec3[], sRGB 0..1: the child parts' colours), then
/// four delta-coded u32 lists: each triangle corner's position, normal, UV and
/// colour index.
fn parse_solid_mesh(b: &[u8]) -> Result<MeshData, String> {
    let mut r = Reader::new(b);
    r.take(9)?;
    let version = r.u32()?;
    if version != 0 {
        return Err(format!("SolidMesh: version {version} isn't read yet"));
    }
    let vec3s = |r: &mut Reader| -> Result<Vec<Vec3>, String> {
        let n = r.u64()? as usize;
        let raw = r.take(n.checked_mul(12).ok_or("SolidMesh: bad count")?)?;
        Ok(raw
            .chunks_exact(12)
            .map(|c| Vec3::new(f32_at(c, 0), f32_at(c, 4), f32_at(c, 8)))
            .collect())
    };
    let positions = vec3s(&mut r)?;
    let normals = vec3s(&mut r)?;
    let n_uv = r.u64()? as usize;
    let uv_raw = r.take(n_uv.checked_mul(8).ok_or("SolidMesh: bad count")?)?;
    let uvs: Vec<Vec2> = uv_raw
        .chunks_exact(8)
        .map(|c| Vec2::new(f32_at(c, 0), f32_at(c, 4)))
        .collect();
    let colors = vec3s(&mut r)?;
    let corners = delta_u32s(&mut r)?;
    let normal_ix = delta_u32s(&mut r)?;
    let uv_ix = delta_u32s(&mut r)?;
    let color_ix = delta_u32s(&mut r)?;
    if corners.len() % 3 != 0 {
        return Err("SolidMesh: corner count isn't a multiple of 3".into());
    }
    let mut out = TriBuilder::with_capacity(corners.len() / 3);
    for (t, tri) in corners.chunks_exact(3).enumerate() {
        let mut p = [Vec3::ZERO; 3];
        let mut nrm = [None; 3];
        let mut uv = [Vec2::ZERO; 3];
        for k in 0..3 {
            let c = t * 3 + k;
            p[k] = *positions
                .get(tri[k] as usize)
                .ok_or("SolidMesh: position index out of range")?;
            nrm[k] = normal_ix.get(c).and_then(|&i| normals.get(i as usize)).copied();
            uv[k] = uv_ix.get(c).and_then(|&i| uvs.get(i as usize)).copied().unwrap_or(Vec2::ZERO);
        }
        out.push(p, nrm, uv);
    }
    let mut m = out.finish();
    // Modern SolidMesh generates a plane for each triangle,
    // from its geometric normal, not the smooth corner normals.
    for tri in m.positions.chunks_exact(3) {
        let plane = solid_mesh_projection_plane(tri[0], tri[1], tri[2]);
        // 9..14 distinguish generated modern planes from CSGMDL's 1..6.
        m.projection_tags.extend([plane as u8 + 9; 3]);
    }
    if !colors.is_empty() && color_ix.len() == corners.len() {
        m.colors = color_ix
            .iter()
            .map(|&i| colors.get(i as usize).map(|c| c.extend(1.0)).unwrap_or(glam::Vec4::ZERO))
            .collect();
    }
    Ok(m)
}

/// Studio normalizes the triangle normal with FLT_MIN;
/// then adds X=.01/Y=.02 before choosing the first maximum.
/// The bias applies to unit normals, so it must not depend on triangle area.
fn solid_mesh_projection_plane(a: Vec3, b: Vec3, c: Vec3) -> usize {
    let cross = (b - a).cross(c - a);
    let n = cross / (cross.length() + f32::MIN_POSITIVE);
    let biased = n.abs() + Vec3::new(0.01, 0.02, 0.0);
    let axis = if biased.x >= biased.y && biased.x >= biased.z {
        0
    } else if biased.y >= biased.z {
        1
    } else {
        2
    };
    axis + if n[axis] < 0.0 { 3 } else { 0 }
}

/// The key that hides a CSGMDL header: MSVC `rand` seeded
/// with 1337, each value mod 127, 31 bytes, repeated.
fn csgmdl_key() -> [u8; 31] {
    let mut x: u32 = 1337;
    let mut k = [0u8; 31];
    for b in k.iter_mut() {
        x = x.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        *b = (((x >> 16) & 0x7fff) % 127) as u8;
    }
    k
}

/// A legacy union mesh (PartOperation.MeshData2 / MeshData): its first 10
/// bytes, un-hidden, read `CSGMDL` + u32 version.
fn csgmdl_header(bytes: &[u8]) -> Option<u32> {
    let k = csgmdl_key();
    let h: Vec<u8> = bytes.get(..10)?.iter().enumerate().map(|(i, b)| b ^ k[i % 31]).collect();
    (&h[..6] == b"CSGMDL").then(|| u32::from_le_bytes([h[6], h[7], h[8], h[9]]))
}

fn is_csgmdl(bytes: &[u8]) -> bool {
    csgmdl_header(bytes).is_some()
}

/// CSGMDL version 5 (Studio 0.746, Studio measurements "Unions"),
/// after the 10-byte header, all per vertex and u16-counted (< 60001):
/// positions (vec3), normals (u32 byte count, then 3 × u16 each: (v − 0x7fff)
/// / 32767), colours (RGBA8), projection plane IDs (u8), UVs (2 × f32),
/// tangents (packed like normals, unused); then an i32 index count, a u32
/// byte length and the indices delta-coded as SolidMesh's; then a u8 LOD
/// count and that many u32 offsets: LOD 0 is indices[lod0..lod1].
/// Versions 2–4 aren't read yet.
fn parse_csgmdl(bytes: &[u8]) -> Result<MeshData, String> {
    let version = csgmdl_header(bytes).ok_or("CSGMDL: bad header")?;
    if version != 5 {
        return Err(format!("CSGMDL version {version} isn't read yet"));
    }
    let mut r = Reader::new(&bytes[10..]);
    let count = |r: &mut Reader| -> Result<usize, String> {
        let n = r.u16()? as usize;
        if n > 60000 {
            return Err("CSGMDL: count over 60000".into());
        }
        Ok(n)
    };
    let packed = |r: &mut Reader, n: usize| -> Result<Vec<Vec3>, String> {
        let nb = r.u32()? as usize;
        if nb != n * 6 {
            return Err("CSGMDL: packed vector size".into());
        }
        let raw = r.take(nb)?;
        Ok(raw
            .chunks_exact(6)
            .map(|c| {
                let g = |i: usize| (u16::from_le_bytes([c[i], c[i + 1]]) as i32 - 0x7fff) as f32 / 32767.0;
                Vec3::new(g(0), g(2), g(4))
            })
            .collect())
    };
    let n = count(&mut r)?;
    let positions: Vec<Vec3> = r
        .take(n * 12)?
        .chunks_exact(12)
        .map(|c| Vec3::new(f32_at(c, 0), f32_at(c, 4), f32_at(c, 8)))
        .collect();
    let nn = count(&mut r)?;
    let normals = if nn > 0 { packed(&mut r, nn)? } else { Vec::new() };
    let nc = count(&mut r)?;
    let colors: Vec<glam::Vec4> = r
        .take(nc * 4)?
        .chunks_exact(4)
        .map(|c| glam::Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, 255.0) / 255.0)
        .collect();
    let nd = count(&mut r)?;
    let tags = r.take(nd)?.to_vec();
    let nu = count(&mut r)?;
    let uvs: Vec<Vec2> = r
        .take(nu * 8)?
        .chunks_exact(8)
        .map(|c| Vec2::new(f32_at(c, 0), f32_at(c, 4)))
        .collect();
    let nt = count(&mut r)?;
    if nt > 0 {
        packed(&mut r, nt)?;
    }
    let ni = r.u32()? as usize;
    if ni == 0 || ni > 120000 {
        return Err("CSGMDL: index count".into());
    }
    let nb = r.u32()? as usize;
    let end = r.pos + nb;
    let mut idx = Vec::with_capacity(ni);
    let mut prev: u32 = 0;
    for i in 0..ni {
        let d = delta_value(&mut r)?;
        prev = if i == 0 { d as u32 } else { prev.wrapping_add(d as u32) };
        idx.push(prev);
    }
    if r.pos != end {
        return Err("CSGMDL: index bytes".into());
    }
    let nl = r.u8()? as usize;
    let mut lods = Vec::with_capacity(nl);
    for _ in 0..nl {
        lods.push(r.u32()? as usize);
    }
    let (a, b) = match lods.as_slice() {
        [a, b, ..] if a <= b && *b <= idx.len() => (*a, *b),
        _ => (0, idx.len()),
    };
    let tris = &idx[a..b - (b - a) % 3];
    let mut out = TriBuilder::with_capacity(tris.len() / 3);
    let mut corner_colors = Vec::with_capacity(tris.len());
    let mut corner_tags = Vec::with_capacity(tris.len());
    for t in tris.chunks_exact(3) {
        let mut p = [Vec3::ZERO; 3];
        let mut nrm = [None; 3];
        let mut uv = [Vec2::ZERO; 3];
        for k in 0..3 {
            let i = t[k] as usize;
            p[k] = *positions.get(i).ok_or("CSGMDL: index out of range")?;
            nrm[k] = normals.get(i).copied();
            uv[k] = uvs.get(i).copied().unwrap_or(Vec2::ZERO);
            corner_tags.push(tags.get(i).copied().unwrap_or(0));
            corner_colors.push(colors.get(i).copied().unwrap_or(glam::Vec4::ZERO));
        }
        out.push(p, nrm, uv);
    }
    let mut m = out.finish();
    m.projection_tags = corner_tags;
    if !colors.is_empty() {
        m.colors = corner_colors;
    }
    Ok(m)
}

/// One value of the 1-or-3-byte signed delta coding.
fn delta_value(r: &mut Reader) -> Result<i32, String> {
    let c = r.u8()?;
    let hi = ((c << 1) as i8 as i32) >> 1;
    Ok(if c & 0x80 == 0 {
        hi
    } else {
        let b1 = r.u8()? as i32;
        let b2 = r.u8()? as i32;
        (((hi << 8) | b1) << 8) | b2
    })
}

/// A delta-coded u32 list: u64 count, then per value one byte
/// (high bit clear: a 7-bit signed delta) or three (high bit set: a 23-bit
/// signed delta, big-endian); the first value is taken from 0.
fn delta_u32s(r: &mut Reader) -> Result<Vec<u32>, String> {
    let n = r.u64()? as usize;
    if n > r.data.len() - r.pos {
        return Err("SolidMesh: index list longer than the data".into());
    }
    let mut out = Vec::with_capacity(n);
    let mut prev: u32 = 0;
    for i in 0..n {
        let c = r.u8()?;
        let hi = ((c << 1) as i8 as i32) >> 1; // c's low 7 bits, sign-extended
        let d = if c & 0x80 == 0 {
            hi
        } else {
            let b1 = r.u8()? as i32;
            let b2 = r.u8()? as i32;
            (((hi << 8) | b1) << 8) | b2
        };
        prev = if i == 0 { d as u32 } else { prev.wrapping_add(d as u32) };
        out.push(prev);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Wavefront OBJ
// ---------------------------------------------------------------------------

/// Parses `v`, `vt`, `vn` and `f` records; other statements are ignored.
/// Polygons are fan-triangulated; corners without a normal get the flat face
/// normal, corners without a texture coordinate get (0,0). OBJ v points up, so
/// it is flipped to `1 - v`.
fn parse_obj(bytes: &[u8]) -> Result<MeshData, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut verts: Vec<Vec3> = Vec::new();
    let mut vnorms: Vec<Vec3> = Vec::new();
    let mut vtex: Vec<Vec2> = Vec::new();
    let mut out = TriBuilder::with_capacity(0);

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let mut it = line.split_whitespace();
        let err = |msg: &str| format!("obj line {}: {msg}", lineno + 1);
        match it.next() {
            Some("v") => verts.push(obj_vec3(&mut it).ok_or_else(|| err("bad vertex"))?),
            Some("vn") => vnorms.push(obj_vec3(&mut it).ok_or_else(|| err("bad normal"))?),
            Some("vt") => {
                let mut f = || it.next().map(str::parse::<f32>);
                let u = f()
                    .and_then(Result::ok)
                    .ok_or_else(|| err("bad texture coordinate"))?;
                // v is optional in OBJ (1D textures); default 0.
                let v = f()
                    .unwrap_or(Ok(0.0))
                    .map_err(|_| err("bad texture coordinate"))?;
                vtex.push(Vec2::new(u, 1.0 - v));
            }
            Some("f") => {
                let mut corners: Vec<(Vec3, Option<Vec3>, Vec2)> = Vec::new();
                for tok in it {
                    let mut parts = tok.split('/');
                    let vi = parts.next().unwrap_or("");
                    let vt = parts.next().filter(|s| !s.is_empty());
                    let vn = parts.next().filter(|s| !s.is_empty());
                    let p = obj_index(vi, verts.len())
                        .map(|i| verts[i])
                        .ok_or_else(|| err(&format!("bad vertex index {tok:?}")))?;
                    let n = match vn {
                        Some(s) => Some(
                            obj_index(s, vnorms.len())
                                .map(|i| vnorms[i])
                                .ok_or_else(|| err(&format!("bad normal index {tok:?}")))?,
                        ),
                        None => None,
                    };
                    let t = match vt {
                        Some(s) => obj_index(s, vtex.len())
                            .map(|i| vtex[i])
                            .ok_or_else(|| err(&format!("bad texture index {tok:?}")))?,
                        None => Vec2::ZERO,
                    };
                    corners.push((p, n, t));
                }
                if corners.len() < 3 {
                    return Err(err("face with fewer than 3 vertices"));
                }
                for i in 1..corners.len() - 1 {
                    let c = [corners[0], corners[i], corners[i + 1]];
                    out.push(c.map(|c| c.0), c.map(|c| c.1), c.map(|c| c.2));
                }
            }
            _ => {}
        }
    }
    if out.positions.is_empty() {
        return Err("not a Roblox mesh, and no OBJ faces found".into());
    }
    Ok(out.finish())
}

/// Reads the first three floats of a `v`/`vn` record (extra values ignored).
fn obj_vec3<'a>(it: &mut impl Iterator<Item = &'a str>) -> Option<Vec3> {
    let mut f = || it.next()?.parse::<f32>().ok();
    Some(Vec3::new(f()?, f()?, f()?))
}

/// Resolves a 1-based (or negative, relative-to-end) OBJ index to 0-based.
fn obj_index(s: &str, len: usize) -> Option<usize> {
    let i: i64 = s.parse().ok()?;
    let idx = if i > 0 {
        i - 1
    } else if i < 0 {
        len as i64 + i
    } else {
        return None;
    };
    (idx >= 0 && (idx as usize) < len).then_some(idx as usize)
}

// ---------------------------------------------------------------------------
// Shared output
// ---------------------------------------------------------------------------

/// Accumulates de-indexed triangles, repairing missing/degenerate normals.
struct TriBuilder {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<Vec2>,
}

impl TriBuilder {
    fn with_capacity(tris: usize) -> Self {
        // Cap the up-front reservation; the real size is bounded by input anyway.
        let n = tris.saturating_mul(3).min(1 << 20);
        Self {
            positions: Vec::with_capacity(n),
            normals: Vec::with_capacity(n),
            uvs: Vec::with_capacity(n),
        }
    }

    /// Adds one triangle. A corner normal that is absent, zero or non-finite is
    /// replaced by the flat face normal (+Y if the triangle is degenerate).
    fn push(&mut self, p: [Vec3; 3], n: [Option<Vec3>; 3], uv: [Vec2; 3]) {
        let face = (p[1] - p[0])
            .cross(p[2] - p[0])
            .try_normalize()
            .unwrap_or(Vec3::Y);
        for k in 0..3 {
            self.positions.push(p[k]);
            self.normals
                .push(n[k].and_then(Vec3::try_normalize).unwrap_or(face));
            self.uvs.push(uv[k]);
        }
    }

    /// Computes bounds (ignoring non-finite components) and returns the mesh.
    fn finish(self) -> MeshData {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for p in &self.positions {
            if p.is_finite() {
                min = min.min(*p);
                max = max.max(*p);
            }
        }
        if !min.is_finite() || !max.is_finite() {
            min = Vec3::ZERO;
            max = Vec3::ZERO;
        }
        MeshData {
            positions: self.positions,
            normals: self.normals,
            uvs: self.uvs,
            projection_tags: Vec::new(),
            colors: Vec::new(),
            min,
            max,
            skin: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Little-endian byte builder for synthetic binary meshes.
    #[derive(Default)]
    struct Buf(Vec<u8>);

    impl Buf {
        fn new(header: &str) -> Self {
            Buf(header.as_bytes().to_vec())
        }
        fn u8(&mut self, v: u8) -> &mut Self {
            self.0.push(v);
            self
        }
        fn u16(&mut self, v: u16) -> &mut Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn u32(&mut self, v: u32) -> &mut Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn f32(&mut self, v: f32) -> &mut Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn zeros(&mut self, n: usize) -> &mut Self {
            self.0.resize(self.0.len() + n, 0);
            self
        }
        /// Vertex record: pos, normal, uv(2), tangent(4 x i8), then padding
        /// up to `size` (e.g. rgba for 40-byte vertices).
        fn vertex(&mut self, size: usize, p: [f32; 3], n: [f32; 3]) -> &mut Self {
            let start = self.0.len();
            for v in p.into_iter().chain(n) {
                self.f32(v);
            }
            self.f32(0.25).f32(0.75); // uv
            self.0.extend_from_slice(&[0, 0, 127, 127]); // tangent
            let written = self.0.len() - start;
            self.zeros(size - written)
        }
        fn face(&mut self, a: u32, b: u32, c: u32) -> &mut Self {
            self.u32(a).u32(b).u32(c)
        }
    }

    const UP: [f32; 3] = [0.0, 1.0, 0.0];

    /// Four vertices of a unit quad in the XZ plane plus one far-away vertex.
    fn quad_verts(b: &mut Buf, size: usize) {
        b.vertex(size, [0.0, 0.0, 0.0], UP)
            .vertex(size, [0.0, 0.0, 1.0], UP)
            .vertex(size, [1.0, 0.0, 1.0], [0.0, 0.0, 0.0]) // zero normal -> repaired
            .vertex(size, [1.0, 0.0, 0.0], UP)
            .vertex(size, [5.0, 5.0, 5.0], UP);
    }

    fn v2_bytes() -> Vec<u8> {
        let mut b = Buf::new("version 2.00\n");
        b.u16(12).u8(40).u8(12).u32(5).u32(2);
        quad_verts(&mut b, 40);
        b.face(0, 1, 2).face(0, 2, 3);
        b.0
    }

    fn v3_bytes() -> Vec<u8> {
        let mut b = Buf::new("version 3.00\n");
        b.u16(16).u8(40).u8(12).u16(4).u16(2).u32(5).u32(2);
        quad_verts(&mut b, 40);
        b.face(0, 1, 2).face(2, 3, 4); // second face is LOD1, must be dropped
        b.u32(0).u32(1);
        b.0
    }

    fn v4_bytes() -> Vec<u8> {
        let mut b = Buf::new("version 4.00\n");
        let names = b"Root\0";
        b.u16(24)
            .u16(0)
            .u32(5)
            .u32(3)
            .u16(3)
            .u16(1)
            .u32(names.len() as u32)
            .u16(0)
            .u8(0)
            .u8(0);
        quad_verts(&mut b, 40);
        for _ in 0..5 {
            b.0.extend_from_slice(&[0, 0, 0, 0, 255, 0, 0, 0]); // skinning
        }
        b.face(0, 1, 2).face(0, 2, 3).face(2, 3, 4);
        b.u32(0).u32(2).u32(3); // LOD0 = faces 0..2
        b.zeros(60); // one bone record (ignored)
        b.0.extend_from_slice(names);
        b.0
    }

    fn v5_bytes() -> Vec<u8> {
        let mut b = Buf::new("version 5.00\r\n");
        b.u16(32)
            .u16(0)
            .u32(5)
            .u32(2)
            .u16(0)
            .u16(0)
            .u32(0)
            .u16(0)
            .u8(0)
            .u8(0);
        b.u32(1).u32(0); // FACS format, size
        quad_verts(&mut b, 40);
        b.face(0, 1, 2).face(0, 2, 3);
        b.0
    }

    fn assert_quad(m: &MeshData) {
        assert_eq!(m.positions.len(), 6);
        assert_eq!(m.normals.len(), 6);
        assert_eq!(m.min, Vec3::ZERO);
        assert_eq!(m.max, Vec3::new(1.0, 0.0, 1.0));
        // Winding (0,1,2) in XZ gives a +Y face normal, so the zero normal
        // of vertex 2 becomes +Y as well.
        for n in &m.normals {
            assert!((*n - Vec3::Y).length() < 1e-6, "normal {n:?}");
        }
    }

    #[test]
    fn v1_text() {
        let src = "version 1.00\r\n1\n\
            [2,0,0][0,0,1][0,0,0]\
            [0,2,0][0,0,0][1,0,0]\
            [0,0,2][nan,0,0][0,1,0]";
        let m = parse(src.as_bytes()).unwrap();
        assert_eq!(m.positions, vec![Vec3::X, Vec3::Y, Vec3::Z]); // scaled by 0.5
        assert_eq!(m.normals[0], Vec3::Z);
        let face = Vec3::ONE.normalize();
        assert!((m.normals[1] - face).length() < 1e-6); // zero -> face normal
        assert!((m.normals[2] - face).length() < 1e-6); // NaN -> face normal
        assert_eq!(m.min, Vec3::ZERO);
        assert_eq!(m.max, Vec3::ONE);

        let v101 = src.replace("1.00", "1.01");
        let m = parse(v101.as_bytes()).unwrap();
        assert_eq!(m.positions[0], Vec3::new(2.0, 0.0, 0.0)); // no scaling
    }

    #[test]
    fn v1_truncated() {
        let src =
            "version 1.00\n2\n[1,0,0][0,0,1][0,0,0][0,1,0][0,0,1][0,0,0][0,0,1][0,0,1][0,0,0]";
        assert!(parse(src.as_bytes()).is_err());
        assert!(parse(b"version 1.01\n999999999999999999\n[1,2,3]").is_err());
        assert!(parse(b"version 1.01\n1\n[1,2,3").is_err());
    }

    #[test]
    fn v2_binary() {
        assert_quad(&parse(&v2_bytes()).unwrap());
    }

    #[test]
    fn v2_36_byte_vertices_and_long_header() {
        // Header declares 16 bytes (4 unknown trailing bytes) and 36-byte vertices.
        let mut b = Buf::new("version 2.00\n");
        b.u16(16).u8(36).u8(12).u32(5).u32(2).u32(0xDEADBEEF);
        quad_verts(&mut b, 36);
        b.face(0, 1, 2).face(0, 2, 3);
        assert_quad(&parse(&b.0).unwrap());
    }

    #[test]
    fn v3_uses_lod0_only() {
        let m = parse(&v3_bytes()).unwrap();
        assert_eq!(m.positions.len(), 3);
        assert_eq!(m.positions[2], Vec3::new(1.0, 0.0, 1.0));
        assert_eq!(m.max, Vec3::new(1.0, 0.0, 1.0)); // vertex 4 (5,5,5) unused
    }

    #[test]
    fn v3_single_lod_uses_all_faces() {
        let mut b = Buf::new("version 3.01\n");
        b.u16(16).u8(40).u8(12).u16(4).u16(1).u32(5).u32(2);
        quad_verts(&mut b, 40);
        b.face(0, 1, 2).face(2, 3, 4);
        b.u32(0);
        let m = parse(&b.0).unwrap();
        assert_eq!(m.positions.len(), 6);
        assert_eq!(m.max, Vec3::splat(5.0));
    }

    #[test]
    fn v4_with_skinning() {
        assert_quad(&parse(&v4_bytes()).unwrap());
    }

    #[test]
    fn v5_with_facs_header() {
        assert_quad(&parse(&v5_bytes()).unwrap());
    }

    #[test]
    fn obj_quad() {
        let src = "# quad\nv 0 0 0\nv 0 0 1\nv 1 0 1\nv 1 0 0\nf 1 2 3 4\n";
        assert_quad(&parse(src.as_bytes()).unwrap());
    }

    #[test]
    fn obj_index_forms() {
        let src = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvn 0 0 1\n\
                   f 1/1/1 2//1 -1\n\
                   f -3/1 -2 3/1/-1\n";
        let m = parse(src.as_bytes()).unwrap();
        assert_eq!(m.positions.len(), 6);
        assert_eq!(m.positions[1], Vec3::X);
        assert_eq!(m.positions[2], Vec3::Y);
        for n in &m.normals {
            assert!((*n - Vec3::Z).length() < 1e-6);
        }
        assert!(parse(b"v 0 0 0\nf 1 2 3\n").is_err()); // index out of range
        assert!(parse(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n").is_err()); // zero index
    }

    #[test]
    fn truncated_files_error() {
        for full in [v2_bytes(), v3_bytes()] {
            assert!(parse(&full).is_ok());
            for len in 0..full.len() {
                assert!(parse(&full[..len]).is_err(), "prefix {len} parsed");
            }
        }
        // Formats with unread trailing data: must at least never panic.
        for full in [v4_bytes(), v5_bytes()] {
            for len in 0..full.len() {
                let _ = parse(&full[..len]);
            }
        }
    }

    #[test]
    fn bad_indices_and_huge_counts() {
        let mut b = Buf::new("version 2.00\n");
        b.u16(12).u8(40).u8(12).u32(3).u32(1);
        quad_verts(&mut b, 40); // 5 written, header says 3: extra bytes are fine
        b.0.truncate(b.0.len() - 80);
        b.face(0, 1, 7);
        assert!(parse(&b.0).is_err());

        let mut b = Buf::new("version 4.01\n");
        b.u16(24)
            .u16(0)
            .u32(u32::MAX)
            .u32(u32::MAX)
            .u16(u16::MAX)
            .u16(u16::MAX)
            .u32(0)
            .u16(0)
            .u8(0)
            .u8(0);
        assert!(parse(&b.0).is_err());

        let mut b = Buf::new("version 2.00\n");
        b.u16(4).u8(40).u8(12).u32(0).u32(0); // header size below minimum
        assert!(parse(&b.0).is_err());
    }

    #[test]
    fn unsupported_versions() {
        let mut b = Buf::new("version 8.00\n");
        b.u16(0).zeros(64);
        let e = parse(&b.0).unwrap_err();
        assert!(e.contains("unsupported"), "{e}");
        // Chunked header bytes that don't form a whole chunk.
        let mut b = Buf::new("version 6.00\n");
        b.u16(0).zeros(64);
        assert!(parse(&b.0).is_err());
        assert!(parse(b"version 7.00\nCOREMESH\x00\x00").is_err());
        assert!(parse(b"version 6.00").is_err());
        assert!(parse(b"version 6.00\n")
            .unwrap_err()
            .contains("no COREMESH"));
    }

    #[test]
    fn v2_uvs_as_stored() {
        let mut b = Buf::new("version 2.00\n");
        b.u16(12).u8(40).u8(12).u32(3).u32(1);
        for (i, p) in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
            .into_iter()
            .enumerate()
        {
            let i = i as f32;
            b.vertex(40, p, UP);
            let uv_at = b.0.len() - 16; // uv sits 16 bytes before the record end
            b.0[uv_at..uv_at + 8].copy_from_slice(
                &[(i * 0.5).to_le_bytes(), (1.0 - i * 0.25).to_le_bytes()].concat(),
            );
        }
        b.face(0, 1, 2);
        let m = parse(&b.0).unwrap();
        assert_eq!(
            m.uvs,
            vec![
                Vec2::new(0.0, 1.0),
                Vec2::new(0.5, 0.75),
                Vec2::new(1.0, 0.5)
            ]
        );
        // Default helper uvs for the other binary versions.
        for bytes in [v3_bytes(), v4_bytes(), v5_bytes()] {
            let m = parse(&bytes).unwrap();
            assert_eq!(m.uvs.len(), m.positions.len());
            assert!(m.uvs.iter().all(|&t| t == Vec2::new(0.25, 0.75)));
        }
    }

    #[test]
    fn v1_and_obj_uvs_flip_v() {
        let src = "version 1.01\n1\n[0,0,0][0,1,0][0.25,0.25,0][1,0,0][0,1,0][1,0,0][0,0,1][0,1,0][0,1,0]";
        let m = parse(src.as_bytes()).unwrap();
        assert_eq!(
            m.uvs,
            vec![
                Vec2::new(0.25, 0.75),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 0.0)
            ]
        );

        let src = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0.25\nvt 0.5 1\n\
                   f 1/1 2/2/ 3/3\nf 1 2 3\n";
        let m = parse(src.as_bytes()).unwrap();
        assert_eq!(m.uvs.len(), 6);
        assert_eq!(
            &m.uvs[..3],
            &[
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 0.75),
                Vec2::new(0.5, 0.0)
            ]
        );
        assert!(m.uvs[3..].iter().all(|&t| t == Vec2::ZERO)); // no vt -> (0,0)
        assert!(parse(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nf 1/2 2/1 3/1\n").is_err());
        assert!(parse(b"v 0 0 0\nvt x 0\n").is_err());
    }

    /// Appends one v6/v7 chunk.
    fn chunk(b: &mut Buf, kind: &[u8; 8], version: u32, data: &[u8]) {
        b.0.extend_from_slice(kind);
        b.u32(version).u32(data.len() as u32);
        b.0.extend_from_slice(data);
    }

    /// COREMESH v1 payload: the five quad vertices and three faces.
    fn coremesh_v1() -> Vec<u8> {
        let mut c = Buf::default();
        c.u32(5);
        quad_verts(&mut c, 40);
        c.u32(3).face(0, 1, 2).face(0, 2, 3).face(2, 3, 4);
        c.0
    }

    /// v6 with LODS (LOD0 = faces 0..2), a SKINNING chunk, then COREMESH last
    /// so that every strict prefix is missing required data.
    fn v6_bytes() -> Vec<u8> {
        let mut b = Buf::new("version 6.00\n");
        let mut lods = Buf::default();
        lods.u16(0).u8(1).u32(3).u32(0).u32(2).u32(3);
        chunk(&mut b, b"LODS\0\0\0\0", 1, &lods.0);
        chunk(&mut b, b"SKINNING", 1, &[0; 12]);
        chunk(&mut b, b"COREMESH", 1, &coremesh_v1());
        b.0
    }

    #[test]
    fn v6_coremesh_with_lods() {
        let m = parse(&v6_bytes()).unwrap();
        assert_quad(&m);
        assert!(m.uvs.iter().all(|&t| t == Vec2::new(0.25, 0.75)));
    }

    #[test]
    fn v7_draco_bad_stream_is_an_error() {
        // A length past the chunk, then a stream that isn't Draco.
        let mut b = Buf::new("version 7.00\n");
        chunk(&mut b, b"COREMESH", 2, &[1, 2, 3, 4]);
        assert!(parse(&b.0).unwrap_err().contains("truncated Draco"));
        let mut b = Buf::new("version 7.00\n");
        chunk(
            &mut b,
            b"COREMESH",
            2,
            &[4, 0, 0, 0, b'N', b'O', b'P', b'E'],
        );
        assert!(parse(&b.0).unwrap_err().contains("Draco decode failed"));
    }

    #[test]
    fn v7_draco_decodes() {
        // A Draco triangle made with draco-core's encoder, in a COREMESH v2 chunk.
        // Roblox's layout: position, normal, uv (float32), tangent and colour
        // (4 × uint8), unique ids 0..4.
        use draco_core::draco_types::DataType as T;
        use draco_core::geometry_attribute::{GeometryAttributeType as G, PointAttribute};
        use draco_core::geometry_indices::{FaceIndex, PointIndex};
        let build = |all: bool| {
            let mut mesh = draco_core::mesh::Mesh::new();
            let layout: &[(G, u8, T)] = &[
                (G::Position, 3, T::Float32),
                (G::Normal, 3, T::Float32),
                (G::TexCoord, 2, T::Float32),
                (G::Generic, 4, T::Uint8),
                (G::Color, 4, T::Uint8),
            ];
            let tri = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
            for (id, (g, k, t)) in layout.iter().enumerate().take(if all { 5 } else { 1 }) {
                let mut a = PointAttribute::new();
                a.init(*g, *k, *t, false, 3);
                for p in 0..3 {
                    let bytes: Vec<u8> = if *t == T::Float32 {
                        (0..*k as usize)
                            .flat_map(|c| {
                                match id {
                                    0 => tri[p][c],
                                    1 => [0.0, 0.0, 1.0][c],
                                    _ => tri[p][c] * 0.5,
                                }
                                .to_le_bytes()
                            })
                            .collect()
                    } else {
                        vec![128; *k as usize]
                    };
                    a.buffer_mut().write(p * bytes.len(), &bytes);
                }
                mesh.add_attribute(a);
            }
            mesh.set_num_faces(1);
            mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
            let stream = encode_draco(&mesh);
            let mut data = (stream.len() as u32).to_le_bytes().to_vec();
            data.extend_from_slice(&stream);
            let mut b = Buf::new("version 7.00\n");
            chunk(&mut b, b"COREMESH", 2, &data);
            b.0
        };
        let m = parse(&build(true)).unwrap();
        assert_eq!(m.positions.len(), 3);
        assert!((m.max - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-3);
        assert!(m.normals.iter().all(|n| (n.z - 1.0).abs() < 1e-3));
        // Studio refuses a stream without all five.
        assert!(parse(&build(false))
            .unwrap_err()
            .contains("missing or has malformed attributes"));
    }

    /// draco-core's encoder (enabled for tests only, see Cargo.toml).
    fn encode_draco(mesh: &draco_core::mesh::Mesh) -> Vec<u8> {
        let mut enc = draco_core::mesh_encoder::MeshEncoder::new();
        enc.set_mesh(mesh.clone());
        let opts = draco_core::encoder_options::EncoderOptions::new();
        let mut out = draco_core::encoder_buffer::EncoderBuffer::new();
        enc.encode(&opts, &mut out).expect("draco encode");
        out.data().to_vec()
    }

    #[test]
    fn chunked_unknown_chunks_skipped() {
        let mut b = Buf::new("version 7.00\n");
        chunk(&mut b, b"HSRAVIS\0", 1, &[0xFF; 9]);
        chunk(&mut b, b"ZZZZZZZZ", 42, &[7; 5]);
        chunk(&mut b, b"COREMESH", 1, &coremesh_v1());
        chunk(&mut b, b"FACS\0\0\0\0", 1, &[]);
        chunk(&mut b, b"LODS\0\0\0\0", 9, &[0; 3]); // unknown LODS version: skipped
        let m = parse(&b.0).unwrap();
        assert_eq!(m.positions.len(), 9); // no usable LODS -> all faces
    }

    #[test]
    fn chunked_truncated_errors() {
        let full = v6_bytes();
        for len in "version 6.00\n".len() + 1..full.len() {
            assert!(parse(&full[..len]).is_err(), "prefix {len} parsed");
        }
        // Chunk size larger than the remaining data.
        let mut b = Buf::new("version 6.00\n");
        b.0.extend_from_slice(b"COREMESH");
        b.u32(1).u32(u32::MAX).u32(0);
        assert!(parse(&b.0).unwrap_err().contains("truncated"));
        // Counts inside COREMESH exceeding the chunk.
        let mut c = Buf::default();
        c.u32(u32::MAX);
        let mut b = Buf::new("version 6.00\n");
        chunk(&mut b, b"COREMESH", 1, &c.0);
        assert!(parse(&b.0).is_err());
    }

    #[test]
    fn garbage_never_panics() {
        let mut seed = 0x1234_5678u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for _ in 0..2000 {
            let len = (rnd() % 200) as usize;
            let mut data: Vec<u8> = (0..len).map(|_| rnd() as u8).collect();
            let prefix: &[u8] = match rnd() % 9 {
                0 => b"version 2.00\n",
                1 => b"version 3.00\n",
                2 => b"version 4.00\n",
                3 => b"version 5.00\n",
                4 => b"version 1.00\n",
                5 => b"version 6.00\nCOREMESH\x01\x00\x00\x00",
                6 => b"version 7.00\nLODS\0\0\0\0\x01\x00\x00\x00",
                _ => b"",
            };
            data.splice(0..0, prefix.iter().copied());
            let _ = parse(&data);
        }
    }
}

#[cfg(test)]
mod csg_projection_tests {
    use super::*;
    #[test]
    fn keeps_plane_ids_when_expanding_indexed_corners() {
        let mut b = b"CSGMDL".to_vec();
        b.extend(5u32.to_le_bytes());
        let key = csgmdl_key();
        for (i, v) in b.iter_mut().enumerate() { *v ^= key[i % key.len()]; }
        b.extend(3u16.to_le_bytes());
        for v in [0.0f32,0.0,0.0,1.0,0.0,0.0,0.0,1.0,0.0] { b.extend(v.to_le_bytes()); }
        b.extend(0u16.to_le_bytes()); // normals
        b.extend(0u16.to_le_bytes()); // colours
        b.extend(3u16.to_le_bytes()); b.extend([1,5,0]);
        b.extend(3u16.to_le_bytes());
        for v in [0.25f32,0.75,0.5,0.5,1.0,1.0] { b.extend(v.to_le_bytes()); }
        b.extend(0u16.to_le_bytes()); // tangents
        b.extend(3u32.to_le_bytes()); b.extend(3u32.to_le_bytes()); b.extend([0,1,1]);
        b.push(0); // LODs
        let m = parse(&b).unwrap();
        assert_eq!(m.projection_tags, [1,5,0]);
        assert_eq!(m.uvs[0], Vec2::new(0.25,0.75));
    }
}

#[cfg(test)]
mod solid_plane_bias_tests {
    use super::*;

    #[test]
    fn projection_bias_is_normalized_and_signed() {
        // Equal X/Y slopes prefer Y, even when the triangle area changes.
        for scale in [0.001, 1.0, 1000.0] {
            let a = Vec3::ZERO;
            let b = Vec3::Z * scale;
            let c = Vec3::new(1.0, -1.0, 0.0) * scale;
            assert_eq!(solid_mesh_projection_plane(a, b, c), 1);
            assert_eq!(solid_mesh_projection_plane(a, c, b), 4);
        }
        // The X bias beats Z for equal X/Z; degenerate triangles choose Y.
        assert_eq!(solid_mesh_projection_plane(Vec3::ZERO, Vec3::Y,
            Vec3::new(-1.0, 0.0, 1.0)), 0);
        assert_eq!(solid_mesh_projection_plane(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO), 1);
    }
}
