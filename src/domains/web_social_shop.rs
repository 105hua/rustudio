//! OpenShop, ShopEntrypoint and InExperienceShop.
//! Current Studio blocks Robux tile clicks; launchRobuxPackagePurchase also
//! returns before any payment request when IsStudio(). Virtual items use the
//! existing Studio test-purchase controller. Development never accepts a prompt.
use super::*;
pub fn install(lua:&Lua)->mlua::Result<()> {
    let ctx=lua.create_table()?;
    ctx.set("server",lua.create_function(|lua,()|Ok(world(lua)?.side==Side::Server))?)?;
    ctx.set("peer",lua.create_function(|lua,()|Ok(world(lua)?.has_peer))?)?;
    ctx.set("log",lua.create_function(|lua,text:String|{world(lua)?.output.push((Level::Warn,text));Ok(())})?)?;
    ctx.set("begin",lua.create_function(|lua,(universe,key):(u64,String)| {
        http::background(lua,move|session,scope|rustudio_account::web_social::Read::ShopWidgets{universe,request:key}.fetch_in(&session,&scope).map_err(|e|e.to_string()))
    })?)?;
    ctx.set("poll",lua.globals().get::<mlua::Function>("__ws_poll")?)?;
    let module:Table=lua.load(SOURCE).set_name("=rustudio.web_social.shop").call(ctx)?;
    lua.globals().get::<Table>("__rs_lua_methods")?.set("OpenShop",module.get::<mlua::Function>("open")?)?;
    lua.set_named_registry_value("web_social.shop",module)
}
pub fn step(lua:&Lua)->mlua::Result<()> {
    if world(lua)?.side!=Side::Client{return Ok(())}
    let module:Table=lua.named_registry_value("web_social.shop")?;
    module.get::<mlua::Function>("boot")?.call(())
}
const SOURCE:&str=r#"
local ctx=...;local module={};local active;local loaded=false;local loading=false;local booted=false
local data={categories={}};local categories={};local items={};local selected='home';local query='';local search_serial=0
local function spawn(f)if ctx.spawn then return ctx.spawn(f)end;task.spawn(f)end
local function wait()if ctx.wait then return ctx.wait()end;return task.wait()end
local function player()return ctx.player and ctx.player()or game:GetService('Players').LocalPlayer end
local function universe()return ctx.universe and ctx.universe()or game.GameId end
local function isplayer(p)return(ctx.isplayer and ctx.isplayer(p))or(typeof(p)=='Instance'and p:IsA('Player'))end
local function request()
    if universe()==0 then return{categories={}}end -- measured SkipShopWidgetsCallInUnpublishedStudio
    local key=ctx.guid and ctx.guid()or game:GetService('HttpService'):GenerateGUID(false)
    if ctx.request then return ctx.request(universe(),key)end
    local token=ctx.begin(universe(),key)
    while true do local done,body,err=ctx.poll(token);if done then if err then error(err,0)end;return game:GetService('HttpService'):JSONDecode(body)end;wait()end
end
local function metadata(id,kind)
    if ctx.metadata then return ctx.metadata(id,kind)end
    return game:GetService('MarketplaceService'):GetProductInfoAsync(tonumber(id),Enum.InfoType[kind])
end
local function owns(id)
    if ctx.owns then return ctx.owns(id)end
    return game:GetService('MarketplaceService'):UserOwnsGamePassAsync(player().UserId,tonumber(id))
end
function module.visible_items()
    local out={};local seen={};local per_category={}
    local ordered=table.clone(items);table.sort(ordered,function(a,b)return a.order<b.order end)
    for _,item in ordered do
        local key=item.kind..':'..item.id
        local match=query~=''and item.kind~='Robux'and string.find(string.lower(item.name),string.lower(query),1,true)~=nil
        if query==''then match=selected=='home'or selected==item.category end
        if match and(query==''or not seen[key])then
            local count=per_category[item.category]or 0
            if query~=''or selected~='home'or count<6 then table.insert(out,item);seen[key]=true;per_category[item.category]=count+1 end
            if query~=''and #out>=50 then break end
        end
    end
    return out
end
local render
function module.close()
    if active then if active.gui then active.gui:Destroy()end;if ctx.destroy then ctx.destroy(active)end;active=nil end
end
function module.select(category)selected=category or'home';query='';if active then render()end end
function module.search(text)
    query=tostring(text or'');if active then render()end
end
function module.buy(item)
    if not active or not item or item.owned then return end
    -- Measured BlockRobuxTileClickInShop=true. The shared launch function's
    -- independent IsStudio guard also returns nil without payment/bonus POST.
    if item.kind=='Robux'then return end
    if ctx.purchase then return ctx.purchase(item.kind,tonumber(item.id),active.player)end
    local marketplace=game:GetService('MarketplaceService')
    if item.kind=='GamePass'then marketplace:PromptGamePassPurchase(active.player,tonumber(item.id))
    elseif item.kind=='Product'then marketplace:PromptProductPurchase(active.player,tonumber(item.id))end
end
render=function()
    local r=active;if not r then return end
    if ctx.present then return ctx.present(r,categories,module.visible_items(),module.close,module.buy)end
    if not r.gui then
        local gui=Instance.new('ScreenGui');gui.Name='InExperienceShop';gui.ResetOnSpawn=false;gui.DisplayOrder=1000;r.gui=gui
        local f=Instance.new('Frame');f.Name='Window';f.Size=UDim2.fromOffset(560,430);f.Position=UDim2.new(.5,-280,.5,-215);f.BackgroundColor3=Color3.fromRGB(35,37,39);f.Parent=gui
        local title=Instance.new('TextLabel');title.Name='Title';title.Text='Shop';title.TextSize=24;title.TextColor3=Color3.new(1,1,1);title.BackgroundTransparency=1;title.Size=UDim2.fromOffset(260,44);title.Position=UDim2.fromOffset(20,10);title.Parent=f
        local close=Instance.new('TextButton');close.Name='CloseAffordance';close.Text='×';close.Size=UDim2.fromOffset(32,32);close.Position=UDim2.new(1,-40,0,8);close.Activated:Connect(module.close);close.Parent=f
        local search=Instance.new('TextBox');search.Name='Search';search.PlaceholderText='Search';search.Text='';search.ClearTextOnFocus=false;search.TextSize=18;search.Size=UDim2.fromOffset(190,30);search.Position=UDim2.fromOffset(318,60);search.Parent=f
        search:GetPropertyChangedSignal('Text'):Connect(function()
            search_serial+=1;local serial=search_serial
            task.delay(.25,function()if serial==search_serial then module.search(search.Text)end end)
        end)
        local tabs=Instance.new('ScrollingFrame');tabs.Name='Categories';tabs.Position=UDim2.fromOffset(20,60);tabs.Size=UDim2.fromOffset(290,30);tabs.BackgroundTransparency=1;tabs.Parent=f;r.tabs=tabs
        local list=Instance.new('ScrollingFrame');list.Name='Items';list.Position=UDim2.fromOffset(20,106);list.Size=UDim2.fromOffset(520,304);list.BackgroundTransparency=1;list.Parent=f;r.list=list
        gui.Parent=r.player:FindFirstChild('PlayerGui')or game:GetService('CoreGui')
    end
    for _,v in r.tabs:GetChildren()do v:Destroy()end
    local all={{id='home',name='Home'}};for _,category in categories do table.insert(all,category)end
    if #categories>0 then for i,category in all do
        local b=Instance.new('TextButton');b.Name='Category'..i;b.Text=category.name;b.TextSize=16;b.Size=UDim2.fromOffset(100,28);b.Position=UDim2.fromOffset((i-1)*105,0);b.Activated:Connect(function()module.select(category.id)end);b.Parent=r.tabs
    end end
    r.tabs.CanvasSize=UDim2.fromOffset(#all*105,28)
    for _,v in r.list:GetChildren()do v:Destroy()end
    local visible=module.visible_items()
    for i,item in visible do
        local b=Instance.new('TextButton');b.Name='Item'..i;b.Text=item.name..'\n'..(item.owned and'Owned'or tostring(item.price or''));b.TextSize=18;b.TextWrapped=true;b.Size=UDim2.fromOffset(500,68);b.Position=UDim2.fromOffset(0,(i-1)*74);b.Activated:Connect(function()module.buy(item)end);b.Parent=r.list
    end
    r.list.CanvasSize=UDim2.fromOffset(500,#visible*74)
end
local function load()
    if loaded or loading then return end;loading=true
    spawn(function()
        local ok,value=pcall(request)
        if not ok or type(value)~='table'or type(value.categories)~='table'then loading=false;return end
        for _,category in value.categories do
            if type(category)~='table'or type(category.name)~='string'or type(category.items)~='table'then loading=false;return end
            for _,record in category.items do if type(record)~='table'then loading=false;return end end
        end
        data=value;categories={};items={}
        local pending=0;local sequence=0
        for _,category in value.categories do
            if type(category)=='table'and type(category.items)=='table'then
                local cat=tostring(category.id);local supported=false;table.insert(categories,{id=cat,name=tostring(category.name or'')})
                for _,record in category.items do
                    sequence+=1;local position=sequence
                    if record.type=='Robux'then
                        local info=record.info;local price=info and info.price;local currency=price and price.currency
                        if info and type(info.robux)=='number'and type(info.productId)=='number'and type(info.providerProductId)=='string'and price and type(price.amount)=='number'and currency and type(currency.currencySymbol)=='string'then
                            supported=true
                            table.insert(items,{id=tostring(record.id),kind='Robux',category=cat,name=tostring(info.robux)..' Robux',price=currency.currencySymbol..string.format('%.2f',price.amount),source=record,order=position})
                        end
                    elseif record.type=='Product'or record.type=='GamePass'then
                        supported=true
                        pending+=1
                        spawn(function()
                            local success,info=pcall(metadata,record.id,record.type)
                            if success and type(info)=='table'then
                                local id=record.type=='Product'and(info.ProductId or record.id)or record.id
                                local item={id=tostring(id),kind=record.type,category=cat,name=info.DisplayName or info.Name or'',price=info.PriceInRobux,description=info.Description,source=record,order=position}
                                if item.kind=='GamePass'then local ok,owned=pcall(owns,id);item.owned=ok and owned==true end
                                table.insert(items,item)
                            end
                            pending-=1;render()
                        end)
                    end
                end
                if not supported then table.remove(categories,#categories)end
            end
        end
        loaded=true;loading=false;render()
    end)
end
function module.boot()
    if booted or ctx.server()or not player()then return end
    booted=true
    local function purchased(p,id,was_purchased)
        if p~=player()or not was_purchased then return end
        local function refresh()
            local ok,value=pcall(owns,id)
            if ok and value then for _,item in items do if item.kind=='GamePass'and item.id==tostring(id)then item.owned=true end end;render()end
        end
        if ctx.delay then ctx.delay(.25,refresh)else task.delay(.25,refresh)end
    end
    if ctx.connect then ctx.connect(purchased)else game:GetService('MarketplaceService').PromptGamePassPurchaseFinished:Connect(purchased)end
    load() -- measured EnableShopPrefetch=true, delay0, retries0
end
local function show(p)
    if active then return end
    active={player=p};selected='home';query='';render();load()
end
function module.open(service,p)
    if p~=nil and not isplayer(p)then
        if typeof(p)~='Instance'then error('Unable to cast value to Object',0)end;p=nil
    end
    if not p then ctx.log('MarketplaceService:OpenShop: player is null.');return end
    if p.UserId<1 then ctx.log('MarketplaceService:OpenShop: cannot open shop for a guest player.');return end
    if not ctx.server()and p~=player()then ctx.log('MarketplaceService:OpenShop: called from a local script, but not called on a local player. Local scripts can only open the shop for the local player.');return end
    if ctx.server()then
        if ctx.peer()then if ctx.send then ctx.send(service,'__shop',p)else __ws_send(service,'__shop',p)end end
    else show(p)end
end
function module.receive(_,sender,p)
    if not ctx.server()and p==player()then show(p)end
end
return module
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shop_prefetch_catalog_cache_search_and_studio_purchase_handoffs()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local p={UserId=1};local jobs={};local reads=0;local products=0;local purchase={};local m;local hook;local owns=false
local function pump()while #jobs>0 do local run=jobs;jobs={};for _,f in run do f()end end end
local ctx={player=function()return p end,universe=function()return 7 end,guid=function()return 'mock'end,
 server=function()return false end,peer=function()return false end,isplayer=function(v)return v==p end,
 spawn=function(f)table.insert(jobs,f)end,connect=function(f)hook=f end,delay=function(_,f)table.insert(jobs,f)end,
 log=function(e)error(e)end,present=function()end,
 request=function(id,key)
  assert(id==7 and key=='mock');reads+=1
  return{categories={{id='c',name='Catalog',items={{id=1,type='GamePass'},{id=2,type='Product'},{id=3,type='Robux',info={robux=500,productId=3,providerProductId='500_Robux',price={amount=4.99,currency={currencySymbol='$'}}}}}}}}
 end,
 metadata=function(id,kind)products+=1;return{Name='Name'..id,DisplayName='Display'..id,ProductId=kind=='Product'and 20 or id,PriceInRobux=10}end,
 owns=function(id)assert(id==1);return owns end,
 purchase=function(kind,id,player)assert(player==p);table.insert(purchase,{kind,id})end}
m=assert(loadstring(source))(ctx)
m.boot();m.boot();pump();assert(reads==1 and products==2)
assert(m.open({},p)==nil);pump()
local items=m.visible_items();assert(#items==3 and items[1].name=='Display1'and items[2].id=='20'and items[3].price=='$4.99')
m.buy(items[3]);assert(#purchase==0) -- both Studio launch guards preclude billing
m.buy(items[1]);m.buy(items[2]);assert(#purchase==2 and purchase[1][1]=='GamePass'and purchase[1][2]==1 and purchase[2][1]=='Product'and purchase[2][2]==20)
m.search('Display');assert(#m.visible_items()==2);m.search('Robux');assert(#m.visible_items()==0)
m.select('c');assert(#m.visible_items()==3)
owns=true;hook(p,1,true);pump();items=m.visible_items();assert(items[1].owned)
m.buy(items[1]);assert(#purchase==2)
m.close();m.open({},p);pump();assert(reads==1 and products==2)
"#).exec()
    }
    #[test]
    fn shop_unpublished_skips_http_and_server_routes_only_to_target()->mlua::Result<()> {
        let lua=Lua::new();lua.globals().set("source",SOURCE)?;
        lua.load(r#"
local p={UserId=1};local other={UserId=2};local guest={UserId=0};local folder={IsA=function()return false end};local logs={};local sends={};local presentations=0;local server=false;local peer=false
local old=typeof;typeof=function(v)if v==folder then return'Instance'end;return old(v)end
local ctx={player=function()return p end,universe=function()return 0 end,server=function()return server end,peer=function()return peer end,
 isplayer=function(v)return v==p or v==other or v==guest end,connect=function()end,spawn=function(f)f()end,
 request=function()error('unpublished Studio must not fetch widgets')end,log=function(v)table.insert(logs,v)end,
 present=function()presentations+=1 end,send=function(_,tag,player)assert(tag=='__shop');table.insert(sends,player)end}
local m=assert(loadstring(source))(ctx)
for _,v in {false,123,'x'}do local ok,e=pcall(m.open,{},v);assert(not ok and tostring(e):find('Unable to cast value to Object',1,true))end
assert(m.open({},folder)==nil and logs[#logs]=='MarketplaceService:OpenShop: player is null.')
m.open({},nil);m.open({},guest);assert(logs[#logs]:find('cannot open shop for a guest player',1,true))
m.open({},other);assert(logs[#logs]:find('not called on a local player',1,true))
m.boot();m.open({},p);assert(#m.visible_items()==0 and presentations>0)
local count=presentations;m.open({},p);assert(presentations==count)
m.close();server=true;m.open({},p);assert(#sends==0)
peer=true;m.open({},p);assert(#sends==1 and sends[1]==p)
server=false;m.receive({},other,other);assert(presentations==count)
m.receive({},other,p);assert(presentations>count)
"#).exec()
    }
}
