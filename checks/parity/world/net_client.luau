-- Network ownership probe (client): angular velocity and awake parts as the
-- client reads them.
task.spawn(function()
	local spin = workspace:WaitForChild("Own"):WaitForChild("Spin")
	task.wait(0.5)
	print(string.format("NET C spin=%s awake=%d vel@=%s", tostring(spin.AssemblyAngularVelocity), workspace:GetNumAwakeParts(), tostring(spin:GetVelocityAtPosition(spin.Position + Vector3.new(1, 0, 0)))))
end)
