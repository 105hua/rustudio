local side=game:GetService('RunService'):IsServer() and 'server' or 'client'
local function emit(k,...) local a=table.pack(...);local s={};for i=1,a.n do s[i]=tostring(a[i]):gsub('\n','\\n') end;print('GLOBALS|'..side..'|'..k..'|'..table.concat(s,' | ')) end
local function p(k,f) emit(k,pcall(f)) end
p('setfenv.native',function()return setfenv(math.abs,{})end)
for _,n in {'getinfo','getlocal','setlocal','getupvalue','setupvalue','getregistry','getmetatable','setmetatable'} do emit('debug.absence.'..n,type(debug[n])) end
p('safeenv.import',function()local function f()return math.abs(-4)end;local e=getfenv(f);local prev=e.math;e.math={abs=function()return 99 end};local r=f();e.math=prev;return r end)
p('coroutine.main.wrap',function() local co=coroutine.create(function()return coroutine.running(),coroutine.isyieldable()end);return coroutine.resume(co)end)
p('error.level2.lua',function()local function inner()error('two',2)end;local function outer()local r=inner();return r end;return outer()end)
p('error.nil',function()error(nil)end)
p('error.table.identity',function()local t={};local ok,e=pcall(error,t);return ok,e==t,type(e)end)
p('newproxy.metatable',function()local a,b=newproxy(),newproxy(true);return getmetatable(a),type(getmetatable(b)),pcall(function()return newproxy(b)end)end)
p('shared.isolation',function() _G.__globalsMarker=42;return getfenv().__globalsMarker,shared.__globalsMarker end)
p('require.cycle',function()return require(script.Cycle)end)
p('require.noresult',function()return require(script.NoResult)end)
p('require.tworesults',function()return require(script.TwoResults)end)
p('require.module.context',function()return require(script.Context)end)
p('require.nil',function()return require(nil)end)
p('gc.types',function()return type(gcinfo()),type(collectgarbage('count'))end)
for _,v in {false,23} do p('gc.arg.'..type(v),function()return collectgarbage(v)end)end
p('clock.delta',function()local a,b,c,d=os.clock(),elapsedTime(),time(),tick();task.wait(.1);return os.clock()-a,elapsedTime()-b,time()-c,tick()-d end)
emit('EXTRA_DONE')
