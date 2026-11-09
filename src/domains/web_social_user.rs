//! Domain-scoped User values. Native constructor/serialization measured using
//! pure User.fromId/fromString; no private identity was allocated live.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DomainUser {
    id: i64,
    kind: u8,
    domain: u64,
}
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
impl DomainUser {
    fn encode(self) -> String {
        let mut bytes = vec![self.kind];
        bytes.extend(self.domain.to_be_bytes());
        bytes.extend(self.id.to_be_bytes());
        let mut out = String::from("U1.");
        for chunk in bytes.chunks(3) {
            let n = (chunk[0] as u32) << 16
                | (chunk.get(1).copied().unwrap_or(0) as u32) << 8
                | chunk.get(2).copied().unwrap_or(0) as u32;
            out.push(BASE64[((n >> 18) & 63) as usize] as char);
            out.push(BASE64[((n >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                out.push(BASE64[((n >> 6) & 63) as usize] as char);
            }
            if chunk.len() > 2 {
                out.push(BASE64[(n & 63) as usize] as char);
            }
        }
        out
    }
    fn decode(value: &str) -> mlua::Result<Self> {
        let payload = value
            .strip_prefix("U1.")
            .ok_or_else(|| rt_err("User string is missing required prefix"))?;
        let invalid = || rt_err("User string has invalid payload length");
        let mut bytes = Vec::new();
        let mut bits = 0u32;
        let mut count = 0;
        for c in payload.bytes().take_while(|c| *c != b'=') {
            let n = BASE64.iter().position(|x| *x == c).ok_or_else(invalid)?;
            bits = (bits << 6) | n as u32;
            count += 6;
            if count >= 8 {
                count -= 8;
                bytes.push((bits >> count) as u8);
            }
        }
        if bytes.len() != 17 {
            return Err(invalid());
        }
        if !matches!(bytes[0], 1 | 3) {
            return Err(rt_err("User string has invalid domain type"));
        }
        Ok(Self {
            kind: bytes[0],
            domain: u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
            id: i64::from_be_bytes(bytes[9..17].try_into().unwrap()),
        })
    }
}
impl mlua::UserData for DomainUser {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field("__type", "User");
    }
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(
            mlua::MetaMethod::ToString,
            |_, value, ()| Ok(value.encode()),
        );
        methods.add_method("ToString", |_, value, ()| Ok(value.encode()));
        methods.add_meta_method(mlua::MetaMethod::Eq, |_, value, other: Value| {
            Ok(opt_ud::<DomainUser>(&other).is_some_and(|x| x == *value))
        });
        methods.add_meta_method(mlua::MetaMethod::Index, |lua, value, key: String| match key
            .as_str()
        {
            "Id" => Ok(Value::Number(value.id as f64)),
            "DomainId" => Ok(Value::Number(value.domain as f64)),
            "DomainType" => crate::ltypes::enum_value(
                lua,
                LEnumItem::from_name(
                    "DomainType",
                    if value.kind == 1 {
                        "EXPERIENCE"
                    } else {
                        "OAUTH"
                    },
                )
                .ok_or_else(|| rt_err("Invalid value for enum 'DomainType'"))?,
            ),
            _ => Err(rt_err(format!("{key} is not a valid member of User"))),
        });
        methods.add_meta_method(
            mlua::MetaMethod::NewIndex,
            |_, _, (key, _): (String, Value)| {
                Err::<(), _>(rt_err(
                    if matches!(key.as_str(), "Id" | "DomainId" | "DomainType") {
                        format!("{key} member of User is read-only and cannot be assigned to")
                    } else {
                        format!("{key} is not a valid member of User")
                    },
                ))
            },
        );
    }
}
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let table = lua.create_table()?;
    table.set("fromId",lua.create_function(|lua,args:MultiValue| {
        if !matches!(args.len(),1|3){return Err(rt_err("Invalid number of arguments. Expected either (domainUserId) or (domainUserId, domainType, domainId)."));}
        let id=lua.coerce_number(arg(&args,0))?.ok_or_else(||rt_err("number expected"))? as i64;
        let (kind,domain)=if args.len()==3 {
            let item=opt_ud::<LEnumItem>(&arg(&args,1)).filter(|x|x.enum_type=="DomainType").ok_or_else(||rt_err("Invalid value for enum 'DomainType'"))?;
            let domain=lua.coerce_number(arg(&args,2))?.ok_or_else(||rt_err("number expected"))? as i64 as u64;
            (if item.name=="EXPERIENCE"{1}else{3},domain)
        }else{
            let domain=world(lua).ok().and_then(|w|w.d().dom.get_by_ref(w.d().dom.root_ref()).and_then(|i|i.properties.get(&"GameId".into())).and_then(|v|match v {Variant::Int64(v)=>Some(*v as u64),Variant::Int32(v)=>Some(*v as u64),_=>None})).unwrap_or(0);
            (1,domain)
        };
        lua.create_userdata(DomainUser{id,kind,domain})
    })?)?;
    table.set(
        "fromString",
        lua.create_function(|lua, value: String| lua.create_userdata(DomainUser::decode(&value)?))?,
    )?;
    lua.globals().set("User", table)
}
pub fn install_service(lua: &Lua) -> mlua::Result<()> {
    let ctx = lua.create_table()?;
    ctx.set(
        "begin",
        lua.create_function(|lua, (global, universe): (u64, u64)| {
            http::background(lua, move |session, _| {
                rustudio_account::web_social::resolve_private_user(&session, global, universe)
                    .map_err(|e| e.to_string())
            })
        })?,
    )?;
    ctx.set("poll", lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    ctx.set(
        "value",
        lua.create_function(|lua, (id, domain): (String, u64)| {
            let id = id
                .parse::<u64>()
                .map_err(|_| rt_err("UserService: User not found"))?;
            lua.create_userdata(DomainUser {
                id: id as i64,
                kind: 1,
                domain,
            })
        })?,
    )?;
    let f: mlua::Function = lua
        .load(SERVICE)
        .set_name("=rustudio.web_social.user")
        .call(ctx)?;
    lua.globals()
        .get::<Table>("__rs_lua_methods")?
        .set("GetUserFromGlobalUserIdAsync", f)
}
const SERVICE: &str = r#"
local ctx=...;local cache={};local window=0;local count=0
local function server()return ctx.server and ctx.server()or(not ctx.server and game:GetService('RunService'):IsServer())end
return function(s,id)
    if id==nil then error('Argument 1 missing or nil',0)end
    id=tonumber(id);if not id then error('Unable to cast value to int64',0)end
    id=id<0 and math.ceil(id)or math.floor(id)
    if not server()then error('UserService: GetUserFromGlobalUserIdAsync can only be called from the server',0)end
    if id<=0 then error('UserService: Invalid userId',0)end
    if cache[id]then return cache[id]end
    local universe=ctx.universe and ctx.universe()or game.GameId
    if universe<=0 then error('UserService: GetUserFromGlobalUserIdAsync requires a valid universeId',0)end
    local now=ctx.now and ctx.now()or os.clock()
    if now-window>60 then window=now;count=0 end
    count+=1
    if count>100 then error('UserService: GetUserFromGlobalUserIdAsync is throttled. Limit: 100 requests per minute',0)end
    local private
    if ctx.request then private=ctx.request(id,universe)else
        local token=ctx.begin(id,universe)
        while true do
            local done,body,err=ctx.poll(token)
            if done then if err then error(err,0)end;private=game:GetService('HttpService'):JSONDecode(body).privateUserId;break end
            task.wait()
        end
    end
    local user=ctx.value(private,universe);cache[id]=user;return user
end
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_user_guards_cache_and_window_match_native() -> mlua::Result<()> {
        let lua = Lua::new();
        install(&lua)?;
        let ctx = lua.create_table()?;
        lua.globals().set("ctx", ctx.clone())?;
        ctx.set(
            "value",
            lua.create_function(|lua, (id, domain): (String, u64)| {
                lua.create_userdata(DomainUser {
                    id: id.parse().unwrap(),
                    kind: 1,
                    domain,
                })
            })?,
        )?;
        lua.load(r#"local n=0;ctx.server=function()return true end;ctx.universe=function()return 0 end;ctx.now=function()return 0 end;ctx.request=function(id,universe)assert(universe==7);n+=1;return tostring(id+100)end;ctx.calls=function()return n end"#).exec()?;
        let f: mlua::Function = lua.load(SERVICE).call(ctx)?;
        lua.globals().set("resolve", f)?;
        lua.load(r#"
local function fails(id,text)local ok,e=pcall(resolve,{},id);assert(not ok and tostring(e):find(text,1,true),tostring(e))end
fails(0,'UserService: Invalid userId');fails(1,'requires a valid universeId')
ctx.server=function()return false end;fails(1,'can only be called from the server');ctx.server=function()return true end
ctx.universe=function()return 7 end
local u=resolve({},1);assert(u.Id==101 and u.DomainId==7);assert(resolve({},1)==u and ctx.calls()==1)
for i=2,100 do resolve({},i)end
fails(101,'is throttled. Limit: 100 requests per minute');assert(resolve({},1)==u and ctx.calls()==100)
ctx.now=function()return 60.001 end;assert(resolve({},101).Id==201 and ctx.calls()==101)
"#).exec()
    }
    #[test]
    fn native_user_values_serialize_compare_and_reject_writes() -> mlua::Result<()> {
        let lua = Lua::new();
        install(&lua)?;
        lua.load(r#"
assert(tostring(User.fromId(0))=='U1.AQAAAAAAAAAAAAAAAAAAAAA')
assert(tostring(User.fromId(1))=='U1.AQAAAAAAAAAAAAAAAAAAAAE')
assert(tostring(User.fromId(-1))=='U1.AQAAAAAAAAAA__________8')
assert(tostring(User.fromId(123))=='U1.AQAAAAAAAAAAAAAAAAAAAHs')
assert(User.fromId('123')==User.fromString('U1.AQAAAAAAAAAAAAAAAAAAAHs='))
local u=User.fromId(123);assert(u.Id==123 and u.DomainId==0 and tostring(u.DomainType)=='Enum.DomainType.EXPERIENCE'and u:ToString()==tostring(u))
for _,field in {'Id','DomainId','DomainType'}do local ok,e=pcall(function()u[field]=1 end);assert(not ok and tostring(e):find(field..' member of User is read-only and cannot be assigned to',1,true))end
local ok,e=pcall(function()return u.Unknown end);assert(not ok and tostring(e):find('Unknown is not a valid member of User',1,true))
for _,entry in {{'bad','User string is missing required prefix'},{'U2.AQAAAAAAAAAAAAAAAAAAAHs','User string is missing required prefix'},{'U1.AQ','User string has invalid payload length'},{'U1.!!!!','User string has invalid payload length'},{'U1.AgAAAAAAAAAAAAAAAAAAAHs','User string has invalid domain type'}}do
 local ok,e=pcall(User.fromString,entry[1]);assert(not ok and tostring(e):find(entry[2],1,true))
end
"#).exec()?;
        let u = DomainUser {
            id: 123,
            kind: 1,
            domain: 7,
        };
        assert_eq!(u.encode(), "U1.AQAAAAAAAAAHAAAAAAAAAHs");
        assert_eq!(DomainUser::decode(&u.encode())?, u);
        let u = DomainUser {
            id: 123,
            kind: 3,
            domain: 8,
        };
        assert_eq!(u.encode(), "U1.AwAAAAAAAAAIAAAAAAAAAHs");
        assert_eq!(DomainUser::decode(&u.encode())?, u);
        Ok(())
    }
}
