//! Studio CSM Test-channel messaging and isolated local-session delivery.
pub const LUA_METHODS: &[(&str, &str)] = &[
    ("PublishAsync", "MessagingService"),
    ("SubscribeAsync", "MessagingService"),
];
pub const SOURCE: &str = r###"
local localMode, create, jsonEncode, isServer, remote = ...
local M=__rs_lua_methods
local channels={}
local function server()
    if not isServer() then error('MessagingService can only be executed by game server',0) end
end
local function topic(t)
    if type(t)~='string' then error('Unable to cast value to string',0) end
    if #t==0 then error('MessagingService: Topic names cannot be empty.',0) end
    if #t>80 then error('MessagingService: Topic names cannot be more than 80 characters.',0) end
end
local function copy(v,seen)
    if type(v)~='table' then return v end
    seen=seen or {};if seen[v] then error('MessagingService:PublishAsync(): Cannot publish message, can only accept valid UTF-8 characters.',0) end
    seen[v]=true;local t={};for k,x in pairs(v)do t[k]=copy(x,seen)end;seen[v]=nil;return t
end
local function channel(t)
    if not channels[t] then channels[t]=create('BindableEvent') end
    return channels[t]
end
local priorSubscribe=M.SubscribeAsync
M.SubscribeAsync=function(self,t,callback)
    if self.ClassName~='MessagingService' then if priorSubscribe then return priorSubscribe(self,t,callback)end;error('Unsupported SubscribeAsync receiver',0)end
    if type(callback)~='function' then error('Unable to cast '..(type(callback)=='number' and 'double' or type(callback))..' to Function',0) end
    server();topic(t);task.wait()
    if not localMode then return remote.subscribe(t,function(message)callback(copy(message))end) end
    return channel(t).Event:Connect(function(message)callback(copy(message))end)
end
local priorPublish=M.PublishAsync
M.PublishAsync=function(self,t,value)
    if self.ClassName~='MessagingService' then if priorPublish then return priorPublish(self,t,value)end;error('Unsupported PublishAsync receiver',0)end
    server();topic(t)
    local bytes
    if type(value)=='string' then
        if not utf8.len(value) then error('MessagingService:PublishAsync(): Cannot publish message, can only accept valid UTF-8 characters.',0)end
        bytes=value
    else
        local ok,result=pcall(jsonEncode,value)
        if not ok then error('MessagingService:PublishAsync(): Cannot publish message, can only accept valid UTF-8 characters.',0)end
        bytes=result
    end
    if #bytes>1024 then error('MessagingService:PublishAsync(): Cannot publish, data exceeded 1kb',0)end
    local value=copy(value);task.wait()
    if not localMode then return remote.publish(t,value) end
    if channels[t] then channels[t]:Fire({Data=value,Sent=os.time()})end
end
"###;
#[cfg(test)]
mod tests {
    use super::*;
    use mlua::{Function, Lua};
    #[test]
    fn local_messaging_validation_delivery_disconnect() {
        let lua = Lua::new();
        lua.load(r#"
__rs_lua_methods={}; task={wait=function()end};server=true
function create()
 local event={listeners={}}
 event.Event={Connect=function(_,fn)local c={Connected=true};function c:Disconnect()self.Connected=false end;table.insert(event.listeners,{fn=fn,c=c});return c end}
 function event:Fire(v)for _,r in self.listeners do if r.c.Connected then r.fn(v)end end end
 return event
end
function encode(v)if type(v)=='function' then error('invalid')end;return '{}'end
function isServer()return server end
service={ClassName='MessagingService'};setmetatable(service,{__index=__rs_lua_methods})
"#).exec().unwrap();
        lua.load(SOURCE)
            .call::<()>((
                true,
                lua.globals().get::<Function>("create").unwrap(),
                lua.globals().get::<Function>("encode").unwrap(),
                lua.globals().get::<Function>("isServer").unwrap(),
            ))
            .unwrap();
        lua.load(r#"
local hits=0;local c=service:SubscribeAsync('x',function(m)assert(m.Data.a==2 and type(m.Sent)=='number');m.Data.a=9;hits+=1 end)
local c2=service:SubscribeAsync('x',function(m)assert(m.Data.a==2);hits+=1 end)
local data={a=2};service:PublishAsync('x',data);assert(hits==2 and data.a==2)
c:Disconnect();assert(not c.Connected);service:PublishAsync('x',data);assert(hits==3)
c2:Disconnect();service:PublishAsync('x',data);assert(hits==3)
service:PublishAsync('no-listeners',string.rep('a',1024))
for _,v in {string.rep('a',1025),string.char(255),function()end}do assert(not pcall(service.PublishAsync,service,'x',v))end
local ok,e=pcall(service.SubscribeAsync,service,'',function()end);assert(not ok and e=='MessagingService: Topic names cannot be empty.')
server=false;local ok,e=pcall(service.PublishAsync,service,'x','a');assert(not ok and e=='MessagingService can only be executed by game server')
"#).exec().unwrap();
    }
}

use mlua::{Lua,Table};
use std::{collections::HashMap,sync::mpsc};
struct Transport {
    next:u64,
    tx:mpsc::Sender<(u64,String,serde_json::Value)>,
    rx:mpsc::Receiver<serde_json::Value>,
}
pub fn install_backend(lua:&Lua,local:bool)->mlua::Result<()> {
    if local{return Ok(())}
    let remote:Table=lua.named_registry_value("rustudio_messaging_remote")?;
    let wire:Table=lua.named_registry_value("rustudio_data_wire")?;
    let submit=lua.create_function(|lua,(target,args):(String,String)|{
        let args:serde_json::Value=serde_json::from_str(&args).map_err(mlua::Error::external)?;
        if lua.app_data_ref::<Transport>().is_none(){
            use rbx_dom_weak::types::Variant;
            let w=lua.app_data_ref::<crate::runtime::World>().unwrap();let d=w.d();let root=d.dom.root_ref();
            let id=|n|match d.prop(root,n){Some(Variant::Int64(v))=>v.max(0)as u64,Some(Variant::Int32(v))=>v.max(0)as u64,_=>0};
            let context=rustudio_account::data::Context{place:id("PlaceId"),universe:id("GameId"),players:0};drop(w);
            let(tx,commands)=mpsc::channel::<(u64,String,serde_json::Value)>();let(events,rx)=mpsc::channel();
            std::thread::spawn(move||{
                let run=||->Result<(),String>{
                    let session=super::store::session()?;
                    let mut socket=rustudio_account::data::messaging::Connection::connect(&session,context).map_err(|e|e.to_string())?;
                    socket.timeout(std::time::Duration::from_millis(100));let mut pending=HashMap::new();let mut heartbeat=std::time::Instant::now();
                    loop {
                        if heartbeat.elapsed().as_secs()>=15 {socket.keepalive().map_err(|e|e.to_string())?;heartbeat=std::time::Instant::now();}
                        loop {match commands.try_recv(){
                            Ok((id,target,args))=>{let invocation=socket.invoke(&target,args).map_err(|e|e.to_string())?;pending.insert(invocation,id);},
                            Err(mpsc::TryRecvError::Empty)=>break,
                            Err(mpsc::TryRecvError::Disconnected)=>return Ok(())
                        }}
                        for frame in socket.read().map_err(|e|e.to_string())?{
                            if frame["type"]==3 {
                                if let Some(id)=frame["invocationId"].as_str().and_then(|i|pending.remove(i)){
                                    let result=&frame["result"];
                                    let error=if frame["error"].is_string(){frame["error"].clone()}else if result["status"].as_str().is_some_and(|s|s!="Ok"){serde_json::Value::String(format!("MessagingService: {}",result["status"]))}else{serde_json::Value::Null};
                                    if events.send(serde_json::json!({"id":id,"error":error})).is_err(){return Ok(())}
                                }
                            }else if frame["target"]=="Message"{
                                if events.send(serde_json::json!({"topic":frame["arguments"][0],"message":frame["arguments"][1]})).is_err(){return Ok(())}
                            }
                        }
                    }
                };
                if let Err(error)=run(){let _=events.send(serde_json::json!({"fatal":error}));}
            });
            lua.set_app_data(Transport{next:0,tx,rx});
        }
        let mut state=lua.app_data_mut::<Transport>().unwrap();state.next+=1;let id=state.next;
        state.tx.send((id,target,args)).map_err(|_|mlua::Error::runtime("MessagingService: CSM connection closed"))?;Ok(id)
    })?;
    let poll=lua.create_function(|lua,()|{
        let state=lua.app_data_ref::<Transport>();let frames:Vec<_>=state.as_ref().map(|s|s.rx.try_iter().collect()).unwrap_or_default();
        serde_json::to_string(&frames).map_err(mlua::Error::external)
    })?;
    lua.load(REMOTE_SOURCE).set_name("=rustudio.data.csm").call::<()>((remote,wire,submit,poll))
}
const REMOTE_SOURCE:&str=r###"
local remote,wire,submit,poll=...
local channels,pending={},{};local running=false;local fatal=nil
local http=setmetatable({}, {__index=function(_,key)return function(_,...)local service=game:GetService('HttpService');return service[key](service,...)end end})
local function request(target,args)
 if fatal then error(fatal,0)end
 local id=submit(target,http:JSONEncode(args));pending[id]={}
 if not running then
  running=true;task.spawn(function()
   while true do
    task.wait()
    for _,frame in http:JSONDecode(poll())do
     if frame.fatal then fatal=frame.fatal;for _,p in pending do p.done=true;p.error=fatal end
     elseif frame.id then local p=pending[frame.id];if p then p.done=true;p.error=frame.error end
     elseif frame.topic then local c=channels[frame.topic];if c then local ok,message=pcall(http.JSONDecode,http,frame.message);if ok then c.event:Fire({Data=message.Data,Sent=message.Sent})end end end
    end
    for topic,c in channels do
     local live=false;for _,connection in c.connections do if connection.Connected then live=true;break end end
     if c.ready and not live then channels[topic]=nil;task.spawn(function()pcall(request,'Unsubscribe',{topic,0})end)end
    end
   end
  end)
 end
 repeat task.wait()until pending[id].done
 local result=pending[id];pending[id]=nil;if result.error then error(result.error,0)end
end
function remote.subscribe(topic,callback)
 local c=channels[topic];if not c then c={event=wire.create('BindableEvent'),connections={},ready=false};channels[topic]=c end
 local connection=c.event.Event:Connect(callback);table.insert(c.connections,connection)
 if not c.ready then local ok,e=pcall(request,'Subscribe',{topic,0});if not ok then connection:Disconnect();error(e,0)end;c.ready=true end
 return connection
end
function remote.publish(topic,value)
 request('Publish',{topic,http:JSONEncode({Sent=DateTime.now().UnixTimestampMillis/1000,Data=value,TraceId=0,TracingEnabled=false}),0})
end
"###;
