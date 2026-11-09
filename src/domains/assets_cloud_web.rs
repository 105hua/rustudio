//! Worker-backed asset reads. A script yields on the scheduler during HTTP.
use super::*;
#[path = "assets_cloud_load.rs"]
mod load;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use rustudio_account::assets_cloud::Read;
enum Reply { Json(String), Asset(Vec<u8>,u64) }
#[derive(Default)]
struct Jobs { next: u64, pending: HashMap<u64, Receiver<Result<Reply,String>>> }
pub const LUA_METHODS: &[(&str,&str)] = &[
    ("CheckUploadCaptureStatusAsync","CaptureService"),
    ("GetBundleDetailsAsync","AssetService"),
    ("SearchAudio","AssetService"),("SearchAudioAsync","AssetService"),
    ("GetAudioMetadataAsync","AssetService"),
    ("GetGamePlacesAsync","AssetService"),
    ("GetLatestAssetVersionAsync","InsertService"),
    ("LoadAssetVersion","InsertService"),("LoadAsset","InsertService"),("LoadAssetAsync","AssetService"),
    ("CreatePlaceInPlayerInventoryAsync","AssetService"),
    ("GetFreeModels","InsertService"),("GetFreeModelsAsync","InsertService"),
    ("GetFreeDecals","InsertService"),("GetFreeDecalsAsync","InsertService"),
    ("GetBaseSets","InsertService"),("GetUserSets","InsertService"),
    ("GetCollection","InsertService"),("GetBaseCategories","InsertService"),("GetUserCategories","InsertService"),
    ("GetAssetIdsForPackage","AssetService"),
    ("GetAssetIdsForPackageAsync","AssetService"),
];
pub fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(Jobs::default());
    lua.globals().set("__ac_web_begin",lua.create_function(|lua,(name,a):(String,mlua::Table)| {
        let asset = matches!(name.as_str(), "LoadAsset"|"LoadAssetAsync"|"LoadAssetVersion");
        if asset && world(lua)?.side == Side::Client { return Err(rt_err(if name=="LoadAssetAsync" {"LoadAssetAsync cannot be used to load assets from the client"} else {"InsertService cannot be used to load assets from the client"})); }
        let id = if asset { Some(a.get::<u64>(1)?) } else { None };
        let place = if asset { a.get::<u64>(2)? } else { 0 };
        let read=match name.as_str() {
            "CheckUploadCaptureStatusAsync"=>Read::CaptureStatus{token:a.get(1)?,server:a.get(2)?},
            "GetAudioMetadataAsync"=>{
                let list=a.get::<mlua::Table>(1)?;
                if list.raw_len()>30{return Err(rt_err(format!("GetAudioMetadataAsync received {} assets which is over the maximum limit of 30.",list.raw_len())))}
                let mut ids=Vec::new();
                for v in list.sequence_values::<Value>() {
                    let id=match v? {
                        Value::Integer(n) if n>=0=>n as u64,
                        Value::Number(n) if n>=0. && n.is_finite()=>n as u64,
                        Value::String(s)=>{let raw=s.to_string_lossy();crate::assets::asset_ref(&raw).map(|a|a.id).or_else(||raw.parse().ok()).ok_or_else(||rt_err("Unable to cast value to ContentId"))?},
                        _=>return Err(rt_err("Unable to cast value to ContentId")),
                    };
                    ids.push(id);
                }
                Read::AudioMetadata(ids)
            },
            "SearchAudio"|"SearchAudioAsync"=>Read::AudioSearch(serde_json::from_str(&a.get::<String>(1)?).map_err(|e|rt_err(e.to_string()))?),
            "GetLatestAssetVersionAsync"=>Read::Latest(a.get(1)?),
            "GetBundleDetailsAsync"=>Read::Bundle(a.get(1)?),
            "GetAssetIdsForPackage"|"GetAssetIdsForPackageAsync"=>Read::Package(a.get(1)?),
            "GetFreeModels"|"GetFreeModelsAsync"|"GetFreeDecals"|"GetFreeDecalsAsync"=>Read::Free{decals:name.contains("Decals"),keyword:a.get(1)?,page:a.get(2)?},
            "GetGamePlacesAsync"=>Read::Places{universe:a.get(1)?,cursor:a.get(2)?},
            "LoadAsset"|"LoadAssetAsync"|"LoadAssetVersion"=>Read::Bundle(0),
            _=>return Err(rt_err("Unknown asset request")),
        };
        let (tx,rx)=mpsc::channel();
        let mut jobs=lua.app_data_mut::<Jobs>().unwrap();jobs.next+=1;let token=jobs.next;jobs.pending.insert(token,rx);drop(jobs);
        std::thread::spawn(move || {
            let response=(|| {
                if let Some(id)=id {
                    let bytes=if name=="LoadAssetVersion" {
                        let config=rustudio_account::AccountConfig::load().map_err(|e|e.to_string())?;
                        let c=rustudio_account::Client::open(config).map_err(|e|e.describe())?;
                        let url=format!("https://assetdelivery.roblox.com/v1/asset/?assetversionid={id}&serverplaceid={place}&scriptinsert=1&clientinsert=0");
                        let (status,bytes)=c.session.fetch_bytes(&url,true).map_err(|e|e.to_string())?;
                        if !(200..300).contains(&status){return Err(format!("HTTP {status}"))} bytes
                    } else { load::asset_bytes(id)? };
                    Ok(Reply::Asset(bytes,id))
                } else {
                    let config=rustudio_account::AccountConfig::load().map_err(|e|e.to_string())?;
                    let c=rustudio_account::Client::open(config).map_err(|e|e.describe())?;
                    read.fetch(&c.session).map(|v|Reply::Json(v.to_string())).map_err(|e|e.to_string())
                }
            })();
            let _=tx.send(response);
        });
        Ok(token)
    })?)?;
    lua.globals().set("__ac_web_poll",lua.create_function(|lua,id:u64| {
        let mut jobs=lua.app_data_mut::<Jobs>().unwrap();
        let rx=jobs.pending.get(&id).ok_or_else(||rt_err("Unknown asset request"))?;
        let result=match rx.try_recv(){Err(TryRecvError::Empty)=>return Ok((false,Value::Nil,None)),Err(_)=>Err("Asset worker disconnected".into()),Ok(v)=>v};
        jobs.pending.remove(&id);drop(jobs);
        Ok(match result {
            Ok(Reply::Json(v))=>(true,Value::String(lua.create_string(v)?),None),
            Ok(Reply::Asset(bytes,id))=>{
                if crate::fbx::is_fbx(&bytes) {
                    let r=crate::runtime::ext::load_fbx(lua,id,&bytes)?;
                    return Ok((true,inst_value(lua,r)?,None));
                }
                let mut tmp=crate::doc::Document::decode(load::model_bytes(&bytes)).map_err(|e|rt_err(format!("Asset {id} could not be decoded: {e}")))?;
                let r=crate::runtime::ext::create_any(lua,"Model")?;
                let mut w=world(lua)?;
                for child in tmp.root().children().to_vec(){tmp.transfer(child,&mut w.dm().dom,r);}
                drop(w);
                super::scene::scan_access(lua,r)?;
                (true,inst_value(lua,r)?,None)
            },
            Err(e)=>(true,Value::Nil,Some(e)),
        })
    })?)?;
    lua.globals().set("__ac_pages",lua.create_function(|lua,(class,pages):(String,mlua::Table)| {
        let r=crate::runtime::ext::create_any(lua,&class)?;
        super::super::pages::register_list(lua,r,pages.sequence_values::<mlua::Table>().collect::<mlua::Result<Vec<_>>>()?,None)?;
        inst_value(lua,r)
    })?)?;
    lua.load(r#"
local M=__rs_lua_methods
local function fetch(name,...)
    local token=__ac_web_begin(name,{...})
    while true do
        local done,body,err=__ac_web_poll(token)
        if done then
            if err then error(err,0) end
            if typeof(body)=='Instance' then return body end
            return game:GetService('HttpService'):JSONDecode(body)
        end
        task.wait()
    end
end
for _,name in {'LoadAsset','LoadAssetVersion','LoadAssetAsync'} do
    M[name]=function(self,id) return fetch(name,id,game.PlaceId) end
end
local function audioSubtype(value)
    if type(value)=='number' then
        for _,item in Enum.AudioSubType:GetEnumItems() do if item.Value==value then return item end end
    elseif type(value)=='string' then
        local ok,item=pcall(function() return Enum.AudioSubType[value] end)
        if ok then return item end
    end
end
local function audioRecord(v)
    local a,c=v.asset,v.creator
    local uid=tonumber(c.userId); local user=uid and uid>0
    local out={Id=tonumber(a.id),Name=a.name,Creator={Id=user and uid or tonumber(c.groupId),Type=user and 1 or 2,Name=c.name,IsVerifiedCreator=c.verified}}
    local maps={description='Description',createTime='CreateTime',updateTime='UpdateTime',title='Title',durationSeconds='Duration',artist='Artist'}
    for key,target in maps do if a[key]~=nil then out[target]=a[key] end end
    if a.audioType then out.AudioType=audioSubtype(a.audioType) end
    local tags={}; for _,key in {'genre','subcategory'} do if a[key] then table.insert(tags,a[key]) end end
    out.Tags=tags
    return out
end
M.CheckUploadCaptureStatusAsync=function(self,token)
    if token==nil then error('Argument 1 missing or nil',2) end
    if type(token)~='string' then error('Unable to cast value to std::string',2) end
    local reply=fetch('CheckUploadCaptureStatusAsync',token,__ac_is_server())
    for _,value in Enum.UploadCaptureResult:GetEnumItems() do
        if value.Value==reply.resultCode then return value,reply.assetId end
    end
end
M.SearchAudioAsync=function(self,p)
    if typeof(p)~='Instance' or not p:IsA('AudioSearchParams') then error('Unable to cast value to Object',2) end
    local sort=p.SortType.Name
    local category=({Relevance='Relevance',MostUsed='Top',Trending='Trending',RecentlyCreated='CreateTime',ShortestToLongest='AudioDuration',LongestToShortest='AudioDuration'})[sort] or sort
    local filters={audioTypes={p.AudioSubType.Name}}
    if p.MinDuration>0 then filters.minDurationSeconds=p.MinDuration end
    if p.MaxDuration>0 then filters.maxDurationSeconds=p.MaxDuration end
    if p.Artist~='' then filters.artist=p.Artist end
    if p.Album~='' then filters.album=p.Album end
    local body={searchCategoryType='Audio',searchView='Core',query=p.SearchKeyword,sortCategory=category,sortDirection=sort=='ShortestToLongest' and 'Ascending' or 'Descending',audioSearchFilters=filters}
    if p.MaxPageSize>0 then body.maxPageSize=math.min(p.MaxPageSize,100) end
    if p.Tag~='' then body.tags={p.Tag} end
    local pages,seen={},{}
    repeat
        local v=fetch('SearchAudioAsync',game:GetService('HttpService'):JSONEncode(body))
        local page={}
        for _,row in v.creatorStoreAssets or {} do table.insert(page,audioRecord(row)) end
        table.insert(pages,page)
        body.pageToken=v.nextPageToken
        if body.pageToken and seen[body.pageToken] then error('Repeated audio cursor',0) end
        if body.pageToken then seen[body.pageToken]=true end
    until body.pageToken==nil or body.pageToken==''
    return __ac_pages('AudioPages',pages)
end
M.SearchAudio=M.SearchAudioAsync
M.GetAudioMetadataAsync=function(self,ids)
    if type(ids)~='table' then error('Unable to cast to Array',2) end
    if #ids==0 then return {} end
    local result=fetch('GetAudioMetadataAsync',ids)
    local byId={}
    for _,row in result.audioMetadataResults or {} do byId[tostring(row.assetId)]=row end
    local out={}
    for _,input in ipairs(ids) do
        local raw=tostring(input)
        local id=raw:match('^rbxassetid://(%d+)') or raw:match('[?&]id=(%d+)') or raw:match('/assetId/(%d+)') or raw
        local row=byId[id]
        local value={AssetId=id}
        if row and row.audioMetadata then
            local m=row.audioMetadata
            value.Title=m.title;value.Artist=m.artist;value.Duration=m.duration or m.durationSeconds
            if m.audioType then value.AudioType=audioSubtype(m.audioType) end
        end
        table.insert(out,value)
    end
    return out
end
M.GetBundleDetailsAsync=function(self,id)
    local v=fetch('GetBundleDetailsAsync',id)
    local items={}
    for _,i in v.items or {} do
        local assetType=i.assetType
        if type(assetType)=='number' then
            local found
            for _,item in Enum.AvatarAssetType:GetEnumItems() do if item.Value==assetType then found=item.Name; break end end
            if not found then error('AssetService unexpected value for assetType when converting to string',0) end
            assetType=found
        end
        local item={Id=i.id,Name=i.name,Type=i.type,AssetType=assetType}
        if assetType=='DynamicHead' then item.SupportsHeadShapes=i.supportsHeadShapes or false end
        table.insert(items,item)
    end
    return {Id=v.id,Name=v.name,Description=v.description,BundleType=v.bundleType,Items=items}
end
M.GetAssetIdsForPackage=function(self,id) return fetch('GetAssetIdsForPackage',id).assetIds end
M.GetAssetIdsForPackageAsync=M.GetAssetIdsForPackage
M.GetLatestAssetVersionAsync=function(self,id)
    local result=fetch('GetLatestAssetVersionAsync',id)
    if not result.data or not result.data[1] or not result.data[1].Id then error('getLatestAssetVersion error occurred',0) end
    return result.data[1].Id
end
M.CreatePlaceInPlayerInventoryAsync=function() error("createPlaceInPlayerInventoryAsync has been removed as of Release 471.",0) end
for _,name in {'GetBaseSets','GetUserSets','GetCollection','GetBaseCategories','GetUserCategories'} do
    M[name]=function(self,...) error('LuaWebService error',0) end
end
for _,name in {'GetFreeModels','GetFreeModelsAsync','GetFreeDecals','GetFreeDecalsAsync'} do
    M[name]=function(self,keyword,page) return fetch(name,keyword,page) end
end
M.GetGamePlacesAsync=function(self)
    local pages,cursor,seen={},'',{}
    repeat
        local result=fetch('GetGamePlacesAsync',game.GameId,cursor)
        local page={}
        for _,v in result.data do table.insert(page,{Name=v.name,PlaceId=v.id}) end
        table.insert(pages,page)
        cursor=result.nextPageCursor
        if cursor and seen[cursor] then error('Repeated places cursor',0) end
        if cursor then seen[cursor]=true end
    until cursor==nil or cursor==''
    return __ac_pages('StandardPages',pages)
end
"#).set_name("=rustudio.assets_cloud.web").exec()
}
