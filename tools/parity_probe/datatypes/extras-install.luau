for _,x in ipairs({{"Script",game:GetService("ServerScriptService")},{"LocalScript",game:GetService("Players").LocalPlayer.PlayerScripts}}) do local s=Instance.new(x[1]);s.Name="DatatypeExtras";s.Source=[====[local side=game:GetService("RunService"):IsServer() and "server" or "client"
local function out(k,...) local a={} for i=1,select('#',...) do a[i]=tostring(select(i,...)):gsub("\n","\\n") end print('DT|'..side..'|'..k..'|'..table.concat(a,'|')) end
local function test(k,f) local r=table.pack(pcall(f)); if not r[1] then out(k,false,type(r[2]),r[2]) else out(k,table.unpack(r,1,r.n)) end end
local function capture(f) local mm; xpcall(f,function() mm=debug.info(2,'f') end); return mm end
local function info(k,f) test(k,function() return debug.info(f,'slna') end) end
local part=Instance.new('Part'); part.Anchored=true; part.Position=Vector3.new(10000,10000,10000); part.Parent=workspace
local function makeResult() return workspace:Raycast(Vector3.new(10000,10010,10000),Vector3.new(0,-20,0)) end
local factories={
{"Vector3",function() return Vector3.new(1/3,-0,1e20) end},
{"Vector2",function() return Vector2.new(1/3,-0) end},
{"Vector3int16",function() return Vector3int16.new(1,2,3) end},
{"Vector2int16",function() return Vector2int16.new(1,2) end},
{"CFrame",function() return CFrame.new(1/3,2,3) end},
{"Color3",function() return Color3.new(1/3,0.5,1) end},
{"BrickColor",function() return BrickColor.new(21) end},
{"UDim",function() return UDim.new(1/3,7) end},
{"UDim2",function() return UDim2.new(1/3,7,0.5,-8) end},
{"Rect",function() return Rect.new(1,2,3,4) end},
{"Ray",function() return Ray.new(Vector3.new(1,2,3),Vector3.new(4,5,6)) end},
{"Region3",function() return Region3.new(Vector3.zero,Vector3.one) end},
{"Region3int16",function() return Region3int16.new(Vector3int16.new(),Vector3int16.new(1,2,3)) end},
{"NumberRange",function() return NumberRange.new(1/3,2) end},
{"NumberSequence",function() return NumberSequence.new(0.5) end},
{"NumberSequenceKeypoint",function() return NumberSequenceKeypoint.new(0,0.5,0.1) end},
{"ColorSequence",function() return ColorSequence.new(Color3.new(1,0,0)) end},
{"ColorSequenceKeypoint",function() return ColorSequenceKeypoint.new(0,Color3.new(1,0,0)) end},
{"TweenInfo",function() return TweenInfo.new(1/3) end},
{"PhysicalProperties",function() return PhysicalProperties.new(1,0.5,0.5) end},
{"Faces",function() return Faces.new(Enum.NormalId.Top) end},
{"Axes",function() return Axes.new(Enum.Axis.X) end},
{"Font",function() return Font.new("rbxasset://fonts/families/Arial.json") end},
{"DateTime",function() return DateTime.fromUnixTimestamp(0) end},
{"Random",function() return Random.new(42) end},
{"OverlapParams",function() return OverlapParams.new() end},
{"RaycastParams",function() return RaycastParams.new() end},
{"RaycastResult",function() return makeResult() end},
{"CatalogSearchParams",function() return CatalogSearchParams.new() end},
{"PathWaypoint",function() return PathWaypoint.new(Vector3.new(1,2,3)) end},
{"SharedTable",function() return SharedTable.new() end},
{"Content",function() return Content.fromUri("rbxassetid://123") end},
{"RBXScriptSignal",function() return part.Changed end},
{"RBXScriptConnection",function() return part.Changed:Connect(function() end) end},
}

local fields={Vector3='X',Vector2='X',Vector3int16='X',Vector2int16='X',CFrame='X',Color3='R',BrickColor='Name',UDim='Scale',UDim2='X',Rect='Min',Ray='Origin',Region3='Size',Region3int16='Min',NumberRange='Min',NumberSequence='Keypoints',NumberSequenceKeypoint='Time',ColorSequence='Keypoints',ColorSequenceKeypoint='Time',TweenInfo='Time',PhysicalProperties='Density',Faces='Top',Axes='X',Font='Family',DateTime='UnixTimestamp',OverlapParams='MaxParts',RaycastParams='IgnoreWater',RaycastResult='Distance',CatalogSearchParams='SearchKeyword',PathWaypoint='Position',Content='Uri',RBXScriptConnection='Connected'}
for _,entry in ipairs(factories) do
 local n,make=entry[1],entry[2];local ok,o=pcall(make)
 if ok and o then
 local k=fields[n]; if k then test(n..'.writeKnown',function() o[k]=o[k]; return o[k] end) end
 local f=capture(function() return o.____________ end); local g=capture(function() return o:____________() end)
 out(n..'.index.namecall.same',f==g)
 if f then test(n..'.index.direct.badkey',function() return f(o,{}) end);test(n..'.index.direct.badself',function() return f({},'X') end) end
 end
 local c=getfenv()[n]; if type(c)=='table' then
 test(n..'.constructor.shape',function() return type(c),typeof(c),getmetatable(c),table.isfrozen(c) end)
 for k,f in pairs(c) do if type(f)=='function' then info(n..'.constructor.'..k..'.info',f) end end
 end
end
local b=Instance.new('BindableEvent')
for _,method in ipairs({'Connect','Once'}) do
 test('signal.'..method..'.missing',function() return b.Event[method](b.Event) end)
end
out('EXTRA_DONE')
]====];s.Parent=x[2] end