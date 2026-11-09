//! Config testing snapshots, plus directly observed feature-disabled service
//! gates. PlayerData/StandardQueue records cannot be obtained in this Studio.
pub const LUA_METHODS: &[(&str, &str)] = &[
    ("GetRecordConfig", "PlayerDataService"),
    ("GetStandardQueue", "QueueService"),
    ("SetTestingValue", "ConfigService"),
    ("ClearTestingValue", "ConfigService"),
    ("GetConfigAsync", "ConfigService"),
    ("GetConfigForPlayerAsync", "ConfigService"),
    ("GetValue", "ConfigSnapshot"),
    ("GetValueChangedSignal", "ConfigSnapshot"),
    ("Refresh", "ConfigSnapshot"),
];
pub const SOURCE: &str = r###"
local localMode,create,registerProps,fire,report,isServer,errorNone = ...
local M=__rs_lua_methods
local testing,snapshots={},{}
local function copy(v,seen)
    local ty=type(v)
    if ty=='nil' or ty=='boolean' or ty=='string' then return v end
    if ty=='number' and v==v and math.abs(v)~=math.huge then return v end
    if ty~='table' then error('unsupported',0) end
    seen=seen or {};if seen[v]then error('unsupported',0)end;seen[v]=true
    local out={};for k,x in pairs(v)do if type(k)~='number' and type(k)~='string'then error('unsupported',0)end;out[k]=copy(x,seen)end
    seen[v]=nil;return out
end
local function equal(a,b)
    if type(a)~=type(b)then return false end;if type(a)~='table'then return a==b end
    for k,v in pairs(a)do if not equal(v,b[k])then return false end end
    for k in pairs(b)do if a[k]==nil then return false end end;return true
end
local function key(k)if k==nil then error('Argument 1 missing or nil',0)end;return tostring(k)end
local function wrap(class,name,fn)
 local prior=M[name];M[name]=function(self,...)if self.ClassName==class then return fn(self,...)end;if prior then return prior(self,...)end;error('Unsupported '..name..' receiver',0)end
end
local function changed()
 for obj,s in pairs(snapshots)do s.props.Outdated=true;fire(obj,'UpdateAvailable',{})end
end
wrap('PlayerDataService','GetRecordConfig',function()error('PlayerDataService:GetRecordConfig(string? recordName) is not yet enabled!',0)end)
wrap('QueueService','GetStandardQueue',function(_,name)key(name);error('QueueService is not enabled.',0)end)
wrap('ConfigService','SetTestingValue',function(_,k,v)
 if not isServer()then error('ConfigService is only available in the game server.',0)end
 k=key(k);local ok,value=pcall(copy,v)
 if not ok then local t=typeof(v);t=string.upper(string.sub(t,1,1))..string.sub(t,2);report('ConfigService: Testing value type '..t..' for key '..k..' is unsupported.');return end
 if not equal(testing[k],value)then testing[k]=value;changed()end
end)
wrap('ConfigService','ClearTestingValue',function(_,k)if not isServer()then error('ConfigService is only available in the game server.',0)end;k=key(k);if testing[k]~=nil then testing[k]=nil;changed()end end)
local function snapshot()
 if not isServer()then error('ConfigService is only available in the game server.',0)end
 task.wait()
 if not localMode then error('ConfigService: Failed to load config',0)end
 local obj=create('ConfigSnapshot');local props={Error=errorNone,Outdated=false}
 snapshots[obj]={values=copy(testing),signals={},props=props};registerProps(obj,props);return obj
end
wrap('ConfigService','GetConfigAsync',function()return snapshot()end)
wrap('ConfigService','GetConfigForPlayerAsync',function(_,player)
 if player==nil then error('Argument 1 missing or nil',0)end
 if typeof(player)~='Instance' or not player:IsA('Player')then error('Unable to cast value to Player',0)end
 return snapshot()
end)
local function state(obj)local s=snapshots[obj];if not s then error('ConfigSnapshot has no loaded configuration',0)end;return s end
wrap('ConfigSnapshot','GetValue',function(self,k)return copy(state(self).values[key(k)])end)
wrap('ConfigSnapshot','GetValueChangedSignal',function(self,k)
 k=key(k);local s=state(self);if not s.signals[k]then s.signals[k]=create('BindableEvent')end;return s.signals[k].Event
end)
wrap('ConfigSnapshot','Refresh',function(self)
 local s=state(self);local old=s.values;s.values=copy(testing);s.props.Outdated=false
 local keys={};for k in pairs(old)do keys[k]=true end;for k in pairs(s.values)do keys[k]=true end
 for k in pairs(keys)do if not equal(old[k],s.values[k])and s.signals[k]then s.signals[k]:Fire(copy(s.values[k]))end end
end)
"###;
#[cfg(test)]
mod tests {
    use super::*;
    use mlua::{Function, Lua};
    #[test]
    fn testing_snapshots_refresh_and_disabled_services() {
        let lua = Lua::new();
        lua.load(r#"
__rs_lua_methods={};task={wait=function()end};events={};errors={}
function create(class)
 local obj={ClassName=class};setmetatable(obj,{__index=function(_,k)return __rs_lua_methods[k]end})
 if class=='BindableEvent'then local listeners={};obj.Event={Connect=function(_,fn)table.insert(listeners,fn)end};function obj:Fire(v)for _,fn in listeners do fn(v)end end end;return obj
end
function register(obj,props)obj.props=props end
function fire(obj,event)table.insert(events,{obj,event})end
function report(e)table.insert(errors,e)end
function server()return true end
service=create('ConfigService')
"#).exec().unwrap();
        let f = |n| lua.globals().get::<Function>(n).unwrap();
        lua.load(SOURCE)
            .call::<()>((
                true,
                f("create"),
                f("register"),
                f("fire"),
                f("report"),
                f("server"),
                "None",
            ))
            .unwrap();
        lua.load(r#"
service:SetTestingValue('a',{x=1});local s=service:GetConfigAsync();assert(s.props.Error=='None' and not s.props.Outdated)
local v=s:GetValue('a');v.x=7;assert(s:GetValue('a').x==1)
local hits=0;s:GetValueChangedSignal('a'):Connect(function(v)hits+=1;assert(v.x==2)end)
service:SetTestingValue('a',{x=2});assert(s.props.Outdated and #events==1 and hits==0 and s:GetValue('a').x==1)
s:Refresh();assert(not s.props.Outdated and hits==1 and s:GetValue('a').x==2);s:Refresh();assert(hits==1)
service:SetTestingValue('a',{x=2});assert(#events==1)
service:SetTestingValue('bad',function()end);assert(#errors==1)
local s2=service:GetConfigAsync();service:ClearTestingValue('a');assert(s2.props.Outdated);s2:Refresh();assert(s2:GetValue('a')==nil)
local ok,e=pcall(function()create('PlayerDataService'):GetRecordConfig()end);assert(not ok and e=='PlayerDataService:GetRecordConfig(string? recordName) is not yet enabled!')
local ok,e=pcall(function()create('QueueService'):GetStandardQueue('x')end);assert(not ok and e=='QueueService is not enabled.')
local ok,e=pcall(function()create('QueueService'):GetStandardQueue()end);assert(not ok and e=='Argument 1 missing or nil')
"#).exec().unwrap();
    }
}
