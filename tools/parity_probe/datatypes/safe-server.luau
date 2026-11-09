local s=Instance.new("Script"); s.Name="DatatypeProbe"; s.Source=[====[local side=game:GetService("RunService"):IsServer() and "server" or "client"
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
local ops={
 read=function(o) return o.____________ end,write=function(o) o.____________=1 end,writeX=function(o) o.X=1 end,
 namecall=function(o) return o:____________() end,
 add=function(o) return o+1 end,mul=function(o) return o*Color3.new() end,
 lt=function(o) return o<o end,le=function(o) return o<=o end,
 len=function(o) return #o end,concat=function(o) return o.."x" end,unm=function(o) return -o end,
 idiv=function(o) return o//2 end,mod=function(o) return o%2 end,
}
for _,entry in ipairs(factories) do
 local n,make=entry[1],entry[2]; local ok,o=pcall(make)
 test(n..'.create',function() if not ok then error(o,0) end; return type(o),typeof(o),tostring(o),getmetatable(o) end)
 if ok and o~=nil then
 local _,other=pcall(make)
 test(n..'.values',function() local t={[o]=1}; return o==other,rawequal(o,other),t[other],o==Color3.new(),rawequal(o,o) end)
 for k,op in pairs(ops) do
 test(n..'.'..k,function() return op(o) end)
 local f=capture(function() return op(o) end)
 if f then
 info(n..'.'..k..'.info',f)
 local g=capture(function() return op(other) end)
 out(n..'.'..k..'.identity',f==g)
 end
 end
 for _,method in ipairs({'Dot','Lerp','Inverse','ToHSV','GetComponents','NextNumber','ToIsoDate','GetEnumItems','Connect','Once','Wait','ConnectParallel','Disconnect'}) do
 local yes,f=pcall(function() return o[method] end)
 if yes and type(f)=='function' then
 info(n..'.method.'..method..'.info',f)
 test(n..'.method.'..method..'.identity',function() return f==o[method],f==other[method] end)
 test(n..'.method.'..method..'.dot',function() return f() end)
 test(n..'.method.'..method..'.badself',function() return f({}) end)
 end
 end
 end
 local ctor=getfenv()[n]
 test(n..'.constructor',function() return type(ctor),typeof(ctor),getmetatable(ctor),table.isfrozen(ctor) end)
 test(n..'.constructor.write',function() ctor.foo=1; return ctor.foo end)
 test(n..'.constructor.bad',function() return ctor.new('a') end)
end
for _,c in ipairs({{'Vector3.string',function() return Vector3.new('a') end},{'CFrame.two',function() return CFrame.new(1,2) end},{'Color3.nil',function() return Color3.fromRGB(nil) end},{'UDim2.table',function() return UDim2.new({}) end}}) do test(c[1],c[2]) end
for _,m in ipairs({'Connect','Once','ConnectParallel'}) do
 for _,v in ipairs({1,{},true}) do test('signal.'..m..'.'..type(v),function() return part.Changed[m](part.Changed,v) end) end
 test('signal.'..m..'.nil',function() return part.Changed[m](part.Changed,nil) end)
end
local b=Instance.new('BindableEvent'); local signal=b.Event
local hits=0; local once=signal:Once(function() hits+=1 end)
b:Fire(1); b:Fire(2); task.wait(); test('signal.Once.behavior',function() return hits,once.Connected end)
local conn=signal:Connect(function() end); out('connection.before',conn.Connected); test('connection.disconnect',function() conn:Disconnect(); return conn.Connected end); test('connection.disconnect.twice',function() conn:Disconnect(); return conn.Connected end)
local waiting=false; task.spawn(function() local a,c=signal:Wait(); out('signal.Wait.result',a,c); waiting=true end); task.wait(); b:Fire('hello',42); task.wait(); out('signal.Wait.resumed',waiting)
test('signal.ConnectParallel.valid',function() local c=signal:ConnectParallel(function() end); c:Disconnect(); return c.Connected end)
out('DONE')]====]; s.Parent=game:GetService("ServerScriptService"); s.Disabled=true; s.Disabled=false