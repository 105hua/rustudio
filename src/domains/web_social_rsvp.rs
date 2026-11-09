//! RSVP preconditions/completion//,
//! online VirtualEvents PromptConnections/RsvpPrompt. Live verification reads
//! only; confirmations and their transport are tested against localhost mocks.
use super::*;
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let ctx = lua.create_table()?;
    ctx.set(
        "begin",
        lua.create_function(|lua, (kind, id, going): (String, String, Option<bool>)| {
            http::background(lua, move |session, scope| {
                use rustudio_account::web_social::{Read, update_event_rsvp};
                if kind == "update" {
                    update_event_rsvp(&session, &id, going.unwrap_or(false))
                } else {
                    Read::ExperienceEvent(id).fetch_in(&session, &scope)
                }
                .map_err(|e| e.to_string())
            })
        })?,
    )?;
    ctx.set("poll", lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let module: Table = lua
        .load(SOURCE)
        .set_name("=rustudio.web_social.rsvp")
        .call(ctx)?;
    lua.globals().get::<Table>("__rs_lua_methods")?.set(
        "PromptRsvpToEventAsync",
        module.get::<mlua::Function>("prompt")?,
    )?;
    lua.set_named_registry_value("web_social.rsvp", module)
}
const SOURCE: &str = r#"
local ctx=...;local module={};local pending={};local active
local function client()return ctx.client and ctx.client()or(not ctx.client and game:GetService('RunService'):IsClient())end
local function wait()if ctx.wait then return ctx.wait()end;return task.wait()end
local function spawn(f)if ctx.spawn then return ctx.spawn(f)end;task.spawn(f)end
local function clock()return ctx.clock and ctx.clock()or os.clock()end
local function enum(n)return ctx.enum and ctx.enum(n)or Enum.RsvpStatus[n]end
local function warn_error(prefix,err)if ctx.warn then ctx.warn(prefix,err)else warn(prefix,err)end end
local function request(kind,id,going)
    if ctx.request then return ctx.request(kind,id,going)end
    local token=ctx.begin(kind,id,going)
    while true do local done,body,err=ctx.poll(token);if done then if err then error(err,0)end;return game:GetService('HttpService'):JSONDecode(body)end;wait()end
end
local function destroy(r)if r.gui then r.gui:Destroy();r.gui=nil end;if active==r then active=nil end end
local function finish(r,status,error_text)
    if r.done then return end;r.done=true;r.result=enum(status);r.error=error_text
    if pending[r.id]==r then pending[r.id]=nil end;destroy(r)
end
local completion_error='SocialService:PromptRsvpToEventCompleted: Error completing RSVP prompt.'
function module.close()
    local r=active;if not r then return end
    finish(r,({going='Going',notGoing='NotGoing'})[r.data.userRsvpStatus]or'None')
end
local function date_text(value)
    if not value then return''end
    local locale=game:GetService('LocalizationService').RobloxLocaleId
    local pattern=value:ToLocalTime().Year>DateTime.now():ToLocalTime().Year and'll'or'ddd, MMM DD'
    return value:FormatLocalTime(pattern,locale)..' at '..value:FormatLocalTime('LT',locale)
end
function module.confirm()
    local r=active;if not r or clock()<r.ready then return end
    local going=r.data.userRsvpStatus~='going';destroy(r)
    spawn(function()
        local ok,err=pcall(request,'update',r.id,going)
        if not ok then finish(r,'None',completion_error);warn_error('Error updating rsvp status: ',err);return end
        finish(r,going and'Going'or'NotGoing')
        if going then
            local text='Reminder for '..(r.data.displayTitle or'')..' is set for '..(ctx.date_text and ctx.date_text(r.start)or date_text(r.start))..'.'
            if ctx.toast then ctx.toast(text)else game:GetService('StarterGui'):SetCore('SendNotification',{Title=text})end
        end
    end)
end
local function render(r)
    if active then destroy(active)end;active=r;r.ready=clock()+3
    if ctx.present then ctx.present(r,module.close,module.confirm);return end
    local gui=Instance.new('ScreenGui');gui.Name='EventRsvpPrompt';gui.ResetOnSpawn=false;gui.DisplayOrder=1000;r.gui=gui
    local frame=Instance.new('Frame');frame.Name='Dialog';frame.Size=UDim2.fromOffset(500,350);frame.Position=UDim2.new(.5,-250,.5,-175);frame.BackgroundColor3=Color3.fromRGB(35,37,39);frame.Parent=gui
    local function label(name,text,y,height,size)
        local l=Instance.new('TextLabel');l.Name=name;l.Text=text;l.TextSize=size;l.TextColor3=Color3.new(1,1,1);l.TextWrapped=true;l.BackgroundTransparency=1;l.Size=UDim2.new(1,-40,0,height);l.Position=UDim2.fromOffset(20,y);l.Parent=frame
    end
    label('Title',r.data.displayTitle or'',20,60,24);label('Time',date_text(r.start),86,30,18);label('Description',r.data.displayDescription or'',126,140,18)
    local close=Instance.new('TextButton');close.Name='CloseAffordance';close.Text='×';close.Size=UDim2.fromOffset(32,32);close.Position=UDim2.new(1,-40,0,6);close.Activated:Connect(module.close);close.Parent=frame
    local button=Instance.new('TextButton');button.Name='Confirm';button.Text=r.data.userRsvpStatus=='going'and'Unfollow Event'or'Notify Me';button.Modal=true;button.TextSize=18;button.Size=UDim2.new(1,-40,0,42);button.Position=UDim2.fromOffset(20,286);button.Activated:Connect(module.confirm);button.Parent=frame
    local p=game:GetService('Players').LocalPlayer;gui.Parent=p:FindFirstChild('PlayerGui')or game:GetService('CoreGui')
end
local function id_arg(id)
    if id==nil then error('Argument 1 missing or nil',0)end
    if type(id)~='number'and type(id)~='string'then error('Unable to cast value to std::string',0)end
    id=tostring(id);local n=id:gsub('^0+','')
    if not id:match('^%d+$')or n==''or #n>19 or(#n==19 and n>'9223372036854775807')then error('SocialService:GetEventRsvpStatusAsync Received invalid event id.',0)end
    return id
end
function module.prompt(service,id)
    id=id_arg(id)
    if not client()then error('SocialService:GetEventRsvpStatusAsync must be called on the client.',0)end
    if pending[id]then error('SocialService:PromptRsvpToEventAsync: Event RSVP prompt is already in progress.',0)end
    local ok,value=pcall(request,'status',id)
    if not ok then if value=='Unable to convert response to JSON'then error('Error parsing event details for eventId '..id..': Invalid response format.',0)end;error('Error getting event details: '..tostring(value),0)end
    local time=type(value)=='table'and value.eventTime
    local start=type(time)=='table'and type(time.startUtc)=='string'and DateTime.fromIsoDate(time.startUtc)
    local ending=type(time)=='table'and type(time.endUtc)=='string'and DateTime.fromIsoDate(time.endUtc)
    if type(value)~='table'or type(value.universeId)~='number'or value.universeId%1~=0 or type(value.userRsvpStatus)~='string'or not start or not ending then error('Error parsing event details for eventId '..id..': Invalid response format.',0)end
    if value.universeId~=(ctx.universe and ctx.universe()or game.GameId)then error('SocialService:PromptRsvpToEventAsync Event is not in the current universe.',0)end
    local now=ctx.now and ctx.now()or DateTime.now().UnixTimestampMillis
    if start.UnixTimestampMillis<=now then error('SocialService:PromptRsvpToEventAsync Event has already started.',0)end
    if ending.UnixTimestampMillis<now then error('SocialService:PromptRsvpToEventAsync Event has already ended.',0)end
    local r={id=id,start=start};pending[id]=r
    spawn(function()
        local success,data=pcall(request,'details',id)
        if not success then finish(r,'None',completion_error);warn_error('Error fetching event details: ',data);return end
        r.data=data;local t=data.eventTime and data.eventTime.startUtc;r.start=type(t)=='string'and DateTime.fromIsoDate(t)or nil
        render(r)
    end)
    while not r.done do wait()end
    if r.error then error(r.error,0)end;return r.result
end
return module
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rsvp_cancel_fresh_snapshot_delay_update_and_failure() -> mlua::Result<()> {
        let lua = Lua::new();
        crate::ltypes::install(&lua, &lua.globals())?;
        lua.globals().set("source", SOURCE)?;
        lua.load(r#"
local queue={};local calls={};local shown={};local warnings={};local toasts={};local clock=0;local client=true
local current=DateTime.now().UnixTimestampMillis
local first={universeId=7,userRsvpStatus='none',eventTime={startUtc='2099-01-01T00:00:00Z',endUtc='2099-01-02T00:00:00Z'}}
local details={universeId=7,userRsvpStatus='none',displayTitle='Mock event',displayDescription='Body',eventTime=first.eventTime}
local failure_kind;local module
local ctx={client=function()return client end,universe=function()return 7 end,now=function()return current end,clock=function()return clock end,enum=function(n)return n end,wait=coroutine.yield,spawn=function(f)table.insert(queue,f)end,present=function(r)table.insert(shown,r)end,warn=function(prefix,error)table.insert(warnings,{prefix,error})end,toast=function(text)table.insert(toasts,text)end,date_text=function()return'MOCK_TIME'end}
ctx.request=function(kind,id,going)
 assert(id=='7');table.insert(calls,{kind,going});if failure_kind==kind then error('HTTP 503 (Service Unavailable)',0)end
 if kind=='status'then return first elseif kind=='details'then return details else return{}end
end
module=assert(loadstring(source))(ctx)
local function fails(f,text)local ok,e=pcall(f);assert(not ok and tostring(e):find(text,1,true),tostring(e))end
local function start()local co=coroutine.create(function()return module.prompt({},7)end);local ok,e=coroutine.resume(co);assert(ok,e);return co end
local function step()table.remove(queue,1)()end
local function done(co,value)local ok,r=coroutine.resume(co);assert(ok and r==value,tostring(r))end
client=false;fails(function()module.prompt({},'7')end,'SocialService:GetEventRsvpStatusAsync must be called on the client.');client=true
fails(function()module.prompt({},'+7')end,'SocialService:GetEventRsvpStatusAsync Received invalid event id.')
first.universeId=8;fails(function()module.prompt({},'7')end,'SocialService:PromptRsvpToEventAsync Event is not in the current universe.');first.universeId=7
local time=first.eventTime;first.eventTime={startUtc='2000-01-01T00:00:00Z',endUtc='2000-01-02T00:00:00Z'}
fails(function()module.prompt({},'7')end,'SocialService:PromptRsvpToEventAsync Event has already started.');first.eventTime=time
-- Fresh frontend data determines cancellation status, not the first status read.
details.userRsvpStatus='going';local co=start();fails(function()module.prompt({},'7')end,'Event RSVP prompt is already in progress.');step();module.close();done(co,'Going')
for _,call in calls do assert(call[1]~='update')end
-- Default UIBlox input delay is three seconds. Closing begins before HTTP resolves.
details.userRsvpStatus='none';clock=0;co=start();step();module.confirm();assert(#queue==0);clock=3;module.confirm();assert(#queue==1);module.close();step();done(co,'Going')
assert(calls[#calls][1]=='update'and calls[#calls][2]==true and toasts[#toasts]=='Reminder for Mock event is set for MOCK_TIME.')
details.userRsvpStatus='going';co=start();step();clock=6;module.confirm();step();done(co,'NotGoing');assert(calls[#calls][2]==false)
-- Frontend fetch and confirmation failures both reject through native completion.
failure_kind='details';co=start();step();local ok,e=coroutine.resume(co);assert(not ok and e:find('SocialService:PromptRsvpToEventCompleted: Error completing RSVP prompt.',1,true));assert(warnings[#warnings][1]=='Error fetching event details: ')
failure_kind=nil;co=start();step();clock=9;failure_kind='update';module.confirm();step();ok,e=coroutine.resume(co);assert(not ok and e:find('Error completing RSVP prompt.',1,true));assert(warnings[#warnings][1]=='Error updating rsvp status: ')
failure_kind='status';fails(function()module.prompt({},'7')end,'Error getting event details: HTTP 503 (Service Unavailable)')
"#).exec()
    }
}
