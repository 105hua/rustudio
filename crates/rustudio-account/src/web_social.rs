//! Login-session engine requests. Read is a strict metadata/lookup allowlist. Native URL
//! builders: Group , Badge , Inventory ,
//! Users . Notification POST actions are isolated from Read and are
//! tested against mock HTTP only. Read never selects a purchase, award, invitation
//! or teleport route; action transports are separate from the read allowlist.
use crate::{Error, Result, Session};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, LazyLock, Mutex, atomic::{AtomicU64, Ordering}},
    time::{Duration, Instant},
};
type CacheKey = (String, String);
type Cache = HashMap<CacheKey, (Instant, Duration, Value)>;
static PRODUCT_CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

// Cached native requests retain a pending result as well as a completed one.
// Keep failures only for callers already waiting on that request; a later call
// retries. Cookie and VM scope are both in the key, just as for the value cache.
#[derive(Default)]
struct Flight {
    result: Mutex<Option<std::result::Result<Value, String>>>,
    ready: Condvar,
}
static FLIGHTS: LazyLock<Mutex<HashMap<CacheKey, Arc<Flight>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
struct FlightLeader { key: Option<CacheKey>, flight: Arc<Flight> }
impl FlightLeader {
    fn publish(&mut self, result: std::result::Result<Value, String>) {
        if let Ok(mut value)=self.flight.result.lock() { *value=Some(result); }
        if let Some(key)=self.key.take() {
            if let Ok(mut flights)=FLIGHTS.lock() { flights.remove(&key); }
        }
        self.flight.ready.notify_all();
    }
}
impl Drop for FlightLeader {
    fn drop(&mut self) {
        if self.key.is_some() { self.publish(Err("Request interrupted".into())); }
    }
}
fn shared_request(key: CacheKey, request: impl FnOnce()->Result<Value>) -> Result<Value> {
    let (flight, leader)={
        let mut flights=FLIGHTS.lock().map_err(|_|Error::new("Request cache unavailable"))?;
        if let Some(flight)=flights.get(&key) { (flight.clone(), false) }
        else { let flight=Arc::new(Flight::default()); flights.insert(key.clone(),flight.clone()); (flight,true) }
    };
    if !leader {
        let mut result=flight.result.lock().map_err(|_|Error::new("Request cache unavailable"))?;
        while result.is_none() {
            result=flight.ready.wait(result).map_err(|_|Error::new("Request cache unavailable"))?;
        }
        return result.as_ref().unwrap().clone().map_err(Error::new);
    }
    let mut leader=FlightLeader { key:Some(key), flight };
    let result=request();
    leader.publish(result.as_ref().cloned().map_err(|e|e.0.clone()));
    result
}

/// A service-VM cache lifetime. Server/client VMs and fresh Play tests do not
/// share ownership or metadata cache entries merely because their cookie matches.
#[derive(Clone)]
pub struct CacheScope(Arc<ScopeInner>);
struct ScopeInner(u64);
impl Default for CacheScope {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(Arc::new(ScopeInner(NEXT.fetch_add(1, Ordering::Relaxed))))
    }
}
impl Drop for ScopeInner {
    fn drop(&mut self) {
        let prefix = format!("scope:{}\n", self.0);
        if let Ok(mut cache) = PRODUCT_CACHE.lock() {
            cache.retain(|(_, key), _| !key.starts_with(&prefix));
        }
        if let Ok(mut batches)=BATCHES.lock() { batches.retain(|(_,key),_|!key.starts_with(&prefix)); }
    }
}

#[derive(Clone, Debug)]
pub enum Read {
    Product { id: u64, kind: String },
    Owns { user: u64, id: u64, bundle: bool },
    PassOwnership { user: u64, pass: u64 },
    Badge(u64),
    HasBadge { user: u64, badge: u64 },
    CheckBadges { user: u64, badges: Vec<i64> },
    Group(u64),
    /// CoreScript metadata query; POST is read-only profile retrieval.
    GroupProfile {group:u64,actions_only:bool,deactivated:bool},
    Groups(i64),
    GroupRoles { user: u64, group: u64 },
    Relationships { group: u64, allies: bool, page: u64 },
    Users(Vec<i64>),
    Policy,
    CanPromptOptIn(u64),
    ExperienceDetails(u64),
    /// InExperienceDetailsPrompt DataHydration reads. IDs remain signed like
    /// the native int64 argument; zero/negative IDs are not rejected by the UI.
    ExperienceInfo(i64),
    ExperiencePlayability(i64),
    ExperienceVotes(i64),
    ExperienceAgeRecommendation(i64),
    /// Native PlayerFeedback eligibility GETs; opening/submitting is separate.
    FeedbackEligibility(u64),
    PlayerSupportEligibility(u64),
    /// CoreScript InExperienceShop.Api.fetchShopItemIds, no purchase action.
    ShopWidgets { universe: u64, request: String },
    CommerceProduct(String),
    CampaignEligibility(String),
    AdPlacements(u64),
    InvitePermission {place:i64,trigger:String,recipient:Option<String>},
    RecommendationItem(String),
    RecommendationList(Value),
    RankProducts(Value),
    TopProducts(Value),
    PlayerPlace { place: u64, user: u64 },
    Party { id: String, universe: u64, place: u64, job: String },
    BrandDecision(String),
    PriceLevels { universe: u64, users: Vec<u64> },
    SubscriptionDetails { user: u64, subscription: String },
    RobloxSubscriptionProducts(u64),
    /// Client CoreScript direct fetch (distinct from the RCC engine route).
    RobloxSubscriptionPromptProducts,
    UpcomingEvents(u64),
    ExperienceEvent(String),
    SubscriptionProduct(String),
    SubscriptionStatus { user: u64, subscription: String },
    DeveloperProducts { universe: u64, cursor: String },
}
/// Requests made by the native notification dialog. These are intentionally
/// separate from `Read`: opening a published prompt and confirming a follow
/// are POST actions. Automated verification uses localhost mocks only.
#[derive(Clone, Debug)]
pub enum NotificationAction {
    Prompt { universe: u64 },
    Follow { user: u64, universe: u64 },
}
impl NotificationAction {
    fn request(&self, session: &Session) -> Result<Request> {
        Ok(match self {
            Self::Prompt { universe } => Request { host:"apis",path:"/notifications/v1/prompt-experience-notification-opt-in".into(),body:Some(json!({"universeId":universe})) },
            Self::Follow { user, universe } => {
                if *user!=session.profile.id { return Err(Error::new("The local player does not match the login session")); }
                Request { host:"followings",path:format!("/v1/users/{user}/universes/{universe}"),body:Some(json!([])) }
            },
        })
    }
    pub fn execute(&self, session: &Session) -> Result<Value> {
        self.execute_at(session,|host|format!("https://{host}.roblox.com"))
    }
    fn execute_at(&self, session: &Session, origin: impl Fn(&str)->String) -> Result<Value> {
        let request=self.request(session)?;
        let url=format!("{}{}",origin(request.host),request.path);
        let (status,body)=session.post_csrf(&url,"application/json",&serde_json::to_vec(&request.body.unwrap())?)?;
        if !(200..300).contains(&status) {
            let reason=ureq::http::StatusCode::from_u16(status).ok().and_then(|s|s.canonical_reason()).unwrap_or("Error");
            return Err(Error::new(format!("HTTP {status} ({reason})")));
        }
        if matches!(self,Self::Follow {..}) { return Ok(Value::Null); }
        if body.is_empty() { return Err(Error::new("Empty response received from Notifications API")); }
        let value:Value=serde_json::from_str(&body).map_err(|_|Error::new("Failed to parse Notifications API response"))?;
        if !value.is_object() { return Err(Error::new("Variant cast failed")); }
        Ok(value)
    }
}

/// CoreScript send operation, exercised only against localhost during parity
/// work. Opening/cancelling a prompt never invokes this operation.
/// Recommendation writes have fixed engine routes; development tests are
/// localhost-only. Never invoke these against the signed-in live experience.
#[derive(Clone, Copy, Debug)]
pub enum RecommendationMutation { Register, Update, Remove { client: bool } }
impl RecommendationMutation {
    fn path(self)->&'static str { match self {
        Self::Register=>"/recommendation-api/v1/rcc/recommendation-item/batch",
        Self::Update=>"/recommendation-api/v1/rcc/recommendation-item/update",
        Self::Remove {client:true}=>"/recommendation-api/v1/recommendation-item/delete",
        Self::Remove {client:false}=>"/recommendation-api/v1/rcc/recommendation-item/delete",
    }}
    fn failure(self,detail:&str)->String { match self {
        Self::Register=>format!("Failed to register recommendation items - {detail}"),
        Self::Update=>format!("Failed to update recommendation items - {detail}"),
        Self::Remove {..}=>format!("Failed to remove item - {detail}"),
    }}
    pub fn execute(self,session:&Session,body:Value)->Result<Value> {self.execute_at(session,body,"https://apis.roblox.com")}
    fn execute_at(self,session:&Session,body:Value,origin:&str)->Result<Value> {
        let reply=session.post_csrf(&format!("{origin}{}",self.path()),"application/json",&serde_json::to_vec(&body)?);
        let (status,body)=reply.map_err(|e|Error::new(self.failure(&e.to_string())))?;
        if !(200..300).contains(&status) {return Err(Error::new(self.failure(&format!("HTTP {status} ({})",ureq::http::StatusCode::from_u16(status).ok().and_then(|s|s.canonical_reason()).unwrap_or("Error")))));}
        if matches!(self,Self::Register) {
            let value:Value=serde_json::from_str(&body).map_err(|_|Error::new("Failed to register recommendation item."))?;
            let results=value["results"].as_array().ok_or_else(||Error::new("Failed to register recommendation items."))?;
            if let Some(item)=results.first() {
                let id=item["itemId"].as_str().ok_or_else(||Error::new("Failed to register recommendation items."))?;
                let reference=item["referenceId"].as_str().ok_or_else(||Error::new("Failed to register recommendation items."))?;
                return Ok(json!({"ItemId":id,"ReferenceId":reference}));
            }
            return Ok(json!({"ItemId":"","ReferenceId":""}));
        }
        Ok(json!(null))
    }
}

/// Explicit RSVP confirmation action from VirtualEvents.Common.postRsvpStatus.
/// Development invokes the POST only against localhost fixtures.
pub fn update_event_rsvp(session:&Session,event:&str,going:bool)->Result<Value> {update_event_rsvp_at(session,event,going,"https://apis.roblox.com")}
fn update_event_rsvp_at(session:&Session,event:&str,going:bool,origin:&str)->Result<Value> {
    send_read(session,origin,&Request{host:"apis",path:format!("/virtual-events/v1/virtual-events/{}/rsvps",component(event)),body:Some(json!({"rsvpStatus":if going{"going"}else{"notGoing"}}))})
}

/// Native internal moderation action. Never included in Read; only localhost
/// fixtures submit review requests during development.
pub fn request_content_review(session:&Session,universe:u64,id:&str,key:&str)->Result<Value> {
    request_content_review_at(session,universe,id,key,"https://apis.roblox.com")
}
fn request_content_review_at(session:&Session,universe:u64,id:&str,key:&str,origin:&str)->Result<Value> {
    let url=format!("{origin}/user/cloud/v2/universes/{universe}/reviewable-content/{}:requestReview?idempotencyKey.key={}",component(id),component(key));
    let(status,body)=session.post_csrf(&url,"application/json",b"{}").map_err(|e|Error::new(format!("HTTP failure: {e}")))?;
    if status==200 {return Ok(Value::Null);}
    if status==404 {return Err(Error::new(format!("ReviewableContent with Id \"{id}\" does not exist")));}
    let message=match serde_json::from_str::<Value>(&body) {
        Ok(value)=>value["message"].as_str().unwrap_or("<no returned error string>").to_owned(),
        Err(_)=>"<unparseable response body>".to_owned(),
    };
    Err(Error::new(format!("HTTP status {status} not OK: {message}")))
}

/// Explicit user activation only; development exercises this transport only on localhost.
pub fn join_community(session:&Session,group:u64)->Result<Value> {join_community_at(session,group,"https://groups.roblox.com")}
fn join_community_at(session:&Session,group:u64,origin:&str)->Result<Value> {
    let request=Request{host:"groups",path:format!("/v1/groups/{group}/users"),body:Some(json!([]))};
    send_read(session,origin,&request)
}

/// Private identity resolution uses Studio's fixed get-or-create route. This is
/// deliberately outside Read: development tests call only the localhost fixture.
pub fn resolve_private_user(session:&Session,global:u64,universe:u64)->Result<Value> {
    resolve_private_user_at(session,global,universe,"https://apis.roblox.com")
}
fn resolve_private_user_at(session:&Session,global:u64,universe:u64,origin:&str)->Result<Value> {
    let agent=crate::session::session_agent();
    let url=format!("{origin}/private-users-service/v1beta1/getOrCreatePrivateUserId");
    let body=serde_json::to_vec(&json!({"global_user_id":global,"domain_id":universe,"domain_type":1}))?;
    let mut token=None::<String>;
    let started=Instant::now();
    for _ in 0..=2 {
        let timeout=Duration::from_secs(5).saturating_sub(started.elapsed());
        let mut request=agent.post(&url).header("Cookie",&session.cookie().header()).header("Content-Type","application/json").config().timeout_global(Some(timeout)).build();
        if let Some(t)=&token {request=request.header("X-CSRF-TOKEN",t);}
        let mut reply=request.send(body.as_slice()).map_err(|e|Error::new(format!("UserService: Request failed because {e}")))?;
        let status=reply.status().as_u16();
        let fresh=reply.headers().get("x-csrf-token").and_then(|v|v.to_str().ok()).map(str::to_owned);
        let text=reply.body_mut().read_to_string().map_err(|e|Error::new(format!("UserService: Request failed because {e}")))?;
        if status==403 && fresh.as_ref().is_some_and(|t|!t.is_empty()&&Some(t)!=token.as_ref()) {token=fresh;continue;}
        let value=serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            let message=value["message"].as_str().or_else(||value.pointer("/errors/0/message").and_then(Value::as_str)).map(str::to_owned).unwrap_or_else(||format!("HTTP {status} ({})",ureq::http::StatusCode::from_u16(status).ok().and_then(|s|s.canonical_reason()).unwrap_or("Error")));
            return Err(Error::new(format!("UserService: Request failed because {message}")));
        }
        let private=value["privateUserId"].as_u64().filter(|id|*id>0).ok_or_else(||Error::new("UserService: User not found"))?;
        // Preserve all 64 bits across the worker JSON/Luau boundary.
        return Ok(json!({"privateUserId":private.to_string()}));
    }
    Err(Error::new("UserService: Request failed because Roblox kept refusing the CSRF token"))
}

/// Engine link preparation only. Does not open a share sheet or send a link.
pub fn create_expiring_share_link(session:&Session,body:Value)->Result<Value> {
    create_expiring_share_link_at(session,body,"https://apis.roblox.com")
}
fn create_expiring_share_link_at(session:&Session,body:Value,origin:&str)->Result<Value> {
    let (status,body)=session.post_csrf(&format!("{origin}/deeplinks/v2/rcc/expiring"),"application/json",&serde_json::to_vec(&body)?)?;
    if !(200..300).contains(&status) {
        let reason=ureq::http::StatusCode::from_u16(status).ok().and_then(|s|s.canonical_reason()).unwrap_or("Error");
        return Err(Error::new(format!("HTTP {status} ({reason})")));
    }
    let failure=||Error::new("SocialService:PromptLinkSharing failed to create a share link due to network error");
    let value:Value=serde_json::from_str(&body).map_err(|_|failure())?;
    let link=value["link"].as_str().filter(|s|!s.is_empty()).ok_or_else(failure)?;
    Ok(json!({"link":link}))
}

pub fn send_experience_invite(session: &Session, request: Value) -> Result<Value> {
    send_experience_invite_at(session,request,"https://apis.roblox.com")
}
/// Native feedback submission. Development invokes only localhost;
/// opening/closing the prompt uses Read::FeedbackEligibility and never this POST.
pub fn submit_player_feedback(session:&Session,asset:u64,comment:&str)->Result<Value> {
    submit_player_feedback_at(session,asset,comment,"https://apis.roblox.com")
}
fn submit_player_feedback_at(session:&Session,asset:u64,comment:&str,origin:&str)->Result<Value> {
    let url=format!("{origin}/player-generated-reviews-service/v1/channels/experience-discovery-page/assets/{asset}/reviews");
    let request=json!({"category_type":"InExperience","category_option_ids":[],"comment":comment});
    let (status,_)=session.post_csrf(&url,"application/json",&serde_json::to_vec(&request)?)?;
    if !(200..300).contains(&status) {return Err(Error::new(format!("HTTP {status}")));}
    Ok(Value::Null)
}
fn send_experience_invite_at(session: &Session, request: Value, origin: &str) -> Result<Value> {
    let url=format!("{origin}/notifications/v1/send-experience-invite");
    let (status,body)=session.post_csrf(&url,"application/json",&serde_json::to_vec(&request)?)?;
    if !(200..300).contains(&status) {
        let reason=ureq::http::StatusCode::from_u16(status).ok().and_then(|s|s.canonical_reason()).unwrap_or("Error");
        return Err(Error::new(format!("HTTP {status} ({reason})")));
    }
    serde_json::from_str(&body).map_err(|_|Error::new("Unable to convert response to JSON"))
}

/// Studio bulk-prompt preparation (metadata/order quote, never purchase).
/// Native URL builder, request/parser.
/// Development POST verification is localhost only.
pub fn prepare_studio_bulk(session: &Session, request: Value) -> Result<Value> {
    prepare_studio_bulk_at(session,request,"https://apis.roblox.com")
}
fn prepare_studio_bulk_at(session: &Session, request: Value, origin: &str) -> Result<Value> {
    let url=format!("{origin}/marketplace-ua/game-engine/v1/prepare-studio-bulk-purchase-prompt");
    let (status,body)=session.post_csrf(&url,"application/json",&serde_json::to_vec(&request)?)?;
    if !(200..300).contains(&status) {
        let reason=ureq::http::StatusCode::from_u16(status).ok().and_then(|s|s.canonical_reason()).unwrap_or("Error");
        return Err(Error::new(format!("HTTP {status} ({reason})")));
    }
    let parse_error=||Error::new("an error occurred while parsing the prepare bulk purchase response");
    let value:Value=serde_json::from_str(&body).map_err(|_|parse_error())?;
    if !value.is_object() || !value["purchaserRobuxBalance"].is_number() || !value["orderTotalRobux"].is_number() {return Err(parse_error());}
    Ok(value)
}

/// Native order transport, restricted to the Studio-only authorization
/// generated by. Never accept a production authorization token here.
/// All development fulfillment calls are localhost-only mock requests.
pub fn fulfill_studio_bulk(session:&Session,order:Value,key:&str)->Result<Value> {
    fulfill_studio_bulk_at(session,order,key,"https://apis.roblox.com")
}
fn fulfill_studio_bulk_at(session:&Session,order:Value,key:&str,origin:&str)->Result<Value> {
    let token=order["batchPurchaseAuthToken"].as_str().unwrap_or_default();
    if !token.strip_prefix("studio-bulk-purchase-").is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|b|b.is_ascii_hexdigit()||b==b'-')) {
        return Err(Error::new("Studio bulk purchase requires a Studio test authorization token"));
    }
    let url=format!("{origin}/cloud/v2/avatar-marketplace-orders?idempotencyKey.key={}",component(key));
    let(status,body)=session.post_csrf(&url,"application/json",&serde_json::to_vec(&order)?)?;
    if !(200..300).contains(&status) {
        let reason=ureq::http::StatusCode::from_u16(status).ok().and_then(|s|s.canonical_reason()).unwrap_or("Error");
        return Err(Error::new(format!("HTTP {status} ({reason})")));
    }
    serde_json::from_str(&body).map_err(|_|Error::new("Unable to convert response to JSON"))
}

/// Engine reservation operation. This allocates a reserved server, so our
/// development probes exercise it only against localhost mocks, never live.
/// Native URL ; response parser .
pub fn reserve_server(session: &Session, place: i64) -> Result<Value> {
    reserve_server_at(session,place,"https://apis.roblox.com")
}
fn reserve_server_at(session: &Session, place: i64, origin: &str) -> Result<Value> {
    let url=format!("{origin}/reserved-servers-service/create?placeId={place}");
    let (status,body)=session.post_csrf(&url,"application/x-www-form-urlencoded",b"")?;
    if !(200..300).contains(&status) {
        let reason=ureq::http::StatusCode::from_u16(status).ok().and_then(|s|s.canonical_reason()).unwrap_or("Error");
        return Err(Error::new(format!("HTTP {status} ({reason})")));
    }
    if body.is_empty() {return Err(Error::new("ReserveServer error: Response is empty"));}
    let value:Value=serde_json::from_str(&body).map_err(|_|Error::new("ReserveServer error: Could not parse JSON result"))?;
    if !value.is_object() {return Err(Error::new("ReserveServer error: Could not parse JSON result"));}
    for name in ["ReservedServerAccessCode","ReservedServerGameCode"] {
        let item=value.get(name).ok_or_else(||Error::new(format!("ReserveServer error: No {name} returned")))?;
        if !item.is_string() {return Err(Error::new(format!("ReserveServer error: {name} is not of type string")));}
    }
    Ok(value)
}

/// Subscription sheet initialization, never a purchase. The CoreScript always
/// creates a payment-session handle while fetching products, even in Studio;
/// failure to create that handle is caught and does not fail the sheet.
/// Development tests invoke this only against localhost, never the live POST.
pub fn subscription_prompt_products(session: &Session, scope: &CacheScope) -> Result<Value> {
    subscription_prompt_at(session,scope,|host|format!("https://{host}.roblox.com"))
}
fn subscription_prompt_at(session: &Session, scope: &CacheScope, origin: impl Fn(&str)->String + Sync) -> Result<Value> {
    std::thread::scope(|threads| {
        let products=threads.spawn(|| Read::RobloxSubscriptionPromptProducts.fetch_at_scoped(session,scope.0.0,&origin));
        let url=format!("{}/payments-gateway/v1/payment-sessions",origin("apis"));
        // Network.createPaymentSession catches every failure and resolves "".
        let _=session.post_csrf(&url,"application/json",br#"{"entryPoint":"PromptSubscriptionPurchase"}"#);
        products.join().map_err(|_|Error::new("Subscription product request interrupted"))?
    })
}

#[derive(Clone, Debug)]
pub struct Request {
    pub host: &'static str,
    pub path: String,
    pub body: Option<Value>,
}
fn component(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BatchKind { Product, GamePass, Ownership }
impl BatchKind {
    fn of(read: &Read) -> Option<Self> {
        match read {
            Read::Product { kind, .. } if kind=="Product" => Some(Self::Product),
            Read::Product { kind, .. } if kind=="GamePass" => Some(Self::GamePass),
            Read::PassOwnership { .. } => Some(Self::Ownership),
            _=>None,
        }
    }
    fn request(self, reads: &[Read]) -> Request {
        let (path, body)=match self {
            Self::Product => {
                let ids=reads.iter().filter_map(|r|if let Read::Product {id,..}=r {Some(id.to_string())}else{None}).collect::<Vec<_>>().join(",");
                (format!("/developer-products/v1/developer-products/batch?productIds={ids}"),None)
            },
            Self::GamePass => {
                let ids:Vec<_>=reads.iter().filter_map(|r|if let Read::Product {id,..}=r {Some(*id)}else{None}).collect();
                ("/game-passes/v1/game-passes:batchGet".into(),Some(json!({"gamePassIds":ids,"passView":"Price"})))
            },
            Self::Ownership => {
                let ids:Vec<_>=reads.iter().filter_map(|r|if let Read::PassOwnership {user,pass}=r {Some(json!({"userId":user,"gamePassId":pass}))}else{None}).collect();
                ("/game-passes/v1/game-passes:batchGetOwnership".into(),Some(json!({"ownershipIdentifiers":ids})))
            },
        };
        Request {host:"apis",path,body}
    }
}
struct BatchEntry { read: Read, at: Instant, result: Arc<Flight> }
#[derive(Default)]
struct BatchQueue { pending: std::collections::VecDeque<BatchEntry>, busy: bool, last: Option<Instant>, inflight: usize }
#[derive(Default)]
struct Batcher { queue: Mutex<BatchQueue>, ready: Condvar }
static BATCHES: LazyLock<Mutex<HashMap<CacheKey,Arc<Batcher>>>>=LazyLock::new(||Mutex::new(HashMap::new()));
fn send_read(session: &Session, origin: &str, request: &Request) -> Result<Value> {
    let url=format!("{origin}{}",request.path);
    let (status,body)=if let Some(body)=&request.body {
        session.post_csrf(&url,"application/json",&serde_json::to_vec(body)?)?
    }else{session.get(&url)?};
    if !(200..300).contains(&status) {
        let reason=ureq::http::StatusCode::from_u16(status).ok().and_then(|s|s.canonical_reason()).unwrap_or("Error");
        return Err(Error::new(format!("HTTP {status} ({reason})")));
    }
    if request.path.starts_with("/notifications/v1/can-prompt-experience-notification-opt-in?") {
        if body.is_empty() { return Err(Error::new("Empty response received from Notification API")); }
        return serde_json::from_str(&body).map_err(|_|Error::new("Failed to parse Notifications API response"));
    }
    serde_json::from_str(&body).map_err(|_|Error::new("Unable to convert response to JSON"))
}
fn batched_read(session: &Session, scope: u64, origin: &str, kind: BatchKind, read: Read) -> Result<Value> {
    let key=(session.cookie().cache_key(),format!("scope:{scope}\n{origin}\n{kind:?}"));
    let batcher={
        let mut batches=BATCHES.lock().map_err(|_|Error::new("Request cache unavailable"))?;
        batches.entry(key).or_insert_with(||Arc::new(Batcher::default())).clone()
    };
    let result=Arc::new(Flight::default());
    let mut queue=batcher.queue.lock().map_err(|_|Error::new("Request cache unavailable"))?;
    if queue.inflight>=5000 { return Err(Error::new("Maximum inflight requests reached.")); }
    queue.inflight+=1;
    queue.pending.push_back(BatchEntry {read,at:Instant::now(),result:result.clone()});
    batcher.ready.notify_all();
    loop {
        if let Some(value)=result.result.lock().map_err(|_|Error::new("Request cache unavailable"))?.as_ref() {
            return value.clone().map_err(Error::new);
        }
        let first=queue.pending.front().is_some_and(|entry|Arc::ptr_eq(&entry.result,&result));
        if !queue.busy && first {
            queue.busy=true;
            // Native scheduler : after >=1s idle use 40ms;
            // during activity use 200ms, measured from the first queued call.
            let window=if queue.last.is_none_or(|at|at.elapsed()>=Duration::from_secs(1)) {40}else{200};
            let deadline=queue.pending.front().unwrap().at+Duration::from_millis(window);
            while let Some(remaining)=deadline.checked_duration_since(Instant::now()) {
                queue=batcher.ready.wait_timeout(queue,remaining).map_err(|_|Error::new("Request cache unavailable"))?.0;
            }
            let count=queue.pending.len().min(50);
            let entries:Vec<_>=queue.pending.drain(..count).collect();
            queue.last=Some(Instant::now());
            drop(queue);
            let reads:Vec<_>=entries.iter().map(|entry|entry.read.clone()).collect();
            let response=send_read(session,origin,&kind.request(&reads));
            for entry in &entries {
                *entry.result.result.lock().map_err(|_|Error::new("Request cache unavailable"))?=Some(response.as_ref().cloned().map_err(|e|e.0.clone()));
            }
            queue=batcher.queue.lock().map_err(|_|Error::new("Request cache unavailable"))?;
            queue.inflight-=entries.len();
            queue.busy=false;
            batcher.ready.notify_all();
        }else{
            queue=batcher.ready.wait(queue).map_err(|_|Error::new("Request cache unavailable"))?;
        }
    }
}

impl Read {
    pub fn request(&self) -> Result<Request> {
        let (host, path, body) = match self {
            Self::Product { id, kind } => match kind.as_str() {
                "Asset" => ("economy", format!("/v2/assets/{id}/details"), None),
                "GamePass" => (
                    "apis",
                    "/game-passes/v1/game-passes:batchGet".into(),
                    Some(json!({"gamePassIds":[id],"passView":"Price"})),
                ),
                "Bundle" => ("catalog", format!("/v1/bundles/{id}/details"), None),
                "Product" => (
                    "apis",
                    format!("/developer-products/v1/developer-products/batch?productIds={id}"),
                    None,
                ),
                _ => return Err(Error::new("Unsupported product information type")),
            },
            Self::Owns { user, id, bundle } => (
                "inventory",
                format!(
                    "/v1/users/{user}/items/{}/{id}/is-owned",
                    if *bundle { "Bundle" } else { "asset" }
                ),
                None,
            ),
            Self::PassOwnership { user, pass } => (
                "apis",
                "/game-passes/v1/game-passes:batchGetOwnership".into(),
                Some(json!({"ownershipIdentifiers":[{"userId":user,"gamePassId":pass}]})),
            ),
            Self::Badge(id) => ("badges", format!("/v1/badges/{id}"), None),
            Self::HasBadge { user, badge } => (
                "badges",
                format!("/v1/users/{user}/badges/{badge}/awarded-date"),
                None,
            ),
            Self::CheckBadges { user, badges } => (
                "badges",
                format!(
                    "/v1/users/{user}/badges/awarded-dates?badgeIds={}",
                    badges
                        .iter()
                        .map(i64::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                None,
            ),
            Self::Group(id) => ("groups", format!("/v0/groups/{id}"), None),
            Self::FeedbackEligibility(asset) => ("apis",format!("/player-generated-reviews-service/v1/channels/experience-discovery-page/assets/{asset}"),None),
            Self::PlayerSupportEligibility(universe) => ("apis",format!("/creator-communication/v1beta1/creator-communication-api/universes/{universe}/ticket-creation-eligibility"),None),
            Self::ShopWidgets {universe,request} => ("apis",format!("/marketplace-widgets/v1/widgets/shop-widgets?requestId={}&universeId={universe}",component(request)),None),
            Self::GroupProfile {group,actions_only,deactivated} => {
                let names=if *actions_only {vec!["Actions"]}else if *deactivated {vec!["CommunityProfileHeader","Actions","CommunityLocked","CommunityDeactivated"]}else{vec!["CommunityProfileHeader","Actions","CommunityLocked"]};
                ("apis","/profile-platform-api/v1/profiles/get".into(),Some(json!({"profileId":group,"profileType":"Community","components":names.into_iter().map(|component|json!({"component":component})).collect::<Vec<_>>()})))
            },
            Self::GroupRoles { user, group } => (
                "groups",
                format!("/v2/groups/{group}/users?userIds={user}"),
                None,
            ),
            Self::Groups(user) => ("groups", format!("/v0/users/{user}/groups"), None),
            Self::Relationships {
                group,
                allies,
                page,
            } => (
                "groups",
                format!(
                    "/v0/groups/{group}/relationships?relationshipType={}&page={page}",
                    if *allies { "Ally" } else { "Enemy" }
                ),
                None,
            ),
            Self::RankProducts(body) => ("apis","/product-recommender/v1/virtual-products:rank".into(),Some(body.clone())),
            Self::TopProducts(body) => ("apis","/product-recommender/v1/virtual-products:recommend".into(),Some(body.clone())),
            Self::RecommendationList(body) => ("apis","/recommendation-api/v1/recommendation/generate".into(),Some(body.clone())),
            Self::RecommendationItem(id) => (
                "apis",
                "/recommendation-api/v1/recommendation-item/get".into(),
                Some(json!({"itemId":id})),
            ),
            Self::PlayerPlace { place, user } => (
                "apis",
                format!(
                    "/reserved-servers-service/get-player-place-instance?currentPlaceId={place}&userId={user}"
                ),
                None,
            ),
            Self::BrandDecision(id) => (
                "apis",
                format!(
                    "/brand-platform/v1/brand-projects/{}/decision/studio",
                    component(id)
                ),
                None,
            ),
            Self::PriceLevels { universe, users } => (
                "apis",
                format!(
                    "/developer-products/v1/universes/{universe}/price-levels?{}",
                    users
                        .iter()
                        .map(|id| format!("userIds={id}"))
                        .collect::<Vec<_>>()
                        .join("&")
                ),
                None,
            ),
            Self::RobloxSubscriptionPromptProducts => (
                "apis", "/subscriptions/v2/products?productType=RobloxPlus&referrerId=0".into(), None,
            ),
            Self::RobloxSubscriptionProducts(user) => (
                "apis",
                format!("/v1/subscriptions/rcc/products?userId={user}&productType=RobloxPlus&includePurchased=true"),
                None,
            ),
            Self::SubscriptionDetails { user, subscription } => (
                "apis",
                format!(
                    "/v1/subscriptions/{}/details?userId={user}",
                    component(subscription)
                ),
                None,
            ),
            Self::InvitePermission {place,trigger,recipient} => {
                let mut path=format!("/notifications/v1/can-prompt-experience-invite?PlaceId={place}&Trigger={}",component(trigger));
                if let Some(recipient)=recipient {path.push_str(&format!("&RecipientUserId={}",component(recipient)));}
                ("apis",path,None)
            },
            Self::AdPlacements(universe) => ("apis",format!("/ads-placement-service/v1/getAdPlacements?universeId={universe}"),None),
            Self::CampaignEligibility(id) => ("apis",format!("/brand-platform/v1/immersive-branded-ads/campaigns/{}/eligibility",component(id)),None),
            Self::CommerceProduct(id) => ("apis",format!("/commerce/v1/purchasing/in-experience/commerce-products/{}/product-info",component(id)),None),
            Self::ExperienceDetails(universe) => ("games",format!("/v1/games?universeIds={universe}"),None),
            Self::ExperienceInfo(universe) => ("games",format!("/v1/games?universeIds={universe}"),None),
            Self::ExperiencePlayability(universe) => ("games",format!("/v1/games/multiget-playability-status?universeIds={universe}"),None),
            Self::ExperienceVotes(universe) => ("games",format!("/v1/games/votes?universeIds={universe}"),None),
            Self::ExperienceAgeRecommendation(universe) => ("apis","/experience-guidelines-api/experience-guidelines/get-age-recommendation".into(),Some(json!({"universeId":universe.to_string()}))),
            Self::CanPromptOptIn(universe) => ("apis",format!("/notifications/v1/can-prompt-experience-notification-opt-in?universeId={universe}"),None),
            Self::Policy => (
                "apis",
                "/player-policy-service/v1/player-policy-client".into(),
                None,
            ),
            Self::Party { id, universe, place, job } => (
                "apis",
                format!("/group-up/v1/get-group-up-universe-members?groupUpId={}&universeId={universe}&placeId={place}&instanceId={}", component(id), component(job)),
                None,
            ),
            Self::UpcomingEvents(universe) => (
                "apis",
                format!("/virtual-events/v1/universes/{universe}/virtual-events"),
                None,
            ),
            Self::ExperienceEvent(event) => (
                "apis",
                format!("/virtual-events/v1/virtual-events/{}", component(event)),
                None,
            ),
            Self::Users(ids) => (
                "users",
                "/v1/users".into(),
                Some(json!({"userIds":ids,"excludeBannedUsers":true})),
            ),
            Self::SubscriptionProduct(id) => (
                "apis",
                format!("/v1/subscriptions/{}/product-info", component(id)),
                None,
            ),
            Self::SubscriptionStatus { user, subscription } => (
                "apis",
                format!(
                    "/v1/subscriptions/{}/status?userId={user}",
                    component(subscription)
                ),
                None,
            ),
            Self::DeveloperProducts { universe, cursor } => (
                "apis",
                format!(
                    "/developer-products/v2/universes/{universe}/developerproducts?limit=50{}",
                    if cursor.is_empty() {
                        String::new()
                    } else {
                        format!("&cursor={}", component(cursor))
                    }
                ),
                None,
            ),
        };
        Ok(Request { host, path, body })
    }
    pub fn fetch(&self, session: &Session) -> Result<Value> {
        self.fetch_at(session, |host| format!("https://{host}.roblox.com"))
    }
    pub fn fetch_in(&self, session: &Session, scope: &CacheScope) -> Result<Value> {
        self.fetch_at_scoped(session, scope.0.0, |host| format!("https://{host}.roblox.com"))
    }
    fn fetch_at(&self, session: &Session, origin: impl Fn(&str) -> String) -> Result<Value> {
        self.fetch_at_scoped(session, 0, origin)
    }
    fn fetch_at_scoped(&self, session: &Session, scope: u64, origin: impl Fn(&str) -> String) -> Result<Value> {
        let r = self.request()?;
        let endpoint=origin(r.host);
        let url = format!("{}{}", endpoint, r.path);
        // Native FastInt defaults at ,  and
        //  are 60 seconds. Asset caching defaults to zero.
        let ttl = match self {
            Self::Product { kind, .. } if kind == "GamePass" || kind == "Product" => 60,
            Self::SubscriptionProduct(_) | Self::SubscriptionDetails { .. } | Self::SubscriptionStatus { .. } | Self::RankProducts(_) | Self::TopProducts(_) => 60,
            // Installed native DFIntGamePassOwnershipCacheSeconds. The request
            // body is part of the key: the route itself is shared by all IDs.
            Self::PassOwnership { .. } => 86400,
            _ => 0,
        };
        let key = (
            session.cookie().cache_key(),
            format!(
                "scope:{scope}\n{url}\n{}",
                r.body.as_ref().map(Value::to_string).unwrap_or_default()
            ),
        );
        if ttl > 0 {
            let mut cache = PRODUCT_CACHE
                .lock()
                .map_err(|_| Error::new("Product cache unavailable"))?;
            cache.retain(|_, (at, ttl, _)| at.elapsed() < *ttl);
            if let Some((_, _, value)) = cache.get(&key) {
                return Ok(value.clone());
            }
        }
        let request = || {
            // Another leader may have completed after our first cache check.
            if ttl>0 {
                let cache=PRODUCT_CACHE.lock().map_err(|_|Error::new("Product cache unavailable"))?;
                if let Some((at,lifetime,value))=cache.get(&key) {
                    if at.elapsed()<*lifetime { return Ok(value.clone()); }
                }
            }
            let mut value=if let Some(kind)=BatchKind::of(self) {
                batched_read(session,scope,&endpoint,kind,self.clone())?
            }else{send_read(session,&endpoint,&r)?};
            // Native resolves/cache-fills each requested identifier separately.
            // An omitted batch item becomes an API error, not a cached miss.
            let rows=match self {
                Self::Product {id,kind} if kind=="Product"=>Some(("developerProducts","ProductId",*id)),
                Self::Product {id,kind} if kind=="GamePass"=>Some(("gamePasses","id",*id)),
                _=>None,
            };
            let mut cacheable=true;
            if let Some((field,id_field,id))=rows {
                if let Some(records)=value.get_mut(field).and_then(Value::as_array_mut) {
                    records.retain(|row|row[id_field].as_u64()==Some(id));
                    cacheable=!records.is_empty();
                }else{cacheable=false;}
            }
            if let Self::PassOwnership {user,pass}=self {
                for field in ["ownerships","ownershipIdentifiers"] {
                    if let Some(records)=value.get_mut(field).and_then(Value::as_array_mut) {
                        records.retain(|row|row["userId"].as_u64()==Some(*user)&&row["gamePassId"].as_u64()==Some(*pass));
                    }
                }
            }
            if matches!(self, Self::Party { .. }) && !value.is_object() {
                return Err(Error::new("Unable to convert response to JSON"));
            }
            if ttl > 0 && cacheable {
                PRODUCT_CACHE
                    .lock()
                    .map_err(|_| Error::new("Product cache unavailable"))?
                    .insert(
                        key.clone(),
                        (Instant::now(), Duration::from_secs(ttl), value.clone()),
                    );
            }
            Ok(value)
        };
        if ttl>0 { shared_request(key.clone(), request) } else { request() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockReply, MockServer};
    use crate::{Cookie, Profile};
    use std::sync::Arc;
    #[test]
    fn event_rsvp_updates_are_tested_only_against_localhost() {
        let values=Arc::new(Mutex::new(Vec::new()));let observed=values.clone();
        let server=MockServer::start(Arc::new(move|r| {
            assert_eq!(r.method,"POST");assert_eq!(r.path,"/virtual-events/v1/virtual-events/7/rsvps");
            observed.lock().unwrap().push(serde_json::from_slice::<Value>(&r.body).unwrap());MockReply::json(200,"{}")
        })).unwrap();
        let session=Session::unchecked(Cookie::new("rsvp-local-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        update_event_rsvp_at(&session,"7",true,&server.base).unwrap();update_event_rsvp_at(&session,"7",false,&server.base).unwrap();
        assert_eq!(*values.lock().unwrap(),vec![json!({"rsvpStatus":"going"}),json!({"rsvpStatus":"notGoing"})]);
    }
    #[test]
    fn moderation_review_requests_are_tested_only_against_localhost() {
        let response=Arc::new(Mutex::new((200u16,"ignored success body".to_owned())));let replies=response.clone();
        let server=MockServer::start(Arc::new(move|r| {
            assert_eq!(r.method,"POST");assert_eq!(r.path,"/user/cloud/v2/universes/7/reviewable-content/a%2Fb:requestReview?idempotencyKey.key=mock-key");assert_eq!(r.body,b"{}");
            let(status,body)=replies.lock().unwrap().clone();MockReply::json(status,&body)
        })).unwrap();
        let session=Session::unchecked(Cookie::new("review-local-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        assert_eq!(request_content_review_at(&session,7,"a/b","mock-key",&server.base).unwrap(),Value::Null);
        for(status,body,error)in [(404,"{}","ReviewableContent with Id \"a/b\" does not exist"),(429,r#"{"message":"Slow down"}"#,"HTTP status 429 not OK: Slow down"),(503,"{}","HTTP status 503 not OK: <no returned error string>"),(204,"","HTTP status 204 not OK: <unparseable response body>"),(201,r#"{"message":1}"#,"HTTP status 201 not OK: <no returned error string>")] {
            *response.lock().unwrap()=(status,body.into());assert_eq!(request_content_review_at(&session,7,"a/b","mock-key",&server.base).unwrap_err().to_string(),error);
        }
    }
    #[test]
    fn joining_community_is_tested_exclusively_against_localhost() {
        let server=MockServer::start(Arc::new(|r| {
            assert_eq!(r.method,"POST");assert_eq!(r.path,"/v1/groups/7/users");assert_eq!(r.body,b"[]");
            MockReply::json(200,"{}")
        })).unwrap();
        let session=Session::unchecked(Cookie::new("join-local-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        assert_eq!(join_community_at(&session,7,&server.base).unwrap(),json!({}));
    }
    #[test]
    fn group_profile_reads_use_corescript_component_requests() {
        for (actions_only,deactivated,names) in [(false,false,vec!["CommunityProfileHeader","Actions","CommunityLocked"]),(false,true,vec!["CommunityProfileHeader","Actions","CommunityLocked","CommunityDeactivated"]),(true,true,vec!["Actions"])] {
            let expected=json!({"profileId":1200769,"profileType":"Community","components":names.iter().map(|component|json!({"component":component})).collect::<Vec<_>>()});
            let server=MockServer::start(Arc::new(move |r| {
                assert_eq!(r.method,"POST");assert_eq!(r.path,"/profile-platform-api/v1/profiles/get");assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap(),expected);
                MockReply::json(200,r#"{"components":{"Actions":{"buttons":[{"type":"ViewCommunity"}]}}}"#)
            })).unwrap();
            let session=Session::unchecked(Cookie::new("profile-local-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
            let value=Read::GroupProfile{group:1200769,actions_only,deactivated}.fetch_at_scoped(&session,0,|_:&str|server.base.clone()).unwrap();
            assert_eq!(value["components"]["Actions"]["buttons"][0]["type"],"ViewCommunity");
        }
    }
    #[test]
    #[ignore="explicit login-session read-only profile retrieval"]
    fn live_group_profile_read() {
        let client=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        let response=Read::GroupProfile{group:1200769,actions_only:false,deactivated:true}.fetch(&client.session).unwrap();
        println!("read-only community metadata: {response}");
    }
    #[test]
    #[ignore="explicit login-session read-only feedback/shop eligibility retrieval"]
    fn live_feedback_and_shop_reads() {
        let client=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        for (name,read) in [
            ("feedback",Read::FeedbackEligibility(10000000000008)),
            ("support",Read::PlayerSupportEligibility(10000000007)),
            ("shop",Read::ShopWidgets{universe:10000000007,request:"0c84c90e-9460-4eef-a575-d39a6eea6a92".into()}),
        ] {
            match read.fetch(&client.session) {Ok(value)=>println!("read-only {name}: {value}"),Err(error)=>println!("read-only {name} error: {error}")}
        }
    }
    #[test]
    #[ignore="explicit login-session read-only experience details metadata"]
    fn live_experience_details_reads() {
        let client=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        for(name,read)in[("info",Read::ExperienceInfo(10000000007)),("playability",Read::ExperiencePlayability(10000000007)),("votes",Read::ExperienceVotes(10000000007)),("age",Read::ExperienceAgeRecommendation(10000000007))] {
            match read.fetch(&client.session){Ok(value)=>println!("read-only experience {name}: {value}"),Err(error)=>println!("read-only experience {name} error: {error}")}
        }
    }
    #[test]
    fn feedback_submission_is_tested_only_on_localhost() {
        let code=Arc::new(Mutex::new(200));let reply=code.clone();
        let server=MockServer::start(Arc::new(move |r| {
            assert_eq!(r.method,"POST");assert_eq!(r.path,"/player-generated-reviews-service/v1/channels/experience-discovery-page/assets/7/reviews");
            assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap(),json!({"category_type":"InExperience","category_option_ids":[],"comment":"mock-only \"quote\"\nline"}));
            MockReply::json(*reply.lock().unwrap(),"{}")
        })).unwrap();
        let session=Session::unchecked(Cookie::new("feedback-local-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        assert_eq!(submit_player_feedback_at(&session,7,"mock-only \"quote\"\nline",&server.base).unwrap(),Value::Null);
        *code.lock().unwrap()=429;assert_eq!(submit_player_feedback_at(&session,7,"mock-only \"quote\"\nline",&server.base).unwrap_err().to_string(),"HTTP 429");
    }
    #[test]
    fn bulk_fulfillment_requires_studio_authorization_and_is_tested_only_on_localhost() {
        let count=Arc::new(std::sync::atomic::AtomicUsize::new(0));let requests=count.clone();
        let status=Arc::new(Mutex::new(200));let reply=status.clone();
        let server=MockServer::start(Arc::new(move |r| {
            requests.fetch_add(1,Ordering::SeqCst);
            assert_eq!(r.method,"POST");assert_eq!(r.path,"/cloud/v2/avatar-marketplace-orders?idempotencyKey.key=mock%20key");
            assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap(),json!({"batchPurchaseAuthToken":"studio-bulk-purchase-01234567-89ab-cdef-0123-456789abcdef","items":[{"id":"7","type":"AvatarAsset"}]}));
            MockReply::json(*reply.lock().unwrap(),r#"{"fulfillmentGroups":[{"lineItems":[{"status":"SUCCEEDED"}]}]}"#)
        })).unwrap();
        let session=Session::unchecked(Cookie::new("bulk-fulfillment-local-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        for value in [json!({}),json!({"batchPurchaseAuthToken":"production-token"}),json!({"batchPurchaseAuthToken":"studio-bulk-purchase-"})] {
            assert!(fulfill_studio_bulk_at(&session,value,"mock key",&server.base).unwrap_err().to_string().contains("Studio test authorization"));
        }
        assert_eq!(count.load(Ordering::SeqCst),0);
        let body=json!({"batchPurchaseAuthToken":"studio-bulk-purchase-01234567-89ab-cdef-0123-456789abcdef","items":[{"id":"7","type":"AvatarAsset"}]});
        assert_eq!(fulfill_studio_bulk_at(&session,body.clone(),"mock key",&server.base).unwrap()["fulfillmentGroups"][0]["lineItems"][0]["status"],"SUCCEEDED");
        *status.lock().unwrap()=403;assert_eq!(fulfill_studio_bulk_at(&session,body,"mock key",&server.base).unwrap_err().to_string(),"HTTP 403 (Forbidden)");
    }
    #[test]
    fn private_identity_get_or_create_is_tested_only_on_localhost() {
        let response=Arc::new(Mutex::new((200u16,r#"{"privateUserId":18446744073709551615,"created":false,"userIdMode":"private"}"#.to_owned())));
        let replies=response.clone();
        let server=MockServer::start(Arc::new(move |r| {
            assert_eq!(r.method,"POST");assert_eq!(r.path,"/private-users-service/v1beta1/getOrCreatePrivateUserId");
            assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap(),json!({"global_user_id":123,"domain_id":456,"domain_type":1}));
            let(status,body)=replies.lock().unwrap().clone();MockReply::json(status,&body)
        })).unwrap();
        let session=Session::unchecked(Cookie::new("user-local-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        assert_eq!(resolve_private_user_at(&session,123,456,&server.base).unwrap()["privateUserId"],"18446744073709551615");
        for text in ["{}",r#"{"privateUserId":0}"#,r#"{"privateUserId":-1}"#,"broken"] {
            *response.lock().unwrap()=(200,text.into());assert_eq!(resolve_private_user_at(&session,123,456,&server.base).unwrap_err().to_string(),"UserService: User not found");
        }
        *response.lock().unwrap()=(429,r#"{"message":"Too many requests"}"#.into());
        assert_eq!(resolve_private_user_at(&session,123,456,&server.base).unwrap_err().to_string(),"UserService: Request failed because Too many requests");
    }
    #[test]
    fn share_link_preparation_is_tested_only_on_localhost() {
        let response=Arc::new(Mutex::new((200u16,r#"{"link":"https://mock.invalid/share"}"#.to_owned())));
        let replies=response.clone();
        let server=MockServer::start(Arc::new(move |request| {
            assert_eq!(request.path,"/deeplinks/v2/rcc/expiring");assert_eq!(request.method,"POST");
            assert_eq!(serde_json::from_slice::<Value>(&request.body).unwrap(),json!({"CreatorUserId":1,"FallbackLinkId":"mock","CustomData":"local-data"}));
            let (status,body)=replies.lock().unwrap().clone();MockReply::json(status,&body)
        })).unwrap();
        let session=Session::unchecked(Cookie::new("link-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        let body=json!({"CreatorUserId":1,"FallbackLinkId":"mock","CustomData":"local-data"});
        assert_eq!(create_expiring_share_link_at(&session,body.clone(),&server.base).unwrap()["link"],"https://mock.invalid/share");
        for response_body in ["{}",r#"{"link":""}"#,r#"{"link":1}"#,"not json"] {
            *response.lock().unwrap()=(200,response_body.into());
            assert_eq!(create_expiring_share_link_at(&session,body.clone(),&server.base).unwrap_err().to_string(),"SocialService:PromptLinkSharing failed to create a share link due to network error");
        }
        *response.lock().unwrap()=(403,"{}".into());
        assert_eq!(create_expiring_share_link_at(&session,body,&server.base).unwrap_err().to_string(),"HTTP 403 (Forbidden)");
    }
    #[test]
    fn recommendation_mutations_are_tested_exclusively_against_mock_http() {
        let response=Arc::new(Mutex::new((200u16,r#"{"results":[{"itemId":"mock-id","referenceId":"mock-reference"}]}"#.to_owned())));
        let replies=response.clone();
        let server=MockServer::start(Arc::new(move |request| {
            assert_eq!(request.method,"POST");
            let value:Value=serde_json::from_slice(&request.body).unwrap();
            match request.path.as_str() {
                "/recommendation-api/v1/rcc/recommendation-item/batch"=>assert_eq!(value,json!({"experienceId":7,"items":[{"referenceId":"mock-reference","creator":{"creatorId":1,"creatorType":"User"}}]})),
                "/recommendation-api/v1/rcc/recommendation-item/update"=>assert_eq!(value,json!({"itemId":"mock-id","referenceId":"new"})),
                "/recommendation-api/v1/rcc/recommendation-item/delete"|"/recommendation-api/v1/recommendation-item/delete"=>assert_eq!(value,json!({"itemId":"mock-id"})),
                _=>panic!("unexpected route"),
            }
            let (status,body)=replies.lock().unwrap().clone();MockReply::json(status,&body)
        })).unwrap();
        let session=Session::unchecked(Cookie::new("recommendation-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        let register=json!({"experienceId":7,"items":[{"referenceId":"mock-reference","creator":{"creatorId":1,"creatorType":"User"}}]});
        let value=RecommendationMutation::Register.execute_at(&session,register.clone(),&server.base).unwrap();
        assert_eq!(value,json!({"ItemId":"mock-id","ReferenceId":"mock-reference"}));
        let update=json!({"itemId":"mock-id","referenceId":"new"});
        assert!(RecommendationMutation::Update.execute_at(&session,update.clone(),&server.base).unwrap().is_null());
        for client in [false,true] {assert!(RecommendationMutation::Remove{client}.execute_at(&session,json!({"itemId":"mock-id"}),&server.base).unwrap().is_null());}
        for (body,error) in [("{}","Failed to register recommendation items."),(r#"{"results":[{}]}"#,"Failed to register recommendation items."),("not json","Failed to register recommendation item.")] {
            *response.lock().unwrap()=(200,body.into());
            assert_eq!(RecommendationMutation::Register.execute_at(&session,register.clone(),&server.base).unwrap_err().to_string(),error);
        }
        *response.lock().unwrap()=(200,r#"{"results":[]}"#.into());
        assert_eq!(RecommendationMutation::Register.execute_at(&session,register.clone(),&server.base).unwrap(),json!({"ItemId":"","ReferenceId":""}));
        *response.lock().unwrap()=(429,"{}".into());
        assert_eq!(RecommendationMutation::Register.execute_at(&session,register,&server.base).unwrap_err().to_string(),"Failed to register recommendation items - HTTP 429 (Too Many Requests)");
        assert_eq!(RecommendationMutation::Update.execute_at(&session,update,&server.base).unwrap_err().to_string(),"Failed to update recommendation items - HTTP 429 (Too Many Requests)");
        assert_eq!(RecommendationMutation::Remove{client:false}.execute_at(&session,json!({"itemId":"mock-id"}),&server.base).unwrap_err().to_string(),"Failed to remove item - HTTP 429 (Too Many Requests)");
    }
    #[test]
    fn invitation_send_transport_is_exercised_only_against_localhost() {
        let server=MockServer::start(Arc::new(|request| {
            assert_eq!(request.path,"/notifications/v1/send-experience-invite");assert_eq!(request.method,"POST");
            let body:Value=serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body,json!({"recipientUserId":"2","placeId":"7","trigger":"DeveloperMultiple","idempotencyKey":"mock-key"}));
            MockReply::json(200,r#"{"conversationId":"mock-conversation"}"#)
        })).unwrap();
        let session=Session::unchecked(Cookie::new("invite-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        let value=send_experience_invite_at(&session,json!({"recipientUserId":"2","placeId":"7","trigger":"DeveloperMultiple","idempotencyKey":"mock-key"}),&server.base).unwrap();
        assert_eq!(value["conversationId"],"mock-conversation");assert_eq!(server.requests().len(),1);
    }
    #[test]
    fn studio_bulk_preparation_never_calls_a_purchase_route() {
        let response=Arc::new(Mutex::new((200_u16,r#"{"items":[],"order":{},"purchaserRobuxBalance":100,"orderTotalRobux":7}"#.to_owned())));
        let replies=response.clone();
        let server=MockServer::start(Arc::new(move |request| {
            assert_eq!(request.method,"POST");assert_eq!(request.path,"/marketplace-ua/game-engine/v1/prepare-studio-bulk-purchase-prompt");
            assert_eq!(serde_json::from_slice::<Value>(&request.body).unwrap(),json!({"purchaserUserId":1,"items":[{"id":"7","type":"AvatarAsset"}]}));
            let reply=replies.lock().unwrap();MockReply::json(reply.0,&reply.1)
        })).unwrap();
        let session=Session::unchecked(Cookie::new("bulk-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        let request=json!({"purchaserUserId":1,"items":[{"id":"7","type":"AvatarAsset"}]});
        assert_eq!(prepare_studio_bulk_at(&session,request.clone(),&server.base).unwrap()["orderTotalRobux"],7);
        for (status,body,message)in [
            (400,"{}","HTTP 400 (Bad Request)"),
            (429,"{}","HTTP 429 (Too Many Requests)"),
            (200,"bad","an error occurred while parsing the prepare bulk purchase response"),
            (200,"{}","an error occurred while parsing the prepare bulk purchase response"),
            (200,r#"{"purchaserRobuxBalance":"100","orderTotalRobux":7}"#,"an error occurred while parsing the prepare bulk purchase response"),
        ] {
            *response.lock().unwrap()=(status,body.to_owned());assert_eq!(prepare_studio_bulk_at(&session,request.clone(),&server.base).unwrap_err().0,message);
        }
        assert_eq!(server.requests().len(),6);
    }
    #[test]
    fn reservation_post_and_native_parser_are_mock_only() {
        let response=Arc::new(Mutex::new((200_u16,r#"{"ReservedServerAccessCode":"access","ReservedServerGameCode":"private"}"#.to_owned())));
        let replies=response.clone();
        let server=MockServer::start(Arc::new(move |request| {
            assert_eq!(request.method,"POST");assert_eq!(request.path,"/reserved-servers-service/create?placeId=7");assert!(request.body.is_empty());
            let reply=replies.lock().unwrap();MockReply::json(reply.0,&reply.1)
        })).unwrap();
        let session=Session::unchecked(Cookie::new("reserve-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        assert_eq!(reserve_server_at(&session,7,&server.base).unwrap()["ReservedServerGameCode"],"private");
        for (status,body,message) in [
            (200,"","ReserveServer error: Response is empty"),
            (200,"bad","ReserveServer error: Could not parse JSON result"),
            (200,"{}","ReserveServer error: No ReservedServerAccessCode returned"),
            (200,r#"{"ReservedServerAccessCode":1}"#,"ReserveServer error: ReservedServerAccessCode is not of type string"),
            (200,r#"{"ReservedServerAccessCode":"a"}"#,"ReserveServer error: No ReservedServerGameCode returned"),
            (200,r#"{"ReservedServerAccessCode":"a","ReservedServerGameCode":null}"#,"ReserveServer error: ReservedServerGameCode is not of type string"),
            (429,"{}","HTTP 429 (Too Many Requests)"),
        ] {
            *response.lock().unwrap()=(status,body.to_owned());
            assert_eq!(reserve_server_at(&session,7,&server.base).unwrap_err().0,message);
        }
        assert_eq!(server.requests().len(),8);
    }
    #[test]
    fn subscription_sheet_initialization_uses_only_mock_posts() {
        let server=MockServer::start(Arc::new(|request| {
            if request.method=="POST" {
                assert_eq!(request.path,"/apis/payments-gateway/v1/payment-sessions");
                assert_eq!(serde_json::from_slice::<Value>(&request.body).unwrap(),json!({"entryPoint":"PromptSubscriptionPurchase"}));
                // As in the CoreScript, even a failed session request does not
                // discard successful product metadata.
                MockReply::json(503,"{}")
            }else{
                assert_eq!(request.path,"/apis/subscriptions/v2/products?productType=RobloxPlus&referrerId=0");
                MockReply::json(200,r#"{"products":[{"periodType":"Month","productKey":{"id":"PLUS-1"}}]}"#)
            }
        })).unwrap();
        let session=Session::unchecked(Cookie::new("subscription-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        let value=subscription_prompt_at(&session,&CacheScope::default(),|host|format!("{}/{host}",server.base)).unwrap();
        assert_eq!(value["products"][0]["productKey"]["id"],"PLUS-1");
        assert_eq!(server.requests().len(),2);
    }
    #[test]
    fn notification_post_actions_are_tested_only_with_mock_http() {
        let server=MockServer::start(Arc::new(|request| {
            assert_eq!(request.method,"POST");assert_eq!(request.header("cookie"),Some(".ROBLOSECURITY=notification-mock"));
            if request.path=="/apis/notifications/v1/prompt-experience-notification-opt-in" {
                assert_eq!(serde_json::from_slice::<Value>(&request.body).unwrap(),json!({"universeId":7}));
                MockReply::json(200,r#"{"canPrompt":true}"#)
            }else{
                assert_eq!(request.path,"/followings/v1/users/1/universes/7");
                assert_eq!(request.body,b"[]");MockReply::json(204,"")
            }
        })).unwrap();
        let session=Session::unchecked(Cookie::new("notification-mock").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        assert_eq!(NotificationAction::Prompt {universe:7}.execute_at(&session,|host|format!("{}/{host}",server.base)).unwrap(),json!({"canPrompt":true}));
        assert_eq!(NotificationAction::Follow {user:1,universe:7}.execute_at(&session,|host|format!("{}/{host}",server.base)).unwrap(),Value::Null);
        assert_eq!(server.requests().len(),2);
        assert!(NotificationAction::Follow {user:2,universe:7}.execute_at(&session,|host|format!("{}/{host}",server.base)).is_err());
        assert_eq!(server.requests().len(),2);
    }
    #[test]
    fn concurrent_cached_metadata_uses_one_http_request() {
        use std::sync::{Barrier, atomic::AtomicUsize};
        let count=Arc::new(AtomicUsize::new(0));
        let seen=count.clone();
        let server=MockServer::start(Arc::new(move |_| {
            seen.fetch_add(1,Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(100));
            MockReply::json(200,r#"{"developerProducts":[{"ProductId":7,"Name":"Shared product","Creator":{"Id":1}}]}"#)
        })).unwrap();
        let scope=CacheScope::default();
        let session=Arc::new(Session::unchecked(Cookie::new("coalesced-session").unwrap(),Profile { id:1,name:"Mock".into(),display_name:"Mock".into() }));
        let start=Arc::new(Barrier::new(9));
        let mut threads=Vec::new();
        for _ in 0..8 {
            let session=session.clone();let start=start.clone();let origin=server.base.clone();let scope=scope.clone();
            threads.push(std::thread::spawn(move || {
                start.wait();
                Read::Product { id:7,kind:"Product".into() }.fetch_at_scoped(&session,scope.0.0,|_|origin.clone()).unwrap()
            }));
        }
        start.wait();
        let mut values:Vec<_>=threads.into_iter().map(|t|t.join().unwrap()).collect();
        assert_eq!(count.load(Ordering::SeqCst),1);
        values[0]["developerProducts"][0]["Creator"]["Id"]=json!(9);
        assert_eq!(values[1]["developerProducts"][0]["Creator"]["Id"],1);
        let other=CacheScope::default();
        Read::Product { id:7,kind:"Product".into() }.fetch_at_scoped(&session,other.0.0,|_|server.base.clone()).unwrap();
        assert_eq!(count.load(Ordering::SeqCst),2);
    }
    #[test]
    fn different_ids_batch_separately_and_results_remain_per_request() {
        use std::sync::Barrier;
        let server=MockServer::start(Arc::new(|request| {
            assert_eq!(request.header("cookie"),Some(".ROBLOSECURITY=batch-session"));
            let value=if let Some(ids)=request.path.split("?productIds=").nth(1) {
                let ids:Vec<u64>=ids.split(',').map(|n|n.parse().unwrap()).collect();
                assert!(ids.len()<=50);
                json!({"developerProducts":ids.into_iter().map(|id|json!({"ProductId":id,"Name":format!("P{id}")})).collect::<Vec<_>>()})
            }else if request.path.ends_with(":batchGet") {
                let body:Value=serde_json::from_slice(&request.body).unwrap();
                let ids=body["gamePassIds"].as_array().unwrap();assert!(ids.len()<=50);assert_eq!(body["passView"],"Price");
                json!({"gamePasses":ids.iter().map(|id|json!({"id":id,"displayName":format!("G{id}")})).collect::<Vec<_>>()})
            }else{
                assert!(request.path.ends_with(":batchGetOwnership"));
                let body:Value=serde_json::from_slice(&request.body).unwrap();
                let ids=body["ownershipIdentifiers"].as_array().unwrap();assert!(ids.len()<=50);
                json!({"ownershipIdentifiers":ids.iter().filter(|id|id["userId"]==7).cloned().collect::<Vec<_>>()})
            };
            MockReply::json(200,&value.to_string())
        })).unwrap();
        let session=Arc::new(Session::unchecked(Cookie::new("batch-session").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()}));
        let scope=CacheScope::default();
        let start=Arc::new(Barrier::new(7));let mut threads=Vec::new();
        for read in [Read::Product{id:7,kind:"Product".into()},Read::Product{id:8,kind:"Product".into()},Read::Product{id:7,kind:"GamePass".into()},Read::Product{id:8,kind:"GamePass".into()},Read::PassOwnership{user:7,pass:1},Read::PassOwnership{user:8,pass:1}] {
            let (scope,start,session,origin)=(scope.clone(),start.clone(),session.clone(),server.base.clone());
            threads.push(std::thread::spawn(move || {
                start.wait();let value=read.fetch_at_scoped(&session,scope.0.0,|_|origin.clone()).unwrap();(read,value)
            }));
        }
        start.wait();
        for thread in threads {
            let (read,value)=thread.join().unwrap();
            match read {
                Read::Product{id,kind} if kind=="Product"=>{assert_eq!(value["developerProducts"].as_array().unwrap().len(),1);assert_eq!(value["developerProducts"][0]["ProductId"],id);},
                Read::Product{id,..}=>{assert_eq!(value["gamePasses"].as_array().unwrap().len(),1);assert_eq!(value["gamePasses"][0]["id"],id);},
                Read::PassOwnership{user,..}=>assert_eq!(value["ownershipIdentifiers"].as_array().unwrap().len(),usize::from(user==7)),
                _=>unreachable!(),
            }
        }
        assert_eq!(server.requests().len(),3);
        Read::Product{id:8,kind:"Product".into()}.fetch_at_scoped(&session,scope.0.0,|_|server.base.clone()).unwrap();
        assert_eq!(server.requests().len(),3);
        let start=Arc::new(Barrier::new(52));let mut threads=Vec::new();
        for id in 100..151 {
            let (scope,start,session,origin)=(scope.clone(),start.clone(),session.clone(),server.base.clone());
            threads.push(std::thread::spawn(move || {
                start.wait();let value=Read::Product{id,kind:"GamePass".into()}.fetch_at_scoped(&session,scope.0.0,|_|origin.clone()).unwrap();assert_eq!(value["gamePasses"][0]["id"],id);
            }));
        }
        start.wait();for thread in threads {thread.join().unwrap();}
        let requests=server.requests();
        let mut sizes:Vec<_>=requests[3..].iter().map(|r|serde_json::from_slice::<Value>(&r.body).unwrap()["gamePassIds"].as_array().unwrap().len()).collect();
        sizes.sort_unstable();assert!(sizes.len()>=2);assert!(sizes.iter().all(|n|*n<=50));assert_eq!(sizes.iter().sum::<usize>(),51);
    }
    #[test]
    fn omitted_metadata_item_is_not_cached() {
        let server=MockServer::start(Arc::new(|_|MockReply::json(200,r#"{"developerProducts":[]}"#))).unwrap();
        let session=Session::unchecked(Cookie::new("missing-batch-item").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        for _ in 0..2 { assert_eq!(Read::Product{id:7,kind:"Product".into()}.fetch_at(&session,|_|server.base.clone()).unwrap()["developerProducts"],json!([])); }
        assert_eq!(server.requests().len(),2);
    }
    #[test]
    fn every_read_uses_session_and_exact_route() {
        assert_eq!(Read::Users(vec![-1]).request().unwrap().body, Some(json!({"userIds":[-1],"excludeBannedUsers":true})));
        let cases = vec![
            (Read::ExperienceDetails(7),"/games/v1/games?universeIds=7"),
            (Read::CanPromptOptIn(7),"/apis/notifications/v1/can-prompt-experience-notification-opt-in?universeId=7"),
            (Read::Party { id: "party".into(), universe: 7, place: 8, job: "a b".into() }, "/apis/group-up/v1/get-group-up-universe-members?groupUpId=party&universeId=7&placeId=8&instanceId=a%20b"),
            (
                Read::Product {
                    id: 7,
                    kind: "Asset".into(),
                },
                "/economy/v2/assets/7/details",
            ),
            (
                Read::Product {
                    id: 7,
                    kind: "GamePass".into(),
                },
                "/apis/game-passes/v1/game-passes:batchGet",
            ),
            (
                Read::Product {
                    id: 7,
                    kind: "Bundle".into(),
                },
                "/catalog/v1/bundles/7/details",
            ),
            (
                Read::Product {
                    id: 7,
                    kind: "Product".into(),
                },
                "/apis/developer-products/v1/developer-products/batch?productIds=7",
            ),
            (
                Read::Owns {
                    user: 1,
                    id: 7,
                    bundle: false,
                },
                "/inventory/v1/users/1/items/asset/7/is-owned",
            ),
            (
                Read::Owns {
                    user: 1,
                    id: 7,
                    bundle: true,
                },
                "/inventory/v1/users/1/items/Bundle/7/is-owned",
            ),
            (
                Read::PassOwnership { user: 1, pass: 7 },
                "/apis/game-passes/v1/game-passes:batchGetOwnership",
            ),
            (Read::Badge(7), "/badges/v1/badges/7"),
            (
                Read::HasBadge { user: 1, badge: 7 },
                "/badges/v1/users/1/badges/7/awarded-date",
            ),
            (
                Read::CheckBadges {
                    user: 1,
                    badges: vec![7, 8],
                },
                "/badges/v1/users/1/badges/awarded-dates?badgeIds=7,8",
            ),
            (Read::Group(7), "/groups/v0/groups/7"),
            (Read::ExperienceInfo(-1), "/games/v1/games?universeIds=-1"),
            (Read::ExperiencePlayability(7), "/games/v1/games/multiget-playability-status?universeIds=7"),
            (Read::ExperienceVotes(7), "/games/v1/games/votes?universeIds=7"),
            (Read::ExperienceAgeRecommendation(7), "/apis/experience-guidelines-api/experience-guidelines/get-age-recommendation"),
            (Read::FeedbackEligibility(7), "/apis/player-generated-reviews-service/v1/channels/experience-discovery-page/assets/7"),
            (Read::PlayerSupportEligibility(7), "/apis/creator-communication/v1beta1/creator-communication-api/universes/7/ticket-creation-eligibility"),
            (Read::ShopWidgets{universe:7,request:"mock key".into()}, "/apis/marketplace-widgets/v1/widgets/shop-widgets?requestId=mock%20key&universeId=7"),
            (Read::Groups(1), "/groups/v0/users/1/groups"),
            (
                Read::GroupRoles { user: 1, group: 7 },
                "/groups/v2/groups/7/users?userIds=1",
            ),
            (
                Read::Relationships {
                    group: 7,
                    allies: true,
                    page: 1,
                },
                "/groups/v0/groups/7/relationships?relationshipType=Ally&page=1",
            ),
            (
                Read::Relationships {
                    group: 7,
                    allies: false,
                    page: 1,
                },
                "/groups/v0/groups/7/relationships?relationshipType=Enemy&page=1",
            ),
            (
                Read::Policy,
                "/apis/player-policy-service/v1/player-policy-client",
            ),
            (
                Read::UpcomingEvents(7),
                "/apis/virtual-events/v1/universes/7/virtual-events",
            ),
            (
                Read::ExperienceEvent("event".into()),
                "/apis/virtual-events/v1/virtual-events/event",
            ),
            (Read::Users(vec![1]), "/users/v1/users"),
            (Read::RankProducts(json!({"universeId":7,"productIdentifiers":[{"id":1,"type":"GamePass"}]})), "/apis/product-recommender/v1/virtual-products:rank"),
            (Read::TopProducts(json!({"universeId":7,"productInfoTypes":["Product","GamePass"]})), "/apis/product-recommender/v1/virtual-products:recommend"),
            (Read::RecommendationList(json!({"experienceId":7,"configName":"0","locationId":"0","paginationToken":{"count":10}})), "/apis/recommendation-api/v1/recommendation/generate"),
            (Read::CampaignEligibility("0".into()), "/apis/brand-platform/v1/immersive-branded-ads/campaigns/0/eligibility"),
            (Read::CommerceProduct("COM-0".into()), "/apis/commerce/v1/purchasing/in-experience/commerce-products/COM-0/product-info"),
            (Read::InvitePermission {place:7,trigger:"DeveloperMultiple".into(),recipient:None}, "/apis/notifications/v1/can-prompt-experience-invite?PlaceId=7&Trigger=DeveloperMultiple"),
            (Read::AdPlacements(7), "/apis/ads-placement-service/v1/getAdPlacements?universeId=7"),
            (Read::RobloxSubscriptionPromptProducts, "/apis/subscriptions/v2/products?productType=RobloxPlus&referrerId=0"),
            (Read::RobloxSubscriptionProducts(1), "/apis/v1/subscriptions/rcc/products?userId=1&productType=RobloxPlus&includePurchased=true"),
            (
                Read::RecommendationItem("0".into()),
                "/apis/recommendation-api/v1/recommendation-item/get",
            ),
            (
                Read::PlayerPlace { place: 7, user: 1 },
                "/apis/reserved-servers-service/get-player-place-instance?currentPlaceId=7&userId=1",
            ),
            (
                Read::BrandDecision("0".into()),
                "/apis/brand-platform/v1/brand-projects/0/decision/studio",
            ),
            (
                Read::PriceLevels {
                    universe: 7,
                    users: vec![1, 2],
                },
                "/apis/developer-products/v1/universes/7/price-levels?userIds=1&userIds=2",
            ),
            (
                Read::SubscriptionDetails {
                    user: 1,
                    subscription: "EXP-1".into(),
                },
                "/apis/v1/subscriptions/EXP-1/details?userId=1",
            ),
            (
                Read::SubscriptionProduct("EXP-7".into()),
                "/apis/v1/subscriptions/EXP-7/product-info",
            ),
            (
                Read::SubscriptionStatus {
                    user: 1,
                    subscription: "EXP-7".into(),
                },
                "/apis/v1/subscriptions/EXP-7/status?userId=1",
            ),
            (
                Read::DeveloperProducts {
                    universe: 7,
                    cursor: String::new(),
                },
                "/apis/developer-products/v2/universes/7/developerproducts?limit=50",
            ),
        ];
        let server = MockServer::start(Arc::new(|r| {
            assert_eq!(r.header("cookie"), Some(".ROBLOSECURITY=mock-session"));
            if r.path == "/apis/game-passes/v1/game-passes:batchGet" {
                assert_eq!(
                    serde_json::from_slice::<Value>(&r.body).unwrap(),
                    json!({"gamePassIds":[7],"passView":"Price"})
                );
            }
            if r.path == "/users/v1/users" {
                assert_eq!(
                    serde_json::from_slice::<Value>(&r.body).unwrap(),
                    json!({"userIds":[1],"excludeBannedUsers":true})
                );
            }
            if r.path == "/apis/game-passes/v1/game-passes:batchGetOwnership" {
                assert_eq!(
                    serde_json::from_slice::<Value>(&r.body).unwrap(),
                    json!({"ownershipIdentifiers":[{"userId":1,"gamePassId":7}]})
                );
            }
            MockReply::json(200, "{\"ok\":true}")
        }))
        .unwrap();
        let session = Session::unchecked(
            Cookie::new("mock-session").unwrap(),
            Profile {
                id: 1,
                name: "Mock".into(),
                display_name: "Mock".into(),
            },
        );
        for (read, path) in cases {
            assert_eq!(
                read.fetch_at(&session, |h| format!("{}/{h}", server.base))
                    .unwrap(),
                json!({"ok":true})
            );
            assert_eq!(server.requests().last().unwrap().path, path);
        }
    }
    #[test]
    fn product_cache_is_session_scoped_and_errors_are_retried() {
        let server = MockServer::start(Arc::new(|r| {
            if serde_json::from_slice::<Value>(&r.body)
                .ok()
                .and_then(|v| v["gamePassIds"][0].as_u64())
                == Some(9)
            {
                MockReply::json(429, "{}")
            } else {
                MockReply::json(200, r#"{"gamePasses":[{"id":7,"displayName":"Pass"}]}"#)
            }
        }))
        .unwrap();
        let session = |cookie: &str| {
            Session::unchecked(
                Cookie::new(cookie).unwrap(),
                Profile {
                    id: 1,
                    name: "Mock".into(),
                    display_name: "Mock".into(),
                },
            )
        };
        let first = session("first-session");
        let second = session("second-session");
        let pass = Read::Product {
            id: 7,
            kind: "GamePass".into(),
        };
        let mut v = pass.fetch_at(&first, |_| server.base.clone()).unwrap();
        v["gamePasses"][0]["displayName"] = json!("changed");
        assert_eq!(
            pass.fetch_at(&first, |_| server.base.clone()).unwrap()["gamePasses"][0]["displayName"],
            json!("Pass")
        );
        assert_eq!(server.requests().len(), 1);
        pass.fetch_at(&second, |_| server.base.clone()).unwrap();
        assert_eq!(server.requests().len(), 2);
        let fail = Read::Product {
            id: 9,
            kind: "GamePass".into(),
        };
        assert!(fail.fetch_at(&first, |_| server.base.clone()).is_err());
        assert!(fail.fetch_at(&first, |_| server.base.clone()).is_err());
        assert_eq!(server.requests().len(), 4);
        let scope_a = CacheScope::default();
        let scope_b = CacheScope::default();
        pass.fetch_at_scoped(&first, scope_a.0.0, |_| server.base.clone()).unwrap();
        pass.fetch_at_scoped(&first, scope_a.0.0, |_| server.base.clone()).unwrap();
        assert_eq!(server.requests().len(), 5);
        pass.fetch_at_scoped(&first, scope_b.0.0, |_| server.base.clone()).unwrap();
        assert_eq!(server.requests().len(), 6);
    }
    #[test]
    fn throttle_and_invalid_json_do_not_turn_into_success() {
        let server = MockServer::start(Arc::new(|r| {
            if r.path.ends_with("/8") {
                MockReply::json(200, "not json")
            } else {
                MockReply::json(429, "{}")
            }
        }))
        .unwrap();
        let session = Session::unchecked(
            Cookie::new("mock-session").unwrap(),
            Profile {
                id: 1,
                name: "Mock".into(),
                display_name: "Mock".into(),
            },
        );
        assert_eq!(
            Read::Badge(7)
                .fetch_at(&session, |_| server.base.clone())
                .unwrap_err()
                .0,
            "HTTP 429 (Too Many Requests)"
        );
        assert_eq!(
            Read::Badge(8)
                .fetch_at(&session, |_| server.base.clone())
                .unwrap_err()
                .0,
            "Unable to convert response to JSON"
        );
    }
    #[test]
    #[ignore = "explicit read-only experience invitation prompt permission lookup"]
    fn live_invite_permission() {
        crate::session::set_place_context(10000000000008);
        let client=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        for place in [0,10000000000008] {
            match (Read::InvitePermission {place,trigger:"DeveloperMultiple".into(),recipient:None}).fetch(&client.session) {
                Ok(value)=>println!("invite prompt permission ({place}): {}",value),
                Err(e)=>{println!("invite prompt permission ({place}): {}",e.0);assert!(e.0.starts_with("HTTP "));},
            }
        }
    }
    #[test]
    #[ignore = "explicit read-only Studio ad placement metadata lookup"]
    fn live_ad_placements() {
        crate::session::set_place_context(10000000000008);
        let client=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        match Read::AdPlacements(10000000007).fetch(&client.session) {
            Ok(value)=>println!("ad placement metadata response: {}",value),
            Err(e)=>{println!("ad placement metadata read: {}",e.0);assert!(e.0.starts_with("HTTP "));},
        }
    }
    #[test]
    #[ignore = "explicit read-only Roblox subscription client product lookup"]
    fn live_subscription_prompt_products() {
        crate::session::set_place_context(10000000000008);
        let client=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        let result=Read::RobloxSubscriptionPromptProducts.fetch(&client.session);
        match result {
            Ok(value)=>{
                let products=value["products"].as_array().expect("products array");
                println!("subscription prompt product count: {}",products.len());
                for product in products {
                    println!("subscription product fields: {:?}",product.as_object().unwrap().keys().collect::<Vec<_>>());
                    assert!(product["periodType"].is_string());
                }
            },
            Err(e)=>{println!("subscription prompt product read: {}",e.0);assert!(e.0.starts_with("HTTP "));},
        }
    }
    #[test]
    #[ignore = "explicit read-only notification eligibility over the login session"]
    fn live_notification_eligibility() {
        crate::session::set_place_context(10000000000008);
        let client=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        match Read::CanPromptOptIn(10000000007).fetch(&client.session) {
            Ok(value)=>{println!("notification eligibility: {}",value);assert!(value["canPrompt"].is_boolean());}
            Err(e)=>{println!("notification eligibility read: {}",e.0);assert!(e.0.starts_with("HTTP "));}
        }
        let details=Read::ExperienceDetails(10000000007).fetch(&client.session).unwrap();
        assert!(details["data"][0]["name"].is_string());
        println!("notification dialog experience name: string");

    }
    #[test]
    #[ignore = "explicit public metadata batch over the login session"]
    fn live_metadata_batch() {
        crate::session::set_place_context(10000000000008);
        let client=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        let products=Read::DeveloperProducts { universe:2440500124,cursor:String::new() }.fetch(&client.session).unwrap();
        let ids:Vec<_>=products["developerProducts"].as_array().unwrap().iter().filter_map(|row|row["ProductId"].as_u64()).take(2).collect();
        assert_eq!(ids.len(),2);
        let scope=CacheScope::default();let start=std::sync::Barrier::new(3);
        std::thread::scope(|threads| {
            let handles:Vec<_>=ids.iter().map(|id|{
                let scope=scope.clone();let start=&start;let session=&client.session;
                threads.spawn(move || {
                    start.wait();let value=Read::Product {id:*id,kind:"Product".into()}.fetch_in(session,&scope).unwrap();
                    assert_eq!(value["developerProducts"][0]["ProductId"],*id);
                })
            }).collect();
            start.wait();for handle in handles {handle.join().unwrap();}
        });
        println!("login-session metadata batch: two public products returned independently");
    }
    #[test]
    #[ignore = "explicit read-only party lookup using the login session"]
    fn live_party_read() {
        crate::session::set_place_context(10000000000008);
        let client=crate::Client::open(crate::AccountConfig::load().unwrap())
            .unwrap_or_else(|e|panic!("{}",e.describe()));
        let result=Read::Party { id: "00000000-0000-0000-0000-000000000000".into(), universe:10000000007,place:10000000000008,job:String::new() }.fetch(&client.session);
        match result {
            Ok(value)=>assert!(value.is_object()),
            Err(e)=>{println!("party read: {}",e.0);assert!(e.0.starts_with("HTTP "));}
        }
    }
    #[test]
    #[ignore = "explicit real read-only login-session smoke test"]
    fn live_reads() {
        crate::session::set_place_context(10000000000008);
        let client = crate::Client::open(crate::AccountConfig::load().unwrap())
            .unwrap_or_else(|e| panic!("{}", e.describe()));
        if let Ok(groups) = Read::Groups(client.user_id().try_into().unwrap()).fetch(&client.session) {
            if let Some(id) = groups
                .as_array()
                .and_then(|a| a.first())
                .and_then(|g| g.get("Id"))
                .and_then(Value::as_u64)
            {
                let roles = Read::GroupRoles {
                    user: client.user_id(),
                    group: id,
                }
                .fetch(&client.session)
                .unwrap();
                fn schema(v: &Value) -> Value {
                    match v {
                        Value::Object(o) => {
                            Value::Object(o.iter().map(|(k, v)| (k.clone(), schema(v))).collect())
                        }
                        Value::Array(a) => Value::Array(a.iter().take(1).map(schema).collect()),
                        Value::Null => Value::Null,
                        Value::Bool(_) => json!("bool"),
                        Value::Number(_) => json!("number"),
                        _ => json!("string"),
                    }
                }
                println!("member group roles schema: {}", schema(&roles));
            }
        }
        let group = Read::Group(1200769).fetch(&client.session).unwrap();
        if let Some(owner) = group.pointer("/Owner/Id").and_then(Value::as_u64) {
            let roles = Read::GroupRoles {
                user: owner,
                group: 1200769,
            }
            .fetch(&client.session)
            .unwrap();
            fn shape(v: &Value) -> Value {
                match v {
                    Value::Object(o) => {
                        Value::Object(o.iter().map(|(k, v)| (k.clone(), shape(v))).collect())
                    }
                    Value::Array(a) => Value::Array(a.iter().take(1).map(shape).collect()),
                    Value::Null => Value::Null,
                    Value::Bool(_) => json!("bool"),
                    Value::Number(_) => json!("number"),
                    _ => json!("string"),
                }
            }
            println!("group roles schema: {}", shape(&roles));
        }
        if let Ok(v) = (Read::DeveloperProducts {
            universe: 2440500124,
            cursor: String::new(),
        })
        .fetch(&client.session)
        {
            if let Some(row) = v["developerProducts"].as_array().and_then(|a| a.first()) {
                println!("public developer product record: {}", row);
                if let Some(id) = row["ProductId"].as_u64() {
                    let product = Read::Product {
                        id,
                        kind: "Product".into(),
                    }
                    .fetch(&client.session)
                    .unwrap();
                    println!("public product detail: {}", product);
                }
            }
        }
        let bundle = Read::Product {
            id: 1,
            kind: "Bundle".into(),
        }
        .fetch(&client.session)
        .unwrap();
        println!("public bundle record: {}", bundle);
        println!(
            "public gamepass batch: {}",
            Read::Product {
                id: 1,
                kind: "GamePass".into()
            }
            .fetch(&client.session)
            .unwrap()
        );
        for read in [
            Read::Product {
                id: 1818,
                kind: "Asset".into(),
            },
            Read::Badge(2124422733),
            Read::Group(1200769),
            Read::Users(vec![1]),
            Read::Policy,
            Read::Party { id: "00000000-0000-0000-0000-000000000000".into(), universe: 10000000007, place: 10000000000008, job: String::new() },
            Read::RankProducts(json!({"universeId":10000000007_u64,"productIdentifiers":[{"id":1286521615,"type":"Product"}]})),
            Read::TopProducts(json!({"universeId":10000000007_u64,"productInfoTypes":["Product","GamePass"]})),
            Read::CampaignEligibility("0".into()),
            Read::CommerceProduct("COM-0".into()),
            Read::RobloxSubscriptionProducts(client.user_id()),
            Read::RecommendationList(json!({"experienceId":10000000007_u64,"configName":"0","locationId":"0","paginationToken":{"count":10},"realtimeRenderContext":[{"key":"UserId","value":client.user_id().to_string()}]})),
            Read::RecommendationItem("0".into()),
            Read::PlayerPlace {
                place: 10000000000008,
                user: 1,
            },
            Read::BrandDecision("0".into()),
            Read::PriceLevels {
                universe: 10000000007,
                users: vec![client.user_id()],
            },
            Read::SubscriptionDetails {
                user: client.user_id(),
                subscription: "EXP-1".into(),
            },
            Read::SubscriptionStatus {
                user: client.user_id(),
                subscription: "EXP-1".into(),
            },
            Read::UpcomingEvents(10000000007),
            Read::Groups(1),
            Read::Relationships {
                group: 1200769,
                allies: true,
                page: 1,
            },
            Read::PassOwnership { user: 1, pass: 1 },
            Read::SubscriptionProduct("EXP-1".into()),
            Read::DeveloperProducts {
                universe: 10000000007,
                cursor: String::new(),
            },
        ] {
            match read.fetch(&client.session) {
                Ok(v) => {
                    if matches!(read, Read::RankProducts(_) | Read::TopProducts(_)) { println!("ranking read result: {}",v); }
                    if matches!(
                        read,
                        Read::BrandDecision(_)
                            | Read::PriceLevels { .. }
                            | Read::SubscriptionDetails { .. }
                            | Read::SubscriptionStatus { .. }
                            | Read::SubscriptionProduct(_)
                            | Read::Badge(_)
                    ) {
                        println!(
                            "extra read schema/value: {}",
                            if matches!(read, Read::Badge(_)) {
                                v["statistics"].clone()
                            } else {
                                v.clone()
                            }
                        );
                    }
                    println!(
                        "{}: read succeeded; keys {:?}",
                        read.request().unwrap().host,
                        v.as_object().map(|o| o.keys().collect::<Vec<_>>())
                    )
                }
                Err(e) => println!("{}: {e}", read.request().unwrap().host),
            }
        }
    }
}
