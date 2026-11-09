//! Studio's local EditableImage -> Decal builder; no asset upload is involved.
//! Native helper , runtime probes in parity notes.
use super::*;
pub fn install(lua:&Lua)->mlua::Result<()>{
    lua.globals().set("__ac_create_decal",lua.create_function(|lua,maps:mlua::Table|{
        let result=(|| ->mlua::Result<Value>{
        let mut content=Vec::new();
        let maps=maps.pairs::<String,Value>().collect::<mlua::Result<std::collections::BTreeMap<_,_>>>()?;
        for(key,value)in maps{
            if !["TextureContent","NormalMapContent","MetalnessMapContent","RoughnessMapContent"].contains(&key.as_str()){
                world(lua)?.output.push((Level::Warn,format!("AssetService:CreateDecalAsync: Unknown content parameter: {key}")));
                continue
            }
            let c=crate::runtime::editable::content_of(&value);
            let valid=c.as_ref().and_then(|c|c.as_object()).is_some_and(|r|world(lua).ok().is_some_and(|w|w.d().dom.get_by_ref(r).is_some_and(|i|i.class=="EditableImage")));
            if !valid{return Err(rt_err(format!("AssetService:CreateDecalAsync: {key} value should be an EditableImage Content")))}
            content.push((key,c.unwrap()));
        }
        if content.is_empty(){return Err(rt_err("AssetService:CreateDecalAsync: No Content Maps provided"))}
        let r=crate::runtime::ext::create_any(lua,"Decal")?;
        for(key,value)in content{store_prop(lua,r,&key,Variant::Content(value))?;}
        inst_value(lua,r)
        })();
        Ok(match result{Ok(value)=>(value,None),Err(e)=>(Value::Nil,Some(crate::runtime::describe_error(&e)))})
    })?)?;
    lua.load(r#"
local M=__rs_lua_methods
local function dictionary(content)
    if content==nil then error('Argument 1 missing or nil',3) end
    if type(content)~='table' then error('Unable to cast value to Dictionary',3) end
end
M.CreateDecalAsync=function(self,content)
    dictionary(content)
    task.wait()
    local value,err=__ac_create_decal(content)
    if err then error(err,0) end
    return value
end
M.CreateTextureAsync=function(self,content)
    dictionary(content)
    task.wait()
    error('AssetService::CreateTextureAsync is not yet enabled',0)
end
"#).set_name("=rustudio.assets_cloud.decal").exec()
}
