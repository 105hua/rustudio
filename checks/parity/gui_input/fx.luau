-- ParticleEmitter2D / UIBlur picture (checks/parity/gui_input.md).
task.spawn(function()
local pg = game.Players.LocalPlayer:WaitForChild("PlayerGui")
local sg = Instance.new("ScreenGui") sg.Name = "FX" sg.Parent = pg
for i = 0, 9 do
  local f = Instance.new("Frame") f.BorderSizePixel = 0 f.Size = UDim2.fromOffset(20, 300) f.Position = UDim2.fromOffset(20 + i*40, 20)
  f.BackgroundColor3 = (i % 2 == 0) and Color3.new(1,0,0) or Color3.new(0,0,1) f.Parent = sg
end
local bf = Instance.new("Frame") bf.Name = "BlurBox" bf.Size = UDim2.fromOffset(200, 150) bf.Position = UDim2.fromOffset(120, 80) bf.BackgroundTransparency = 1 bf.Parent = sg
Instance.new("UIBlur").Parent = bf
local bf2 = Instance.new("Frame") bf2.Size = UDim2.fromOffset(220, 160) bf2.Position = UDim2.fromOffset(40, 380) bf2.BackgroundTransparency = 0.6 bf2.Parent = sg
local b2 = Instance.new("UIBlur") b2.Strength = Enum.UIBlurStrength.Light b2.Parent = bf2
local host = Instance.new("Frame") host.Name = "PHost" host.Size = UDim2.fromOffset(100, 100) host.Position = UDim2.fromOffset(550, 150) host.BackgroundColor3 = Color3.new(0.2,0.2,0.2) host.Parent = sg
local pe = Instance.new("ParticleEmitter2D") pe.Parent = host
local host2 = Instance.new("Frame") host2.Size = UDim2.fromOffset(60, 60) host2.Position = UDim2.fromOffset(700, 400) host2.BackgroundTransparency = 1 host2.Parent = sg
local pe2 = Instance.new("ParticleEmitter2D") pe2.EmissionMode = Enum.ParticleEmitter2DEmissionMode.Point pe2.Rate = 0
pe2.Color = ColorSequence.new(Color3.new(1, 0.6, 0), Color3.new(1, 0, 0)) pe2.SpreadAngle = 360 pe2.Speed = NumberRange.new(80, 160)
pe2.Size = NumberSequence.new(24, 4) pe2.Transparency = NumberSequence.new(0, 1) pe2.Acceleration = Vector2.new(0, 120) pe2.Lifetime = NumberRange.new(1.5, 2)
pe2.Parent = host2
task.wait(1) pe2:Emit(60)
print("FX ready")
end)
