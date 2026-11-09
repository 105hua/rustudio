//! Explicit local MemoryStore emulator with expiry, ordering and documented
//! universe request/storage quotas. Published requests use data_memory_backend.
pub const LUA_METHODS: &[(&str, &str)] = &[
    ("GetQueue", "MemoryStoreService"),
    ("GetSortedMap", "MemoryStoreService"),
    ("GetHashMap", "MemoryStoreService"),
    ("GetDistributedCounter", "MemoryStoreService"),
    ("AddAsync", "MemoryStoreQueue"),
    ("GetSizeAsync", "MemoryStoreQueue"),
    ("ReadAsync", "MemoryStoreQueue"),
    ("RemoveAsync", "MemoryStoreQueue"),
    ("GetAsync", "MemoryStoreSortedMap"),
    ("GetRangeAsync", "MemoryStoreSortedMap"),
    ("GetSizeAsync", "MemoryStoreSortedMap"),
    ("SetAsync", "MemoryStoreSortedMap"),
    ("UpdateAsync", "MemoryStoreSortedMap"),
    ("RemoveAsync", "MemoryStoreSortedMap"),
    ("GetAsync", "MemoryStoreHashMap"),
    ("ListItemsAsync", "MemoryStoreHashMap"),
    ("SetAsync", "MemoryStoreHashMap"),
    ("UpdateAsync", "MemoryStoreHashMap"),
    ("RemoveAsync", "MemoryStoreHashMap"),
    ("GetAsync", "MemoryStoreDistributedCounter"),
    ("IncrementAsync", "MemoryStoreDistributedCounter"),
];
pub const SOURCE: &str = r###"
local localMode, create, registerPages, now, encode = ...
local M=__rs_lua_methods
local handles, namespaces={},{}
local serial=0
local function copy(v, seen)
    if type(v)~='table' then
        if type(v)=='number' and (v~=v or math.abs(v)==math.huge) then error('InvalidRequest: Unable to convert value to json.',0) end
        if v~=nil and type(v)~='number' and type(v)~='string' and type(v)~='boolean' then error('InvalidRequest: Unable to convert value to json.',0) end
        return v
    end
    seen=seen or {}; if seen[v] then error('InvalidRequest: Unable to convert value to json.',0) end
    seen[v]=true;local out={}
    for k,x in pairs(v) do if type(k)~='string' and type(k)~='number' then error('InvalidRequest: Unable to convert value to json.',0) end;out[k]=copy(x,seen) end
    seen[v]=nil;return out
end
local function expiration(v)
    if type(v)~='number' or v~=v or v<0 or v>3888000 then error("InvalidExpirationTime: The field 'expiration' time must be between 0 and 3,888,000.",0) end
    return math.floor(v)
end
local function key(v)
    if type(v)~='string' then error('Unable to cast value to string',0) end
    if #v==0 then error('InvalidParameter: The key field is required.',0)end
    if #v>128 or not utf8.len(v)then error('InvalidParameter: The provided key is not valid.',0)end
    return v
end
local function purge(data)
    local t=now();for k,r in pairs(data) do if r.expires<=t then data[k]=nil end end
end
local function open(class,name,invisibility)
    if not localMode then
        if class=='MemoryStoreDistributedCounter' then error('MemoryStoreDistributedCounter is not enabled.',0) end
        if game.PlaceId==0 then error('You must publish this place to use MemoryStoreService.',0) end
        error('rustudio: session MemoryStore backend is not implemented',0)
    end
    if type(name)~='string'then error('Unable to cast value to string',0)end
    local namespace=class..':'..name
    local data=namespaces[namespace] or {};namespaces[namespace]=data
    local obj=create(class);handles[obj]={data=data,invisibility=invisibility or 30,class=class,name=name};return obj
end
local function count(v,max)
    if type(v)~='number' or v%1~=0 or v<1 or v>max then error('InvalidRequest: Invalid count.',0) end
    return v
end
local function wrap(class,name,fn)
    local prior=M[name]
    M[name]=function(self,...)
        if self.ClassName==class then return fn(self,...) end
        if prior then return prior(self,...) end
        error(name..' is not implemented for '..self.ClassName,0)
    end
end
wrap('MemoryStoreService','GetQueue',function(_,name,invisibility)
    if not localMode then return open('MemoryStoreQueue',name,invisibility) end
    if invisibility~=nil and (type(invisibility)~='number' or invisibility<0 or invisibility>43200 or invisibility~=invisibility) then error('InvalidRequest: Invalid invisibility timeout.',0) end
    return open('MemoryStoreQueue',name,invisibility)
end)
for method,class in pairs({GetSortedMap='MemoryStoreSortedMap',GetHashMap='MemoryStoreHashMap',GetDistributedCounter='MemoryStoreDistributedCounter'}) do
    wrap('MemoryStoreService',method,function(_,name)return open(class,name)end)
end
local tokens,lastBudget=1000,now()
local population={}
local function players()
    local ok,n=pcall(function()return #game:GetService('Players'):GetPlayers()end);return ok and n or 0
end
local function consume(n)
    local t=now();local limit=1000+120*players();tokens=math.min(limit,tokens+math.max(0,t-lastBudget)*limit/60);lastBudget=t
    if tokens<n then error('RequestThrottled: Recent MemoryStores requests hit one or more limits.',0)end;tokens-=n
end
local function storageLimit()
    local t=now();local n=players();local last=population[#population];if last and last.players==n then last.time=t else table.insert(population,{time=t,players=n})end;local peak=0
    for i=#population,1,-1 do local p=population[i];if t-p.time>8*86400 then table.remove(population,i)else peak=math.max(peak,p.players)end end
    return 64*1024+1.2*1024*peak
end
local function data(self)
    local h=handles[self];if not h then error('MemoryStore handle is unavailable',0)end
    if #h.name==0 then error('InvalidParameter: The name field is required.',0)end
    if #h.name>128 or not utf8.len(h.name)then error('InvalidParameter: The provided '..(h.class=='MemoryStoreHashMap'and'hash map'or h.class=='MemoryStoreSortedMap'and'sorted map'or'queue')..' name is not valid.',0)end
    purge(h.data);return h.data,h
end
local function valueBytes(v)
    local ok,serialized=pcall(encode,v)
    if not ok then error('InvalidRequest: Unable to convert value to json.',0)end
    if #serialized>32768 then error('ItemValueSizeTooLarge: The item value size is too large.',0)end
    return #serialized
end
local function reserve(d,k,size,class)
    local total,count,structure=0,0,0
    for _,namespace in namespaces do purge(namespace);for _,row in namespace do total+=row.bytes or 0 end end
    for _,row in d do count+=1;structure+=row.bytes or 0 end
    local previous=d[k]and d[k].bytes or 0
    local limit=math.floor(storageLimit());if size>previous and total-previous+size>limit then error('TotalMemoryOverLimit: Memory usage of '..(total-previous+size)..' bytes would exceed the quota of '..limit..' bytes',0)end
    if class~='MemoryStoreHashMap'then
        if not d[k]and count>=1000000 then error('DataStructureItemsOverLimit: The data structure item count limit has been reached.',0)end
        if structure-previous+size>100*1024*1024 then error('DataStructureMemoryOverLimit: The data structure memory size limit has been reached.',0)end
    end
end
local function sortKey(v)
    if v~=nil and type(v)~='string' and (type(v)~='number' or v~=v or math.abs(v)==math.huge) then error('InvalidRequest: Unable to convert sortKey to a valid number or string.',0) end
    return v
end
local function put(d,k,value,ttl,sk,class)
    local bytes=valueBytes(value);reserve(d,k,bytes,class)
    local previous=d[k];d[k]={value=copy(value),sortKey=sortKey(sk),expires=now()+expiration(ttl),bytes=bytes,revision=(previous and previous.revision or 0)+1}
end
for _,class in {'MemoryStoreHashMap','MemoryStoreSortedMap'} do
    local sorted=class=='MemoryStoreSortedMap'
    wrap(class,'GetAsync',function(self,k) key(k);consume(1);task.wait();local d=data(self);local r=d[k];if not r then return nil end;if sorted then return copy(r.value),r.sortKey end;return copy(r.value) end)
    wrap(class,'SetAsync',function(self,k,v,ttl,sk) key(k);expiration(ttl);local value=copy(v);sortKey(sk);consume(1);task.wait();local d=data(self);local existed=d[k]~=nil;put(d,k,value,ttl,sorted and sk or nil,class);return existed end)
    wrap(class,'RemoveAsync',function(self,k) key(k);consume(1);task.wait();local d=data(self);d[k]=nil end)
    wrap(class,'UpdateAsync',function(self,k,fn,ttl)
        key(k);expiration(ttl);if type(fn)~='function' then error('Unable to cast value to Function',0) end;consume(1);task.wait()
        while true do
            local d=data(self);local before=d[k];local ok,v,sk=pcall(fn,before and copy(before.value),before and before.sortKey);if not ok then error('TransformCallbackFailed: Failed to invoke transformation function.',0)end
            if v==nil then return nil end
            consume(1);if d[k]==before then put(d,k,v,ttl,sorted and sk or nil,class);if sorted then return copy(v),sk end;return copy(v) end
        end
    end)
end
local function less(a,b)
    local function rank(v) if type(v)=='number' then return 1 elseif type(v)=='string' then return 2 else return 3 end end
    local x,y=rank(a.sortKey),rank(b.sortKey)
    if x~=y then return x<y end
    if a.sortKey~=b.sortKey then return a.sortKey<b.sortKey end
    return a.key<b.key
end
wrap('MemoryStoreSortedMap','GetRangeAsync',function(self,direction,n,lower,upper)
    count(n,200);task.wait();local d=data(self);local rows={}
    local function bound(row,b,isLower)
        if b==nil then return true end
        if type(b)=='string' then b={key=b} end
        if type(b)~='table' then error('InvalidRequest: Invalid bound.',0) end
        if b.key==nil then
            if row.sortKey==b.sortKey then return false end
            local edge={key='',sortKey=b.sortKey};return isLower and less(edge,row) or not isLower and less(row,edge)
        end
        local edge={key=b.key,sortKey=b.sortKey};return isLower and less(edge,row) or not isLower and less(row,edge)
    end
    for k,r in pairs(d) do local row={key=k,value=copy(r.value),sortKey=r.sortKey};if bound(row,lower,true) and bound(row,upper,false) then table.insert(rows,row) end end
    local desc=(type(direction)=='string' and direction=='Descending') or (typeof(direction)=='EnumItem' and direction.Name=='Descending')
    table.sort(rows,function(a,b) if desc then return less(b,a) else return less(a,b) end end)
    while #rows>n do table.remove(rows) end;consume(math.max(1,#rows));return rows
end)
for _,class in {'MemoryStoreQueue','MemoryStoreSortedMap'} do
    wrap(class,'GetSizeAsync',function(self,excludeInvisible)consume(1);task.wait();local d=data(self);local n=0;for _,r in pairs(d) do if not excludeInvisible or not r.invisibleUntil or r.invisibleUntil<=now() then n+=1 end end;return n end)
end
wrap('MemoryStoreHashMap','ListItemsAsync',function(self,n)
    count(n,200);task.wait();local d=data(self);local rows={};for k,r in pairs(d) do table.insert(rows,{key=k,value=copy(r.value)})end
    table.sort(rows,function(a,b)return a.key<b.key end)
    local pages={};for i=1,#rows,n do local page={};for j=i,math.min(i+n-1,#rows) do table.insert(page,rows[j]) end;table.insert(pages,page) end
    consume(1+math.min(n,#rows));local obj=create('MemoryStoreHashMapPages');registerPages(obj,pages,function(index)consume(1+#pages[index])end);return obj
end)
wrap('MemoryStoreQueue','AddAsync',function(self,v,ttl,priority)
    expiration(ttl);priority=priority or 0;if type(priority)~='number' or priority~=priority or math.abs(priority)==math.huge then error('InvalidRequest: Invalid priority.',0) end
    local value=copy(v);consume(1);task.wait();local d=data(self);serial+=1;local bytes=valueBytes(value);reserve(d,serial,bytes,'MemoryStoreQueue');d[serial]={value=value,bytes=bytes,priority=priority,sequence=serial,expires=now()+expiration(ttl),invisibleUntil=0}
end)
wrap('MemoryStoreQueue','ReadAsync',function(self,n,allOrNothing,timeout)
    count(n,100);timeout=timeout or -1;if type(timeout)~='number' or timeout~=timeout or timeout< -1 then error('InvalidRequest: Invalid wait timeout.',0) end
    local deadline=timeout<0 and math.huge or now()+timeout;local lastPoll=now();consume(1);task.wait()
    while true do
        local d,h=data(self);local rows={};for _,r in pairs(d) do if r.invisibleUntil<=now() then table.insert(rows,r) end end
        table.sort(rows,function(a,b)if a.priority==b.priority then return a.sequence<b.sequence end;return a.priority>b.priority end)
        if #rows>0 and (not allOrNothing or #rows>=n) then
            consume(math.max(0,math.min(n,#rows)-1));serial+=1;local receipt=tostring(serial);local values={}
            for i=1,math.min(n,#rows) do local r=rows[i];r.invisibleUntil=now()+h.invisibility;r.receipt=receipt;table.insert(values,copy(r.value)) end
            return values,receipt
        end
        if now()>=deadline then return {},nil end;task.wait();if now()-lastPoll>=2 then consume(1);lastPoll=now()end
    end
end)
wrap('MemoryStoreQueue','RemoveAsync',function(self,receipt)
    if type(receipt)~='string' then error('Unable to cast value to string',0) end
    consume(1);task.wait();local d=data(self);for k,r in pairs(d) do if r.receipt==receipt and r.invisibleUntil>now() then d[k]=nil end end
end)
wrap('MemoryStoreDistributedCounter','GetAsync',function(self)consume(1);task.wait();local d=data(self);return d.counter and d.counter.value or 0 end)
wrap('MemoryStoreDistributedCounter','IncrementAsync',function(self,delta,ttl)
    expiration(ttl);if type(delta)~='number' or delta%1~=0 then error('Unable to cast value to int64',0) end
    consume(1);task.wait();local d=data(self);local v=(d.counter and d.counter.value or 0)+delta;reserve(d,'counter',8,'MemoryStoreDistributedCounter');d.counter={value=v,bytes=8,expires=now()+expiration(ttl)};return v
end)
"###;

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::{Function, Lua};
    fn setup(local: bool) -> Lua {
        let lua = Lua::new();
        lua.load(r#"
__rs_lua_methods={}; game={PlaceId=0};clock=0;task={wait=function()end}
function create(class)return setmetatable({ClassName=class},{__index=function(_,k)return __rs_lua_methods[k]end})end
function register(obj,pages)obj.pages=pages end
function now()return clock end
service=create('MemoryStoreService')
"#).exec().unwrap();
        lua.load(SOURCE)
            .call::<()>((
                local,
                lua.globals().get::<Function>("create").unwrap(),
                lua.globals().get::<Function>("register").unwrap(),
                lua.globals().get::<Function>("now").unwrap(),
                lua.create_function(|_, v: mlua::Value| {
                    Ok(match v {
                        mlua::Value::String(s) => {
                            serde_json::to_string(s.to_str()?.as_ref()).unwrap()
                        }
                        mlua::Value::Integer(n) => n.to_string(),
                        mlua::Value::Number(n) => n.to_string(),
                        mlua::Value::Boolean(b) => b.to_string(),
                        _ => "{}".into(),
                    })
                })
                .unwrap(),
            ))
            .unwrap();
        lua
    }
    #[test]
    fn memory_unpublished_errors() {
        let lua = setup(false);
        lua.load(r#"
for _,m in {'GetQueue','GetSortedMap','GetHashMap'} do local ok,e=pcall(service[m],service,'x');assert(not ok and e=='You must publish this place to use MemoryStoreService.')end
local ok,e=pcall(service.GetDistributedCounter,service,'x');assert(not ok and e=='MemoryStoreDistributedCounter is not enabled.')
"#).exec().unwrap();
    }
    #[test]
    fn memory_map_expiry_order_and_retry() {
        let lua = setup(true);
        lua.load(r#"
local h=service:GetHashMap('x');local s=service:GetSortedMap('x')
h:SetAsync('a',{n=1},10);local v=h:GetAsync('a');v.n=9;assert(h:GetAsync('a').n==1)
assert(s:GetAsync('a')==nil)
s:SetAsync('b',2,10,1);s:SetAsync('a',1,10,1);s:SetAsync('c',3,10,'first');s:SetAsync('d',4,10)
local r=s:GetRangeAsync('Ascending',200);assert(r[1].key=='a' and r[2].key=='b' and r[3].key=='c' and r[4].key=='d')
r=s:GetRangeAsync('Descending',2);assert(r[1].key=='d' and r[2].key=='c')
r=s:GetRangeAsync('Ascending',200,{sortKey=1});assert(#r==2 and r[1].key=='c')
local calls=0;h:UpdateAsync('a',function(old)calls+=1;if calls==1 then h:SetAsync('a',{n=3},10)end;return {n=old.n+1}end,10)
assert(calls==2 and h:GetAsync('a').n==4)
h:UpdateAsync('a',function()return nil end,10);assert(h:GetAsync('a').n==4)
assert(#h:ListItemsAsync(1).pages==1)
clock=10;assert(h:GetAsync('a')==nil and s:GetSizeAsync()==0)
local c=service:GetDistributedCounter('x');assert(c:GetAsync()==0);assert(c:IncrementAsync(2,3)==2);assert(c:IncrementAsync(-1,3)==1);clock=13;assert(c:GetAsync()==0)
assert(not pcall(function()h:SetAsync('x',{},3888001)end))
"#).exec().unwrap();
    }
    #[test]
    fn memory_queue_receipts_and_expiry() {
        let lua = setup(true);
        lua.load(r#"
local q=service:GetQueue('q',3);q:AddAsync('low',20,0);q:AddAsync('first',20,5);q:AddAsync('second',20,5)
local items,id=q:ReadAsync(2,false,0);assert(items[1]=='first' and items[2]=='second');assert(q:GetSizeAsync()==3 and q:GetSizeAsync(true)==1)
local none=q:ReadAsync(2,true,0);assert(#none==0 and q:GetSizeAsync(true)==1)
clock=3;local retry,newid=q:ReadAsync(2,false,0);assert(retry[1]=='first' and newid~=id)
q:RemoveAsync(id);assert(q:GetSizeAsync()==3);q:RemoveAsync(newid);assert(q:GetSizeAsync()==1)
clock=20;assert(q:GetSizeAsync()==0);local empty,receipt=q:ReadAsync(1,false,0);assert(#empty==0 and receipt==nil)
"#).exec().unwrap();
    }
    #[test]
    fn memory_value_capacity_release_and_request_quotas() {
        let lua = setup(true);
        lua.load(r#"
local h=service:GetHashMap('quota')
assert(h:SetAsync('a',string.rep('x',32760),100)==false)
assert(h:SetAsync('b',string.rep('x',32760),100)==false)
local ok,e=pcall(h.SetAsync,h,'oversize',string.rep('x',32767),100);assert(not ok and e=='ItemValueSizeTooLarge: The item value size is too large.')
local ok,e=pcall(h.SetAsync,h,'c',string.rep('x',30),100);assert(not ok and string.find(e,'TotalMemoryOverLimit',1,true));assert(h:GetAsync('c')==nil)
assert(h:SetAsync('a','small',100)==true);assert(h:SetAsync('c',string.rep('x',1000),100)==false)
h:RemoveAsync('b');assert(h:SetAsync('d',string.rep('x',32760),1)==false)
clock=1;assert(h:GetAsync('d')==nil);assert(h:SetAsync('e',string.rep('x',32760),100)==false)
"#).exec().unwrap();
        let lua = setup(true);
        lua.load(r#"
local h=service:GetHashMap('requests');for i=1,1000 do assert(h:GetAsync('missing')==nil)end
local ok,e=pcall(h.GetAsync,h,'missing');assert(not ok and e=='RequestThrottled: Recent MemoryStores requests hit one or more limits.')
clock=0.061;assert(h:GetAsync('missing')==nil);assert(not pcall(h.GetAsync,h,'missing'))
clock=61;assert(h:UpdateAsync('k',function()return 1 end,100)==1)
for i=1,998 do h:GetAsync('missing')end;assert(not pcall(h.GetAsync,h,'missing'))
"#).exec().unwrap();
    }
    #[test]
    fn memory_capacity_tracks_eight_day_population_peak() {
        let lua = setup(true);
        lua.load(r#"
local people={};function game:GetService()return {GetPlayers=function()return people end}end
for i=1,100 do people[i]=i end
local h=service:GetHashMap('population');for i=1,4 do h:SetAsync(tostring(i),string.rep('x',32760),3888000)end
people={};clock=1;h:SetAsync('kept','yes',3888000)
clock=8*86400+2;local ok,e=pcall(h.SetAsync,h,'growth','more',3888000);assert(not ok and string.find(e,'TotalMemoryOverLimit',1,true))
assert(h:SetAsync('1','shrunk',3888000)==true);h:RemoveAsync('2');h:RemoveAsync('3');assert(h:SetAsync('growth','more',3888000)==false)
"#).exec().unwrap();
    }
}
