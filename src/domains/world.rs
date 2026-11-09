//! Engine API domain: world (physics and the 3D world API). See
//! src/domains/mod.rs for the hook contract and checks/parity/world.md for
//! what was measured in Studio (offline Studio, Studio 0.705) and what's left.
//!
//! Submodules:
//! - world_part.rs: BasePart mass / assemblies / network ownership /
//!   impulses, Model (ScaleTo, identity orientation, persistent players),
//!   WorldRoot (ArePartsTouchingOthers, IsRegion3Empty…), Workspace.
//! - world_camera.rs: Camera (field of view forms, Pan/TiltUnits,
//!   Interpolate + InterpolationFinished, projection properties).
//! - world_constraints.rs: Constraint (Active, Current* readouts, debug
//!   force queries), Attachment, AlignOrientation axes, legacy movers
//!   (BodyPosition / RocketPropulsion + ReachedTarget), sensors.
//! - world_terrain.rs: Terrain's cell API (GetCell / SetCells / water
//!   cells / CountCells / material slots / Autowedge).
//! - world_misc.rs: Tool grips, Decal, Beam, Trail, WireframeHandleAdornment,
//!   controllers / ControlState, DragDetector, Dragger, JointsService,
//!   Skateboard*, VehicleSeat, StarterGear, ProceduralModel.
//!
//! Where Studio itself has nothing behind a member (no streaming in
//! Studio, no applied-force queries, Roblox-internal controller API…), the
//! member does exactly what Studio does there (its error, its empty result)
//! and the submodule says so.
#![allow(unused_imports, unused_variables)]

use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::Ref;

use crate::runtime::*;

#[path = "world_part.rs"]
mod part;
#[path = "world_camera.rs"]
mod camera;
#[path = "world_constraints.rs"]
mod constraints;
#[path = "world_terrain.rs"]
mod terrain;
#[path = "world_misc.rs"]
pub mod misc;
#[path = "world_controls.rs"]
mod controls;
#[path = "world_drag.rs"]
pub mod drag;

pub const METHODS: &[(&str, &str)] = &[
    // BasePart (some core methods are answered here first, see `call`).
    ("AngularAccelerationToTorque", "BasePart"),
    ("TorqueToAngularAcceleration", "BasePart"),
    ("BindToCollisionSummaries", "BasePart"),
    ("GetNetworkOwnershipAuto", "BasePart"),
    ("IsGrounded", "BasePart"),
    // Model
    ("AddPersistentPlayer", "Model"),
    ("GetPersistentPlayers", "Model"),
    ("RemovePersistentPlayer", "Model"),
    ("ResetOrientationToIdentity", "Model"),
    ("SetIdentityOrientation", "Model"),
    ("ScaleTo", "Model"),
    // WorldRoot / Workspace
    ("ArePartsTouchingOthers", "WorldRoot"),
    ("IsRegion3Empty", "WorldRoot"),
    ("IsRegion3EmptyWithIgnoreList", "WorldRoot"),
    ("GetNumAwakeParts", "Workspace"),
    ("GetPhysicsThrottling", "Workspace"),
    ("JoinToOutsiders", "Workspace"),
    ("UnjoinFromOutsiders", "Workspace"),
    ("PGSIsEnabled", "Workspace"),
    // Camera
    ("GetPanSpeed", "Camera"),
    ("GetTiltSpeed", "Camera"),
    ("PanUnits", "Camera"),
    ("TiltUnits", "Camera"),
    ("SetCameraPanMode", "Camera"),
    ("Interpolate", "Camera"),
    // Constraints, attachments, movers, sensors
    ("GetDebugAppliedForce", "Constraint"),
    ("GetDebugAppliedTorque", "Constraint"),
    ("GetConstraints", "Attachment"),
    ("SetAxis", "Attachment"),
    ("SetSecondaryAxis", "Attachment"),
    ("GetLastForce", "BodyPosition"),
    ("lastForce", "BodyPosition"),
    ("GetLastForce", "BodyVelocity"),
    ("lastForce", "BodyVelocity"),
    ("Fire", "RocketPropulsion"),
    ("fire", "RocketPropulsion"),
    ("Abort", "RocketPropulsion"),
    ("Sense", "SensorBase"),
    ("EvaluateAsync", "FluidForceSensor"),
    // Terrain cells
    ("GetCell", "Terrain"),
    ("SetCell", "Terrain"),
    ("SetCells", "Terrain"),
    ("GetWaterCell", "Terrain"),
    ("SetWaterCell", "Terrain"),
    ("CountCells", "Terrain"),
    ("AutowedgeCell", "Terrain"),
    ("AutowedgeCells", "Terrain"),
    ("GetMaterialSlot", "Terrain"),
    ("SetMaterialSlot", "Terrain"),
    ("ResetMaterialSlot", "Terrain"),
    ("ReadVoxelsAsync_beta", "Terrain"),
    ("WriteVoxelsAsync_beta", "Terrain"),
    ("ModifyVoxelsAsync_beta", "Terrain"),
    ("IterateVoxelsAsync_beta", "Terrain"),
    ("ClearVoxelsAsync_beta", "Terrain"),
    // Misc
    ("SetTextureOffset", "Beam"),
    ("Clear", "Trail"),
    ("AddLine", "WireframeHandleAdornment"),
    ("AddLines", "WireframeHandleAdornment"),
    ("AddPath", "WireframeHandleAdornment"),
    ("AddText", "WireframeHandleAdornment"),
    ("Clear", "WireframeHandleAdornment"),
    ("ForceGeneration", "ProceduralModel"),
    ("WaitForGenerationAsync", "ProceduralModel"),
    ("ClearJoinAfterMoveJoints", "JointsService"),
    ("CreateJoinAfterMoveJoints", "JointsService"),
    ("SetJoinAfterMoveInstance", "JointsService"),
    ("SetJoinAfterMoveTarget", "JointsService"),
    ("ShowPermissibleJoints", "JointsService"),
    ("GetReferenceFrame", "DragDetector"),
    ("SetDragStyleFunction", "DragDetector"),
    ("SetPermissionPolicyFunction", "DragDetector"),
    ("AddConstraintFunction", "DragDetector"),
    ("RestartDrag", "DragDetector"),
    ("MouseDown", "Dragger"),
    ("MouseMove", "Dragger"),
    ("MouseUp", "Dragger"),
    ("AxisRotate", "Dragger"),
    ("BindButton", "Controller"),
    ("bindButton", "Controller"),
    ("GetButton", "Controller"),
    ("getButton", "Controller"),
    ("UnbindButton", "Controller"),
    ("ApplySpecificImpulse", "SkateboardPlatform"),
    ("AddBoolField", "ControlState"),
    ("AddCFrameField", "ControlState"),
    ("AddInstanceField", "ControlState"),
    ("AddIntField", "ControlState"),
    ("AddNumberField", "ControlState"),
    ("AddUnitVector3Field", "ControlState"),
    ("AddVector2Field", "ControlState"),
    ("AddVector3Field", "ControlState"),
    ("GetChangedState", "ControlState"),
    ("GetReplicationWeight", "ControlState"),
    ("GetState", "ControlState"),
    ("HasField", "ControlState"),
    ("SetField", "ControlState"),
    ("UpdateFields", "ControlState"),
];

pub const EVENTS: &[(&str, &str)] = &[
    // play.rs fires these beside Touched / TouchEnded (Studio does).
    ("LocalSimulationTouched", "BasePart"),
    ("StoppedTouching", "BasePart"),
    ("ReachedTarget", "BodyPosition"),
    ("ReachedTarget", "RocketPropulsion"),
    ("InterpolationFinished", "Camera"),
    ("OnSensorOutputChanged", "SensorBase"),
    ("OnStateChanged", "ControlState"),
    ("ButtonChanged", "Controller"),
    ("Equipped", "SkateboardPlatform"),
    ("equipped", "SkateboardPlatform"),
];

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let class = {
        let w = world(lua).ok()?;
        w.d().class_of(this)?.to_string()
    };
    if let Some(r) = part::call(lua, name, this, &class, args) {
        return Some(r);
    }
    if let Some(r) = camera::call(lua, name, this, &class, args) {
        return Some(r);
    }
    if let Some(r) = constraints::call(lua, name, this, &class, args) {
        return Some(r);
    }
    if let Some(r) = terrain::call(lua, name, this, &class, args) {
        return Some(r);
    }
    if let Some(r) = controls::call(lua, name, this, &class, args) {
        return Some(r);
    }
    misc::call(lua, name, this, &class, args)
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if let Some(v) = part::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    if let Some(v) = camera::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    if let Some(v) = constraints::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    if let Some(v) = controls::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    misc::prop_get(lua, r, class, key)
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    Ok(part::newindex(lua, r, class, key, value)?
        || camera::newindex(lua, r, class, key, value)?
        || constraints::newindex(lua, r, class, key, value)?
        || misc::newindex(lua, r, class, key, value)?
        || controls::newindex(lua, r, class, key, value)?)
}

pub fn write_acts(class: &str, key: &str) -> bool {
    camera::write_acts(class, key) || constraints::write_acts(class, key) || misc::write_acts(class, key)
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    Ok(())
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    camera::step(lua, dt)?;
    constraints::step(lua, dt)?;
    controls::step(lua)?;
    Ok(())
}

// ---------------------------------------------------------------- helpers

pub(crate) mod util {
    use glam::{Mat3, Vec3};
    use mlua::{Lua, MultiValue, Value};
    use rbx_dom_weak::types::{Ref, Variant};

    use crate::doc::Document;
    use crate::ltypes::*;
    use crate::math::Xform;

    pub fn none() -> mlua::Result<MultiValue> {
        Ok(MultiValue::new())
    }
    pub fn one(v: Value) -> mlua::Result<MultiValue> {
        Ok(MultiValue::from_iter([v]))
    }
    fn arg(args: &MultiValue, i: usize) -> Value {
        args.get(i).cloned().unwrap_or(Value::Nil)
    }
    pub fn ud<T: Clone + 'static>(v: &Value) -> Option<T> {
        if let Some(x) = crate::ltypes::vec_any::<T>(v) {
            return Some(x);
        }
        match v {
            Value::UserData(u) => u.borrow::<T>().ok().map(|r| (*r).clone()),
            _ => None,
        }
    }
    pub fn vec3(lua: &Lua, v: Vec3) -> mlua::Result<Value> {
        Ok(LVector3(v).into_value())
    }
    pub fn cframe(lua: &Lua, x: Xform) -> mlua::Result<Value> {
        Ok(Value::UserData(lua.create_userdata(LCFrame(x))?))
    }
    pub fn num_arg(args: &MultiValue, i: usize) -> Option<f64> {
        crate::ltypes::num(&arg(args, i))
    }
    /// Studio's message for a missing required argument.
    pub fn missing(i: usize) -> mlua::Error {
        rt_err(format!("Argument {i} missing or nil"))
    }
    pub fn vec3_arg(args: &MultiValue, i: usize) -> mlua::Result<Vec3> {
        match arg(args, i) {
            Value::Nil => Err(missing(i)),
            v => ud::<LVector3>(&v).map(|v| v.0).ok_or_else(|| rt_err("Unable to cast value to Vector3")),
        }
    }
    pub fn enum_item(lua: &Lua, ty: &str, name: &str) -> mlua::Result<Value> {
        match LEnumItem::from_name(ty, name) {
            Some(i) => enum_value(lua, i),
            None => Ok(Value::Nil),
        }
    }
    /// An enum argument as its value: an EnumItem, its name or its number.
    pub fn enum_of(v: &Value, ty: &str) -> Option<u32> {
        match v {
            Value::Integer(i) => Some(*i as u32),
            Value::Number(n) => Some(*n as u32),
            Value::String(s) => LEnumItem::from_name(ty, &s.to_string_lossy()).map(|e| e.value),
            v => ud::<LEnumItem>(v).filter(|e| e.enum_type == ty).map(|e| e.value),
        }
    }
    pub fn mul(a: &Xform, b: &Xform) -> Xform {
        Xform { pos: a.pos + a.rot * b.pos, rot: a.rot * b.rot }
    }
    pub fn xf(d: &Document, r: Ref, name: &str) -> Option<Xform> {
        match d.prop(r, name)? {
            Variant::CFrame(cf) => Some(Xform::from_cframe(&cf)),
            _ => None,
        }
    }
    pub fn refp(d: &Document, r: Ref, name: &str) -> Option<Ref> {
        match d.prop(r, name)? {
            Variant::Ref(x) if x.is_some() && d.exists(x) => Some(x),
            _ => None,
        }
    }
    pub fn boolp(d: &Document, r: Ref, name: &str, default: bool) -> bool {
        match d.prop(r, name) {
            Some(Variant::Bool(b)) => b,
            _ => default,
        }
    }
    pub fn f32p(d: &Document, r: Ref, name: &str, default: f32) -> f32 {
        crate::scene::f32_of(d.prop(r, name)).unwrap_or(default)
    }
    pub fn v3p(d: &Document, r: Ref, name: &str) -> Option<Vec3> {
        crate::scene::vec3_of(d.prop(r, name))
    }
    /// An attachment's world frame (its parent part's CFrame × its CFrame;
    /// a Bone's too). None when it isn't under a part.
    pub fn attachment_world(d: &Document, a: Ref) -> Option<Xform> {
        let local = xf(d, a, "CFrame").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY });
        let parent = d.dom.get_by_ref(a)?.parent();
        let pc = d.class_of(parent)?;
        if !crate::doc::is_a(&pc, "BasePart") {
            return None;
        }
        Some(mul(&xf(d, parent, "CFrame")?, &local))
    }
    /// Whether `r` is under Workspace.
    pub fn in_workspace(d: &Document, r: Ref) -> bool {
        d.workspace().is_some_and(|ws| d.is_ancestor(ws, r))
    }
    pub fn is_part(d: &Document, r: Ref) -> bool {
        d.class_of(r).is_some_and(|c| crate::doc::is_a(&c, "BasePart"))
    }
}
