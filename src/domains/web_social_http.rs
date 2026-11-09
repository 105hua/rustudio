//! Yielding login-session web reads. Workers never access the Lua state.
use super::*;
use rustudio_account::web_social::Read;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
fn login_session() -> Result<Arc<rustudio_account::Session>, String> {
    static CACHE: Mutex<Option<(String, Arc<rustudio_account::Session>)>> = Mutex::new(None);
    let config = rustudio_account::AccountConfig::load().map_err(|e| e.to_string())?;
    let (cookie, _) = rustudio_account::session::load_cookie(!config.signed_out)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "signed out".to_string())?;
    let key = cookie.cache_key();
    let mut cache = CACHE
        .lock()
        .map_err(|_| "Session cache unavailable".to_string())?;
    if let Some((old, session)) = &*cache {
        if old == &key {
            return Ok(session.clone());
        }
    }
    let session = Arc::new(rustudio_account::Session::validate(cookie).map_err(|e| e.to_string())?);
    *cache = Some((key, session.clone()));
    Ok(session)
}
#[derive(Default)]
struct Jobs {
    scope: rustudio_account::web_social::CacheScope,
    next: u64,
    pending: HashMap<u64, Receiver<Result<String, String>>>,
}

pub const LUA_METHODS: &[(&str, &str)] = &[
    ("OpenShop", "MarketplaceService"),
    ("PromptExperienceDetailsAsync", "TeleportService"),
    ("PromptFeedbackSubmissionAsync", "SocialService"),
    ("GetCampaignEligibilityAsync", "AdService"),
    ("RegisterAdOpportunityAsync", "AdService"),
    ("UnregisterAdOpportunity", "AdService"),
    ("GetCommerceProductInfoAsync", "CommerceService"),
    ("GetRobloxSubscriptionDetailsAsync", "MarketplaceService"),
    ("GetPartyAsync", "SocialService"),
    ("GetPlayersByPartyId", "SocialService"),
    ("GetPlayerPlaceInstanceAsync", "TeleportService"),
    ("RankProductsAsync", "MarketplaceService"),
    ("RecommendTopProductsAsync", "MarketplaceService"),
    ("LogActionEvent", "RecommendationService"),
    ("LogImpressionEvent", "RecommendationService"),
    ("LogPreferenceEvent", "RecommendationService"),
    ("RegisterItemAsync", "RecommendationService"),
    ("UpdateItemAsync", "RecommendationService"),
    ("RemoveItemAsync", "RecommendationService"),
    ("GenerateItemListAsync", "RecommendationService"),
    ("GetRecommendationItemAsync", "RecommendationService"),
    ("ReserveServer", "TeleportService"),
    ("ReserveServerAsync", "TeleportService"),
    ("Teleport", "TeleportService"),
    ("TeleportToSpawnByName", "TeleportService"),
    ("TeleportToPlaceInstance", "TeleportService"),
    ("TeleportToPrivateServer", "TeleportService"),
    ("PromptGameInvite", "SocialService"),
    ("PromptPhoneBook", "SocialService"),
    ("PromptLinkSharing", "SocialService"),
    ("PromptLinkSharingAsync", "SocialService"),
    ("CanSendGameInviteAsync", "SocialService"),
    ("CanSendCallInviteAsync", "SocialService"),
    ("CanPromptOptInAsync", "ExperienceNotificationService"),
    ("PromptOptIn", "ExperienceNotificationService"),
    ("CanViewBrandProjectAsync", "PolicyService"),
    ("GetUsersPriceLevelsAsync", "MarketplaceService"),
    ("GetUserSubscriptionDetailsAsync", "MarketplaceService"),
    (
        "GetUserSubscriptionPaymentHistoryAsync",
        "MarketplaceService",
    ),
    ("GetUserBadgesAsync", "BadgeService"),
    ("BindReceiptHandler", "MarketplaceService"),
    ("PromptPremiumPurchase", "MarketplaceService"),
    ("PromptRobloxSubscriptionPurchase", "MarketplaceService"),
    ("GetUpcomingExperienceEventsAsync", "SocialService"),
    ("GetExperienceEventAsync", "SocialService"),
    ("GetEventRsvpStatusAsync", "SocialService"),
    ("PromptRsvpToEventAsync", "SocialService"),
    ("PromptPurchase", "MarketplaceService"),
    ("PromptGamePassPurchase", "MarketplaceService"),
    ("PromptProductPurchase", "MarketplaceService"),
    ("PromptBundlePurchase", "MarketplaceService"),
    ("PromptSubscriptionPurchase", "MarketplaceService"),
    ("PromptCancelSubscription", "MarketplaceService"),
    ("GetPolicyInfoForPlayerAsync", "PolicyService"),
    ("FilterStringAsync", "TextService"),
    ("FilterAndTranslateStringAsync", "TextService"),
    ("GetProductInfo", "MarketplaceService"),
    ("GetProductInfoAsync", "MarketplaceService"),
    ("PlayerOwnsAsset", "MarketplaceService"),
    ("PlayerOwnsAssetAsync", "MarketplaceService"),
    ("PlayerOwnsBundle", "MarketplaceService"),
    ("PlayerOwnsBundleAsync", "MarketplaceService"),
    ("GetDeveloperProductsAsync", "MarketplaceService"),
    ("GetAlliesAsync", "GroupService"),
    ("GetEnemiesAsync", "GroupService"),
    ("UserOwnsGamePassAsync", "MarketplaceService"),
    ("GetSubscriptionProductInfoAsync", "MarketplaceService"),
    ("GetUserSubscriptionStatusAsync", "MarketplaceService"),
    ("GetBadgeInfoAsync", "BadgeService"),
    ("UserHasBadgeAsync", "BadgeService"),
    ("UserHasBadge", "BadgeService"),
    ("PlayerHasPass", "GamePassService"),
    ("CheckUserBadgesAsync", "BadgeService"),
    ("GetGroupInfoAsync", "GroupService"),
    ("PromptJoinAsync", "GroupService"),
    ("GetGroupsAsync", "GroupService"),
    ("GetRolesInGroupAsync", "GroupService"),
    ("GetUserInfosByUserIdsAsync", "UserService"),
    ("GetUserFromGlobalUserIdAsync", "UserService"),
    ("InternalRequestReviewableContentReviewAsync", "ModerationService"),
];
fn id(a: &mlua::Table, k: usize) -> mlua::Result<u64> {
    let n = a.get::<f64>(k)?;
    if !n.is_finite() || n < 0. {
        return Err(rt_err("Invalid ID"));
    }
    Ok(n as u64)
}
fn read(name: &str, a: &mlua::Table) -> mlua::Result<Read> {
    Ok(match name {
        "CanPromptOptInAsync" => Read::CanPromptOptIn(id(a,1)?),
        "GetPartyAsync" => Read::Party { id: a.get(1)?, universe: id(a,2)?, place: id(a,3)?, job: a.get(4)? },
        "GetCampaignEligibilityAsync" => Read::CampaignEligibility(a.get(1)?),
        "GetCommerceProductInfoAsync" => Read::CommerceProduct(a.get(1)?),
        "GetRobloxSubscriptionDetailsAsync" => Read::RobloxSubscriptionProducts(id(a, 1)?),
        "__ws_roblox_details" => Read::SubscriptionDetails { user: id(a, 1)?, subscription: a.get(2)? },
        "GetPlayerPlaceInstanceAsync" => Read::PlayerPlace {
            place: id(a, 1)?,
            user: id(a, 2)?,
        },
        "RankProductsAsync" => Read::RankProducts(serde_json::from_str(&a.get::<String>(1)?).map_err(|e| rt_err(e.to_string()))?),
        "RecommendTopProductsAsync" => Read::TopProducts(serde_json::from_str(&a.get::<String>(1)?).map_err(|e| rt_err(e.to_string()))?),
        "GenerateItemListAsync" => Read::RecommendationList(serde_json::from_str(&a.get::<String>(1)?).map_err(|e| rt_err(e.to_string()))?),
        "GetRecommendationItemAsync" => Read::RecommendationItem(a.get(1)?),
        "GetUserSubscriptionDetailsAsync" => Read::SubscriptionDetails {
            user: id(a, 1)?,
            subscription: a.get(2)?,
        },
        "CanViewBrandProjectAsync" => Read::BrandDecision(a.get(1)?),
        "GetUsersPriceLevelsAsync" => Read::PriceLevels {
            universe: id(a, 1)?,
            users: a
                .get::<Table>(2)?
                .sequence_values::<u64>()
                .collect::<mlua::Result<_>>()?,
        },
        "GetUpcomingExperienceEventsAsync" => Read::UpcomingEvents(id(a, 1)?),
        "GetExperienceEventAsync" | "GetEventRsvpStatusAsync" => Read::ExperienceEvent(a.get(1)?),
        "GetPolicyInfoForPlayerAsync" => Read::Policy,
        "GetProductInfo" | "GetProductInfoAsync" => Read::Product {
            id: id(a, 1)?,
            kind: a.get(2)?,
        },
        "PlayerOwnsAsset"
        | "PlayerOwnsAssetAsync"
        | "PlayerOwnsBundle"
        | "PlayerOwnsBundleAsync" => Read::Owns {
            user: id(a, 1)?,
            id: id(a, 2)?,
            bundle: name.contains("Bundle"),
        },
        "GetDeveloperProductsAsync" => Read::DeveloperProducts {
            universe: id(a, 1)?,
            cursor: a.get(2)?,
        },
        "GetAlliesAsync" | "GetEnemiesAsync" => Read::Relationships {
            group: id(a, 1)?,
            allies: name == "GetAlliesAsync",
            page: id(a, 2)?,
        },
        "UserOwnsGamePassAsync" => Read::PassOwnership {
            user: id(a, 1)?,
            pass: id(a, 2)?,
        },
        "GetSubscriptionProductInfoAsync" => Read::SubscriptionProduct(a.get(1)?),
        "GetUserSubscriptionStatusAsync" => Read::SubscriptionStatus {
            user: id(a, 1)?,
            subscription: a.get(2)?,
        },
        "GetBadgeInfoAsync" => Read::Badge(id(a, 1)?),
        "UserHasBadgeAsync" => Read::HasBadge {
            user: id(a, 1)?,
            badge: id(a, 2)?,
        },
        "CheckUserBadgesAsync" | "GetUserBadgesAsync" => Read::CheckBadges {
            user: id(a, 1)?,
            badges: a
                .get::<mlua::Table>(2)?
                .sequence_values::<i64>()
                .collect::<mlua::Result<_>>()?,
        },
        "GetGroupInfoAsync" => Read::Group(id(a, 1)?),
        "GetGroupsAsync" => Read::Groups(a.get(1)?),
        "GetRolesInGroupAsync" => Read::GroupRoles {
            user: id(a, 1)?,
            group: id(a, 2)?,
        },
        "GetUserInfosByUserIdsAsync" => Read::Users(
            a.get::<mlua::Table>(1)?
                .sequence_values::<i64>()
                .collect::<mlua::Result<_>>()?,
        ),
        _ => return Err(rt_err("Unsupported web read")),
    })
}
pub(super) fn background(
    lua: &Lua,
    work: impl FnOnce(std::sync::Arc<rustudio_account::Session>, rustudio_account::web_social::CacheScope) -> Result<serde_json::Value,String> + Send + 'static,
) -> mlua::Result<u64> {
    let (tx,rx)=mpsc::channel();
    let mut jobs=lua.app_data_mut::<Jobs>().ok_or_else(||rt_err("Web requests unavailable"))?;
    jobs.next+=1;let id=jobs.next;jobs.pending.insert(id,rx);
    let scope=jobs.scope.clone();drop(jobs);
    std::thread::spawn(move || {
        let result=login_session().and_then(|session|work(session,scope)).map(|value|value.to_string());
        let _=tx.send(result);
    });
    Ok(id)
}
pub fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set("__ws_info_type", lua.create_function(|_, value: Value| {
        Ok(opt_ud::<LEnumItem>(&value).filter(|item| item.enum_type == "InfoType").map(|item| item.name.clone()))
    })?)?;
    lua.globals().set(
        "__ws_owned",
        lua.create_function(|lua, (kind, user, id): (String, u64, u64)| {
            state(lua)?.raw_get::<Option<bool>>(format!("owned:{kind}:{user}:{id}"))
        })?,
    )?;
    lua.set_app_data(Jobs::default());
    lua.globals().set(
        "__ws_begin",
        lua.create_function(|lua, (name, a): (String, mlua::Table)| {
            let request = read(&name, &a)?;
            background(lua,move |session,scope|request.fetch_in(&session,&scope).map_err(|e|e.to_string()))
        })?,
    )?;
    lua.globals().set(
        "__ws_poll",
        lua.create_function(|lua, id: u64| {
            let mut jobs = lua.app_data_mut::<Jobs>().unwrap();
            let Some(rx) = jobs.pending.get(&id) else {
                return Err(rt_err("Unknown web request"));
            };
            let result = match rx.try_recv() {
                Err(TryRecvError::Empty) => return Ok((false, None, None)),
                Err(_) => Err("Web request worker disconnected".into()),
                Ok(r) => r,
            };
            jobs.pending.remove(&id);
            Ok(match result {
                Ok(s) => (true, Some(s), None),
                Err(e) => (true, None, Some(e)),
            })
        })?,
    )?;
    lua.globals().set(
        "__ws_page_copy",
        lua.create_function(|lua, value: Table| copy(lua, Value::Table(value), &mut Vec::new()))?,
    )?;
    lua.globals().set(
        "__ws_filter_text",
        lua.create_function(|lua, inst: Value| {
            let r = opt_inst(&inst).ok_or_else(|| rt_err("Instance expected"))?;
            state(lua)?.raw_get::<Value>(format!("filter:{r:?}:text"))
        })?,
    )?;
    lua.globals().set("__ws_filter",lua.create_function(|lua,(text,user,translated):(String,i64,bool)| {
        if world(lua)?.side != Side::Server {return Err(rt_err("Must only be called from server scripts"))}
        let connected=world(lua)?.d().dom.descendants().any(|i|i.class.as_str()=="Player" && matches!(i.properties.get(&"UserId".into()),Some(rbx_dom_weak::types::Variant::Int64(id)) if *id==user));
        if !connected {return Err(rt_err("The sender must be connected to the current server"))}
        let source=crate::runtime::ext::create_any(lua,"TextFilterResult")?;
        state(lua)?.raw_set(format!("filter:{source:?}:text"),text)?;
        if !translated { return inst_value(lua,source); }
        let r=crate::runtime::ext::create_any(lua,"TextFilterTranslatedResult")?;
        state(lua)?.raw_set(format!("filter:{r:?}:SourceText"),inst_value(lua,source)?)?;
        inst_value(lua,r)
    })?)?;
    lua.load(r#"
local M=__rs_lua_methods
for _,method in {'CanSendGameInviteAsync','CanSendCallInviteAsync'} do
    M[method]=function(self,player,recipient)
        if typeof(player)~='Instance' then error('Argument 1 missing or nil',2) end
        local client=game:GetService('RunService'):IsClient()
        if method=='CanSendGameInviteAsync' and not client then error('SocialService:CanSendGameInviteAsync Must be called from a local script.',0) end
        if client and player~=game:GetService('Players').LocalPlayer then error('SocialService:'..method..' Must prompt the LocalPlayer when called from a local script.',0) end
        task.wait();return true
    end
end
M.GetNonChatStringForBroadcastAsync=function(self) return __ws_filter_text(self) end
M.GetNonChatStringForUserAsync=function(self,user) if tonumber(user)==nil then error('Argument 1 missing or nil',2) end;return __ws_filter_text(self) end
M.GetChatForUserAsync=function(self,user) if tonumber(user)==nil then error('Argument 1 missing or nil',2) end;return __ws_filter_text(self) end
-- __ws_filter's refusals reach scripts as the engine's plain message (a
-- Rust error raised under this wrapper carried mlua's "runtime error:"
-- prefix and a traceback into pcall).
local function filter(text,user,translated)
    local ok,r=pcall(__ws_filter,text,user,translated)
    if ok then return r end
    error((string.match(string.gsub(tostring(r),'^runtime error: ',''),'^[^\n]*')),0)
end
M.FilterStringAsync=function(self,text,user)
    return filter(text,user,false)
end
M.FilterAndTranslateStringAsync=function(self,text,user,locales)
    return filter(text,user,true)
end
local function fetch(name,...)
    local args={...};local token=__ws_begin(name,args)
    while true do
        local done,body,err=__ws_poll(token)
        if done then
            if err then
                if name=='CanPromptOptInAsync' and string.match(err,'^HTTP 429') then return {canPrompt=false} end
                if name=='CanPromptOptInAsync' then err='ExperienceNotificationService request failed: '..err
                elseif name=='GetPartyAsync' then err=err=='Unable to convert response to JSON' and 'SocialService:GetPartyAsync An unknown error occurred.' or 'SocialService:GetPartyAsync Request failed: %s'
                elseif name=='GetCampaignEligibilityAsync' then err='Request failed'
                elseif name=='GetRobloxSubscriptionDetailsAsync' then err='SubscriptionRequest::listAvailableSubscriptionProductsAsync failed because: HTTP '..(string.match(err,'^HTTP (%d+)') or err)
                elseif name=='__ws_roblox_details' then err='MarketPlaceService::GetUserSubscriptionDetailsAsync failed because: '..err
                elseif name=='RankProductsAsync' or name=='RecommendTopProductsAsync' then err='task: wait for '..(name=='RankProductsAsync' and 'RankProducts' or 'TopRecommendedProducts')..' - Failed: HTTP request failed with status code: '..(string.match(err,'^HTTP (%d+)') or err)
                elseif name=='GenerateItemListAsync' then err='Failed to fetch recommendation items - '..err
                elseif name=='GetRecommendationItemAsync' then err='Failed to get item - '..err
                elseif name=='GetUserInfosByUserIdsAsync' then err='UserService: Request Failed because '..err
                elseif name=='GetExperienceEventAsync' and(string.match(err,'^HTTP 404')or err=='Unable to convert response to JSON')then return nil
                elseif name=='GetEventRsvpStatusAsync' then err=err=='Unable to convert response to JSON'and('Error parsing event details for eventId '..tostring(args[1])..': Invalid response format.')or('Error getting event details: '..err)
                elseif name=='GetUpcomingExperienceEventsAsync' or name=='GetExperienceEventAsync' then err='HttpError '..(string.match(err,'^HTTP (%d+)') or err)
                elseif name=='UserOwnsGamePassAsync' then err='MarketplaceService::userOwnsGamePassAsync - GetGamePassOwnership failed: HTTP request failed with status code: '..(string.match(err,'^HTTP (%d+)') or err)
                elseif name=='GetBadgeInfoAsync' and string.match(err,'^HTTP 404') then err='badgeId is invalid or does not exist.'
                elseif name=='UserHasBadgeAsync' then err='UserHasBadgeAsync: Internal error.'
                elseif name=='GetUserBadgesAsync' and string.match(err,'^HTTP 404') then err='GetUserBadgesAsync: User is invalid or does not exist.'
                elseif name=='CheckUserBadgesAsync' or name=='GetUserBadgesAsync' then err=name..': '..(string.match(err,'^HTTP 429') and 'Too many requests' or err)
                elseif name=='GetUserSubscriptionStatusAsync' or name=='GetUserSubscriptionDetailsAsync' then err='MarketPlaceService::'..name..' failed because: '..err
                elseif name=='GetProductInfo' or name=='GetProductInfoAsync' then
                    if args[2]=='Product' or args[2]=='GamePass' then err='MarketplaceService::getProductInfo - '..(args[2]=='Product' and 'GetDeveloperProductInfo' or 'GetGamePassInfo')..' failed: HTTP request failed with status code: '..(string.match(err,'^HTTP (%d+)') or err)
                    else err='MarketplaceService:getProductInfo() failed because '..err end
                elseif name=='GetGroupInfoAsync' or name=='GetGroupsAsync' or name=='GetRolesInGroupAsync' then err='GroupService:'..name..'() failed because '..err end
                error(err,0)
            end
            return game:GetService('HttpService'):JSONDecode(body)
        end
        task.wait()
    end
end
M.CanPromptOptInAsync=function(self)
    if not game:GetService('RunService'):IsClient() then error('ExperienceNotificationService:CanPromptOptInAsync can only be executed by a local script',0) end
    if (tonumber(game.GameId) or 0)==0 then task.wait();return true end
    local value=fetch('CanPromptOptInAsync',game.GameId)
    if type(value)~='table' then error('Variant cast failed',0) end
    if value.canPrompt==nil then error('ExperienceNotificationService request failed: Invalid response received from Notifications API',0) end
    if type(value.canPrompt)~='boolean' then error('Variant cast failed',0) end
    return value.canPrompt
end
local function canonical(value)
    if type(value)~='table' then return value end
    local out={};for k,v in value do local key=type(k)=='string' and string.upper(string.sub(k,1,1))..string.sub(k,2) or k;out[key]=canonical(v)end;return out
end
local function product(self,id,kind)
    id=tonumber(id);if id==nil then error('Argument 1 missing or nil',2) end
    id=math.floor(id);if id<=0 then error('MarketplaceService:GetProductInfo() '..tostring(id)..' is not a valid assetId',0) end
    local name=kind and kind.Name or 'Asset'
    if name=='Subscription' then error('MarketplaceService:GetProductInfo() does not support InfoType INFO_SUBSCRIPTION',0) end
    local v=fetch('GetProductInfo',id,name)
    if name=='Product' then
        for _,entry in v.developerProducts or {} do if entry.ProductId==id then return canonical(entry) end end
        error('MarketplaceService::getProductInfo - GetDeveloperProductInfo failed: Item not found in batch response.',0)
    elseif name=='GamePass' then
        local pass
        for _,entry in v.gamePasses or {} do if entry.id==id then pass=entry;break end end
        if not pass then error('MarketplaceService::getProductInfo - GetGamePassInfo failed: Item not found in batch response.',0)end
        local c=pass.creator or {}
        return {AssetId=0,AssetTypeId=0,TargetId=pass.id,ProductId=pass.productId,ProductType='Game Pass',Name=pass.displayName or pass.name,Description=pass.displayDescription or '',IconImageAssetId=pass.displayIconImageAssetId or 0,Created=pass.created,Updated=pass.updated,PriceInRobux=pass.price,UserBasePriceInRobux=pass.userBasePriceInRobux or 0,PriceDiscountDetails=canonical(pass.priceDiscountDetails or {}),Creator={Id=c.deprecatedId,CreatorTargetId=c.creatorId,CreatorType=c.creatorType,Name=c.name},IsNew=false,IsForSale=pass.isForSale==true,IsPublicDomain=false,IsLimited=false,IsLimitedUnique=false,Sales=0,MinimumMembershipLevel=0}
    elseif name=='Bundle' then
        local items={};local types={};for _,item in Enum.AssetType:GetEnumItems()do types[item.Value]=item.Name end
        for _,x in v.items or {} do table.insert(items,{Id=x.id,Name=x.name,Type=x.type,AssetType=types[x.assetType]})end
        return {Id=v.id,Name=v.name,Description=v.description,BundleType=v.bundleType,Items=items,UserBasePriceInRobux=v.userBasePriceInRobux or 0,PriceDiscountDetails=canonical(v.priceDiscountDetails or {})}
    end
    return canonical(v)
end
-- The rank/recommend APIs return ranked identifiers, then their metadata.
-- Native current settings: 60s cache; 10/5 calls per minute; rank fallback off.
local ranking_cache={};local ranking_windows={}
local function info_type(value)
    return __ws_info_type(value)
end
local function market_context(method)
    if game:GetService('RunService'):IsServer() then error('MarketplaceService:'..method..'() Cannot be called from the Server.',0) end
end
local function market_rank(method,body,skip_failed_info)
    local now=os.clock();local limit=method=='RankProductsAsync' and 10 or 5
    local window=ranking_windows[method]
    if not window or now-window.start>60 then window={start=now,count=0};ranking_windows[method]=window end
    window.count+=1
    if window.count>limit then error('MarketplaceService:'..method..'() request was throttled. This API is limited to '..limit..' requests per minute.',0) end
    local encoded=game:GetService('HttpService'):JSONEncode(body)
    local key=method..':'..encoded
    local cached=ranking_cache[key]
    if cached and now-cached.at<60 then task.wait();return __ws_page_copy(cached.value) end
    local response=fetch(method,encoded)
    local fetched_at=os.clock()
    if type(response.rankedItems)~='table' then error('task: wait for '..(method=='RankProductsAsync' and 'RankProducts' or 'TopRecommendedProducts').." - Failed: 'rankedItems' field missing or invalid in response.",0) end
    local out={}
    for index,item in response.rankedItems or {} do
        local identifier=item.productIdentifier
        if type(identifier)~='table' or type(identifier.id)~='number' or (identifier.type~='Product' and identifier.type~='GamePass') then
            error("'RecommendTopProducts Backend response at index "..(index-1).." is invalid.",0)
        end
        local kind=Enum.InfoType[identifier.type]
        local ok,metadata=pcall(product,nil,identifier.id,kind)
        if ok then
            table.insert(out,{ProductIdentifier={Id=identifier.id,InfoType=kind},ProductInfo=metadata})
        elseif not skip_failed_info then
            error('Error received from GetProductInfo for [Id: '..identifier.id..', InfoType: '..identifier.type..'] at index: '..index..'. Error: '..tostring(metadata),0)
        end
    end
    ranking_cache[key]={at=fetched_at,value=__ws_page_copy(out)}
    return out
end
M.RankProductsAsync=function(self,identifiers)
    if type(identifiers)~='table' then error('Argument 1 missing or nil',2) end
    if #identifiers==0 then error('MarketplaceService:RankProductsAsync() must have at least one ProductIdentifier.',0) end
    if #identifiers>50 then error('MarketplaceService:RankProductsAsync() must have fewer than 50 ProductIdentifiers in the input.',0) end
    market_context('RankProductsAsync')
    local body={universeId=game.GameId,productIdentifiers={}};local seen={}
    for index,entry in identifiers do
        local prefix='MarketplaceService:RankProductsAsync(): '
        if type(entry)~='table' then error(prefix..'Invalid ProductIdentifier at index '..index,0) end
        if entry.Id==nil then error(prefix.."ProductIdentifier at index "..index.." does not contain 'Id'",0) end
        if type(entry.Id)~='number' then error(prefix..'ProductIdentifier at index '..index..' does not contain valid Id',0) end
        if entry.InfoType==nil then error(prefix.."ProductIdentifier at index "..index.." does not contain 'InfoType'",0) end
        local kind=info_type(entry.InfoType)
        if not kind then error(prefix..'ProductIdentifier at index '..index..' does not contain valid InfoType',0) end
        if kind~='Product' and kind~='GamePass' then error(prefix..'Found Unsupported InfoType at index '..index..'. Only GamePass and Product are currently supported.',0) end
        local id=entry.Id>=0 and math.floor(entry.Id) or math.ceil(entry.Id)
        local key=kind..':'..id
        if not seen[key] then table.insert(body.productIdentifiers,{id=id,type=kind});seen[key]=true end
    end
    return market_rank('RankProductsAsync',body,false)
end
M.RecommendTopProductsAsync=function(self,types)
    if type(types)~='table' then error('Argument 1 missing or nil',2) end
    market_context('RecommendTopProductsAsync')
    if #types==0 then error('MarketplaceService:RecommendTopProductsAsync() must specify at least InfoType',0) end
    local seen={}
    for index,value in types do
        local prefix='MarketplaceService:RecommendTopProductsAsync(): '
        local kind=info_type(value)
        if not kind then error(prefix..'Invalid InfoType at index '..(index-1),0) end
        if kind~='Product' and kind~='GamePass' then error(prefix..'Found Unsupported InfoType at index '..(index-1)..'. Only GamePass and Product are currently supported.',0) end
        if seen[kind] then error(prefix..'InfoType at index '..(index-1)..' is a duplicate. Specify the required infoTypes only once.',0) end
        seen[kind]=true
    end
    local body={universeId=game.GameId,productInfoTypes={}}
    for _,kind in {'Product','GamePass'} do if seen[kind] then table.insert(body.productInfoTypes,kind)end end
    return market_rank('RecommendTopProductsAsync',body,true)
end
-- PurchasePrompt's Network.getBundleDetails uses the catalog shape, including
-- product availability/price. The public GetProductInfo shape is different.
__ws_bundle_details=function(id)return fetch('GetProductInfo',id,'Bundle')end
M.GetProductInfo=product;M.GetProductInfoAsync=product
for _,name in {'PlayerOwnsAsset','PlayerOwnsAssetAsync','PlayerOwnsBundle','PlayerOwnsBundleAsync'} do
    M[name]=function(self,player,id)
        if typeof(player)~='Instance' or not player:IsA('Player') then error('Argument 1 missing or nil',2) end
        local kind=name:find('Bundle') and 'Bundle' or 'Asset'
        local owned=__ws_owned(kind,player.UserId,id);if owned~=nil then return owned end
        return fetch(name,player.UserId,id)
    end
end
local function clone_policy(value)
    if type(value)~='table' then return value end
    local out={};for k,v in value do out[k]=clone_policy(v) end;return out
end
local policies=setmetatable({},{__mode='k'})
M.GetPolicyInfoForPlayerAsync=function(self,player)
    if typeof(player)~='Instance' or not player:IsA('Player') then error('Argument 1 missing or nil',2) end
    if policies[player] then return clone_policy(policies[player]) end
    local v=fetch('GetPolicyInfoForPlayerAsync');local result={}
    for key,value in v do result[string.upper(string.sub(key,1,1))..string.sub(key,2)]=value end
    policies[player]=result
    return clone_policy(result)
end
-- Native parser : skip ended/cancelled/moderated/unpublished
-- records, preserve backend order, and expose UTC date tables and enum items.
local function event_record(v)
    if type(v)~='table' or type(v.id)~='string' or type(v.eventTime)~='table' then return nil end
    local start=DateTime.fromIsoDate(v.eventTime.startUtc or '')
    local finish=DateTime.fromIsoDate(v.eventTime.endUtc or '')
    if not start or not finish then return nil end
    local status=({active='Active',cancelled='Cancelled',moderated='Moderated',unpublished='Unpublished'})[v.eventStatus] or 'Unknown'
    local now=DateTime.now().UnixTimestampMillis
    local record={Id=v.id,Title=v.title,Description=v.description,StartTime=start:ToUniversalTime(),EndTime=finish:ToUniversalTime(),HasStarted=start.UnixTimestampMillis<=now,HasEnded=finish.UnixTimestampMillis<now,Status=Enum.ExperienceEventStatus[status]}
    if type(v.userRsvpStatus)=='string' then record.UserRsvpStatus=Enum.RsvpStatus[({going='Going',notGoing='NotGoing'})[v.userRsvpStatus] or 'None'] end
    return record,status
end
local function event_read_context()local r=game:GetService('RunService');return r:IsClient()or(r.IsEdit and r:IsEdit())end
M.GetUpcomingExperienceEventsAsync=function(self)
    if not event_read_context() then error('SocialService:GetUpcomingExperienceEventsAsync must be called on the client.',0) end
    local response=fetch('GetUpcomingExperienceEventsAsync',game.GameId);local out={}
    for _,v in response.data or {} do
        local event,status=event_record(v)
        if event and not event.HasEnded and status~='Cancelled' and status~='Moderated' and status~='Unpublished' then table.insert(out,event) end
    end
    return out
end
local function event_id(method,id)
    if id==nil then error('Argument 1 missing or nil',0)end
    if type(id)~='string'and type(id)~='number'then error('Unable to cast value to std::string',0)end
    id=tostring(id)
    local canonical=string.gsub(id,'^0+','')
    if not string.match(id,'^%d+$')or canonical==''or #canonical>19 or(#canonical==19 and canonical>'9223372036854775807')then error('SocialService:'..method..' Received invalid event id.',0)end
    if not event_read_context()then error('SocialService:'..method..' must be called on the client.',0)end
    return id
end
M.GetExperienceEventAsync=function(self,id)
    id=event_id('GetExperienceEventAsync',id)
    local response=fetch('GetExperienceEventAsync',id)
    if type(response)~='table'or type(response.universeId)~='number'or response.universeId~=game.GameId then return nil end
    local event=event_record(response);return event
end
M.GetEventRsvpStatusAsync=function(self,id)
    id=event_id('GetEventRsvpStatusAsync',id)
    local response=fetch('GetEventRsvpStatusAsync',id)
    local time=type(response)=='table'and response.eventTime
    if type(response)~='table'or type(response.universeId)~='number'or type(response.userRsvpStatus)~='string'or response.universeId%1~=0 or type(time)~='table'or type(time.startUtc)~='string'or type(time.endUtc)~='string'or not DateTime.fromIsoDate(time.startUtc)or not DateTime.fromIsoDate(time.endUtc)then error('Error parsing event details for eventId '..id..': Invalid response format.',0)end
    if response.universeId~=game.GameId then error('SocialService:GetEventRsvpStatusAsync Event is not in the current universe.',0)end
    return Enum.RsvpStatus[({going='Going',notGoing='NotGoing'})[response.userRsvpStatus] or 'None']
end
-- Shared Pages (src/domains/pages.rs): next_page() → (page, finished).
local function make_pager(next_page,class)
    return __rs_pages_new(class or 'StandardPages',next_page)
end
M.GetDeveloperProductsAsync=function(self)
    local cursor='';local seen={}
    return make_pager(function()
        local v=fetch('GetDeveloperProductsAsync',game.GameId,cursor)
        local next_cursor=v.nextPageCursor or ''
        if next_cursor~='' and seen[next_cursor] then error('Repeated developer product page cursor',0) end
        seen[next_cursor]=true;cursor=next_cursor
        return v.developerProducts or {},cursor==''
    end)
end
for _,name in {'GetAlliesAsync','GetEnemiesAsync'} do
    M[name]=function(self,id)
        local index=1
        return make_pager(function()
            local v=fetch(name,id,index);index+=1
            return v.Groups or {},v.FinalPage==true
        end)
    end
end
M.UserOwnsGamePassAsync=function(self,user,id)
    local numeric_user,numeric_id=tonumber(user),tonumber(id)
    if numeric_user and numeric_id and (numeric_user<=0 or numeric_id<=0) then return false end
    local owned=__ws_owned('GamePass',user,id);if owned~=nil then return owned end
    local v=fetch('UserOwnsGamePassAsync',user,id)
    if v.ownerships then
        for _,entry in v.ownerships do
            if entry.userId==user and entry.gamePassId==id then return entry.owned==true end
        end
    end
    for _,entry in v.ownershipIdentifiers or {} do
        if entry.userId==user and entry.gamePassId==id then return true end
    end
    return false
end
-- Native  parses per-user locations;  overlays
-- connected players, removes stale local entries, and sorts newest join first.
local party_requests=0;local party_window=0
local function valid_party_id(id)
    return type(id)=='string' and #id==36 and string.match(id,'^%x%x%x%x%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%x%x%x%x%x%x%x%x$')~=nil
end
M.GetPartyAsync=function(self,id)
    if not game:GetService('RunService'):IsServer() then error('SocialService:GetPartyAsync Must be called from a server script.',0) end
    if type(id)~='string' then error('Argument 1 missing or nil',0) end
    if not valid_party_id(id) then error('SocialService:GetPartyAsync Received invalid party id.',0) end
    local now=os.clock();if now-party_window>60 then party_window=now;party_requests=0 end
    party_requests+=1
    if party_requests>120 then error('SocialService:GetPartyAsync Request was throttled. This API is limited to 120 requests per minute.',0) end
    local v=fetch('GetPartyAsync',id,game.GameId,game.PlaceId,game.JobId)
    if type(v)~='table' then error('SocialService:GetPartyAsync An unknown error occurred.',0) end
    local members={}
    for _,entry in type(v.universeMembers)=='table' and v.universeMembers or {} do
        if type(entry)=='table' and type(entry.userId)=='number' and entry.userId>0 and type(entry.placeId)=='number' and entry.placeId>0 and type(entry.instanceId)=='string' and entry.instanceId~='' then
            local record={UserId=entry.userId,PlaceId=entry.placeId,JobId=entry.instanceId}
            if type(entry.privateServerId)=='string' and entry.privateServerId~='' then record.PrivateServerId=entry.privateServerId end
            if type(entry.reservedServerAccessCode)=='string' and entry.reservedServerAccessCode~='' then record.ReservedServerAccessCode=entry.reservedServerAccessCode end
            members[entry.userId]={record=record,time=type(entry.latestGroupUpJoinTimeMs)=='number' and entry.latestGroupUpJoinTimeMs or 0}
        end
    end
    for _,player in game:GetService('Players'):GetPlayers() do
        if player.PartyId==id then
            local record={UserId=player.UserId,PlaceId=game.PlaceId,JobId=game.JobId}
            if game.PrivateServerId and game.PrivateServerId~='' then record.PrivateServerId=game.PrivateServerId end
            if type(v.reservedServerAccessCode)=='string' and v.reservedServerAccessCode~='' then record.ReservedServerAccessCode=v.reservedServerAccessCode end
            members[player.UserId]={record=record,time=0}
        else members[player.UserId]=nil end
    end
    local ordered={};for _,entry in members do table.insert(ordered,entry) end
    table.sort(ordered,function(a,b)return a.time>b.time end)
    local out={};for _,entry in ordered do table.insert(out,entry.record) end
    return out
end
M.GetPlayersByPartyId=function(self,id)
    if not game:GetService('RunService'):IsServer() then error('SocialService:GetPlayersByPartyId Must be called from a server script.',0) end
    if type(id)~='string' then error('Argument 1 missing or nil',0) end
    if #id~=36 or string.sub(id,9,9)~='-' or string.sub(id,14,14)~='-' or string.sub(id,19,19)~='-' or string.sub(id,24,24)~='-' or not string.match((string.gsub(id,'-','')),'^%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x%x$') then return {} end
    local out={};for _,player in game:GetService('Players'):GetPlayers() do if player.PartyId==id then table.insert(out,player)end end;return out
end
M.GetPlayerPlaceInstanceAsync=function(self,user)
    if not game:GetService('RunService'):IsServer() then error('TeleportService::GetPlayerPlaceInstanceAsync can only be called from the server.',0) end
    local v=fetch('GetPlayerPlaceInstanceAsync',game.PlaceId,user)
    if type(v.PlaceId)~='number' or type(v.GameId)~='string' then error('Unexpected result',0) end
    return v.GameId==game.JobId,'',v.PlaceId,v.GameId
end
local function recommendation_item(v,tracing)
    if type(v)~='table' then return {} end
    local function named(kind,name)
        if type(name)~='string' then return nil end
        local ok,value=pcall(function()return Enum[kind][name]end);if ok and value~=nil then return value end
        for _,item in Enum[kind]:GetEnumItems() do if string.lower(item.Name)==string.lower(name) then return item end end
    end
    local out={ItemId=v.itemId or '',ReferenceId=v.referenceId or '',CustomTags=clone_policy(v.customTags or {}),TracingId=tracing or ''}
    if v.creator then out.Creator={CreatorId=tonumber(v.creator.creatorId),CreatorType=named('CreatorType',v.creator.creatorType)} end
    out.Visibility=named('RecommendationItemVisibility',v.visibilityStatus)
    local a=v.contentAttributes or {};local attrs={AssetId=tonumber(a.assetId),Text=a.text,Description=a.contentDescription}
    for old,new in {seekStartTimeSeconds='SeekStartTime',seekEndTimeSeconds='SeekEndTime',trimStartTimeSeconds='TrimStartTime',trimEndTimeSeconds='TrimEndTime'} do attrs[new]=a[old] end
    out.Attributes=attrs
    if v.stats then
        out.Stats={Impressions={},Actions={}}
        for _,x in v.stats.impressions or {} do table.insert(out.Stats.Impressions,{ImpressionType=named('RecommendationImpressionType',x.impressionType),Counts=clone_policy(x.counts or {})})end
        for _,x in v.stats.actions or {} do table.insert(out.Stats.Actions,{ActionType=named('RecommendationActionType',x.actionType),ReactionType=x.reactionType,Counts=clone_policy(x.counts or {})})end
    end
    return out
end
M.GetRecommendationItemAsync=function(self,id)
    if not game:GetService('RunService'):IsServer() then error('GetRecommendationItemAsync must run as Server Script.',0) end
    if type(id)~='string' then error('Argument 1 missing or nil',0) end
    return recommendation_item(fetch('GetRecommendationItemAsync',id).item)
end
M.GenerateItemListAsync=function(self,request)
    if type(request)~='table' then error('Argument 1 missing or nil',2) end
    if type(request.ConfigName)~='string' then error('generateItemListAsync: ConfigName must be provided',0) end
    if request.LocationId==nil then error('generateItemListAsync: LocationId must be provided',0) end
    if type(request.LocationId)~='string' then error('generateItemListAsync: LocationId must be a string',0) end
    local allowed={ConfigName=true,LocationId=true,PageSize=true,BoostCustomTags=true,CustomContexts=true}
    for key in request do if not allowed[key] then error('Unexpected key in GenerateRecommendationItemListRequest: '..tostring(key)..'. Allowed keys are: ConfigName, LocationId, PageSize, BoostCustomTags, CustomContexts',0) end end
    if request.BoostCustomTags~=nil then
        if type(request.BoostCustomTags)~='table' then error('GenerateRecommendationItemListRequest: BoostCustomTags must be an array',0) end
        for _,tag in request.BoostCustomTags do if type(tag)~='string' then error('BoostCustomTags must be a string array',0) end end
    end
    local contexts=request.CustomContexts
    if contexts~=nil and type(contexts)~='table' then error('GenerateRecommendationItemListRequest: CustomContexts must be a table',0) end
    if game:GetService('RunService'):IsServer() then
        if contexts==nil then error('generateItemListAsync: (Server script) CustomContexts must be provided and contain UserId key',0) end
        if contexts.UserId==nil then error('generateItemListAsync: CustomContexts must contain UserId key for server script',0) end
    end
    for key,value in contexts or {} do if type(value)~='string' then error('CustomContexts table values must be strings: error for key = '..tostring(key),0) end end
    local http=game:GetService('HttpService')
    local body={experienceId=game.GameId,configName=request.ConfigName,locationId=request.LocationId,paginationToken={count=math.floor(tonumber(request.PageSize) or 10)}}
    if request.BoostCustomTags and #request.BoostCustomTags>0 then body.boostCustomTags=clone_policy(request.BoostCustomTags) end
    if contexts and next(contexts) then
        body.realtimeRenderContext={}
        local keys={};for key in contexts do table.insert(keys,key)end;table.sort(keys)
        for _,key in keys do table.insert(body.realtimeRenderContext,{key=key,value=contexts[key]})end
    end
    return make_pager(function()
        local response=fetch('GenerateItemListAsync',http:JSONEncode(body))
        local items={};for _,item in response.items or {} do table.insert(items,recommendation_item(item,response.tracingId or ''))end
        local cursor=response.responseToken and response.responseToken.cursor
        body.paginationToken.cursor=type(cursor)=='string' and cursor~='' and cursor or nil
        return items,body.paginationToken.cursor==nil
    end,'RecommendationPages')
end

M.CanViewBrandProjectAsync=function(self,player,id)
    if not game:GetService('RunService'):IsServer() then error('PolicyService:CanViewBrandProjectAsync can only be called on the server',0) end
    if typeof(player)~='Instance' or not player:IsA('Player') then error('PolicyService:CanViewBrandProjectAsync expected Player, got nil',0) end
    return fetch('CanViewBrandProjectAsync',id).isProjectAvailable==true
end
M.GetUsersPriceLevelsAsync=function(self,ids)
    if not game:GetService('RunService'):IsServer() then error('MarketplaceService:GetUsersPriceLevelsAsync() can only be called on the server',0) end
    if #ids==0 then error('MarketplaceService:GetUsersPriceLevelsAsync() must have at least one user ID',0) end
    local v=fetch('GetUsersPriceLevelsAsync',game.GameId,ids);local out={}
    for _,id in ids do local price=v.userPriceLevels[tostring(id)];if price~=nil then table.insert(out,{UserId=id,PriceLevel=price})end end
    return out
end
local function subscription_id(id)
    if id==''then error('Missing SubscriptionId',0)end
    local prefix,digits
    if type(id)=='string'then prefix,digits=string.match(id,'^([a-zA-Z]+)%-(%d+)$')end
    if not prefix or #prefix>5 or #digits>19 or #id>25 then error('Invalid SubscriptionId',0)end
end
M.GetSubscriptionProductInfoAsync=function(self,id)
    subscription_id(id)
    local v=fetch('GetSubscriptionProductInfoAsync',id).subscriptionProductInfo
    if not v then error('Product info not found for subscription: '..id,0) end
    -- Native parser  (GetProductInfo mode, no prompt-only fields).
    if v.subscriptionPeriod~='Month' then error('Product info not found for subscription: '..id,0) end
    local out={Name=v.name or '',Description=v.description or '',IconImageAssetId=v.iconImageAssetId or 0,SubscriptionPeriod=Enum.SubscriptionPeriod.Month,DisplayPrice=v.displayPrice or '',DisplaySubscriptionPeriod='/month',PriceTier=v.priceTier or 0,SubscriptionProviderName=v.subscriptionProviderName or '',IsForSale=v.isForSale==true,PriceInRobux=v.priceInRobux or 0,UserBasePriceInRobux=v.userBasePriceInRobux or 0,PriceDiscountDetails={}}
    for _,d in v.priceDiscountDetails or {} do local discount={Type=d.type or '',AmountInRobux=d.amountInRobux or 0,Percent=d.percent or 0};if d.endTimeMs and d.endTimeMs>0 then discount.EndTime=DateTime.fromUnixTimestampMillis(d.endTimeMs) end;table.insert(out.PriceDiscountDetails,discount)end
    return out
end
M.GetUserSubscriptionStatusAsync=function(self,player,id)
    subscription_id(id)
    local override=__ws_subscription_override and __ws_subscription_override('status',player.UserId,id)
    if override then task.wait();return override end
    local v=fetch('GetUserSubscriptionStatusAsync',player.UserId,id)
    return {IsSubscribed=v.isSubscribed==true,IsRenewing=v.isRenewing==true}
end
local campaign_cache={}
M.GetCampaignEligibilityAsync=function(self,id,player)
    if type(id)~='string' then error('Argument 1 missing or nil',2) end
    if #id>36 then error('Campaign ID should not be longer than 36 characters',0) end
    local players=game:GetService('Players')
    if game:GetService('RunService'):IsServer() then
        if typeof(player)~='Instance' or not player:IsA('Player') then error('GetCampaignEligibilityAsync must be called with a valid player on the server',0) end
        if player.Parent~=players then error('The specified player is not in the experience',0) end
    else
        if player~=nil and player~=players.LocalPlayer then error('GetCampaignEligibilityAsync may only be called for the local player',0) end
        player=players.LocalPlayer
    end
    local cache=campaign_cache[player] or {};campaign_cache[player]=cache
    local v=cache[id]
    if not v then
        local count=0;for _ in cache do count+=1 end
        if count>=50 then error('Too many campaigns in this experience (50)',0) end
        v=fetch('GetCampaignEligibilityAsync',id)
        if type(v.isEligible)~='boolean' then error('isEligible is not a bool',0) end
        if type(v.advertiserName)~='string' then error('advertiserName is not a string',0) end
        if type(v.immersiveBrandedAdPlacementIds)~='table' then error('immersiveBrandedAdPlacementIds is not an array',0) end
        if #v.immersiveBrandedAdPlacementIds>110 then error('too many immersiveBrandedAdPlacementIds: '..#v.immersiveBrandedAdPlacementIds,0) end
        for _,placement in v.immersiveBrandedAdPlacementIds do if type(placement)~='string' then error('encountered a non-string element in immersiveBrandedAdPlacementIds',0) end end
        cache[id]=v
    else task.wait() end
    if __ws_campaign_record then __ws_campaign_record(player,id,v)end
    return {IsEligible=v.isEligible}
end
M.GetCommerceProductInfoAsync=function(self,id)
    if type(id)~='string' then error('Argument 1 missing or nil',2) end
    local numeric=string.gsub(id,'^COM%-','')
    if #numeric<1 or #numeric>20 or not string.match(numeric,'^%d+$') then error('Invalid commerceProductId',0) end
    local v=fetch('GetCommerceProductInfoAsync',id)
    if type(v.isForSale)~='boolean' or type(v.item)~='table' or type(v.benefits)~='table' then error("Error when requesting product info for commerce product: "..id.." with response'"..game:GetService('HttpService'):JSONEncode(v).."'",0) end
    local item=v.item
    local out={IsForSale=v.isForSale,Item={Name=item.name or '',Description=item.description or '',IconImageAssetId=item.iconImageAssetId or 0,DisplayPrice=item.displayPrice or '',IsPurchasable=item.isPurchasable==true},Benefits={}}
    for _,b in v.benefits do
        local kind=Enum.BenefitType[b.benefitType]
        if not kind then error('Unable to parse GetCommerceProductInfo response for commerce product: '..id,0) end
        table.insert(out.Benefits,{Name=b.name or '',Description=b.description or '',Id=b.id or '',BenefitType=kind})
    end
    return out
end
M.GetRobloxSubscriptionDetailsAsync=function(self,player)
    if typeof(player)~='Instance' or not player:IsA('Player') then error('Invalid user argument.',0) end
    if not game:GetService('RunService'):IsServer() then error('Can only call GetRobloxSubscriptionDetailsAsync from Script with RunContext = Server',0) end
    local response=fetch('GetRobloxSubscriptionDetailsAsync',player.UserId)
    local keys=response.productKeys
    if type(keys)~='table' or #keys==0 then error('No product keys found in response',0) end
    if type(keys[1])~='table' then error('Invalid product key object',0) end
    if type(keys[1].id)~='string' then error('Product key id is not a string',0) end
    local id='RBP-'..keys[1].id
    local response=fetch('__ws_roblox_details',player.UserId,id)
    local details=response.details
    local state=details and details.subscriptionState
    if not state or not Enum.SubscriptionState[state] then error('Details not found for subscription: '..id,0) end
    local out={IsSubscribed=state~='NeverSubscribed' and state~='Expired',IsOriginExperience=false}
    local product=response.subscriptionProductTypeStateDetails
    local roblox=product and product.robloxSubscriptionStateDetails
    local timestamp=roblox and roblox.startTimestampMs
    if type(timestamp)=='number' and timestamp>0 then out.StartTime=DateTime.fromUnixTimestampMillis(timestamp);out.IsOriginExperience=true end
    return out
end
M.GetUserSubscriptionDetailsAsync=function(self,player,id)
    subscription_id(id)
    if not game:GetService('RunService'):IsServer() then error('Can only call GetUserSubscriptionDetailsAsync from Script with RunContext = Server',0) end
    local override=__ws_subscription_override and __ws_subscription_override('details',player.UserId,id)
    if override then task.wait();return override end
    local v=fetch('GetUserSubscriptionDetailsAsync',player.UserId,id).details
    local out={SubscriptionState=Enum.SubscriptionState[v.subscriptionState]}
    if v.nextRenewTimestampMs and v.nextRenewTimestampMs>0 then out.NextRenewTime=DateTime.fromUnixTimestampMillis(v.nextRenewTimestampMs) end
    if v.expireTimestampMs and v.expireTimestampMs>0 then out.ExpireTime=DateTime.fromUnixTimestampMillis(v.expireTimestampMs) end
    if v.expirationDetails then out.ExpirationDetails={Reason=Enum.SubscriptionExpirationReason[v.expirationDetails.reason]} end
    return out
end
M.GetUserSubscriptionPaymentHistoryAsync=function(self,player,id)
    subscription_id(id)
    if not game:GetService('RunService'):IsServer() then error('Can only call GetUserSubscriptionPaymentHistoryAsync from Script with RunContext = Server',0) end
    local now=DateTime.now().UnixTimestampMillis;local rows={}
    -- Native Studio mock provider . Every other valid EXP ID
    -- warns and returns an empty array; it does not make a live request.
    if id=='EXP-0' then rows={{now-2419200000,now+172800000,'Paid'},{now-5011200000,now-2419200000,'Paid'},{now-7603200000,now-5011200000,'Paid'}}
    elseif id=='EXP-1' then rows={{1680652800000,1683244800000,'Paid'},{1674086400000,1676764800000,'Paid'}}
    elseif id=='EXP-2' then rows={{1677801600000,1678838400000,'Refunded'},{1675382400000,1677801600000,'Paid'},{1672704000000,1675382400000,'Paid'}}
    elseif id=='EXP-3' then rows={{now-5356800000,now-2764800000,'Paid'},{now-7948800000,now-5356800000,'Paid'}}
    else warn('GetUserSubscriptionPaymentHistoryAsync only returns valid response for a limited set of mocked Subscription IDs when invoked in studio mode. For more information on available mocks, see https://create.roblox.com/docs/reference/engine/classes/MarketplaceService#GetUserSubscriptionPaymentHistoryAsync') end
    local out={};for _,r in rows do table.insert(out,{CycleStartTime=DateTime.fromUnixTimestampMillis(r[1]),CycleEndTime=DateTime.fromUnixTimestampMillis(r[2]),PaymentStatus=Enum.SubscriptionPaymentStatus[r[3]]})end;return out
end
M.GetBadgeInfoAsync=function(self,id)
    id=tonumber(id)
    if id==nil then error('Argument 1 missing or nil',2) end
    id=math.floor(id)
    if id<=0 then error('Invalid badgeId',0) end
    local v=fetch('GetBadgeInfoAsync',id)
    return {Name=v.name,Description=v.description,IconImageId=v.iconImageId,IsEnabled=v.enabled,AwardCount=v.statistics and v.statistics.awardedCount,PastDayAwardCount=v.statistics and v.statistics.pastDayAwardedCount,Rarity=v.statistics and v.statistics.winRatePercentage}
end
M.UserHasBadgeAsync=function(self,user,id)
    if game:GetService('RunService'):IsClient() and user~=game:GetService('Players').LocalPlayer.UserId then error('UserHasBadgeAsync: can only query local player',0) end
    local numeric_user,numeric_id=tonumber(user),tonumber(id)
    if numeric_user and numeric_id and (numeric_user<=0 or numeric_id<=0) then return false end
    return fetch('UserHasBadgeAsync',user,id).awardedDate~=nil
end
M.UserHasBadge=M.UserHasBadgeAsync
M.PlayerHasPass=function(self,player,id) return M.UserOwnsGamePassAsync(game:GetService('MarketplaceService'),player.UserId,id) end
M.GetUserBadgesAsync=function(self,user,ids)
    if game:GetService('RunService'):IsClient() and user~=game:GetService('Players').LocalPlayer.UserId then error('GetUserBadgesAsync: Can only query local player from client.',0) end
    if tonumber(user) and tonumber(user)<=0 then error('GetUserBadgesAsync: User is invalid or does not exist.',0) end
    if #ids==0 then return {} end
    if #ids>100 then error('GetUserBadgesAsync: Cannot query more than 100 badges.',0) end
    for _,id in ids do if type(id)~='number' then error('GetUserBadgesAsync: badgeIds must be numeric.',0) end end
    local out={}
    for _,v in fetch('GetUserBadgesAsync',user,ids).data or {} do
        if type(v.badgeId)=='number' and type(v.awardedDate)=='string' then local date=DateTime.fromIsoDate(v.awardedDate);if date then table.insert(out,{BadgeId=v.badgeId,AwardedDate=date})end end
    end
    return out
end
M.CheckUserBadgesAsync=function(self,user,ids)
    if #ids>10 then error('CheckUserBadgesAsync: Cannot query more than 10 badges.',0) end
    if game:GetService('RunService'):IsClient() and user~=game:GetService('Players').LocalPlayer.UserId then error('CheckUserBadgesAsync: Can only query local player from client.',0) end
    if #ids==0 then return {} end
    for _,id in ids do if type(id)~='number' then error('CheckUserBadgesAsync: badgeIds must be numeric.',0) end end
    local out={}
    for _,v in fetch('CheckUserBadgesAsync',user,ids).data do table.insert(out,v.badgeId) end
    return out
end
M.GetGroupInfoAsync=function(self,id)
    id=tonumber(id)
    if id==nil then error('Argument 1 missing or nil',2) end
    id=math.floor(id)
    if id<=0 then error('GroupService:GetGroupInfoAsync() argument is not a valid groupId',0) end
    return fetch('GetGroupInfoAsync',id)
end
M.GetGroupsAsync=function(self,id) return fetch('GetGroupsAsync',id) end
M.GetRolesInGroupAsync=function(self,user,group)
    user=tonumber(user);group=tonumber(group)
    if not tonumber(user) or not tonumber(group) or user<=0 or group<=0 then error('GroupService:GetRolesInGroupAsync() requires positive userId and groupId',0) end
    local response=fetch('GetRolesInGroupAsync',user,group);local roles={}
    if response.data and response.data[1] then
        for _,r in response.data[1].roles do table.insert(roles,{Id=r.id,Name=r.name,Rank=r.rank}) end
    end
    return {Roles=roles,IsMember=#roles>0}
end
M.GetUserInfosByUserIdsAsync=function(self,ids)
    for _,id in ids do if type(id)~='number' then error('UserService: invalid userId(s) given. Please make sure they are of number type',0) end end
    local out={}
    for _,v in fetch('GetUserInfosByUserIdsAsync',ids).data do
        table.insert(out,{Id=v.id,Username=v.name,DisplayName=v.displayName,HasVerifiedBadge=v.hasVerifiedBadge==true})
    end
    return out
end
"#).set_name("web_social_http").exec()
}
