//! Real Studio UMS paths, bodies, status codes and conditional updates captured
//! against the authorized scratch universe; local emulation remains separate.
use mlua::{Lua, Table};
pub fn install(lua: &Lua, local: bool) -> mlua::Result<()> {
    if local {
        return Ok(());
    }
    let wire: Table = lua.named_registry_value("rustudio_data_wire")?;
    lua.load(SOURCE)
        .set_name("=rustudio.data.ums")
        .call::<()>(wire)
}
pub const SOURCE: &str = r###"
local wire=...
local M=__rs_lua_methods
local handles,states={}, {}
local function copy(v,seen)
 if type(v)~='table'then return v end;seen=seen or {};if seen[v]then error('InvalidRequest: Unable to convert value to json.',0)end;seen[v]=true;local t={};for k,x in pairs(v)do t[k]=copy(x,seen)end;seen[v]=nil;return t
end
local function http()return game:GetService('HttpService')end
local function json(v)return http():JSONEncode(v)end
local function decode(v)if v==''then return nil end;local ok,result=pcall(http().JSONDecode,http(),v);if ok then return result end;return nil end
local function q(v)return http():UrlEncode(tostring(v))end
local function request(method,path,body)
 local id=wire.begin(json({method=method,path=path,body=body and json(body)or'',headers={}}),#game:GetService('Players'):GetPlayers())
 repeat task.wait();local done,data,err=wire.poll(id);if done then if err then error(err,0)end;local r=decode(data);return r,decode(r.body)end until false
end
local statuses={[1]='InvalidRequest',[2]='InvalidParameter',[3]='AccessDenied',[4]='ItemValueSizeTooLarge',[5]='TotalMemoryOverLimit',[6]='TotalRequestsOverLimit',[0]='Unknown',[7]='Created',[8]='DataUpdateConflict',[9]='NoItemFound',[10]='Success',[11]='OutOfRange',[12]='InsufficientScope',[13]='NotFound',[14]='Aborted',[15]='Cancelled',[16]='Timeout',[17]='Throttled',[18]='InternalError',[19]='DataStructureItemsOverLimit',[20]='DataStructureMemoryOverLimit',[21]='DataStructureRequestsOverLimit',[22]='PartitionRequestsOverLimit'}
local function checked(r,v)
 if r.status>=200 and r.status<300 then return v end
 if type(v)=='table'and v.message then error((statuses[v.code]or'Unknown')..': '..v.message..(v.correlationId and' RequestId: '..v.correlationId or''),0)end
 if r.status==429 then error('RequestThrottled: Recent MemoryStores requests hit one or more limits.',0)end
 error('InternalError: Internal Error.',0)
end
local function path(s,suffix,key)
 local p='/ums-service/studio/'..s.kind..'/v1/'..suffix..'?universeId='..game.GameId..'&name='..q(s.name);if key~=nil then p=p..'&key='..q(key)end;return p
end
local function expiration(n)if type(n)~='number'or n~=n or n<0 or n>3888000 then error("InvalidExpirationTime: The field 'expiration' time must be between 0 and 3,888,000.",0)end;return math.floor(n)*1000 end
local function key(k)if k==nil then error('Argument 1 missing or nil',0)end;return tostring(k)end
local function open(class,kind,name,invisibility)
 if not game:GetService('RunService'):IsServer()then error('MemoryStoreService can only be executed by game server',0)end
 if game.PlaceId==0 then error('You must publish this place to use MemoryStoreService.',0)end
 name=key(name);local id=class..':'..name..':'..tostring(invisibility)
 if not handles[id]then local o=wire.create(class);states[o]={kind=kind,name=name,invisibility=invisibility or 30};handles[id]=o end;return handles[id]
end
local function wrap(class,name,fn)local prior=M[name];M[name]=function(self,...)if self.ClassName==class then return fn(self,...)end;return prior(self,...)end end
wrap('MemoryStoreService','GetHashMap',function(_,name)return open('MemoryStoreHashMap','hash-map',name)end)
wrap('MemoryStoreService','GetSortedMap',function(_,name)return open('MemoryStoreSortedMap','sorted-map',name)end)
wrap('MemoryStoreService','GetQueue',function(_,name,invisibility)return open('MemoryStoreQueue','queue',name,invisibility)end)
-- Counter acquisition keeps the directly observed disabled-feature gate.
for _,class in {'MemoryStoreHashMap','MemoryStoreSortedMap'}do
 local sorted=class=='MemoryStoreSortedMap'
 wrap(class,'GetAsync',function(self,k)local s=states[self];local r,v=request('GET',path(s,'item',key(k)));v=checked(r,v);if not v then return nil end;if sorted then return copy(v.data),v.sortKey end;return copy(v.data)end)
 wrap(class,'SetAsync',function(self,k,value,ttl,sk)
  local s=states[self];local body={data=value,expires=expiration(ttl)};if sorted then body.sortKey=sk end;local r,v=request('POST',path(s,'item',key(k)),body);checked(r,v);return r.status==200
 end)
 wrap(class,'RemoveAsync',function(self,k)local r,v=request('DELETE',path(states[self],'item',key(k)));checked(r,v)end)
 wrap(class,'UpdateAsync',function(self,k,fn,ttl)
  local s=states[self];local api=sorted and'SortedMap.Update'or'HashMap.Update';ttl=expiration(ttl);k=key(k)
  if type(fn)~='function'then error('Unable to cast value to Function',0)end
  local r,old=request('GET',path(s,'item',k)..'&clientApiName='..api);old=checked(r,old)
  while true do
   local ok,value,sk=pcall(fn,old and copy(old.data),old and old.sortKey);if not ok then error('TransformCallbackFailed: Failed to invoke transformation function.',0)end;if value==nil then return nil end
   local body={data=value,expires=ttl,expect=old and old.usn or'',clientApiName=api};if sorted then body.sortKey=sk end
   local response,nextValue=request('POST',path(s,'item',k),body)
   if response.status~=409 then checked(response,nextValue);if sorted then return copy(value),sk end;return copy(value)end
   old=nextValue
  end
 end)
end
wrap('MemoryStoreSortedMap','GetSizeAsync',function(self)local r,v=request('GET',path(states[self],'size'));return checked(r,v).size end)
wrap('MemoryStoreSortedMap','GetRangeAsync',function(self,direction,count,lower,upper)
 local function bound(v)if v==nil then return{key='',sortKey=''}elseif type(v)=='string'then return{key=v,sortKey=''}else return{key=v.key or'',sortKey=v.sortKey or''}end end
 local r,v=request('POST',path(states[self],'item/range'),{ascending=direction==Enum.SortDirection.Ascending,count=count,lowerBound=bound(lower),upperBound=bound(upper)})
 v=checked(r,v);local rows={};for _,row in ipairs(v.items or{})do table.insert(rows,{key=row.key,value=copy(row.data),sortKey=row.sortKey})end;return rows
end)
wrap('MemoryStoreQueue','AddAsync',function(self,value,ttl,priority)local r,v=request('POST',path(states[self],'add'),{data=value,priority=priority or 0,expires=expiration(ttl)});checked(r,v)end)
wrap('MemoryStoreQueue','GetSizeAsync',function(self,exclude)local r,v=request('GET',path(states[self],'size')..'&excludeInvisible='..tostring(exclude or false));return checked(r,v).size end)
wrap('MemoryStoreQueue','RemoveAsync',function(self,id)local r,v=request('POST',path(states[self],'remove')..'&id='..q(id));checked(r,v)end)
wrap('MemoryStoreQueue','ReadAsync',function(self,count,allOrNothing,timeout)
 local s=states[self];timeout=timeout or -1;local deadline=timeout<0 and math.huge or time()+timeout
 repeat
  local r,v=request('POST',path(s,'read')..'&count='..count..'&allOrNothing='..tostring(allOrNothing or false)..'&waitTimeout=0&invisibilityTimeout='..math.floor(s.invisibility*1000));v=checked(r,v)
  if v and v.data and #v.data>0 then return copy(v.data),v.id end
  if time()>=deadline then return {},nil end;task.wait(math.min(2,deadline-time()))
 until false
end)
local function fetch(p)
 local api=p.started and'HashMap.ListItems_AdvanceToNextPage'or'HashMap.ListItems';p.started=true
 local r,v=request('GET',path(p.state,'keys')..'&count='..p.count..'&cursor='..q(p.cursor)..'&clientApiName='..api);v=checked(r,v);p.cursor=v.cursor or'';local rows={}
 if #v.keys>0 then local rr,values=request('POST',path(p.state,'values'),{keys=v.keys,clientApiName=api});values=checked(rr,values);for _,row in ipairs(values.items or{})do table.insert(rows,{key=row.key,value=copy(row.data)})end end
 return rows,p.cursor==''
end
-- Shared Pages (src/domains/pages.rs): this file only supplies the source.
wrap('MemoryStoreHashMap','ListItemsAsync',function(self,count)
 local p={state=states[self],count=count,cursor=''}
 return __rs_pages_new('MemoryStoreHashMapPages',function()return fetch(p)end)
end)
"###;
#[cfg(test)]
mod tests {
    use super::*;
    pub const LIVE_PROBE: &str = r#"
assert(game.GameId==10000000007 and game.PlaceId==10000000000008)
local service=game:GetService('MemoryStoreService');local name='rustudio_ums_'..os.time()
local h=service:GetHashMap(name);assert(h:SetAsync('a',{n=1},120)==false);assert(h:GetAsync('a').n==1)
assert(h:UpdateAsync('a',function(v)return{n=v.n+1}end,120).n==2)
assert(h:SetAsync('b',3,120)==false)
local pages=h:ListItemsAsync(100);local found={};for i=1,100 do for _,row in pages:GetCurrentPage()do found[row.key]=row.value end;if pages.IsFinished then break end;pages:AdvanceToNextPageAsync()end;assert(found.a.n==2 and found.b==3)
h:RemoveAsync('a');h:RemoveAsync('b');assert(h:GetAsync('a')==nil)
local s=service:GetSortedMap(name);s:SetAsync('a','first',120,1);s:SetAsync('b','second',120,2)
local v,k=s:GetAsync('a');assert(v=='first'and k==1)
local v,k=s:UpdateAsync('a',function(v,k)return v..'!',k+2 end,120);assert(v=='first!'and k==3)
assert(s:GetSizeAsync()==2);local rows=s:GetRangeAsync(Enum.SortDirection.Ascending,10);assert(rows[1].key=='b'and rows[2].sortKey==3)
s:RemoveAsync('a');s:RemoveAsync('b')
local q=service:GetQueue(name,2);q:AddAsync('low',120,1);q:AddAsync('high',120,2);assert(q:GetSizeAsync()==2)
local values,id=q:ReadAsync(1,false,0);assert(values[1]=='high'and type(id)=='string');assert(q:GetSizeAsync(true)==1);q:RemoveAsync(id)
local values,id=q:ReadAsync(1,false,0);assert(values[1]=='low');q:RemoveAsync(id);assert(q:GetSizeAsync()==0)
shared.memoryLiveDone=true;print('MEMORY SCRATCH ROUNDTRIP PASSED',name)
"#;
    #[test]
    #[ignore = "writes only the explicitly authorized scratch universe via Studio UMS scope"]
    fn live_memory_scratch_roundtrip() {
        use crate::{doc::Document,runtime::{RunMode,Runtime,World}};
        use rbx_dom_weak::{types::Variant,ustr,InstanceBuilder};
        let mut d=Document::new_place();let root=d.dom.root_ref();
        let i=d.dom.get_by_ref_mut(root).unwrap();i.properties.insert(ustr("PlaceId"),Variant::Int64(10000000000008));i.properties.insert(ustr("GameId"),Variant::Int64(10000000007));
        let parent=d.find_child(root,"ServerScriptService").unwrap();
        let script=d.dom.insert(parent,InstanceBuilder::new("Script").with_name("MemoryScratch").with_property("Source",LIVE_PROBE.to_string()));
        let rt=Runtime::new().unwrap();rt.enter(World::new(d,RunMode::Run));rt.bind_globals().unwrap();rt.run_script(script);
        for _ in 0..6000 {
            rt.step(0.02);
            if rt.lua.globals().get::<Table>("shared").unwrap().get::<bool>("memoryLiveDone").unwrap_or(false){return}
            if rt.world_mut().unwrap().output.iter().any(|(l,_)|matches!(l,crate::app::Level::Error)){break}
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("memory scratch roundtrip failed: {:?}",rt.world_mut().unwrap().output);
    }
}
