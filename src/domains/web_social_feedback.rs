//! Native feedback state machine// and
//! CoreScriptsPlayerFeedback text prompt. Submission verification is mock only.
//! PlayerSupport is disabled by the measured current native feature flag.
use super::*;
use mlua::ObjectLike;
pub fn install(lua:&Lua)->mlua::Result<()> {
    let ctx=lua.create_table()?;
    ctx.set("client",lua.create_function(|lua,()|Ok(world(lua)?.side==Side::Client))?)?;
    ctx.set("under13",lua.create_function(|lua,()| {
        // Native Player's private field defaults false. Preserve an engine
        // supplied value; account age must not be inferred from AccountAge.
        let player:Value=lua.globals().get::<Value>("game")?.as_userdata().ok_or_else(||rt_err("DataModel expected"))?.get::<Value>("Players")?;
        let Value::UserData(players)=player else{return Ok(false)};
        let local_player:Value=players.get("LocalPlayer")?;
        let Some(r)=opt_inst(&local_player)else{return Ok(false)};
        Ok(matches!(world(lua)?.d().dom.get_by_ref(r).and_then(|i|i.properties.get(&"Under13".into())),Some(Variant::Bool(true))))
    })?)?;
    ctx.set("begin",lua.create_function(|lua,(kind,id,comment):(String,u64,Option<String>)| {
        http::background(lua,move|session,scope| {
            use rustudio_account::web_social::{Read,submit_player_feedback};
            match kind.as_str() {
                "root"=>Read::ExperienceDetails(id).fetch_in(&session,&scope),
                "eligibility"=>Read::FeedbackEligibility(id).fetch_in(&session,&scope),
                "submit"=>submit_player_feedback(&session,id,comment.as_deref().unwrap_or("")),
                _=>return Err("Unknown feedback request".into()),
            }.map_err(|e|e.to_string())
        })
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let module:Table=lua.load(SOURCE).set_name("=rustudio.web_social.feedback").call(ctx)?;
    lua.globals().get::<Table>("__rs_lua_methods")?.set("PromptFeedbackSubmissionAsync",module.get::<mlua::Function>("prompt")?)?;
    lua.set_named_registry_value("web_social.feedback",module)
}
const SOURCE:&str=r#"
local ctx=...;local module={};local pending;local shown;local queue={}
local function clock()return ctx.clock and ctx.clock()or os.clock()end
local function wait()if ctx.wait then return ctx.wait()end;return task.wait()end
local function spawn(f)if ctx.spawn then return ctx.spawn(f)end;task.spawn(f)end
local function client()if ctx.client then return ctx.client()end;return game:GetService('RunService'):IsClient()end
local function request(kind,id,comment)
    if ctx.request then return ctx.request(kind,id,comment)end
    local token=ctx.begin(kind,id,comment)
    while true do local done,body,err=ctx.poll(token);if done then if err then error(err,0)end;return game:GetService('HttpService'):JSONDecode(body)end;wait()end
end
local function boolean(value)
    if type(value)=='boolean'then return value,true end
    if type(value)=='number'then return value~=0,true end
    if type(value)=='string'then value=string.lower(value);if value=='true'or value=='1'then return true,true elseif value=='false'or value=='0'then return false,true end end
    return nil,false
end
function module.classify(value,under13)
    local review,comment,category=false,true,false
    if type(value)=='table'then
        if type(value.capabilities)=='table'then value=value.capabilities end
        local function read(snake,camel,default)
            local v,valid=boolean(value[snake]);if valid then return v end
            v,valid=boolean(value[camel]);if valid then return v end;return default
        end
        review=read('can_submit_review','canSubmitReview',false)
        comment=read('can_submit_comment','canSubmitComment',true)
        category=read('can_submit_category_options','canSubmitCategoryOptions',false)
    end
    if review then return nil end
    return (under13 or(not comment and category))and'Under13'or'RateLimited'
end
local function complete(r)
    r.done=true;if pending==r then pending=nil end
end
local present
local function advance()
    if shown or #queue==0 then return end
    shown=table.remove(queue,1);shown.opened=clock();present(shown)
end
local function destroy(r)
    if r.gui then r.gui:Destroy();r.gui=nil end
    if ctx.destroy then ctx.destroy(r)end
    if shown==r then shown=nil;advance()end
end
function module.close(r)
    r=r or shown;if not r or shown~=r then return end
    if not r.reason and pending then complete(pending)end
    destroy(r)
end
function module.submit(r,text)
    r=r or shown;if not r or r.reason or shown~=r or r.submitting then return end
    if type(text)~='string'or #text<10 or #text>500 or clock()-r.opened<2 then return end
    r.submitting=true;destroy(r)
    -- Native hook ignores stale UI submissions once its watchdog cleared state.
    if not pending then return end
    local original=pending
    spawn(function()pcall(request,'submit',original.asset,text);if pending then complete(pending)end end)
end
local unavailable={
    RateLimited={"You've reached the feedback limit for now.",'Thanks for sharing your thoughts. Please try again later.'},
    Under13={"Feedback isn't available for this account.","To protect younger players, accounts under 13 can't send feedback."},
}
present=function(r)
    if ctx.present then return ctx.present(r,module.close,module.submit)end
    local gui=Instance.new('ScreenGui');gui.Name='PlayerFeedbackPrompt';gui.DisplayOrder=1000;gui.ResetOnSpawn=false;r.gui=gui
    local f=Instance.new('Frame');f.Name='Dialog';f.Size=UDim2.fromOffset(460,340);f.Position=UDim2.new(.5,-230,.5,-170);f.BackgroundColor3=Color3.fromRGB(35,37,39);f.Parent=gui
    local function label(name,text,y,h,size)
        local l=Instance.new('TextLabel');l.Name=name;l.Text=text;l.Size=UDim2.new(1,-40,0,h);l.Position=UDim2.fromOffset(20,y);l.TextWrapped=true;l.TextSize=size;l.TextColor3=Color3.new(1,1,1);l.BackgroundTransparency=1;l.Parent=f;return l
    end
    label('Title',r.reason and'Feedback Unavailable'or'Share Your Feedback',18,48,24)
    local function button(name,text,x,y,width,fn)
        local b=Instance.new('TextButton');b.Name=name;b.Text=text;b.TextSize=18;b.Size=UDim2.fromOffset(width,36);b.Position=UDim2.fromOffset(x,y);b.Activated:Connect(fn);b.Parent=f;return b
    end
    button('CloseAffordance','×',422,4,32,function()module.close(r)end)
    if r.reason then
        local messages=unavailable[r.reason]or{'Feedback is unavailable right now.','Please try again later.'}
        label('Header',messages[1],90,62,20);label('Hint',messages[2],160,62,18)
        button('OkayButton','OK',20,278,420,function()if clock()-r.opened>=2 then module.close(r)end end).Modal=true
    else
        label('Intro','Tell us about your experience.',72,30,18)
        local text=Instance.new('TextBox');text.Name='FeedbackTextBox';text.Position=UDim2.fromOffset(20,112);text.Size=UDim2.fromOffset(420,95);text.Text='';text.PlaceholderText='Tell us what you think...';text.TextSize=18;text.TextWrapped=true;text.MultiLine=true;text.ClearTextOnFocus=false;text.Parent=f
        label('Footer','Responses are shared anonymously and constitute feedback per the Roblox Terms of Use.',216,48,14)
        local submit=button('Submit','Submit',20,278,420,function()module.submit(r,text.Text)end);submit.Modal=true
    end
    local player=game:GetService('Players').LocalPlayer;gui.Parent=player and player:FindFirstChild('PlayerGui')or game:GetService('CoreGui')
end
local function show(r)
    if not client()then return end
    table.insert(queue,r);advance()
end
function module.prompt(service,options)
    if options~=nil then
        local kind=type(options)
        if kind~='table'then error('Unable to cast '..(({boolean='bool',number='double',['function']='Function'})[kind]or typeof(options))..' to Dictionary',0)end
        if #options>0 then error('Unable to cast Array to Dictionary',0)end
    end
    if pending then return end
    local r={deadline=clock()+30};pending=r
    local function timeout()if pending==r then complete(r)end end
    if ctx.delay then ctx.delay(30,timeout)else task.delay(30,timeout)end
    spawn(function()
        local place=ctx.place and ctx.place()or game.PlaceId
        local asset=place
        local universe=ctx.universe and ctx.universe()or game.GameId
        if universe>0 then
            local ok,value=pcall(request,'root',universe)
            if ok and type(value)=='table'and type(value.data)=='table'and type(value.data[1])=='table'and type(value.data[1].rootPlaceId)=='number'and value.data[1].rootPlaceId>0 then asset=value.data[1].rootPlaceId end
        end
        if pending~=r then return end
        -- Native submit falls back to current PlaceId. Root
        -- resolution changes the eligibility target, not that fallback.
        r.asset=place
        local ok,value=pcall(request,'eligibility',asset)
        if not ok and tostring(value):find('Unable to convert response to JSON',1,true)then ok=true;value=nil end
        if pending~=r then return end
        if ok then r.reason=module.classify(value,ctx.under13 and ctx.under13()or false)else r.reason='RateLimited'end
        show(r)
        if r.reason then complete(r)end
    end)
    while not r.done do if clock()>=r.deadline then complete(r);break end;wait()end
end
return module
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_capabilities_match_native_conversions_and_priority()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local m=assert(loadstring(source))({})
for _,case in {{false,'bool'},{123,'double'},{'x','string'},{{1},'Array'}}do local ok,e=pcall(m.prompt,{},case[1]);assert(not ok and tostring(e):find('Unable to cast '..case[2]..' to Dictionary',1,true),tostring(e))end
assert(m.classify(nil,false)=='RateLimited'and m.classify(nil,true)=='Under13')
assert(m.classify({capabilities={can_submit_review=true}},true)==nil)
assert(m.classify({can_submit_review='FALSE',canSubmitReview=true},false)=='RateLimited')
assert(m.classify({can_submit_review='invalid',canSubmitReview='TRUE'},false)==nil)
assert(m.classify({can_submit_review=0,can_submit_comment='0',can_submit_category_options=2},false)=='Under13')
assert(m.classify({can_submit_comment='true',can_submit_category_options=true},false)=='RateLimited')
assert(m.classify({can_submit_review=0.5},false)==nil)
assert(m.classify({capabilities={canSubmitReview='1'}},false)==nil)
assert(m.classify({can_submit_review=true,capabilities={}},false)=='RateLimited')
"#).exec()
    }
    #[test]
    fn feedback_dialog_cancellation_submission_queue_and_watchdog()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local function fixture(eligible,mode)
    local now=0;local jobs={};local timers={};local reads={};local posts={};local renders={};local current;local m
    local ctx={clock=function()return now end,place=function()return 10 end,universe=function()return 7 end,client=function()return true end,
        spawn=function(f)table.insert(jobs,f)end,delay=function(seconds,f)table.insert(timers,{now+seconds,f})end,
        request=function(kind,id,text)
            table.insert(reads,kind)
            if kind=='root'then assert(id==7);return{data={{rootPlaceId=99}}}end
            if kind=='submit'then assert(id==10);table.insert(posts,text);if mode=='postFailure'then error('HTTP 429')end;return nil end
            assert(id==99)
            if mode=='network'then error('HTTP 503')end
            return{capabilities={can_submit_review=eligible}}
        end,
        present=function(r,close,submit)
            current=r;table.insert(renders,r)
            -- A second request while an original is pending completes nil.
            assert(m.prompt({})==nil)
            if mode=='cancel'then close(r)end
        end,
        wait=function()
            now+=1
            local running=jobs;jobs={};for _,f in running do f()end
            if current and not current.submitting and(mode=='submit'or mode=='postFailure')and now>=3 then
                m.submit(current,'too short');assert(#posts==0)
                m.submit(current,string.rep('a',501));assert(#posts==0)
                m.submit(current,'mock-only feedback text')
            end
            for i=#timers,1,-1 do if timers[i][1]<=now then local f=table.remove(timers,i)[2];f()end end
        end}
    m=assert(loadstring(source))(ctx)
    assert(m.prompt({},{FeedbackType='ignored while native flag disabled'})==nil)
    return m,reads,posts,renders,function()return now end
end
local m,reads,posts,renders,now=fixture(false)
assert(#reads==2 and #renders==1 and renders[1].reason=='RateLimited'and #posts==0 and now()<30)
m.close(renders[1]);assert(#posts==0)
for _,mode in {'cancel','submit','postFailure','network','timeout'}do
    local m,reads,posts,renders,now=fixture(true,mode)
    assert(#renders==1)
    if mode=='submit'or mode=='postFailure'then assert(#posts==1 and posts[1]=='mock-only feedback text')else assert(#posts==0)end
    if mode=='timeout'then assert(now()==30);m.submit(renders[1],'stale after watchdog');assert(#posts==0)end
    if mode=='network'then assert(renders[1].reason=='RateLimited')end
end
"#).exec()
    }
}
