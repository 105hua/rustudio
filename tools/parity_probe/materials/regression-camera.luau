-- In-memory material swatches, left to right: Slate, Rock, Cardboard, Leather.
local mats={Enum.Material.Slate,Enum.Material.Rock,Enum.Material.Cardboard,Enum.Material.Leather}
for i,m in mats do
 local p=Instance.new('Part');p.Name='Regression_'..m.Name;p.Anchored=true;p.Size=Vector3.new(4,6,1);p.Position=Vector3.new(1000+(i-2.5)*4.5,180,300);p.Color=Color3.fromRGB(160,140,110);p.Material=m;p.Parent=workspace
end
local cf=CFrame.lookAt(Vector3.new(1000,181,316),Vector3.new(1000,180,300))
game:GetService('RunService'):BindToRenderStep('MaterialParityCamera',Enum.RenderPriority.Camera.Value+10,function()
 workspace.CurrentCamera.CameraType=Enum.CameraType.Scriptable;workspace.CurrentCamera.FieldOfView=70;workspace.CurrentCamera.CFrame=cf
end)
