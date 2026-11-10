//! Asset loading and material overrides. Studio probes: checks/parity/assets_cloud.md.
//! Unimplemented cloud APIs deliberately remain visible as gaps.
use crate::runtime::*;
use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::Ref;
#[path = "assets_cloud_web.rs"]
mod web;
#[path = "assets_cloud_capture.rs"]
pub mod capture;
#[path = "assets_cloud_scene.rs"]
pub mod scene;
#[path = "assets_cloud_content.rs"]
mod content;
#[path = "assets_cloud_decal.rs"]
mod decal;
#[path = "assets_cloud_gates.rs"]
mod gates;
pub const LUA_METHODS: &[(&str, &str)] = &[
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
    ("CaptureScreenshot","CaptureService"),("TakeScreenshotCaptureAsync","CaptureService"),("StartVideoCaptureAsync","CaptureService"),("StopVideoCapture","CaptureService"),("ReadCapturesFromGalleryAsync","CaptureService"),("PromptSaveCapturesToGallery","CaptureService"),("PromptShareCapture","CaptureService"),

    ("CreateDataModelContentAsync","AssetService"),
    ("CreateDecalAsync","AssetService"),("CreateTextureAsync","AssetService"),
    ("CreateAssetAsync","AssetService"),("CreateAssetVersionAsync","AssetService"),("PromptImportAnimationClipFromVideoAsync","AssetService"),
    ("CreateSessionAsync","MLService"),("LoadPostProcessModelAsync","MLService"),
    ("SetPostProcessEnabled","MLService"),("IsPostProcessReady","MLService"),("ForwardAsync","MLSession"),
    ("GetInstanceCompositionAsync","SceneAnalysisService"),("GetScriptMemoryAsync","SceneAnalysisService"),("GetUnparentedInstancesAsync","SceneAnalysisService"),
    ("GetSchema","FeatureMatchingTable"),
    ("SegmentMeshAsync","GenerationService"),("LoadGeneratedMeshAsync","GenerationService"),
    ("GetVideoGenSessionAsync","GenerationService"),("GetVideoGenTriggersAsync","GenerationService"),
    ("StartVideoGenSessionAsync","GenerationService"),("UpdateVideoGenSessionPromptAsync","GenerationService"),("UpdateVideoGenSessionTriggersAsync","GenerationService"),
    ("GetExecutingPlayer","RemoteCommandService"),("GetReceivedUpdateSignal","RemoteCommandService"),
    ("GetStoppingSignal","RemoteCommandService"),("SendUpdate","RemoteCommandService"),
    ("Call","AppHandle"),("Close","AppHandle"),("ExecuteLuau","AppHandle"),("RunTest","AppHandle"),
    ("Click","ParticipantHandle"),("InputText","ParticipantHandle"),("Launch","ParticipantHandle"),
    ("Screenshot","ParticipantHandle"),("Scroll","ParticipantHandle"),("Shutdown","ParticipantHandle"),
    ("GetChannels","FeatureMatchingSession"),
    ("GetDimensionCount","FeatureMatchingSession"),
    ("GetFeatureRow","FeatureMatchingSession"),
    ("GetFeatureRows","FeatureMatchingSession"),
    ("GetMetadataRow","FeatureMatchingSession"),
    ("GetRowCount","FeatureMatchingSession"),
    ("GetSchemaName","FeatureMatchingSession"),
    ("GetTagColumns","FeatureMatchingSession"),
    ("StandardizeQuery","FeatureMatchingSession"),
    ("GetDeviceInfo","CaptureService"),
    ("InternalCheckPlayabilityAsync","CaptureService"),
    ("InternalGetStartPlaceIdAsync","CaptureService"),
    ("Load","FeatureMatchingService"),
    ("LoadAsync","FeatureMatchingService"),
    ("ConnectAsync","GenerationService"),
    ("DisconnectAsync","GenerationService"),
    ("Disconnect","RealtimeMedia"),
    ("GetConnectedWires","RealtimeMedia"),
    ("GetInputPins","RealtimeMedia"),
    ("GetOutputPins","RealtimeMedia"),
    ("SendMessage","RealtimeMedia"),
    ("ConnectAsync","RealtimeMedia"),
    ("GetApiV1","OpenCloudService"),
    ("InvokeAsync","OpenCloudService"),
];
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    fetches: HashMap<String, Arc<Mutex<Option<bool>>>>,
    statuses: HashMap<String, String>,
    providers: HashMap<String, Ref>,
    watched: HashMap<String, (Ref,u8)>,
    failure_seq: u64,
    /// `assets::content_status_generation` when `watched` was last compared.
    status_gen: u64,
}
pub const METHODS: &[(&str, &str)] = &[
    ("GetBaseMaterialOverride", "MaterialService"),
    ("SetBaseMaterialOverride", "MaterialService"),
    ("GetMaterialVariant", "MaterialService"),
    ("Insert", "InsertService"),
    ("ApproveAssetId", "InsertService"),
    ("ApproveAssetVersionId", "InsertService"),
    ("GetAssetFetchStatus", "ContentProvider"),
    ("Preload", "ContentProvider"),
    ("ListEncryptedAssets", "ContentProvider"),
    ("RegisterDefaultEncryptionKey", "ContentProvider"),
    ("RegisterDefaultSessionKey", "ContentProvider"),
    ("RegisterEncryptedAsset", "ContentProvider"),
    ("RegisterSessionEncryptedAsset", "ContentProvider"),
    ("UnregisterDefaultEncryptionKey", "ContentProvider"),
    ("UnregisterEncryptedAsset", "ContentProvider"),
    ("GetAssetFetchStatusChangedSignal", "ContentProvider"),
];
pub const EVENTS: &[(&str, &str)] = &[("AssetFetchFailed", "ContentProvider")];
fn status(lua: &Lua, id: &str) -> mlua::Result<Value> {
    let n=crate::assets::content_fetch_status(id);
    let s=match n {1=>"Loading",2=>"Success",3=>"Failure",_=>"None"};
    Ok(Value::UserData(lua.create_userdata(
        LEnumItem::from_name("AssetFetchStatus", s).unwrap(),
    )?))
}
pub fn call(
    lua: &Lua,
    name: &str,
    this: Ref,
    args: &MultiValue,
) -> Option<mlua::Result<MultiValue>> {
    if !METHODS.iter().any(|(n, _)| *n == name) {
        return None;
    }
    Some((|| {
        let one = |v| Ok(MultiValue::from_vec(vec![v]));
        if matches!(name,"ApproveAssetId"|"ApproveAssetVersionId") {
            if !matches!(args.get(1),Some(Value::Number(_)|Value::Integer(_))) {return Err(rt_err("Argument 1 missing or nil"));}
            // Native descriptors point at the empty _guard_check_icall thunk.
            return Ok(MultiValue::new());
        }
        if name=="Insert" {
            let r=args.get(1).and_then(opt_inst).ok_or_else(||rt_err("instance must be non-nil"))?;
            let ws={let w=world(lua)?;w.d().workspace().ok_or_else(||rt_err("Workspace not available"))?};
            set_parent(lua,r,ws)?;
            return Ok(MultiValue::new());
        }

        if name.starts_with("GetAssetFetchStatus") {
            let id = match args.get(1) {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(rt_err("ContentId expected")),
            };
            return one(if name.ends_with("Signal") {
                let current=crate::assets::content_fetch_status(&id);
                {
                    let mut s=lua.app_data_mut::<State>().unwrap();
                    s.watched.entry(id.clone()).or_insert((this,current));
                    s.status_gen=0;
                }
                signal(lua, this, &format!("AssetFetchStatus:{id}"))?
            } else {
                status(lua, &id)?
            });
        }
        if name=="Preload" {
            let id=match args.get(1) {Some(Value::String(s))=>s.to_string_lossy(),_=>return Err(rt_err("ContentId expected"))};
            let begin:mlua::Function=lua.globals().get("__rs_asset_begin")?;
            begin.call::<()>((inst_value(lua,this)?,lua.create_sequence_from([id])?))?;
            return Ok(MultiValue::new());
        }
        // Studio's EnableLuaApiToRegisterEncryptedAssets defaults false. Native
        // entrypoints/9a0/a50 return without changing keys in this mode.
        if name=="ListEncryptedAssets" {return one(Value::Table(lua.create_table()?));}
        if name.starts_with("Register") || name.starts_with("Unregister") {
            let count=match name {"UnregisterDefaultEncryptionKey"=>0,"RegisterEncryptedAsset"|"RegisterSessionEncryptedAsset"=>2,_=>1};
            for n in 1..=count {if !matches!(args.get(n),Some(Value::String(_))) {return Err(rt_err(format!("Argument {n} missing or nil")));}}
            return Ok(MultiValue::new());
        }
        let material = args
            .get(1)
            .and_then(|v| {
                opt_ud::<LEnumItem>(v).or_else(|| match v {
                    Value::String(s) => LEnumItem::from_name("Material", &s.to_string_lossy()),
                    _ => None,
                })
            })
            .filter(|e| e.enum_type == "Material")
            .ok_or_else(|| rt_err("Unable to cast token to token"))?;
        if name != "GetMaterialVariant" && matches!(material.name.as_str(), "Air" | "Water") {
            return Err(rt_err(format!(
                "{name} failed: Material '{}' is not supported.",
                material.name
            )));
        }
        if name == "GetBaseMaterialOverride" {
            let s={let w=world(lua)?;
                w.d().dom.get_by_ref(this).and_then(|i|i.properties.get(&ustr(&format!("{}Name",material.name)))).and_then(|v|match v {Variant::String(s)=>Some(s.clone()),_=>None}).unwrap_or(material.name)
            };
            return one(Value::String(lua.create_string(s)?));
        }
        let variant = match args.get(2) {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(rt_err("string expected")),
        };
        if name == "SetBaseMaterialOverride" {
            store_prop(lua,this,&format!("{}Name",material.name),Variant::String(variant))?;
            return Ok(MultiValue::new());
        }
        let found = {
            let mut w = world(lua)?;
            let d = w.dm();
            d.dom
                .get_by_ref(this)
                .into_iter()
                .flat_map(|i| i.children())
                .find_map(|r| {
                    let i = d.dom.get_by_ref(*r)?;
                    if i.class.as_str() != "MaterialVariant" || i.name != variant {
                        return None;
                    }
                    match d.prop(*r, "BaseMaterial") {
                        Some(Variant::Enum(e)) if e.to_u32() == material.value => Some(*r),
                        _ => None,
                    }
                })
        };
        one(match found {
            Some(r) => inst_value(lua, r)?,
            None => Value::Nil,
        })
    })())
}
pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if let Some(v)=capture::prop_get(lua,r,class,key)? {return Ok(Some(v));}
    if class=="FeatureMatchingTable" && key=="SchemaName" {
        let v=world(lua)?.d().prop(r,key).unwrap_or(Variant::String(String::new()));
        return Ok(Some(variant_to_lua(lua,&v,None)?));
    }
    if (class=="SceneAnalysisService" && key=="GetCollisionGeometryMemoryAsync") || (class=="AssetService" && key=="GetLatestVersionUriAsync") {
        return Err(rt_err(format!("{key} is not a valid member of {class} \"{}\"",world(lua)?.full_name(r))));
    }
    if class=="SurfaceAppearance" && key=="SurfaceSlot" {
        let v=world(lua)?.d().prop(r,key).unwrap_or(Variant::Int32(0));
        return Ok(Some(variant_to_lua(lua,&v,None)?));
    }
    if matches!(class,"PackageLink"|"MeshImportData"|"MaterialImportData"|"BaseImportData"|"RootImportData") {
        // Preserve actual metadata from a loaded/imported DOM. Missing analysis
        // results stay gaps; do not substitute fabricated cage validation results.
        let v=world(lua)?.d().dom.get_by_ref(r).and_then(|i|i.properties.get(&ustr(key))).cloned();
        if let Some(v)=v {return Ok(Some(variant_to_lua(lua,&v,if key=="PermissionLevel"{Some("PackagePermission")}else{None})?));}
    }
    if class != "ContentProvider" {
        return Ok(None);
    }
    Ok(match key {
        "BaseUrl" => Some(Value::String(lua.create_string("https://www.roblox.com/")?)),
        "RequestQueueSize" => Some(Value::Integer(
            crate::assets::content_queue_size() as i64,
        )),
        _ => None,
    })
}
pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if class=="SurfaceAppearance" && key=="SurfaceSlot" {
        let n=match value {Value::Integer(n)=>*n as i32,Value::Number(n)=>*n as i32,_=>return Err(rt_err("Unable to cast value to int"))};
        store_prop(lua,r,key,Variant::Int32(n))?;return Ok(true)
    }
    Ok(false)
}
pub fn write_acts(_: &str, _: &str) -> bool {
    false
}
pub fn install(lua: &Lua) -> mlua::Result<()> {
    web::install(lua)?;
    capture::install(lua)?;
    gates::install(lua)?;
    scene::install(lua)?;
    decal::install(lua)?;
    content::install(lua)?;
    lua.set_app_data(State {failure_seq:crate::assets::content_failures_since(u64::MAX).0,..State::default()});
    lua.globals().set(
        "__rs_asset_begin",
        lua.create_function(|lua, (provider, ids): (Value, mlua::Table)| {
            let r = opt_inst(&provider).ok_or_else(|| rt_err("Instance expected"))?;
            let mut s = lua.app_data_mut::<State>().unwrap();
            for id in ids.sequence_values::<String>() {
                let id = id?;
                if id.is_empty() || s.fetches.contains_key(&id) {
                    continue;
                }
                let result = Arc::new(Mutex::new(None));
                s.fetches.insert(id.clone(), result.clone());
                crate::assets::content_track(&id,1);
                s.statuses.insert(id.clone(), "Loading".into());
                s.providers.insert(id.clone(), r);
                s.watched.insert(id.clone(),(r,1));
                s.status_gen=0;
                let v = Value::UserData(lua.create_userdata(
                    LEnumItem::from_name("AssetFetchStatus", "Loading").unwrap(),
                )?);
                world(lua)?.fire(r, &format!("AssetFetchStatus:{id}"), vec![v]);
                std::thread::spawn(move || {
                    let ok = crate::assets::script_content_blocking(&id).is_ok();
                    *result.lock().unwrap() = Some(ok);
                });
            }
            Ok(())
        })?,
    )?;
    lua.globals().set("__ac_content_ids", lua.create_function(|lua, assets: mlua::Table| {
        let mut out = Vec::<String>::new();
        let mut seen = std::collections::HashSet::new();
        let mut add = |id: String| { if !id.is_empty() && seen.insert(id.clone()) { out.push(id); } };
        let mut w = world(lua)?;
        let d = w.dm();
        for value in assets.sequence_values::<Value>() {
            let value = value?;
            if let Value::String(s) = &value { add(s.to_string_lossy()); continue; }
            let Some(root) = opt_inst(&value) else { continue };
            let mut pending = vec![root];
            while let Some(r) = pending.pop() {
                let Some(i) = d.dom.get_by_ref(r) else { continue };
                pending.extend_from_slice(i.children());
                let db = crate::doc::db();
                let Some(class) = db.classes.get(i.class.as_str()) else { continue };
                for c in db.superclasses_iter(class) {
                    for key in c.properties.keys() {
                        match d.prop(r, key) {
                            Some(Variant::ContentId(id)) => add(id.as_str().to_string()),
                            Some(Variant::Content(content)) => { if let Some(uri) = content.as_uri() { add(uri.to_string()); } },
                            _ => {},
                        }
                    }
                }
            }
        }
        lua.create_sequence_from(out)
    })?)?;
    lua.load(r#"
local M = __rs_lua_methods
function M.PreloadAsync(self, assets, callback)
    if type(assets) ~= 'table' then error('Unable to cast to Array', 2) end
    local ids = __ac_content_ids(assets)
    __rs_asset_begin(self,ids)
    for _,id in ipairs(ids) do
        local s = self:GetAssetFetchStatus(id)
        while s == Enum.AssetFetchStatus.Loading do task.wait(); s = self:GetAssetFetchStatus(id) end
        if callback then callback(id,s) end
    end
end
"#).exec()
}
pub fn step(lua: &Lua, _: f64) -> mlua::Result<()> {
    let completed: Vec<_> = {
        let s = lua.app_data_ref::<State>().unwrap();
        s.fetches
            .iter()
            .filter_map(|(id, r)| r.lock().ok().and_then(|v| *v).map(|ok| (id.clone(), ok)))
            .collect()
    };
    for (id, ok) in completed {
        let provider = {
            let mut s = lua.app_data_mut::<State>().unwrap();
            s.fetches.remove(&id);
            s.statuses
                .insert(id.clone(), if ok { "Success" } else { "Failure" }.into());
            let provider=s.providers.remove(&id).unwrap();
            s.watched.insert(id.clone(),(provider,if ok{2}else{3}));
            s.status_gen=0;
            provider
        };
        let v = status(lua, &id)?;
        let mut w = world(lua)?;
        w.fire(provider, &format!("AssetFetchStatus:{id}"), vec![v]);
    }
    // Only when some content's status changed (or a new id is watched):
    // asking for every watched id's status each frame was 0.85 ms a frame
    // in a live Place A match (a lock, a canonical key and a lookup per id).
    let generation=crate::assets::content_status_generation();
    let changes: Vec<_> = if lua.app_data_ref::<State>().unwrap().status_gen==generation {Vec::new()} else {
        let mut s=lua.app_data_mut::<State>().unwrap();
        s.status_gen=generation;
        s.watched.iter().filter_map(|(id,(r,old))| {
            let current=crate::assets::content_fetch_status(id);
            (current!=*old).then(||(id.clone(),*r,current))
        }).collect()
    };
    for (id,r,current) in changes {
        lua.app_data_mut::<State>().unwrap().watched.insert(id.clone(),(r,current));
        let value=status(lua,&id)?;
        world(lua)?.fire(r,&format!("AssetFetchStatus:{id}"),vec![value]);
    }
    let cursor=lua.app_data_ref::<State>().unwrap().failure_seq;
    let (next,failures)=crate::assets::content_failures_since(cursor);
    lua.app_data_mut::<State>().unwrap().failure_seq=next;
    let provider={let w=world(lua)?;let d=w.d();d.find_child(d.dom.root_ref(),"ContentProvider")};
    if let Some(r)=provider {for id in failures {world(lua)?.fire(r,"AssetFetchFailed",vec![Value::String(lua.create_string(id)?)]);}}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;
    use rbx_dom_weak::InstanceBuilder;
    #[test]
    fn realtime_class_access_matches_studio(){
        let mut d=Document::new_place();
        let ws=d.workspace().unwrap();
        d.dom.insert(ws,InstanceBuilder::new("RealtimeMedia").with_name("AC_Realtime"));
        let parent=d.find_child(d.dom.root_ref(),"ServerScriptService").unwrap();
        let script=d.dom.insert(parent,InstanceBuilder::new("Script").with_property("Source",r#"
local ok,e=pcall(function()return workspace:FindFirstChild('AC_Realtime')end)
assert(not ok and string.find(tostring(e),"The current thread cannot access 'RealtimeMedia' (lacking capability InternalTest)",1,true))
local ok,e=pcall(function()return Instance.new('RealtimeMedia')end)
assert(not ok and string.find(tostring(e),"The current thread cannot create 'RealtimeMedia' (lacking capability InternalTest)",1,true))
print('assets-cloud-realtime-ok')
"#.to_string()));
        let rt=Runtime::new().unwrap();rt.enter(World::new(d,RunMode::Run));rt.bind_globals().unwrap();rt.run_script(script);rt.step(1./60.);
        let output=rt.leave().output;
        assert!(output.iter().any(|(_,s)|s.contains("assets-cloud-realtime-ok")),"{output:?}");
    }
    #[test]
    fn assets_cloud_material_and_preload() {
        let mut d = Document::new_place();
        let parent = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        let storage=d.find_child(d.dom.root_ref(),"ReplicatedStorage").unwrap();
        d.dom.insert(storage,InstanceBuilder::new("ModuleScript").with_name("MemoryModule").with_property("Source","return string.rep('m',10000)".to_string()));
        let script = d.dom.insert(
            parent,
            InstanceBuilder::new("Script").with_property(
                "Source",
                r#"
local m = game:GetService('MaterialService')
assert(m:GetBaseMaterialOverride(Enum.Material.Plastic) == 'Plastic')
m:SetBaseMaterialOverride(Enum.Material.Plastic, 'custom')
assert(m:GetBaseMaterialOverride('Plastic') == 'custom')
assert(m:GetMaterialVariant(Enum.Material.Plastic,'absent') == nil)
local ok, err = pcall(function() m:SetBaseMaterialOverride(Enum.Material.Water,'x') end)
assert(not ok and string.find(err, "Material 'Water' is not supported.",1,true))
local ins=game:GetService('InsertService')
ins:ApproveAssetId(1); ins:ApproveAssetVersionId(1)
local model=Instance.new('Model'); ins:Insert(model); assert(model.Parent==workspace)
local insertOk,insertErr=pcall(function() ins:Insert(nil) end)
assert(not insertOk and string.find(insertErr,'instance must be non-nil',1,true))
local c = game:GetService('ContentProvider')
assert(c.BaseUrl == 'https://www.roblox.com/')
assert(c:GetAssetFetchStatus('') == Enum.AssetFetchStatus.Failure)
local id = 'rbxtemp://rustudio/parity-preload'
local changes = 0
c:GetAssetFetchStatusChangedSignal(id):Connect(function() changes += 1 end)
c:PreloadAsync({id}, function(content, status)
    assert(content == id and status == Enum.AssetFetchStatus.Success)
end)
assert(c.RequestQueueSize == 0)
assert(changes >= 1)
local scene = game:GetService('SceneAnalysisService')
local before=scene:GetInstanceCompositionAsync()
assert(before.Name=='InstanceComposition' and type(before.Children)=='table')
local part=Instance.new('Part',workspace)
local light=Instance.new('PointLight',part)
local folder=Instance.new('Folder',game:GetService('ServerStorage'))
local after=scene:GetInstanceCompositionAsync()
assert(after.Size==before.Size+3, tostring(before.Size)..' -> '..tostring(after.Size))
local categories={}
for _,category in after.Children do
    local count=0
    for _,class in category.Children do count+=class.Size end
    assert(count==category.Size)
    categories[category.Name]=category
end
assert(categories['3D Objects'] and categories.Lights and categories['Services & Storage'])
local moduleMemory=require(game:GetService('ReplicatedStorage').MemoryModule)
assert(#moduleMemory==10000)
__ac_test_collect()
local memoryBefore=scene:GetScriptMemoryAsync()
shared.__ac_memory_probe=string.rep('x',262144)
local memoryAfter=scene:GetScriptMemoryAsync()
assert(memoryAfter.Name=='ScriptMemory' and memoryAfter.Size>memoryBefore.Size+200000)
local foundScript,foundModule=false,false
for _,group in memoryAfter.Children do
    for _,kind in group.Children do
        for _,leaf in kind.Children do
            if leaf.Name==script:GetFullName() then foundScript=leaf.Size>262144 end
            if leaf.Name=='ReplicatedStorage.MemoryModule' then foundModule=leaf.Size>10000 end
        end
    end
end
assert(foundScript and foundModule,'script/module memory attribution missing')
local unparentedBefore=scene:GetUnparentedInstancesAsync()
local orphan=Instance.new('Model')
Instance.new('Folder',orphan)
Instance.new('IntValue',orphan)
local unparentedAfter=scene:GetUnparentedInstancesAsync()
assert(unparentedAfter.Name=='UnparentedInstances' and unparentedAfter.Size==unparentedBefore.Size+3)
local foundCreator=false
for _,group in unparentedAfter.Children do
    if group.Name==script:GetFullName() then
        local classes={}
        for _,leaf in group.Children do classes[leaf.Name]=leaf.Size end
        assert(classes.Model and classes.Folder and classes.IntValue)
        foundCreator=true
    end
end
assert(foundCreator,'orphan creator missing')
orphan.Parent=workspace
assert(scene:GetUnparentedInstancesAsync().Size==unparentedBefore.Size)
local asset=game:GetService('AssetService')
local editable=asset:CreateEditableImage({Size=Vector2.new(2,2)})
local content=Content.fromObject(editable)
local decal=asset:CreateDecalAsync({TextureContent=content,NormalMapContent=content})
assert(decal:IsA('Decal') and decal.Parent==nil)
assert(decal.TextureContent.Object==editable and decal.NormalMapContent.Object==editable)
local emptyOk,emptyError=pcall(function()asset:CreateDecalAsync({})end)
assert(not emptyOk and emptyError=='AssetService:CreateDecalAsync: No Content Maps provided')
local uriOk,uriError=pcall(function()asset:CreateDecalAsync({TextureContent=Content.fromUri('rbxassetid://1818')})end)
assert(not uriOk and uriError=='AssetService:CreateDecalAsync: TextureContent value should be an EditableImage Content')
local textureOk,textureError=pcall(function()asset:CreateTextureAsync({})end)
assert(not textureOk and textureError=='AssetService::CreateTextureAsync is not yet enabled')
editable:DrawRectangle(Vector2.zero,Vector2.new(2,2),Color3.new(1,0,0),0,Enum.ImageCombineType.Overwrite)
local unsupportedOk,unsupportedError=pcall(function()asset:CreateDataModelContentAsync(Content.fromUri('rbxassetid://1818'))end)
assert(not unsupportedOk and unsupportedError=='The input content type is not supported by AssetService::CreateDataModelContentAsync yet!')
local result,snapshot=asset:CreateDataModelContentAsync(Content.fromObject(editable))
assert(result==Enum.CreateContentResult.Success and snapshot.SourceType==Enum.ContentSourceType.Opaque)
assert(snapshot.Uri==nil and snapshot.Object==nil and tostring(snapshot)=='Content{SourceType=Opaque}')
editable:DrawRectangle(Vector2.zero,Vector2.new(2,2),Color3.new(0,0,1),0,Enum.ImageCombineType.Overwrite)
local imageCopy=asset:CreateEditableImageAsync(snapshot)
local pixels=imageCopy:ReadPixelsBuffer(Vector2.zero,Vector2.new(2,2))
assert(buffer.readu8(pixels,0)==255 and buffer.readu8(pixels,2)==0,'snapshot changed with original')
local mesh=asset:CreateEditableMesh()
local meshOk,meshError=pcall(function()asset:CreateDataModelContentAsync(Content.fromObject(mesh))end)
assert(not meshOk and meshError=='Error creating FileMeshData: Empty Result')
local v1=mesh:AddVertex(Vector3.new(0,0,0)); local v2=mesh:AddVertex(Vector3.new(1,0,0));local v3=mesh:AddVertex(Vector3.new(0,1,0))
mesh:AddTriangle(v1,v2,v3)
local meshResult,meshSnapshot=asset:CreateDataModelContentAsync(Content.fromObject(mesh))
assert(meshResult==Enum.CreateContentResult.Success and meshSnapshot.SourceType==Enum.ContentSourceType.Opaque)
local meshCopy=asset:CreateEditableMeshAsync(meshSnapshot)
assert(#meshCopy:GetVertices()==3 and #meshCopy:GetFaces()==1)
print('assets-cloud-ok')
"#
                .to_string(),
            ),
        );
        crate::assets::register_generated("rbxtemp://rustudio/parity-preload", vec![1]);
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.lua.globals().set("__ac_test_collect",rt.lua.create_function(|lua,()|lua.gc_collect()).unwrap()).unwrap();
        rt.run_script(script);
        for _ in 0..120 {
            rt.step(1.0 / 60.0);
            std::thread::yield_now();
        }
        let output = rt.leave().output;
        assert!(
            output.iter().any(|(_, s)| s.contains("assets-cloud-ok")),
            "{output:?}"
        );
    }
}
