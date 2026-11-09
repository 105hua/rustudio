//! Internal review action traced from/. It does not
//! create or upload content. Development sends this action only to localhost.
use super::*;
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let ctx = lua.create_table()?;
    ctx.set(
        "begin",
        lua.create_function(|lua, (universe, id, key): (u64, String, String)| {
            http::background(lua, move |session, _| {
                rustudio_account::web_social::request_content_review(&session, universe, &id, &key)
                    .map_err(|e| e.to_string())
            })
        })?,
    )?;
    ctx.set("poll", lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let method: mlua::Function = lua
        .load(REVIEW)
        .set_name("=rustudio.web_social.moderation")
        .call(ctx)?;
    lua.globals()
        .get::<Table>("__rs_lua_methods")?
        .set("InternalRequestReviewableContentReviewAsync", method)
}
const REVIEW: &str = r#"
local ctx=...
return function(service,config)
    if type(config)~='table'then error('Argument 1 missing or nil',0)end
    local server=ctx.server and ctx.server()or(not ctx.server and game:GetService('RunService'):IsServer())
    if not server then error('RequestReviewableContentReviewAsync() can only be called on the server',0)end
    if type(config.ReviewableContentId)~='string'then error('ReviewableContentId is not specified',0)end
    local universe=ctx.universe and ctx.universe()or game.GameId
    local key=ctx.guid and ctx.guid()or game:GetService('HttpService'):GenerateGUID(false)
    if ctx.request then ctx.request(universe,config.ReviewableContentId,key);return end
    local token=ctx.begin(universe,config.ReviewableContentId,key)
    while true do local done,body,err=ctx.poll(token);if done then if err then error(err,0)end;return end;task.wait()end
end
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_request_guards_and_response_contract() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("source", REVIEW)?;
        lua.load(r#"
local calls={};local server=false
local method=assert(loadstring(source))({server=function()return server end,universe=function()return 7 end,guid=function()return 'mock-key'end,request=function(universe,id,key)assert(universe==7 and key=='mock-key');table.insert(calls,id)end})
local function fails(config,text)local ok,e=pcall(method,{},config);assert(not ok and tostring(e):find(text,1,true),tostring(e))end
fails({},'RequestReviewableContentReviewAsync() can only be called on the server');server=true
fails({},'ReviewableContentId is not specified');fails({ReviewableContentId=123},'ReviewableContentId is not specified')
assert(method({},{ReviewableContentId='mock-id',Unknown='ignored'})==nil)
assert(method({},{ReviewableContentId=''})==nil);assert(#calls==2 and calls[1]=='mock-id'and calls[2]=='')
"#).exec()
    }
}
