//! Group join dialog port: online Groups.Common metadata/actions requests and
//! useGroupJoinPromptConnections controller. Development confirms only mocks;
//! native measurement dismisses Not Now and never joins a real community.
use super::*;
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let ctx = lua.create_table()?;
    ctx.set(
        "begin",
        lua.create_function(|lua, (kind, group): (String, u64)| {
            http::background(lua, move |session, scope| {
                use rustudio_account::web_social::{Read, join_community};
                match kind.as_str() {
                    "details" => Read::GroupProfile {
                        group,
                        actions_only: false,
                        deactivated: true,
                    }
                    .fetch_in(&session, &scope),
                    "actions" => Read::GroupProfile {
                        group,
                        actions_only: true,
                        deactivated: true,
                    }
                    .fetch_in(&session, &scope),
                    "join" => join_community(&session, group),
                    _ => unreachable!(),
                }
                .map_err(|e| e.to_string())
            })
        })?,
    )?;
    ctx.set("poll", lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let module: Table = lua
        .load(SOURCE)
        .set_name("=rustudio.web_social.group")
        .call(ctx)?;
    lua.globals()
        .get::<Table>("__rs_lua_methods")?
        .set("PromptJoinAsync", module.get::<mlua::Function>("prompt")?)?;
    lua.set_named_registry_value("web_social.group", module)
}
pub const SOURCE: &str = r#"
local ctx=...;local module={};local active
local function wait()if ctx.wait then return ctx.wait()end;return task.wait()end
local function spawn(f)if ctx.spawn then return ctx.spawn(f)end;task.spawn(f)end
local function now()return ctx.now and ctx.now()or os.clock()end
local function client()return ctx.client and ctx.client()or(not ctx.client and game:GetService('RunService'):IsClient())end
local function status(name)return ctx.enum and ctx.enum(name)or Enum.GroupMembershipStatus[name]end
local function request(kind,id)
    if ctx.request then return ctx.request(kind,id)end
    local token=ctx.begin(kind,id)
    while true do local done,body,err=ctx.poll(token);if done then if err then error(err,0)end;return game:GetService('HttpService'):JSONDecode(body)end;wait()end
end
local function membership(value)
    local first=((value.components or {}).Actions or {}).buttons or {};first=first[1]or{}
    return {member=(first.type=='ViewCommunity'or first.type=='ViewFullProfile')and not first.disabledReason,
        pending=first.type=='CancelJoinCommunityRequest',eligible=(first.type=='JoinCommunity'or first.type=='Follow')and not first.disabledReason,disabled=first.disabledReason}
end
local function notify(text)
    if ctx.notify then ctx.notify(text)else game:GetService('StarterGui'):SetCore('SendNotification',{Title=text})end
end
local function finish(r,name,err)
    if active~=r or r.done then return end
    r.done=true;r.result=status(name);r.error=err;active=nil
    if r.gui then r.gui:Destroy()end
end
local function fail(r,error_text,toast)
    finish(r,'None',error_text);notify(toast)
end
function module.close()
    local r=active;if not r or r.busy then return end
    local m=r.membership or {};finish(r,m.member and 'AlreadyMember'or m.pending and 'JoinRequestPending'or'None')
end
function module.join()
    local r=active
    if not r or r.busy or not r.membership or not r.membership.eligible or now()-r.ready<2 then return end
    r.busy=true
    spawn(function()
        local ok,value=pcall(function()request('join',r.id);return request('actions',r.id)end)
        if active~=r then return end
        local social=r.header.hasSocialModules~=false
        if not ok then fail(r,'Network error on join prompt: '..tostring(value),social and 'Failed to join community. Please try again later.'or'Failed to follow profile. Please try again later.');return end
        if value.errors then fail(r,'Network error on join prompt: Some group components failed to load',social and 'Failed to join community. Please try again later.'or'Failed to follow profile. Please try again later.');return end
        local m=membership(value)
        if m.member then finish(r,'Joined');notify((social and 'Joined 'or'Following ')..r.header.name..'.')
        elseif m.pending then finish(r,'JoinRequestPending');notify('Requested to join '..r.header.name..'.')
        else fail(r,'Unexpected membership status after join prompt',social and 'Failed to join community. Please try again later.'or'Failed to follow profile. Please try again later.')end
    end)
end
-- English strings are extracted from shipped Common.Locales.en-us. The shell
-- is simplified; controller decisions, input delay and completion are source.
local text={["Action.BugReportingEnable"]="Enable",["Action.BugReportingTurnOn"]="Turn on",["Action.Cancel"]="Cancel",["Action.CancelRequest"]="Cancel request",["Action.Close"]="Close",["Action.Continue"]="Continue",["Action.CreateGroup"]="Create Community",["Action.Follow"]="Follow",["Action.JoinGroup"]="Join Community",["Action.LeaveGroup"]="Leave Community",["Action.MakePrimary"]="Make Primary",["Action.More"]="more",["Action.NotNow"]="Not Now",["Action.RemoveAffiliate"]="Remove",["Action.RemovePrimary"]="Remove Primary",["Action.RemoveTicket"]="Remove ticket",["Action.SeeAll"]="See All",["Action.Unfollow"]="Unfollow",["Action.VerifyMyAccount"]="Verify My Account",["Action.ViewGroup"]="View Community",["Description.AccountTenureRequirementExplanation"]="Only accounts created more than the selected number of days ago can join.",["Description.AddRoles"]="Only roles you assign to this category will be able to view it when the restrict toggle is enabled",["Description.BugReportingEnableForumsUpsell"]="To automatically route player bug reports from forums to your Creator Hub Helpdesk queue, you must first enable your community forums in settings.",["Description.BugReportingForumCategoriesUpsell"]="Select the roles in each category that can attach bug reports below",["Description.BugReportingForumsEnabledUpsell"]="Skip the manual work of copying threads to other tools for triage. Players can now file bugs directly in forums, which will automatically route straight to your Creator Hub helpdesk queue.",["Description.JoinGroupPending"]="Your join request is pending review by the community.",["Description.NotEligibleToJoin"]="Your account cannot join this community.",["Description.PostUnavailable"]="This post is not accessible to you at this time.",["Description.PremiumMaxGroups"]="You have joined the maximum number of communities.",["Description.RequiresAccountTenureTooltipOneDay"]="You must have an account for 1 day before joining this community.",["Description.RequiresAccountTenureTooltipOneMonth"]="You must have an account for 30 days before joining this community.",["Description.RequiresAccountTenureTooltipOneWeek"]="You must have an account for 7 days before joining this community.",["Description.RequiresAccountTenureTooltipThreeDays"]="You must have an account for 3 days before joining this community.",["Description.RequiresAccountTenureTooltipThreeMonths"]="You must have an account for 90 days before joining this community.",["Description.RoleRestrictedCategory"]="This is a role restricted category",["Description.RoleRestrictedToggle"]="Only members with the required role(s) can participate.",["Description.UnrestrictDialog"]="Unrestricting this category will immediately make all topics, comments, and replies in the category publicly visible.",["Description.UnrestrictedMessagesCategory"]="Members can send unrestricted messages and attach images in this category.",["Label.ByOwnerWithFormat"]="By {ownerName}",["Message.BannedFromGroup"]="You have been banned from this community.",["Message.GroupClosed"]="You cannot join a closed community.",["Modal.ContentTextHigh"]="Verify your account to join this community.\n\n1. Go to Account Info > ID Verify\n2. Complete the verification process",["Modal.ContentTextLow"]="Verify your account to join this community.\n\n1. Go to Account Info\n2. Choose one verification option:\n    - Add Phone\n    - Add Email\n    - ID Verify\n3. Complete the verification process",["Modal.ContentTextMedium"]="Verify your account to join this community.\n\n1. Go to Account Info\n2. Choose one verification option:\n    - Add Phone AND Add Email\n    - ID Verify\n3. Complete the verification process",["Title.AlreadyFollowing"]="You are already following {groupName}",["Title.AlreadyMember"]="You are already a member of {groupName}",["Title.Follow"]="Follow {groupName}",["Title.Join"]="Join {groupName}",["Title.RequestedToJoin"]="You have already requested to join {groupName}",["Title.UnableToJoin"]="Unable to join {groupName}"}
local disabled={BannedFromCommunity='Message.BannedFromGroup',CommunityIsClosed='Message.GroupClosed',MaxCommunitiesJoined='Description.PremiumMaxGroups',AccountCreatedLessThanOneDay='Description.RequiresAccountTenureTooltipOneDay',AccountCreatedLessThanThreeDays='Description.RequiresAccountTenureTooltipThreeDays',AccountCreatedLessThanOneWeek='Description.RequiresAccountTenureTooltipOneWeek',AccountCreatedLessThanOneMonth='Description.RequiresAccountTenureTooltipOneMonth',AccountCreatedLessThanThreeMonths='Description.RequiresAccountTenureTooltipThreeMonths',InsufficientVerificationLevelHigh='Modal.ContentTextHigh',InsufficientVerificationLevelMedium='Modal.ContentTextMedium',InsufficientVerificationLevelLow='Modal.ContentTextLow'}
local function update(r)
    if ctx.present then ctx.present(r,module.close,module.join);return end
    local gui=r.gui
    if not gui then
        gui=Instance.new('ScreenGui');gui.Name='GroupJoinPrompt';gui.ResetOnSpawn=false;gui.DisplayOrder=1000;r.gui=gui
        local frame=Instance.new('Frame');frame.Name='Dialog';frame.Size=UDim2.fromOffset(450,340);frame.Position=UDim2.new(.5,-225,.5,-170);frame.BackgroundColor3=Color3.fromRGB(35,37,39);frame.Parent=gui
        local function label(name,y,height,size)
            local l=Instance.new('TextLabel');l.Name=name;l.Size=UDim2.new(1,-40,0,height);l.Position=UDim2.fromOffset(20,y);l.TextWrapped=true;l.TextSize=size;l.TextColor3=Color3.new(1,1,1);l.BackgroundTransparency=1;l.Text='';l.Parent=frame;return l
        end
        r.title=label('Title',20,64,24);r.owner=label('Owner',86,26,18);r.counts=label('Members',118,26,18);r.sub=label('Description',150,92,16)
        local function button(name,x,y,width,fn)
            local b=Instance.new('TextButton');b.Name=name;b.TextSize=18;b.Size=UDim2.fromOffset(width,36);b.Position=UDim2.fromOffset(x,y);b.Activated:Connect(fn);b.Parent=frame;return b
        end
        r.close=button('NotNow',230,278,190,module.close);r.close.Modal=true;r.buy=button('JoinCommunity',30,278,190,module.join)
        local x=button('CloseAffordance',410,4,32,module.close);x.Text='×'
        local p=game:GetService('Players').LocalPlayer;gui.Parent=p:FindFirstChild('PlayerGui')or game:GetService('CoreGui')
    end
    if not r.header then r.title.Text='';r.buy.Visible=false;r.close.Visible=false;return end
    local m=r.membership;local h=r.header;local social=h.hasSocialModules~=false
    local key=m.member and(social and'Title.AlreadyMember'or'Title.AlreadyFollowing')or m.pending and'Title.RequestedToJoin'or not m.eligible and'Title.UnableToJoin'or(social and'Title.Join'or'Title.Follow')
    r.title.Text=(text[key]or''):gsub('{groupName}',function()return h.name or''end)
    r.owner.Text=(text['Label.ByOwnerWithFormat']or''):gsub('{ownerName}',function()return h.ownerName or''end)
    local count=h.counts and h.counts.membersCount or 0
    local suffix='';for _,unit in {{1e12,'T'},{1e9,'B'},{1e6,'M'},{1e3,'K'}}do if count>=unit[1]then count=math.floor(count/unit[1]*10+.5)/10;suffix=unit[2];break end end
    r.counts.Text=tostring(count)..suffix..' '..(social and'Members'or'Followers')
    r.sub.Text=m.pending and text['Description.JoinGroupPending']or(m.disabled and(text[disabled[m.disabled]]or text['Description.NotEligibleToJoin']))or''
    r.buy.Visible=m.eligible==true and not m.member and not m.pending;r.buy.Text=text[social and'Action.JoinGroup'or'Action.Follow'];r.close.Visible=true;r.close.Text=text[r.buy.Visible and'Action.NotNow'or'Action.Close']
end
function module.prompt(service,id)
    if id==nil then error('Argument 1 missing or nil',0)end
    id=tonumber(id);if not id then error('Unable to cast value to int64',0)end
    id=id<0 and math.ceil(id)or math.floor(id)
    if id<=0 then error('GroupService:PromptJoinAsync: argument is not a valid groupId',0)end
    if not client()then error('GroupService:PromptJoinAsync: Must be called on the client.',0)end
    if active then error('GroupService:PromptJoinAsync: Join prompt is already in progress.',0)end
    local r={id=id};active=r;update(r)
    spawn(function()
        local ok,value=pcall(request,'details',id)
        if active~=r then return end
        if not ok then fail(r,'Network error on join prompt: '..tostring(value),'Failed to load community. Please try again later.');return end
        local c=value.components or{}
        if value.errors then fail(r,'Network error on join prompt: Some group components failed to load','Failed to load community. Please try again later.');return end
        if not next(c)then fail(r,'Group does not exist','The community does not exist.');return end
        if c.CommunityDeactivated then fail(r,'Group is deactivated','This community has been deactivated.');return end
        if c.CommunityLocked then fail(r,'Group is currently moderated','This community is currently moderated.');return end
        r.header=c.CommunityProfileHeader or{};r.membership=membership(value);r.ready=now();update(r)
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
    fn join_dialog_cancel_pending_late_data_and_mock_join() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("source", SOURCE)?;
        lua.load(r#"
local client=true;local now=0;local queue={};local calls={};local toast={};local shown={};local mode='JoinCommunity';local fail;local module
local function profile(action)return{components={CommunityProfileHeader={name='Mock Group',ownerName='Mock',hasSocialModules=true,counts={membersCount=2842}},Actions={buttons={{type=action}}}}}end
local ctx={client=function()return client end,now=function()return now end,enum=function(n)return n end,wait=coroutine.yield,spawn=function(f)table.insert(queue,f)end,present=function(r)shown[#shown+1]=r end,notify=function(t)table.insert(toast,t)end}
ctx.request=function(kind,id)
 assert(id==7);table.insert(calls,kind)
 if fail then error(fail,0)end
 if kind=='details'then return profile(mode)end
 if kind=='join'then return{}end
 return profile(mode=='PendingAfterJoin'and'CancelJoinCommunityRequest'or'ViewCommunity')
end
module=assert(loadstring(source))(ctx)
local function start()
 local co=coroutine.create(function()return module.prompt({},7)end);local ok,e=coroutine.resume(co);assert(ok,e);return co
end
local function step()table.remove(queue,1)()end
local function complete(co,want)local ok,value=coroutine.resume(co);assert(ok and value==want,tostring(value))end
local function guard(fn,text)local ok,e=pcall(fn);assert(not ok and e:find(text,1,true),tostring(e))end
client=false;guard(function()module.prompt({},7)end,'Must be called on the client.');client=true
guard(function()module.prompt({},0)end,'argument is not a valid groupId')
local co=start();guard(function()module.prompt({},7)end,'Join prompt is already in progress.');step();module.close();complete(co,'None');assert(#calls==1)
mode='ViewCommunity';co=start();step();module.close();complete(co,'AlreadyMember');assert(#calls==2)
mode='CancelJoinCommunityRequest';co=start();step();module.close();complete(co,'JoinRequestPending');assert(#calls==3)
-- Close while metadata is still pending; late results cannot reopen the dialog.
co=start();module.close();complete(co,'None');local n=#shown;step();assert(#shown==n)
mode='JoinCommunity';now=0;co=start();step();module.join();assert(#queue==0);now=2;module.join();assert(#queue==1);module.close();assert(coroutine.status(co)=='suspended');step();complete(co,'Joined');assert(calls[#calls-1]=='join'and calls[#calls]=='actions'and toast[#toast]=='Joined Mock Group.')
mode='PendingAfterJoin';now=4;co=start();step();shown[#shown].membership.eligible=true;now=6;module.join();step();complete(co,'JoinRequestPending');assert(toast[#toast]=='Requested to join Mock Group.')
mode='JoinCommunity';fail='HTTP 503 (Service Unavailable)';co=start();step();local ok,e=coroutine.resume(co);assert(not ok and e:find('Network error on join prompt: HTTP 503 (Service Unavailable)',1,true));assert(toast[#toast]=='Failed to load community. Please try again later.')
"#).exec()
    }
}
