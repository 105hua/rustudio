//! Engine API domain: gui_input (GUI objects, input services, the new input
//! action system, styles, haptics / VR / gamepad). Hook contract in mod.rs.
//!
//! Most behaviour is Luau (gui_input.luau, loaded by `install`): its
//! methods are reached through `call` by "BaseClass.Method", its computed
//! properties through `prop_get` by "Class.Prop". Rust here: the glue, the
//! property store for classes rbx-reflection doesn't know (UIBlur,
//! ParticleEmitter2D, …), Path2D's curve maths and Path2DControlPoint.
//!
//! Studio's answers where there's no device (measured in an offline Studio
//! in an offline Studio, 2026-11-07; checks/parity/gui_input.md): no VR (identity
//! CFrames, VREnabled false), no accelerometer / gyroscope (warnings and an
//! empty InputObject), no touch (Touch* events exist and never fire), no
//! on-screen keyboard (0, 0), haptics as Studio's desktop (gamepad motors
//! Large / Small on connected pads), notifications "not yet enabled".

#![allow(clippy::too_many_arguments)]

use mlua::{Function, Lua, MultiValue, Table, Value};
use rbx_dom_weak::types::{Ref, Variant, VariantType};

use crate::ltypes::*;
use crate::runtime::*;

#[path = "gui_input_path2d.rs"]
mod path2d;


/// (method, base class), each one a function in gui_input.luau keyed
/// "Base.Method" (Path2D's are Rust, in gui_input_path2d.rs).
pub const METHODS: &[(&str, &str)] = &[
    // GuiService
    ("AddSelectionParent", "GuiService"),
    ("AddSelectionTuple", "GuiService"),
    ("RemoveSelectionGroup", "GuiService"),
    ("CloseInspectMenu", "GuiService"),
    ("DismissNotification", "GuiService"),
    ("SendNotification", "GuiService"),
    ("GetGameplayPausedNotificationEnabled", "GuiService"),
    ("SetGameplayPausedNotificationEnabled", "GuiService"),
    ("GetInsetArea", "GuiService"),
    ("InspectPlayerFromHumanoidDescription", "GuiService"),
    ("InspectPlayerFromUserId", "GuiService"),
    ("Select", "GuiService"),
    ("SetEmotesMenuOpen", "GuiService"),
    ("SetInspectMenuEnabled", "GuiService"),
    // UserInputService
    ("CreateVirtualInput", "UserInputService"),
    ("GetDeviceAcceleration", "UserInputService"),
    ("GetMouseButtonsPressed", "UserInputService"),
    ("GetUserCFrame", "UserInputService"),
    ("RecenterUserHeadCFrame", "UserInputService"),
    ("SetNavigationGamepad", "UserInputService"),
    ("GetFocusedTextBox", "UserInputService"),
    // VirtualInput (also its non-Instance form from CreateVirtualInput)
    ("SendKey", "VirtualInput"),
    ("SendMouseButton", "VirtualInput"),
    ("SendMouseDelta", "VirtualInput"),
    ("SendMousePosition", "VirtualInput"),
    ("SendPointerAction", "VirtualInput"),
    ("SendTextInput", "VirtualInput"),
    // ContextActionService
    ("BindActionToInputTypes", "ContextActionService"),
    ("GetCurrentLocalToolIcon", "ContextActionService"),
    // LayerCollector / PlayerGui
    ("GetGuiObjectsAtPosition", "LayerCollector"),
    ("GetGuiObjectsAtPosition", "BasePlayerGui"),
    ("GetLayoutNodeTree", "LayerCollector"),
    ("GetTopbarTransparency", "PlayerGui"),
    ("SetTopbarTransparency", "PlayerGui"),
    // TextBox
    ("CaptureFocus", "TextBox"),
    ("IsFocused", "TextBox"),
    ("ReleaseFocus", "TextBox"),
    // ScrollingFrame
    ("GetScrollVelocity", "ScrollingFrame"),
    ("ResetScrollVelocity", "ScrollingFrame"),
    // Layouts
    ("JumpTo", "UIPageLayout"),
    ("JumpToIndex", "UIPageLayout"),
    ("Next", "UIPageLayout"),
    ("Previous", "UIPageLayout"),
    ("ApplyLayout", "UIGridStyleLayout"),
    ("SetCustomSortFunction", "UIGridStyleLayout"),
    // UIDragDetector
    ("AddConstraintFunction", "UIDragDetector"),
    ("GetReferencePosition", "UIDragDetector"),
    ("GetReferenceRotation", "UIDragDetector"),
    ("SetDragStyleFunction", "UIDragDetector"),
    // Styles
    ("GetDefaultPropertyTransition", "StyleRule"),
    ("GetProperties", "StyleRule"),
    ("GetProperty", "StyleRule"),
    ("GetPropertyTransitions", "StyleRule"),
    ("SetDefaultPropertyTransition", "StyleRule"),
    ("SetProperties", "StyleRule"),
    ("SetProperty", "StyleRule"),
    ("SetPropertyTransition", "StyleRule"),
    ("SetPropertyTransitions", "StyleRule"),
    ("GetDerives", "StyleSheet"),
    ("SetDerives", "StyleSheet"),
    ("GetCondition", "StyleQuery"),
    ("GetConditions", "StyleQuery"),
    ("SetCondition", "StyleQuery"),
    ("SetConditions", "StyleQuery"),
    ("GetStyleRules", "StyleBase"),
    ("InsertStyleRule", "StyleBase"),
    ("SetStyleRules", "StyleBase"),
    // ParticleEmitter2D
    ("Clear", "ParticleEmitter2D"),
    ("Emit", "ParticleEmitter2D"),
    // Path2D (Rust)
    ("GetBoundingRect", "Path2D"),
    ("GetControlPoint", "Path2D"),
    ("GetControlPoints", "Path2D"),
    ("GetLength", "Path2D"),
    ("GetMaxControlPoints", "Path2D"),
    ("GetPositionOnCurve", "Path2D"),
    ("GetPositionOnCurveArcLength", "Path2D"),
    ("GetTangentOnCurve", "Path2D"),
    ("GetTangentOnCurveArcLength", "Path2D"),
    ("InsertControlPoint", "Path2D"),
    ("RemoveControlPoint", "Path2D"),
    ("SetControlPoints", "Path2D"),
    ("UpdateControlPoint", "Path2D"),
    // Input actions
    ("Fire", "InputAction"),
    ("GetState", "InputAction"),
    ("Fire", "InputBinding"),
    // Gamepad / haptics / VR
    ("DisableGamepadCursor", "GamepadService"),
    ("EnableGamepadCursor", "GamepadService"),
    ("GetMotor", "HapticService"),
    ("IsMotorSupported", "HapticService"),
    ("IsVibrationSupported", "HapticService"),
    ("SetMotor", "HapticService"),
    ("Play", "HapticEffect"),
    ("SetWaveformKeys", "HapticEffect"),
    ("Stop", "HapticEffect"),
    ("GetTouchpadMode", "VRService"),
    ("SetTouchpadMode", "VRService"),
    // ProximityPrompt / Dialog / TextGenerator
    ("InputHoldBegin", "ProximityPrompt"),
    ("InputHoldEnd", "ProximityPrompt"),
    ("GetCurrentPlayers", "Dialog"),
];

/// Yielding methods, written in gui_input.luau (put in `__rs_lua_methods`).
pub const LUA_METHODS: &[(&str, &str)] = &[("GenerateTextAsync", "TextGenerator")];

/// Events this domain fires (the rest of its classes' events were already
/// known to runtime_ext.rs; gui_input.luau fires many of those too).
pub const EVENTS: &[(&str, &str)] = &[
    ("DeviceAccelerationChanged", "UserInputService"),
    ("DeviceGravityChanged", "UserInputService"),
    ("TouchDrag", "UserInputService"),
    ("UserCFrameChanged", "UserInputService"),
    ("LocalToolEquipped", "ContextActionService"),
    ("LocalToolUnequipped", "ContextActionService"),
    ("DragBegin", "GuiObject"),
    ("DragStopped", "GuiObject"),
    ("TouchLongPress", "GuiObject"),
    ("TouchPan", "GuiObject"),
    ("TouchPinch", "GuiObject"),
    ("TouchRotate", "GuiObject"),
    ("TouchSwipe", "GuiObject"),
    ("SelectionChanged", "GuiBase2d"),
    ("TopbarTransparencyChangedSignal", "PlayerGui"),
    ("PageEnter", "UIPageLayout"),
    ("PageLeave", "UIPageLayout"),
    ("Stopped", "UIPageLayout"),
    ("DragStart", "UIDragDetector"),
    ("DragContinue", "UIDragDetector"),
    ("DragEnd", "UIDragDetector"),
    ("StyleRulesChanged", "StyleBase"),
    ("ControlPointChanged", "Path2D"),
    ("Pressed", "InputAction"),
    ("Released", "InputAction"),
    ("StateChanged", "InputAction"),
    ("Ended", "HapticEffect"),
    ("TouchpadModeChanged", "VRService"),
    ("IndicatorShown", "ProximityPrompt"),
    ("IndicatorHidden", "ProximityPrompt"),
    ("PromptShown", "ProximityPromptService"),
    ("PromptHidden", "ProximityPromptService"),
    ("PromptTriggered", "ProximityPromptService"),
    ("PromptTriggerEnded", "ProximityPromptService"),
    ("PromptButtonHoldBegan", "ProximityPromptService"),
    ("PromptButtonHoldEnded", "ProximityPromptService"),
    ("IndicatorShown", "ProximityPromptService"),
    ("IndicatorHidden", "ProximityPromptService"),
    ("DialogChoiceSelected", "Dialog"),
    ("MouseButton1Down", "HandleAdornment"),
    ("MouseButton1Up", "HandleAdornment"),
    ("MouseEnter", "HandleAdornment"),
    ("MouseLeave", "HandleAdornment"),
    ("MouseButton1Down", "HandlesBase"),
    ("MouseButton1Up", "HandlesBase"),
    ("MouseDrag", "HandlesBase"),
    ("MouseEnter", "HandlesBase"),
    ("MouseLeave", "HandlesBase"),
];

/// Properties of classes rbx-reflection doesn't know, kept in the DOM (so
/// saved and replicated like any other): class, name, type, enum, Studio's
/// default (a Luau expression; measured, the API dump's are placeholders).
const STORED: &[(&str, &str, VariantType, &str, &str)] = &[
    ("UIBlur", "Color", VariantType::Color3, "", "Color3.new(1, 1, 1)"),
    ("UIBlur", "Enabled", VariantType::Bool, "", "true"),
    ("UIBlur", "Strength", VariantType::Enum, "UIBlurStrength", "Enum.UIBlurStrength.Heavy"),
    ("UIBlur", "Transparency", VariantType::Float32, "", "0"),
    ("UIDragDetector", "Selectable", VariantType::Bool, "", "true"),
    ("InputBinding", "Deadzone", VariantType::NumberRange, "", "NumberRange.new(0.1, 1)"),
    ("ParticleEmitter2D", "Acceleration", VariantType::Vector2, "", "Vector2.zero"),
    ("ParticleEmitter2D", "Blend", VariantType::Float32, "", "0"),
    ("ParticleEmitter2D", "Brightness", VariantType::Float32, "", "1"),
    ("ParticleEmitter2D", "Color", VariantType::ColorSequence, "", "ColorSequence.new(Color3.new(1, 1, 1))"),
    ("ParticleEmitter2D", "Drag", VariantType::Float32, "", "0"),
    ("ParticleEmitter2D", "EmissionAngle", VariantType::Float32, "", "0"),
    ("ParticleEmitter2D", "EmissionMode", VariantType::Enum, "ParticleEmitter2DEmissionMode", "Enum.ParticleEmitter2DEmissionMode.BoundingBox"),
    ("ParticleEmitter2D", "EmissionOffset", VariantType::Vector2, "", "Vector2.zero"),
    ("ParticleEmitter2D", "EmissionSpread", VariantType::Vector2, "", "Vector2.zero"),
    ("ParticleEmitter2D", "Enabled", VariantType::Bool, "", "true"),
    ("ParticleEmitter2D", "FlipbookBlendFrames", VariantType::Bool, "", "true"),
    ("ParticleEmitter2D", "FlipbookFramerate", VariantType::NumberRange, "", "NumberRange.new(1)"),
    ("ParticleEmitter2D", "FlipbookLayout", VariantType::Enum, "ParticleFlipbookLayout", "Enum.ParticleFlipbookLayout.None"),
    ("ParticleEmitter2D", "FlipbookMode", VariantType::Enum, "ParticleFlipbookMode", "Enum.ParticleFlipbookMode.Loop"),
    ("ParticleEmitter2D", "FlipbookSizeX", VariantType::Int32, "", "1"),
    ("ParticleEmitter2D", "FlipbookSizeY", VariantType::Int32, "", "1"),
    ("ParticleEmitter2D", "FlipbookStartRandom", VariantType::Bool, "", "false"),
    ("ParticleEmitter2D", "Lifetime", VariantType::NumberRange, "", "NumberRange.new(5, 10)"),
    ("ParticleEmitter2D", "LightEmission", VariantType::Float32, "", "0"),
    ("ParticleEmitter2D", "LockedToParent", VariantType::Bool, "", "false"),
    ("ParticleEmitter2D", "Rate", VariantType::Float32, "", "20"),
    ("ParticleEmitter2D", "RotSpeed", VariantType::NumberRange, "", "NumberRange.new(0)"),
    ("ParticleEmitter2D", "Rotation", VariantType::NumberRange, "", "NumberRange.new(0)"),
    ("ParticleEmitter2D", "Size", VariantType::NumberSequence, "", "NumberSequence.new(32)"),
    ("ParticleEmitter2D", "Speed", VariantType::NumberRange, "", "NumberRange.new(64)"),
    ("ParticleEmitter2D", "SpreadAngle", VariantType::Float32, "", "0"),
    ("ParticleEmitter2D", "Squash", VariantType::NumberSequence, "", "NumberSequence.new(0)"),
    ("ParticleEmitter2D", "TextureContent", VariantType::Content, "", "Content.fromUri(\"rbxasset://textures/particles/sparkles_main.dds\")"),
    ("ParticleEmitter2D", "TimeScale", VariantType::Float32, "", "1"),
    ("ParticleEmitter2D", "Transparency", VariantType::NumberSequence, "", "NumberSequence.new(0)"),
    ("ParticleEmitter2D", "ZIndex", VariantType::Int32, "", "1"),
];

fn stored(class: &str, key: &str) -> Option<&'static (&'static str, &'static str, VariantType, &'static str, &'static str)> {
    STORED.iter().find(|s| s.1 == key && crate::doc::is_a(class, s.0))
}

/// Computed properties answered by gui_input.luau's `get["Class.Prop"]`
/// (class matched with IsA).
const LUA_PROPS: &[(&str, &str)] = &[
    ("GuiService", "CoreGuiNavigationEnabled"),
    ("GuiService", "PreferredTextSize"),
    ("GuiService", "PreferredTransparency"),
    ("GuiService", "ReducedMotionEnabled"),
    ("GuiService", "TopbarInset"),
    ("GuiService", "ViewportDisplaySize"),
    ("UserInputService", "OnScreenKeyboardPosition"),
    ("UserInputService", "OnScreenKeyboardSize"),
    ("UserInputService", "UserHeadCFrame"),
    ("GuiObject", "GuiState"),
    ("GuiObject", "InputSink"),
    ("BillboardGui", "CurrentDistance"),
    ("PlayerGui", "CurrentScreenOrientation"),
    ("CoreGui", "Version"),
    ("TextBox", "ContentText"),
    ("TextBox", "CursorPosition"),
    ("TextBox", "SelectionStart"),
    ("TextBox", "OpenTypeFeaturesError"),
    ("TextBox", "TextFits"),
    ("TextButton", "ContentText"),
    ("TextButton", "LocalizedText"),
    ("TextButton", "OpenTypeFeaturesError"),
    ("TextButton", "TextFits"),
    ("TextLabel", "ContentText"),
    ("TextLabel", "LocalizedText"),
    ("TextLabel", "OpenTypeFeaturesError"),
    ("TextLabel", "TextFits"),
    ("ImageButton", "IsLoaded"),
    ("ImageLabel", "IsLoaded"),
    ("ScrollingFrame", "AbsoluteCanvasSize"),
    ("ScrollingFrame", "AbsoluteWindowSize"),
    ("UIPageLayout", "CurrentPage"),
    ("UIGridLayout", "AbsoluteCellCount"),
    ("UIGridLayout", "AbsoluteCellSize"),
    ("StyleRule", "SelectorError"),
    ("StyleQuery", "IsActive"),
    ("InputAction", "PreferredBinding"),
    ("InputActionLabel", "ResolvedText"),
    ("InputActionLabel", "ResolvedImageContent"),
    ("VRService", "GuiInputUserCFrame"),
    ("Dialog", "InUse"),
    ("GuiService", "MenuIsOpen"),
];

fn lua_prop(class: &str, key: &str) -> Option<&'static str> {
    LUA_PROPS
        .iter()
        .find(|(c, k)| *k == key && crate::doc::is_a(class, c))
        .map(|(c, _)| *c)
}

/// Writes gui_input.luau acts on ("Class.Prop" in its `set` table).
const LUA_WRITES: &[(&str, &str)] = &[
    ("TextBox", "CursorPosition"),
    ("TextBox", "SelectionStart"),
    ("GuiService", "SelectedObject"),
    ("GuiService", "CoreGuiNavigationEnabled"),
    ("GuiObject", "InputSink"),
    ("VRService", "GuiInputUserCFrame"),
    ("Dialog", "InUse"),
    ("InputAction", "Type"),
    ("InputAction", "Enabled"),
    ("InputContext", "Enabled"),
];

fn lua_write(class: &str, key: &str) -> Option<&'static str> {
    LUA_WRITES
        .iter()
        .find(|(c, k)| *k == key && crate::doc::is_a(class, c))
        .map(|(c, _)| *c)
}

/// A non-Instance engine Object (typeof "Object": VirtualInput), its
/// methods from a Lua table.
struct LObject {
    class: String,
    methods: Table,
}

impl mlua::UserData for LObject {
    fn add_fields<F: mlua::UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Object");
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::Index, |_, this, key: String| {
            if key == "ClassName" {
                return Ok(Value::String(this.methods.raw_get::<mlua::String>("__class")?));
            }
            let f: Value = this.methods.raw_get(key.as_str())?;
            if f.is_nil() || key.starts_with("__") {
                return Err(rt_err(format!("{key} is not a valid member of {}", this.class)));
            }
            Ok(f)
        });
        m.add_meta_method(mlua::MetaMethod::NewIndex, |_, this, (k, _): (String, Value)| {
            Err::<(), _>(rt_err(format!("{k} is not a valid member of {}", this.class)))
        });
        m.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| Ok(this.class.clone()));
    }
}

fn module(lua: &Lua) -> mlua::Result<Table> {
    lua.named_registry_value::<Table>("rs_gui_input")
}

fn class_of(lua: &Lua, r: Ref) -> mlua::Result<String> {
    Ok(world(lua)?
        .d()
        .class_of(r)
        .map(|c| c.to_string())
        .unwrap_or_default())
}

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if !METHODS.iter().any(|(m, _)| *m == name) {
        return None;
    }
    let class = match class_of(lua, this) {
        Ok(c) => c,
        Err(e) => return Some(Err(e)),
    };
    let base = METHODS
        .iter()
        .find(|(m, b)| *m == name && crate::doc::is_a(&class, b))
        .map(|(_, b)| *b)?;
    if base == "Path2D" {
        return Some(path2d::call(lua, name, this, args));
    }
    Some((|| {
        let m: Table = module(lua)?.get("methods")?;
        let f: Function = m.get(format!("{base}.{name}"))?;
        f.call::<MultiValue>(args.clone())
    })())
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if let Some(s) = stored(class, key) {
        let v = world(lua)?.d().prop(r, key);
        return Ok(Some(match v {
            Some(v) => variant_to_lua(lua, &v, (!s.3.is_empty()).then_some(s.3))?,
            None => default_of(lua, s.4)?,
        }));
    }
    if let Some(base) = lua_prop(class, key) {
        let get: Table = module(lua)?.get("get")?;
        let f: Function = get.get(format!("{base}.{key}"))?;
        return Ok(Some(f.call::<Value>((inst_value(lua, r)?, key))?));
    }
    Ok(None)
}

fn default_of(lua: &Lua, expr: &str) -> mlua::Result<Value> {
    let cache: Table = module(lua)?.get("defaults")?;
    let v: Value = cache.get(expr)?;
    if !v.is_nil() {
        return Ok(v);
    }
    let v: Value = lua.load(format!("return {expr}")).set_name("=gui_input.default").eval()?;
    cache.set(expr, v.clone())?;
    Ok(v)
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if let Some(s) = stored(class, key) {
        let existing = world(lua)?.d().prop(r, key);
        let en = (!s.3.is_empty()).then_some(s.3);
        let v = lua_to_variant(key, value, s.2, en, existing.as_ref())?;
        store_prop(lua, r, key, v)?;
        return Ok(true);
    }
    if let Some(base) = lua_write(class, key) {
        let set: Table = module(lua)?.get("set")?;
        let f: Function = set.get(format!("{base}.{key}"))?;
        // true: the Luau stored it itself; false: store it as usual too.
        let done: bool = f.call((inst_value(lua, r)?, key, value.clone()))?;
        return Ok(done);
    }
    Ok(false)
}

pub fn write_acts(class: &str, key: &str) -> bool {
    stored(class, key).is_some() || lua_write(class, key).is_some()
}

/// The library's InputObject maker, for Rust callers.
fn input_object(lua: &Lua, fields: Table) -> mlua::Result<Value> {
    let f: Function = lua.globals().get("__rs_new_input_object")?;
    f.call(fields)
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    path2d::install(lua)?;
    let g = lua.create_table()?;
    // fire(inst, name, ...): queue an event the way the engine does.
    g.set(
        "fire",
        lua.create_function(|lua, args: MultiValue| {
            let mut it = args.into_iter();
            let r = it.next().as_ref().and_then(opt_inst).ok_or_else(|| rt_err("fire: Instance expected"))?;
            let name = match it.next() {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(rt_err("fire: event name expected")),
            };
            world(lua)?.fire(r, &name, it.collect());
            Ok(())
        })?,
    )?;
    // Studio's error / warning lines (Output) without throwing.
    g.set(
        "log",
        lua.create_function(|lua, (level, msg): (String, String)| {
            let lv = match level.as_str() {
                "error" => crate::app::Level::Error,
                "warn" => crate::app::Level::Warn,
                _ => crate::app::Level::Info,
            };
            world(lua)?.output.push((lv, msg));
            Ok(())
        })?,
    )?;
    // The world's DataModel as it is now (the global `game` this chunk
    // saw at install can be a DataModel since replaced).
    g.set(
        "game",
        lua.create_function(|lua, ()| {
            let root = world(lua)?.d().dom.root_ref();
            inst_value(lua, root)
        })?,
    )?;
    // class_signals({classes}): (added, removing) signals on game for the
    // instances of those classes only (runtime::class_signals).
    g.set(
        "class_signals",
        lua.create_function(|lua, classes: Vec<String>| crate::runtime::class_signals(lua, classes))?,
    )?;
    g.set(
        "is_client",
        lua.create_function(|lua, ()| Ok(world(lua).is_ok_and(|w| w.side == Side::Client)))?,
    )?;
    g.set(
        "playing",
        lua.create_function(|lua, ()| {
            Ok(world(lua).is_ok_and(|w| w.side == Side::Client && w.mode == RunMode::Play && w.has_peer))
        })?,
    )?;
    g.set(
        "time",
        lua.create_function(|lua, ()| Ok(world(lua).map(|w| w.time).unwrap_or(0.0)))?,
    )?;
    g.set(
        "view",
        lua.create_function(|lua, ()| {
            let w = world(lua)?;
            let v = w.input.view_size;
            Ok((v[0], v[1], w.input.mouse_xy[0], w.input.mouse_xy[1]))
        })?,
    )?;
    g.set(
        "gamepads",
        lua.create_function(|lua, ()| Ok(world(lua).map(|w| w.input.gamepads).unwrap_or(0)))?,
    )?;
    g.set(
        "mouse",
        lua.create_function(|lua, ()| {
            // (The World borrow ends before inst_value borrows it again.)
            let m = world(lua)?.input.mouse;
            match m {
                Some(m) => inst_value(lua, m),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    g.set(
        "menu_open",
        lua.create_function(|lua, ()| Ok(world(lua).is_ok_and(|w| w.input.menu_open)))?,
    )?;
    // Drawn box of a GuiObject last frame: x, y, w, h, canvas w, canvas h (or nil).
    g.set(
        "layout",
        lua.create_function(|lua, inst: Value| {
            let r = opt_inst(&inst).ok_or_else(|| rt_err("Instance expected"))?;
            let w = world(lua)?;
            Ok(match w.input.gui_layout.get(&r) {
                Some(b) => {
                    let c = b.canvas.unwrap_or([-1.0, -1.0]);
                    let t = b.text.unwrap_or([-1.0, -1.0]);
                    MultiValue::from_iter(
                        [b.pos[0], b.pos[1], b.size[0], b.size[1], c[0], c[1], t[0], t[1]]
                            .map(|x| Value::Number(x as f64)),
                    )
                }
                None => MultiValue::new(),
            })
        })?,
    )?;
    // The viewport GUI's shared state (src/gui.rs GuiShare).
    g.set(
        "focus",
        lua.create_function(|lua, ()| match crate::gui::share().focus {
            Some(r) => inst_value(lua, r),
            None => Ok(Value::Nil),
        })?,
    )?;
    g.set(
        "set_focus",
        lua.create_function(|_, inst: Value| {
            let r = opt_inst(&inst);
            let mut sh = crate::gui::share();
            sh.focus = r;
            sh.focus_req.push(r);
            Ok(())
        })?,
    )?;
    g.set(
        "focus_events",
        lua.create_function(|lua, ()| {
            let evs = std::mem::take(&mut crate::gui::share().focus_events);
            let out = lua.create_table()?;
            for (r, gained, enter, by) in evs {
                let t = lua.create_table()?;
                t.set("box", inst_value(lua, r)?)?;
                t.set("gained", gained)?;
                t.set("enter", enter)?;
                if let Some((b, x, y)) = by {
                    t.set("button", b)?;
                    t.set("x", x)?;
                    t.set("y", y)?;
                }
                out.push(t)?;
            }
            Ok(out)
        })?,
    )?;
    // Mouse presses / releases since the last call: {button, down, gui}.
    g.set(
        "mouse_edges",
        lua.create_function(|lua, ()| {
            let evs = std::mem::take(&mut crate::gui::share().mouse_edges);
            let out = lua.create_table()?;
            for (b, down, gui) in evs {
                let t = lua.create_table()?;
                t.set("button", b)?;
                t.set("down", down)?;
                t.set("gui", gui)?;
                out.push(t)?;
            }
            Ok(out)
        })?,
    )?;
    // The mouse's ray into the world this frame (origin, unit direction).
    g.set(
        "mouse_ray",
        lua.create_function(|lua, ()| {
            let (o, d) = world(lua)?.input.mouse_ray;
            Ok((
                LVector3(o).into_value(),
                LVector3(d.normalize_or_zero()).into_value(),
            ))
        })?,
    )?;
    // ParticleEmitter2D:Emit(n) / Clear() (n = 0) for the GUI walk.
    g.set(
        "p2d",
        lua.create_function(|_, (inst, n): (Value, u32)| {
            if let Some(r) = opt_inst(&inst) {
                crate::gui::share().p2d.push((r, n));
            }
            Ok(())
        })?,
    )?;
    g.set(
        "drags",
        lua.create_function(|lua, ()| {
            let evs = std::mem::take(&mut crate::gui::share().drags);
            let out = lua.create_table()?;
            for (r, kind, x, y) in evs {
                let t = lua.create_table()?;
                t.set("obj", inst_value(lua, r)?)?;
                t.set("kind", kind)?;
                t.set("x", x)?;
                t.set("y", y)?;
                out.push(t)?;
            }
            Ok(out)
        })?,
    )?;
    g.set(
        "hover_state",
        lua.create_function(|_, inst: Value| {
            let Some(r) = opt_inst(&inst) else { return Ok((false, false)) };
            let sh = crate::gui::share();
            Ok((sh.hovered.contains(&r), sh.pressed == Some(r)))
        })?,
    )?;
    g.set(
        "image_loaded",
        lua.create_function(|_, id: String| Ok(!id.is_empty() && crate::gui::share().loaded.contains(&id)))?,
    )?;
    g.set(
        "buttons",
        lua.create_function(|_, ()| {
            let b = crate::gui::share().buttons;
            Ok((b[0], b[1], b[2]))
        })?,
    )?;
    g.set(
        "content_text",
        lua.create_function(|_, s: String| Ok(crate::gui::content_text(&s)))?,
    )?;
    g.set(
        "set_page",
        lua.create_function(|_, (lay, from, to, alpha): (Value, Value, Value, f32)| {
            let Some(l) = opt_inst(&lay) else { return Ok(()) };
            let mut sh = crate::gui::share();
            match opt_inst(&to) {
                Some(t) => {
                    sh.pages.insert(l, (opt_inst(&from).unwrap_or(t), t, alpha));
                }
                None => {
                    sh.pages.remove(&l);
                }
            }
            Ok(())
        })?,
    )?;
    g.set(
        "camera",
        lua.create_function(|lua, ()| {
            // The client camera's position (BillboardGui.CurrentDistance).
            let w = world(lua)?;
            let d = w.d();
            let cam = d
                .workspace()
                .and_then(|ws| match d.prop(ws, "CurrentCamera") {
                    Some(Variant::Ref(c)) if d.exists(c) => Some(c),
                    _ => None,
                });
            Ok(match cam.and_then(|c| d.prop(c, "CFrame")) {
                Some(Variant::CFrame(cf)) => (cf.position.x as f64, cf.position.y as f64, cf.position.z as f64, true),
                _ => (0.0, 0.0, 0.0, false),
            })
        })?,
    )?;
    g.set(
        "make_object",
        lua.create_function(|_, (class, methods): (String, Table)| {
            methods.raw_set("__class", class.as_str())?;
            Ok(LObject { class, methods })
        })?,
    )?;
    // Studio's own proximity prompt UI (CoreScripts/ProximityPrompt.lua from
    // the Studio install), with its two CoreGui-only requires shimmed.
    g.set(
        "prompt_ui",
        lua.create_function(|lua, ()| {
            let Some(dir) = crate::default_scripts::studio_dir() else { return Ok(Value::Nil) };
            let path = dir.join("ExtraContent/scripts/CoreScripts/CoreScripts/ProximityPrompt.lua");
            let Ok(mut src) = std::fs::read_to_string(&path) else { return Ok(Value::Nil) };
            let shims: [(&str, &str); 6] = [
                ("local CoreGui = game:GetService(\"CoreGui\")\n", ""),
                ("local CorePackages = game:GetService(\"CorePackages\")\n", ""),
                ("local RobloxGui = CoreGui.RobloxGui\n", ""),
                (
                    "local CoreUtility = require(RobloxGui.Modules.CoreUtility)",
                    "local CoreUtility = { waitForChildOfClass = function(p, c) local x = p:FindFirstChildOfClass(c) while not x do p.ChildAdded:Wait() x = p:FindFirstChildOfClass(c) end return x end }",
                ),
                (
                    "local AppFonts = require(CorePackages.Workspace.Packages.Style).AppFonts",
                    "local AppFonts = { default = { getMedium = function() return Enum.Font.BuilderSansMedium end } }",
                ),
                ("UserInputService:GetPlatform() == Enum.Platform.OSX", "false"),
            ];
            for (from, to) in shims {
                if !src.contains(from) {
                    return Ok(Value::Nil);
                }
                src = src.replacen(from, to, 1);
            }
            let f = lua.load(src).set_name("=CoreGui.RobloxGui.CoreScripts/ProximityPrompt").set_environment(crate::globals_parity::new_env(lua, None)?).into_function()?;
            Ok(Value::Function(f))
        })?,
    )?;
    // Studio's legacy Dialog conversations (CoreScripts/MainBotChatScript2.lua
    // from the Studio install): its CoreGui-only pieces (RobloxGui, the
    // SetDialogInUse remote, three CoreGui modules, analytics, core actions,
    // SignalDialogChoiceSelected, the dialog collection) point at stand-ins
    // gui_input.luau passes in as `...`.
    g.set(
        "dialog_ui",
        lua.create_function(|lua, ()| {
            let Some(dir) = crate::default_scripts::studio_dir() else { return Ok(Value::Nil) };
            let path = dir.join("ExtraContent/scripts/CoreScripts/CoreScripts/MainBotChatScript2.lua");
            let Ok(mut src) = std::fs::read_to_string(&path) else { return Ok(Value::Nil) };
            let once: [(&str, &str); 14] = [
                ("local CoreGui = game:GetService(\"CoreGui\")", "local CoreGui = __D.coregui"),
                ("local CorePackages = game:GetService(\"CorePackages\")", ""),
                ("local RobloxGui = CoreGui.RobloxGui", "local RobloxGui = __D.robloxgui"),
                ("local RobloxReplicatedStorage = game:GetService('RobloxReplicatedStorage')", "local RobloxReplicatedStorage = __D.rrs"),
                ("local AnalyticsService = game:GetService(\"RbxAnalyticsService\")", "local AnalyticsService = __D.analytics"),
                ("require(RobloxGui.Modules.TenFootInterface)", "__D.tenfoot"),
                ("require(RobloxGui.Modules.Settings.Utility)", "__D.utility"),
                ("require(RobloxGui.Modules.GameTranslator)", "__D.translator"),
                ("currentConversationDialog:SignalDialogChoiceSelected(localPlayer, dialogChoice)", "__D.signal(currentConversationDialog, localPlayer, dialogChoice)"),
                ("game:GetService(\"Chat\"):ChatLocal(...)", "__D.chat_local(...)"),
                ("game:GetService(\"CollectionService\").ItemAdded", "__D.item_added"),
                ("game:GetService(\"CollectionService\").ItemRemoved", "__D.item_removed"),
                ("game:GetService(\"CollectionService\"):GetCollection(\"Dialog\")", "__D.dialogs()"),
                ("local getClosestDialogToPosition = guiService.GetClosestDialogToPosition", "local getClosestDialogToPosition = __D.closest"),
            ];
            for (from, to) in once {
                if !src.contains(from) {
                    return Ok(Value::Nil);
                }
                src = src.replacen(from, to, 1);
            }
            src = src
                .replace("RobloxLocked = true", "Archivable = true")
                .replace("BindCoreAction", "BindAction")
                .replace("UnbindCoreAction", "UnbindAction")
                .replace("SelectedCoreObject", "SelectedObject")
                .replace("game:GetEngineFeature(\"ClosestVisibleDialogAPI\")", "false");
            let src = format!("local __D = ...\n{src}");
            let f = lua.load(src).set_name("=CoreGui.RobloxGui.CoreScripts/MainBotChatScript2").set_environment(crate::globals_parity::new_env(lua, None)?).into_function()?;
            Ok(Value::Function(f))
        })?,
    )?;
    // TextGenerator: Studio's user-auth generateText call on a worker
    // thread (the login session); the Luau polls it.
    g.set(
        "textgen_start",
        lua.create_function(|_, (universe, body): (i64, String)| {
            let url = format!("https://apis.roblox.com/user/cloud/v2/universes/{universe}:generateText");
            Ok(super::avatar::web::start("POST", url, Some(body)))
        })?,
    )?;
    // A read-only GET through the login session (poll with textgen_poll).
    g.set(
        "web_get",
        lua.create_function(|_, url: String| {
            if !url.starts_with("https://avatar.roblox.com/") {
                return Err(rt_err("web_get: only the avatar API"));
            }
            Ok(super::avatar::web::start("GET", url, None))
        })?,
    )?;
    g.set(
        "textgen_poll",
        lua.create_function(|lua, id: u64| {
            Ok(match super::avatar::web::poll(id) {
                None => MultiValue::new(),
                Some(Ok((status, text))) => MultiValue::from_vec(vec![
                    Value::Integer(status as i64),
                    Value::String(lua.create_string(text)?),
                ]),
                Some(Err(e)) => MultiValue::from_vec(vec![Value::Integer(0), Value::String(lua.create_string(e)?)]),
            })
        })?,
    )?;
    g.set(
        "http_error",
        lua.create_function(|_, status: u16| Ok(super::avatar::web::http_error(status)))?,
    )?;
    // The inspect menu: set(view table or nil) / closed() (taken once).
    g.set(
        "inspect_set",
        lua.create_function(|_, v: Option<Table>| {
            let mut sh = crate::gui::share();
            sh.inspect = match v {
                None => None,
                Some(t) => {
                    let mut items = Vec::new();
                    if let Ok(list) = t.get::<Table>("items") {
                        for it in list.sequence_values::<Table>() {
                            let it = it?;
                            items.push((it.get::<String>("name").unwrap_or_default(), it.get::<Option<i64>>("price").unwrap_or(None)));
                        }
                    }
                    let scroll = sh.inspect.as_ref().map(|i| i.scroll).unwrap_or(0.0);
                    Some(crate::gui::InspectView {
                        title: t.get("title").unwrap_or_default(),
                        robux: t.get("robux").unwrap_or(0),
                        items,
                        loading: t.get("loading").unwrap_or(false),
                        scroll,
                    })
                }
            };
            Ok(())
        })?,
    )?;
    g.set(
        "inspect_closed",
        lua.create_function(|_, ()| Ok(std::mem::take(&mut crate::gui::share().inspect_closed)))?,
    )?;
    g.set("new_input_object", lua.create_function(|lua, t: Table| input_object(lua, t))?)?;
    // send(inst, kind, ...): an engine message to the server's copy of inst
    // (the client's prompt input; see `engine_message`).
    g.set(
        "send",
        lua.create_function(|lua, args: MultiValue| {
            let mut it = args.into_iter();
            let r = it.next().as_ref().and_then(opt_inst).ok_or_else(|| rt_err("send: Instance expected"))?;
            let mut xs = vec![Xfer::Str(ENGINE_MSG.to_vec())];
            xs.extend(it.map(|v| export(&v, 0)));
            let mut w = world(lua)?;
            if w.side == Side::Client {
                w.outbox.push(RemoteMsg { inst: r, kind: RemoteKind::Event, args: xs });
            }
            Ok(())
        })?,
    )?;
    let m: Table = lua
        .load(include_str!("gui_input.luau"))
        .set_name("=rustudio.gui_input")
        .call(g)?;
    m.set("defaults", lua.create_table()?)?;
    lua.set_named_registry_value("rs_gui_input", m)?;
    Ok(())
}

/// First argument marking a client → server message as the engine's own
/// (not a RemoteEvent's).
const ENGINE_MSG: &[u8] = b"\x01rustudio.gui_input";

/// Whether a client → server message is one of this domain's (play.rs
/// hands those to `engine_message` instead of OnServerEvent).
pub fn is_engine_msg(args: &[Xfer]) -> bool {
    matches!(args.first(), Some(Xfer::Str(s)) if s.as_slice() == ENGINE_MSG)
}

/// The server's side of an engine message: the prompt events the client's
/// input caused, fired on the server's prompt (gui_input.luau remote_in).
pub fn engine_message(rt: &Runtime, inst: Ref, player: Ref, args: &[Xfer], map: &dyn Fn(Ref) -> Option<Ref>) {
    let lua = &rt.lua;
    let run = || -> mlua::Result<()> {
        let f: Function = module(lua)?.get("remote_in")?;
        let mut vals = vec![inst_value(lua, inst)?, inst_value(lua, player)?];
        for a in args.iter().skip(1) {
            vals.push(import(lua, a, map)?);
        }
        f.call::<()>(MultiValue::from_vec(vals))
    };
    if let Err(e) = run() {
        if let Ok(mut w) = world(lua) {
            w.output.push((crate::app::Level::Error, format!("gui_input: {}", error_message(&e))));
        }
    }
    rt.dispatch();
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    let Ok(m) = module(lua) else { return Ok(()) };
    let f: Function = m.get("step")?;
    if let Err(e) = f.call::<()>(dt) {
        if let Ok(mut w) = world(lua) {
            w.output.push((crate::app::Level::Error, format!("gui_input step: {}", error_message(&e))));
        }
    }
    Ok(())
}
