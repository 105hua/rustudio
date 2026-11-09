//! Studio purchase state and client/server transport. The only web calls here
//! obtain metadata. Confirmation never calls a purchase HTTP endpoint.
//! Bytecode: PurchasePrompt.{Thunks.completeRequest,Thunks.completePurchase,
//! Utils.isMockingPurchases,Network.getBalanceInfo} in DataModelPatch.rbxm.
use super::*;
struct HandlerConnection {
    connected: bool,
    disconnect: Option<mlua::Function>,
}
impl mlua::UserData for HandlerConnection {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field("__type", "RBXScriptConnection");
        fields.add_field_method_get("Connected", |_, this| Ok(this.connected));
    }
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, _, ()| Ok("Connection"));
        for name in ["Disconnect", "disconnect"] {
            methods.add_method_mut(name, |_, this, ()| {
                this.connected = false;
                if let Some(f) = this.disconnect.take() {
                    f.call::<()>(())?;
                }
                Ok(())
            });
        }
    }
}
pub(super) fn connection(lua: &Lua, disconnect: mlua::Function) -> mlua::Result<mlua::AnyUserData> {
    lua.create_userdata(HandlerConnection {
        connected: true,
        disconnect: Some(disconnect),
    })
}
const MARKER: &[u8] = b"\x01rustudio.web_social";
pub fn is_engine_msg(args: &[Xfer]) -> bool {
    matches!(args.first(),Some(Xfer::Str(s)) if s==MARKER)
}
pub fn engine_message(
    rt: &Runtime,
    inst: Ref,
    player: Ref,
    args: &[Xfer],
    map: &dyn Fn(Ref) -> Option<Ref>,
) {
    let lua = &rt.lua;
    let run = || -> mlua::Result<()> {
        let teleport = matches!(args.get(1),Some(Xfer::Str(s)) if s==b"__teleport");
        let social = matches!(args.get(1),Some(Xfer::Str(s)) if s==b"__social");
        let cancel = matches!(args.get(1),Some(Xfer::Str(s)) if s==b"__cancel");
        let shop = matches!(args.get(1),Some(Xfer::Str(s)) if s==b"__shop");
        let module: Table = lua.named_registry_value(if shop {
            "web_social.shop"
        } else if cancel {
            "web_social.cancel_subscription"
        } else if social {
            "web_social.social_prompts"
        } else if teleport {
            "web_social.teleport_module"
        } else {
            "web_social.purchase"
        })?;
        let f: mlua::Function = module.get("receive")?;
        let mut values = vec![inst_value(lua, inst)?, inst_value(lua, player)?];
        for arg in args.iter().skip(if teleport || social || cancel || shop { 2 } else { 1 }) {
            values.push(import(lua, arg, map)?)
        }
        f.call::<()>(MultiValue::from_vec(values))
    };
    if let Err(e) = run() {
        if let Ok(mut w) = world(lua) {
            w.output.push((
                Level::Error,
                format!("MarketplaceService: {}", error_message(&e)),
            ))
        }
    }
    rt.dispatch();
}
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let ctx = lua.create_table()?;
    ctx.set(
        "connection",
        lua.create_function(|lua, disconnect: mlua::Function| connection(lua, disconnect))?,
    )?;
    ctx.set(
        "receipt_name",
        lua.create_function(|_, v: Value| {
            Ok(crate::ltypes::enum_arg(Some(&v), "ReceiptType", "DeveloperProduct")?.name)
        })?,
    )?;
    ctx.set(
        "owned",
        lua.create_function(|lua, (kind, user, id): (String, u64, u64)| {
            if kind == "GamePass" || kind == "Bundle" {
                state(lua)?.raw_set(format!("owned:{kind}:{user}:{id}"), true)?;
            }
            Ok(())
        })?,
    )?;
    ctx.set(
        "server",
        lua.create_function(|lua, ()| Ok(world(lua)?.side == Side::Server))?,
    )?;
    ctx.set(
        "peer",
        lua.create_function(|lua, ()| Ok(world(lua)?.has_peer))?,
    )?;
    ctx.set(
        "isplayer",
        lua.create_function(|lua, v: Value| {
            Ok(opt_inst(&v)
                .and_then(|r| inst_meta(lua, r).ok())
                .is_some_and(|(c, _, _)| crate::doc::is_a(&c, "Player")))
        })?,
    )?;
    ctx.set(
        "send",
        lua.create_function(|lua, args: MultiValue| {
            let _inst = args
                .front()
                .and_then(opt_inst)
                .ok_or_else(|| rt_err("Instance expected"))?;
            let mut xs = vec![Xfer::Str(MARKER.to_vec())];
            xs.extend(args.iter().skip(1).map(|v| export(v, 0)));
            let inst = world(lua)?.d().dom.root_ref();
            world(lua)?.outbox.push(RemoteMsg {
                inst,
                kind: RemoteKind::Event,
                args: xs,
            });
            Ok(())
        })?,
    )?;
    ctx.set(
        "fire",
        lua.create_function(|lua, args: MultiValue| {
            let inst = args
                .front()
                .and_then(opt_inst)
                .ok_or_else(|| rt_err("Instance expected"))?;
            let Value::String(name) = arg(&args, 1) else {
                return Err(rt_err("Event expected"));
            };
            world(lua)?.fire(
                inst,
                &name.to_str()?,
                args.iter().skip(2).cloned().collect(),
            );
            Ok(())
        })?,
    )?;
    ctx.set("mock_subscription",lua.create_function(|lua,value:Value| {
        let player=opt_inst(&value).ok_or_else(||rt_err("Player expected"))?;
        // Native mock signal handler  calls the HasRobloxSubscription
        // setter ; descriptor binding is at .
        store_prop(lua,player,"HasRobloxSubscription",Variant::Bool(true))
    })?)?;
    ctx.set("subscription_begin",lua.create_function(|lua,()| {
        http::background(lua,|session,scope| {
            rustudio_account::web_social::subscription_prompt_products(&session,&scope).map_err(|e|e.to_string())
        })
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    lua.globals()
        .set("__ws_send", ctx.get::<mlua::Function>("send")?)?;
    let module: Table = lua
        .load(SOURCE)
        .set_name("=rustudio.web_social.purchase")
        .call(ctx)?;
    let methods: Table = module.get("methods")?;
    let target: Table = lua.globals().get("__rs_lua_methods")?;
    for entry in methods.pairs::<String, mlua::Function>() {
        let (k, v) = entry?;
        target.set(k, v)?
    }
    lua.set_named_registry_value("web_social.purchase", module)
}

pub const SOURCE: &str = r#"
local ctx=...
local pending={};local client={};local queue={};local active;local waiting={}
local module={methods={}};local bound_handlers={}
local names={PromptPurchase='Asset',PromptGamePassPurchase='GamePass',PromptProductPurchase='Product',PromptBundlePurchase='Bundle',PromptSubscriptionPurchase='Subscription',PromptPremiumPurchase='RobloxSubscription',PromptRobloxSubscriptionPurchase='RobloxSubscription'}
local events={Asset='PromptPurchaseFinished',GamePass='PromptGamePassPurchaseFinished',Product='PromptProductPurchaseFinished',Bundle='PromptBundlePurchaseFinished',Subscription='PromptSubscriptionPurchaseFinished'}
local function guid() return ctx.guid and ctx.guid() or game:GetService('HttpService'):GenerateGUID(false) end
local function schedule(f) if ctx.schedule then ctx.schedule(f) else task.spawn(f) end end
local function fire(request,purchased)
    if request.kind=='RobloxSubscription' then
        ctx.fire(request.service,'PromptRobloxSubscriptionPurchaseFinished',request.player,purchased);return
    end
    if purchased and ctx.owned then ctx.owned(request.kind,request.player.UserId,request.id) end
    local who=request.kind=='Product' and request.player.UserId or request.player
    ctx.fire(request.service,events[request.kind],who,request.id,purchased)
end
local function receipt(request)
    if request.kind~='Product' or request.receipt then return end
    request.receipt={PurchaseId=guid(),PlayerId=request.player.UserId,ProductId=request.id,
        PlaceIdWherePurchased=ctx.place and ctx.place() or game.PlaceId,
        CurrencySpent=request.price or 0,CurrencyType=ctx.robux or Enum.CurrencyType.Robux,
        ReceiptType=ctx.receipt_type or Enum.ReceiptType.DeveloperProduct}
    waiting[request.receipt.PurchaseId]={request=request,running=false}
    for id,entry in waiting do
        local bindings=bound_handlers.DeveloperProduct
        local bound=bindings and (bindings.products[entry.request.id] or bindings.catchall)
        local callback
        if bound then callback=bound.callback
        elseif ctx.callback then callback=ctx.callback(entry.request.service)
        else callback=__rs_get_callback(entry.request.service,'ProcessReceipt') end
        if callback then
            if entry.request.player.UserId==request.player.UserId and not entry.running then
                entry.running=true
                schedule(function()
                    local results=table.pack(pcall(callback,entry.request.receipt));entry.running=false
                    if not results[1] then error(results[2],0) end
                    local decision=results[2]
                    if bound then
                        if results.n~=2 then error(string.format('BindReceiptHandler callback must return a single Enum.ReceiptDecision value, got %d return values',results.n-1),0) end
                        local processed=ctx.processed or Enum.ReceiptDecision.Processed
                        local not_processed=ctx.not_processed or Enum.ReceiptDecision.NotProcessedYet
                        if decision~=processed and decision~=not_processed then error('BindReceiptHandler callback must return an Enum.ReceiptDecision value',0) end
                        if decision==processed then waiting[id]=nil end
                    elseif decision==(ctx.granted or Enum.ProductPurchaseDecision.PurchaseGranted) then waiting[id]=nil end
                end)
            end
        end
    end
end
local function complete_server(token,player,purchased)
    local request=pending[token]
    if not request or request.player~=player then return end
    pending[token]=nil
    fire(request,purchased and request.purchased==true)
end
function module.close(token)
    local request=client[token]
    if not request or request.closed then return end
    request.closed=true
    if request.gui then request.gui:Destroy() end
    client[token]=nil;active=nil
    local purchased=request.purchased==true
    if not request.signaled then
        if ctx.server() and not ctx.peer() then complete_server(token,request.player,purchased)
        else fire(request,purchased);if ctx.peer() then ctx.send(request.service,'complete',token,purchased) end end
    end
    module.pump()
end
function module.confirm(token)
    local request=client[token]
    if not request or request.closed or request.purchased or request.error then return end
    request.purchased=true
    if ctx.peer() and not ctx.server() then ctx.send(request.service,'purchase',token,request.price)
    else
        local server=pending[token];if server then server.purchased=true;receipt(server) end
        if request.kind=='RobloxSubscription' and ctx.mock_subscription then ctx.mock_subscription(request.player) end
    end
    if request.kind=='RobloxSubscription' then
        -- completeRobloxSubscriptionUpsellFlow signals first, then waits for
        -- the mock server property replication (measured Client timeout: eight seconds).
        request.signaled=true
        if ctx.server() and not ctx.peer() then complete_server(token,request.player,true)
        else fire(request,true);if ctx.peer() then ctx.send(request.service,'complete',token,true) end end
        schedule(function()
            local start=os.clock()
            while not request.player.HasRobloxSubscription and os.clock()-start<8 do task.wait() end
            module.close(token)
        end)
        return
    end
    if ctx.present then return ctx.present(request,'complete',function()module.close(token)end) end
    local started=os.clock()
    if request.label then request.label.Text='Loading...' end
    if request.buy then request.buy.Visible=false end
    if request.cancel then request.cancel.Visible=false end
    task.delay(math.max(0,1-(os.clock()-started)),function()
        if request.closed then return end
        request.label.Text='Your purchase of '..(request.name or tostring(request.id))..' succeeded!'
        request.cancel.Text='OK';request.cancel.Visible=true
    end)
end
-- RobloxSubscriptionPurchaseApp: direct REST product selection is the active
-- online Client path. The first valid period product is selected; an empty list
-- shows AlreadySubscribed. Payment-session failures are swallowed by Network.
local function subscription_info()
    if ctx.metadata then return ctx.metadata('RobloxSubscription') end
    local token=ctx.subscription_begin()
    while true do
        local done,body,err=ctx.poll(token)
        if done then
            if err then error(err,0) end
            return game:GetService('HttpService'):JSONDecode(body)
        end
        task.wait()
    end
end
local function render_subscription(request)
    local gui=Instance.new('ScreenGui');gui.Name='RobloxSubscriptionPurchaseOverlay';gui.ResetOnSpawn=false;gui.DisplayOrder=1000
    local frame=Instance.new('Frame');frame.Name='Sheet';frame.Size=UDim2.fromOffset(440,420);frame.Position=UDim2.new(.5,-220,.5,-210);frame.BackgroundColor3=Color3.fromRGB(35,37,39);frame.Parent=gui
    local function label(name,text,y,h,size)
        local l=Instance.new('TextLabel');l.Name=name;l.BackgroundTransparency=1;l.Position=UDim2.fromOffset(20,y);l.Size=UDim2.new(1,-40,0,h);l.Text=text;l.TextSize=size or 18;l.TextWrapped=true;l.TextColor3=Color3.new(1,1,1);l.Parent=frame;return l
    end
    label('Title',request.error and (request.already and 'Good news' or 'Something went wrong') or 'Get Roblox Plus',16,44,24)
    if request.error then
        label('Message',request.already and 'You are already subscribed to Plus.' or "We couldn't complete this right now. Please try again.",80,110)
    else
        label('Features','Discounts on items and avatars\nCustomize with profile frames and app themes\nFree private servers\nSend Robux for free',76,148)
        label('Price',request.display_price or '',228,40,22)
        label('LegalDisclosure',request.trial and ('By clicking “Try it for free,” you agree to the Roblox Subscription Terms. Your free trial will end in '..request.trial..' and you will be charged automatically each month and can cancel at any time.') or 'By subscribing, you agree to the Roblox Subscription Terms. You will be charged automatically each month and can cancel at any time.',276,66,14)
    end
    local close=Instance.new('TextButton');close.Name='CloseAffordance';close.Text='×';close.Size=UDim2.fromOffset(32,32);close.Position=UDim2.new(1,-40,0,8);close.Parent=frame
    close.Activated:Connect(function()module.close(request.token)end)
    local buy=Instance.new('TextButton');buy.Name=request.error and 'OK' or 'Subscribe';buy.Modal=true;buy.Size=UDim2.new(1,-40,0,42);buy.Position=UDim2.fromOffset(20,358);buy.Text=request.error and 'OK' or request.trial and 'Try it for free' or 'Subscribe';buy.TextSize=18;buy.Parent=frame
    buy.Activated:Connect(function()if request.error then module.close(request.token)else module.confirm(request.token)end end)
    gui.Parent=request.player:FindFirstChild('PlayerGui') or game:GetService('CoreGui');request.gui=gui
end
local function render(request)
    if ctx.present then return ctx.present(request,'prompt',function()module.close(request.token)end,function()module.confirm(request.token)end) end
    if request.kind=='RobloxSubscription' then return render_subscription(request) end
    local gui=Instance.new('ScreenGui');gui.Name='PurchasePrompt';gui.ResetOnSpawn=false;gui.DisplayOrder=1000
    local frame=Instance.new('Frame');frame.Name='ProductPurchasePrompt';frame.Size=UDim2.fromOffset(440,280);frame.Position=UDim2.new(.5,-220,.5,-140);frame.BackgroundColor3=Color3.fromRGB(35,37,39);frame.Parent=gui
    local title=Instance.new('TextLabel');title.Name='Title';title.Size=UDim2.new(1,-32,0,40);title.Position=UDim2.fromOffset(16,12);title.BackgroundTransparency=1;title.TextColor3=Color3.new(1,1,1);title.TextSize=22;title.Text=request.error and 'Error' or 'Buy item';title.Parent=frame
    local label=Instance.new('TextLabel');label.Name='Message';label.Size=UDim2.new(1,-32,0,92);label.Position=UDim2.fromOffset(16,58);label.BackgroundTransparency=1;label.TextColor3=Color3.new(1,1,1);label.TextSize=18;label.TextWrapped=true;label.Text=request.error and (request.message or 'Your purchase failed because something went wrong. Your account has not been charged. Please try again later.') or ('Would you like to buy "'..request.name..'"?');label.Parent=frame
    local disclaimer=Instance.new('TextLabel');disclaimer.Name='TestPurchaseText';disclaimer.Size=UDim2.new(1,-32,0,42);disclaimer.Position=UDim2.fromOffset(16,152);disclaimer.BackgroundTransparency=1;disclaimer.TextColor3=Color3.fromRGB(190,190,190);disclaimer.TextSize=14;disclaimer.TextWrapped=true;disclaimer.Text='This is a test purchase. Your account will not be charged.';disclaimer.Visible=not request.error;disclaimer.Parent=frame
    local cancel=Instance.new('TextButton');cancel.Name='Cancel';cancel.Modal=true;cancel.Size=UDim2.fromOffset(192,40);cancel.Position=UDim2.fromOffset(16,220);cancel.Text=request.error and 'OK' or 'Cancel';cancel.TextSize=18;cancel.Parent=frame
    local buy=Instance.new('TextButton');buy.Name='Buy';buy.Modal=true;buy.Size=UDim2.fromOffset(192,40);buy.Position=UDim2.fromOffset(232,220);buy.Text='Buy';buy.TextSize=18;buy.Visible=not request.error;buy.Parent=frame
    local close=Instance.new('TextButton');close.Name='CloseAffordance';close.Text='×';close.Size=UDim2.fromOffset(32,32);close.Position=UDim2.new(1,-40,0,8);close.Parent=frame
    close.Activated:Connect(function()module.close(request.token)end)
    cancel.Activated:Connect(function()module.close(request.token)end)
    buy.Activated:Connect(function()module.confirm(request.token)end)
    -- rustudio's renderer currently draws PlayerGui. The modal is engine-owned
    -- and ResetOnSpawn=false, matching the CoreGui prompt lifetime.
    gui.Parent=request.player:FindFirstChild('PlayerGui') or game:GetService('CoreGui')
    request.gui=gui;request.label=label;request.buy=buy;request.cancel=cancel
end
function module.pump()
    if active or #queue==0 then return end
    local request=table.remove(queue,1);active=request.token;client[request.token]=request
    schedule(function()
        local ok,info=pcall(function()
            if request.kind=='RobloxSubscription' then return subscription_info() end
            if ctx.metadata then return ctx.metadata(request.kind,request.id) end
            local m=game:GetService('MarketplaceService')
            if request.kind=='Subscription' then return m:GetSubscriptionProductInfoAsync(request.id) end
            local info=request.kind=='Bundle' and __ws_bundle_details(request.id) or m:GetProductInfoAsync(request.id,Enum.InfoType[request.kind])
            if request.kind=='Asset' then info.__owned=m:PlayerOwnsAssetAsync(request.player,request.id)
            elseif request.kind=='Bundle' then info.__owned=m:PlayerOwnsBundleAsync(request.player,request.id)
            elseif request.kind=='GamePass' then info.__owned=m:UserOwnsGamePassAsync(request.player.UserId,request.id) end
            return info
        end)
        if request.closed then return end
        request.error=not ok
        if ok and request.kind=='RobloxSubscription' then
            if type(info)~='table' then request.error=true;info={} end
            local product
            for _,p in type(info.products)=='table' and info.products or {} do
                if type(p)=='table' and type(p.periodType)=='string' and p.periodType~='' then product=p;break end
            end
            if not product then request.already=not request.error;request.error=true
            else
                request.id=tostring(product.productKey and product.productKey.id or '')
                local period=string.lower(product.periodType)
                request.display_price=tostring(product.localizedPriceDisplayString or '')..'/'..period
                for _,offer in type(product.eligibleOffers)=='table' and product.eligibleOffers or {} do
                    if offer.offerType=='FreeTrial' and type(offer.freeTrialOffer)=='table' then
                        local trial=offer.freeTrialOffer;local duration=tonumber(trial.duration) or 0
                        local unit=string.lower(tostring(trial.periodType or ''))
                        request.trial=tostring(duration)..' '..unit..(duration==1 and '' or 's')
                        request.display_price=request.trial..' free, then '..request.display_price;break
                    end
                end
            end
        elseif ok then
            request.name=info.Name or info.name or tostring(request.id)
            request.price=info.PriceInRobux or info.priceInRobux or info.price or (info.product and info.product.priceInRobux) or 0
            if info.__owned then request.error=true;request.message='You already own this item. Your account has not been charged.'
            elseif info.IsForSale==false or info.isForSale==false or (info.product and info.product.isForSale==false) then request.error=true;request.message='This item is not currently for sale. Your account has not been charged.' end
            if pending[request.token] then pending[request.token].price=request.price end
        end
        render(request)
    end)
end
local function local_prompt(service,token,player,kind,id)
    table.insert(queue,{service=service,token=token,player=player,kind=kind,id=id})
    module.pump()
end
function module.receive(service,sender,verb,token,a,b,c)
    service=ctx.service and ctx.service() or game:GetService('MarketplaceService')
    if verb=='prompt' and not ctx.server() then return local_prompt(service,token,a,b,c) end
    if not ctx.server() then return end
    local request=pending[token]
    if not request or request.player~=sender then return end
    if verb=='purchase' and not request.purchased then
        request.purchased=true;request.price=tonumber(a) or 0;receipt(request)
        if request.kind=='RobloxSubscription' and ctx.mock_subscription then ctx.mock_subscription(request.player) end
    elseif verb=='complete' then complete_server(token,sender,a) end
end
function module.methods.BindReceiptHandler(service,kind,callback,filter)
    if kind==nil then error('Argument 1 missing or nil',2) end
    if type(callback)~='function' then error('Argument 2 missing or nil',2) end
    if not ctx.server() then error('BindReceiptHandler can only be called from the server',0) end
    local name=ctx.receipt_name and ctx.receipt_name(kind) or kind.Name
    if name~='DeveloperProduct' and name~='RobuxTransferSender' and name~='RobuxTransferReceiver' then error('Unable to cast value to ReceiptType',0) end
    if filter~=nil and type(filter)~='table' then error('Unable to cast value to Array',0) end
    local ids={}
    for _,id in filter or {} do
        local n=tonumber(id);if not n then error('Unable to cast '..type(id)..' to int64',0) end
        ids[math.floor(n)]=true
    end
    if name~='DeveloperProduct' and next(ids) then error('BindReceiptHandler: filters are not supported for RobuxTransfer receipt types',0) end
    local bindings=bound_handlers[name] or {products={}};bound_handlers[name]=bindings
    local catchall=next(ids)==nil
    if catchall and bindings.catchall then error('BindReceiptHandler: a catch-all handler is already registered for this ReceiptType',0) end
    for id in ids do if bindings.products[id] then error('BindReceiptHandler: a handler is already registered for this ReceiptType and ProductId',0) end end
    local entry={callback=callback}
    if catchall then bindings.catchall=entry else for id in ids do bindings.products[id]=entry end end
    return ctx.connection(function()
        if bindings.catchall==entry then bindings.catchall=nil end
        for id in ids do if bindings.products[id]==entry then bindings.products[id]=nil end end
    end)
end
for method,kind in names do
    module.methods[method]=function(service,player,id)
        if not ctx.isplayer(player) then
            if method=='PromptPremiumPurchase' and player~=nil then error('First argument to PromptPremiumPurchase must be a Player',0) end
            error('Argument 1 missing or nil',2)
        end
        if kind=='RobloxSubscription' then
            if method=='PromptRobloxSubscriptionPurchase' and player.UserId<1 then
                (ctx.warn or warn)('MarketplaceService:promptRobloxSubscriptionPurchase() was called on a guest, please use this call only on players with an account.');return
            end
            local islocal
            if ctx.islocal then islocal=ctx.islocal(player) else islocal=ctx.server() or player==game:GetService('Players').LocalPlayer end
            if not ctx.server() and not islocal then
                if method=='PromptPremiumPurchase' then error('Can only call PromptPremiumPurchase from LocalScript on the local player',0) end
                error('Can only call PromptRobloxSubscriptionPurchase from LocalScript on the local player',0)
            end
        elseif kind=='Subscription' then
            if type(id)~='string' then error('Argument 2 missing or nil',2) end
        else
            id=tonumber(id)
            if not id then error('Argument 2 missing or nil',2) end
            if id<=0 then
                if kind=='GamePass' then error('MarketplaceService::PromptGamePassPurchase() was called with an invalid game pass id (supplied game pass id was less than zero)',0)
                elseif kind=='Bundle' then error('MarketplaceService:PromptBundlePurchase: second argument is not a valid bundleId (supplied bundleId was less than 0)',0)
                else error('MarketplaceService:PromptPurchase() second argument is not a valid '..(kind=='Product' and 'productId' or 'assetId')..' (supplied '..(kind=='Product' and 'productId' or 'assetId')..' was less than 0)',0) end
            end
        end
        local token=guid()
        if ctx.server() then
            pending[token]={service=service,player=player,kind=kind,id=id}
            if ctx.peer() then ctx.send(service,'prompt',token,player,kind,id);return end
        end
        local_prompt(service,token,player,kind,id)
    end
end
return module
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn premium_routes_to_plus_and_mock_completion_has_native_arguments() -> mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local log={};local seq=0;local player={UserId=123};local server=false;local current
local products={products={{periodType='Month',localizedPriceDisplayString='$4.99',productKey={id='PLUS-1'},eligibleOffers={{offerType='FreeTrial',freeTrialOffer={duration=1,periodType='Week'}}}}}}
local ctx={server=function()return server end,peer=function()return false end,isplayer=function(p)return p==player end,islocal=function(p)return p==player end,
 guid=function()seq+=1;return tostring(seq)end,schedule=function(f)f()end,
 metadata=function(kind)assert(kind=='RobloxSubscription');return products end,
 mock_subscription=function(p)p.HasRobloxSubscription=true end,
 present=function(request,phase,close,confirm)current={request=request,close=close,confirm=confirm};assert(phase=='prompt')end,
 fire=function(_,name,...)table.insert(log,{name=name,args=table.pack(...)})end}
local module=assert(loadstring(source))(ctx)
module.methods.PromptPremiumPurchase({},player)
assert(current.request.kind=='RobloxSubscription' and current.request.id=='PLUS-1')
assert(current.request.trial=='1 week' and current.request.display_price=='1 week free, then $4.99/month')
current.close();current.close()
assert(#log==1 and log[1].name=='PromptRobloxSubscriptionPurchaseFinished')
assert(log[1].args.n==2 and log[1].args[1]==player and log[1].args[2]==false)
-- The active Studio mock flag takes an immediate local completion path. This
-- is exercised only in the mock VM; no native offer was ever accepted.
module.methods.PromptRobloxSubscriptionPurchase({},player)
current.confirm();current.close()
assert(#log==2 and log[2].args.n==2 and log[2].args[2]==true)
products={products={}};module.methods.PromptPremiumPurchase({},player)
assert(current.request.error and current.request.already);current.close()
assert(#log==3 and log[3].args[2]==false)
server=true;products={products={{periodType='Month',productKey={id='PLUS-2'}}}}
module.methods.PromptPremiumPurchase({},player);current.confirm()
assert(#log==4 and log[4].args[1]==player and log[4].args[2]==true)
"#).exec()
    }
    #[test]
    fn bound_receipts_filter_prioritize_and_disconnect() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.globals().set("purchase_source", SOURCE)?;
        lua.load(r#"
local log={};local seq=0;local player={UserId=123};local service={}
local ctx={server=function()return true end,peer=function()return false end,isplayer=function()return true end,
 receipt_name=function(x)return x end,guid=function()seq+=1;return tostring(seq)end,place=function()return 10000000000008 end,
 robux='Robux',receipt_type='DeveloperProduct',granted='PurchaseGranted',processed='Processed',not_processed='NotProcessedYet',
 metadata=function()return {Name='Test',PriceInRobux=7}end,present=function()end,schedule=function(f)f()end,
 fire=function()end,callback=function()return function()error('legacy callback should not run')end end,
 connection=function(fn)local c={Connected=true};function c:Disconnect()if self.Connected then self.Connected=false;fn()end end;return c end}
local p=assert(loadstring(purchase_source))(ctx)
local exact=p.methods.BindReceiptHandler(service,'DeveloperProduct',function(r)table.insert(log,{'specific',r.ProductId});return 'Processed' end,{7})
p.methods.BindReceiptHandler(service,'DeveloperProduct',function(r)table.insert(log,{'catchall',r.ProductId});return 'Processed' end,{})
local ok,err=pcall(p.methods.BindReceiptHandler,service,'DeveloperProduct',function()end,{7});assert(not ok and err:find('a handler is already registered',1,true))
p.methods.PromptProductPurchase(service,player,7);p.confirm('1');p.close('1')
assert(#log==1 and log[1][1]=='specific')
exact:Disconnect();assert(not exact.Connected)
p.methods.PromptProductPurchase(service,player,7);p.confirm('3');p.close('3')
assert(#log==2 and log[2][1]=='catchall')
p.methods.BindReceiptHandler(service,'DeveloperProduct',function()return 'Processed'end,{7})
local ok,err=pcall(p.methods.BindReceiptHandler,service,'RobuxTransferSender',function()end,{1});assert(not ok and err:find('filters are not supported',1,true))
"#).exec()
    }
    #[test]
    fn receipt_connection_disconnects_once() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.load("calls=0").exec()?;
        let f: mlua::Function = lua.load("return function()calls+=1 end").eval()?;
        lua.globals().set(
            "connection",
            lua.create_userdata(HandlerConnection {
                connected: true,
                disconnect: Some(f),
            })?,
        )?;
        lua.load("assert(connection.Connected);connection:Disconnect();connection:Disconnect();assert(not connection.Connected and calls==1 and tostring(connection)=='Connection')").exec()
    }
    #[test]
    fn peer_purchase_orders_receipt_before_completion_and_rejects_wrong_sender() -> mlua::Result<()>
    {
        let lua = Lua::new();
        lua.globals().set("purchase_source", SOURCE)?;
        lua.load(r#"
local log={};local player={UserId=123};local impostor={UserId=456};local service={};local seq=0
local server,client
local function context(is_server)
 return {server=function()return is_server end,peer=function()return true end,isplayer=function(p)return p==player end,
 service=function()return service end,guid=function()seq+=1;return tostring(seq)end,place=function()return 10000000000008 end,robux='Robux',receipt_type='DeveloperProduct',granted='PurchaseGranted',
 metadata=function()return {Name='Test',PriceInRobux=7}end,present=function()end,schedule=function(f)f()end,
 fire=function(_,event,who,id,success)table.insert(log,{is_server and 'server' or 'client',event,who,id,success})end,
 callback=function()return function(r)table.insert(log,{'receipt',r})end end,
 send=function(_,...)if is_server then client.receive(service,nil,...)else server.receive(service,player,...)end end}
end
server=assert(loadstring(purchase_source))(context(true));client=assert(loadstring(purchase_source))(context(false))
server.methods.PromptProductPurchase(service,player,7)
server.receive(service,impostor,'purchase','1',7)
assert(#log==0)
client.confirm('1');client.confirm('1')
assert(#log==1 and log[1][1]=='receipt' and log[1][2].CurrencySpent==7)
client.close('1');client.close('1')
assert(#log==3 and log[2][1]=='client' and log[3][1]=='server')
assert(log[2][2]=='PromptProductPurchaseFinished' and log[2][3]==123 and log[2][5]==true)
"#).exec()
    }
    #[test]
    fn cancellation_is_once_and_product_receipt_precedes_finished() -> mlua::Result<()> {
        let lua = Lua::new();
        let ctx:Table=lua.load(r#"local log={};local n=0
return {server=function()return true end,peer=function()return false end,isplayer=function()return true end,
 guid=function()n+=1;return tostring(n)end,place=function()return 10000000000008 end,robux='Robux',receipt_type='DeveloperProduct',granted='PurchaseGranted',
 metadata=function()return {Name='Test product',PriceInRobux=7}end,present=function()end,
 schedule=function(f)f()end,fire=function(_,event,who,id,success)table.insert(log,{event,who,id,success})end,
 callback=function()return function(receipt)table.insert(log,{'receipt',receipt})end end,log=log}"#).eval()?;
        let module: Table = lua.load(SOURCE).call(ctx.clone())?;
        lua.globals().set("probe", module)?;
        lua.globals().set("ctx", ctx)?;
        lua.load(r#"local player={UserId=123};local service={}
probe.methods.PromptProductPurchase(service,player,7)
probe.close('1');probe.close('1')
assert(#ctx.log==1)
assert(ctx.log[1][1]=='PromptProductPurchaseFinished' and ctx.log[1][2]==123 and ctx.log[1][4]==false)
probe.methods.PromptProductPurchase(service,player,7)
probe.confirm('2');probe.confirm('2')
assert(ctx.log[2][1]=='receipt')
local r=ctx.log[2][2];assert(r.CurrencySpent==7 and r.ReceiptType=='DeveloperProduct');assert(r.ProductId==7 and r.PlayerId==123 and r.PlaceIdWherePurchased==10000000000008 and r.CurrencyType=='Robux')
probe.close('2')
assert(ctx.log[3][1]=='PromptProductPurchaseFinished' and ctx.log[3][4]==true)
"#).exec()
    }
}
