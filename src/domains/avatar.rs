//! Engine API domain: avatar. See src/domains/mod.rs for the hook contract.
#![allow(unused_imports, unused_variables)]

use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::Ref;

use crate::runtime::*;

#[path = "avatar_props.rs"]
mod props;
#[path = "avatar_desc.rs"]
pub mod desc;
#[path = "avatar_rig.rs"]
mod rig;
#[path = "avatar_web.rs"]
pub mod web;
#[path = "avatar_face.rs"]
pub mod face;

pub const METHODS: &[(&str, &str)] = &[
    // HumanoidDescription (avatar_desc.rs)
    ("GetAccessories", "HumanoidDescription"),
    ("SetAccessories", "HumanoidDescription"),
    ("AddEmote", "HumanoidDescription"),
    ("RemoveEmote", "HumanoidDescription"),
    ("GetEmotes", "HumanoidDescription"),
    ("SetEmotes", "HumanoidDescription"),
    ("GetEquippedEmotes", "HumanoidDescription"),
    ("SetEquippedEmotes", "HumanoidDescription"),
    // HumanoidRigDescription / DigitsRigDescription (avatar_rig.rs)
    ("GetJoint", "HumanoidRigDescription"),
    ("GetJointFromName", "HumanoidRigDescription"),
    ("GetJointNames", "HumanoidRigDescription"),
    ("GetR15JointLabels", "HumanoidRigDescription"),
    ("GetR15JointNames", "HumanoidRigDescription"),
    ("GetR6JointNames", "HumanoidRigDescription"),
    ("SetJoint", "HumanoidRigDescription"),
    ("SetTposeAdjustment", "HumanoidRigDescription"),
    ("GetFingerControl", "DigitsRigDescription"),
    ("GetFingerTip", "DigitsRigDescription"),
    ("SetFingerControl", "DigitsRigDescription"),
    ("SetFingerTip", "DigitsRigDescription"),
    ("SetJoint", "DigitsRigDescription"),
    ("GetAccessoryType", "AvatarEditorService"),
];
pub const EVENTS: &[(&str, &str)] = &[
    ("PromptAllowInventoryReadAccessCompleted", "AvatarEditorService"),
    ("PromptCreateOutfitCompleted", "AvatarEditorService"),
    ("PromptDeleteOutfitCompleted", "AvatarEditorService"),
    ("PromptRenameOutfitCompleted", "AvatarEditorService"),
    ("PromptSaveAvatarCompleted", "AvatarEditorService"),
    ("PromptSetFavoriteCompleted", "AvatarEditorService"),
    ("PromptUpdateOutfitCompleted", "AvatarEditorService"),
    ("ApplyDescriptionFinished", "Humanoid"),
    ("EmotesChanged", "HumanoidDescription"),
    ("EquippedEmotesChanged", "HumanoidDescription"),
];
/// Methods written in Luau because they yield (installed into
/// `__rs_lua_methods` by `install`).
pub const LUA_METHODS: &[(&str, &str)] = &[
    // Humanoid:ApplyDescription family, GetAppliedInstance (avatar_apply.luau)
    ("ApplyDescription", "Humanoid"),
    ("ApplyDescriptionAsync", "Humanoid"),
    ("ApplyDescriptionReset", "Humanoid"),
    ("ApplyDescriptionResetAsync", "Humanoid"),
    ("GetAppliedInstance", "AccessoryDescription"),
    ("GetAppliedInstance", "MakeupDescription"),
    ("SetCageMeshContent", "WrapDeformer"),
    ("CreateEditableMeshAsync", "WrapDeformer"),
    ("GetDeformedCFrameAsync", "WrapDeformer"),
    ("CreateTextureInCageSpaceAsync", "WrapTarget"),
    ("CreateTextureInTargetSpaceAsync", "WrapTarget"),
    ("PrepareProjectionMeshDataAsync", "WrapTextureTransfer"),
    // AvatarEditorService / AvatarCreationService (avatar_aes.luau)
    ("PromptAllowInventoryReadAccess", "AvatarEditorService"),
    ("PromptCreateOutfit", "AvatarEditorService"),
    ("PromptDeleteOutfit", "AvatarEditorService"),
    ("PromptRenameOutfit", "AvatarEditorService"),
    ("PromptSaveAvatar", "AvatarEditorService"),
    ("PromptSetFavorite", "AvatarEditorService"),
    ("PromptUpdateOutfit", "AvatarEditorService"),
    ("CheckApplyDefaultClothing", "AvatarEditorService"),
    ("CheckApplyDefaultClothingAsync", "AvatarEditorService"),
    ("ConformToAvatarRules", "AvatarEditorService"),
    ("ConformToAvatarRulesAsync", "AvatarEditorService"),
    ("GetAvatarRules", "AvatarEditorService"),
    ("GetAvatarRulesAsync", "AvatarEditorService"),
    ("GetBatchItemDetails", "AvatarEditorService"),
    ("GetBatchItemDetailsAsync", "AvatarEditorService"),
    ("GetBundlesByAssetIdAsync", "AvatarEditorService"),
    ("GetFavorite", "AvatarEditorService"),
    ("GetFavoriteAsync", "AvatarEditorService"),
    ("GetHeadShapesAsync", "AvatarEditorService"),
    ("GetInventory", "AvatarEditorService"),
    ("GetInventoryAsync", "AvatarEditorService"),
    ("GetItemDetails", "AvatarEditorService"),
    ("GetItemDetailsAsync", "AvatarEditorService"),
    ("GetOutfitDetails", "AvatarEditorService"),
    ("GetOutfitDetailsAsync", "AvatarEditorService"),
    ("GetOutfits", "AvatarEditorService"),
    ("GetOutfitsAsync", "AvatarEditorService"),
    ("GetRecommendedAssets", "AvatarEditorService"),
    ("GetRecommendedAssetsAsync", "AvatarEditorService"),
    ("GetRecommendedBundles", "AvatarEditorService"),
    ("GetRecommendedBundlesAsync", "AvatarEditorService"),
    ("SearchCatalog", "AvatarEditorService"),
    ("SearchCatalogAsync", "AvatarEditorService"),
    ("GetValidationRules", "AvatarCreationService"),
    ("AutoSetupAvatarAsync", "AvatarCreationService"),
    ("GenerateAvatar2DPreviewAsync", "AvatarCreationService"),
    ("GenerateAvatarAsync", "AvatarCreationService"),
    ("GetBatchTokenDetailsAsync", "AvatarCreationService"),
    ("LoadAvatar2DPreviewAsync", "AvatarCreationService"),
    ("LoadGeneratedAvatarAsync", "AvatarCreationService"),
    ("PrepareAvatarForPreviewAsync", "AvatarCreationService"),
    ("PromptCreateAvatarAssetAsync", "AvatarCreationService"),
    ("PromptCreateAvatarAsync", "AvatarCreationService"),
    ("PromptCreateMakeupAsync", "AvatarCreationService"),
    ("PromptSelectAvatarGenerationImageAsync", "AvatarCreationService"),
    ("RequestAvatarGenerationSessionAsync", "AvatarCreationService"),
    ("ValidateUGCAccessoryAsync", "AvatarCreationService"),
    ("ValidateUGCBodyPartAsync", "AvatarCreationService"),
    ("ValidateUGCFullBodyAsync", "AvatarCreationService"),
];

fn class_of(lua: &Lua, r: Ref) -> Option<String> {
    lua.app_data_ref::<World>()?.d().class_of(r).map(|c| c.to_string())
}

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let class = class_of(lua, this)?;
    if class == "HumanoidDescription" && desc::METHODS.iter().any(|(m, _)| *m == name) {
        return Some(desc::call(lua, name, this, args));
    }
    if class == "AvatarEditorService" && name == "GetAccessoryType" {
        return Some(accessory_type(lua, args));
    }
    if matches!(class.as_str(), "HumanoidRigDescription" | "DigitsRigDescription") {
        if let Some(r) = rig::call(lua, &class, name, this, args) {
            return Some(r);
        }
    }
    None
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if let Some(v) = props::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    if class == "FaceControls" {
        if let Some(v) = face::prop_get(lua, r, key)? {
            return Ok(Some(v));
        }
    }
    Ok(None)
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if props::newindex(lua, r, class, key, value)? {
        return Ok(true);
    }
    if class == "FaceControls" && face::newindex(lua, r, key, value)? {
        return Ok(true);
    }
    Ok(false)
}

pub fn write_acts(class: &str, key: &str) -> bool {
    props::write_acts(class, key) || (class == "FaceControls" && face::is_control(key))
}

/// AvatarEditorService:GetAccessoryType: the AccessoryType an accessory
/// AvatarAssetType becomes (Hat → Hat, HairAccessory → Hair, …), Unknown
/// for everything else (checked item by item against Studio).
fn accessory_type(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let it = match args.get(1) {
        None | Some(Value::Nil) => return Err(rt_err("Argument 1 missing or nil")),
        Some(v) => match opt_ud::<crate::ltypes::LEnumItem>(v) {
            Some(e) if e.enum_type == "AvatarAssetType" => e,
            _ => match num(v).and_then(|n| crate::ltypes::LEnumItem::from_value("AvatarAssetType", n as u32)) {
                Some(e) => e,
                None => return Err(rt_err("Unable to cast value to AvatarAssetType")),
            },
        },
    };
    let name = if it.name == "Hat" {
        "Hat"
    } else {
        it.name.strip_suffix("Accessory").unwrap_or("Unknown")
    };
    let item = crate::ltypes::LEnumItem::from_name("AccessoryType", name)
        .or_else(|| crate::ltypes::LEnumItem::from_name("AccessoryType", "Unknown"))
        .ok_or_else(|| rt_err("Enum.AccessoryType missing"))?;
    Ok(MultiValue::from_iter([crate::ltypes::enum_value(lua, item)?]))
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(rig::Fingers::default());
    lua.set_app_data(face::Dirty::default());
    let web_fn = lua.create_function(|_, (method, url, body): (String, String, Option<String>)| {
        Ok(web::start(&method, url, body) as f64)
    })?;
    let poll_fn = lua.create_function(|lua, id: f64| {
        Ok(match web::poll(id as u64) {
            None => MultiValue::from_iter([Value::Boolean(false)]),
            Some(Ok((status, text))) => MultiValue::from_iter([
                Value::Boolean(true),
                Value::Number(status as f64),
                Value::String(lua.create_string(&text)?),
            ]),
            Some(Err(e)) => MultiValue::from_iter([Value::Nil, Value::String(lua.create_string(&e)?)]),
        })
    })?;
    let httperr = lua.create_function(|_, status: f64| Ok(web::http_error(status as u16)))?;
    let uid = lua.create_function(|_, ()| Ok(web::user_id() as f64))?;
    let is_client = lua.create_function(|lua, ()| Ok(world(lua)?.side == Side::Client))?;
    let fire = lua.create_function(|lua, args: MultiValue| {
        let mut it = args.into_iter();
        let inst = it.next().and_then(|v| opt_inst(&v)).ok_or_else(|| rt_err("fire: Instance expected"))?;
        let name = match it.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(rt_err("fire: event name expected")),
        };
        world(lua)?.fire(inst, &name, it.collect());
        Ok(())
    })?;
    let create = lua.create_function(|lua, class: String| {
        let r = crate::runtime::ext::create_any(lua, &class)?;
        inst_value(lua, r)
    })?;
    let rules_fn: mlua::Function = lua
        .load(include_str!("avatar_rules.luau"))
        .set_name("=rustudio.avatar.rules")
        .call(())?;
    let warnf = lua.create_function(|lua, msg: String| {
        world(lua)?.output.push((crate::app::Level::Warn, msg));
        Ok(())
    })?;
    lua.load(include_str!("avatar_aes.luau"))
        .set_name("=rustudio.avatar")
        .call::<()>((web_fn, poll_fn, httperr, uid, is_client, fire, create, rules_fn, warnf))?;
    let is_client = lua.create_function(|lua, ()| Ok(world(lua)?.side == Side::Client))?;
    let fire = lua.create_function(|lua, args: MultiValue| {
        let mut it = args.into_iter();
        let inst = it.next().and_then(|v| opt_inst(&v)).ok_or_else(|| rt_err("fire: Instance expected"))?;
        let name = match it.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(rt_err("fire: event name expected")),
        };
        world(lua)?.fire(inst, &name, it.collect());
        Ok(())
    })?;
    // Asset loads for ApplyDescription (yielding: Luau polls the worker).
    let load_start = lua.create_function(|_, (id, format): (f64, Option<String>)| {
        Ok(web::start_asset(id as u64, format) as f64)
    })?;
    let load_poll = lua.create_function(|lua, job: f64| {
        Ok(match web::poll_asset(job as u64) {
            None => MultiValue::from_iter([Value::Boolean(false)]),
            Some(Err(e)) => MultiValue::from_iter([Value::Nil, Value::String(lua.create_string(&e)?)]),
            Some(Ok(bytes)) => match crate::doc::Document::decode(&bytes) {
                Err(e) => MultiValue::from_iter([Value::Nil, Value::String(lua.create_string(&e)?)]),
                Ok(mut tmp) => {
                    let model = {
                        let mut w = world(lua)?;
                        let dom = &mut w.dm().dom;
                        let model = dom.insert(Ref::none(), rbx_dom_weak::InstanceBuilder::new("Model").with_name("Model"));
                        for c in tmp.root().children().to_vec() {
                            tmp.transfer(c, dom, model);
                        }
                        model
                    };
                    MultiValue::from_iter([inst_value(lua, model)?])
                }
            },
        })
    })?;
    lua.load(include_str!("avatar_apply.luau"))
        .set_name("=rustudio.avatar.apply")
        .call::<()>((is_client, fire, load_start, load_poll))?;
    Ok(())
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    face::step(lua)?;
    Ok(())
}
