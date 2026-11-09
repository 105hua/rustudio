//! Notification prompt engine guards and CoreScript dialog lifecycle. The
//! unpublished Studio flow performs no HTTP. Published POSTs are exercised only
//! through localhost mocks during development; no live prompt/follow is sent.
use super::*;

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let ctx=lua.create_table()?;
    // Measured in the signed-in Studio client (CoreScript flag).
    ctx.set("disable_background_dismiss", true)?;
    ctx.set("client",lua.create_function(|lua,()|Ok(world(lua)?.side==Side::Client))?)?;
    ctx.set("emit",lua.create_function(|lua,(value,name):(Value,String)| {
        let r=opt_inst(&value).ok_or_else(||rt_err("Service expected"))?;
        world(lua)?.fire(r,&name,vec![]);Ok(())
    })?)?;
    ctx.set("log",lua.create_function(|lua,message:String| {
        world(lua)?.output.push((Level::Error,message));Ok(())
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    ctx.set("begin",lua.create_function(|lua,(kind,user,universe):(String,u64,u64)| {
        http::background(lua,move |session,scope| {
            use rustudio_account::web_social::{NotificationAction,Read};
            let result=match kind.as_str() {
                "prompt"=>NotificationAction::Prompt {universe}.execute(&session),
                "follow"=>NotificationAction::Follow {user,universe}.execute(&session),
                "name"=>Read::ExperienceDetails(universe).fetch_in(&session,&scope),
                _=>return Err("Unknown notification request".into()),
            };
            result.map_err(|e|e.to_string())
        })
    })?)?;
    let module:Table=lua.load(SOURCE).set_name("=rustudio.web_social.prompts").call(ctx)?;
    let methods:Table=module.get("methods")?;
    let target:Table=lua.globals().get("__rs_lua_methods")?;
    for entry in methods.pairs::<String,mlua::Function>() {
        let (name,value)=entry?;target.set(name,value)?;
    }
    lua.set_named_registry_value("web_social.prompts",module)
}

pub const SOURCE:&str=r#"
local ctx=...
local module={methods={}}
local active;local serial=0;local names={}
local function defer(fn) if ctx.defer then ctx.defer(fn) else task.defer(fn) end end
local function universe()return ctx.universe and ctx.universe() or game.GameId end
local function player()return ctx.player and ctx.player() or game:GetService('Players').LocalPlayer end
local function await(token)
    while true do
        local done,body,err=ctx.poll(token)
        if done then
            if err then error(err,0) end
            return game:GetService('HttpService'):JSONDecode(body)
        end
        task.wait()
    end
end
local function request(kind,user,id)return await(ctx.begin(kind,user,id))end
local function name_for(id)
    local entry=names[id]
    if entry then while not entry.done do task.wait() end;return entry.name end
    entry={done=false};names[id]=entry
    local ok,value=pcall(request,'name',0,id)
    if ok and type(value)=='table' and type(value.data)=='table' and type(value.data[1])=='table' and type(value.data[1].name)=='string' then entry.name=value.data[1].name end
    entry.done=true;return entry.name
end
local function destroy(prompt)
    if prompt.gui then prompt.gui:Destroy() end
    if ctx.destroy then ctx.destroy(prompt) end
end
function module.close(token)
    local prompt=active
    if not prompt or prompt.token~=token then return end
    active=nil;destroy(prompt)
    ctx.emit(prompt.service,'OptInPromptClosed')
end
local function toast(text)
    if ctx.toast then return ctx.toast(text) end
    local gui=Instance.new('ScreenGui');gui.Name='ExperienceNotificationToast';gui.ResetOnSpawn=false;gui.DisplayOrder=1001
    local label=Instance.new('TextLabel');label.Size=UDim2.fromOffset(440,64);label.Position=UDim2.new(.5,-220,1,-100);label.BackgroundColor3=Color3.fromRGB(35,37,39);label.TextColor3=Color3.new(1,1,1);label.TextSize=18;label.TextWrapped=true;label.Text=text;label.Parent=gui
    local p=player();gui.Parent=p and p:FindFirstChild('PlayerGui') or game:GetService('CoreGui')
    task.delay(4,function()gui:Destroy()end)
end
function module.confirm(token)
    local prompt=active
    if not prompt or prompt.token~=token then return end
    local p=player()
    if prompt.universe~=0 and p then
        -- CoreScript starts FollowingsPostFollow, then immediately closes.
        -- Its asynchronous completion only produces a toast, never another
        -- OptInPromptClosed event.
        local ok,work=pcall(ctx.begin,'follow',p.UserId,prompt.universe)
        defer(function()
            local success=ok and pcall(await,work)
            if success then toast(prompt.name and ('Notifications turned on for '..prompt.name) or 'Notifications turned on for this experience')
            else toast('Something went wrong. Please try again later.') end
        end)
    end
    module.close(token)
end
local function render(prompt)
    if ctx.present then return ctx.present(prompt,function()module.close(prompt.token)end,function()module.confirm(prompt.token)end) end
    local gui=Instance.new('ScreenGui');gui.Name='ExperienceNotificationPrompt';gui.ResetOnSpawn=false;gui.DisplayOrder=1000
    local background=Instance.new('TextButton');background.Name='Background';background.Size=UDim2.fromScale(1,1);background.Text='';background.BackgroundColor3=Color3.new(0,0,0);background.BackgroundTransparency=.4;background.Parent=gui
    local frame=Instance.new('Frame');frame.Name='ConfirmationPrompt';frame.Active=true;frame.Size=UDim2.fromOffset(440,240);frame.Position=UDim2.new(.5,-220,.5,-120);frame.BackgroundColor3=Color3.fromRGB(35,37,39);frame.ZIndex=2;frame.Parent=gui
    local title=Instance.new('TextLabel');title.Name='Title';title.BackgroundTransparency=1;title.Position=UDim2.fromOffset(16,16);title.Size=UDim2.new(1,-32,0,36);title.Text='Turn on Notifications';title.TextSize=22;title.TextColor3=Color3.new(1,1,1);title.ZIndex=3;title.Parent=frame
    local message=Instance.new('TextLabel');message.Name='Message';message.BackgroundTransparency=1;message.Position=UDim2.fromOffset(16,60);message.Size=UDim2.new(1,-32,0,92);message.TextSize=18;message.TextColor3=Color3.new(1,1,1);message.TextWrapped=true;message.RichText=true;message.ZIndex=3
    if prompt.name then
        local name=prompt.name:gsub('&','&amp;'):gsub('<','&lt;'):gsub('>','&gt;'):gsub('"','&quot;'):gsub("'",'&apos;')
        message.Text="Receive notifications from \n<b>"..name.."</b> while you're away?"
    else message.Text="Receive updates on activity while you're away" end
    message.Parent=frame
    for index,text in {'Cancel','Yes'} do
        local button=Instance.new('TextButton');button.Name=text;button.Text=text;button.TextSize=18;button.Modal=true;button.Size=UDim2.fromOffset(192,40);button.Position=UDim2.fromOffset(index==1 and 16 or 232,180);button.ZIndex=3;button.Parent=frame
        button.Activated:Connect(function()if index==1 then module.close(prompt.token)else module.confirm(prompt.token)end end)
    end
    background.Activated:Connect(function()
        if not ctx.disable_background_dismiss then module.close(prompt.token) end
    end)
    local p=player();gui.Parent=p and p:FindFirstChild('PlayerGui') or game:GetService('CoreGui');prompt.gui=gui
end
local function show(service,id)
    ctx.emit(service,'PromptOptInRequested')
    local function present(name)
        -- NotificationModalsManager replaces another UENUpsell without
        -- invoking the old CloseCallback. A later dismissal fires once.
        if active then destroy(active) end
        serial+=1;active={service=service,universe=id,name=name,token=serial}
        render(active)
    end
    if id==0 then present(nil) else defer(function()present(name_for(id))end) end
end
local function fail(service,message)
    ctx.log(message);ctx.emit(service,'OptInPromptClosed')
end
module.methods.PromptOptIn=function(service)
    if not ctx.client() then error('ExperienceNotificationService:PromptOptIn can only be executed by a local script',0) end
    local id=universe()
    if id==0 then show(service,id);return end
    local p=player();if not p then return end
    local ok,token=pcall(ctx.begin,'prompt',p.UserId,id)
    defer(function()
        if not ok then fail(service,'Failed to call Notifications API: '..tostring(token));return end
        local success,value=pcall(await,token)
        if not success then
            local message=tostring(value)
            if message=='Variant cast failed' then error(message,0) end
            if message=='Empty response received from Notifications API' then
                fail(service,message);fail(service,'Failed to parse Notifications API response');return
            end
            if message~='Empty response received from Notifications API' and message~='Failed to parse Notifications API response' then message='Failed to call Notifications API: '..message end
            fail(service,message);return
        end
        if type(value)~='table' then error('Variant cast failed',0) end
        if value.canPrompt==nil then fail(service,'Invalid response received from Notifications API');error('Variant cast failed',0) end
        if type(value.canPrompt)~='boolean' then error('Variant cast failed',0) end
        if not value.canPrompt then fail(service,'The player cannot be prompted. Please check ExperienceNotificationService:CanPromptOptIn first before calling ExperienceNotificationService:PromptOptIn')
        else show(service,id) end
    end)
end
return module
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_cancel_replacement_and_follow_order() -> mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local tasks={};local log={};local requests={};local shown={};local next_request=0
local id=0;local client=true;local follow_error;local prompt_error;local prompt_response={canPrompt=true};local destroyed=0
local service={};local player={UserId=1}
game={GetService=function()return {JSONDecode=function(_,body)return requests[tonumber(body)].value end}end}
local ctx={client=function()return client end,universe=function()return id end,player=function()return player end,
 emit=function(_,name)table.insert(log,{'event',name})end,log=function(message)table.insert(log,{'error',message})end,
 present=function(prompt,close,confirm)table.insert(shown,{prompt=prompt,close=close,confirm=confirm})end,
 destroy=function()destroyed+=1 end,toast=function(message)table.insert(log,{'toast',message})end,
 defer=function(fn)table.insert(tasks,fn)end,
 begin=function(kind,user,universe)
  next_request+=1;local result={}
  if kind=='prompt' then assert(user==1 and universe==7);result.value=prompt_response;result.error=prompt_error
  elseif kind=='name' then assert(user==0 and universe==7);result.value={data={{name='Test experience'}}}
  elseif kind=='follow' then assert(user==1 and universe==7);result.value={};result.error=follow_error
  else error('unexpected request')end
  requests[next_request]=result;table.insert(log,{'request',kind});return next_request
 end,
 poll=function(token)return true,tostring(token),requests[token].error end}
local module=assert(loadstring(source))(ctx)
local function drain()while #tasks>0 do local fn=table.remove(tasks,1);fn()end end
for _=1,3 do assert(select('#',module.methods.PromptOptIn(service))==0)end
assert(#shown==3 and destroyed==2 and next_request==0)
local closes=0;for _,row in log do if row[2]=='OptInPromptClosed' then closes+=1 end end;assert(closes==0)
shown[3].close();shown[3].close();assert(log[#log][2]=='OptInPromptClosed' and destroyed==3)
closes=0;for _,row in log do if row[2]=='OptInPromptClosed' then closes+=1 end end;assert(closes==1)
module.methods.PromptOptIn(service);shown[4].confirm();assert(next_request==0 and log[#log][2]=='OptInPromptClosed')
id=7;module.methods.PromptOptIn(service);assert(log[#log][1]=='request' and log[#log][2]=='prompt');drain()
assert(shown[5].prompt.name=='Test experience')
shown[5].confirm();assert(log[#log][2]=='OptInPromptClosed' and log[#log-1][2]=='follow');drain()
assert(log[#log][1]=='toast' and log[#log][2]=='Notifications turned on for Test experience')
local before=next_request;module.methods.PromptOptIn(service);drain();assert(next_request==before+1)
follow_error='HTTP 500';shown[6].confirm();assert(log[#log][2]=='OptInPromptClosed');drain();assert(log[#log][2]=='Something went wrong. Please try again later.')
prompt_response={canPrompt=false};module.methods.PromptOptIn(service);drain()
assert(log[#log][2]=='OptInPromptClosed' and log[#log-1][1]=='error' and log[#log-1][2]=='The player cannot be prompted. Please check ExperienceNotificationService:CanPromptOptIn first before calling ExperienceNotificationService:PromptOptIn')
local before=#log;prompt_error='Empty response received from Notifications API';module.methods.PromptOptIn(service);drain()
local closed=0;for i=before+1,#log do if log[i][2]=='OptInPromptClosed' then closed+=1 end end;assert(closed==2)
prompt_error='HTTP 429 (Too Many Requests)';module.methods.PromptOptIn(service);drain();assert(log[#log-1][2]=='Failed to call Notifications API: HTTP 429 (Too Many Requests)' and log[#log][2]=='OptInPromptClosed')
prompt_error=nil
prompt_response={};module.methods.PromptOptIn(service);local ok,e=pcall(drain);assert(not ok and e=='Variant cast failed' and log[#log-1][2]=='Invalid response received from Notifications API')
client=false;local before=next_request;local ok,e=pcall(module.methods.PromptOptIn,service);assert(not ok and next_request==before and e=='ExperienceNotificationService:PromptOptIn can only be executed by a local script')
"#).exec()
    }
}
