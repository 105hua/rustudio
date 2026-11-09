//! Native PromptExperienceDetailsAsync plus shipped
//! InExperienceDetailsPromptApp: completion precedes its legacy Teleport call.
//! Native verification opens metadata dialogs and dismisses X only.
use super::*;
pub fn install(lua:&Lua)->mlua::Result<()> {
    let ctx=lua.create_table()?;
    ctx.set("client",lua.create_function(|lua,()|Ok(world(lua)?.side==Side::Client))?)?;
    ctx.set("begin",lua.create_function(|lua,(kind,id):(String,i64)| {
        http::background(lua,move|session,scope| {
            use rustudio_account::web_social::Read;
            let read=match kind.as_str(){"info"=>Read::ExperienceInfo(id),"playability"=>Read::ExperiencePlayability(id),"votes"=>Read::ExperienceVotes(id),"age"=>Read::ExperienceAgeRecommendation(id),_=>return Err("Unknown experience detail request".into())};
            read.fetch_in(&session,&scope).map_err(|e|e.to_string())
        })
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let module:Table=lua.load(SOURCE).set_name("=rustudio.web_social.details").call(ctx)?;
    lua.globals().get::<Table>("__rs_lua_methods")?.set("PromptExperienceDetailsAsync",module.get::<mlua::Function>("prompt")?)?;
    lua.set_named_registry_value("web_social.details",module)
}
const SOURCE:&str=r#"
local ctx=...;local module={};local pending={};local active;local cache={}
local function clock()return ctx.clock and ctx.clock()or os.clock()end
local function spawn(f)if ctx.spawn then return ctx.spawn(f)end;task.spawn(f)end
local function wait()if ctx.wait then return ctx.wait()end;return task.wait()end
local function client()if ctx.client then return ctx.client()end;return game:GetService('RunService'):IsClient()end
local function local_player()return ctx.player and ctx.player()or game:GetService('Players').LocalPlayer end
local function enum(name)return ctx.enum and ctx.enum(name)or Enum.PromptExperienceDetailsResult[name]end
local function request(kind,id)
    if ctx.request then return ctx.request(kind,id)end
    local token=ctx.begin(kind,id)
    while true do local done,body,err=ctx.poll(token);if done then if err then error(err,0)end;return game:GetService('HttpService'):JSONDecode(body)end;wait()end
end
local function fetch(kind,id)
    local key=kind..':'..id;local record=cache[key]
    if record and(record.pending or clock()<record.expires)then
        while record.pending do wait()end;return record.ok,record.value
    end
    record={pending=true};cache[key]=record
    record.ok,record.value=pcall(function()
        local value=request(kind,id)
        if type(value)~='table'or((kind=='info'or kind=='votes')and type(value.data)~='table')then error('Invalid experience metadata response',0)end
        if kind=='playability'then for _,item in value do if type(item)~='table'or type(item.isPlayable)~='boolean'or type(item.playabilityStatus)~='string'then error('Invalid playability response',0)end end end
        return value
    end);record.pending=false
    -- Measured current Client values: playability/info refresh30s, default
    -- metadata3600s, failures60s. These differ from the shipped1800s default.
    record.expires=clock()+(record.ok and((kind=='info'or kind=='playability')and 30 or 3600)or 60)
    return record.ok,record.value
end
local function complete(r,status)
    if not r.done then r.done=true;r.result=enum(status);if pending[r.player]==r then pending[r.player]=nil end end
end
local function destroy(r)
    if r.gui then r.gui:Destroy();r.gui=nil end
    if ctx.destroy then ctx.destroy(r)end
    if active==r then active=nil end
end
function module.close(r)
    r=r or active;if not r then return end
    complete(r,'PromptClosed');destroy(r)
end
function module.join(r)
    r=r or active;if not r then return end
    local root=r.info and tonumber(r.info.rootPlaceId)
    if not root then return end
    complete(r,'TeleportAttempted')
    -- This is deliberately not pcall: native Studio's Teleport error leaves
    -- the sheet open after completing the waiting API with TeleportAttempted.
    if ctx.teleport then ctx.teleport(root)else game:GetService('TeleportService'):Teleport(root)end
    destroy(r)
end
local messages={AccountRestricted='This experience is not accessible due to your content maturity settings',ContextualPlayabilityAgeGated='This experience is not accessible due to age restrictions',ContextualPlayabilityAgeGatedByDescriptor='This experience is not accessible due to age restrictions',ContextualPlayabilityAgeRecommendationParentalControls='To join this experience, update your content maturity setting.',ContextualPlayabilityExperienceBlockedParentalControls='Your parent blocked this experience. To join this experience, ask your parent for permission in Settings.',ContextualPlayabilityRegionalAvailability='This experience is currently not available in your region',ContextualPlayabilityRegionalCompliance='This experience is currently not available in your region',ContextualPlayabilityRequireParentApproval='Ask your parent to unlock this game for you.',ContextualPlayabilityUnrated='This experience is not accessible because it is unrated',DeviceRestrictedDefault='This experience is not available on your device',FiatPurchaseDeviceRestricted='This experience is not purchasable on this device',InsufficientPermissionFriendsOnly='You do not currently have access to this experience',InsufficientPermissionGroupOnly='You do not currently have access to this experience',Moderated='This game is no longer playable',UnderReview='This experience is currently under review',UniverseRootPlaceIsPrivate='This experience is set as private'}
local function update(r)
    if active~=r then return end
    if ctx.present then return ctx.present(r,module.close,module.join)end
    if not r.gui then
        local gui=Instance.new('ScreenGui');gui.Name='InExperienceDetailsPromptApp';gui.DisplayOrder=1000;gui.ResetOnSpawn=false;r.gui=gui
        local f=Instance.new('Frame');f.Name='Sheet';f.Size=UDim2.fromOffset(540,400);f.Position=UDim2.new(.5,-270,.5,-200);f.BackgroundColor3=Color3.fromRGB(35,37,39);f.Parent=gui
        local function label(name,y,h,size)
            local l=Instance.new('TextLabel');l.Name=name;l.Size=UDim2.new(1,-40,0,h);l.Position=UDim2.fromOffset(20,y);l.Text='';l.TextSize=size;l.TextWrapped=true;l.TextColor3=Color3.new(1,1,1);l.BackgroundTransparency=1;l.Parent=f;return l
        end
        r.title=label('Title',18,48,24);r.creator=label('CreatorName',68,28,18);r.age=label('AgeRecommendation',104,28,16);r.votes=label('Rating',140,24,16);r.description=label('Description',175,105,18);r.status=label('UnplayableError',292,40,16)
        local x=Instance.new('TextButton');x.Name='CloseAffordance';x.Text='×';x.Size=UDim2.fromOffset(32,32);x.Position=UDim2.new(1,-40,0,4);x.Activated:Connect(function()module.close(r)end);x.Parent=f
        local b=Instance.new('TextButton');b.Name='PlayButton';b.TextSize=18;b.Modal=true;b.Size=UDim2.new(1,-40,0,40);b.Position=UDim2.fromOffset(20,344);b.Activated:Connect(function()if r.playability and r.playability.isPlayable then module.join(r)end end);b.Parent=f;r.button=b
        gui.Parent=r.player:FindFirstChild('PlayerGui')or game:GetService('CoreGui')
    end
    local info=r.info
    local error_text=r.info_error and'No internet connection'or(info and info.isUnavailable and'This experience cannot be visited at this time.'or nil)
    r.title.Text=error_text and''or(info and info.name or'')
    r.creator.Text=not error_text and info and info.creator and info.creator.name or''
    r.description.Text=error_text or(info and info.description or'')
    local age=r.age_data and r.age_data.ageRecommendationDetails
    age=age and age.summary and age.summary.ageRecommendation
    r.age.Text=not error_text and age and(age.displayNameWithHeaderShort or age.displayName)or''
    local votes=r.vote_data;local up=votes and votes.upVotes or 0;local down=votes and votes.downVotes or 0
    r.votes.Text=error_text and''or(up+down>0 and(tostring(math.floor(100*up/(up+down)))..'%')or'--')
    r.button.Visible=not error_text
    local play=r.playability
    r.button.Text=play and play.isPlayable and'Join'or'Unavailable'
    r.status.Text=not error_text and play and not play.isPlayable and(messages[play.playabilityStatus]or'This experience is currently not available')or''
end
local function first(data,id,field)
    if type(data)~='table'then return nil end
    if field then data=data[field]end
    if type(data)~='table'then return nil end
    for _,item in data do if type(item)=='table'and tostring(item.id or item.universeId)==tostring(id)then return item end end
end
function module.prompt(service,player,id)
    if player==nil then error('Argument 1 missing or nil',0)end
    if not(typeof(player)=='Instance'and player:IsA('Player'))and not(ctx.isplayer and ctx.isplayer(player))then
        error(typeof(player)=='Instance'and"Argument 1 is invalid: expected 'Player' instance type"or'Unable to cast value to Object',0)
    end
    if id==nil then error('Argument 2 missing or nil',0)end
    if type(id)~='number'and type(id)~='string'then error('Unable to cast '..(type(id)=='boolean'and'bool'or typeof(id))..' to int64',0)end
    id=tonumber(id);if not id then error('Unable to cast string to int64',0)end;id=id<0 and math.ceil(id)or math.floor(id)
    if not client()then error('TeleportService::PromptExperienceDetailsAsync can only be called on the client',0)end
    if player~=local_player()then error('TeleportService::PromptExperienceDetailsAsync: player must be local player',0)end
    if pending[player]then error('TeleportService::PromptExperienceDetailsAsync: already has prompt in progress for this Player',0)end
    local r={player=player,id=id};pending[player]=r
    if active then destroy(active)end;active=r;update(r)
    for _,kind in {'info','playability','age','votes'}do
        spawn(function()
            local ok,value=fetch(kind,id)
            if kind=='info'then
                r.info_error=not ok;r.info=ok and first(value,id,'data')or nil
                if ok and not r.info then r.info={isUnavailable=true}
                elseif r.info and type(r.info.name)=='string'then r.info=table.clone(r.info);r.info.name=r.info.name:match('^%s*(.-)%s*$')end
            elseif kind=='playability'then r.playability=ok and first(value,id)or nil
            elseif kind=='votes'then r.vote_data=ok and first(value,id,'data')or nil
            elseif kind=='age'then r.age_data=ok and value or nil end
            update(r)
        end)
    end
    while not r.done do wait()end;return r.result
end
return module
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn details_cancel_guards_and_metadata_cache_match_native()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local player={};local outsider={};local now=0;local jobs={};local calls={};local current;local fail=false;local m
local ctx={clock=function()return now end,client=function()return true end,player=function()return player end,isplayer=function(v)return v==player or v==outsider end,enum=function(v)return v end,
 spawn=function(f)table.insert(jobs,f)end,
 request=function(kind,id)
  assert(id==7);table.insert(calls,kind)
  if kind=='info'then if fail then error('HTTP 503')end;return{data={{id=7,rootPlaceId=99,name=' Mock place ',creator={name='Creator'},description='Description'}}}end
  if kind=='playability'then return{{universeId=7,isPlayable=false,playabilityStatus='UniverseRootPlaceIsPrivate'}}end
  if kind=='votes'then return{data={{id=7,upVotes=9,downVotes=1}}}end
  return{ageRecommendationDetails={summary={ageRecommendation={displayNameWithHeaderShort='Maturity: Mild'}}}}
 end,
 present=function(r,close)
  current=r
  local ok,e=pcall(m.prompt,{},player,0);assert(not ok and tostring(e):find('already has prompt in progress for this Player',1,true))
  if r.info or r.info_error then close(r)end
 end,
 wait=function()local run=jobs;jobs={};for _,f in run do f()end end,
 teleport=function()error('teleport forbidden in cancellation fixture')end}
m=assert(loadstring(source))(ctx)
for _,v in {false,123,'x'}do local ok,e=pcall(m.prompt,{},v,7);assert(not ok and tostring(e):find('Unable to cast value to Object',1,true))end
for _,case in {{nil,7,'Argument 1 missing or nil'},{player,nil,'Argument 2 missing or nil'},{outsider,7,'player must be local player'}}do local ok,e=pcall(m.prompt,{},case[1],case[2]);assert(not ok and tostring(e):find(case[3],1,true),tostring(e))end
assert(m.prompt({},player,7)=='PromptClosed'and #calls==4)
assert(m.prompt({},player,'7')=='PromptClosed'and #calls==4)
now=31;fail=true
assert(m.prompt({},player,7)=='PromptClosed'and #calls==6)
now=32;assert(m.prompt({},player,7)=='PromptClosed'and #calls==6)
ctx.client=function()return false end
local ok,e=pcall(m.prompt,{},player,0);assert(not ok and tostring(e):find('can only be called on the client',1,true))
"#).exec()
    }
    #[test]
    fn details_join_completes_before_native_legacy_teleport_failure()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local player={};local jobs={};local events={};local shown;local destroyed=0;local m
local ctx={client=function()return true end,player=function()return player end,isplayer=function(v)return v==player end,enum=function(v)return v end,
 spawn=function(f)table.insert(jobs,f)end,
 request=function(kind,id)
  if kind=='info'then return{data={{id=id,rootPlaceId=99,name='Mock'}}}end
  if kind=='playability'then return{{universeId=id,isPlayable=true,playabilityStatus='Playable'}}end
  return{data={}}
 end,
 present=function(r)
  shown=r
  if r.info and r.playability and not r.done then m.join(r)end
 end,
 teleport=function(id)assert(id==99 and shown.done and shown.result=='TeleportAttempted');table.insert(events,'teleport');error('Cannot teleport in Studio')end,
 destroy=function()destroyed+=1 end,
 wait=function()local run=jobs;jobs={};for _,f in run do local ok,e=pcall(f);if not ok then assert(tostring(e):find('Cannot teleport in Studio',1,true));table.insert(events,'error')end end end}
m=assert(loadstring(source))(ctx)
assert(m.prompt({},player,-1)=='TeleportAttempted')
assert(events[1]=='teleport'and events[2]=='error'and destroyed==0)
m.close(shown);assert(destroyed==1 and shown.result=='TeleportAttempted')
"#).exec()
    }
}
