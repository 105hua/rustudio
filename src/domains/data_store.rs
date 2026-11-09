//! Session-backed persistence-v2 and legacy ordered persistence, captured from
//! Studio Play against the explicitly authorized scratch experience (data.md).
use crate::runtime::{LInst, World};
use mlua::{Lua, Table};
use rustudio_account::data::{Context, Request};
use std::collections::HashMap;
use std::sync::{
    mpsc::{self, Receiver, TryRecvError},
    Arc, Mutex,
};
#[derive(Default)]
struct Jobs {
    next: u64,
    pending: HashMap<u64, Receiver<Result<String, String>>>,
}
fn error(s: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(s.into())
}
pub(super) fn session() -> Result<Arc<rustudio_account::Session>, String> {
    static CACHE: Mutex<Option<(String, Arc<rustudio_account::Session>)>> = Mutex::new(None);
    let config = rustudio_account::AccountConfig::load().map_err(|e| e.to_string())?;
    let (cookie, _) = rustudio_account::session::load_cookie(!config.signed_out)
        .map_err(|e| e.to_string())?
        .ok_or("signed out")?;
    let key = cookie.cache_key();
    let mut held = CACHE.lock().map_err(|_| "session cache unavailable")?;
    if let Some((k, s)) = &*held {
        if *k == key {
            return Ok(s.clone());
        }
    }
    let s = Arc::new(rustudio_account::Session::validate(cookie).map_err(|e| e.to_string())?);
    *held = Some((key, s.clone()));
    Ok(s)
}
pub const LUA_METHODS: &[(&str, &str)] = &[
    ("BatchGetAsync", "GlobalDataStore"),
    ("OnUpdate", "GlobalDataStore"),
    ("GetAsync", "GlobalDataStore"),
    ("SetAsync", "GlobalDataStore"),
    ("UpdateAsync", "GlobalDataStore"),
    ("IncrementAsync", "GlobalDataStore"),
    ("RemoveAsync", "GlobalDataStore"),
    ("GetVersionAsync", "DataStore"),
    ("GetVersionAtTimeAsync", "DataStore"),
    ("RemoveVersionAsync", "DataStore"),
    ("ListVersionsAsync", "DataStore"),
    ("ListKeysAsync", "DataStore"),
    ("ListDataStoresAsync", "DataStoreService"),
    ("GetSortedAsync", "OrderedDataStore"),
    ("GetMetadata", "DataStoreKeyInfo"),
    ("GetUserIds", "DataStoreKeyInfo"),
];
pub fn install(lua: &Lua, local_mode: bool) -> mlua::Result<()> {
    lua.set_app_data(Jobs::default());
    let wire = lua.create_table()?;
    wire.set(
        "consume",
        lua.create_function(
            |lua, (method, path, units): (String, String, Option<usize>)| {
                super::budget::consume(lua, &method, &path, units.unwrap_or(1))
            },
        )?,
    )?;
    wire.set(
        "watch_key",
        lua.create_function(|lua, ()| super::budget::watch_key(lua))?,
    )?;
    wire.set(
        "queue",
        lua.create_function(
            |lua, (method, path, delta, op): (String, String, i32, String)| {
                super::budget::queue(lua, &method, &path, delta, &op)
            },
        )?,
    )?;
    wire.set(
        "create",
        lua.create_function(|lua, c: String| {
            crate::runtime::inst_value(lua, crate::runtime::ext::create_any(lua, &c)?)
        })?,
    )?;
    wire.set(
        "properties",
        lua.create_function(|lua, (obj, props): (LInst, Table)| {
            lua.app_data_mut::<super::State>()
                .unwrap()
                .properties
                .insert(obj.0, props);
            Ok(())
        })?,
    )?;
    wire.set(
        "ids",
        lua.create_function(|_, ids: Table| {
            let ids = ids
                .sequence_values::<i64>()
                .collect::<mlua::Result<Vec<_>>>()?;
            serde_json::to_string(&ids).map_err(|e| error(e.to_string()))
        })?,
    )?;
    wire.set(
        "begin",
        lua.create_function(|lua, (json, players): (String, usize)| {
            let request: Request = serde_json::from_str(&json).map_err(|e| error(e.to_string()))?;
            let w = lua
                .app_data_ref::<World>()
                .ok_or_else(|| error("the play session has ended"))?;
            let d = w.d();
            let root = d.dom.root_ref();
            let id = |name| match d.prop(root, name) {
                Some(rbx_dom_weak::types::Variant::Int64(n)) => n.max(0) as u64,
                Some(rbx_dom_weak::types::Variant::Int32(n)) => n.max(0) as u64,
                _ => 0,
            };
            let context = Context {
                place: id("PlaceId"),
                universe: id("GameId"),
                players,
            };
            drop(w);
            let (tx, rx) = mpsc::channel();
            let mut jobs = lua.app_data_mut::<Jobs>().unwrap();
            jobs.next += 1;
            let id = jobs.next;
            jobs.pending.insert(id, rx);
            drop(jobs);
            std::thread::spawn(move || {
                let response = (|| {
                    let s = session()?;
                    let r = request.execute(&s, context).map_err(|e| e.to_string())?;
                    serde_json::to_string(&r).map_err(|e| e.to_string())
                })();
                let _ = tx.send(response);
            });
            Ok(id)
        })?,
    )?;
    wire.set(
        "poll",
        lua.create_function(|lua, id: u64| {
            let mut j = lua.app_data_mut::<Jobs>().unwrap();
            let rx = j
                .pending
                .get(&id)
                .ok_or_else(|| error("unknown data request"))?;
            let r = match rx.try_recv() {
                Err(TryRecvError::Empty) => return Ok((false, None, None)),
                Err(_) => Err("data request worker disconnected".to_string()),
                Ok(r) => r,
            };
            j.pending.remove(&id);
            Ok(match r {
                Ok(v) => (true, Some(v), None),
                Err(e) => (true, None, Some(e)),
            })
        })?,
    )?;
    lua.set_named_registry_value("rustudio_data_wire", wire.clone())?;
    lua.load(SOURCE)
        .set_name("=rustudio.data.store")
        .call::<()>((local_mode, wire))
}
pub const SOURCE: &str = r###"
local localMode,wire=...
local M=__rs_lua_methods
if localMode then return end
local stores,infos={}, {}
local handles={}
local function copy(v,seen)
 if type(v)~='table'then return v end;seen=seen or {};if seen[v]then error('Tables cannot be cyclic',0)end;seen[v]=true
 local out={};for k,x in pairs(v)do out[k]=copy(x,seen)end;seen[v]=nil;return out
end
local function http()return game:GetService('HttpService')end
local function json(v)return http():JSONEncode(v)end
local function decode(s)if s==''then return nil end;return http():JSONDecode(s)end
local function q(s)return http():UrlEncode(tostring(s))end
local function now()return time()end
local function timestamp(s)
 if not s then return 0 end;local d=DateTime.fromIsoDate(s);if not d then error('504: Data store request successful, but response not formatted correctly.',0)end;return d.UnixTimestampMillis
end
local function iso(n)return DateTime.fromUnixTimestampMillis(n):ToIsoDate()end
local function props(class,t)local o=wire.create(class);wire.properties(o,t);return o end
local function info(headers,row,metadata,userids)
 local version=row and row.version or headers['roblox-object-version-id'];if not version then return nil end
 local o=props('DataStoreKeyInfo',{Version=version,CreatedTime=timestamp(row and row.objectCreatedTime or headers['roblox-object-created-time']),UpdatedTime=timestamp(row and row.createdTime or headers['roblox-object-version-created-time'])})
 infos[o]={metadata=copy(metadata or decode(headers['roblox-object-attributes']or'{}')),ids=copy(userids or decode(headers['roblox-object-userids']or'[]'))};return o
end
local priorMetadata=M.GetMetadata
M.GetMetadata=function(self)if infos[self]then return copy(infos[self].metadata)end;if priorMetadata then return priorMetadata(self)end;error('DataStoreKeyInfo has no metadata',0)end
M.GetUserIds=function(self)return copy(assert(infos[self],'DataStoreKeyInfo has no metadata').ids)end
local function net(method,path,body,headers,operation,units)
 for attempt=0,5 do
  local queued=false
  while not wire.consume(method,path,units)do
   if not queued then wire.queue(method,path,1,operation or 'GetAsync');queued=true end
   task.wait()
  end
  if queued then wire.queue(method,path,-1,operation or 'GetAsync')end
  local job=wire.begin(json({method=method,path=path,body=body or '',headers=headers or {}}),#game:GetService('Players'):GetPlayers())
  local result
  repeat task.wait();local done,response,err=wire.poll(job);if done then if err then error(err,0)end;result=decode(response)end until result
  local ok,errorBody=pcall(decode,result.body)
  local retryable=ok and errorBody and type(errorBody)=='table'and errorBody.errors and errorBody.errors[1]and errorBody.errors[1].retryable
  if attempt==5 or not retryable or not game:GetService('DataStoreService').AutomaticRetry then return result end
  task.wait(tonumber(result.headers['retry-after'])or 5)
 end
end
local function failure(r)
 local ok,v=pcall(decode,r.body);local e=ok and v and v.errors and v.errors[1] or {};return e.code or 0,e.message or 'An internal server error occurred.',e.retryable
end
local function checked(r,missing)
 if r.status>=200 and r.status<300 then return r end
 local code,message=failure(r);if missing and(code==11 or code==12)then return nil end
 error('502: API Services rejected request with error. Error code: '..code..' Reason: '..message,0)
end
local function published()
 if not game:GetService('RunService'):IsServer()then error("DataStore can't be accessed from client",0)end
 if game.PlaceId==0 then error('You must publish this place to the web to access DataStore.',0)end
end
local function label(v,what)
 if v==nil then error('Argument 1 missing or nil',0)end;v=tostring(v)
 if #v==0 then error('101: '..what.." name can't be empty.",0)end
 if #v>50 then error('102: '..what..' name exceeds the 50 character limit.',0)end
 return v
end
local function getStore(name,scope,ordered,options)
 published();if name==nil then error('Argument 1 missing or nil',0)end;name=tostring(name);if #name==0 then error("DataStore name can't be empty string",0)end;if #name>50 then error('DataStore name is too long',0)end;scope=scope==nil and 'global'or tostring(scope);if #scope==0 then error("DataStore scope can't be empty string",0)end;if #scope>50 then error('DataStore scope is too long',0)end
 local all=options and options.AllScopes or false;local id=(ordered and 'ordered:'or'standard:')..name..'\0'..scope..'\0'..tostring(all)
 if not handles[id]then local obj=wire.create(ordered and 'OrderedDataStore'or'DataStore');stores[obj]={name=name,scope=scope,ordered=ordered,all=all,cache={}};handles[id]=obj end;return handles[id]
end
M.GetDataStore=function(_,name,scope,options)return getStore(name,scope,false,options)end
M.GetOrderedDataStore=function(_,name,scope)return getStore(name,scope,true)end
M.GetGlobalDataStore=function()return getStore('__global__3a0c3317-5845-4d63-bd11-1acc26b8a6c3-1','u',false)end
local function state(self)return assert(stores[self],'DataStore handle is unavailable')end
local function path(s,key,suffix)
 if s.all then if key==nil then error('Argument 1 missing or nil',0)end;key=tostring(key);local split=string.find(key,'/',1,true);if not split or split==1 then error('Invalid key format with allScopes = true.',0)end;label(string.sub(key,split+1),'Key');if split-1>50 then error('DataStore scope is too long',0)end else key=label(key,'Key')end;suffix=suffix or ''
 if s.ordered then return '/v1/persistence/sorted'..suffix..'?key='..q(s.name)..'&scope='..q(s.scope)..'&target='..q(key)end
 if s.all and not string.find(key,'/',1,true)then error('Invalid key format with allScopes = true.',0)end
 local object=s.all and key or s.scope..'/'..key
 return '/v2/persistence/'..game.GameId..'/datastores/objects/object'..suffix..'?datastore='..q(s.name)..'&objectKey='..q(object)
end
local notify
local function get(s,key,force,extra)
 local c=s.cache[key];if not force and not extra and c and now()-c.time<4 then return copy(c.value),c.info end
 local r=checked(net('GET',path(s,key)..(extra or'')),true);if not r then if not extra then s.cache[key]={time=now()};if notify then notify(s,key,nil)end end;return nil end
 local value=decode(r.body);local i=not s.ordered and info(r.headers)or nil;if not extra then s.cache[key]={value=copy(value),info=i,time=now(),usn=value==nil and''or(r.headers.usn or r.body)};if notify then notify(s,key,value)end end;return value,i
end
local function wrap(class,name,fn)
 local prior=M[name];M[name]=function(self,...)if self:IsA(class)then return fn(self,...)end;if prior then return prior(self,...)end;error('Unsupported '..name..' receiver',0)end
end
wrap('GlobalDataStore','GetAsync',function(self,key,options)return get(state(self),key,options and options.UseCache==false)end)
notify=function(s,key,value)
 local watch=s.watches and s.watches[key];if not watch then return end
 local signature=value~=nil and json(value)or nil
 if signature==watch.signature then return end;watch.signature=signature;watch.event:Fire(copy(value))
end
wrap('GlobalDataStore','OnUpdate',function(self,key,callback)
 local s=state(self);key=label(key,'Key');if type(callback)~='function'then error('Unable to cast value to Function',0)end
 s.watches=s.watches or{};local watch=s.watches[key]
 if not watch then watch={event=wire.create('BindableEvent'),connections={}};s.watches[key]=watch;wire.watch_key()end
 local connection=watch.event.Event:Connect(function(value)callback(copy(value))end);table.insert(watch.connections,connection)
 task.spawn(function()pcall(get,s,key,true)end)
 if not s.polling then
  s.polling=true;task.spawn(function()
   while true do
    task.wait(30-(now()%30))
    local keys={};for key,watch in s.watches do for _,c in watch.connections do if c.Connected then table.insert(keys,key);break end end end
    if #keys>0 then
     if s.ordered then
      for first=1,#keys,100 do local batch={};for i=first,math.min(first+99,#keys)do table.insert(batch,keys[i])end
       local ok,rows=pcall(M.BatchGetAsync,self,batch);if ok then for _,key in batch do notify(s,key,rows[key]and rows[key].value)end end
      end
     else for _,key in keys do pcall(get,s,key,true)end end
    end
   end
  end)
 end
 return connection
end)
local function validate(value,ordered)
 if value==nil then error('Argument 2 missing or nil',0)end
 if ordered and(type(value)~='number'or value%1~=0 or math.abs(value)==math.huge)then error('103: double is not allowed in data stores.',0)end
 local function valid(v,seen)
  local t=type(v);if t=='string'then return utf8.len(v)~=nil elseif t=='number'then return v==v and math.abs(v)~=math.huge elseif t=='boolean'then return true elseif t~='table'then return false end
  seen=seen or {};if seen[v]then return false end;seen[v]=true
  for k,x in pairs(v)do if not valid(k,seen)or not valid(x,seen)then return false end end;seen[v]=nil;return true
 end
 if not valid(value)then error('104: Cannot store '..typeof(value)..' in data store. Data stores can only accept valid UTF-8 characters.',0)end
 local body=json(value);if #body>4194304 then error('105: Serialized value exceeds 4MB limit.',0)end;return body
end
local function set(s,key,value,ids,metadata,condition)
 local headers={};if ids then headers['Roblox-Object-Userids']=wire.ids(ids)end;if metadata then headers['Roblox-Object-Attributes']=next(metadata)==nil and'{}'or json(metadata)end
 for k,v in pairs(condition or{})do if k~='usn' then headers[k]=v end end
 local target=path(s,key);if s.ordered and condition then target=target..'&usn='..q(condition.usn)end
 local r=net('POST',target,validate(value,s.ordered),headers,condition and 'UpdateAsync'or'SetAsync')
 if r.status==412 or r.status==409 then return false end;checked(r)
 if s.ordered then s.cache[key]={value=value,time=now(),usn=r.headers.usn or tostring(value)};notify(s,key,value);return true,nil,nil end
 local row=decode(r.body);local i=info(r.headers,row,metadata,ids);s.cache[key]={value=copy(value),info=i,time=now()};notify(s,key,value);return true,row.version,i
end
wrap('GlobalDataStore','BatchGetAsync',function(self,keys,options)
 local s=state(self)
 if not s.ordered then error('{"errorType":"BatchGetUnsupportedOperation","errorMessage":"BatchGetAsync is not supported.","callbackMessage":"BatchGetAsync is not supported."}',0)end
 if type(keys)~='table'then error('Unable to cast value to Array',0)end
 if #keys==0 then error('OrderedDataStore BatchGetAsync keys array cannot be empty.',0)end
 if #keys>100 then error('OrderedDataStore BatchGetAsync exceeded the 100 key limit.',0)end
 local rows={};for _,key in ipairs(keys)do if type(key)~='string'then error('513: Attribute keys format is invalid',0)end;label(key,'Key');table.insert(rows,{key=s.name,scope=s.scope,target=key})end
 local r=checked(net('POST','/v1/persistence/sorted/multi-get',json({keys=rows}),nil,'BatchGetAsync',#keys));local response=decode(r.body);local result={}
 for _,row in ipairs(response.entries or{})do if row.value and row.value~=''then result[row.target]={value=tonumber(row.value)}end end;return result
end)
wrap('GlobalDataStore','SetAsync',function(self,key,value,ids,options)
 local s=state(self);local _,version=set(s,key,value,ids,options and options:GetMetadata());return version
end)
wrap('GlobalDataStore','UpdateAsync',function(self,key,fn)
 local s=state(self);if type(fn)~='function'then error('Unable to cast value to Function',0)end
 local force=false
 while true do
  local value,i=get(s,key,force);local v,ids,metadata=fn(copy(value),i);if v==nil then return nil end
  if i then ids=ids or i:GetUserIds();metadata=metadata or i:GetMetadata()end
  local condition;if s.ordered then condition={usn=value==nil and''or(s.cache[key]and s.cache[key].usn or tostring(value))} elseif i then condition={['If-Match']='\"'..i.Version..'\"'}else condition={['If-None-Match']='*'}end
  local ok,_,newInfo=set(s,key,v,ids,metadata,condition);if ok then return copy(v),newInfo end;force=true
 end
end)
wrap('GlobalDataStore','IncrementAsync',function(self,key,delta,ids,options)
 local s=state(self);delta=delta or 1;if type(delta)~='number'or delta%1~=0 then error('Unable to cast value to int64',0)end
 local p=path(s,key,'/increment')..(s.ordered and '&by='or'&incrementBy=')..delta
 local h={};if ids then h['Roblox-Object-Userids']=wire.ids(ids)end;if options then local metadata=options:GetMetadata();h['Roblox-Object-Attributes']=next(metadata)==nil and'{}'or json(metadata)end
 local r=checked(net('POST',p,'',h,'IncrementAsync'));local value=decode(r.body);if s.ordered then value=tonumber(value.value)end;local i=not s.ordered and info(r.headers)or nil;s.cache[key]={value=value,info=i,time=now()};notify(s,key,value);return value,i
end)
wrap('GlobalDataStore','RemoveAsync',function(self,key)
 local s=state(self);local r=checked(net(s.ordered and'POST'or'DELETE',path(s,key,s.ordered and'/remove'or''),'',nil,'RemoveAsync'),true);s.cache[key]=nil;notify(s,key,nil)
 if not r then return nil end;return decode(r.body),not s.ordered and info(r.headers)or nil
end)
wrap('DataStore','GetVersionAsync',function(self,key,version)return get(state(self),key,true,'&version='..q(version))end)
wrap('DataStore','GetVersionAtTimeAsync',function(self,key,t)return get(state(self),key,true,'&versionCurrentTime='..q(iso(t)))end)
wrap('DataStore','RemoveVersionAsync',function(self,key,version)checked(net('DELETE',path(state(self),key)..'&version='..q(version)))end)
local function pageFetch(p)
 local path=p.path;if p.cursor~=''then path=path..(p.ordered and'&exclusiveStartKey='or'&exclusiveStartKey=')..q(p.cursor)end
 local r=checked(net('GET',path,'',nil,p.ordered and'GetSorted'or'GetAsync'));local data=decode(r.body);local rows={}
 if p.kind=='keys'then for _,row in ipairs(data.keys or{})do local k=type(row)=='string'and row or(row.key or row.name or row.keyName);if not p.all then k=string.sub(k,#p.scope+2)end;table.insert(rows,props('DataStoreKey',{KeyName=k}))end
 elseif p.kind=='stores'then for _,row in ipairs(data.datastores or{})do table.insert(rows,props('DataStoreInfo',{DataStoreName=row.name,CreatedTime=timestamp(row.createdTime),UpdatedTime=timestamp(row.updatedTime)}))end
 elseif p.kind=='versions'then for _,row in ipairs(data.versions or{})do table.insert(rows,props('DataStoreObjectVersionInfo',{Version=row.version,CreatedTime=timestamp(row.createdTime),IsDeleted=row.deleted}))end
 else for _,row in ipairs(data.entries or{})do table.insert(rows,{key=row.target,value=row.value})end end
 p.cursor=(p.ordered and data.lastEvaluatedKey or data.lastReturnedKey)or'';if p.props.Cursor~=nil then p.props.Cursor=p.cursor end
 return rows,p.cursor==''
end
-- Shared Pages (src/domains/pages.rs): this file only supplies the source.
local function pages(class,p)
 p.props={};if class=='DataStoreKeyPages'or class=='DataStoreListingPages'then p.props.Cursor=''end;p.cursor=p.cursor or''
 local o=__rs_pages_new(class,function()return pageFetch(p)end);wire.properties(o,p.props);return o
end
local function pageSize(n,default,max)n=n or default;if type(n)~='number'or n%1~=0 or n<(default==0 and 0 or 1) or n>max then error('Invalid page size',0)end;return n end
wrap('DataStore','ListKeysAsync',function(self,prefix,n,cursor,excludeDeleted)
 local s=state(self);prefix=prefix or'';n=pageSize(n,0,100)
 local p='/v2/persistence/'..game.GameId..'/datastores/objects?datastore='..q(s.name)..'&prefix='..q(s.all and prefix or s.scope..'/'..prefix)
 if n>0 then p=p..'&maxItemsToReturn='..n end
 if excludeDeleted then p=p..'&excludeDeleted=true'end
 return pages('DataStoreKeyPages',{path=p,kind='keys',scope=s.scope,all=s.all,cursor=cursor or''})
end)
wrap('DataStore','ListVersionsAsync',function(self,key,direction,min,max,n)
 local s=state(self);n=pageSize(n,0,100);local order=direction and direction.Name or'Ascending'
 local p=path(s,key,'/versions')..'&sortOrder='..order
 if n>0 then p=p..'&maxItemsToReturn='..n end
 if min and min~=0 then p=p..'&startTime='..q(iso(min))end;if max and max~=0 then p=p..'&endTime='..q(iso(max))end
 return pages('DataStoreVersionPages',{path=p,kind='versions'})
end)
wrap('DataStoreService','ListDataStoresAsync',function(_,prefix,n,cursor)
 published();n=pageSize(n,0,100);local p='/v2/persistence/'..game.GameId..'/datastores?prefix='..q(prefix or'');if n>0 then p=p..'&maxItemsToReturn='..n end;return pages('DataStoreListingPages',{path=p,kind='stores',cursor=cursor or''})
end)
wrap('OrderedDataStore','GetSortedAsync',function(self,ascending,n,min,max)
 local s=state(self);n=pageSize(n,50,100);local p='/v1/persistence/sorted/list?scope='..q(s.scope)..'&key='..q(s.name)..'&pageSize='..n..'&direction='..(ascending and'asc'or'dsc')
 if min~=nil then p=p..'&minValue='..q(min)end;if max~=nil then p=p..'&maxValue='..q(max)end
 return pages('DataStorePages',{path=p,kind='sorted',ordered=true})
end)
"###;

/// The headless harness already accepts RUSTUDIO_PLACE_CONTEXT for asset
/// delivery. Resolve that explicit published context before scripts start,
/// rather than requiring an upload merely to attach an existing place id.
pub fn bind_context(lua: &Lua) -> mlua::Result<()> {
    let Some(place) = std::env::var("RUSTUDIO_PLACE_CONTEXT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|p| *p != 0)
    else {
        return Ok(());
    };
    {
        let w = lua
            .app_data_ref::<World>()
            .ok_or_else(|| error("the play session has ended"))?;
        let d = w.d();
        if matches!(d.prop(d.dom.root_ref(),"PlaceId"),Some(rbx_dom_weak::types::Variant::Int64(n))if n!=0)
        {
            return Ok(());
        }
    }
    static IDS: Mutex<Option<(u64, u64)>> = Mutex::new(None);
    let cached = IDS
        .lock()
        .ok()
        .and_then(|x| *x)
        .filter(|x| x.0 == place)
        .map(|x| x.1);
    let universe = match cached {
        Some(u) => u,
        None => {
            let s = session().map_err(error)?;
            let (status, body) = s
                .get(&format!(
                    "https://apis.roblox.com/universes/v1/places/{place}/universe"
                ))
                .map_err(|e| error(e.to_string()))?;
            if status != 200 {
                return Err(error(format!(
                    "published place context returned HTTP {status}"
                )));
            }
            let v: serde_json::Value = serde_json::from_str(&body)
                .map_err(|_| error("invalid published place context"))?;
            let u = v["universeId"]
                .as_u64()
                .filter(|u| *u != 0)
                .ok_or_else(|| error("published place has no universe"))?;
            if let Ok(mut held) = IDS.lock() {
                *held = Some((place, u))
            };
            u
        }
    };
    let mut w = lua.app_data_mut::<World>().unwrap();
    let d = w
        .doc
        .as_mut()
        .ok_or_else(|| error("the play session has ended"))?;
    let root = d.dom.root_ref();
    if let Some(i) = d.dom.get_by_ref_mut(root) {
        i.properties.insert(
            rbx_dom_weak::ustr("PlaceId"),
            rbx_dom_weak::types::Variant::Int64(place as i64),
        );
        i.properties.insert(
            rbx_dom_weak::ustr("GameId"),
            rbx_dom_weak::types::Variant::Int64(universe as i64),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conditional_update_retries_and_preserves_metadata() {
        let lua = Lua::new();
        lua.load(r#"
__rs_lua_methods={GetCurrentPage=function()end,AdvanceToNextPageAsync=function()end}
local encoded,seq={},0
local function encode(v)seq+=1;local k='json:'..seq;encoded[k]=v;return k end
local function decode(v)return assert(encoded[v],v)end
local H={JSONEncode=function(_,v)return encode(v)end,JSONDecode=function(_,v)return decode(v)end,UrlEncode=function(_,v)return tostring(v)end}
game={PlaceId=10000000000008,GameId=10000000007}
function game:GetService(n)if n=='HttpService'then return H elseif n=='Players'then return{GetPlayers=function()return{}end}elseif n=='RunService'then return{IsServer=function()return true end}else return{AutomaticRetry=true}end end
task={wait=function()end};function time()return 0 end
DateTime={fromIsoDate=function()return{UnixTimestampMillis=123}end}
wire={consume=function()return true end,queue=function()error('unexpected queue')end}
function wire.create(class)local o={ClassName=class};function o:IsA(c)return c==class or(c=='GlobalDataStore'and class=='DataStore')end;return setmetatable(o,{__index=function(t,k)local p=rawget(t,'props');return p and p[k]or __rs_lua_methods[k]end})end
function wire.properties(o,p)o.props=p end
function wire.ids(ids)return encode(ids)end
local calls=0
function wire.begin(s)
 local r=decode(s);calls+=1
 local response
 if calls==1 then assert(r.method=='GET');response={status=404,headers={},body=encode({errors={{code=11,message='missing'}}})}
 elseif calls==2 then assert(r.method=='POST'and r.headers['If-None-Match']=='*');response={status=412,headers={},body=encode({errors={{code=5,message='conflict'}}})}
 elseif calls==3 then assert(r.method=='GET');response={status=200,headers={['roblox-object-version-id']='v1',['roblox-object-created-time']='date',['roblox-object-version-created-time']='date',['roblox-object-attributes']=encode({z=7}),['roblox-object-userids']=encode({10000000001})},body=encode(10)}
 elseif calls==4 then assert(r.headers['If-Match']=='"v1"');assert(decode(r.headers['Roblox-Object-Attributes']).z==7);assert(decode(r.headers['Roblox-Object-Userids'])[1]==10000000001);assert(decode(r.body)==11);response={status=200,headers={},body=encode({version='v2',createdTime='date',objectCreatedTime='date'})}
 else error('unexpected request')end
 pending=encode(response);return calls
end
function wire.poll()return true,pending,nil end
"#).exec().unwrap();
        lua.load(SOURCE)
            .call::<()>((false, lua.globals().get::<Table>("wire").unwrap()))
            .unwrap();
        lua.load(r#"local s=__rs_lua_methods.GetDataStore({},'test');local calls=0;local v,i=s:UpdateAsync('key',function(old)calls+=1;return (old or 0)+1 end);assert(v==11 and calls==2 and i.Version=='v2'and i:GetMetadata().z==7 and i:GetUserIds()[1]==10000000001);assert(s:GetAsync('key')==11)"#).exec().unwrap();
    }
    #[test]
    fn ordered_batch_and_watch_mock_protocol() {
        let lua = Lua::new();
        lua.load(r#"
__rs_lua_methods={GetCurrentPage=function()end,AdvanceToNextPageAsync=function()end}
local encoded,seq={},0
local function encode(v)if type(v)=='number'then local s='n:'..v;encoded[s]=v;return s end;seq+=1;local s='j:'..seq;encoded[s]=v;return s end
local function decode(v)return encoded[v]end
local H={JSONEncode=function(_,v)return encode(v)end,JSONDecode=function(_,v)return decode(v)end,UrlEncode=function(_,v)return tostring(v)end}
game={PlaceId=10000000000008,GameId=10000000007}
function game:GetService(n)if n=='HttpService'then return H elseif n=='Players'then return{GetPlayers=function()return{}end}elseif n=='RunService'then return{IsServer=function()return true end}else return{AutomaticRetry=true}end end
spawned={};task={wait=function()end,spawn=function(fn)table.insert(spawned,fn)end};function time()return 0 end
wire={};watched=0;units=0;local current=7;local pending
function wire.watch_key()watched+=1 end
function wire.consume(_,_,n)units=n or 1;return true end
function wire.create(class)
 if class=='BindableEvent'then
  local event={listeners={}};event.Event={Connect=function(_,fn)local c={Connected=true};function c:Disconnect()self.Connected=false end;table.insert(event.listeners,{fn=fn,c=c});return c end};function event:Fire(v)for _,r in self.listeners do if r.c.Connected then r.fn(v)end end end;return event
 end
 local o={ClassName=class};function o:IsA(c)return c==class or(c=='GlobalDataStore'and(class=='DataStore'or class=='OrderedDataStore'))end;return setmetatable(o,{__index=__rs_lua_methods})
end
function wire.begin(s)
 local req=decode(s);local r={status=200,headers={usn=tostring(current)},body=current and encode(current)or''}
 if string.find(req.path,'/multi-get',1,true)then local body=decode(req.body);assert(body.keys[1].target=='a'and body.keys[1].scope=='global');r.body=encode({entries={{target='a',value='11'}}})
 elseif req.method=='POST'then if string.find(req.path,'/remove?',1,true)then current=nil else current=decode(req.body);r.body=encode({usn=tostring(current)})end end
 pending=encode(r);return 1
end
function wire.poll()return true,pending,nil end
"#).exec().unwrap();
        lua.load(SOURCE)
            .call::<()>((false, lua.globals().get::<Table>("wire").unwrap()))
            .unwrap();
        lua.load(r#"
local d=__rs_lua_methods.GetOrderedDataStore({},'mock');local values={};local c=d:OnUpdate('k',function(v)table.insert(values,v or'gone')end)
assert(c.Connected and watched==1 and #values==0);spawned[1]();assert(values[1]==7)
d:SetAsync('k',9);assert(values[2]==9);d:RemoveAsync('k');assert(values[3]=='gone');c:Disconnect();d:SetAsync('k',10);assert(#values==3)
local rows=d:BatchGetAsync({'a','missing'});assert(rows.a.value==11 and rows.missing==nil and units==2)
assert(not pcall(d.BatchGetAsync,d,{}));assert(not pcall(d.BatchGetAsync,d,{1}))
local standard=__rs_lua_methods.GetDataStore({},'mock');local ok,e=pcall(standard.BatchGetAsync,standard,{});assert(not ok and string.find(e,'BatchGetUnsupportedOperation',1,true))
"#).exec().unwrap();
    }
    #[test]
    #[ignore = "writes only the explicitly authorized scratch universe through the login session"]
    fn live_scratch_roundtrip() {
        use crate::doc::Document;
        use crate::runtime::{RunMode, Runtime};
        use rbx_dom_weak::{types::Variant, ustr, InstanceBuilder};
        let mut d = Document::new_place();
        let root = d.dom.root_ref();
        let i = d.dom.get_by_ref_mut(root).unwrap();
        i.properties
            .insert(ustr("PlaceId"), Variant::Int64(10000000000008));
        i.properties
            .insert(ustr("GameId"), Variant::Int64(10000000007));
        let parent = d.find_child(root, "ServerScriptService").unwrap();
        let script=d.dom.insert(parent,InstanceBuilder::new("Script").with_name("DataLiveScratch").with_property("Source",r#"
assert(game.GameId==10000000007 and game.PlaceId==10000000000008)
local name='rustudio_rt_'..os.time();shared.testStore=name
local d=game:GetService('DataStoreService'):GetDataStore(name)
local observed={};local watch=d:OnUpdate('roundtrip',function(value)table.insert(observed,value and value.n or'gone')end)
local options=Instance.new('DataStoreSetOptions');options:SetMetadata({origin='rustudio-live'})
local version=d:SetAsync('roundtrip',{n=1},{10000000001},options);assert(type(version)=='string')
local value,info=d:GetAsync('roundtrip');assert(value.n==1 and info.Version==version and info:GetMetadata().origin=='rustudio-live' and info:GetUserIds()[1]==10000000001)
local updated,newInfo=d:UpdateAsync('roundtrip',function(old,keyInfo)assert(old.n==1 and keyInfo.Version==version);return {n=2},nil,{origin='updated'}end);assert(updated.n==2 and newInfo:GetMetadata().origin=='updated')
local noCache=Instance.new('DataStoreGetOptions');noCache.UseCache=false;local wireValue,wireInfo=d:GetAsync('roundtrip',noCache);assert(wireValue.n==2 and wireInfo.Version==newInfo.Version)
local atTime=d:GetVersionAtTimeAsync('roundtrip',DateTime.now().UnixTimestampMillis);assert(atTime.n==2)
local historical,historicalInfo=d:GetVersionAsync('roundtrip',newInfo.Version);assert(historical.n==2 and historicalInfo.Version==newInfo.Version)
local versions=d:ListVersionsAsync('roundtrip',Enum.SortDirection.Descending,0,0,10);assert(#versions:GetCurrentPage()>0 and versions:GetCurrentPage()[1].Version==newInfo.Version)
local increment,incrementInfo=d:IncrementAsync('counter',3,{10000000001});assert(increment==3 and incrementInfo:GetUserIds()[1]==10000000001)
local keys=d:ListKeysAsync('',10);local found=false;for i=1,100 do for _,k in keys:GetCurrentPage()do if k.KeyName=='roundtrip'then found=true end end;if keys.IsFinished then break end;keys:AdvanceToNextPageAsync()end;assert(found,'roundtrip key absent from listing')
local stores=game:GetService('DataStoreService'):ListDataStoresAsync(name,10);assert(stores:GetCurrentPage()[1].DataStoreName==name)
local removed,removedInfo=d:RemoveAsync('roundtrip');assert(removed.n==2,'removed value');print('REMOVE_VERSIONS',removedInfo.Version,newInfo.Version);assert(removedInfo.Version==newInfo.Version,'removed version');assert(d:GetAsync('roundtrip')==nil,'value after remove')
task.wait(0.1);assert(observed[1]==1 and observed[#observed]=='gone');watch:Disconnect()
local deletedVersions=d:ListVersionsAsync('roundtrip',Enum.SortDirection.Descending,0,0,10);d:RemoveVersionAsync('roundtrip',deletedVersions:GetCurrentPage()[1].Version)
local ordered=game:GetService('DataStoreService'):GetOrderedDataStore(name);ordered:SetAsync('a',10);ordered:SetAsync('b',20);assert(ordered:GetAsync('a')==10);assert(ordered:IncrementAsync('a',2)==12);assert(ordered:UpdateAsync('a',function(old)assert(old==12);return 15 end)==15)
local batch=ordered:BatchGetAsync({'a','b','missing'});assert(batch.a.value==15 and batch.b.value==20 and batch.missing==nil)
local sorted=ordered:GetSortedAsync(false,10):GetCurrentPage();assert(sorted[1].key=='b' and sorted[1].value==20 and sorted[2].value==15);ordered:RemoveAsync('a');ordered:RemoveAsync('b')
shared.dataLiveDone=true;print('SCRATCH ROUNDTRIP PASSED',name)
"#.to_string()));
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(script);
        for _ in 0..6000 {
            rt.step(0.02);
            if rt
                .lua
                .globals()
                .get::<Table>("shared")
                .unwrap()
                .get::<bool>("dataLiveDone")
                .unwrap_or(false)
            {
                return;
            }
            if rt
                .world_mut()
                .unwrap()
                .output
                .iter()
                .any(|(l, _)| matches!(l, crate::app::Level::Error))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!(
            "scratch roundtrip failed: {:?}",
            rt.world_mut().unwrap().output
        );
    }
}
