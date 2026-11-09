//! Geometry domain tests: Luau run in the real runtime, expectations taken
//! from Roblox Studio (offline Studio) running the same code.

use rbx_dom_weak::InstanceBuilder;

use crate::doc::Document;
use crate::runtime::{RunMode, Runtime, World};

pub fn run_with(src: &str, setup: impl FnOnce(&mut Document), steps: usize) -> String {
    let mut d = Document::new_place();
    setup(&mut d);
    let sss = d.find_child(d.dom.root_ref(), "ServerScriptService").unwrap();
    let s = d.dom.insert(sss, InstanceBuilder::new("Script").with_name("T").with_property("Source", src.to_string()));
    let rt = Runtime::new().unwrap();
    rt.enter(World::new(d, RunMode::Run));
    rt.bind_globals().unwrap();
    rt.run_script(s);
    for _ in 0..steps {
        rt.step(1.0 / 60.0);
    }
    let w = rt.leave();
    w.output.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join("\n")
}

pub fn run(src: &str) -> String {
    run_with(src, |_| {}, 3)
}

#[test]
fn editable_mesh_ids_and_corners() {
    let out = run(r#"
local AS=game:GetService("AssetService")
local em=AS:CreateEditableMesh()
local a=em:AddVertex(Vector3.new(0,0,0)) local b=em:AddVertex(Vector3.new(1,0,0)) local c=em:AddVertex(Vector3.new(0,1,0)) local d=em:AddVertex(Vector3.new(1,1,0))
print("ids",a,b,em:IdDebugString(a))
print(pcall(function() return em:AddFace(a,b,d,c) end))
local f=em:AddFace({a,b,d,c})
print("quad",f,table.concat(em:GetFaceNormals(f),","))
print(pcall(function() return em:AddFace({a,b}) end))
local t=em:AddTriangle(a,b,c)
print("counts",#em:GetNormals(),#em:GetUVs(),#em:GetColors())
em:Triangulate()
for _,ff in em:GetFaces() do print("face",em:IdDebugString(ff),table.concat(em:GetFaceVertices(ff),",")) end
print(pcall(function() return em:GetFaceVertices(f) end))
print(pcall(function() return em:GetPosition(12345) end))
print(pcall(function() return em:GetPosition(t) end))
local bone=em:AddBone({Name="B1",CFrame=CFrame.new(1,2,3),Virtual=false})
print("bone",bone,em:IdDebugString(bone),em:GetBoneName(bone),em:GetBoneParent(bone),em:GetBoneIsVirtual(bone))
print(pcall(function() return em:AddBone({Name="x"}) end))
print(pcall(function() return em:AddBone({Virtual=true}) end))
print(pcall(function() return em:AddBone({Name="B2",Virtual=true,CFrame=CFrame.new(),Parent=bone}) end))
print(pcall(function() return em:GetBoneByName("zz") end))
print("n",em:GetNormal(em:GetFaceNormals(t)[1]))
print("closest",em:FindClosestPointOnSurface(Vector3.new(0.2,0.2,1)))
print(pcall(function() em:SetVertexBones(a,{bone}) end), pcall(function() em:SetVertexBoneWeights(a,{0.5}) end), em:GetVertexBoneWeights(a)[1])
print(pcall(function() em:SetVertexBoneWeights(b,{0.5}) end))
print(pcall(function() em:SetFacsPose(Enum.FacsActionUnit.JawDrop,{bone},{CFrame.new()}) end))
print(pcall(function() em:SetFacsCorrectivePose({Enum.FacsActionUnit.JawDrop},{bone},{CFrame.new()}) end))
print("fixed", em.FixedSize)
"#);
    for want in [
        "ids 4294967297 4294967298 v1",
        "Unable to cast double to Array",
        "quad 21474836481 8589934593,8589934594,8589934595,8589934596",
        "AddFace called with 2 vertices, currently only supports 3 or 4 vertices",
        "counts 4 4 4",
        "face f2 4294967297,4294967298,4294967299",
        "face f3 4294967297,4294967298,4294967300",
        "face f1,2 4294967300,4294967299,4294967297",
        "false Invalid id",
        "Expected Vertex id, received value that doesn't appear to be an id",
        "Expected Vertex id, received Face id",
        "bone 25769803777 b1 B1 0 false",
        "Missing bone CFrame value",
        "Missing bone Name value",
        "Unexpected entry in table: Parent",
        "No bone named \"zz\"",
        "n 0, 0, 1",
        "true true 0.50196",
        "Number of bone weights provided does not match number of bone ids",
        "Only virtual bones can be used in Facs Poses, unable to use bone \"B1\", id 25769803777",
        "Correctives must have 2 or 3 FACS controls",
        "fixed false",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
}

#[test]
fn editable_mesh_merge_vertices() {
    let out = run(r#"
local em=game:GetService("AssetService"):CreateEditableMesh()
local a=em:AddVertex(Vector3.new(0,0,0)) local b=em:AddVertex(Vector3.new(1,0,0)) local c=em:AddVertex(Vector3.new(0,1,0)) local d=em:AddVertex(Vector3.new(0,1.001,0)) local e=em:AddVertex(Vector3.new(0,1,0))
local t1=em:AddTriangle(a,b,c) local t2=em:AddTriangle(a,b,d) local t3=em:AddTriangle(b,a,e)
local m=em:MergeVertices(0.01) local n=0 for k,v in m do n+=1 end
print("merge",n,#em:GetVertices(),table.concat(em:GetFaceVertices(t2),","),table.concat(em:GetFaceVertices(t3),","))
local m=em:MergeVertices(1.5) local n=0 for k,v in m do n+=1 end print("merge2",n)
print("adj",#em:GetAdjacentFaces(t1))
"#);
    assert!(out.contains("merge 2 5 4294967297,4294967298,4294967299 4294967298,4294967297,4294967299"), "{out}");
    assert!(out.contains("merge2 4"), "{out}");
}

const PATH_SETUP: &str = r#"
local function part(sz,pos,mat) local b=Instance.new("Part") b.Anchored=true b.Size=sz b.Position=pos if mat then b.Material=mat end b.Parent=workspace return b end
local PS=game:GetService("PathfindingService")
local function fmt(p) local s={} for _,wp in p:GetWaypoints() do table.insert(s, string.format("(%.1f,%.1f,%.1f %s %s)",wp.Position.X,wp.Position.Y,wp.Position.Z,wp.Action.Name,wp.Label)) end return table.concat(s," ") end
for _,c in workspace:GetChildren() do if c:IsA("BasePart") and not c:IsA("Terrain") then c:Destroy() end end
part(Vector3.new(100,1,100),Vector3.new(0,-0.5,0))
"#;

/// Studio: around a 2×10×30 wall, 16 waypoints every 4 studs, Plastic,
/// on the floor; jump over 1.5–6 stud walls, walk over ≤1, NoPath at 7.
#[test]
fn pathfinding_shapes() {
    let out = run_with(&format!("{PATH_SETUP}{}", r#"
local p=PS:CreatePath()
print("pre",p.Status.Name,p.ClassName,p.Name)
local w=part(Vector3.new(2,10,30),Vector3.new(0,5,0))
p:ComputeAsync(Vector3.new(-20,3,0),Vector3.new(20,3,0))
local wps=p:GetWaypoints()
print("around",p.Status.Name,#wps,tostring(wps[1].Position),tostring(wps[#wps].Position),wps[1].Label)
local maxz=0 for _,x in wps do maxz=math.max(maxz,math.abs(x.Position.Z)) end print("detour",maxz>15 and maxz<20)
local gap=(wps[2].Position-wps[1].Position).Magnitude print("gap",math.abs(gap-4)<0.05)
print("occl",p:CheckOcclusionAsync(1),pcall(function() return p:CheckOcclusionAsync(100) end))
w:Destroy()
for _,h in {1,2,6,7} do
 local x=part(Vector3.new(2,h,100),Vector3.new(0,h/2,0))
 local q=PS:CreatePath({WaypointSpacing=math.huge}) q:ComputeAsync(Vector3.new(-20,3,0),Vector3.new(20,3,0))
 print("wall",h,q.Status.Name,fmt(q))
 x:Destroy()
end
local r=PS:CreatePath({AgentCanJump=false}) local x=part(Vector3.new(2,3,100),Vector3.new(0,1.5,0)) r:ComputeAsync(Vector3.new(-20,3,0),Vector3.new(20,3,0)) print("nojump",r.Status.Name)
print(pcall(function() r:ComputeAsync(Vector3.new(0,0,0),Vector3.new(5000,0,0)) end))
print(pcall(function() PS:CreatePath({AgentRadius="x"}) end))
print(tostring(PathWaypoint.new(Vector3.new(1,2,3),Enum.PathWaypointAction.Jump,"x")), typeof(PathWaypoint.new()))
"#), |_| {}, 30);
    for want in [
        "pre NoPath Path Instance",
        "around Success",
        "-20, 0, 0 20, 0, 0 Plastic",
        "detour true",
        "gap true",
        "occl -1 false Start point value is invalid",
        "wall 1 Success",
        "wall 2 Success (-20.0,0.0,0.0 Walk Plastic)",
        "Jump Jump)",
        "wall 7 NoPath",
        "nojump NoPath",
        "false PathfindingService: path request is too long.",
        "false Unable to cast string to float",
        "{1, 2, 3}, Jump, x PathWaypoint",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
}

#[test]
fn pathfinding_blocked_events() {
    let out = run_with(&format!("{PATH_SETUP}{}", r#"
local p=PS:CreatePath() p:ComputeAsync(Vector3.new(-20,3,0),Vector3.new(20,3,0))
print("n",#p:GetWaypoints())
local ev={} p.Blocked:Connect(function(i) table.insert(ev,"B"..i) end) p.Unblocked:Connect(function(i) table.insert(ev,"U"..i) end)
local blk=part(Vector3.new(2,6,6),Vector3.new(5,3,0))
print("occ",p:CheckOcclusionAsync(1),p:CheckOcclusionAsync(6),p:CheckOcclusionAsync(7),p:CheckOcclusionAsync(8))
task.wait(1) print("ev",table.concat(ev,","))
blk:Destroy() task.wait(1) print("ev2",table.concat(ev,","))
"#), |_| {}, 200);
    assert!(out.contains("n 11"), "{out}");
    assert!(out.contains("occ 7 7 -1 -1"), "{out}");
    assert!(out.contains("ev B7"), "{out}");
    assert!(out.contains("ev2 B7,U7"), "{out}");
}

/// Studio: union of two 4-stud cubes (second at +2,+1,0) is an unparented
/// UnionOperation "Union" with Size 6,5,4 centred at 1,10.5,0; intersect
/// "Intersection" 2,3,4; empty intersect nil; GeometryService splits.
#[test]
fn csg_results() {
    let out = run_with(r#"
local function part(sz,pos,col) local b=Instance.new("Part") b.Anchored=true b.Size=sz b.Position=pos b.Color=col or Color3.new(1,0,0) b.Parent=workspace return b end
local a=part(Vector3.new(4,4,4),Vector3.new(0,10,0))
local b=part(Vector3.new(4,4,4),Vector3.new(2,11,0),Color3.new(0,1,0))
local u=a:UnionAsync({b})
print("u",u.ClassName,u.Name,u.Parent,tostring(u.Position),tostring(u.Size),tostring(u.Color),u.UsePartColor,u.CollisionFidelity.Name)
local s=a:SubtractAsync({b}) print("s",s.ClassName,tostring(s.Position),tostring(s.Size))
local i=a:IntersectAsync({b}) print("i",i.ClassName,i.Name,tostring(i.Position),tostring(i.Size))
local far=part(Vector3.new(2,2,2),Vector3.new(30,10,0))
print("none",a:IntersectAsync({far}))
print(pcall(function() return a:UnionAsync({Instance.new("Folder")}) end))
local GS=game:GetService("GeometryService")
local r=GS:UnionAsync(a,{b,far}) print("gs",#r,tostring(r[1].Size),tostring(r[2].Size))
local r=GS:UnionAsync(a,{b,far},{SplitApart=false}) print("gs1",#r,tostring(r[1].Size))
print(#GS:IntersectAsync(a,{far}))
print(pcall(function() return GS:UnionAsync(a,{b},{Foo=1}) end))
u.Parent=workspace a:Destroy() b:Destroy()
local hit=workspace:Raycast(Vector3.new(3.9,50,0),Vector3.new(0,-100,0)) print("hit",hit and hit.Instance.Name, hit and math.floor(hit.Position.Y*10+0.5)/10)
"#, |_| {}, 30);
    for want in [
        "u UnionOperation Union nil 1, 10.5, 0 6, 5, 4 1, 0, 0 false Default",
        "s UnionOperation 0, 10, 0 4, 4, 4",
        "i IntersectOperation Intersection 1, 10.5, 0 2, 3, 4",
        "none nil",
        "CSG: One of the primitives was of unknown type (error -4)",
        "gs 2 6, 5, 4 2, 2, 2",
        "gs1 1 33, 5, 4",
        "Key 'Foo' is not a valid member of CSGOptions",
        "hit Union 13",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
}


/// Studio: a 1×1 box over a 2×2 quad mapped to an 8×8 image paints the
/// middle 4×4; missing keys and zero sizes error as Studio.
#[test]
fn editable_image_projected() {
    let out = run(r#"
local AS=game:GetService("AssetService")
local img=AS:CreateEditableImage({Size=Vector2.new(8,8)})
local dec=AS:CreateEditableImage({Size=Vector2.new(2,2)})
dec:DrawRectangle(Vector2.zero,Vector2.new(2,2),Color3.new(1,0,0),0,Enum.ImageCombineType.Overwrite)
local em=AS:CreateEditableMesh()
local a=em:AddVertex(Vector3.new(-1,-1,0)) local b=em:AddVertex(Vector3.new(1,-1,0)) local c=em:AddVertex(Vector3.new(1,1,0)) local d=em:AddVertex(Vector3.new(-1,1,0))
local f=em:AddFace({a,b,c,d})
local uvs=em:GetFaceUVs(f) em:SetUV(uvs[1],Vector2.new(0,1)) em:SetUV(uvs[2],Vector2.new(1,1)) em:SetUV(uvs[3],Vector2.new(1,0)) em:SetUV(uvs[4],Vector2.new(0,0))
print(pcall(function() img:SampleImageProjected(em,dec,{},{}) end))
img:DrawImageProjected(em,{Direction=Vector3.new(0,0,-1),Position=Vector3.new(0,0,1),Size=Vector3.new(1,1,4),Up=Vector3.new(0,1,0)},{Decal=dec,ColorBlendType=Enum.ImageCombineType.BlendSourceOver,AlphaBlendType=Enum.ImageAlphaType.Default,BlendIntensity=1,FadeAngle=90})
local buf=img:ReadPixelsBuffer(Vector2.zero,Vector2.new(8,8)) local rows={}
for y=0,7 do local s="" for x=0,7 do s..=(buffer.readu8(buf,(y*8+x)*4+3)>0 and (buffer.readu8(buf,(y*8+x)*4)>128 and "R" or "x") or ".") end table.insert(rows,s) end print("img",table.concat(rows,"|"))
print(pcall(function() img:DrawImageProjected(em,{Direction=Vector3.new(0,0,-1),Position=Vector3.new(0,0,1),Size=Vector3.new(1,1,4)},{Decal=dec}) end))
print(pcall(function() img:DrawImageProjected(em,{Direction=Vector3.new(0,0,-1),Position=Vector3.new(0,0,1),Size=Vector3.new(0,1,4),Up=Vector3.yAxis},{Decal=dec,ColorBlendType=Enum.ImageCombineType.BlendSourceOver,AlphaBlendType=Enum.ImageAlphaType.Default,BlendIntensity=1,FadeAngle=90}) end))
"#);
    for want in [
        "false EditableImage::DrawImageProjected Param ProjectionParams, is missing Direction",
        "img ........|........|..RRRR..|..RRRR..|..RRRR..|..RRRR..|........|........",
        "false EditableImage::DrawImageProjected Param ProjectionParams, is missing Up",
        "false Projection size cannot be 0 in any dimension.",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
}

#[test]
fn mesh_part_and_substitute() {
    let out = run_with(r#"
local a=Instance.new("MeshPart") a.Size=Vector3.new(1,2,3) a.Parent=workspace
local b=Instance.new("MeshPart") b.Size=Vector3.new(5,5,5)
print(pcall(function() return a:GetSurfaceSlotsAsync() end))
print(pcall(function() a:ApplyMesh(b) end), tostring(a.Size))
print(pcall(function() a:ApplyMesh(Instance.new("Part")) end))
local u1=Instance.new("Part") u1.Position=Vector3.new(0,50,0) u1.Anchored=true u1.Parent=workspace local U=u1:UnionAsync({})
local p2=Instance.new("Part") p2.Size=Vector3.new(8,8,8) p2.Parent=workspace local V=p2:UnionAsync({})
print(pcall(function() U:SubstituteGeometry(V) end), tostring(U.Size), tostring(U.Position))
print(pcall(function() U:SubstituteGeometry(p2) end))
"#, |_| {}, 20);
    for want in [
        "false MeshPart:GetSurfaceSlotsAsync is not yet enabled",
        "true 1, 2, 3",
        "false ApplyMesh requires an MeshPart object",
        "true 8, 8, 8 0, 50, 0",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
}

#[test]
fn geometry_service_misc() {
    let out = run_with(r#"
local GS=game:GetService("GeometryService")
local a=Instance.new("Part") a.Anchored=true a.Size=Vector3.new(4,4,4) a.Position=Vector3.new(0,10,0) a.Parent=workspace
local far=Instance.new("Part") far.Anchored=true far.Size=Vector3.new(2,2,2) far.Position=Vector3.new(3,10,0) far.Parent=workspace
local w=Instance.new("WeldConstraint") w.Part0=a w.Part1=far w.Parent=a
local att=Instance.new("Attachment") att.Parent=a
local res=GS:UnionAsync(a,{})
local rec=GS:CalculateConstraintsToPreserve(a,res)
print("ctp",#rec)
for _,r in rec do local k={} for n in r do table.insert(k,n) end table.sort(k) print("row",table.concat(k,",")) end
print(pcall(function() return GS:CalculateConstraintsToPreserve(a,res,{Foo=1}) end))
local sites=GS:GenerateFragmentSites(a,{SiteSpacing=2}) print("sites",#sites)
print(pcall(function() return GS:GenerateFragmentSites(a,{SiteSpacing=-1}) end))
local frags=GS:FragmentAsync(a,{Vector3.new(-1,10,0),Vector3.new(1,10,0)})
print("frags",#frags,frags[1].Index,frags[1].Instance.ClassName,tostring(frags[1].Instance.Position),tostring(frags[1].Instance.Size),frags[1].Instance.Parent,frags[2].Index,tostring(frags[2].Instance.Size))
local m=Instance.new("Model") frags[2].Instance.Parent=m m.Parent=workspace print("fragbox",tostring((m:GetBoundingBox()).Position))
print("default",#GS:GenerateFragmentSites(a), #GS:GenerateFragmentSites(a,{SiteSpacing=1}), #GS:GenerateFragmentSites(Instance.new("Part"),{}))
local function count(o) local s=GS:GenerateFragmentSites(a,o) local n=0 for _,v in s do n+=typeof(v)=="table" and #v or 1 end return typeof(s[1]),n end
local t1,n1=count({Origin=Vector3.new(0,10,0),Radius=1}) local _,n2=count({Origin=Vector3.new(0,10,0),Radius=1,SiteSpacing=1}) print("sphere",t1,n1,n2)
print(pcall(function() return GS:GenerateFragmentSites(a,{Foo=1}) end))
print(pcall(function() return GS:FragmentAsync(a,{Vector3.new(0,10,0)},{Foo=1}) end))
local one=GS:FragmentAsync(a,{Vector3.new(0,10,0)}) print("single",#one,tostring(one[1].Instance.Size))
print(pcall(function() return GS:BakeToMeshAsync(a) end))
local s=GS:SweepPartAsync(far,{CFrame.new(0,0,0),CFrame.new(10,0,0)}) print("sweep",s.ClassName,tostring(s.Position),tostring(s.Size),s.Parent)
far.Orientation=Vector3.new(0,90,0)
local s=GS:SweepPartAsync(far,{CFrame.new(0,0,0),CFrame.new(10,0,0)}) local m2=Instance.new("Model") s.Parent=m2 m2.Parent=workspace
local function r3(v) return string.format("%.2f,%.2f,%.2f",v.X,v.Y,v.Z) end
print("sweeprot",r3(s.Position),r3(s.Orientation),r3(s.Size),r3((m2:GetBoundingBox()).Position))
local s=GS:SweepPartAsync(far,{CFrame.new(0,0,0)}) print("sweep1",tostring(s.Position),tostring(s.Size))
print(pcall(function() return GS:SweepPartAsync(far,{CFrame.new(),CFrame.new(1,0,0)},{Foo=1}) end))
print(pcall(function() return GS:CreateBasicMeshPart(Enum.BasicMeshPartShape.Capsule) end))
"#, |_| {}, 30);
    for want in [
        "ctp 1",
        "row WeldConstraint,WeldConstraintParent,WeldConstraintPart0,WeldConstraintPart1",
        "Key 'Foo' is not a valid member of CSGConstraintOptions",
        "sites 8",
        "SiteSpacing must be a positive number.",
        "frags 2 1 MeshPart 0, 10, 0 2, 4, 4 nil 2 2, 4, 4",
        "fragbox 1, 10, 0",
        "default 8 64 8",
        "sphere table 343 125",
        "Key 'Foo' is not a valid option.",
        "single 1 4, 4, 4",
        "BakeToMeshAsync is not a valid member of GeometryService \"GeometryService\"",
        "sweep MeshPart 3, 10, 0 12, 2, 2 nil",
        "90.00,0.00 2.00,2.00,12.00 5.00,0.00,", "sweeprot 3.00,10.00,0.00 ",
        "sweep1 3, 10, 0 2, 2, 2",
        "This API is not yet released!",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
}

/// Survey (run by hand): RS_UNION_SURVEY=place.rbxl cargo test union_survey -- --ignored --nocapture
/// Per union: the mesh's bounds against Size / InitialSize.
#[test]
#[ignore]
fn union_survey() {
    use rbx_dom_weak::types::Variant;
    let path = std::env::var("RS_UNION_SURVEY").unwrap();
    let d = crate::doc::Document::open(std::path::Path::new(&path)).unwrap();
    let mut n = 0;
    for r in d.subtree(d.dom.root_ref()) {
        let Some(c) = d.class_of(r) else { continue };
        if !crate::doc::is_a(&c, "PartOperation") {
            continue;
        }
        let Some(m) = crate::scene::union_mesh(&d, r) else { continue };
        let Some(bytes) = crate::assets::generated(&m.id) else { continue };
        let Ok(md) = crate::mesh::parse(&bytes) else { continue };
        let v = |k: &str| match d.prop(r, k) {
            Some(Variant::Vector3(v)) => format!("{:.3},{:.3},{:.3}", v.x, v.y, v.z),
            _ => "-".into(),
        };
        let kind = if matches!(d.prop(r, "SolidMeshHolder"), Some(Variant::NetAssetRef(ref h)) if !h.data().is_empty()) { "solid" } else { "csgmdl" };
        let c = (md.min + md.max) * 0.5;
        let e = md.max - md.min;
        println!("{kind} center {:.3},{:.3},{:.3} ext {:.3},{:.3},{:.3} Size {} Initial {}", c.x, c.y, c.z, e.x, e.y, e.z, v("Size"), v("InitialSize"));
        n += 1;
        if n > 40 {
            break;
        }
    }
}

/// Runs `src`, then hands back the world's document.
fn run_doc(src: &str, steps: usize) -> (String, crate::doc::Document) {
    let mut d = Document::new_place();
    let sss = d.find_child(d.dom.root_ref(), "ServerScriptService").unwrap();
    let s = d.dom.insert(sss, InstanceBuilder::new("Script").with_name("T").with_property("Source", src.to_string()));
    let rt = Runtime::new().unwrap();
    rt.enter(World::new(d, RunMode::Run));
    rt.bind_globals().unwrap();
    rt.run_script(s);
    for _ in 0..steps {
        rt.step(1.0 / 60.0);
    }
    let mut w = rt.leave();
    let out = w.output.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>().join("\n");
    let doc = std::mem::replace(w.dm(), Document::empty());
    (out, doc)
}

/// Studio (offline Studio, main part turned 30° about Y, second cube at 2,11,0):
/// BasePart:UnionAsync centres on the geometry (1.5,10.5,0.134), keeps the
/// main part's rotation, Size 6.464,5,5.732; GeometryService keeps the main
/// part's CFrame (0,10,0) with the same Size and an identity PivotOffset,
/// its geometry's box centred at 1.5,10.5,0.134; doubling Size moves that
/// to 3,11,0.268. Subtract 0,10,0 4,4,4; intersect 0.5,10.5,-0.134
/// 3,3,3.732 (GeometryService: at 0,10,0).
#[test]
fn csg_placement_as_studio() {
    let (out, _) = run_doc(r#"
for _,c in workspace:GetChildren() do if c:IsA("BasePart") and not c:IsA("Terrain") then c:Destroy() end end
local GS=game:GetService("GeometryService")
local function part(sz,cf) local b=Instance.new("Part") b.Anchored=true b.Size=sz b.CFrame=cf b.Parent=workspace return b end
local function r3(v) return string.format("%.3f,%.3f,%.3f",v.X,v.Y,v.Z) end
local a=part(Vector3.new(4,4,4),CFrame.new(0,10,0)*CFrame.Angles(0,math.rad(30),0))
local b=part(Vector3.new(4,4,4),CFrame.new(2,11,0))
local u=a:UnionAsync({b}) print("bp",r3(u.Position),r3(u.Orientation),r3(u.Size),u.PivotOffset==CFrame.identity)
local g=GS:UnionAsync(a,{b})[1] print("gs",r3(g.Position),r3(g.Orientation),r3(g.Size),r3(g.PivotOffset.Position))
print(pcall(function() return GS:UnionAsync(a,{b},{UseOffCenterCoordinates=false}) end))
local s=a:SubtractAsync({b}) print("sub",r3(s.Position),r3(s.Orientation),r3(s.Size))
local i=a:IntersectAsync({b}) print("int",r3(i.Position),r3(i.Orientation),r3(i.Size))
local gi=GS:IntersectAsync(a,{b})[1] print("gsint",r3(gi.Position),r3(gi.Orientation),r3(gi.Size))
g.Parent=workspace a:Destroy() b:Destroy()
local m=Instance.new("Model") g.Parent=m m.Parent=workspace local cf=m:GetBoundingBox() print("gsbbox",r3(cf.Position))
g.Size=g.Size*2 local cf=m:GetBoundingBox() print("scaled",r3(g.Position),r3(cf.Position))
"#, 30);
    for want in [
        "bp 1.500,10.500,0.134 0.000,30.000,0.000 6.464,5.000,5.732 true",
        "gs 0.000,10.000,0.000 0.000,30.000,0.000 6.464,5.000,5.732 0.000,0.000,0.000",
        "Key 'UseOffCenterCoordinates' is not a valid member of CSGOptions",
        "sub 0.000,10.000,0.000 0.000,30.000,0.000 4.000,4.000,4.000",
        "int 0.500,10.500,-0.134 0.000,30.000,0.000 3.000,3.000,3.732",
        "gsint 0.000,10.000,0.000 0.000,30.000,0.000 3.000,3.000,3.732",
        "gsbbox 1.500,10.500,0.134",
        "scaled 0.000,10.000,0.000 3.000,11.000,0.268",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
}

/// A GeometryService union (CFrame 0,10,0; geometry x -2..4, top 13 where
/// x ≥ 0): drawn, ray-tested and collided where its geometry is, also after
/// a save and reload; a centred union draws at its CFrame as before.
#[test]
fn off_centre_union_draws_collides_and_ray_tests() {
    use glam::Vec3;
    let (out, doc) = run_doc(r#"
for _,c in workspace:GetChildren() do if c:IsA("BasePart") and not c:IsA("Terrain") then c:Destroy() end end
local GS=game:GetService("GeometryService")
local function part(sz,pos) local b=Instance.new("Part") b.Anchored=true b.Size=sz b.Position=pos b.Parent=workspace return b end
local a=part(Vector3.new(4,4,4),Vector3.new(0,10,0)) local b=part(Vector3.new(4,4,4),Vector3.new(2,11,0))
local g=GS:UnionAsync(a,{b})[1] g.Name="GS" g.Parent=workspace
local c=part(Vector3.new(4,4,4),Vector3.new(40,10,0)) local d=part(Vector3.new(4,4,4),Vector3.new(42,11,0))
local u=c:UnionAsync({d}) u.Name="BP" u.Parent=workspace
for _,p in {a,b,c,d} do p:Destroy() end
local function ray(x) local h=workspace:Raycast(Vector3.new(x,50,0),Vector3.new(0,-100,0)) return h and (h.Instance.Name..string.format(" %.2f",h.Position.Y)) or "miss" end
print("ray", ray(3.9), ray(-1.9), ray(-2.5), ray(43.9))
print("parts", #workspace:GetPartBoundsInBox(CFrame.new(3.5,12.5,0),Vector3.new(0.5,0.5,0.5)))
"#, 30);
    assert!(out.contains("ray GS 13.00 GS 12.00 miss BP 13.00"), "{out}");
    assert!(out.contains("parts 1"), "{out}");

    let check = |doc: &mut crate::doc::Document, label: &str| {
        let ws = doc.workspace().unwrap();
        let find = |doc: &crate::doc::Document, n: &str| doc.find_named(ws, n).unwrap();
        let gs = find(doc, "GS");
        let bp = find(doc, "BP");
        let draw = |r| crate::scene::part_draw(doc, doc.dom.get_by_ref(r).unwrap()).unwrap().draw.xf.pos;
        assert!((draw(gs) - Vec3::new(1.0, 10.5, 0.0)).length() < 1e-3, "{label}: drawn at {:?}", draw(gs));
        assert!((draw(bp) - Vec3::new(41.0, 10.5, 0.0)).length() < 1e-3, "{label}: centred union drawn at {:?}", draw(bp));
        // 1-stud cubes dropped on it: x 2.5 lands on its top (13); x -2.6
        // has no geometry under it (a collider centred on the CFrame would
        // catch it at 12); with Box fidelity x 3.4 lands on the box top.
        // (Default fidelity is a convex decomposition: it rounds the
        // geometry's outer corners, as for MeshParts.)
        let drop = |doc: &mut crate::doc::Document, x: f32| {
            doc.dom.insert(ws, InstanceBuilder::new("Part").with_name("Drop")
                .with_property("Size", rbx_dom_weak::types::Vector3::new(1.0, 1.0, 1.0))
                .with_property("CFrame", crate::math::Xform { pos: Vec3::new(x, 16.0, 0.0), rot: glam::Mat3::IDENTITY }.to_cframe()))
        };
        let settle = |doc: &mut crate::doc::Document, cubes: &[rbx_dom_weak::types::Ref]| -> Vec<f32> {
            let mut p = crate::physics::Physics::new(doc);
            for _ in 0..120 {
                p.sync(doc);
                p.step(doc, 1.0 / 60.0);
            }
            let ys = cubes.iter().map(|c| crate::scene::xform_of(doc, *c).unwrap().pos.y).collect();
            doc.remove(cubes);
            ys
        };
        let (c1, c2) = (drop(doc, 2.5), drop(doc, -2.6));
        let ys = settle(doc, &[c1, c2]);
        assert!((ys[0] - 13.5).abs() < 0.2, "{label}: cube on top at {}", ys[0]);
        assert!(ys[1] < 5.0, "{label}: cube beside the geometry stopped at {}", ys[1]);
        doc.set_prop_live(gs, rbx_dom_weak::ustr("CollisionFidelity"), rbx_dom_weak::types::Variant::Enum(rbx_dom_weak::types::Enum::from_u32(2)));
        let c3 = drop(doc, 3.4);
        let ys = settle(doc, &[c3]);
        assert!((ys[0] - 13.5).abs() < 0.2, "{label}: Box fidelity cube at {}", ys[0]);
        doc.set_prop_live(gs, rbx_dom_weak::ustr("CollisionFidelity"), rbx_dom_weak::types::Variant::Enum(rbx_dom_weak::types::Enum::from_u32(0)));
    };
    let mut doc = doc;
    check(&mut doc, "made");
    let bytes = doc.encode_all(false).unwrap();
    let mut back = crate::doc::Document::from_place_bytes(&bytes, "reload".into()).unwrap();
    check(&mut back, "reloaded");
}

/// Studio: a 20-stud tower with a truss on its side; AgentCanClimb walks
/// to 2 studs before the truss, then a Custom "Climb" waypoint at the truss
/// top, then walks on (NoPath without AgentCanClimb or with Costs.Climb =
/// math.huge). Coming down it drops off the edge with a "Jump" waypoint at
/// the landing; drops work to 100 studs (110: NoPath) and need AgentCanJump
/// and a finite Costs.Jump.
#[test]
fn pathfinding_climb_and_drop() {
    let out = run_with(&format!("{PATH_SETUP}{}", r#"
local function p2(cls,sz,pos) local b=Instance.new(cls) b.Anchored=true b.Size=sz b.Position=pos b.Parent=workspace return b end
p2("Part",Vector3.new(10,20,10),Vector3.new(20,10,0))
p2("TrussPart",Vector3.new(2,20,2),Vector3.new(14,10,0))
for _,c in {true,false} do
 local p=PS:CreatePath({AgentCanClimb=c,WaypointSpacing=math.huge}) p:ComputeAsync(Vector3.new(-10,3,0),Vector3.new(22,23,0)) print("climb",c,p.Status.Name,fmt(p))
end
local p=PS:CreatePath({AgentCanClimb=true,WaypointSpacing=math.huge,Costs={Climb=math.huge}}) p:ComputeAsync(Vector3.new(-10,3,0),Vector3.new(22,23,0)) print("costclimb",p.Status.Name)
local p=PS:CreatePath({WaypointSpacing=math.huge}) p:ComputeAsync(Vector3.new(22,23,0),Vector3.new(-10,3,0)) print("down",p.Status.Name,fmt(p))
local p=PS:CreatePath({WaypointSpacing=math.huge,AgentCanJump=false}) p:ComputeAsync(Vector3.new(22,23,0),Vector3.new(-10,3,0)) print("downnojump",p.Status.Name)
local p=PS:CreatePath({WaypointSpacing=math.huge,Costs={Jump=math.huge}}) p:ComputeAsync(Vector3.new(22,23,0),Vector3.new(-10,3,0)) print("downjumpcost",p.Status.Name)
for _,h in {100,110} do
 local t=p2("Part",Vector3.new(10,h,10),Vector3.new(-30,h/2,30))
 local p=PS:CreatePath({WaypointSpacing=math.huge}) p:ComputeAsync(Vector3.new(-28,h+3,30),Vector3.new(-30,3,0)) print("drop",h,p.Status.Name)
 t:Destroy()
end
"#), |_| {}, 60);
    for want in [
        "climb true Success (-10.0,0.0,0.0 Walk Plastic)",
        "Custom Climb) (22.0,20.0,0.0 Walk Plastic)",
        "climb false NoPath",
        "costclimb NoPath",
        "down Success (22.0,20.0,0.0 Walk Plastic)",
        "Jump Jump) (-10.0,0.0,0.0 Walk Plastic)",
        "downnojump NoPath",
        "downjumpcost NoPath",
        "drop 100 Success",
        "drop 110 NoPath",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
}

/// Studio: a 100-stud wall with an 8×8 doorway cut by SubtractAsync; the
/// path goes through the doorway with every CollisionFidelity (Studio's
/// navmesh follows the union's geometry); CanCollide=false is ignored. A
/// MeshPart of the same geometry (one FragmentAsync piece) behaves the same.
#[test]
fn pathfinding_through_mesh_geometry() {
    let out = run_with(&format!("{PATH_SETUP}{}", r#"
local wall=part(Vector3.new(100,12,2),Vector3.new(0,6,0))
local hole=part(Vector3.new(8,8,6),Vector3.new(0,4,0))
local u=wall:SubtractAsync({hole}) wall:Destroy() hole:Destroy()
for _,fid in {Enum.CollisionFidelity.Default,Enum.CollisionFidelity.Hull,Enum.CollisionFidelity.Box,Enum.CollisionFidelity.PreciseConvexDecomposition} do
 local v=u:Clone() v.CollisionFidelity=fid v.Parent=workspace
 local p=PS:CreatePath({WaypointSpacing=math.huge}) p:ComputeAsync(Vector3.new(0,3,-20),Vector3.new(0,3,20)) print(fid.Name,p.Status.Name,fmt(p))
 v:Destroy()
end
local v=u:Clone() v.CanCollide=false v.Parent=workspace
local p=PS:CreatePath({WaypointSpacing=math.huge}) p:ComputeAsync(Vector3.new(0,3,-20),Vector3.new(20,3,20)) print("nocollide",p.Status.Name,fmt(p))
v:Destroy()
local m=game:GetService("GeometryService"):FragmentAsync(u,{Vector3.new(0,6,0)})[1].Instance m.Anchored=true m.Parent=workspace
local p=PS:CreatePath({WaypointSpacing=math.huge}) p:ComputeAsync(Vector3.new(0,3,-20),Vector3.new(0,3,20)) print("meshpart",p.Status.Name,fmt(p))
local p=PS:CreatePath({WaypointSpacing=math.huge}) p:ComputeAsync(Vector3.new(30,3,-20),Vector3.new(30,3,20)) print("around",p.Status.Name,#p:GetWaypoints())
"#), |_| {}, 40);
    for want in [
        "Default Success (0.0,0.0,-20.0 Walk Plastic) (0.0,0.0,20.0 Walk Plastic)",
        "Hull Success (0.0,0.0,-20.0 Walk Plastic) (0.0,0.0,20.0 Walk Plastic)",
        "Box Success (0.0,0.0,-20.0 Walk Plastic) (0.0,0.0,20.0 Walk Plastic)",
        "PreciseConvexDecomposition Success (0.0,0.0,-20.0 Walk Plastic) (0.0,0.0,20.0 Walk Plastic)",
        "nocollide Success (0.0,0.0,-20.0 Walk Plastic) (20.0,0.0,20.0 Walk Plastic)",
        "meshpart Success (0.0,0.0,-20.0 Walk Plastic) (0.0,0.0,20.0 Walk Plastic)",
        "around Success",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
}

/// Studio's CalculateConstraintsToPreserve rows for a source cube A with a
/// weld to a touching part (W1, under A), a weld from a far part (W2, in
/// Workspace), a NoCollisionConstraint (NC, under A), a rope on an
/// attachment (Rope, under A) and an attachment with no constraint (Lone),
/// split into H1 (lower half) and H2 (upper half).
#[test]
fn constraints_to_preserve_as_studio() {
    let out = run(r#"
local GS=game:GetService("GeometryService")
local function part(n,sz,pos) local b=Instance.new("Part") b.Name=n b.Anchored=true b.Size=sz b.Position=pos b.Parent=workspace return b end
local a=part("A",Vector3.new(4,4,4),Vector3.new(0,10,0))
local touch=part("Touch",Vector3.new(2,2,2),Vector3.new(3,10,0))
local away=part("Away",Vector3.new(2,2,2),Vector3.new(30,10,0))
local w1=Instance.new("WeldConstraint") w1.Name="W1" w1.Part0=a w1.Part1=touch w1.Parent=a
local w2=Instance.new("WeldConstraint") w2.Name="W2" w2.Part0=away w2.Part1=a w2.Parent=workspace
local nc=Instance.new("NoCollisionConstraint") nc.Name="NC" nc.Part0=a nc.Part1=away nc.Parent=a
local att=Instance.new("Attachment") att.Name="Att" att.Position=Vector3.new(1,1,1) att.Parent=a
local lone=Instance.new("Attachment") lone.Name="Lone" lone.Position=Vector3.new(-1,-1,-1) lone.Parent=a
local att2=Instance.new("Attachment") att2.Parent=away
local rope=Instance.new("RopeConstraint") rope.Name="Rope" rope.Attachment0=att rope.Attachment1=att2 rope.Parent=a
local h1=part("H1",Vector3.new(4,2,4),Vector3.new(0,9,0)) local h2=part("H2",Vector3.new(4,2,4),Vector3.new(0,11,0))
local function dump(tag,rec) for i,r in rec do local k={} for n,v in r do table.insert(k,n.."="..(typeof(v)=="Instance" and v.Name or tostring(v))) end table.sort(k) print(tag,i,table.concat(k," ")) end end
dump("default",GS:CalculateConstraintsToPreserve(a,{h1,h2}))
dump("all",GS:CalculateConstraintsToPreserve(a,{h1,h2},{weldConstraintPreserve=Enum.WeldConstraintPreserve.All}))
dump("none",GS:CalculateConstraintsToPreserve(a,{h1,h2},{weldConstraintPreserve=Enum.WeldConstraintPreserve.None}))
dump("far",GS:CalculateConstraintsToPreserve(a,{away}))
dump("tol",GS:CalculateConstraintsToPreserve(a,{away},{tolerance=100}))
print(pcall(function() return GS:CalculateConstraintsToPreserve(nil,{h1}) end))
print(pcall(function() return GS:CalculateConstraintsToPreserve(a,{h1,nil,h2}) end))
"#);
    let studio = r#"default 1 Attachment=Att AttachmentParent=H2 Constraint=Rope ConstraintParent=H2
default 2 NoCollisionConstraint=NC NoCollisionParent=H1 NoCollisionPart0=H1 NoCollisionPart1=Away
default 3 NoCollisionConstraint=NC NoCollisionParent=H2 NoCollisionPart0=H2 NoCollisionPart1=Away
default 4 WeldConstraint=W1 WeldConstraintParent=H1 WeldConstraintPart0=H1 WeldConstraintPart1=Touch
default 5 WeldConstraint=W1 WeldConstraintParent=H2 WeldConstraintPart0=H2 WeldConstraintPart1=Touch
default 6 WeldConstraint=W2 WeldConstraintPart0=Away WeldConstraintPart1=A
all 6 WeldConstraint=W2 WeldConstraintPart0=Away WeldConstraintPart1=H1
all 7 WeldConstraint=W2 WeldConstraintPart0=Away WeldConstraintPart1=H2
none 3 NoCollisionConstraint=NC NoCollisionParent=H2 NoCollisionPart0=H2 NoCollisionPart1=Away
far 1 Attachment=Att Constraint=Rope
far 2 NoCollisionConstraint=NC NoCollisionParent=Away NoCollisionPart0=Away NoCollisionPart1=Away
far 3 WeldConstraint=W1 WeldConstraintPart0=A WeldConstraintPart1=Touch
far 4 WeldConstraint=W2 WeldConstraintPart0=Away WeldConstraintPart1=Away
tol 1 Attachment=Att AttachmentParent=Away Constraint=Rope ConstraintParent=Away
false CalculateConstraintsToPreserve: source cannot be nil
false CalculateConstraintsToPreserve: cannot have nil as a destination"#;
    for want in studio.lines() {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
    assert!(!out.contains("none 4"), "{out}");
    assert!(!out.contains("all 8"), "{out}");
}
