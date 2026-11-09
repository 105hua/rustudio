//! Login-session metadata and Studio-mode prompts, local state and failures.
//! Development POST tests use localhost; native purchase verification cancels UI.
#![allow(unused_imports, unused_variables)]
use crate::runtime::*;
use mlua::{Lua, MultiValue, Table, Value};
use rbx_dom_weak::types::Ref;
#[path = "web_social_bulk.rs"]
mod bulk;
#[path = "web_social_shop.rs"]
mod shop;
#[path = "web_social_details.rs"]
mod details;
#[path = "web_social_feedback.rs"]
mod feedback;
#[path = "web_social_rsvp.rs"]
mod rsvp;
#[path = "web_social_recommendation_logs.rs"]
mod recommendation_logs;
#[path = "web_social_moderation.rs"]
mod moderation;
#[path = "web_social_group.rs"]
mod group;
#[path = "web_social_user.rs"]
mod user;
#[path = "web_social_cancel_subscription.rs"]
mod cancel_subscription;
#[path = "web_social_ads.rs"]
mod ads;
#[path = "web_social_http.rs"]
mod http;
#[path = "web_social_purchase.rs"]
mod purchase;
pub use purchase::{engine_message, is_engine_msg};
#[path = "web_social_studio.rs"]
mod studio;
#[path = "web_social_prompts.rs"]
mod prompts;
#[path = "web_social_recommendations.rs"]
mod recommendations;
#[path = "web_social_social_prompts.rs"]
mod social_prompts;
#[path = "web_social_teleport.rs"]
mod teleport;
pub const LUA_METHODS: &[(&str, &str)] = http::LUA_METHODS;

pub const METHODS: &[(&str, &str)] = &[
    ("HideSelfView", "SocialService"),
    ("ShowSelfView", "SocialService"),
    ("BindReviewableContentEventProcessor", "ModerationService"),
    ("CreateReviewableContentKey", "ModerationService"),
    ("IsDisabled", "BadgeService"),
    ("IsLegal", "BadgeService"),
    ("PromptVIPServerPurchase", "MarketplaceService"),
    ("GetPassOwnershipAsync", "MarketplaceService"),
    ("CanUsersTransferRobuxAsync", "MarketplaceService"),
    ("ShowRewardedVideoAdAsync", "AdService"),
    ("CreateAdRewardFromDevProductId", "AdService"),
    ("AwardBadge", "BadgeService"),
    ("AwardBadgeAsync", "BadgeService"),
    ("ShowVideoAd", "AdService"),
    ("GetAdAvailabilityNowAsync", "AdService"),
    ("BindViolationProcessor", "ModerationService"),
    ("MockViolationCallbackAsync", "ModerationService"),
    ("GetAwardablePoints", "PointsService"),
    ("GetGamePointBalance", "PointsService"),
    ("GetPointBalance", "PointsService"),
    ("AwardPoints", "PointsService"),
    ("InitializeServerAttributesForStudio", "MatchmakingService"),
    ("GetServerAttribute", "MatchmakingService"),
    ("SetServerAttribute", "MatchmakingService"),
    ("UserEligibleForRealWorldCommerceAsync", "CommerceService"),
    ("PromptRealWorldCommerceBrowser", "CommerceService"),
    ("GetTranslations", "TextFilterTranslatedResult"),
    ("GetTranslationForLocale", "TextFilterTranslatedResult"),
    ("GetNonChatStringForBroadcastAsync", "TextFilterResult"),
    ("GetNonChatStringForUserAsync", "TextFilterResult"),
    ("GetChatForUserAsync", "TextFilterResult"),
    ("GetTeleportData", "TeleportOptions"),
    ("SetTeleportData", "TeleportOptions"),
    ("GetTeleportSetting", "TeleportService"),
    ("SetTeleportSetting", "TeleportService"),
    ("GetLocalPlayerTeleportData", "TeleportService"),
    ("GetArrivingTeleportGui", "TeleportService"),
    ("SetTeleportGui", "TeleportService"),
];
pub const EVENTS: &[(&str, &str)] = &[
    // CoreScript signals; these are protected in the native API dump.
    ("GameInvitePromptClosed", "SocialService"),
    // Desktop Studio has no native contacts picker; measured dormant signal.
    ("PhoneBookPromptClosed", "SocialService"),
    ("SelfViewVisible", "SocialService"),
    ("SelfViewHidden", "SocialService"),
    // The deprecated point pipeline is disabled in Studio: connectable,
    // but AwardPoints returns its Studio tuple without emitting (measured).
    ("PointsAwarded", "PointsService"),
    ("OptInPromptClosed", "ExperienceNotificationService"),
    ("PromptOptInRequested", "ExperienceNotificationService"),
    ("UserSubscriptionStatusChanged", "MarketplaceService"),
    ("VideoAdClosed", "AdService"),
    ("PromptPurchaseFinished", "MarketplaceService"),
    ("PromptGamePassPurchaseFinished", "MarketplaceService"),
    ("PromptProductPurchaseFinished", "MarketplaceService"),
    ("PromptBundlePurchaseFinished", "MarketplaceService"),
    ("PromptSubscriptionPurchaseFinished", "MarketplaceService"),
    // Current online CoreScripts route Premium through the Roblox Plus flow.
    ("PromptPremiumPurchaseFinished", "MarketplaceService"),
    ("PromptRobloxSubscriptionPurchaseFinished", "MarketplaceService"),
    ("TeleportInitFailed", "TeleportService"),
];

// Roblox variants copy containers across the engine boundary, retaining instance
// references. Shared child tables become separate containers; cycles reject.
fn copy(lua: &Lua, value: Value, seen: &mut Vec<(Table, Table)>) -> mlua::Result<Value> {
    let Value::Table(source) = value else {
        return Ok(value);
    };
    if seen.iter().any(|(t, _)| t == &source) {
        return Err(rt_err("tables cannot be cyclic"));
    }
    let target = lua.create_table()?;
    seen.push((source.clone(), target.clone()));
    for pair in source.pairs::<Value, Value>() {
        let (key, value) = pair?;
        target.raw_set(key, copy(lua, value, seen)?)?;
    }
    seen.pop();
    Ok(Value::Table(target))
}
fn state(lua: &Lua) -> mlua::Result<Table> {
    if let Ok(t) = lua.named_registry_value::<Table>("web_social.teleport") {
        return Ok(t);
    }
    let t = lua.create_table()?;
    lua.set_named_registry_value("web_social.teleport", t.clone())?;
    Ok(t)
}
pub fn call(
    lua: &Lua,
    name: &str,
    this: Ref,
    args: &MultiValue,
) -> Option<mlua::Result<MultiValue>> {
    if let Some(result) = studio::call(lua, name, this, args) {
        return Some(result);
    }
    if !METHODS.iter().any(|(n, _)| *n == name) {
        return None;
    }
    Some((|| {
        let key = match name {
            "GetTeleportData" | "SetTeleportData" => format!("data:{this:?}"),
            "GetTeleportSetting" | "SetTeleportSetting" => {
                format!("setting:{}", arg_str(args, 1, "string")?)
            }
            _ => String::new(),
        };
        let value = match name {
            "GetTranslations" => Value::Table(lua.create_table()?),
            "GetTranslationForLocale" => {
                arg_str(args, 1, "string")?;
                Value::Nil
            }
            "GetNonChatStringForBroadcastAsync"
            | "GetNonChatStringForUserAsync"
            | "GetChatForUserAsync" => state(lua)?.raw_get(format!("filter:{this:?}:text"))?,
            "GetTeleportData" | "GetTeleportSetting" => {
                copy(lua, state(lua)?.raw_get::<Value>(key)?, &mut Vec::new())?
            }
            "SetTeleportData" | "SetTeleportSetting" => {
                let index = if name == "SetTeleportData" { 1 } else { 2 };
                let value = arg(args, index);
                if value.is_nil() {
                    return Err(rt_err(format!("Argument {index} missing or nil")));
                }
                state(lua)?.raw_set(key, copy(lua, value, &mut Vec::new())?)?;
                return Ok(MultiValue::new());
            }
            "SetTeleportGui" => {
                let value = arg(args, 1);
                if !value.is_nil() && opt_inst(&value).is_none() {
                    return Err(rt_err("Unable to cast value to Object"));
                }
                state(lua)?.raw_set("gui", arg(args, 1))?;
                return Ok(MultiValue::new());
            }
            _ => Value::Nil,
        };
        Ok(MultiValue::from_vec(vec![value]))
    })())
}
pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if class == "TeleportAsyncResult" && matches!(key, "PrivateServerId" | "ReservedServerAccessCode") {
        // Native constructor initializes the result's string storage
        // empty. Engine-produced results replace these with returned server IDs.
        let value=world(lua)?.d().dom.get_by_ref(r)
            .and_then(|instance|instance.properties.get(&key.into()).cloned())
            .unwrap_or_else(||Variant::String(String::new()));
        return Ok(Some(variant_to_lua(lua,&value,None)?));
    }
    if class == "AdPortal" && key == "PortalInvalidReason" {
        return Err(rt_err(format!(
            "PortalInvalidReason is not a valid member of AdPortal \"{}\"",
            world(lua)?.full_name(r)
        )));
    }
    if matches!(class, "AdGui" | "AdPortal") && key == "Status" {
        return Ok(Some(crate::ltypes::enum_value(
            lua,
            LEnumItem::from_name("AdUnitStatus", "Inactive")
                .ok_or_else(|| rt_err("AdUnitStatus unavailable"))?,
        )?));
    }
    if class == "AdPlacement" {
        if matches!(key, "RewardId" | "RewardName") {
            return Ok(Some(Value::String(lua.create_string("")?)));
        }
        if key == "RewardImageContent" {
            return Ok(Some(variant_to_lua(
                lua,
                &Variant::Content(Content::none()),
                None,
            )?));
        }
    }
    if class == "TextFilterTranslatedResult" {
        if key == "SourceLanguage" {
            return Ok(Some(Value::String(lua.create_string("en-us")?)));
        }
        if key == "SourceText" {
            return Ok(Some(
                state(lua)?.raw_get(format!("filter:{r:?}:SourceText"))?,
            ));
        }
    }
    Ok(None)
}
pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if class == "MarketplaceService" && key == "ProcessReceipt" && world(lua)?.side == Side::Client
    {
        return Err(rt_err(
            "Can only register ProcessReceipt callback on server",
        ));
    }
    Ok(false)
}
pub fn write_acts(class: &str, key: &str) -> bool {
    false
}
pub fn install(lua: &Lua) -> mlua::Result<()> {
    user::install(lua)?;
    http::install(lua)?;
    user::install_service(lua)?;
    group::install(lua)?;
    moderation::install(lua)?;
    purchase::install(lua)?;
    ads::install(lua)?;
    prompts::install(lua)?;
    social_prompts::install(lua)?;
    rsvp::install(lua)?;
    feedback::install(lua)?;
    details::install(lua)?;
    shop::install(lua)?;
    recommendations::install(lua)?;
    recommendation_logs::install(lua)?;
    cancel_subscription::install(lua)?;
    teleport::install(lua)
}
pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    shop::step(lua)?;
    studio::observe_captures(lua)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn teleport_result_reads_engine_storage_and_empty_native_defaults() -> mlua::Result<()> {
        let mut d=crate::doc::Document::new_place();
        let root=d.dom.root_ref();
        let result=d.dom.insert(root,rbx_dom_weak::InstanceBuilder::new("TeleportAsyncResult"));
        let rt=Runtime::new().map_err(rt_err)?;rt.enter(World::new(d,RunMode::Run));rt.bind_globals()?;
        for key in ["PrivateServerId","ReservedServerAccessCode"] {
            let value=prop_get(&rt.lua,result,"TeleportAsyncResult",key)?.unwrap();
            assert!(matches!(value,Value::String(ref value)if value.to_str()?.is_empty()));
            store_prop(&rt.lua,result,key,Variant::String(format!("mock-{key}")))?;
            let value=prop_get(&rt.lua,result,"TeleportAsyncResult",key)?.unwrap();
            assert!(matches!(value,Value::String(ref value)if value.to_str()?.as_ref()==format!("mock-{key}")));
        }
        rt.lua.globals().set("result",inst_value(&rt.lua,result)?)?;
        rt.lua.load("local ok,e=pcall(function()result.PrivateServerId='write'end);assert(not ok and string.find(tostring(e),'read only',1,true),tostring(e))").exec()?;
        rt.leave();Ok(())
    }
    #[test]
    fn studio_points_and_matchmaking_state() -> mlua::Result<()> {
        let lua = Lua::new();
        let r = Ref::new();
        let invoke = |name: &str, values: Vec<Value>| {
            call(&lua, name, r, &MultiValue::from_vec(values)).unwrap()
        };
        let result = invoke(
            "AwardPoints",
            vec![Value::Nil, Value::Integer(1), Value::Integer(4)],
        )?;
        assert_eq!(result.len(), 3);
        assert!(matches!(result.get(1), Some(Value::Integer(0))));
        assert!(matches!(result.get(2), Some(Value::Integer(4))));
        let key = Value::String(lua.create_string("x")?);
        let result = invoke(
            "SetServerAttribute",
            vec![Value::Nil, key.clone(), Value::Integer(7)],
        )?;
        assert!(matches!(result.front(), Some(Value::Boolean(false))));
        let attrs = lua.create_table()?;
        attrs.set("x", 7)?;
        invoke(
            "InitializeServerAttributesForStudio",
            vec![Value::Nil, Value::Table(attrs.clone())],
        )?;
        attrs.set("x", 8)?;
        let result = invoke("GetServerAttribute", vec![Value::Nil, key.clone()])?;
        assert!(matches!(result.front(), Some(Value::Integer(7))));
        invoke(
            "SetServerAttribute",
            vec![Value::Nil, key.clone(), Value::Integer(9)],
        )?;
        let result = invoke("GetServerAttribute", vec![Value::Nil, key])?;
        assert!(matches!(result.front(), Some(Value::Integer(9))));
        Ok(())
    }
    #[test]
    fn notification_permission_uses_backend_only_for_published_universes() -> mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("__rs_lua_methods",lua.create_table()?)?;http::install(&lua)?;
        lua.load(r#"
local client=true;local calls=0;local response={canPrompt=true};local failure
task={wait=function()end}
game={GameId=0,GetService=function(_,name)
 if name=='RunService' then return {IsClient=function()return client end} end
 return {JSONDecode=function()return response end}
end}
__ws_begin=function(name,args)assert(name=='CanPromptOptInAsync' and args[1]==7);calls+=1;return calls end
__ws_poll=function()return true,'fixture',failure end
local can=__rs_lua_methods.CanPromptOptInAsync
assert(can({})==true and calls==0)
game.GameId=7;assert(can({})==true and calls==1)
response={canPrompt=false};assert(can({})==false and calls==2)
failure='HTTP 429 (Too Many Requests)';assert(can({})==false and calls==3)
failure='HTTP 403 (Forbidden)';local ok,e=pcall(can,{});assert(not ok and e=='ExperienceNotificationService request failed: HTTP 403 (Forbidden)')
failure=nil;response={};local ok,e=pcall(can,{});assert(not ok and e=='ExperienceNotificationService request failed: Invalid response received from Notifications API')
response={canPrompt='bad'};local ok,e=pcall(can,{});assert(not ok and e=='Variant cast failed')
client=false;local before=calls;local ok,e=pcall(can,{});assert(not ok and calls==before and e=='ExperienceNotificationService:CanPromptOptInAsync can only be executed by a local script')
"#).exec()
    }
    #[test]
    fn party_locations_merge_local_players_sort_and_throttle() -> mlua::Result<()> {
        let lua=Lua::new();
        lua.globals().set("__rs_lua_methods",lua.create_table()?)?;
        http::install(&lua)?;
        lua.load(r#"
local id='00000000-0000-0000-0000-000000000000'
local local_players={{UserId=1,PartyId=id},{UserId=2,PartyId='different'}}
local server=true;local response;local failure;local calls=0;local clock=0
os.clock=function()return clock end
game={GameId=7,PlaceId=8,JobId='local',PrivateServerId='private',GetService=function(_,name)
 if name=='RunService' then return {IsServer=function()return server end} end
 if name=='Players' then return {GetPlayers=function()return local_players end} end
 return {JSONDecode=function()return response end}
end}
__ws_begin=function(name,a)assert(name=='GetPartyAsync' and a[1]==id and a[2]==7 and a[3]==8 and a[4]=='local');calls+=1;return calls end
__ws_poll=function()return true,'fixture',failure end
response={reservedServerAccessCode="current-reserved",universeMembers={
 {userId=1,placeId=100,instanceId='stale',latestGroupUpJoinTimeMs=900},
 {userId=2,placeId=100,instanceId='left',latestGroupUpJoinTimeMs=1000},
 {userId=3,placeId=99,instanceId='newer',latestGroupUpJoinTimeMs=20,privateServerId='p',reservedServerAccessCode='r'},
 {userId=4,placeId=98,instanceId='older',latestGroupUpJoinTimeMs=10},
 {userId=5,placeId=0,instanceId='invalid'}, {userId=6,placeId=99,instanceId=''},
}}
local get=__rs_lua_methods.GetPartyAsync
local result=get({},id)
assert(#result==3 and result[1].UserId==3 and result[2].UserId==4 and result[3].UserId==1)
assert(result[1].JobId=='newer' and result[1].PlaceId==99 and result[1].PrivateServerId=='p' and result[1].ReservedServerAccessCode=='r' and result[1].latestGroupUpJoinTimeMs==nil)
assert(result[2].PrivateServerId==nil and result[3].JobId=='local' and result[3].PrivateServerId=='private' and result[3].ReservedServerAccessCode=='current-reserved')
result[1].JobId='changed';assert(get({},id)[1].JobId=='newer')
local ok,e=pcall(get,{},'bad');assert(not ok and e=='SocialService:GetPartyAsync Received invalid party id.')
server=false;local ok,e=pcall(get,{},id);assert(not ok and e=='SocialService:GetPartyAsync Must be called from a server script.');server=true
local_players={};response={};assert(#get({},id)==0)
failure='HTTP 400 (Bad Request)';local ok,e=pcall(get,{},id);assert(not ok and e=='SocialService:GetPartyAsync Request failed: %s')
failure='Unable to convert response to JSON';local ok,e=pcall(get,{},id);assert(not ok and e=='SocialService:GetPartyAsync An unknown error occurred.')
failure=nil;response={};for i=calls+1,120 do get({},id) end
local before=calls;local ok,e=pcall(get,{},id);assert(not ok and calls==before and e=='SocialService:GetPartyAsync Request was throttled. This API is limited to 120 requests per minute.')
clock=61;assert(#get({},id)==0 and calls==121)
"#).exec()
    }
    #[test]
    fn social_events_use_native_fields_dates_and_filtering() -> mlua::Result<()> {
        let lua = Lua::new();
        crate::ltypes::install(&lua, &lua.globals())?;
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        http::install(&lua)?;
        lua.load(r#"
Enum={ExperienceEventStatus={Active='active',Cancelled='cancelled',Moderated='moderated',Unpublished='unpublished',Unknown='unknown'},RsvpStatus={None='none',Going='going',NotGoing='notGoing'}}
local function event(id,status,start,finish) return {id=id,title='Title',description='Description',eventStatus=status,eventTime={startUtc=start,endUtc=finish},userRsvpStatus='going'} end
local future='2099-01-01T00:00:00Z';local past='2000-01-01T00:00:00Z'
local response={data={event('1','active',past,future),event('2','active',past,past),event('3','cancelled',past,future),{id='bad'}}}
game={GameId=10000000007,GetService=function(_,name)
 if name=='RunService' then return {IsClient=function()return true end} end
 return {JSONDecode=function()return response end}
end}
__ws_begin=function(name,args)assert(name=='GetUpcomingExperienceEventsAsync' and args[1]==10000000007);return 1 end
__ws_poll=function()return true,'fixture',nil end
local out=__rs_lua_methods.GetUpcomingExperienceEventsAsync({})
assert(#out==1 and out[1].Id=='1' and out[1].Title=='Title' and out[1].Status=='active')
assert(out[1].StartTime.Year==2000 and out[1].EndTime.Year==2099)
assert(out[1].HasStarted and not out[1].HasEnded and out[1].UserRsvpStatus=='going')
"#).exec()
    }
    #[test]
    fn event_read_ids_edit_context_missing_and_foreign_records()->mlua::Result<()> {
        let lua=Lua::new();crate::ltypes::install(&lua,&lua.globals())?;
        lua.globals().set("__rs_lua_methods",lua.create_table()?)?;http::install(&lua)?;
        lua.load(r#"
Enum={ExperienceEventStatus={Active='active',Unknown='unknown'},RsvpStatus={None='none',Going='going',NotGoing='notGoing'}}
local client=false;local edit=true;local response;local failure;local called=0
local service={IsClient=function()return client end,IsEdit=function()return edit end}
game={GameId=7,GetService=function(_,name)if name=='RunService'then return service end;return{JSONDecode=function()return response end}end}
__ws_begin=function(name,args)called+=1;if name~='GetUpcomingExperienceEventsAsync'then assert(type(args[1])=='string')end;return 1 end
__ws_poll=function()return true,'fixture',failure end
local m=__rs_lua_methods
failure='HTTP 404 (Not Found)';assert(m.GetExperienceEventAsync({},1)==nil)
local ok,e=pcall(m.GetEventRsvpStatusAsync,{},'0001');assert(not ok and e=='Error getting event details: HTTP 404 (Not Found)',tostring(e))
failure='HTTP 503 (Service Unavailable)';ok,e=pcall(m.GetExperienceEventAsync,{},1);assert(not ok and e=='HttpError 503')
failure='Unable to convert response to JSON';assert(m.GetExperienceEventAsync({},1)==nil)
ok,e=pcall(m.GetEventRsvpStatusAsync,{},1);assert(not ok and e=='Error parsing event details for eventId 1: Invalid response format.')
failure=nil;response={universeId=7,id='1',title='Title',description='Body',eventStatus='active',eventTime={startUtc='2099-01-01T00:00:00Z',endUtc='2099-01-02T00:00:00Z'},userRsvpStatus='going'}
assert(m.GetExperienceEventAsync({},1).Id=='1');assert(m.GetEventRsvpStatusAsync({},1)=='going')
response.universeId=8;assert(m.GetExperienceEventAsync({},1)==nil)
ok,e=pcall(m.GetEventRsvpStatusAsync,{},1);assert(not ok and e=='SocialService:GetEventRsvpStatusAsync Event is not in the current universe.')
response={};assert(m.GetExperienceEventAsync({},1)==nil)
ok,e=pcall(m.GetEventRsvpStatusAsync,{},1);assert(not ok and e=='Error parsing event details for eventId 1: Invalid response format.')
local before=called
for _,id in {'+1',' 1','1x','0','9223372036854775808','99999999999999999999999'}do ok,e=pcall(m.GetExperienceEventAsync,{},id);assert(not ok and e=='SocialService:GetExperienceEventAsync Received invalid event id.')end
assert(called==before)
edit=false;ok,e=pcall(m.GetExperienceEventAsync,{},1);assert(not ok and e=='SocialService:GetExperienceEventAsync must be called on the client.')
"#).exec()
    }
    #[test]
    fn yielding_wrappers_compile() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        http::install(&lua)
    }
    #[test]
    fn read_result_conversions_match_studio() -> mlua::Result<()> {
        let lua = Lua::new();
        crate::ltypes::install(&lua, &lua.globals())?;
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        http::install(&lua)?;
        lua.load(r#"
local response;local requested
Enum={SubscriptionState={NeverSubscribed='NeverSubscribed'},SubscriptionPaymentStatus={Paid='Paid',Refunded='Refunded'},SubscriptionPeriod={Month='Month'}}
local warnings={};warn=function(x)table.insert(warnings,x)end
local run={IsClient=function()return false end,IsServer=function()return true end}
game={GameId=10000000007,GetService=function(_,name)if name=='RunService' then return run end;return {JSONDecode=function()return response end}end}
__ws_begin=function(name,args)requested={name,args};return 1 end
__ws_poll=function()return true,'fixture',nil end
local m=__rs_lua_methods
local ok,e=pcall(m.GetBadgeInfoAsync,{},0);assert(not ok and e=='Invalid badgeId')
local ok,e=pcall(m.GetGroupInfoAsync,{},0);assert(not ok and e=='GroupService:GetGroupInfoAsync() argument is not a valid groupId')
assert(m.UserOwnsGamePassAsync({},1,-1)==false and m.UserOwnsGamePassAsync({},-1,1)==false)
assert(m.UserHasBadgeAsync({},1,-1)==false and m.UserHasBadgeAsync({},-1,1)==false)
response={name='Badge',description='Description',enabled=true,iconImageId=7,statistics={awardedCount=123,pastDayAwardedCount=5,winRatePercentage=.25}}
local b=m.GetBadgeInfoAsync({},1);assert(b.Name=='Badge' and b.AwardCount==123 and b.PastDayAwardCount==5 and b.Rarity==.25)
response={data={{badgeId=7,awardedDate='2023-01-01T00:00:00Z'},{badgeId=8}}}
local b=m.GetUserBadgesAsync({},1,{7,8});assert(#b==1 and b[1].BadgeId==7 and b[1].AwardedDate.UnixTimestamp==1672531200)
response={data={{roles={{id=1,name='Member',rank=1}}}}}
local g=m.GetRolesInGroupAsync({},1,7);assert(g.IsMember and g.Roles[1].Id==1)
response={data={}};assert(not m.GetRolesInGroupAsync({},1,7).IsMember)
response={details={subscriptionState='NeverSubscribed'}}
local d=m.GetUserSubscriptionDetailsAsync({},{UserId=1},'EXP-1');assert(d.SubscriptionState=='NeverSubscribed' and d.NextRenewTime==nil)
response={isSubscribed=false,isRenewing=false};local d=m.GetUserSubscriptionStatusAsync({},{UserId=1},'EXP-1');assert(d.IsSubscribed==false and d.IsRenewing==false)
response={subscriptionProductInfo={name='Subscription',subscriptionPeriod='Month',isForSale=true,priceInRobux=99,priceDiscountDetails={{type='Sale',percent=10,endTimeMs=1700000000000}}}}
local d=m.GetSubscriptionProductInfoAsync({},'EXP-1');assert(d.Name=='Subscription' and d.SubscriptionPeriod=='Month' and d.DisplaySubscriptionPeriod=='/month' and d.PriceInRobux==99 and d.PriceDiscountDetails[1].EndTime.UnixTimestampMillis==1700000000000)
response={};local ok,e=pcall(m.GetSubscriptionProductInfoAsync,{},'EXP-1');assert(not ok and e=='Product info not found for subscription: EXP-1')
local h=m.GetUserSubscriptionPaymentHistoryAsync({},{UserId=1},'EXP-2');assert(#h==3 and h[1].PaymentStatus=='Refunded' and h[1].CycleStartTime.UnixTimestampMillis==1677801600000)
local h=m.GetUserSubscriptionPaymentHistoryAsync({},{UserId=1},'EXP-3');assert(#h==2 and h[1].CycleEndTime.UnixTimestampMillis-h[1].CycleStartTime.UnixTimestampMillis==2592000000)
local h=m.GetUserSubscriptionPaymentHistoryAsync({},{UserId=1},'EXP-4');assert(#h==0 and #warnings==1)
response={userPriceLevels={['1']=1000,['2']=900}};local p=m.GetUsersPriceLevelsAsync({},{2,1});assert(p[1].UserId==2 and p[1].PriceLevel==900 and p[2].UserId==1)
response={data={}};assert(#m.GetUserInfosByUserIdsAsync({},{-1})==0 and requested[2][1][1]==-1)
local ok,e=pcall(m.GetUserInfosByUserIdsAsync,{},{'bad'});assert(not ok and e=='UserService: invalid userId(s) given. Please make sure they are of number type')
"#).exec()
    }
    #[test]
    fn group_pages_fetch_only_when_advanced() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        lua.load(super::super::pages::MOCK).exec()?;
        http::install(&lua)?;
        lua.load(r#"
local requests={};local response;local fail=false
local m=__rs_lua_methods
game={GetService=function()return {JSONDecode=function()return response end}end}
__ws_begin=function(method,args)
 assert(method=='GetAlliesAsync');table.insert(requests,args[2]);
 response=args[2]==1 and {Groups={{Id=7}},FinalPage=false} or {Groups={{Id=8}},FinalPage=true};return 1
end
__ws_poll=function()if fail then return true,nil,'HTTP 503 (Service Unavailable)' end;return true,'fixture',nil end
local page=m.GetAlliesAsync({},7);assert(#requests==1 and requests[1]==1 and not page.IsFinished and page.ClassName=='StandardPages')
local rows=m.GetCurrentPage(page);rows[1].Id=99;assert(m.GetCurrentPage(page)[1].Id==7)
fail=true;local ok=pcall(m.AdvanceToNextPageAsync,page);assert(not ok and not page.IsFinished and m.GetCurrentPage(page)[1].Id==7)
fail=false;m.AdvanceToNextPageAsync(page);assert(requests[3]==2 and page.IsFinished and m.GetCurrentPage(page)[1].Id==8)
local n=#requests;local ok=pcall(m.AdvanceToNextPageAsync,page);assert(not ok and #requests==n)
"#).exec()
    }
    #[test]
    fn studio_variants_reject_cycles_and_duplicate_shared_children() -> mlua::Result<()> {
        let lua = Lua::new();
        let shared = lua.create_table()?;
        shared.set("x", 1)?;
        let input = lua.create_table()?;
        input.set("a", shared.clone())?;
        input.set("b", shared)?;
        let Value::Table(out) = copy(&lua, Value::Table(input), &mut Vec::new())? else {
            panic!()
        };
        assert_ne!(out.get::<Table>("a")?, out.get::<Table>("b")?);
        let cycle = lua.create_table()?;
        cycle.set("self", cycle.clone())?;
        assert!(
            error_message(&copy(&lua, Value::Table(cycle), &mut Vec::new()).unwrap_err())
                .contains("tables cannot be cyclic")
        );
        Ok(())
    }
    #[test]
    fn marketplace_ranking_fetches_metadata_caches_and_throttles() -> mlua::Result<()> {
        let lua = Lua::new();
        crate::ltypes::install(&lua, &lua.globals())?;
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        http::install(&lua)?;
        lua.load(r#"
local response;local body;local calls=0;local clock=0;local fail=false
local infoType=Enum.InfoType
os={clock=function()return clock end};task={wait=function()end}
game={GameId=7,GetService=function(_,name)if name=='RunService' then return {IsServer=function()return false end}end;return {JSONEncode=function(_,v)body=v;return v.productIdentifiers and 'rank' or 'top' end,JSONDecode=function()return response end}end}
__ws_begin=function(name,args)
 if name=='RankProductsAsync' or name=='RecommendTopProductsAsync' then
  calls+=1;assert(body.universeId==7)
  if name=='RankProductsAsync' then assert(body.productIdentifiers[1].id==1286521615 and body.productIdentifiers[1].type=='Product')else assert(body.productInfoTypes[1]=='Product' and (body.productInfoTypes[2]==nil or body.productInfoTypes[2]=='GamePass'))end
  response={rankedItems={{productIdentifier={id=1286521615,type='Product'}}}}
 else assert(name=='GetProductInfo' and args[1]==1286521615 and args[2]=='Product');response={developerProducts={{ProductId=1286521615,Name='Product',PriceInRobux=40}}} end
 return 1
end
__ws_poll=function()if fail then return true,nil,'HTTP 400 (Bad Request)' end;return true,'fixture',nil end
local m=__rs_lua_methods
local input={{Id=1286521615,InfoType=infoType.Product}}
local r=m.RankProductsAsync({},input);assert(calls==1 and r[1].ProductIdentifier.Id==1286521615 and r[1].ProductIdentifier.InfoType==infoType.Product and r[1].ProductInfo.Name=='Product')
r[1].ProductInfo.Name='mutated';local r=m.RankProductsAsync({},input);assert(calls==1 and r[1].ProductInfo.Name=='Product')
for i=3,10 do m.RankProductsAsync({},input)end
local ok,e=pcall(m.RankProductsAsync,{},input);assert(not ok and e=='MarketplaceService:RankProductsAsync() request was throttled. This API is limited to 10 requests per minute.')
clock=61;local r=m.RankProductsAsync({},input);assert(calls==2)
local r=m.RecommendTopProductsAsync({},{infoType.GamePass,infoType.Product});assert(calls==3 and r[1].ProductInfo.PriceInRobux==40)
local ok,e=pcall(m.RankProductsAsync,{},{{Id=1,InfoType=3}});assert(not ok and e=='MarketplaceService:RankProductsAsync(): ProductIdentifier at index 1 does not contain valid InfoType')
local ok,e=pcall(m.RecommendTopProductsAsync,{},{2});assert(not ok and e=='MarketplaceService:RecommendTopProductsAsync(): Invalid InfoType at index 0')
local ok,e=pcall(m.RecommendTopProductsAsync,{},{infoType.Asset});assert(not ok and e=='MarketplaceService:RecommendTopProductsAsync(): Found Unsupported InfoType at index 0. Only GamePass and Product are currently supported.')
clock=122;fail=true;local ok,e=pcall(m.RecommendTopProductsAsync,{},{infoType.Product});assert(not ok and e=='task: wait for TopRecommendedProducts - Failed: HTTP request failed with status code: 400')
"#).exec()
    }
    #[test]
    fn recommendation_pages_keep_request_context_cursor_and_tracing() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        lua.load(super::super::pages::MOCK).exec()?;
        http::install(&lua)?;
        lua.load(r#"
local requests={};local response;local body;local count=0
local http={JSONEncode=function(_,v)body=v;return 'request' end,JSONDecode=function()return response end}
game={GameId=7,GetService=function(_,name)if name=='RunService' then return {IsServer=function()return true end}end;return http end}
__ws_begin=function(name,args)
 assert(name=='GenerateItemListAsync' and args[1]=='request');count+=1
 assert(body.experienceId==7 and body.configName=='config' and body.locationId=='location' and body.paginationToken.count==3 and body.realtimeRenderContext[1].key=='UserId' and body.realtimeRenderContext[1].value=='1')
 if count==1 then assert(body.paginationToken.cursor==nil);response={items={{itemId='first'}},tracingId='trace1',responseToken={cursor='next'}}
 else assert(body.paginationToken.cursor=='next');response={items={{itemId='last'}},tracingId='trace2'}end
 return count
end
__ws_poll=function()return true,'fixture',nil end
local m=__rs_lua_methods
local p=m.GenerateItemListAsync({},{ConfigName='config',LocationId='location',PageSize=3,CustomContexts={UserId='1'}})
assert(p.ClassName=='RecommendationPages' and not p.IsFinished and count==1)
assert(m.GetCurrentPage(p)[1].ItemId=='first' and m.GetCurrentPage(p)[1].TracingId=='trace1')
m.AdvanceToNextPageAsync(p);assert(p.IsFinished and count==2 and m.GetCurrentPage(p)[1].ItemId=='last' and m.GetCurrentPage(p)[1].TracingId=='trace2')
local ok,e=pcall(m.GenerateItemListAsync,{},{ConfigName='config',LocationId='location'});assert(not ok and e=='generateItemListAsync: (Server script) CustomContexts must be provided and contain UserId key')
local ok,e=pcall(m.GenerateItemListAsync,{},{ConfigName='config',LocationId='location',Extra=1});assert(not ok and e=='Unexpected key in GenerateRecommendationItemListRequest: Extra. Allowed keys are: ConfigName, LocationId, PageSize, BoostCustomTags, CustomContexts')
"#).exec()
    }
    #[test]
    fn ad_campaign_lookup_validates_and_reuses_successful_response() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        http::install(&lua)?;
        lua.load(r#"
local calls=0;local response={isEligible=false,advertiserName='Advertiser',immersiveBrandedAdPlacementIds={}}
local player={};local players={LocalPlayer=player}
task={wait=function()end}
game={GetService=function(_,name)if name=='RunService' then return {IsServer=function()return false end}elseif name=='Players' then return players end;return {JSONDecode=function()return response end}end}
__ws_begin=function(name,args)assert(name=='GetCampaignEligibilityAsync' and string.match(args[1],'^campaign%d*$'));calls+=1;return 1 end
__ws_poll=function()return true,'fixture',nil end
local m=__rs_lua_methods
assert(m.GetCampaignEligibilityAsync({},'campaign').IsEligible==false)
assert(m.GetCampaignEligibilityAsync({},'campaign',player).IsEligible==false and calls==1)
local ok,e=pcall(m.GetCampaignEligibilityAsync,{},'campaign',{});assert(not ok and e=='GetCampaignEligibilityAsync may only be called for the local player')
local ok,e=pcall(m.GetCampaignEligibilityAsync,{},string.rep('x',37));assert(not ok and e=='Campaign ID should not be longer than 36 characters' and calls==1)
for i=2,50 do m.GetCampaignEligibilityAsync({},'campaign'..i)end
local ok,e=pcall(m.GetCampaignEligibilityAsync,{},'campaign51');assert(not ok and e=='Too many campaigns in this experience (50)' and calls==50)
"#).exec()
    }
    #[test]
    fn commerce_product_conversion_and_identifier_validation() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        http::install(&lua)?;
        lua.load(r#"
local response={isForSale=true,item={name='Item',description='Physical item',iconImageAssetId=7,displayPrice='$5',isPurchasable=false},benefits={{name='Benefit',description='Virtual item',id='1',benefitType='AvatarAsset'}}}
Enum={BenefitType={AvatarAsset='AvatarAsset'}}
game={GetService=function()return {JSONDecode=function()return response end}end}
__ws_begin=function(name,args)assert(name=='GetCommerceProductInfoAsync' and args[1]=='COM-1');return 1 end
__ws_poll=function()return true,'fixture',nil end
local m=__rs_lua_methods;local p=m.GetCommerceProductInfoAsync({},'COM-1')
assert(p.IsForSale and p.Item.Name=='Item' and not p.Item.IsPurchasable and p.Item.IconImageAssetId==7 and p.Benefits[1].BenefitType=='AvatarAsset' and p.Benefits[1].Id=='1')
local ok,e=pcall(m.GetCommerceProductInfoAsync,{},'bad');assert(not ok and e=='Invalid commerceProductId')
"#).exec()
    }
    #[test]
    fn roblox_subscription_resolves_product_key_then_details() -> mlua::Result<()> {
        let lua = Lua::new();
        crate::ltypes::install(&lua, &lua.globals())?;
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        http::install(&lua)?;
        lua.load(r#"
local response;local count=0;local subscribed=false
local player={UserId=1,IsA=function(_,class)return class=='Player'end}
typeof=function(v)return v==player and 'Instance' or type(v)end
Enum={SubscriptionState={NeverSubscribed=0,SubscribedWillRenew=1,Expired=4}}
game={GetService=function(_,name)if name=='RunService' then return {IsServer=function()return true end}end;return {JSONDecode=function()return response end}end}
__ws_begin=function(name,args)
 count+=1
 if name=='GetRobloxSubscriptionDetailsAsync' then assert(args[1]==1);response={productKeys={{id='key'}}}
 else assert(name=='__ws_roblox_details' and args[2]=='RBP-key');response={details={subscriptionState=subscribed and 'SubscribedWillRenew' or 'NeverSubscribed'}};if subscribed then response.subscriptionProductTypeStateDetails={robloxSubscriptionStateDetails={startTimestampMs=1700000000000}} end end
 return 1
end
__ws_poll=function()return true,'fixture',nil end
local m=__rs_lua_methods
local d=m.GetRobloxSubscriptionDetailsAsync({},player);assert(count==2 and not d.IsSubscribed and not d.IsOriginExperience and d.StartTime==nil)
subscribed=true;local d=m.GetRobloxSubscriptionDetailsAsync({},player);assert(count==4 and d.IsSubscribed and d.IsOriginExperience and d.StartTime.UnixTimestampMillis==1700000000000)
"#).exec()
    }
    #[test]
    fn product_batches_and_bundle_shapes_match_online_studio() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        http::install(&lua)?;
        lua.load(r#"
local response;game={GetService=function()return {JSONDecode=function()return response end}end}
Enum={AssetType={GetEnumItems=function()return {{Value=29,Name='LeftArm'}}end}}
__ws_begin=function()return 1 end;__ws_poll=function()return true,'fixture',nil end
local m=__rs_lua_methods
response={gamePasses={{id=1,productId=3613243303,name='Never get banned',displayName='Never get banned',displayDescription='Description',displayIconImageAssetId=835789180,isForSale=false,userBasePriceInRobux=0,creator={creatorId=111054863,deprecatedId=111054863,creatorType='User',name='Creator'}}}}
local g=m.GetProductInfoAsync({},1,{Name='GamePass'});assert(g.AssetId==0 and g.AssetTypeId==0 and g.TargetId==1 and g.ProductType=='Game Pass' and g.Sales==0 and g.Name=='Never get banned' and g.PriceInRobux==nil and g.Creator.CreatorType=='User')
response={developerProducts={{ProductId=1286521615,DeveloperProductId=14198404,Name='1 REVIVE',PriceInRobux=40,displayIcon=10277642616}}}
local g=m.GetProductInfoAsync({},1286521615,{Name='Product'});assert(g.ProductId==1286521615 and g.DeveloperProductId==14198404 and g.DisplayIcon==10277642616 and g.displayIcon==nil)
response={developerProducts={}};local ok,e=pcall(m.GetProductInfoAsync,{},14198404,{Name='Product'});assert(not ok and e=='MarketplaceService::getProductInfo - GetDeveloperProductInfo failed: Item not found in batch response.')
response={id=1,name='Bundle',description='Description',bundleType='BodyParts',items={{id=7,name='Arm',type='Asset',assetType=29},{id=8,name='Outfit',type='UserOutfit'}},product={isForSale=false,priceInRobux=0}}
local b=m.GetProductInfoAsync({},1,{Name='Bundle'});assert(b.Id==1 and b.Name=='Bundle' and b.Items[1].AssetType=='LeftArm' and b.Items[2].AssetType==nil and b.Product==nil)
local ok,e=pcall(m.GetProductInfoAsync,{},1,{Name='Subscription'});assert(not ok and e=='MarketplaceService:GetProductInfo() does not support InfoType INFO_SUBSCRIPTION')
local ok,e=pcall(m.GetProductInfoAsync,{},0);assert(not ok and e=='MarketplaceService:GetProductInfo() 0 is not a valid assetId')
"#).exec()
    }
    #[test]
    fn internal_read_results_use_native_tuple_and_recommendation_fields() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("__rs_lua_methods", lua.create_table()?)?;
        http::install(&lua)?;
        lua.load(r#"
local response;game={PlaceId=10000000000008,JobId='same-job',GetService=function(_,name)if name=='RunService' then return {IsServer=function()return true end}end;return {JSONDecode=function()return response end}end}
__ws_begin=function()return 1 end;__ws_poll=function()return true,'fixture',nil end
Enum={CreatorType={User='User'},RecommendationItemVisibility={Public='Public'},RecommendationImpressionType={View='View'},RecommendationActionType={AddReaction='AddReaction'}}
response={PlaceId=10000000000008,GameId='same-job'};local same,e,id,job=__rs_lua_methods.GetPlayerPlaceInstanceAsync({},1);assert(same and e=='' and id==10000000000008 and job=='same-job')
response={PlaceId=7,GameId='other-job'};assert(not __rs_lua_methods.GetPlayerPlaceInstanceAsync({},1))
response={};local ok,e=pcall(__rs_lua_methods.GetPlayerPlaceInstanceAsync,{},1);assert(not ok and e=='Unexpected result')
response={item={itemId='item',referenceId='reference',creator={creatorId='1',creatorType='User'},visibilityStatus='Public',customTags={'tag'},contentAttributes={assetId='7',text='Text',contentDescription='Description',seekStartTimeSeconds=2},stats={impressions={{impressionType='View',counts={All=10}}},actions={{actionType='AddReaction',reactionType='Like',counts={All=2}}}}}}
local r=__rs_lua_methods.GetRecommendationItemAsync({},'item')
assert(r.ItemId=='item' and r.ReferenceId=='reference' and r.Creator.CreatorId==1 and r.Creator.CreatorType=='User' and r.Visibility=='Public' and r.TracingId=='')
assert(r.Attributes.AssetId==7 and r.Attributes.Text=='Text' and r.Attributes.Description=='Description' and r.Attributes.SeekStartTime==2)
assert(r.Stats.Impressions[1].ImpressionType=='View' and r.Stats.Impressions[1].Counts.All==10 and r.Stats.Actions[1].ActionType=='AddReaction')
response={};assert(next(__rs_lua_methods.GetRecommendationItemAsync({},'missing'))==nil)
"#).exec()
    }
    #[test]
    fn teleport_variants_copy_on_set_and_get() -> mlua::Result<()> {
        let lua = Lua::new();
        let r = Ref::new();
        let data = lua.create_table()?;
        data.set("x", 7)?;
        let args = MultiValue::from_vec(vec![Value::Nil, Value::Table(data.clone())]);
        call(&lua, "SetTeleportData", r, &args).unwrap()?;
        data.set("x", 8)?;
        let get = || call(&lua, "GetTeleportData", r, &MultiValue::new()).unwrap();
        let Value::Table(first) = get()?.front().unwrap().clone() else {
            panic!()
        };
        assert_eq!(first.get::<i32>("x")?, 7);
        first.set("x", 9)?;
        let Value::Table(second) = get()?.front().unwrap().clone() else {
            panic!()
        };
        assert_eq!(second.get::<i32>("x")?, 7);
        Ok(())
    }
}
