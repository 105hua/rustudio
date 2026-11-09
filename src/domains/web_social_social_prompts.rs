//! Current desktop GameInvite ModalManager lifecycle. Native verification only
//! dismissed dialogs. Send transport is tested exclusively against localhost.
use super::*;
pub fn install(lua:&Lua)->mlua::Result<()> {
    let ctx=lua.create_table()?;
    ctx.set("server",lua.create_function(|lua,()|Ok(world(lua)?.side==Side::Server))?)?;
    ctx.set("peer",lua.create_function(|lua,()|Ok(world(lua)?.has_peer))?)?;
    ctx.set("fire",lua.create_function(|lua,args:MultiValue| {
        let r=args.front().and_then(opt_inst).ok_or_else(||rt_err("Service expected"))?;
        let name=arg_str(&args,1,"string")?;
        world(lua)?.fire(r,&name,args.iter().skip(2).cloned().collect());Ok(())
    })?)?;
    ctx.set("begin",lua.create_function(|lua,body:String| {
        let body=serde_json::from_str(&body).map_err(|e|rt_err(e.to_string()))?;
        http::background(lua,move |session,_|rustudio_account::web_social::send_experience_invite(&session,body).map_err(|e|e.to_string()))
    })?)?;
    ctx.set("log",lua.create_function(|lua,message:String|{world(lua)?.output.push((Level::Error,message));Ok(())})?)?;
    ctx.set("link_begin",lua.create_function(|lua,body:String| {
        let body=serde_json::from_str(&body).map_err(|e|rt_err(e.to_string()))?;
        http::background(lua,move |session,_|rustudio_account::web_social::create_expiring_share_link(&session,body).map_err(|e|e.to_string()))
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let module:Table=lua.load(SOURCE).set_name("=rustudio.web_social.social_prompts").call(ctx)?;
    let target:Table=lua.globals().get("__rs_lua_methods")?;
    for pair in module.get::<Table>("methods")?.pairs::<String,mlua::Function>() {
        let (name,value)=pair?;target.set(name,value)?;
    }
    lua.set_named_registry_value("web_social.social_prompts",module)
}
pub const SOURCE:&str=r#"
local ctx=...;local module={methods={}};local active;local serial=0
local function service()return game:GetService('SocialService')end
local function player()return ctx.player and ctx.player() or game:GetService('Players').LocalPlayer end
local function defer(fn)if ctx.defer then ctx.defer(fn)else task.spawn(fn)end end
local function send(...)if ctx.send then ctx.send(...)else __ws_send(...)end end
local function is_instance(v,class)return typeof(v)=='Instance' and v:IsA(class)end
local function finished(s,p)
    ctx.fire(s,'GameInvitePromptClosed',p,{})
    if not ctx.server() and ctx.peer()then send(s,'__social','closed',p)end
end
function module.close(token)
    if not active or active.token~=token then return end
    local old=active;active=nil
    if old.gui then old.gui:Destroy()end
    if ctx.destroy then ctx.destroy(old)end
    finished(old.service,old.player)
end
function module.invite(token,id,update)
    local prompt=active
    if not prompt or prompt.token~=token then return end
    local key=tostring(id)
    if prompt.status[key]=='Pending' or prompt.status[key]=='Success' then return end
    if (ctx.place and ctx.place()or game.PlaceId)==0 then
        warn('Game Invite failed to send. Cannot send invite to unpublished Place.')
        prompt.status[key]='Failed';if update then update('Failed')end;return
    end
    prompt.status[key]='Pending';if update then update('Pending')end
    defer(function()
        local http=game:GetService('HttpService')
        local body={recipientUserId=key,placeId=tostring(ctx.place and ctx.place()or game.PlaceId),trigger='DeveloperMultiple',idempotencyKey=http:GenerateGUID(false)}
        local ok=pcall(function()
            local token=ctx.begin(http:JSONEncode(body))
            while true do local done,_,err=ctx.poll(token);if done then if err then error(err,0)end;return end;task.wait()end
        end)
        prompt.status[key]=ok and 'Success' or 'Failed'
        if active==prompt and update then update(prompt.status[key])end
    end)
end
local function roster()
    if ctx.roster then return ctx.roster()end
    local pages=game:GetService('Players'):GetFriendsAsync(player().UserId);local rows={}
    repeat for _,row in pages:GetCurrentPage()do table.insert(rows,row)end;if pages.IsFinished then break end;pages:AdvanceToNextPageAsync()until false
    return rows
end
local function render(prompt)
    if ctx.present then return ctx.present(prompt,function()module.close(prompt.token)end)end
    local gui=Instance.new('ScreenGui');gui.Name='GameInvite';gui.ResetOnSpawn=false;gui.DisplayOrder=1000;prompt.gui=gui
    local shade=Instance.new('TextButton');shade.Name='Background';shade.Size=UDim2.fromScale(1,1);shade.Text='';shade.BackgroundColor3=Color3.new(0,0,0);shade.BackgroundTransparency=.4;shade.Parent=gui
    local frame=Instance.new('Frame');frame.Name='GameInviteModal';frame.Active=true;frame.Size=UDim2.fromOffset(480,480);frame.Position=UDim2.new(.5,-240,.5,-240);frame.BackgroundColor3=Color3.fromRGB(35,37,39);frame.Parent=gui
    local title=Instance.new('TextLabel');title.Name='Title';title.BackgroundTransparency=1;title.Size=UDim2.new(1,-80,0,56);title.Position=UDim2.fromOffset(40,0);title.Text='Invite Friends';title.TextSize=24;title.TextColor3=Color3.new(1,1,1);title.Parent=frame
    local close=Instance.new('TextButton');close.Name='CloseButton';close.Text='X';close.Size=UDim2.fromOffset(40,40);close.Position=UDim2.fromOffset(8,8);close.Parent=frame;close.Activated:Connect(function()module.close(prompt.token)end)
    local search=Instance.new('TextBox');search.Name='SearchBox';search.PlaceholderText='Search Friends';search.Text='';search.Size=UDim2.new(1,-32,0,40);search.Position=UDim2.fromOffset(16,60);search.Parent=frame
    local list=Instance.new('ScrollingFrame');list.Name='Friends';list.Size=UDim2.new(1,-32,1,-120);list.Position=UDim2.fromOffset(16,108);list.BackgroundTransparency=1;list.Parent=frame
    gui.Parent=player():FindFirstChild('PlayerGui')or game:GetService('CoreGui')
    defer(function()
        local ok,rows=pcall(roster);if active~=prompt then return end
        if not ok then rows={}end
        local rendered={}
        for _,friend in rows do
            local row=Instance.new('Frame');row.Name=tostring(friend.Id);row.Size=UDim2.new(1,-8,0,56);row.BackgroundTransparency=1;row.Parent=list
            local name=Instance.new('TextLabel');name.Size=UDim2.new(1,-120,1,0);name.BackgroundTransparency=1;name.Text=friend.DisplayName or friend.Username or '';name.TextColor3=Color3.new(1,1,1);name.TextSize=18;name.Parent=row
            local button=Instance.new('TextButton');button.Name='InviteButton';button.Text='Invite';button.Size=UDim2.fromOffset(100,36);button.Position=UDim2.new(1,-108,0,10);button.Parent=row
            button.Activated:Connect(function()module.invite(prompt.token,friend.Id,function(status)button.Text=({Pending='Inviting',Success='Invited',Failed='Retry'})[status]end)end)
            table.insert(rendered,{row=row,search=string.lower((friend.DisplayName or '')..' '..(friend.Username or ''))})
        end
        local function filter()
            local count=0;local query=string.lower(search.Text)
            for _,entry in rendered do entry.row.Visible=string.find(entry.search,query,1,true)~=nil;if entry.row.Visible then entry.row.Position=UDim2.fromOffset(0,count*56);count+=1 end end
            list.CanvasSize=UDim2.fromOffset(0,count*56)
        end
        search:GetPropertyChangedSignal('Text'):Connect(filter);filter()
    end)
end
local function show(s,p)
    if active then finished(s,p);return end
    serial+=1;active={service=s,player=p,token=serial,status={}};render(active)
end
module.methods.PromptGameInvite=function(s,p,options)
    if not is_instance(p,'Player')then error('SocialService:PromptGameInvite First argument should be a Player instance.',0)end
    if not ctx.server()and p~=player()then error('SocialService:PromptGameInvite Must prompt the LocalPlayer when called from a local script.',0)end
    if options~=nil and not is_instance(options,'ExperienceInviteOptions')then error('SocialService:PromptGameInvite Second argument should be nil or an ExperienceInviteOptions instance.',0)end
    if ctx.server()and options~=nil then error('SocialService:PromptGameInvite ExperienceInviteOptions must be used from a local script.',0)end
    if ctx.server()then if ctx.peer()then send(s,'__social','request',p)end;return end
    -- Current native EnableNewInviteMenuCustomization2=false: use default menu.
    show(s,p)
end
-- Measured current desktop Studio returns without opening a phone-book UI
-- or signaling PhoneBookPromptClosed. Mobile host behavior is outside this build.
module.methods.PromptPhoneBook=function(s,p,tag)
    if tag==nil then error('Argument 2 missing or nil',0)end
    if type(tag)~='string'and type(tag)~='number'then error('Unable to cast value to std::string',0)end
    if not is_instance(p,'Player')then error('SocialService:PromptPhoneBook First argument should be a Player instance.',0)end
    if not ctx.server()and p~=player()then error('SocialService:PromptPhoneBook Must prompt the LocalPlayer when called from a local script.',0)end
end
local function link_result(name,value)
    return ctx.link_result and ctx.link_result(name)or Enum.PromptLinkSharingResult[name],value
end
local function link(s,p,options)
    if options~=nil and type(options)~='table'then error('Unable to cast value to Dictionary',0)end
    if p==nil then error('SocialService:PromptLinkSharing player cannot be null',0)end
    if not is_instance(p,'Player')then error('Unable to cast Instance to Player',0)end
    if not ctx.server()then error('SocialService:PromptLinkSharing can only be called from script with Server RunContext',0)end
    options=options or {}
    local body={CreatorUserId=p.UserId}
    for _,key in {'FallbackLinkId','PreviewTitle','PreviewDescription'}do if type(options[key])=='string'and options[key]~=''then body[key]=options[key]end end
    for _,key in {'ExpirationSeconds','PreviewAssetId'}do if type(options[key])=='number'and options[key]>0 then body[key]=math.floor(options[key])end end
    local data=type(options.LaunchData)=='string'and options.LaunchData or ''
    if #data>(ctx.launch_data_limit or 200)then
        (ctx.log or warn)('SocialService:PromptLinkSharing Max allowed LaunchData length is '..tostring(ctx.launch_data_limit or 200))
        return link_result('InvalidLaunchData','')
    end
    body.CustomData=data
    local response
    if ctx.link_request then response=ctx.link_request(body)else
        local http=game:GetService('HttpService');local token=ctx.link_begin(http:JSONEncode(body))
        while true do local done,value,err=ctx.poll(token);if done then if err then error(err,0)end;response=http:JSONDecode(value);break end;task.wait()end
    end
    return link_result('Success',response.link)
end
module.methods.PromptLinkSharing=link
module.methods.PromptLinkSharingAsync=link
function module.receive(_,sender,verb,p)
    local s=service()
    if ctx.server()then
        if verb=='closed'and p==sender then ctx.fire(s,'GameInvitePromptClosed',sender,{})end
    elseif verb=='request'and p==player()then show(s,p)end
end
return module
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn link_preparation_returns_native_tuple_without_opening_share_sheet()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local p={UserId=5,IsA=function(_,name)return name=='Player'end};local old=typeof;typeof=function(v)if v==p then return 'Instance'end;return old(v)end
local server=true;local requests={};local logs={};local failure
local module=assert(loadstring(source))({server=function()return server end,link_result=function(name)return name end,log=function(text)table.insert(logs,text)end,
link_request=function(body)table.insert(requests,body);if failure then error(failure,0)end;return {link='https://mock.invalid/share'}end})
for _,name in {'PromptLinkSharing','PromptLinkSharingAsync'}do
 local status,url=module.methods[name]({},p,{FallbackLinkId='fallback',ExpirationSeconds=15.75,PreviewTitle='Title',PreviewDescription='',PreviewAssetId=7.2,LaunchData='data',Ignored=true})
 assert(status=='Success'and url=='https://mock.invalid/share')
 local body=requests[#requests];assert(body.CreatorUserId==5 and body.FallbackLinkId=='fallback'and body.ExpirationSeconds==15 and body.PreviewTitle=='Title'and body.PreviewDescription==nil and body.PreviewAssetId==7 and body.CustomData=='data'and body.Ignored==nil)
end
local status,url=module.methods.PromptLinkSharing({},p,{LaunchData=string.rep('a',201)})
assert(status=='InvalidLaunchData'and url==''and #requests==2 and logs[1]=='SocialService:PromptLinkSharing Max allowed LaunchData length is 200')
failure='HTTP 403 (Forbidden)';local ok,e=pcall(module.methods.PromptLinkSharingAsync,{},p);assert(not ok and e==failure)
server=false;local before=#requests;ok,e=pcall(module.methods.PromptLinkSharing,{},p,{});assert(not ok and e=='SocialService:PromptLinkSharing can only be called from script with Server RunContext'and #requests==before)
ok,e=pcall(module.methods.PromptLinkSharing,{},nil);assert(not ok and e=='SocialService:PromptLinkSharing player cannot be null')
"#).exec()
    }
    #[test]
    fn invite_duplicate_cancel_and_replication_match_modal_manager()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local p={IsA=function(_,c)return c=='Player'end};local other={IsA=p.IsA};local options={IsA=function(_,c)return c=='ExperienceInviteOptions'end}
local oldtypeof=typeof;typeof=function(v)if v==p or v==other or v==options then return 'Instance'end;return oldtypeof(v)end
local server=false;local log={};local shown={};local sent={};local destroyed=0;local s={};local place=0;local work={};local posts=0;local failure
local http={GenerateGUID=function()return 'test-guid'end,JSONEncode=function(_,v)assert(v.recipientUserId=='2'and v.placeId=='7'and v.trigger=='DeveloperMultiple'and v.idempotencyKey=='test-guid');return '{}'end}
game={GetService=function(_,name)if name=='SocialService'then return s end;assert(name=='HttpService');return http end}
warn=function(message)assert(message=='Game Invite failed to send. Cannot send invite to unpublished Place.')end
local module=assert(loadstring(source))({server=function()return server end,peer=function()return true end,player=function()return p end,place=function()return place end,
 fire=function(s,name,who,recipients)assert(name=='GameInvitePromptClosed'and who==p and next(recipients)==nil);table.insert(log,true)end,
 send=function(_,marker,verb,who)assert(marker=='__social'and who==p);table.insert(sent,verb)end,
 present=function(prompt,close)table.insert(shown,{prompt=prompt,close=close})end,destroy=function()destroyed+=1 end,
 defer=function(fn)table.insert(work,fn)end,begin=function(body)posts+=1;return posts end,poll=function()return true,'{}',failure end})
for _=1,3 do assert(select('#',module.methods.PromptGameInvite(s,p))==0)end
assert(#shown==1 and #log==2 and #sent==2 and destroyed==0)
module.invite(shown[1].prompt.token,2);assert(posts==0 and shown[1].prompt.status['2']=='Failed')
shown[1].close();shown[1].close();assert(#log==3 and destroyed==1)
module.methods.PromptGameInvite(s,p,options);place=7;local token=shown[2].prompt.token
module.invite(token,2);module.invite(token,2);assert(#work==1);table.remove(work,1)();assert(posts==1 and shown[2].prompt.status['2']=='Success');module.invite(token,2);assert(#work==0)
shown[2].close();module.methods.PromptGameInvite(s,p);failure='HTTP 429';module.invite(shown[3].prompt.token,2);table.remove(work,1)();assert(shown[3].prompt.status['2']=='Failed')
local ok,e=pcall(module.methods.PromptGameInvite,s,other);assert(not ok and e=='SocialService:PromptGameInvite Must prompt the LocalPlayer when called from a local script.')
ok,e=pcall(module.methods.PromptGameInvite,s,p,{});assert(not ok and e=='SocialService:PromptGameInvite Second argument should be nil or an ExperienceInviteOptions instance.')
server=true;ok,e=pcall(module.methods.PromptGameInvite,s,p,options);assert(not ok and e=='SocialService:PromptGameInvite ExperienceInviteOptions must be used from a local script.')
module.methods.PromptGameInvite(s,p);assert(sent[#sent]=='request')
local before=#log;assert(select('#',module.methods.PromptPhoneBook(s,p,''))==0 and #log==before)
local ok,e=pcall(module.methods.PromptPhoneBook,s,p);assert(not ok and e=='Argument 2 missing or nil')
module.receive(nil,other,'closed',p);assert(#log==before);module.receive(nil,p,'closed',p);assert(#log==before+1)
"#).exec()
    }
}
