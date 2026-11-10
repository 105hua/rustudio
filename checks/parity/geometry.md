# Geometry domain (parity push, 2026-11-07)

Code: `src/domains/geometry.rs` (hooks), `geometry_nav.rs` (pathfinding),
`geometry_csg.rs` (BSP CSG + CSGMDL writer), `geometry_ops.rs` (CSG and
GeometryService calls), `geometry_tests.rs`; `src/editable.rs` +
`src/editable_mesh.rs` (EditableMesh). Every measurement below is from an
offline Studio in an offline Studio's MCP (own instance, empty place).

## EditableMesh (measured)

- Why the coverage probe saw 53 stubs: they really were missing. editable.rs
  had the 2024 API (AddTriangle, Get/SetFace*, Batch{Add,Get,Set}Values…);
  Studio's current API adds AddFace (3–4 vertex faces), bones, FACS poses,
  per-corner attribute getters/setters, MergeVertices, Triangulate, the
  Batch*Attributes calls and IdDebugString. `FixedSize` read as a stub because
  the probe's EditableMesh is a bare DOM instance with no store entry; it now
  reads false on any EditableMesh.
- Ids: int64 `((generation-1)*8 + kind) << 32 | slot` with 1-based slots and
  kinds Vertex 1, Normal 2, Color 3, UV 4, Face 5, Bone 6. A removed slot is
  reused with the next generation: `IdDebugString` → `f1,2`. Lists come back
  in creation order (an ordered list with slot reuse; Triangulate's
  quad → (0,1,2) new slot, quad removed, (2,3,0) reusing its slot matches
  Studio's `f2, f3, f1,2`).
- AddTriangle/AddFace: each corner gets its vertex's default normal / UV /
  colour, made on the vertex's first face (auto normal = area-weighted face
  average, UV 0,0, white alpha 1); later faces reuse them (counts 3/3/3 →
  4/4/4 when one new vertex joins). `AddNormal()` with nil and `ResetNormal`
  give automatic normals; an unused one reads 0,0,0.
- Errors: `Unexpected EditableMesh id type. Expected Vertex id, received Face
  id`, `… received value that doesn't appear to be an id`, `Invalid id` (stale
  generation), `AddFace called with 2 vertices, currently only supports 3 or 4
  vertices`, `Unable to cast double to Array` (AddFace with a tuple),
  `Missing bone Name/CFrame/Virtual value`, `Unexpected entry in table: X`
  (AddBone keys: Name, CFrame, Virtual, ParentId), `No bone named "x"`,
  `Number of bone weights provided does not match number of bone ids`,
  `Only virtual bones can be used in Facs Poses, unable to use bone "C", id N`,
  `Correctives must have 2 or 3 FACS controls`.
- Bone weights are stored as bytes (0.5 reads 0.50196). RemoveBone orphans
  children (parent 0). GetAdjacentFaces = faces sharing an edge in the
  opposite direction. FindClosestPointOnSurface returns face, point,
  barycentric, then the three vertex ids. MergeVertices(tol) maps later
  vertices within tol onto the first (greedy, creation order), rewrites
  faces, keeps the vertices, returns {merged → kept}. BatchRemove takes faces
  only. Enum.MeshAttribute: Vertex 0, Normal 1, Color 2, UV 3, Face 4.

## Pathfinding (measured)

- `CreatePath()` → Path named "Instance", unparented, Status NoPath. Agent
  params cast errors as Studio (`Unable to cast string to float`); unknown
  keys are ignored.
- Floor 100×100, wall 2×10×30 at origin, -20 → 20: Success, 16 waypoints on
  the floor (y 0), first/last are the request's x,z, every 4 studs along
  each straight run (spacing restarts at each corner), around the wall's end,
  Label = surface material ("Plastic").
- Wall across the floor: ≤1 stud walks over; 1.5–6 studs: Walk to 2 studs
  before (-3, 0, .5), then a Jump waypoint (label "Jump") on the far side;
  7+ studs: NoPath. AgentCanJump=false: NoPath.
- Requests longer than 3000 studs error `PathfindingService: path request is
  too long.` (2999 OK, 3001 not).
- CheckOcclusionAsync(start): checks waypoints after `start`; returns the
  first obstructed one or -1; `Start point value is invalid` outside
  1..#waypoints. A 0.5-stud slab across the path isn't an obstruction.
- Studio rebuilds its navmesh asynchronously (a path computed right after
  creating parts in the same script can miss them; a later request sees
  them). rustudio rasterises the current world per request.
- Blocked/Unblocked: Studio fired Unblocked(8) after removing a blocker; its
  Blocked fired later than our 1 s window. rustudio re-checks computed paths
  every 0.5 s when the world changed and fires Blocked(i) / Unblocked(i) with
  the first obstructed waypoint.

Solver: per-request heightfield (1-stud cells, coarser for long requests),
spans from collidable parts (box, wedge, corner wedge, ball, cylinder;
MeshParts / unions as their box) and terrain voxels (water walkable,
labelled Water); surfaces with AgentHeight headroom; erosion by AgentRadius;
8-way A* with Costs (material, PathfindingModifier label, link label;
math.huge closes), jump links over ≤6.6-stud obstacles, PathfindingLinks
(Action Custom, link label), string pulling, WaypointSpacing subdivision.
Waypoint positions near walls differ from Studio's Recast mesh by a stud or
so (Studio's corner around the wall end is at x ±1.49, z 16.4).

## CSG (measured)

- `a:UnionAsync({b})` → unparented UnionOperation "Union": main part's
  properties, UsePartColor false, CollisionFidelity Default, RenderFidelity
  Automatic, Position = centre of the result's box, Size = that box.
  SubtractAsync → UnionOperation "Union"; IntersectAsync → IntersectOperation
  "Intersection". Empty intersection → nil. Union with {} → the part as a
  union. Non-parts: `CSG: One of the primitives was of unknown type (error
  -4)`. Studio prints `Reporting success or error SUCCESS for operation …`
  (a Studio log line; not reproduced).
- GeometryService:UnionAsync(part, parts, options) → array, SplitApart by
  default (2 pieces for a far part, 1 with SplitApart=false), {} for an empty
  intersection; CSGOptions keys (Studio's): CollisionFidelity,
  RenderFidelity, FluidFidelity, SplitApart, UseOffCenterCoordinates,
  RebuildHistory, UseDynamicConvexDecomposition (warns deprecated); others
  `Key 'Foo' is not a valid member of CSGOptions`.
- GeometryService results keep the main part's CFrame (the geometry is
  off-centre: Size 6,5,4 at Position 0,10,0, GetBoundingBox centre 1,10.5,0;
  PivotOffset identity; Size×2 moves the geometry centre to 3,11,0.268 with
  the main part turned 30°). Both paths keep the main part's rotation
  (BasePart:UnionAsync centres on the geometry: 1.5,10.5,0.134).
  `UseOffCenterCoordinates` is not a script option (Studio rejects it).
  rustudio: `mesh_bvh::union_offset` (the mesh's centre / extent; zero for
  Studio's own unions, which the survey of Place D.rbxl and the Place B archives
  found always centred with extent = InitialSize) shifts the drawn mesh
  (scene.rs), the collider (physics.rs, kept on teleports) and the query
  box (runtime_ext.rs: rays, GetPartsInPart, GetBoundingBox). Studio
  ray-tests unions by their geometry (12.00 at x -1.9, not the box top 13),
  so unions now ray-test their mesh like MeshParts.
- CSG output is closed (T-junctions from BSP splits are stitched): before,
  the convex decomposition of a union leaked and parts fell through it.
- Results are a CSGMDL v5 blob in MeshData2 (the format mesh.rs already
  parses), so they draw, collide by CollisionFidelity and ray-test like
  loaded unions; per-corner colours keep the source parts' colours.
- Triangle counts differ (BSP splits; Studio's union of two cubes is 36).
- CreateBasicMeshPart: Studio errors (`This API is not yet released!` in
  game scripts; the command bar says it lacks InternalTest).

## Other calls

- DrawImageProjected / SampleImageProjected: params ProjectionParams
  (Direction, Position, Size, Up) and BrushConfig (Decal, ColorBlendType,
  AlphaBlendType, BlendIntensity, FadeAngle); both report missing keys as
  `EditableImage::DrawImageProjected Param ProjectionParams, is missing X`;
  size/vector checks from Studio's messages. A 1×1 box over a 2×2 quad
  mapped to an 8×8 image paints its middle 4×4 (as Studio).
- MeshPart:ApplyMesh keeps Size, errors `ApplyMesh requires an MeshPart
  object`. MeshPart:GetSurfaceSlotsAsync errors `MeshPart:GetSurfaceSlotsAsync
  is not yet enabled` (Studio, this build).
- PartOperation:SubstituteGeometry: takes the source's geometry and Size
  (a plain Part source works too) and keeps its CFrame.
- GeometryService:CreateBasicMeshPart errors (not released).
- CalculateConstraintsToPreserve rows: {Attachment, AttachmentParent},
  {Constraint, ConstraintParent}, {WeldConstraint, WeldConstraintParent,
  WeldConstraintPart0/1}, {NoCollisionConstraint, NoCollisionParent,
  NoCollisionPart0/1}; options tolerance, weldConstraintPreserve,
  dropAttachmentsWithoutConstraints (Studio's key names).

## Coverage

Before (coverage-start): 78 functions + 2 properties stubbed across
PathfindingService 4, Path 4+Status, GeometryService 9, PartOperation 1,
EditableMesh 53+FixedSize, EditableImage 2, MeshPart 2, BasePart CSG 3.
After: 0 missing for all of them; Path.Blocked / Unblocked exist and fire.

## Round 2 (measured)

- AgentCanClimb: tower 20 high, truss on its side: Walk to 2 studs before
  the truss (11,0), a Custom waypoint labelled "Climb" at the truss top
  (13.5,20), then walking on; NoPath without AgentCanClimb or with
  Costs.Climb = math.huge. rustudio: (9.5,0) then (13.5,20) Climb.
- Drops: coming down a ledge Studio walks to its edge and puts a Jump
  waypoint (label "Jump") at the landing; drops work to 100 studs (110:
  NoPath), need AgentCanJump and a finite Costs.Jump.
- MeshParts / unions in paths: a 100-stud wall with a doorway cut by
  SubtractAsync is walked through with every CollisionFidelity, Box and Hull
  included: Studio's navmesh follows the part's own geometry. rustudio
  rasterises the mesh's triangles (entry / exit crossings per column).
  Before: the wall's box blocked (NoPath).
- GenerateFragmentSites: random (two calls differ), 2 per axis by default
  (8 for any size), ceil(size / SiteSpacing) otherwise; with Origin+Radius
  a grid of 343 (SiteSpacing 0.5) / 125 (SiteSpacing 1) whatever the part
  or radius, the sites outside the sphere grouped in one array first;
  unknown keys `Key 'Foo' is not a valid option.`
- FragmentAsync: an array of {Index, Instance}, Instance an unparented
  MeshPart with the source's CFrame (0,10,0) and properties, Size the
  fragment's extents (2,4,4 for each half of a cube cut by two sites), its
  geometry off-centre (handled like GeometryService unions).
- SweepPartAsync: a MeshPart at the part's CFrame, Size its extents in the
  part's frame (12,2,2; 2,2,12 with the part turned 90°), the geometry at
  the given CFrames in world space (bbox centre 5,0,0). An empty or
  non-CFrame list crashes Studio (two instances died); rustudio raises the
  messages in Studio's program instead.
- BakeToMeshAsync: `BakeToMeshAsync is not a valid member of
  GeometryService "GeometryService"` in this Studio build (feature off);
  rustudio raises the same on call (the member still indexes, as the API
  dump lists it).
- CalculateConstraintsToPreserve: rows measured with welds, a
  NoCollisionConstraint, a rope and a lone attachment against a split
  into two halves (geometry_tests.rs `constraints_to_preserve_as_studio`
  reproduces every row and its order).

## Left, and why

- Path shapes match Studio's in structure (spacing, corners, jump and climb
  waypoints, labels, statuses, limits) but not to the stud: Studio's
  Recast navmesh places corners differently (rustudio uses 1-stud cells).
- Blocked fires when rustudio's 0.5 s re-check sees the obstruction; Studio
  fires on its own navmesh rebuild schedule (it fired later than 1 s).
- CSG triangle counts differ (BSP splits vs Studio's simplified mesh); CSG on
  a MeshPart needs its mesh loaded (else `… Missing child data. (error -6)`).
- GenerateFragmentSites' points are random in Studio; rustudio matches the
  counts and grouping, not the distribution (unmeasurable).
- Default-fidelity colliders are V-HACD decompositions (collision.rs), which
  round a union's outer corners slightly more than Studio's.
