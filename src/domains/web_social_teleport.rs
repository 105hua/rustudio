//! Studio local teleport failures. Native failure handler ,
//! failure event builder . Reservation POST tests use localhost only.
use super::*;
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let ctx = lua.create_table()?;
    ctx.set(
        "peer",
        lua.create_function(|lua, ()| Ok(world(lua)?.has_peer))?,
    )?;
    ctx.set(
        "emit",
        lua.create_function(|lua, args: MultiValue| {
            let r = args
                .front()
                .and_then(opt_inst)
                .ok_or_else(|| rt_err("Instance expected"))?;
            world(lua)?.fire(
                r,
                "TeleportInitFailed",
                args.iter().skip(1).cloned().collect(),
            );
            Ok(())
        })?,
    )?;
    ctx.set("report",lua.create_function(|lua,()| {
        world(lua)?.output.push((Level::Error,"raiseTeleportInitFailedEvent: Teleport failed because Cannot Teleport in the Roblox Studio. (Failure)".into()));Ok(())
    })?)?;
    ctx.set("reserve_begin",lua.create_function(|lua,place:i64| {
        http::background(lua,move |session,_scope| rustudio_account::web_social::reserve_server(&session,place).map_err(|e|e.to_string()))
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let module: Table = lua
        .load(SOURCE)
        .set_name("=rustudio.web_social.teleport")
        .call(ctx)?;
    let target: Table = lua.globals().get("__rs_lua_methods")?;
    for pair in module
        .get::<Table>("methods")?
        .pairs::<String, mlua::Function>()
    {
        let (k, v) = pair?;
        target.set(k, v)?;
    }
    lua.set_named_registry_value("web_social.teleport_module", module)
}
pub const SOURCE: &str = r#"
local ctx=...;local module={methods={}}
local function is_client()return game:GetService('RunService'):IsClient()end
local function reserve(service,id)
    id=tonumber(id);if not id then error('Argument 1 missing or nil',0)end
    if is_client() then error('TeleportService:ReserveServer can only be called by the server.',0)end
    local token=ctx.reserve_begin(id<0 and math.ceil(id) or math.floor(id))
    while true do
        local done,body,err=ctx.poll(token)
        if done then
            if err then error(err,0)end
            local v=game:GetService('HttpService'):JSONDecode(body)
            return v.ReservedServerAccessCode,v.ReservedServerGameCode
        end
        task.wait()
    end
end
module.methods.ReserveServer=reserve
module.methods.ReserveServerAsync=reserve
local function player_arg(player)
    if player==nil and is_client() then player=game:GetService('Players').LocalPlayer end
    if typeof(player)~='Instance' or not player:IsA('Player') then error('Argument 2 missing or nil',0)end
    return player
end
local function fail(service,player,id,data,instance,access)
    local options=Instance.new('TeleportOptions')
    if data~=nil then options:SetTeleportData(data)end
    options.ServerInstanceId=instance or '';options.ReservedServerAccessCode=access or ''
    ctx.emit(service,player,Enum.TeleportResult.Failure,'Cannot Teleport in the Roblox Studio.',id,options)
    -- Resume subscribers before the caller receives the native Studio error.
    if not ctx.test then task.wait()end
    ctx.report()
    error('Cannot Teleport in the Roblox Studio.',0)
end
local function perform(service,id,player,data,instance,access)
    if not is_client() and ctx.peer() then
        __ws_send(service,'__teleport','request',player,id,data,instance or '',access or '')
        return
    end
    return fail(service,player,id,data,instance,access)
end
function module.receive(_,sender,verb,player,id,data,instance,access)
    if verb~='request' or not is_client() then return end
    if player~=game:GetService('Players').LocalPlayer then return end
    -- The sending server already returned from its request. A client's local
    -- failure is reported as an engine error, not as an OnClientEvent payload.
    task.spawn(function()pcall(fail,game:GetService('TeleportService'),player,id,data,instance,access)end)
end
local function place(id)
    id=tonumber(id);if not id then error('Argument 1 missing or nil',0)end
    if id<=0 then
        if is_client() then warn('Cannot teleport to invalid place id. Aborting teleport.');error('Cannot teleport to invalid place id. Aborting teleport.',0)end
        error('Invalid placeId.',0)
    end
    return math.floor(id)
end
module.methods.Teleport=function(service,id,player,data,gui)
    return perform(service,place(id),player_arg(player),data)
end
module.methods.TeleportToSpawnByName=function(service,id,spawn,player,data,gui)
    if type(spawn)~='string' then error('Argument 2 missing or nil',0)end
    return perform(service,place(id),player_arg(player),data)
end
module.methods.TeleportToPlaceInstance=function(service,id,instance,player,spawn,data,gui)
    if type(instance)~='string' then error('Argument 2 missing or nil',0)end
    if instance=='' then warn('Cannot teleport to empty instance id. Aborting teleport.');error('Cannot teleport to empty instance id. Aborting teleport.',0)end
    return perform(service,place(id),player_arg(player),data,instance)
end
module.methods.TeleportToPrivateServer=function(service,id,access,players,spawn,data,gui)
    if is_client() then error('TeleportToPrivateServer can only be called from the server',0)end
    id=place(id)
    if type(access)~='string' then error('Argument 2 missing or nil',0)end
    if type(players)~='table' then error('Argument 3 missing or nil',0)end
    if #players==0 then error('Invalid list of players for teleport.',0)end
    for _,player in players do perform(service,id,player_arg(player),data,nil,access)end
end
return module
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserve_aliases_yield_two_native_strings_and_propagate_errors() -> mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local client=false;local ready=false;local count=0;local failure
local http={JSONDecode=function()return {ReservedServerAccessCode='access',ReservedServerGameCode='private'}end}
game={GetService=function(_,name)if name=='RunService'then return {IsClient=function()return client end}end;assert(name=='HttpService');return http end}
task={wait=function()count+=1;ready=true end}
local module=assert(loadstring(source))({reserve_begin=function(id)assert(id==7);ready=false;return 1 end,poll=function(token)assert(token==1);return ready,'{}',failure end})
for _,name in {'ReserveServer','ReserveServerAsync'}do
 local code,id=module.methods[name]({},7);assert(code=='access' and id=='private')
end
assert(count==2)
failure='HTTP 429 (Too Many Requests)';local ok,e=pcall(module.methods.ReserveServerAsync,{},7);assert(not ok and e==failure)
client=true;local before=count;local ok,e=pcall(module.methods.ReserveServerAsync,{},7);assert(not ok and e=='TeleportService:ReserveServer can only be called by the server.' and count==before)
"#).exec()
    }
    #[test]
    fn client_failure_emits_options_before_error() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("source", SOURCE)?;
        lua.load(r#"
local log={};local p={};local service={};p.IsA=function()return true end
local original_typeof=typeof;typeof=function(x)if x==p then return 'Instance'end;return original_typeof(x)end
local options;Instance={new=function(class)assert(class=='TeleportOptions');options={SetTeleportData=function(self,data)self.data=table.clone(data)end};return options end}
Enum={TeleportResult={Failure='Failure'}}
game={GetService=function(_,name)if name=='RunService' then return {IsClient=function()return true end}end;return {LocalPlayer=p}end}
local module=assert(loadstring(source))({test=true,peer=function()return true end,emit=function(s,who,result,error,id,o)table.insert(log,{s,who,result,error,id,o});end,report=function()table.insert(log,'report')end})
local data={x=7};local ok,error=pcall(module.methods.TeleportToPlaceInstance,service,10000000000008,'instance',p,'',data)
assert(not ok and error=='Cannot Teleport in the Roblox Studio.')
assert(#log==2 and log[1][1]==service and log[1][2]==p and log[1][3]=='Failure' and log[1][5]==10000000000008)
assert(options.ServerInstanceId=='instance' and options.ReservedServerAccessCode=='' and options.data.x==7)
data.x=9;assert(options.data.x==7)
"#).exec()
    }
}
