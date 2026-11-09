//! Native Studio recommendation logging validates/builds, then discards its
//! event. Both shipped binaries omit the dispatch call used by other events:
//! old/60b0/63d0; online/9950/9c70. No telemetry HTTP.
use super::*;
fn cast_kind(value: &Value) -> String {
    if let Some(item) = opt_ud::<LEnumItem>(value) {
        return item.enum_type;
    }
    match value {
        Value::Number(_) | Value::Integer(_) => "double",
        Value::Boolean(_) => "bool",
        Value::String(_) => "string",
        Value::Table(_) => "Dictionary",
        Value::Nil => "nil",
        _ => "Object",
    }
    .into()
}
fn enum_name(_: &Lua, (kind, value, index): (String, Value, usize)) -> mlua::Result<String> {
    if matches!(value, Value::Nil) {
        return Err(rt_err(format!("Argument {index} missing or nil")));
    }
    crate::ltypes::enum_arg(Some(&value), &kind, "")
        .map(|v| v.name)
        .map_err(|_| rt_err(format!("Unable to cast {} to {kind}", cast_kind(&value))))
}
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let ctx = lua.create_table()?;
    ctx.set("enum", lua.create_function(enum_name)?)?;
    ctx.set(
        "optional_enum",
        lua.create_function(|_, (kind, value): (String, Value)| {
            Ok(opt_ud::<LEnumItem>(&value)
                .filter(|v| v.enum_type == kind)
                .map(|v| v.name))
        })?,
    )?;
    let module: Table = lua
        .load(SOURCE)
        .set_name("=rustudio.web_social.recommendation_logs")
        .call(ctx)?;
    let target: Table = lua.globals().get("__rs_lua_methods")?;
    for pair in module.pairs::<String, mlua::Function>() {
        let (k, v) = pair?;
        target.set(k, v)?;
    }
    Ok(())
}
const SOURCE: &str = r#"
local ctx=...;local methods={}
local function client()return ctx.client and ctx.client()or(not ctx.client and game:GetService('RunService'):IsClient())end
local function string_arg(value,index,optional)
    if value==nil then if optional then return''end;error('Argument '..index..' missing or nil',0)end
    if type(value)~='number'and type(value)~='string'then error('Unable to cast value to std::string',0)end
    return tostring(value)
end
local function dictionary(value)
    if value==nil then return end
    local kind=type(value)
    if kind~='table'then error('Unable to cast '..(({number='double',boolean='bool'})[kind]or kind)..' to Dictionary',0)end
    local array=#value>0
    if array then for k in value do if type(k)~='number'or k<1 or k%1~=0 or k>#value then array=false;break end end end
    if array then error('Unable to cast Array to Dictionary',0)end
end
local function uuid(id)
    local a,b,c,d,e=id:match('^([0-9a-f]+)%-([0-9a-f]+)%-([0-9a-f]+)%-([0-9a-f]+)%-([0-9a-f]+)$')
    return a and #a==8 and #b==4 and #c==4 and #d==4 and #e==12
end
local function validate(details,kind,allowed)
    if not details then return end
    local keys={};for key in details do table.insert(keys,tostring(key))end;table.sort(keys)
    for _,key in keys do if not table.find(allowed,key)then error('Unexpected key in '..kind..': '..key..'. Allowed keys are: '..table.concat(allowed,', '),0)end end
end
local function numeric(input,output,key,name,integer)
    local value=input[key];if type(value)=='number'then output[name]=integer and(value<0 and math.ceil(value)or math.floor(value))or value end
end
local function build(kind,typ,item,tracing,details)
    dictionary(details)
    if not client()then error('LogImpressionEvent should only be called with Client Run Context',0)end
    if item==''then error('itemId is empty',0)end
    if not uuid(item)then error('itemId is not a UUID, please provide the itemId returned from RegisterItemAsync or GenerateItemListAsync',0)end
    if tracing==''then error('tracingId is empty',0)end
    local out={item_id=item,tracing_id=tracing};local event
    if kind=='action'then
        event='RecommendationActionEvent';out.action_type=typ
        validate(details,'RecommendationActionEventDetails',{'Weight','ReactionType','DestinationPlaceId','CommentText'})
        if details then
            numeric(details,out,'Weight','action_weight');numeric(details,out,'DestinationPlaceId','destination_place_id',true)
            for key,name in {ReactionType='reaction_type',CommentText='comment_text'}do if type(details[key])=='string'then out[name]=details[key]end end
        end
    else
        event='RecommendationImpressionEvent';out.impression_type=typ
        validate(details,'RecommendationImpressionEventDetails',{'Duration','ItemPosition','Weight','DepartureIntent'})
        if details then
            numeric(details,out,'Duration','duration_seconds');numeric(details,out,'ItemPosition','item_position',true);numeric(details,out,'Weight','weight')
            out.departure_intent=ctx.optional_enum('RecommendationDepartureIntent',details.DepartureIntent)
        end
    end
    -- Deliberately no dispatch: native destroys this constructed event without
    -- invoking EventStream's send routine. Observer exists only in unit fixtures.
    if ctx.observe then ctx.observe(event,out)end
end
methods.LogActionEvent=function(s,typ,item,tracing,details)
    typ=ctx.enum('RecommendationActionType',typ,1);item=string_arg(item,2);tracing=string_arg(tracing,3)
    build('action',typ,item,tracing,details)
end
methods.LogImpressionEvent=function(s,typ,item,tracing,details)
    typ=ctx.enum('RecommendationImpressionType',typ,1);item=string_arg(item,2);tracing=string_arg(tracing,3)
    build('impression',typ,item,tracing,details)
end
methods.LogPreferenceEvent=function(s,typ,target,target_id,tracing,item)
    typ=ctx.enum('RecommendationPreferenceType',typ,1);target=ctx.enum('RecommendationPreferenceTargetType',target,2)
    target_id=string_arg(target_id,3);tracing=string_arg(tracing,4,true);item=string_arg(item,5,true)
    if not client()then error('LogPreferenceEvent should only be called with Client Run Context',0)end
    if target_id==''then error('targetId is empty',0)end
    local out={preference_type=typ,target_type=target,target_id=target_id}
    if tracing~=''then out.tracing_id=tracing end;if item~=''then out.item_id=item end
    if ctx.observe then ctx.observe('RecommendationPreferenceEvent',out)end
end
return methods
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_log_guards_fields_and_no_dispatch() -> mlua::Result<()> {
        let lua = Lua::new();
        crate::ltypes::install(&lua, &lua.globals())?;
        let ctx = lua.create_table()?;
        ctx.set("enum", lua.create_function(enum_name)?)?;
        ctx.set(
            "optional_enum",
            lua.create_function(|_, (kind, value): (String, Value)| {
                Ok(opt_ud::<LEnumItem>(&value)
                    .filter(|v| v.enum_type == kind)
                    .map(|v| v.name))
            })?,
        )?;
        lua.globals().set("ctx", ctx.clone())?;
        lua.load("ctx.events={};ctx.client=function()return true end;ctx.observe=function(name,data)table.insert(ctx.events,{name,data})end").exec()?;
        let methods: Table = lua.load(SOURCE).call(ctx)?;
        lua.globals().set("m", methods)?;
        lua.load(r#"
local id='12345678-1234-1234-1234-123456789abc'
local function fails(f,text)local ok,e=pcall(f);assert(not ok and tostring(e):find(text,1,true),tostring(e))end
local action=Enum.RecommendationActionType.AddReaction
ctx.client=function()return false end;fails(function()m.LogActionEvent({},action,'','')end,'LogImpressionEvent should only be called with Client Run Context');ctx.client=function()return true end
fails(function()m.LogActionEvent({},action,'','')end,'itemId is empty')
for _,value in {'bad',string.upper(id),'{'..id..'}',string.gsub(id,'%-','')}do fails(function()m.LogActionEvent({},action,value,'')end,'itemId is not a UUID')end
fails(function()m.LogActionEvent({},action,id,'')end,'tracingId is empty')
fails(function()m.LogActionEvent({},action,'bad','',1)end,'Unable to cast double to Dictionary')
fails(function()m.LogActionEvent({},action,id,'trace',{Unknown=1})end,'Unexpected key in RecommendationActionEventDetails: Unknown. Allowed keys are: Weight, ReactionType, DestinationPlaceId, CommentText')
fails(function()m.LogActionEvent({},'bad',id,'trace')end,'Unable to cast string to RecommendationActionType')
assert(#ctx.events==0)
assert(m.LogActionEvent({},action,id,'trace',{Weight=.5,ReactionType='Like',DestinationPlaceId=7.9,CommentText=''})==nil)
local event=ctx.events[1];assert(event[1]=='RecommendationActionEvent'and event[2].action_type=='AddReaction'and event[2].action_weight==.5 and event[2].destination_place_id==7 and event[2].reaction_type=='Like'and event[2].comment_text=='')
assert(m.LogImpressionEvent({},0,id,'trace',{Duration=.75,ItemPosition=-2.9,Weight=1,DepartureIntent=Enum.RecommendationDepartureIntent.Positive})==nil)
event=ctx.events[2];assert(event[1]=='RecommendationImpressionEvent'and event[2].impression_type=='View'and event[2].duration_seconds==.75 and event[2].item_position==-2 and event[2].departure_intent=='Positive')
assert(m.LogPreferenceEvent({},'AddFollow','User','123')==nil)
event=ctx.events[3];assert(event[1]=='RecommendationPreferenceEvent'and event[2].target_id=='123'and event[2].tracing_id==nil and event[2].item_id==nil)
-- Fixtures observe construction, not a production telemetry transport.
ctx.observe=nil;assert(m.LogActionEvent({},action,id,'trace')==nil)
"#).exec()
    }
}
