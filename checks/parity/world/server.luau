-- World domain probe (server, Run/Play). Expected values were measured in
-- Studio 0.705 in an offline Studio (checks/parity/world.md). Prints
-- "WORLD PASS name" / "WORLD FAIL name: got … want …" and a summary.
-- (the harness runs this on the main thread: the body runs in a task so it can wait)
task.spawn(function()
local pass, fail = 0, 0
local function near(a, b, eps)
	eps = eps or 1e-3
	if typeof(a) == "Vector3" then return (a - b).Magnitude <= eps * math.max(1, b.Magnitude) end
	if type(a) == "number" then return math.abs(a - b) <= eps * math.max(1, math.abs(b)) end
	return a == b
end
local function check(name, got, want, eps)
	if near(got, want, eps) then
		pass += 1
	else
		fail += 1
		print(("WORLD FAIL %s: got %s want %s"):format(name, tostring(got), tostring(want)))
	end
end
local function err(f)
	local ok, e = pcall(f)
	if ok then return nil end
	return tostring(e):gsub("^.-:%d+: ", "")
end

-- Mass and inertia -------------------------------------------------------
local p = Instance.new("Part") p.Size = Vector3.new(4, 2, 6) p.Anchored = true p.Position = Vector3.new(0, 300, 0) p.Parent = workspace
check("mass", p:GetMass(), 33.6)
check("cpp density", p.CurrentPhysicalProperties.Density, 0.7)
check("specific gravity", p.SpecificGravity, 0.7)
check("resize inc", p.ResizeIncrement, 1)
check("faces", tostring(p.ResizeableFaces), "Right, Top, Back, Left, Bottom, Front")
check("receive age", p.ReceiveAge, 0)
check("grounded anchored", p:IsGrounded(), true)
check("A2T grounded", p:AngularAccelerationToTorque(Vector3.new(1, 0, 0)).X == math.huge, true)
check("netauto anchored err", err(function() return p:GetNetworkOwnershipAuto() end),
	"Network Ownership API cannot be called on Anchored parts or parts welded to Anchored parts.")
local ok, msg = p:CanSetNetworkOwnership()
check("canset anchored", ok, false)
p.Anchored = false
check("grounded", p:IsGrounded(), false)
check("A2T", p:AngularAccelerationToTorque(Vector3.new(1, 2, 3)), Vector3.new(112, 291.2, 168))
check("A2Tw", p:AngularAccelerationToTorque(Vector3.new(1, 2, 3), Vector3.new(1, 2, 3)), Vector3.new(-425.6, 459.2, 235.2))
check("T2Aw", p:TorqueToAngularAcceleration(Vector3.new(1, 2, 3), Vector3.new(1, 2, 3)), Vector3.new(4.808929, -1.140110, -1.146429))
p.Material = Enum.Material.Wood
check("wood density", p.CurrentPhysicalProperties.Density, 0.35)
p.Material = Enum.Material.Plastic
check("canset", (p:CanSetNetworkOwnership()), true)
check("netauto", p:GetNetworkOwnershipAuto(), true)
p:SetNetworkOwner(nil)
check("netauto after set", p:GetNetworkOwnershipAuto(), false)
check("owner", p:GetNetworkOwner(), nil)
check("setowner str", err(function() p:SetNetworkOwner("x") end), "Unable to cast value to Object")
p:SetNetworkOwnershipAuto()
check("netauto again", p:GetNetworkOwnershipAuto(), true)
local loose = Instance.new("Part")
check("ws err", err(function() loose:SetNetworkOwner(nil) end), "Can only call Network Ownership API on a part that is descendent of Workspace")

local function mk(cls, f)
	local q = Instance.new(cls) if f then f(q) end q.Size = Vector3.new(2, 4, 6) q.Anchored = true q.Parent = workspace
	local m, c = q:GetMass(), q.CenterOfMass - q.Position
	q:Destroy()
	return m, c
end
check("ball mass", (mk("Part", function(q) q.Shape = Enum.PartType.Ball end)), 2.9321532)
check("cyl mass", (mk("Part", function(q) q.Shape = Enum.PartType.Cylinder end)), 17.592918)
local wm, wc = mk("WedgePart")
check("wedge mass", wm, 16.8) check("wedge com", wc, Vector3.new(0, -0.6666667, 1))
local cm, cc = mk("CornerWedgePart")
check("cwedge mass", cm, 11.2) check("cwedge com", cc, Vector3.new(0.25, -1, -0.75))
check("cpp mass", (mk("Part", function(q) q.CustomPhysicalProperties = PhysicalProperties.new(3, 0.1, 0.2) end)), 144)

-- Assemblies --------------------------------------------------------------
local function part(n, sz, pos) local q = Instance.new("Part") q.Name = n q.Size = sz q.Position = pos q.Anchored = true q.Parent = workspace return q end
local A = part("A", Vector3.new(1, 1, 1), Vector3.new(0, 400, 0))
local B = part("B", Vector3.new(4, 1, 1), Vector3.new(3, 400, 0))
local C = part("C", Vector3.new(2, 2, 2), Vector3.new(6, 400, 0))
local D = part("D", Vector3.new(1, 1, 1), Vector3.new(9, 400, 0))
local w1 = Instance.new("WeldConstraint") w1.Part0 = A w1.Part1 = B w1.Parent = A
local w2 = Instance.new("Weld") w2.Part0 = B w2.Part1 = C w2.C0 = CFrame.new(3, 0, 0) w2.Parent = B
local a0 = Instance.new("Attachment", C) local a1 = Instance.new("Attachment", D)
local h = Instance.new("HingeConstraint") h.Attachment0 = a0 h.Attachment1 = a1 h.Parent = C
local function names(t) local s = {} for _, q in t do s[#s + 1] = q.Name end table.sort(s) return table.concat(s, ",") end
check("conn A", names(A:GetConnectedParts()), "A,B")
check("conn A rec", names(A:GetConnectedParts(true)), "A,B,C")
check("conn D", names(D:GetConnectedParts(true)), "D")
check("joints C", #C:GetJoints(), 2)
check("roots anchored", B.AssemblyRootPart, B)
for _, q in { A, B, C, D } do q.Anchored = false end
check("root C", A.AssemblyRootPart, C)
check("assembly mass", A.AssemblyMass, 9.1)
A.RootPriority = 1 check("root prio", B.AssemblyRootPart, A) A.RootPriority = 0
B.Massless = true check("massless", B:GetMass(), 0) check("massless asm", A.AssemblyMass, 6.3) B.Massless = false
C.Anchored = true check("anchored C root", A.AssemblyRootPart, C) check("grounded A", A:IsGrounded(), true) check("D free", D:IsGrounded(), false)
for _, q in { A, B, C, D } do q:Destroy() end

-- WorldRoot ---------------------------------------------------------------
local P = part("P", Vector3.new(4, 2, 6), Vector3.new(0, 500, 0))
local Q = part("Q", Vector3.new(4, 2, 6), Vector3.new(0, 501, 0))
check("touching", workspace:ArePartsTouchingOthers({ P }), true)
check("touching both", workspace:ArePartsTouchingOthers({ P, Q }), false)
check("touching err", err(function() workspace:ArePartsTouchingOthers({ workspace }) end),
	"All objects passed to ArePartsTouchingOthers must be BaseParts, got a Workspace.")
local R3 = Region3.new(Vector3.new(-1, 499, -1), Vector3.new(1, 501, 1))
check("r3 empty", workspace:IsRegion3Empty(R3), false)
check("r3 ignore", workspace:IsRegion3EmptyWithIgnoreList(R3, { P, Q }), true)
P:Destroy() Q:Destroy()
check("throttle", workspace:GetPhysicsThrottling(), 100)
check("pgs", workspace:PGSIsEnabled(), true)
check("fe", workspace.FilteringEnabled, true)

-- Model -------------------------------------------------------------------
local m = Instance.new("Model")
local mp = Instance.new("Part") mp.Anchored = true mp.Size = Vector3.new(2, 2, 2) mp.CFrame = CFrame.new(1, 2, 3) * CFrame.Angles(0, 0.5, 0) mp.Parent = m
local mq = Instance.new("Part") mq.Anchored = true mq.Size = Vector3.new(2, 4, 2) mq.CFrame = CFrame.new(5, 2, 3) mq.Parent = m
m.Parent = workspace
m.WorldPivot = CFrame.new(3, 2, 3) * CFrame.Angles(0.3, 0.2, 0.1)
m:ScaleTo(2)
check("scale", m:GetScale(), 2)
check("scaled size", mp.Size, Vector3.new(4, 4, 4))
check("scaled pos", mp.Position, Vector3.new(-1, 2, 3))
check("scaled q", mq.Position, Vector3.new(7, 2, 3))
check("scale0", err(function() m:ScaleTo(0) end), "Only positive non-zero scale values are allowed.")
check("persist", err(function() m:AddPersistentPlayer() end), "PersistentPerPlayer player not specified.")
m:Destroy()

-- Camera ------------------------------------------------------------------
local cam = Instance.new("Camera")
cam.FieldOfView = 70
check("diag 1x1", cam.DiagonalFieldOfView, 88.876534)
check("max 1x1", cam.MaxAxisFieldOfView, 70)
cam.FieldOfView = 10 check("diag 10", cam.DiagonalFieldOfView, 1.7540916)
cam.DiagonalFieldOfView = 100 check("set diag", cam.FieldOfView, 75.331291)
check("proj", cam.ProjectionType, Enum.ProjectionType.Perspective)
check("ortho", cam.OrthographicSize, 300)
cam.Parent = workspace cam.Focus = CFrame.new()
cam.CFrame = CFrame.new(Vector3.new(0, 0, 20)) * CFrame.Angles(-0.2, 0, 0)
cam:TiltUnits(1)
check("tilt", math.deg(math.asin(cam.CFrame.LookVector.Y)), 0, 1e-2)
cam:TiltUnits(1)
check("tilt2", math.deg(math.asin(cam.CFrame.LookVector.Y)), 10, 1e-2)
cam.CFrame = CFrame.new(Vector3.new(3, 0, 20))
cam:PanUnits(2)
check("pan pos", cam.CFrame.Position, Vector3.new(-20.2237, 0, 0), 1e-2)
check("pan look", cam.CFrame.LookVector, Vector3.new(1, 0, 0), 1e-2)
check("interp err", err(function() cam:Interpolate(CFrame.new(), CFrame.new(), 1) end),
	"Attempted to use interpolation with a camera mode other than scriptable.")
cam.CameraType = Enum.CameraType.Scriptable
check("interp time", err(function() cam:Interpolate(CFrame.new(), CFrame.new(), 0) end), "Interpolation time must be positive.")
local fired = 0
cam.InterpolationFinished:Connect(function() fired += 1 end)
cam:Interpolate(CFrame.new(10, 10, 0), CFrame.new(1, 2, 3), 0.3)

-- Constraints (attachment frames from the Studio session) --------------------
local CA = part("CA", Vector3.new(4, 1, 2), Vector3.zero) CA.CFrame = CFrame.new(0, 600, 50) * CFrame.Angles(0.2, 0.4, 0.1)
local CB = part("CB", Vector3.new(4, 1, 2), Vector3.zero) CB.CFrame = CFrame.new(3, 601, 50) * CFrame.Angles(0.5, 0, 0.3)
local c0 = Instance.new("Attachment", CA) local c1 = Instance.new("Attachment", CB)
local function con(cls) local c = Instance.new(cls) c.Attachment0 = c0 c.Attachment1 = c1 c.Parent = workspace return c end
check("hinge angle", con("HingeConstraint").CurrentAngle, 8.709246)
local cyl = con("CylindricalConstraint")
for _, t in { { 0, 8.709246 }, { 90, 26.628349 }, { 45, 9.421770 }, { -30, 12.121257 }, { 180, 171.290741 } } do
	cyl.InclinationAngle = t[1]
	check("cyl angle " .. t[1], cyl.CurrentAngle, t[2])
end
cyl.InclinationAngle = 45 check("cyl axis", cyl.WorldRotationAxis, Vector3.new(0.583014, 0.807706, -0.087778))
check("torsion", con("TorsionSpringConstraint").CurrentAngle, 18.869450)
check("spring", con("SpringConstraint").CurrentLength, 3.162278)
check("prism", con("PrismaticConstraint").CurrentPosition, 2.924201)
local hc = con("HingeConstraint")
check("active now", hc.Active, false)
check("dbg", err(function() hc:GetDebugAppliedForce(0) end), "Querying applied forces is not currently supported.")
check("getconstraints", #c0:GetConstraints() >= 6, true)
local at = Instance.new("Attachment")
at:SetAxis(Vector3.new(0, 1, 0))
check("setaxis", at.CFrame.RightVector, Vector3.new(0, 1, 0))
check("setaxis up", at.CFrame.UpVector, Vector3.new(-1, 0, 0))
at:SetSecondaryAxis(Vector3.new(1, 0, 0))
check("setsec", at.CFrame.UpVector, Vector3.new(1, 0, 0))
check("setsec z", -at.CFrame.LookVector, Vector3.new(0, 0, -1))
local ao = Instance.new("AlignOrientation")
ao.PrimaryAxis = Vector3.new(0, 2, 0) check("ao prim", ao.PrimaryAxis, Vector3.new(0, 1, 0))
ao.LookAtPosition = Vector3.new(5, 0, 0) check("ao look", ao.CFrame.Position, Vector3.new(5, 0, 0))

-- Tool grips ----------------------------------------------------------------
local tl = Instance.new("Tool")
check("grip fwd", tl.GripForward, Vector3.new(0, 0, -1))
tl.Grip = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7) tl.GripForward = Vector3.new(1, 2, 3)
check("grip fwd set", tl.Grip.RightVector, Vector3.new(-0.295041, -0.746699, 0.596147))
tl.Grip = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7) tl.GripRight = Vector3.new(1, 2, 3)
check("grip right set", -tl.Grip.LookVector, Vector3.new(-0.295041, -0.746699, 0.596147))
tl.Grip = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7) tl.GripUp = Vector3.new(1, 2, 3)
check("grip up set", tl.Grip.RightVector, Vector3.new(0.616428, 0.544701, -0.568610))

-- Terrain cells -------------------------------------------------------------
local T = workspace.Terrain
T:SetCell(0, 300, 0, Enum.CellMaterial.Granite, Enum.CellBlock.Solid, Enum.CellOrientation.X)
check("cell", (T:GetCell(0, 300, 0)), Enum.CellMaterial.Granite)
local mats = T:ReadVoxels(Region3.new(Vector3.new(0, 1200, 0), Vector3.new(4, 1204, 4)), 4)
check("cell smooth", mats[1][1][1], Enum.Material.Slate)
T:SetWaterCell(1, 300, 0, Enum.WaterForce.Medium, Enum.WaterDirection.X)
check("water", (T:GetWaterCell(1, 300, 0)), true)
check("water cell", (T:GetCell(1, 300, 0)), Enum.CellMaterial.Water)
check("getcell err", err(function() T:GetCell() end), "Argument 3 missing or nil")
check("slot", (T:GetMaterialSlot(2)), Enum.Material.Grass)
check("slot oor", err(function() T:GetMaterialSlot(64) end), "slotIndex is out of range")
check("slot water", err(function() T:SetMaterialSlot(1, Enum.Material.Grass, "", Color3.new()) end), "Air and water slots cannot be set")
check("slot mat", err(function() T:SetMaterialSlot(3, Enum.Material.Plastic, "", Color3.new()) end), "baseMaterial is not a terrain material")

-- Sensors -------------------------------------------------------------------
local S = part("S", Vector3.new(4, 1, 2), Vector3.new(0, 51, 0))
local atm = Instance.new("AtmosphereSensor") atm.Parent = S
check("air density", atm.AirDensity, 0.001198168, 1e-4)
workspace.GlobalWind = Vector3.new(10, 0, 0)
check("wind", atm.RelativeWindVelocity, Vector3.new(16.185783, 0, 0), 1e-3)
workspace.GlobalWind = Vector3.zero
local ffs = Instance.new("FluidForceSensor") ffs.Parent = S
local f = ffs:EvaluateAsync(Vector3.new(0, 0, -50), Vector3.zero, CFrame.new(3, 500, 50))
check("fluid", f, Vector3.new(0, 0, 14.185914), 1e-3)

-- Movers & events (need frames) -----------------------------------------------
local floor = part("F", Vector3.new(20, 1, 20), Vector3.new(100, 10, 0))
local box = Instance.new("Part") box.Size = Vector3.new(2, 2, 2) box.Position = Vector3.new(100, 14, 0) box.Parent = workspace
local ev = {}
for _, n in { "Touched", "TouchEnded", "StoppedTouching", "LocalSimulationTouched" } do
	box[n]:Connect(function() ev[n] = (ev[n] or 0) + 1 end)
end
task.wait(1.5)
check("touched", (ev.Touched or 0) > 0, true)
check("lst", ev.LocalSimulationTouched == ev.Touched, true)
check("stopped", ev.StoppedTouching == ev.TouchEnded, true)
check("interp fired", fired, 1)
check("interp pos", cam.CFrame.Position, Vector3.new(10, 10, 0), 1e-2)
check("active later", hc.Active, false) -- both ends anchored
CB.Anchored = false task.wait(0.1)
check("active", hc.Active, true)
hc.Enabled = false check("active off", hc.Active, false)

box.CFrame = CFrame.new(100, 40, 0)
local bp = Instance.new("BodyPosition") bp.Position = Vector3.new(100, 50, 0) bp.Parent = box
local reached = 0 bp.ReachedTarget:Connect(function() reached += 1 end)
check("lastforce", bp:GetLastForce(), Vector3.zero)
task.wait(3)
check("bp reached", reached, 1)
bp:Destroy()
local tgt = part("T", Vector3.new(1, 1, 1), Vector3.new(130, 50, 0))
local rp = Instance.new("RocketPropulsion") rp.Target = tgt rp.Parent = box
local rr = 0 rp.ReachedTarget:Connect(function() rr += 1 end)
rp:Fire()
task.wait(3)
check("rocket reached", rr, 1)
check("rocket near", (box.Position - tgt.Position).Magnitude < 5, true)
rp:Abort()

local spin = Instance.new("Part") spin.Size = Vector3.new(2, 2, 2) spin.Position = Vector3.new(0, 800, 200) spin.Parent = workspace
spin:ApplyAngularImpulse(Vector3.new(0, 10, 0))
task.wait(0.1)
check("spin", spin.AssemblyAngularVelocity.Y > 1, true)
check("awake", workspace:GetNumAwakeParts() > 0, true)

-- Controllers ----------------------------------------------------------------
local cs = Instance.new("ControlState")
local csn = 0 cs.OnStateChanged:Connect(function() csn += 1 end)
cs:AddNumberField("thr", 0.5, 0, 1) cs:AddIntField("gear", 2, 0, 5) cs:AddVector3Field("dir", Vector3.new(1, 2, 3), 10) cs:AddBoolField("jump")
check("cs state", cs:GetState().thr, 0.5)
cs:SetField("thr", 2) cs:SetField("gear", 3.7) cs:SetField("dir", Vector3.new(100, 0, 0))
check("cs staged", cs:GetState().thr, 0.5)
check("cs type", err(function() cs:SetField("jump", 5) end), "ControlState: field 'jump' expects a boolean")
check("cs unknown", err(function() cs:SetField("nope", 1) end), "ControlState: unknown field 'nope'")
check("cs hasfield", err(function() cs:HasField("jump") end), "ControlState.HasField is not enabled")
check("cs weight", cs:GetReplicationWeight(), 64)
cs.Parent = workspace
local hc2 = Instance.new("HumanoidController")
check("btn unbound", err(function() hc2:GetButton(Enum.Button.Jump) end), "Button has not been bound. Call BindButton first.")
hc2:BindButton(Enum.Button.Jump, "Jump!")
check("btn bound", hc2:GetButton(Enum.Button.Jump), false)
check("beta", err(function() workspace.Terrain:ReadVoxelsAsync_beta(Region3.new(), 4, {}) end), "Beta is not enabled")
local dd = Instance.new("DragDetector")
check("dd ref", dd:GetReferenceFrame(), CFrame.new())
check("dd style nil", err(function() dd:SetDragStyleFunction(nil) end), "Argument 1 missing or nil")
check("dd conn", typeof(dd:AddConstraintFunction(1, function(c) return c end)), "RBXScriptConnection")
local DF = part("DF", Vector3.new(50, 2, 50), Vector3.new(500, 99, 0))
local DP = part("DP", Vector3.new(4, 2, 2), Vector3.new(500, 101, 0))
local DQ = part("DQ", Vector3.new(2, 2, 2), Vector3.new(504, 101, 0))
local drg = Instance.new("Dragger")
check("drag before", err(function() drg:MouseMove(Ray.new(Vector3.new(), Vector3.new(0, -1, 0))) end), "Call to LuaDragger::mouseMove without mouseDown")
drg:MouseDown(DP, Vector3.new(501, 102, 0), { DP, DQ })
drg:MouseMove(Ray.new(Vector3.new(520, 150, -10), Vector3.new(0, -100, 0)))
check("drag P", DP.Position, Vector3.new(520, 101, -10))
check("drag Q", DQ.Position, Vector3.new(524, 101, -10))
drg:AxisRotate(Enum.Axis.Y)
check("rot P", DP.Position, Vector3.new(521.5, 101, -8.5))
check("rot Q", DQ.Position, Vector3.new(521.5, 101, -12.5))
drg:MouseUp()
check("drag after", err(function() drg:MouseMove(Ray.new(Vector3.new(), Vector3.new(0, -1, 0))) end), "Call to LuaDragger::mouseMove after mouseUp")
local WA = part("WA", Vector3.new(1, 1, 1), Vector3.new(0, 700, 0)) local WB = part("WB", Vector3.new(1, 1, 1), Vector3.new(0, 702, 0))
local wc = Instance.new("WeldConstraint") wc.Part0 = WA wc.Part1 = WB wc.Parent = WA
WB.Anchored = false
local skb = Instance.new("SkateboardPlatform") skb.Position = Vector3.new(0, 900, 300) skb.Parent = workspace
skb:ApplySpecificImpulse(Vector3.new(0, 100, 0))
task.wait(0.1)
check("cs committed", cs:GetState().thr, 1)
check("cs gear", cs:GetState().gear, 4)
check("cs dir", cs:GetState().dir, Vector3.new(10, 0, 0))
check("cs event", csn, 1)
check("weld active", wc.Active, true)
check("skate impulse", skb.AssemblyLinearVelocity.Y > 50, true)

print(("WORLD SUMMARY pass=%d fail=%d"):format(pass, fail))
end)
