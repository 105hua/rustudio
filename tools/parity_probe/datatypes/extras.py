from pathlib import Path
p=Path(__file__).parent
s=(p/'probe.luau').read_text().split('local ops=')[0]
s+='''
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
'''
# Avoid captured direct calls in rustudio after the full corpus crashed.
(p/'extras.luau').write_text(s)
for side,parent in [('client','game:GetService("Players").LocalPlayer:WaitForChild("PlayerScripts")'),('server','game:GetService("ServerScriptService")')]:
 safe='\n'.join(l for l in s.splitlines() if '.index.direct.' not in l)
 (p/f'extras-{side}.luau').write_text('local s=Instance.new("'+('LocalScript' if side=='client' else 'Script')+'"); s.Name="DatatypeExtras"; s.Source=[====['+safe+']====]; s.Parent='+parent+'; s.Disabled=true; s.Disabled=false')
(p/'extras-install.luau').write_text('for _,x in ipairs({{"Script",game:GetService("ServerScriptService")},{"LocalScript",game:GetService("Players").LocalPlayer.PlayerScripts}}) do local s=Instance.new(x[1]);s.Name="DatatypeExtras";s.Source=[====['+s+']====];s.Parent=x[2] end')
