local a=Instance.new('Actor'); a.Name='DatatypeActor'; a.Parent=workspace
local s=Instance.new('Script'); s.Source=[==[
local b=Instance.new('BindableEvent'); local hits=0
local c=b.Event:ConnectParallel(function(x) task.synchronize(); hits+=1; print('DT|server|signal.Actor.callback|'..tostring(x)) end)
b:Fire(42); task.wait(); print('DT|server|signal.Actor.result|'..tostring(c.Connected)..'|'..hits); c:Disconnect()
for _,v in ipairs({1,{},true}) do local ok,e=pcall(function() return b.Event:ConnectParallel(v) end); print('DT|server|signal.Actor.bad.'..type(v)..'|'..tostring(ok)..'|'..tostring(e)) end
]==]; s.Parent=a; s.Disabled=true; s.Disabled=false
