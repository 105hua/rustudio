//! DebuggerManager, ScriptDebugger, DebuggerBreakpoint and DebuggerWatch:
//! Studio's legacy script debugger as Studio (build 1708278) has it today.
//! Measured: every script in the DataModel has a ScriptDebugger under
//! DebuggerManager ("<Script>Debugger", DebuggerAdded when the script comes
//! in, DebuggerRemoved and Parent = nil when it goes); SetBreakpoint makes
//! (or returns) "Breakpoint<line>", AddWatch a DebuggerWatch, each firing
//! its Added event and its Removed event when destroyed; IsDebugging turns
//! true once the script runs. The breakpoints never stop a script any more
//! (Studio's Luau debugger replaced this one), so IsPaused stays false,
//! EncounteredBreak / Resuming never fire, the stack / variable calls give
//! "Cannot perform this operation unless the thread is paused" and the
//! steps "Can't step while running". Game scripts can't reach the manager
//! in Studio (GetService gives nil; DebuggerManager() needs Plugin).

use std::collections::{HashMap, HashSet};

use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::ustr;

use super::*;

#[derive(Default)]
struct DbgState {
    /// script → its ScriptDebugger.
    by_script: HashMap<Ref, Ref>,
    /// ScriptDebugger → its script.
    script_of: HashMap<Ref, Ref>,
    /// What each debugger had last frame (to fire the Removed events).
    breakpoints: HashMap<Ref, Vec<Ref>>,
    watches: HashMap<Ref, Vec<Ref>>,
    /// A script asked for the debuggers: keep them in step with the scripts.
    tracking: bool,
    /// Frames since the last full look at the scripts.
    frames: u32,
}

fn st(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, DbgState>> {
    if lua.app_data_ref::<DbgState>().is_none() {
        lua.set_app_data(DbgState::default());
    }
    lua.app_data_mut::<DbgState>()
        .ok_or_else(|| rt_err("debugger state missing"))
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(DbgState::default());
    Ok(())
}

const PAUSED_ONLY: &str = "Cannot perform this operation unless the thread is paused";

fn is_script(class: &str) -> bool {
    matches!(class, "Script" | "LocalScript" | "ModuleScript")
}

fn set_props(lua: &Lua, r: Ref, props: Vec<(&str, Variant)>) -> mlua::Result<()> {
    let mut w = world(lua)?;
    if let Some(i) = w.dm().dom.get_by_ref_mut(r) {
        for (k, v) in props {
            i.properties.insert(ustr(k), v);
        }
    }
    Ok(())
}

fn fire(lua: &Lua, r: Ref, event: &str, arg: Ref) -> mlua::Result<()> {
    let v = inst_value(lua, arg)?;
    world(lua)?.fire(r, event, vec![v]);
    Ok(())
}

/// The ScriptDebugger of `script` under the manager `dm`, made (and
/// announced) if it has none.
fn debugger_for(lua: &Lua, dm: Ref, script: Ref) -> mlua::Result<Ref> {
    let have = st(lua)?.by_script.get(&script).copied();
    if let Some(d) = have {
        if world(lua)?.d().dom.get_by_ref(d).is_some_and(|i| i.parent() == dm) {
            return Ok(d);
        }
    }
    let name = format!("{}Debugger", name_of(lua, script)?);
    let d = new_object(lua, "ScriptDebugger", &name, dm)?;
    {
        let mut s = st(lua)?;
        s.by_script.insert(script, d);
        s.script_of.insert(d, script);
    }
    fire(lua, dm, "DebuggerAdded", d)?;
    Ok(d)
}

fn manager(lua: &Lua) -> mlua::Result<Option<Ref>> {
    service_ref(lua, "DebuggerManager")
}

/// Every script in the DataModel has its debugger; scripts gone take
/// theirs with them (DebuggerRemoved, then Parent = nil).
fn sync(lua: &Lua, dm: Ref) -> mlua::Result<()> {
    let scripts: Vec<Ref> = {
        let w = world(lua)?;
        let d = w.d();
        let root = d.dom.root_ref();
        d.dom
            .descendants_of(root)
            .filter(|i| is_script(i.class.as_str()))
            .map(|i| i.referent())
            .collect()
    };
    let live: HashSet<Ref> = scripts.iter().copied().collect();
    for s in scripts {
        debugger_for(lua, dm, s)?;
    }
    let gone: Vec<(Ref, Ref)> = st(lua)?
        .by_script
        .iter()
        .filter(|(s, _)| !live.contains(s))
        .map(|(s, d)| (*s, *d))
        .collect();
    for (s, d) in gone {
        {
            let mut st = st(lua)?;
            st.by_script.remove(&s);
            st.script_of.remove(&d);
            st.breakpoints.remove(&d);
            st.watches.remove(&d);
        }
        fire(lua, dm, "DebuggerRemoved", d)?;
        let mut w = world(lua)?;
        if w.d().dom.get_by_ref(d).is_some() {
            w.dm().dom.transfer_within(d, Ref::none());
            w.orphans.insert(d);
            w.structure_changed = true;
        }
    }
    Ok(())
}

fn children_of_class(lua: &Lua, r: Ref, class: &str) -> mlua::Result<Vec<Ref>> {
    let w = world(lua)?;
    let d = w.d();
    Ok(d.dom
        .get_by_ref(r)
        .map(|i| i.children().to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter(|c| d.class_of(*c).is_some_and(|k| k == class))
        .collect())
}

fn inst_list(lua: &Lua, refs: Vec<Ref>) -> mlua::Result<MultiValue> {
    let t = lua.create_table()?;
    for (i, r) in refs.into_iter().enumerate() {
        t.raw_set(i + 1, inst_value(lua, r)?)?;
    }
    ret1(Value::Table(t))
}

pub fn call(
    lua: &Lua,
    class: &str,
    name: &str,
    this: Ref,
    args: &MultiValue,
) -> Option<mlua::Result<MultiValue>> {
    let r = match (class, name) {
        ("DebuggerManager", "AddDebugger") => (|| {
            let script = opt_inst(&arg(args, 1))
                .filter(|s| {
                    world(lua)
                        .ok()
                        .and_then(|w| w.d().class_of(*s).map(|c| is_script(c.as_str())))
                        .unwrap_or(false)
                })
                .ok_or_else(|| rt_err("Can only add debugger for a Script"))?;
            st(lua)?.tracking = true;
            let d = debugger_for(lua, this, script)?;
            ret1(inst_value(lua, d)?)
        })(),
        ("DebuggerManager", "GetDebuggers") => (|| {
            st(lua)?.tracking = true;
            sync(lua, this)?;
            inst_list(lua, children_of_class(lua, this, "ScriptDebugger")?)
        })(),
        // Nothing is ever paused: Resume does nothing, steps refuse.
        ("DebuggerManager", "Resume") => ret0(),
        ("DebuggerManager", "StepIn" | "StepOut" | "StepOver") => {
            Err(rt_err("Can't step while running"))
        }
        ("ScriptDebugger", "SetBreakpoint") => (|| {
            let line = match arg(args, 1) {
                Value::Number(n) => n as i32,
                Value::Integer(n) => n as i32,
                Value::String(s) => s
                    .to_str()?
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| rt_err("Unable to cast string to int"))? as i32,
                _ => return Err(rt_err("Argument 1 missing or nil")),
            };
            let ctx = matches!(arg(args, 2), Value::Boolean(true));
            for b in children_of_class(lua, this, "DebuggerBreakpoint")? {
                if matches!(doc_prop(lua, b, "Line")?, Some(Variant::Int32(l)) if l == line) {
                    return ret1(inst_value(lua, b)?);
                }
            }
            let b = new_object(lua, "DebuggerBreakpoint", &format!("Breakpoint{line}"), this)?;
            set_props(
                lua,
                b,
                vec![
                    ("Line", Variant::Int32(line)),
                    ("Condition", Variant::String(String::new())),
                    ("IsEnabled", Variant::Bool(true)),
                    ("ContinueExecution", Variant::Bool(false)),
                    ("LogExpression", Variant::String(String::new())),
                    ("isContextDependentBreakpoint", Variant::Bool(ctx)),
                ],
            )?;
            st(lua)?.breakpoints.entry(this).or_default().push(b);
            fire(lua, this, "BreakpointAdded", b)?;
            ret1(inst_value(lua, b)?)
        })(),
        ("ScriptDebugger", "GetBreakpoints") => children_of_class(lua, this, "DebuggerBreakpoint")
            .and_then(|v| inst_list(lua, v)),
        ("ScriptDebugger", "AddWatch") => (|| {
            let expr = match arg(args, 1) {
                Value::String(s) => s.to_string_lossy(),
                Value::Number(n) => num_str(n),
                Value::Integer(n) => n.to_string(),
                _ => return Err(rt_err("Argument 1 missing or nil")),
            };
            let wch = new_object(lua, "DebuggerWatch", "DebuggerWatch", this)?;
            set_props(lua, wch, vec![("Expression", Variant::String(expr))])?;
            st(lua)?.watches.entry(this).or_default().push(wch);
            fire(lua, this, "WatchAdded", wch)?;
            ret1(inst_value(lua, wch)?)
        })(),
        ("ScriptDebugger", "GetWatches") => children_of_class(lua, this, "DebuggerWatch")
            .and_then(|v| inst_list(lua, v)),
        ("ScriptDebugger", "GetWatchValue") => (|| {
            let ok = opt_inst(&arg(args, 1)).is_some_and(|w| {
                world(lua).is_ok_and(|wd| {
                    wd.d()
                        .dom
                        .get_by_ref(w)
                        .is_some_and(|i| i.class == "DebuggerWatch" && i.parent() == this)
                })
            });
            if !ok {
                return Err(rt_err("bad watch argument"));
            }
            Err(rt_err(PAUSED_ONLY))
        })(),
        ("ScriptDebugger", "GetStack" | "GetLocals" | "GetGlobals" | "GetUpvalues" | "SetLocal"
            | "SetGlobal" | "SetUpvalue") => Err(rt_err(PAUSED_ONLY)),
        _ => return None,
    };
    Some(r)
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    Ok(Some(match (class, key) {
        ("DebuggerManager", "DebuggingEnabled") => Value::Boolean(true),
        ("ScriptDebugger", "CurrentLine") => Value::Number(0.0),
        ("ScriptDebugger", "IsPaused") => Value::Boolean(false),
        ("ScriptDebugger", "IsDebugging") => {
            let s = st(lua)?.script_of.get(&r).copied();
            Value::Boolean(s.is_some_and(|s| world(lua).is_ok_and(|w| w.started.contains(&s))))
        }
        ("ScriptDebugger", "Script") => match st(lua)?.script_of.get(&r).copied() {
            Some(s) => inst_value(lua, s)?,
            None => Value::Nil,
        },
        ("DebuggerBreakpoint", "Line") => Value::Number(match doc_prop(lua, r, "Line")? {
            Some(Variant::Int32(l)) => l as f64,
            _ => 0.0,
        }),
        _ => return Ok(None),
    }))
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if matches!(class, "ScriptDebugger" | "DebuggerBreakpoint") && matches!(key, "Parent" | "parent") {
        return Err(rt_err(format!("Cannot change Parent of type {class}")));
    }
    Ok(false)
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    // Breakpoints and watches destroyed since last frame.
    let tracked: Vec<(Ref, Vec<Ref>, Vec<Ref>)> = {
        let s = st(lua)?;
        let mut keys: HashSet<Ref> = s.breakpoints.keys().copied().collect();
        keys.extend(s.watches.keys().copied());
        keys.into_iter()
            .map(|d| {
                (
                    d,
                    s.breakpoints.get(&d).cloned().unwrap_or_default(),
                    s.watches.get(&d).cloned().unwrap_or_default(),
                )
            })
            .collect()
    };
    for (d, bps, ws) in tracked {
        let gone = |x: &Ref| world(lua).is_ok_and(|w| w.d().dom.get_by_ref(*x).is_none_or(|i| i.parent() != d));
        let (bgone, bkeep): (Vec<Ref>, Vec<Ref>) = bps.into_iter().partition(|x| gone(x));
        let (wgone, wkeep): (Vec<Ref>, Vec<Ref>) = ws.into_iter().partition(|x| gone(x));
        if bgone.is_empty() && wgone.is_empty() {
            continue;
        }
        {
            let mut s = st(lua)?;
            s.breakpoints.insert(d, bkeep);
            s.watches.insert(d, wkeep);
        }
        for b in bgone {
            fire(lua, d, "BreakpointRemoved", b)?;
        }
        for x in wgone {
            fire(lua, d, "WatchRemoved", x)?;
        }
    }
    // Debuggers follow the scripts once a script looks at them, or listens.
    let Some(dm) = manager(lua)? else {
        return Ok(());
    };
    let listening = world(lua)?.listening.contains(&dm);
    if st(lua)?.tracking || listening {
        // (A full look at the scripts when the tree changed, else every
        // few frames: structure_changed may be taken before this step.)
        let changed = world(lua)?.structure_changed;
        let due = {
            let mut s = st(lua)?;
            s.frames += 1;
            let due = changed || !s.tracking || s.frames >= 6;
            if due {
                s.frames = 0;
                s.tracking = true;
            }
            due
        };
        if due {
            sync(lua, dm)?;
        }
    }
    Ok(())
}
