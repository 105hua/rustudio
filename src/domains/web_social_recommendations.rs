//! Native recommendation mutation serializers/response parser. Development
//! invokes mutation transports only against localhost, never live Roblox.
use super::*;
pub fn install(lua:&Lua)->mlua::Result<()> {
    let ctx=lua.create_table()?;
    ctx.set("begin",lua.create_function(|lua,(kind,client,body):(String,bool,String)| {
        use rustudio_account::web_social::RecommendationMutation;
        let kind=match kind.as_str(){"register"=>RecommendationMutation::Register,"update"=>RecommendationMutation::Update,"remove"=>RecommendationMutation::Remove{client},_=>return Err(rt_err("Unknown recommendation operation"))};
        let body=serde_json::from_str(&body).map_err(|e|rt_err(e.to_string()))?;
        http::background(lua,move |session,_|kind.execute(&session,body).map_err(|e|e.to_string()))
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let module:Table=lua.load(SOURCE).set_name("=rustudio.web_social.recommendations").call(ctx)?;
    let target:Table=lua.globals().get("__rs_lua_methods")?;
    for pair in module.get::<Table>("methods")?.pairs::<String,mlua::Function>() {
        let (name,value)=pair?;target.set(name,value)?;
    }
    lua.set_named_registry_value("web_social.recommendations",module)
}
pub const SOURCE:&str=r#"
local ctx=...;local module={methods={}}
local function server()return ctx.server and ctx.server()or(not ctx.server and game:GetService('RunService'):IsServer())end
local function enum(v,kind)
    if typeof(v)=='EnumItem'and tostring(v.EnumType)=='Enum.'..kind then return v.Name end
end
local function number(v)if type(v)=='number'then return v<0 and math.ceil(v)or math.floor(v)end end
local function array(t)
    if type(t)~='table'then return false end
    local n=0;for k in t do if type(k)~='number'or k%1~=0 or k<1 then return false end;n+=1 end
    return n==#t
end
local function attributes(request,out)
    if request.Attributes~=nil then
        if not array(request.Attributes)then error('Attributes must be an array.',0)end
        out.contentAttributes={}
        for _,entry in request.Attributes do
            if type(entry)~='table'then error('Attributes must be an array of tables.',0)end
            local converted={}
            converted.assetId=number(entry.AssetId)
            for key,name in {Text='text',Description='contentDescription'}do
                if type(entry[key])=='string'and entry[key]~=''then converted[name]=entry[key]end
            end
            for key,name in {SeekStartTime='seekStartTimeSeconds',SeekEndTime='seekEndTimeSeconds',TrimStartTime='trimStartTimeSeconds',TrimEndTime='trimEndTimeSeconds'}do
                if type(entry[key])=='number'then converted[name]=entry[key]end
            end
            table.insert(out.contentAttributes,converted)
        end
    end
    if request.CustomTags~=nil then
        if not array(request.CustomTags)then error('CustomTag must be a string array',0)end
        out.customTags={}
        for _,tag in request.CustomTags do if type(tag)~='string'then error('CustomTags must be a string array',0)end;table.insert(out.customTags,tag)end
    end
end
local function request(kind,body)
    if ctx.request then return ctx.request(kind,not server(),body)end
    local http=game:GetService('HttpService');local token=ctx.begin(kind,not server(),http:JSONEncode(body))
    while true do local done,body,err=ctx.poll(token);if done then if err then error(err,0)end;return http:JSONDecode(body)end;task.wait()end
end
local function dictionary(v,index)
    if type(v)~='table'then error('Argument '..index..' missing or nil',0)end
end
module.methods.RegisterItemAsync=function(s,player,input)
    dictionary(input,2)
    if not server()then error('RegisterItemAsync must run as Server Script.',0)end
    if typeof(player)~='Instance'or not player:IsA('Player')then error('RegisterItemAsync must be called with a valid player.',0)end
    if type(input.ReferenceId)~='string'then error('RegisterRecommendationItemRequest must contain ReferenceId',0)end
    local allowed={'ReferenceId','Duration','Visibility','Attributes','CustomTags','ContentType'}
    for key in input do if not table.find(allowed,key)then error('Unexpected key in RegisterRecommendationItemRequest: '..tostring(key)..'. Allowed keys are: '..table.concat(allowed,', '),0)end end
    local item={referenceId=input.ReferenceId,creator={creatorId=player.UserId,creatorType='User'}}
    item.contentType=enum(input.ContentType,'RecommendationItemContentType')
    item.durationSeconds=number(input.Duration)
    item.visibilityStatus=enum(input.Visibility,'RecommendationItemVisibility')
    attributes(input,item)
    return request('register',{experienceId=ctx.universe and ctx.universe()or game.GameId,items={item}})
end
module.methods.UpdateItemAsync=function(s,input)
    dictionary(input,1)
    if not server()then error('UpdateItemAsync must run as Server Script.',0)end
    if type(input.ItemId)~='string'then error('UpdateRecommendationItemRequest must contain ItemId',0)end
    local item={itemId=input.ItemId}
    if type(input.ReferenceId)=='string'then item.referenceId=input.ReferenceId end
    item.durationSeconds=number(input.Duration);item.visibilityStatus=enum(input.Visibility,'RecommendationItemVisibility')
    local creator=input.Creator
    if type(creator)=='table'and enum(creator.CreatorType,'CreatorType')=='User'and type(creator.CreatorId)=='number'then item.creator={creatorId=number(creator.CreatorId),creatorType='User'}end
    attributes(input,item);request('update',item)
end
module.methods.RemoveItemAsync=function(s,id)
    if id==nil then error('Argument 1 missing or nil',0)end
    if type(id)~='string'and type(id)~='number'then error('Unable to cast value to std::string',0)end
    request('remove',{itemId=tostring(id)})
end
return module
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recommendation_serializers_and_request_context()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local server=true;local calls={};local p={UserId=5,IsA=function(_,name)return name=='Player'end}
local old=typeof;typeof=function(v)if v==p then return 'Instance'elseif type(v)=='table'and v.EnumType then return 'EnumItem'end;return old(v)end
local content={Name='Static',EnumType='Enum.RecommendationItemContentType'};local visible={Name='Public',EnumType='Enum.RecommendationItemVisibility'}
local module=assert(loadstring(source))({server=function()return server end,universe=function()return 7 end,request=function(kind,client,body)table.insert(calls,{kind=kind,client=client,body=body});return {ItemId='mock-item',ReferenceId='test'}end})
local m=module.methods;local s={}
local result=m.RegisterItemAsync(s,p,{ReferenceId='test',ContentType=content,Visibility=visible,Duration=1.75,Attributes={{AssetId=8,Text='hello',SeekStartTime=1.5}},CustomTags={'a','b'}})
assert(result.ItemId=='mock-item'and #calls==1 and calls[1].kind=='register'and not calls[1].client)
local body=calls[1].body;local item=body.items[1];assert(body.experienceId==7 and item.creator.creatorId==5 and item.creator.creatorType=='User'and item.contentType=='Static'and item.durationSeconds==1 and item.visibilityStatus=='Public'and item.contentAttributes[1].assetId==8 and item.contentAttributes[1].text=='hello'and item.contentAttributes[1].seekStartTimeSeconds==1.5 and item.customTags[2]=='b')
assert(select('#',m.UpdateItemAsync(s,{ItemId='mock-item',ReferenceId='new',Duration=-2.8,Attributes={{Description='desc'}}}))==0)
assert(calls[2].kind=='update'and calls[2].body.itemId=='mock-item'and calls[2].body.durationSeconds==-2 and calls[2].body.contentAttributes[1].contentDescription=='desc')
local checks={
 {function()m.RegisterItemAsync(s,p,{})end,'RegisterRecommendationItemRequest must contain ReferenceId'},
 {function()m.RegisterItemAsync(s,p,{ReferenceId='test',Unexpected=1})end,'Unexpected key in RegisterRecommendationItemRequest: Unexpected. Allowed keys are: ReferenceId, Duration, Visibility, Attributes, CustomTags, ContentType'},
 {function()m.RegisterItemAsync(s,{}, {ReferenceId=''})end,'RegisterItemAsync must be called with a valid player.'},
 {function()m.UpdateItemAsync(s,{})end,'UpdateRecommendationItemRequest must contain ItemId'},
 {function()m.UpdateItemAsync(s,{ItemId='',Attributes={Text='wrong'}})end,'Attributes must be an array.'},
 {function()m.UpdateItemAsync(s,{ItemId='',Attributes={2}})end,'Attributes must be an array of tables.'},
 {function()m.UpdateItemAsync(s,{ItemId='',CustomTags='wrong'})end,'CustomTag must be a string array'},
 {function()m.UpdateItemAsync(s,{ItemId='',CustomTags={2}})end,'CustomTags must be a string array'},
}
for _,check in checks do local ok,e=pcall(check[1]);assert(not ok and e==check[2],tostring(e))end
assert(#calls==2);server=false
local ok,e=pcall(m.RegisterItemAsync,s,p,{ReferenceId='test'});assert(not ok and e=='RegisterItemAsync must run as Server Script.')
ok,e=pcall(m.UpdateItemAsync,s,{ItemId='mock-item'});assert(not ok and e=='UpdateItemAsync must run as Server Script.')
assert(select('#',m.RemoveItemAsync(s,'mock-item'))==0 and calls[3].client and calls[3].kind=='remove'and calls[3].body.itemId=='mock-item')
"#).exec()
    }
}
