for i=1,2 do local s=Instance.new("LocalScript"); s.Name="GlobalsProbe"..i; s.Source=[====[local side=game:GetService('RunService'):IsServer() and 'server' or 'client'
local function emit(k,...)
 local a=table.pack(...); local s={}; for i=1,a.n do s[i]=tostring(a[i]):gsub('\n','\\n'):gsub('\r','\\r'):gsub('[^%g ]',function(c)return string.format('\\x%02x',string.byte(c))end) end
 print('GLOBALS|'..side..'|'..k..'|'..table.concat(s,' | '))
end
local function probe(k,f) emit(k,pcall(f)) end
local env=getfenv()
local function inv(t,path,depth)
 local keys={}; for k in pairs(t) do table.insert(keys,tostring(k)) end; table.sort(keys)
 for _,k in keys do local v=t[k]; if type(v)=='function' then probe('inventory.'..path..'.'..k,function() local a,b,c,d,e=debug.info(v,'slna'); return type(v),typeof(v),a,b,c,d,e,tostring(v) end) else emit('inventory.'..path..'.'..k,type(v),typeof(v),tostring(v)) end end
 local mt=getmetatable(t); emit('meta.'..path,type(mt),tostring(mt)); if type(mt)=='table' then for k,v in pairs(mt) do emit('metakey.'..path..'.'..tostring(k),type(v),tostring(v)) end; if type(mt.__index)=='table' and depth<8 then inv(mt.__index,path..'.index',depth+1) end end
end
inv(env,'env',0)
local candidates={"Axes","BrickColor","CFrame","CatalogSearchParams","Color3","ColorSequence","ColorSequenceKeypoint","Content","DateTime","DebuggerManager","Delay","ElapsedTime","Enum","Faces","FloatCurveKey","Font","Game","Instance","LoadLibrary","NumberRange","NumberSequence","NumberSequenceKeypoint","OverlapParams","Path2DControlPoint","PathWaypoint","PhysicalProperties","PluginManager","Random","Ray","RaycastParams","Rect","Region3","Region3int16","RotationCurveKey","SharedTable","Spawn","TweenInfo","UDim","UDim2","User","UserSettings","ValueCurveKey","Vector2","Vector2int16","Vector3","Vector3int16","Version","Workspace","_G","_VERSION","__ac_capture_begin","__ac_capture_known","__ac_capture_poll","__ac_capture_prompt","__ac_content_begin","__ac_content_ids","__ac_content_poll","__ac_create_decal","__ac_instance_composition","__ac_is_server","__ac_pages","__ac_scene_mem_begin","__ac_scene_mem_restore","__ac_script_memory","__ac_unparented","__ac_web_begin","__ac_web_poll","__geo_nav","__geo_ops","__rs_anim_after","__rs_anim_asset","__rs_anim_drive","__rs_anim_joint_spec","__rs_anim_joint_upgrade","__rs_anim_register","__rs_anim_registered","__rs_anim_rotation_list","__rs_anim_rotation_value","__rs_asset_begin","__rs_asset_ready","__rs_avatar_get","__rs_avatar_load","__rs_avatar_prompt_answer","__rs_cg","__rs_gated_events","__rs_get_callback","__rs_has_peer","__rs_input_event","__rs_input_frame_end","__rs_invoke_reply","__rs_invoke_server","__rs_kfs_parts","__rs_lua_methods","__rs_make_input_object","__rs_make_signal","__rs_member_check","__rs_new_input_object","__rs_new_signal","__rs_pages_list","__rs_pages_new","__rs_particles","__rs_pc_chat_input","__rs_pc_chat_view","__rs_pc_clear_view","__rs_pc_client_receive","__rs_pc_get","__rs_pc_outbox","__rs_pc_server_receive","__rs_pc_set","__rs_pc_step","__rs_pc_translator","__rs_product_info","__rs_profile","__rs_require_for","__rs_seat","__rs_serve_client_invoke","__rs_serve_invoke","__rs_set_raw","__rs_track_get","__rs_track_set","__rs_tracks","__rs_tween_get","__rs_unseat","__rs_user_game_settings","__rs_window_focus","__ws_begin","__ws_bundle_details","__ws_campaign_record","__ws_filter","__ws_filter_text","__ws_info_type","__ws_owned","__ws_page_copy","__ws_poll","__ws_send","__ws_subscription_override","addDialog","assert","bit32","buffer","checkForLeaveArea","collectgarbage","coroutine","createChatNotificationGui","createMessageDialog","currentTone","debug","delay","dialogIsValid","doDialog","elapsedTime","endDialog","error","game","gcinfo","getChatColor","getLocalHumanoidRootPart","getfenv","getmetatable","initialize","integer","ipairs","loadstring","math","newChoice","newproxy","next","normalEndDialog","onLoad","os","pairs","pcall","presentDialogChoices","print","printidentity","rawequal","rawget","rawlen","rawset","removeDialog","renewKillswitch","require","resetColor","sanitizeMessage","script","select","selectChoice","setChatNotificationTone","setUsingGamepad","setfenv","setmetatable","settings","shared","showMessage","spawn","startDialog","stats","string","styleChoices","styleMainFrame","table","task","tick","time","timeoutDialog","tonumber","tostring","type","typeof","unpack","utf8","variableDelay","vector","version","wait","wanderDialog","warn","workspace","xpcall","ypcall"}
local visible={}; for _,k in candidates do visible[k]=env[k] end; inv(visible,'visible',0)
for _,lib in {'coroutine','task','debug','os','math','string','table','utf8','buffer','bit32'} do if type(env[lib])=='table' then inv(env[lib],lib,0) end end
local old=shared.__globalsProbe
if old then for _,k in candidates do emit('identity.'..k,rawequal(old[k],env[k])) end else local snap={}; for _,k in candidates do snap[k]=env[k] end; shared.__globalsProbe=snap end
for k,v in pairs({func=function()end,tab={},thread=coroutine.create(function()end),proxy=newproxy(true),instance=game,buf=buffer.create(3)}) do emit('tostring.'..k,type(v),typeof(v),tostring(v),tostring(v),rawequal(v,v)) end
for _,op in {'count','collect','stop','restart','step','setpause','setstepmul','isrunning','bad'} do probe('gc.'..op,function() return collectgarbage(op) end) end
probe('gc.default',function() return collectgarbage() end)
probe('gc.info',function() return gcinfo(),collectgarbage('count'),gcinfo()==collectgarbage('count') end)
for _,level in {0,1,2,10,-1} do probe('error.level'..level,function() error('message',level) end) end
for _,v in {23,false,{},newproxy()} do probe('error.value.'..type(v),function() error(v) end) end
probe('assert.default',function() assert(false) end)
probe('assert.table',function() assert(false,{}) end)
probe('assert.returns',function() return assert(true,'a',4) end)
probe('xpcall.handler',function() return xpcall(function() error('inner') end,function() error('handler') end) end)
probe('trace.default',function() return debug.traceback() end)
probe('trace.message',function() return debug.traceback('message',0) end)
probe('trace.coroutine',function() local co=coroutine.create(function() coroutine.yield() end); coroutine.resume(co); return debug.traceback(co,'co',0) end)
for _,level in {0,1,2,50,-1} do probe('debug.level'..level,function() return debug.info(level,'slna') end) end
probe('debug.thread',function() return debug.info(coroutine.running(),1,'slna') end)
for _,level in {0,1,2,50,-1} do probe('getfenv.'..level,function() local e=getfenv(level); return type(e),e==env,e.script end) end
probe('getfenv.C',function() return getfenv(print)==env,getfenv(print)==getfenv(0) end)
probe('setfenv.C',function() return setfenv(math.abs,{}) end)
probe('setfenv.lua',function() local function f()return xyz end; setfenv(f,{xyz=42}); return f() end)
for _,level in {-1,50} do probe('setfenv.'..level,function() return setfenv(level,{}) end) end
probe('loadstring',function() local f,e=loadstring('return script,debug.info(1,"s")'); if f then return f() end return f,e end)
probe('loadstring.syntax',function() return loadstring('this is invalid') end)
probe('loadstring.named',function() local f,e=loadstring('return debug.info(1,"s")','chosen'); if f then return f() end return f,e end)
for _,v in {false,{},game,script,'bad',0,-1} do task.spawn(function() probe('require.'..type(v)..'.'..tostring(v),function() return require(v) end) end) end
emit('coroutine.running',coroutine.running(),coroutine.status(coroutine.running()),coroutine.isyieldable())
probe('coroutine.pcall',function() return pcall(coroutine.isyieldable) end)
probe('coroutine.metamethod',function() return setmetatable({},{__index=function()return coroutine.isyieldable()end}).x end)
probe('task.spawn',function() return task.spawn(function()end) end)
probe('task.defer',function() return task.defer(function()end) end)
probe('task.delay',function() return task.delay(0,function()end) end)
probe('spawn',function() return spawn(function()end) end)
probe('delay',function() return delay(0,function()end) end)
task.spawn(function() probe('task.cancel.running',function() return task.cancel(coroutine.running()) end) end)
probe('coroutine.close.running',function() return coroutine.close(coroutine.running()) end)
probe('coroutine.close.suspended',function() return coroutine.close(coroutine.create(function()end)) end)
for _,n in {'Version','version','elapsedTime','time','tick','DebuggerManager','PluginManager','settings','stats','UserSettings'} do probe('misc.'..n,function() local v=env[n](); return type(v),typeof(v),tostring(v) end) end
probe('string.metatable',function() local mt=getmetatable(''); return type(mt),tostring(mt),type(mt)=='table' and mt.__index==string end)
probe('string.setmetatable',function() return setmetatable('',{}) end)
probe('rawlen.userdata',function() return rawlen(newproxy()) end)
probe('rawequal.userdata',function() return rawequal(newproxy(),newproxy()) end)
probe('newproxy.bad',function() return newproxy(1) end)
probe('freeze',function() local t={}; table.freeze(t); return table.isfrozen(t),pcall(function()t.a=1 end) end)
probe('os',function() return os.time(),os.clock(),os.date('!%Y-%m-%d'),os.difftime(10,3) end)
probe('os.date.table',function() local t=os.date('!*t',0); return t.year,t.month,t.day,t.hour,t.min,t.sec,t.wday,t.yday,t.isdst end)
probe('random.seed',function() math.randomseed(123); local a,b,c=math.random(),math.random(1,100),math.random(); math.randomseed(123); return a,b,c,a==math.random(),b==math.random(1,100),c==math.random() end)
probe('shared.G',function() return shared==_G,getfenv()._G==_G,rawget(env,'script')==script end)
probe('script.callback',function() local f=function()return script end; return f()==script end)
for _,n in {0,0.001,0.02} do probe('task.wait.'..n,function() local t=os.clock(); local a,b=task.wait(n); return a,b,os.clock()-t end) end
probe('wait',function() return wait(0) end)
emit('DONE')
]====]; s.Parent=game:GetService('Players').LocalPlayer:WaitForChild('PlayerScripts') end