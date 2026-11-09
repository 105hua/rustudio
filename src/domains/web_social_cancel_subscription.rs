//! CancelSubscriptionApp controller and native PerformCancelSubscription's local
//! cache update. No cancellation HTTP endpoint is called.
use super::*;
pub fn install(lua:&Lua)->mlua::Result<()> {
    let ctx=lua.create_table()?;
    ctx.set("server",lua.create_function(|lua,()|Ok(world(lua)?.side==Side::Server))?)?;
    ctx.set("peer",lua.create_function(|lua,()|Ok(world(lua)?.has_peer))?)?;
    ctx.set("begin",lua.create_function(|lua,(user,id):(u64,String)| {
        http::background(lua,move |session,scope|rustudio_account::web_social::Read::SubscriptionDetails{user,subscription:id}.fetch_in(&session,&scope).map_err(|e|e.to_string()))
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    ctx.set("fire",lua.create_function(|lua,(service,id):(Value,String)| {
        let r=opt_inst(&service).ok_or_else(||rt_err("Service expected"))?;
        world(lua)?.fire(r,"UserSubscriptionStatusChanged",vec![Value::String(lua.create_string(&id)?)]);Ok(())
    })?)?;
    let module:Table=lua.load(SOURCE).set_name("=rustudio.web_social.cancel_subscription").call(ctx)?;
    lua.globals().set("__ws_subscription_override",module.get::<mlua::Function>("cached")?)?;
    lua.globals().get::<Table>("__rs_lua_methods")?.set("PromptCancelSubscription",module.get::<Table>("methods")?.get::<mlua::Function>("PromptCancelSubscription")?)?;
    lua.set_named_registry_value("web_social.cancel_subscription",module)
}
pub const SOURCE:&str=r#"
local ctx=...;local module={methods={}};local active;local serial=0;local sequence=0
local cache={status={},details={}}
local function clock()return ctx.clock and ctx.clock()or os.clock()end
local function player()return ctx.player and ctx.player()or game:GetService('Players').LocalPlayer end
local function defer(fn)if ctx.defer then ctx.defer(fn)else task.spawn(fn)end end
local function copy(value)if type(value)~='table'then return value end;local out={};for k,v in value do out[k]=copy(v)end;return out end
local function enum(kind,name)return ctx.enum and ctx.enum(kind,name)or Enum[kind][name]end
local function now()return ctx.now and ctx.now()or DateTime.now()end
local function put(kind,user,id,value)
    local map=cache[kind];local key=tostring(user)..':'..id
    if not map[key]then
        local count=0;local oldest;for k,v in map do count+=1;if not oldest or v.used<map[oldest].used then oldest=k end end
        if count>=100 then map[oldest]=nil end
    end
    sequence+=1;map[key]={at=clock(),used=sequence,value=copy(value)}
end
function module.cached(kind,user,id)
    local map=cache[kind];if not map then return end
    local key=tostring(user)..':'..id;local record=map[key]
    if not record then return end
    if clock()-record.at>60 then map[key]=nil;return end
    sequence+=1;record.used=sequence;return copy(record.value)
end
local function destroy(prompt)
    if prompt.gui then prompt.gui:Destroy();prompt.gui=nil end
    if ctx.destroy then ctx.destroy(prompt)end
end
function module.close(token)
    if not active or active.token~=token then return end
    local prompt=active;active=nil;destroy(prompt)
    if prompt.phase=='Success'or prompt.phase=='Expiring'then ctx.fire(prompt.service,prompt.id)end
end
local texts={
 ErrorCancelling={'Error','An error occurred and subscription renewal was not canceled. Please try again.'},
 Unsubscribed={'Can’t Cancel','You are not currently subscribed.'},
 Expiring={'Already Canceled','Your subscription has already been successfully canceled. No further action is needed.'},
 Request={'Cancel Subscription','If you cancel your subscription now, you can still access it until {date}.'},
 Success={'Your subscription was successfully cancelled',''},
}
local render
function module.confirm(token)
    local prompt=active
    if not prompt or prompt.token~=token or prompt.phase~='Request'then return end
    destroy(prompt);prompt.phase='Loading'
    defer(function()
        -- Native helper updates both local caches synchronously, then resolves.
        -- It does not call the production /subscriptions/{id}/cancel route.
        put('details',prompt.player.UserId,prompt.id,{SubscriptionState=enum('SubscriptionState','SubscribedWillNotRenew'),ExpireTime=now(),ExpirationDetails={Reason=enum('SubscriptionExpirationReason','SubscriberCancelled')}})
        put('status',prompt.player.UserId,prompt.id,{IsSubscribed=true,IsRenewing=false})
        if active~=prompt then return end
        prompt.phase='Success';render(prompt)
    end)
end
render=function(prompt)
    if ctx.present then return ctx.present(prompt,function()module.close(prompt.token)end,function()module.confirm(prompt.token)end)end
    local gui=Instance.new('ScreenGui');gui.Name='CancelSubscriptionApp';gui.ResetOnSpawn=false;gui.DisplayOrder=1000;prompt.gui=gui
    local success=prompt.phase=='Success';local frame=Instance.new('Frame');frame.Name=success and 'SuccessToast'or prompt.phase;frame.Size=UDim2.fromOffset(460,success and 68 or 250);frame.Position=success and UDim2.new(.5,-230,0,20)or UDim2.new(.5,-230,.5,-125);frame.BackgroundColor3=Color3.fromRGB(35,37,39);frame.Parent=gui
    local label=Instance.new('TextLabel');label.Name='Title';label.Size=UDim2.new(1,-32,0,52);label.Position=UDim2.fromOffset(16,8);label.BackgroundTransparency=1;label.Text=texts[prompt.phase][1];label.TextSize=22;label.TextColor3=Color3.new(1,1,1);label.TextWrapped=true;label.Parent=frame
    if not success then
        local ready=clock()+.75
        local body=label:Clone();body.Name='Body';body.Text=texts[prompt.phase][2]:gsub('{date}',prompt.date or '');body.TextSize=18;body.Size=UDim2.new(1,-32,0,100);body.Position=UDim2.fromOffset(16,64);body.Parent=frame
        local labels=prompt.phase=='Request'and {"Don't Cancel",'Yes, Cancel'}or {'OK'}
        for i,text in labels do
            local button=Instance.new('TextButton');button.Name=i==2 and 'YesCancelButton'or prompt.phase=='Request'and 'DontCancelButton'or 'OkButton';button.Text=text;button.Modal=true;button.TextSize=18;button.Size=UDim2.fromOffset(#labels==1 and 428 or 206,40);button.Position=UDim2.fromOffset(16+(i-1)*222,192);button.Parent=frame
            button.Activated:Connect(function()if clock()<ready then return end;if i==2 then module.confirm(prompt.token)else module.close(prompt.token)end end)
        end
    end
    -- UIBlox Toast has no duration supplied by this CoreScript. Do not invent
    -- a timed dismissal/status signal. Its onDismissed hook calls module.close.
    gui.Parent=prompt.player:FindFirstChild('PlayerGui')or game:GetService('CoreGui')
end
local function details(p,id)
    local cached=module.cached('details',p.UserId,id);if cached then return cached end
    if ctx.details then local value=ctx.details(p,id);put('details',p.UserId,id,value);return value end
    local token=ctx.begin(p.UserId,id)
    while true do
        local done,body,err=ctx.poll(token)
        if done then
            if err then error(err,0)end
            local value=game:GetService('HttpService'):JSONDecode(body).details
            if type(value)~='table'then error('Details not found for subscription: '..id,0)end
            local out={SubscriptionState=enum('SubscriptionState',value.subscriptionState)}
            if type(value.nextRenewTimestampMs)=='number'and value.nextRenewTimestampMs>0 then out.NextRenewTime=DateTime.fromUnixTimestampMillis(value.nextRenewTimestampMs)end
            if type(value.expireTimestampMs)=='number'and value.expireTimestampMs>0 then out.ExpireTime=DateTime.fromUnixTimestampMillis(value.expireTimestampMs)end
            put('details',p.UserId,id,out);return out
        end
        task.wait()
    end
end
local function show(service,p,id)
    if active then return end
    serial+=1;local prompt={service=service,player=p,id=id,token=serial,phase='Loading'};active=prompt
    defer(function()
        local ok,value=pcall(details,p,id)
        if active~=prompt then return end
        prompt.phase='ErrorCancelling'
        if ok then
            if value.SubscriptionState==enum('SubscriptionState','SubscribedWillRenew')or value.SubscriptionState==enum('SubscriptionState','SubscribedRenewalPaymentPending')then
                local valid,date=pcall(function()return value.NextRenewTime:FormatLocalTime('LL','en-us')end)
                if valid then prompt.phase='Request';prompt.date=date end
            elseif value.SubscriptionState==enum('SubscriptionState','NeverSubscribed')or value.SubscriptionState==enum('SubscriptionState','Expired')then prompt.phase='Unsubscribed'
            elseif value.SubscriptionState==enum('SubscriptionState','SubscribedWillNotRenew')then prompt.phase='Expiring'end
        end
        render(prompt)
    end)
end
module.methods.PromptCancelSubscription=function(service,p,id)
    if typeof(p)~='Instance'or not p:IsA('Player')then error('First argument to PromptCancelSubscription must be a player',0)end
    if id==nil then error('Argument 2 missing or nil',0)end
    if type(id)~='string'and type(id)~='number'then error('Unable to cast value to std::string',0)end
    id=tostring(id);if id==''then error('Missing SubscriptionId',0)end
    local prefix,numeric=string.match(id,'^([a-zA-Z]+)%-(%d+)$')
    if #id>25 or not prefix or #prefix>5 or #numeric>19 then error('Invalid SubscriptionId',0)end
    if ctx.server()then
        if ctx.peer()then __ws_send(service,'__cancel','request',p,id)end;return
    end
    if p~=player()then error('Can only call PromptCancelSubscription from LocalScript on the local player',0)end
    show(service,p,id)
end
function module.receive(_,sender,verb,p,id)
    if not ctx.server()and verb=='request'and p==player()then show(game:GetService('MarketplaceService'),p,id)end
end
return module
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_controller_is_local_and_signals_only_source_dismissal_paths()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local p={UserId=5,IsA=function(_,name)return name=='Player'end};local other={IsA=p.IsA}
local old=typeof;typeof=function(v)if v==p or v==other then return 'Instance'end;return old(v)end
local tasks={};local shown={};local events={};local reads=0;local server=false;local timestamp=10;local failure
local metadata={SubscriptionState='SubscribedWillRenew',NextRenewTime={FormatLocalTime=function(_,format,locale)assert(format=='LL'and locale=='en-us');return 'November 8, 2026'end}}
local s={};local ctx={server=function()return server end,peer=function()return true end,player=function()return p end,clock=function()return timestamp end,now=function()return 'mock-datetime'end,
 enum=function(kind,name)return name end,details=function(player,id)reads+=1;if failure then error(failure,0)end;return metadata end,
 defer=function(fn)table.insert(tasks,fn)end,present=function(prompt,close,confirm)table.insert(shown,{prompt=prompt,close=close,confirm=confirm})end,
 fire=function(service,id)assert(service==s);table.insert(events,id)end}
local sent={};__ws_send=function(service,marker,verb,who,id)assert(service==s and marker=='__cancel'and verb=='request'and who==p);table.insert(sent,id)end
local module=assert(loadstring(source))(ctx)
local function drain()while #tasks>0 do table.remove(tasks,1)()end end
assert(select('#',module.methods.PromptCancelSubscription(s,p,'EXP-0'))==0)
module.methods.PromptCancelSubscription(s,p,'EXP-0');assert(#tasks==1);drain()
assert(#shown==1 and shown[1].prompt.phase=='Request'and shown[1].prompt.date=='November 8, 2026'and reads==1)
shown[1].close();assert(#events==0)
module.methods.PromptCancelSubscription(s,p,'EXP-0');drain();shown[2].confirm();assert(shown[2].prompt.phase=='Loading');drain()
assert(shown[3].prompt.phase=='Success'and #events==0)
local status=module.cached('status',5,'EXP-0');assert(status.IsSubscribed and not status.IsRenewing)
status.IsSubscribed=false;assert(module.cached('status',5,'EXP-0').IsSubscribed)
local value=module.cached('details',5,'EXP-0');assert(value.SubscriptionState=='SubscribedWillNotRenew'and value.ExpireTime=='mock-datetime'and value.ExpirationDetails.Reason=='SubscriberCancelled')
-- Source onDismissed, not the confirmation itself, emits this protected event.
shown[3].close();assert(#events==1 and events[1]=='EXP-0')
module.methods.PromptCancelSubscription(s,p,'EXP-0');drain();assert(shown[4].prompt.phase=='Expiring'and reads==1);shown[4].close();assert(#events==2)
timestamp=71;assert(module.cached('status',5,'EXP-0')==nil and module.cached('details',5,'EXP-0')==nil)
metadata={SubscriptionState='NeverSubscribed'};module.methods.PromptCancelSubscription(s,p,'EXP-1');drain();assert(shown[5].prompt.phase=='Unsubscribed');shown[5].close();assert(#events==2)
failure='HTTP 503 (Service Unavailable)';module.methods.PromptCancelSubscription(s,p,'EXP-2');drain();assert(shown[6].prompt.phase=='ErrorCancelling');shown[6].close();assert(#events==2)
local checks={
 {function()module.methods.PromptCancelSubscription(s,{},'EXP-0')end,'First argument to PromptCancelSubscription must be a player'},
 {function()module.methods.PromptCancelSubscription(s,other,'EXP-0')end,'Can only call PromptCancelSubscription from LocalScript on the local player'},
 {function()module.methods.PromptCancelSubscription(s,p,'')end,'Missing SubscriptionId'},
 {function()module.methods.PromptCancelSubscription(s,p,'TOOLONG-1')end,'Invalid SubscriptionId'},
 {function()module.methods.PromptCancelSubscription(s,p,'EXP-12345678901234567890')end,'Invalid SubscriptionId'},
}
for _,check in checks do local ok,e=pcall(check[1]);assert(not ok and e==check[2],tostring(e))end
server=true;module.methods.PromptCancelSubscription(s,p,'RBP-1');assert(sent[1]=='RBP-1')
"#).exec()
    }
}
