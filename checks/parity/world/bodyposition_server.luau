-- BodyPosition response probe (server): Studio's four runs (world.md);
-- prints gap / velocity every 0.25 s; the rate v/d is the comparison.
local function run(size, P, D, label, mf)
	local B = Instance.new("Part") B.Size = Vector3.new(size, size, size) B.CFrame = CFrame.new(400 + size * 10, 30, P / 100) B.Parent = workspace
	local bp = Instance.new("BodyPosition") bp.P = P bp.D = D bp.MaxForce = mf or Vector3.new(1e9, 1e9, 1e9) bp.Position = B.Position + Vector3.new(0, 10, 0) bp.Parent = B
	local goal = bp.Position local out = {}
	for i = 1, 6 do task.wait(0.25) local d = (goal - B.Position).Y out[#out + 1] = string.format("%.4f/%.3f(%.2f)", d, B.AssemblyLinearVelocity.Y, B.AssemblyLinearVelocity.Y / math.max(d, 1e-6)) end
	print("BP", label, "m", B:GetMass(), table.concat(out, " "))
end
task.spawn(run, 2, 10000, 1250, "base")
task.spawn(run, 4, 10000, 1250, "heavy")
task.spawn(run, 2, 40000, 1250, "P4x")
task.spawn(run, 2, 10000, 5000, "D4x")
task.spawn(run, 2, 1000, 100, "lowD")
task.spawn(run, 4, 10000, 1250, "heavyDefaultMax", Vector3.new(4000, 4000, 4000))
