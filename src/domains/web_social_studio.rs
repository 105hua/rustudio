//! Behaviors observed in the owned unpublished Studio Play probe. These
//! services do not make live writes in Studio.
use super::*;
#[derive(Clone)]
struct AdReward(i64);
impl mlua::UserData for AdReward {
    fn add_fields<F: mlua::UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "AdReward");
    }
    fn add_methods<M: mlua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "AdReward {{ type = DevProduct, productId = {} }}",
                this.0
            ))
        });
        m.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: Value| {
            Ok(opt_ud::<AdReward>(&other).is_some_and(|v| v.0 == this.0))
        });
        m.add_meta_method(mlua::MetaMethod::Index, |_, _, key: String| {
            Err::<Value, _>(rt_err(format!("'{key}' is not a valid member of AdReward")))
        });
    }
}
// Native SocialService signals CoreScripts.
// Headless Studio has no camera/self-view window; the signals still dispatch.
fn self_view_position(value: &Value) -> mlua::Result<LEnumItem> {
    let result = crate::ltypes::enum_arg(Some(value), "SelfViewPosition", "LastPosition");
    result.map_err(|_| {
        let kind = if let Some(e) = opt_ud::<LEnumItem>(value) { e.enum_type }
            else { match value { Value::Integer(_) | Value::Number(_) => "double", Value::Boolean(_) => "bool", Value::String(_) => "string", Value::Table(_) => "Dictionary", _ => "Object" }.to_owned() };
        rt_err(format!("Unable to cast {kind} to SelfViewPosition"))
    })
}
fn bind_reviewable(lua: &Lua, priority: i32, callback: mlua::Function) -> mlua::Result<Value> {
    let state = state(lua)?;
    let entries = if let Some(t) = state.raw_get::<Option<Table>>("review.handlers")? {
        t
    } else {
        let t = lua.create_table()?;
        state.raw_set("review.handlers", t.clone())?;
        t
    };
    let id = state.raw_get::<Option<u64>>("review.next")?.unwrap_or(0) + 1;
    state.raw_set("review.next", id)?;
    let record = lua.create_table()?;
    record.set("priority", priority)?;
    record.set("callback", callback)?;
    entries.raw_set(id, record)?;
    let disconnect = lua.create_function(move |_, ()| entries.raw_set(id, Value::Nil))?;
    Ok(Value::UserData(purchase::connection(lua, disconnect)?))
}
// CaptureService owns capture creation. Observe its completed local capture so
// Content.fromObject can resolve the same bytes as the legacy rbxtemp URI.
pub fn observe_captures(lua: &Lua) -> mlua::Result<()> {
    let data = state(lua)?;
    if data.raw_get::<Option<bool>>("review.capture_observer")?.unwrap_or(false) {
        return Ok(());
    }
    let Ok(poll) = lua.globals().get::<mlua::Function>("__ac_capture_poll") else {
        return Ok(());
    };
    lua.globals().set("__ac_capture_poll", lua.create_function(move |lua, token: u64| {
        let values: MultiValue = poll.call(token)?;
        if let (Some(Value::Boolean(true)), Some(Value::String(uri)), Some(object)) =
            (values.front(), values.get(1), values.get(2))
        {
            if let Some(r) = opt_inst(object) {
                state(lua)?.raw_set(format!("review.capture:{r:?}"), uri.clone())?;
            }
        }
        Ok(values)
    })?)?;
    data.raw_set("review.capture_observer", true)
}
fn key_encoding(id: &str, mime: &str, length: usize) -> String {
    // writes this order, compact JSON, then padded Base64.
    let json = format!(r#"{{"id":"{id}","contentType":"{mime}","contentLength":{length}}}"#);
    const T: &[u8;64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(json.len().div_ceil(3)*4);
    for c in json.as_bytes().chunks(3) {
        let n = (c[0] as u32)<<16 | (*c.get(1).unwrap_or(&0) as u32)<<8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n>>18) as usize & 63] as char);
        out.push(T[(n>>12) as usize & 63] as char);
        out.push(if c.len()>1 { T[(n>>6) as usize & 63] as char } else {'='});
        out.push(if c.len()>2 { T[n as usize & 63] as char } else {'='});
    }
    out
}
fn reviewable_key(lua: &Lua, value: Value) -> mlua::Result<Value> {
    let content = crate::runtime::editable::content_of(&value)
        .ok_or_else(|| rt_err("Unsupported content format"))?;
    let state = state(lua)?;
    let (uri, mime) = if let Some(uri) = content.as_uri() {
        let known = lua.globals().get::<mlua::Function>("__ac_capture_known")
            .and_then(|f| f.call::<bool>((value.clone(), false))).unwrap_or(false);
        if !known {
            return Err(rt_err(format!("Content URI {uri} does not refer to capture from CaptureService:CaptureScreenshot")));
        }
        (uri.to_owned(), "image/png")
    } else if let Some(r) = content.as_object() {
        let class = inst_meta(lua, r)?.0;
        let mime = match class.as_str() {
            "ScreenshotCapture" => "image/png",
            "VideoCapture" => "video/mp4",
            _ => return Err(rt_err("Unsupported content format")),
        };
        let uri = state.raw_get::<Option<String>>(format!("review.capture:{r:?}"))?
            .ok_or_else(|| rt_err("Unsupported content format"))?;
        (uri, mime)
    } else {
        return Err(rt_err("Unsupported content format"));
    };
    let bytes = crate::assets::generated(&uri).ok_or_else(|| rt_err("Unsupported content format"))?;
    let id: String = lua.load("return game:GetService('HttpService'):GenerateGUID(false)").eval()?;
    let id = id.to_uppercase();
    state.raw_set(format!("review.content:{id}"), value)?;
    Ok(Value::String(lua.create_string(key_encoding(&id, mime, bytes.len()))?))
}
pub fn call(
    lua: &Lua,
    name: &str,
    this: Ref,
    args: &MultiValue,
) -> Option<mlua::Result<MultiValue>> {
    if !matches!(
        name,
        "HideSelfView"
            | "ShowSelfView"
            | "CreateReviewableContentKey"
            | "BindReviewableContentEventProcessor"
            | "PromptVIPServerPurchase"
            | "ShowRewardedVideoAdAsync"
            | "IsDisabled"
            | "IsLegal"
            | "GetPassOwnershipAsync"
            | "CanUsersTransferRobuxAsync"
            | "CreateAdRewardFromDevProductId"
            | "AwardBadge"
            | "AwardBadgeAsync"
            | "GetAwardablePoints"
            | "GetGamePointBalance"
            | "GetPointBalance"
            | "AwardPoints"
            | "GetServerAttribute"
            | "SetServerAttribute"
            | "InitializeServerAttributesForStudio"
            | "UserEligibleForRealWorldCommerceAsync"
            | "PromptRealWorldCommerceBrowser"
            | "ShowVideoAd"
            | "GetAdAvailabilityNowAsync"
            | "BindViolationProcessor"
            | "MockViolationCallbackAsync"
    ) {
        return None;
    }
    Some((|| {
        let values = match name {
            "HideSelfView" => {
                world(lua)?.fire(this, "SelfViewHidden", vec![]);
                vec![]
            }
            "ShowSelfView" => {
                let position = self_view_position(&arg(args, 1))?;
                let value = crate::ltypes::enum_value(lua, position)?;
                world(lua)?.fire(this, "SelfViewVisible", vec![value]);
                vec![]
            }
            "CreateReviewableContentKey" => vec![reviewable_key(lua, arg(args, 1))?],
            "BindReviewableContentEventProcessor" => {
                let priority =
                    num(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))? as i32;
                let Value::Function(callback) = arg(args, 2) else {
                    return Err(rt_err("Argument 2 missing or nil"));
                };
                if world(lua)?.side != Side::Server {
                    return Err(rt_err(
                        "BindReviewableContentEventProcessor() may only be called on the server",
                    ));
                }
                vec![bind_reviewable(lua, priority, callback)?]
            }
            "ShowRewardedVideoAdAsync" => {
                let p =
                    opt_inst(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
                if !crate::doc::is_a(&inst_meta(lua, p)?.0, "Player") {
                    return Err(rt_err("Unable to cast Instance to Player"));
                }
                if opt_ud::<AdReward>(&arg(args, 2)).is_none() {
                    return Err(rt_err("Argument 2 missing or nil"));
                }
                if world(lua)?.side == Side::Client {
                    return Err(rt_err(
                        "AdService:ShowRewardedVideoAdAsync can only be called on the server",
                    ));
                }
                vec![crate::ltypes::enum_value(
                    lua,
                    LEnumItem::from_name("ShowAdResult", "InternalError")
                        .ok_or_else(|| rt_err("ShowAdResult unavailable"))?,
                )?]
            }
            "PromptVIPServerPurchase" => {
                opt_inst(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
                return Err(rt_err(
                    "MarketplaceService:PromptVIPServerPurchase() API is not yet released.",
                ));
            }
            "GetPassOwnershipAsync" => {
                return Err(rt_err(
                    "MarketplaceService:GetPassOwnershipAsync() is not enabled",
                ));
            }
            "CanUsersTransferRobuxAsync" => {
                return Err(rt_err("CanUsersTransferRobuxAsync is not enabled"));
            }
            "IsDisabled" | "IsLegal" => {
                num(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
                if name == "IsDisabled" {
                    world(lua)?.output.push((Level::Warn,"Sorry, badges can only be tested if they are disabled on Roblox game servers".into()));
                }
                vec![Value::Boolean(true)]
            }
            "CreateAdRewardFromDevProductId" => {
                let id =
                    num(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))? as i64;
                vec![Value::UserData(lua.create_userdata(AdReward(id))?)]
            }
            "ShowVideoAd" => {
                world(lua)?
                    .output
                    .push((Level::Warn, "Sorry, AdService is currently off.".into()));
                world(lua)?.fire(this, "VideoAdClosed", vec![Value::Boolean(false)]);
                vec![]
            }
            "GetAdAvailabilityNowAsync" => {
                if arg(args, 1).is_nil() {
                    return Err(rt_err("Argument 1 missing or nil"));
                }
                if world(lua)?.side == Side::Server {
                    return Err(rt_err("Local player is not available"));
                }
                world(lua)?.output.push((
                    Level::Warn,
                    "AdService:GetAdAvailabilityNowAsync: ad system not initialized yet".into(),
                ));
                let t = lua.create_table()?;
                t.set(
                    "AdAvailabilityResult",
                    crate::ltypes::enum_value(
                        lua,
                        LEnumItem::from_name("AdAvailabilityResult", "InternalError")
                            .ok_or_else(|| rt_err("AdAvailabilityResult unavailable"))?,
                    )?,
                )?;
                vec![Value::Table(t)]
            }
            "BindViolationProcessor" => {
                num(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
                if !matches!(arg(args, 2), Value::Function(_)) {
                    return Err(rt_err("Argument 2 missing or nil"));
                }
                return Err(rt_err("Safety callback API is not enabled"));
            }
            "MockViolationCallbackAsync" => {
                return Err(rt_err("Safety callback API is not enabled"));
            }
            "AwardBadge" | "AwardBadgeAsync" => {
                num(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
                num(&arg(args, 2)).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
                vec![Value::Boolean(false)]
            }
            "GetAwardablePoints" => vec![Value::Integer(i32::MAX as i64)],
            "GetGamePointBalance" | "GetPointBalance" => {
                num(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
                vec![Value::Integer(0)]
            }
            "AwardPoints" => {
                let user = num(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
                let amount =
                    num(&arg(args, 2)).ok_or_else(|| rt_err("Argument 2 missing or nil"))? as i32;
                vec![
                    Value::Number(user),
                    Value::Integer(0),
                    Value::Integer(amount as i64),
                ]
            }
            "UserEligibleForRealWorldCommerceAsync" => {
                return Err(rt_err(
                    "UserEligibleForRealWorldCommerceAsync is not enabled",
                ));
            }
            "PromptRealWorldCommerceBrowser" => {
                return Err(rt_err(
                    "PromptRealWorldCommerceBrowser endpoint is not enabled",
                ));
            }
            "InitializeServerAttributesForStudio" => {
                let Value::Table(t) = arg(args, 1) else {
                    return Err(rt_err("Argument 1 missing or nil"));
                };
                state(lua)?.raw_set("matchmaking", copy(lua, Value::Table(t), &mut Vec::new())?)?;
                vec![Value::Boolean(true), Value::Nil]
            }
            "GetServerAttribute" | "SetServerAttribute" => {
                let key = arg_str(args, 1, "string")?;
                if name == "SetServerAttribute" && arg(args, 2).is_nil() {
                    return Err(rt_err("Argument 2 missing or nil"));
                }
                let table = state(lua)?.raw_get::<Option<Table>>("matchmaking")?;
                let value = if let Some(t) = &table {
                    t.raw_get::<Value>(key.as_str())?
                } else {
                    Value::Nil
                };
                if value.is_nil() {
                    vec![
                        if name == "GetServerAttribute" {
                            Value::Nil
                        } else {
                            Value::Boolean(false)
                        },
                        Value::String(
                            lua.create_string(format!("Server attribute '{key}' not found"))?,
                        ),
                    ]
                } else if name == "SetServerAttribute" {
                    table
                        .unwrap()
                        .raw_set(key, copy(lua, arg(args, 2), &mut Vec::new())?)?;
                    vec![Value::Boolean(true), Value::Nil]
                } else {
                    vec![copy(lua, value, &mut Vec::new())?, Value::Nil]
                }
            }
            _ => unreachable!(),
        };
        Ok(MultiValue::from_vec(values))
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_key_matches_native_compact_encoding() {
        assert_eq!(key_encoding("D2EA7CAE-C8F6-4390-B013-D894BE3ED7A4", "image/png", 731751),
            "eyJpZCI6IkQyRUE3Q0FFLUM4RjYtNDM5MC1CMDEzLUQ4OTRCRTNFRDdBNCIsImNvbnRlbnRUeXBlIjoiaW1hZ2UvcG5nIiwiY29udGVudExlbmd0aCI6NzMxNzUxfQ==");
    }
    #[test]
    fn reviewable_processors_disconnect_without_losing_other_priorities() -> mlua::Result<()> {
        let lua = Lua::new();
        let f = lua.load("return function()end").eval::<mlua::Function>()?;
        let first = bind_reviewable(&lua, 1, f.clone())?;
        let second = bind_reviewable(&lua, 1, f)?;
        lua.globals().set("first", first)?;
        lua.globals().set("second", second)?;
        lua.load("assert(first.Connected and second.Connected);first:Disconnect();assert(not first.Connected and second.Connected)").exec()?;
        let entries = state(&lua)?.get::<Table>("review.handlers")?;
        assert_eq!(entries.pairs::<Value, Value>().count(), 1);
        lua.load("second:Disconnect();second:Disconnect()").exec()?;
        assert_eq!(entries.pairs::<Value, Value>().count(), 0);
        Ok(())
    }
}

#[cfg(test)]
mod self_view_tests {
    use super::*;
    #[test]
    fn self_view_signals_in_the_runtime() {
        use rbx_dom_weak::InstanceBuilder;
        let mut d = crate::doc::Document::new_place();
        let parent = d.find_child(d.dom.root_ref(), "ServerScriptService").unwrap();
        let script = d.dom.insert(parent, InstanceBuilder::new("Script").with_property("Source", r#"
local service=game:GetService('SocialService')
local points=game:GetService('PointsService');local awarded=0
points.PointsAwarded:Connect(function()awarded+=1 end)
local user,game_points,all_points=points:AwardPoints(1,7)
assert(user==1 and game_points==0 and all_points==7)
-- SelfViewVisible / SelfViewHidden are RobloxScriptSecurity (API dump):
-- a game script can't listen (src/security.rs).
for _,ev in {"SelfViewVisible","SelfViewHidden"} do
    local ok,e=pcall(function() return service[ev]:Connect(function() end) end)
    assert(not ok and e=="The current thread cannot connect '"..ev.."' (lacking capability RobloxScript)", tostring(e))
end
assert(select('#',service:ShowSelfView('TopLeft'))==0)
assert(select('#',service:HideSelfView())==0)
task.delay(1/60,function()
    assert(awarded==0)
    print('WS_SELF_VIEW_PASS')
end)
"#.to_owned()));
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(script);
        for _ in 0..3 { rt.step(1.0/60.0); }
        let output=rt.leave().output;
        assert!(output.iter().any(|(_,s)|s.contains("WS_SELF_VIEW_PASS")), "{output:?}");
    }
    #[test]
    fn native_self_view_casts() -> mlua::Result<()> {
        let lua = Lua::new();
        assert_eq!(self_view_position(&Value::Nil)?.name, "LastPosition");
        for n in 0..5 { assert_eq!(self_view_position(&Value::Integer(n))?.value, n as u32); }
        assert_eq!(self_view_position(&Value::String(lua.create_string("TopLeft")?))?.name, "TopLeft");
        for (value, expected) in [
            (Value::Integer(7), "Unable to cast double to SelfViewPosition"),
            (Value::Boolean(true), "Unable to cast bool to SelfViewPosition"),
            (Value::String(lua.create_string("bogus")?), "Unable to cast string to SelfViewPosition"),
            (crate::ltypes::enum_value(&lua, LEnumItem::from_name("InfoType", "Asset").unwrap())?, "Unable to cast InfoType to SelfViewPosition"),
        ] { assert!(self_view_position(&value).unwrap_err().to_string().contains(expected)); }
        Ok(())
    }
}
