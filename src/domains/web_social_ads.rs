//! Studio ad opportunity registration. Native placement lookup,
//! validation, local tracker.
//! Registration does not request or display an ad. Metadata is login-session GET.
use super::*;
pub fn install(lua:&Lua)->mlua::Result<()> {
    let ctx=lua.create_table()?;
    ctx.set("client",lua.create_function(|lua,()|Ok(world(lua)?.side==Side::Client))?)?;
    ctx.set("begin",lua.create_function(|lua,universe:u64| {
        http::background(lua,move|session,scope|rustudio_account::web_social::Read::AdPlacements(universe).fetch_in(&session,&scope).map_err(|e|e.to_string()))
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let module:Table=lua.load(SOURCE).set_name("=rustudio.web_social.ads").call(ctx)?;
    lua.globals().set("__ws_campaign_record",module.get::<mlua::Function>("campaign_record")?)?;
    let target:Table=lua.globals().get("__rs_lua_methods")?;
    for pair in module.get::<Table>("methods")?.pairs::<String,mlua::Function>() {
        let (name,method)=pair?;target.set(name,method)?;
    }
    lua.set_named_registry_value("web_social.ads",module)
}
pub const SOURCE:&str=r#"
local ctx=...
local module={methods={}}
local cache={}
local tracked={}
local campaigns={};local disclosures={};local impressions={}
local function client()if ctx.client then return ctx.client()end;return game:GetService('RunService'):IsClient()end
local function universe()return ctx.universe and ctx.universe()or game.GameId end
local function wait()if ctx.wait then ctx.wait()else task.wait()end end
local function placements(id)
    local entry=cache[id]
    if entry then
        repeat wait()until entry.done
    else
        entry={};cache[id]=entry
        local ok,value=pcall(function()
            if ctx.fetch then return ctx.fetch(id)end
            local token=ctx.begin(id)
            while true do
                local done,body,err=ctx.poll(token)
                if done then if err then error(err,0)end;return game:GetService('HttpService'):JSONDecode(body)end
                wait()
            end
        end)
        entry.done=true
        if ok and type(value)=='table'and type(value.placements)=='table'then
            entry.value=value.placements
            for kind,record in entry.value do
                if kind=='PLACEMENT_TYPE_REWARDED_VIDEO'or kind=='PLACEMENT_TYPE_PLAY_WITH_REWARD'then
                    if type(record)~='table'or type(record.defaultPlacementID)~='number'or(record.customPlacementIDs~=nil and type(record.customPlacementIDs)~='table')then entry.error=true;break end
                    for _,placement in record.customPlacementIDs or {}do if type(placement)~='number'then entry.error=true;break end end
                end
            end
        else entry.error=true end
    end
    if entry.error then error('Placement validation request failed.',0)end
    return entry.value
end
local function remove(instance)
    local record=tracked[instance]
    if record then tracked[instance]=nil;if record.connection then record.connection:Disconnect()end end
end
module.methods.RegisterAdOpportunityAsync=function(service,instance,placement)
    if not client()then error('RegisterAdOpportunityAsync can only be called from the client.',0)end
    local records=placements(universe())
    local rewarded=records.PLACEMENT_TYPE_REWARDED_VIDEO
    if not rewarded then error('No rewarded video placements available for this universe.',0)end
    local selected=rewarded.defaultPlacementID
    if placement~=nil then
        placement=tonumber(placement);if not placement then error('Unable to cast value to int64',0)end
        placement=placement<0 and math.ceil(placement)or math.floor(placement)
        local found=placement==selected
        for _,id in rewarded.customPlacementIDs or {}do if id==placement then found=true;break end end
        if not found then error('Placement ID is invalid.',0)end
        selected=placement
    end
    local count=0;for _ in tracked do count+=1 end
    if count>=(ctx.limit or 1000)then error('Maximum number of tracked instances reached. Cannot track more.',0)end
    if instance==nil then error('Instance is null.',0)end
    if not instance:IsA('GuiButton')then error('Instance is not a GuiButton.',0)end
    if not instance:FindFirstAncestorOfClass('ScreenGui')then error('Instance must be a descendant of a ScreenGui.',0)end
    if tracked[instance]then return end
    local record={placement=selected};tracked[instance]=record
    if instance.Destroying then record.connection=instance.Destroying:Connect(function()remove(instance)end)end
end
module.methods.UnregisterAdOpportunity=function(service,instance)remove(instance)end
-- SDK placement map is separate from the rewarded opportunity map. Native
-- GetCampaignEligibilityAsync retains the full payload, not just IsEligible.
function module.campaign_record(player,id,value)
    if not client()then return end
    for _,placement in value.immersiveBrandedAdPlacementIds do
        campaigns[placement]={campaignId=id,advertiserName=value.advertiserName,isEligible=value.isEligible}
    end
end
local function campaign(placement)
    local value=campaigns[placement]
    if not value then error('Missing associated campaign eligibility status for placement "'..placement..'". Make sure GetCampaignEligibilityAsync for your campaign succeeded and the user is eligible',0)end
    if not value.isEligible then error('User is ineligible to see this ad campaign: "'..value.campaignId..'"',0)end
    return value
end
local function placement_id(value)
    if value==nil then error('Argument 2 missing or nil',0)end
    if type(value)~='number'and type(value)~='string'then error('Unable to cast value to std::string',0)end
    return tostring(value)
end
local function sdk_remove(map,instance)
    local record=map[instance];if not record then return end
    map[instance]=nil
    for _,connection in record.connections do connection:Disconnect()end
end
local function sdk_watch(map,instance,record)
    record.connections={}
    map[instance]=record
    if instance.Destroying then table.insert(record.connections,instance.Destroying:Connect(function()sdk_remove(map,instance)end))end
    if instance.AncestryChanged then table.insert(record.connections,instance.AncestryChanged:Connect(function()
        if not instance:IsDescendantOf(game)then sdk_remove(map,instance)end
    end))end
end
local function sdk_existing(map,instance,placement,kind,limit)
    local previous=map[instance]
    if previous then
        if previous.placement~=placement then error('Cannot register '..tostring(instance)..' as '..kind..' for placement "'..placement..'" because it is already registered for placement "'..previous.placement..'"',0)end
        return true
    end
    local count=0;for _ in map do count+=1 end
    if count>=limit then error((kind=='a disclosure button'and 'Ad disclosure button limit reached: 'or 'Ad impression source limit reached: ')..limit,0)end
    return false
end
module.register_disclosure=function(service,button,placement)
    placement=placement_id(placement)
    if not client()then error('RegisterDisclosureButton may only be called on the client',0)end
    if not button or not button:IsA('GuiButton')then error('Argument 1 must be a GuiButton',0)end
    local metadata=campaign(placement)
    if sdk_existing(disclosures,button,placement,'a disclosure button',ctx.disclosure_limit or 512)then return end
    local record={placement=placement};sdk_watch(disclosures,button,record)
    table.insert(record.connections,button.Activated:Connect(function(inputObject)
        if not disclosures[button]then return end
        local data={disclosureButton=button,inputObject=inputObject,immersiveBrandedAdId=placement,advertiserName=metadata.advertiserName,campaignId=metadata.campaignId}
        if ctx.disclosure then ctx.disclosure(service,data)
        elseif __rs_get_callback then
            local callback=__rs_get_callback(service,'OnImmersiveBrandedAdDisclosureButtonActivated')
            if callback then callback(data)end
        end
    end))
end
module.register_impression=function(service,instance,placement)
    placement=placement_id(placement)
    if not client()then error('RegisterImpressionSource may only be called on the client',0)end
    if not instance or instance.ClassName~='Part'then error('Instance must be a Part',0)end
    if instance.Shape~=(ctx.block or Enum.PartType.Block)then error('Part.Shape must be set to Enum.Shape.Block',0)end
    campaign(placement)
    if sdk_existing(impressions,instance,placement,'an impression source',ctx.impression_limit or 128)then return end
    local record={placement=placement};sdk_watch(impressions,instance,record)
end
module.disclosures=disclosures;module.impressions=impressions
-- Engine registry retained for focused lifecycle tests, not exposed as game API.
module.tracked=tracked
return module
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_registry_cache_duplicate_validation_and_callback_payload()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local client=true;local clicked={}
local ctx={client=function()return client end,block='Block',disclosure_limit=1,impression_limit=1,disclosure=function(service,data)table.insert(clicked,data)end}
local module=assert(loadstring(source))(ctx);local s={}
local function object(class)
 local o={ClassName=class,Shape='Block'}
 function o:IsA(name)return name=='GuiButton'and self.ClassName=='TextButton'end
 local function event()return {Connect=function(_,fn)local c={Disconnect=function()end};return c end}end
 o.Destroying={Connect=function(_,fn)o.destroy=fn;return {Disconnect=function()end}end}
 o.Activated={Connect=function(_,fn)o.click=fn;return {Disconnect=function()end}end}
 return setmetatable(o,{__tostring=function()return class end})
end
local b=object('TextButton');local p=object('Part');local folder=object('Folder')
local function fails(fn,text)local ok,e=pcall(fn);assert(not ok and e==text,tostring(e))end
fails(function()module.register_disclosure(s,nil,'0')end,'Argument 1 must be a GuiButton')
fails(function()module.register_impression(s,folder,'0')end,'Instance must be a Part')
p.Shape='Ball';fails(function()module.register_impression(s,p,'0')end,'Part.Shape must be set to Enum.Shape.Block');p.Shape='Block'
fails(function()module.register_disclosure(s,b,'0')end,'Missing associated campaign eligibility status for placement "0". Make sure GetCampaignEligibilityAsync for your campaign succeeded and the user is eligible')
module.campaign_record(nil,'campaign',{isEligible=false,advertiserName='Mock',immersiveBrandedAdPlacementIds={'7','8'}})
fails(function()module.register_disclosure(s,b,'7')end,'User is ineligible to see this ad campaign: "campaign"')
module.campaign_record(nil,'campaign',{isEligible=true,advertiserName='Mock',immersiveBrandedAdPlacementIds={'7','8'}})
module.register_disclosure(s,b,'7');module.register_disclosure(s,b,'7');local input={mockInputObject=true};b.click(input)
assert(#clicked==1 and clicked[1].disclosureButton==b and clicked[1].campaignId=='campaign'and clicked[1].immersiveBrandedAdId=='7'and clicked[1].advertiserName=='Mock'and clicked[1].inputObject==input)
fails(function()module.register_disclosure(s,b,'8')end,'Cannot register TextButton as a disclosure button for placement "8" because it is already registered for placement "7"')
fails(function()module.register_disclosure(s,object('TextButton'),'7')end,'Ad disclosure button limit reached: 1')
module.register_impression(s,p,'7');module.register_impression(s,p,'7')
fails(function()module.register_impression(s,p,'8')end,'Cannot register Part as an impression source for placement "8" because it is already registered for placement "7"')
fails(function()module.register_impression(s,object('Part'),'7')end,'Ad impression source limit reached: 1')
b.destroy();p.destroy();assert(next(module.disclosures)==nil and next(module.impressions)==nil)
client=false;fails(function()module.register_disclosure(s,b,'7')end,'RegisterDisclosureButton may only be called on the client');fails(function()module.register_impression(s,p,'7')end,'RegisterImpressionSource may only be called on the client')
assert(module.methods.RegisterDisclosureButton==nil and module.methods.RegisterImpressionSource==nil)
"#).exec()
    }
    #[test]
    fn placement_cache_validations_and_tracker_lifecycle_match_native() -> mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local client=true;local universe=1;local calls=0;local waits=0
local response={placements={PLACEMENT_TYPE_REWARDED_VIDEO={defaultPlacementID=7,customPlacementIDs={8}}}}
local failure
local ctx={client=function()return client end,universe=function()return universe end,wait=function()waits+=1 end,limit=2,
fetch=function(id)calls+=1;if failure then error(failure,0)end;return response end}
local module=assert(loadstring(source))(ctx);local service={}
local function instance(class,ancestor)
 local v={class=class,ancestor=ancestor}
 function v:IsA(name)return name=='GuiButton' and (self.class=='TextButton'or self.class=='ImageButton')end
 function v:FindFirstAncestorOfClass(name)assert(name=='ScreenGui');return self.ancestor end
 v.Destroying={Connect=function(_,callback)local c={Disconnect=function()end};v.destroy=callback;return c end}
 return v
end
local a=instance('TextButton',{});local b=instance('ImageButton',{})
assert(select('#',module.methods.RegisterAdOpportunityAsync(service,a))==0)
assert(module.tracked[a].placement==7 and calls==1)
module.methods.RegisterAdOpportunityAsync(service,a,8)
assert(module.tracked[a].placement==7 and calls==1 and waits==1)
module.methods.RegisterAdOpportunityAsync(service,b,8);assert(module.tracked[b].placement==8)
local ok,e=pcall(module.methods.RegisterAdOpportunityAsync,service,a);assert(not ok and e=='Maximum number of tracked instances reached. Cannot track more.')
assert(select('#',module.methods.UnregisterAdOpportunity(service,a))==0 and module.tracked[a]==nil)
module.methods.UnregisterAdOpportunity(service,a);module.methods.UnregisterAdOpportunity(service,nil)
b.destroy();assert(next(module.tracked)==nil)
local checks={
 {nil,nil,'Instance is null.'},
 {instance('Frame',{}),nil,'Instance is not a GuiButton.'},
 {instance('TextButton',nil),nil,'Instance must be a descendant of a ScreenGui.'},
 {a,9,'Placement ID is invalid.'},
}
for _,check in checks do local ok,e=pcall(module.methods.RegisterAdOpportunityAsync,service,check[1],check[2]);assert(not ok and e==check[3],tostring(e))end
universe=2;response={placements={}};local ok,e=pcall(module.methods.RegisterAdOpportunityAsync,service,a);assert(not ok and e=='No rewarded video placements available for this universe.')
universe=3;failure='HTTP 400 (Bad Request)';local ok,e=pcall(module.methods.RegisterAdOpportunityAsync,service,a);assert(not ok and e=='Placement validation request failed.')
failure=nil;local before=calls;local ok,e=pcall(module.methods.RegisterAdOpportunityAsync,service,a);assert(not ok and e=='Placement validation request failed.' and calls==before)
universe=4;response={placements={PLACEMENT_TYPE_REWARDED_VIDEO={defaultPlacementID='7'}}};local ok,e=pcall(module.methods.RegisterAdOpportunityAsync,service,a);assert(not ok and e=='Placement validation request failed.')
client=false;local before=calls;local ok,e=pcall(module.methods.RegisterAdOpportunityAsync,service,a);assert(not ok and e=='RegisterAdOpportunityAsync can only be called from the client.' and calls==before)
"#).exec()
    }
}
