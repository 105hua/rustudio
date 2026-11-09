//! `rustudio --api-coverage OUT`: how much of Studio's API (app/api-dump.json)
//! rustudio implements, measured through the gap telemetry rather than
//! estimated. Headless: one instance of every class in the dump is put in a
//! place (services at the root, the rest in a folder, so even classes a
//! script can't create are covered), then a script running in the real
//! runtime reads every property and indexes every function and event in
//! pcall. A member counts as implemented when that worked and the telemetry
//! recorded no gap for it (`missing` for unknown members, `default-only` for
//! run-time properties answered with a default). Functions are indexed, not
//! called (calls have side effects); a method that resolves but has nothing
//! behind it shows up as `call-unimplemented` in the telemetry of real runs
//! and is subtracted when such a gap file is given (`--with-gaps FILE…`).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use mlua::{Table, Value};
use rbx_dom_weak::InstanceBuilder;

use crate::api::Kind;
use crate::doc::Document;
use crate::runtime::{RunMode, Runtime, World};

const PROBE: &str = r#"
local folder = game:GetService("ReplicatedStorage"):FindFirstChild("__CoverageProbe")
-- Objects rustudio builds in Luau, got the way scripts get them.
local made = {}
local factories = {
	["@AnimationTrack"] = function()
		local h = Instance.new("Humanoid")
		local an = Instance.new("Animator")
		an.Parent = h
		return an:LoadAnimation(Instance.new("Animation"))
	end,
	["@Tween"] = function()
		return game:GetService("TweenService"):Create(Instance.new("Part"), TweenInfo.new(1), {})
	end,
	["@InputObject"] = function()
		return __rs_new_input_object({ UserInputType = Enum.UserInputType.Keyboard, KeyCode = Enum.KeyCode.A })
	end,
	["@Mouse"] = function()
		return folder:FindFirstChild("Player"):GetMouse()
	end,
}
local function target(t)
	if t == "game" then return game end
	if factories[t] then
		if made[t] == nil then
			local ok, v = pcall(factories[t])
			made[t] = ok and v or false
		end
		return made[t] or nil
	end
	local inst = folder:FindFirstChild(t)
	if inst then return inst end
	local ok, svc = pcall(function() return game:GetService(t) end)
	return ok and svc or game:FindFirstChildOfClass(t)
end
local fails = {}
for _, entry in __cov do
	local class, name, t = entry[1], entry[2], entry[3]
	-- Class capability gates can reject even a file-loaded probe target.
	-- Record its members as unavailable instead of aborting the entire run.
	local target_ok, inst = pcall(target, t)
	if not target_ok then inst = nil end
	local ok, err = false, nil
	if inst ~= nil then ok, err = pcall(function() return inst[name] end) end
	-- A callback exists when reading it gives Studio's "callback member" error.
	if not ok and tostring(err):find("is a callback member", 1, true) then ok = true end
	if not ok then
		fails[class .. "." .. name] = true
	end
end
__cov_fails = fails
"#;

pub fn run(out: &Path, gap_files: &[std::path::PathBuf]) -> Result<serde_json::Value, String> {
    let api = crate::api::get().ok_or("app/api-dump.json not found (RUSTUDIO_API_DUMP)")?;
    crate::telemetry::enable();

    let mut doc = Document::new_place();
    let root = doc.dom.root_ref();
    let rs = doc
        .find_child(root, "ReplicatedStorage")
        .ok_or("no ReplicatedStorage")?;
    let folder = doc.dom.insert(
        rs,
        InstanceBuilder::new("Folder").with_name("__CoverageProbe"),
    );
    for name in &api.class_order {
        let c = &api.classes[name];
        if c.service {
            if doc.find_child(root, name).is_none() {
                doc.dom
                    .insert(root, InstanceBuilder::new(name.as_str()).with_name(name.as_str()));
            }
        } else if name != "DataModel" {
            doc.dom.insert(
                folder,
                InstanceBuilder::new(name.as_str()).with_name(name.as_str()),
            );
        }
    }

    let rt = Runtime::new()?;
    rt.enter(World::new(doc, RunMode::Play));
    rt.bind_globals().map_err(|e| e.to_string())?;
    // Every declared member, on an instance of its declaring class.
    let entries: Vec<(&str, &str, Kind, &str)> = api
        .class_order
        .iter()
        .flat_map(|c| {
            let class = &api.classes[c];
            class.order.iter().map(move |m| {
                let info = &class.members[m];
                (c.as_str(), m.as_str(), info.kind, info.security.as_str())
            })
        })
        .collect();
    // Where each class's members are tried: itself when it can exist on its
    // own (creatable or a service), a concrete subclass for abstract ones
    // (ServiceProvider → the DataModel, BasePart → Part…), and the Luau-built
    // objects through their factories.
    let target = |c: &str| -> String {
        match c {
            "AnimationTrack" => return "@AnimationTrack".into(),
            "Tween" => return "@Tween".into(),
            "InputObject" => return "@InputObject".into(),
            "Mouse" | "PlayerMouse" => return "@Mouse".into(),
            "DataModel" | "ServiceProvider" => return "game".into(),
            _ => {}
        }
        let info = &api.classes[c];
        if info.creatable || info.service {
            return c.to_string();
        }
        api.class_order
            .iter()
            .find(|sub| api.classes[sub.as_str()].creatable && api.is_a(sub, c))
            .cloned()
            .unwrap_or_else(|| c.to_string())
    };
    let lua = &rt.lua;
    let list = lua.create_table().map_err(|e| e.to_string())?;
    for (i, (c, m, _, _)) in entries.iter().enumerate() {
        let t = target(c);
        let row = lua
            .create_sequence_from([c.to_string(), m.to_string(), t])
            .map_err(|e| e.to_string())?;
        list.set(i + 1, row).map_err(|e| e.to_string())?;
    }
    lua.globals().set("__cov", list).map_err(|e| e.to_string())?;
    let f = lua
        .load(PROBE)
        .set_name("CoverageProbe")
        .into_function()
        .map_err(|e| e.to_string())?;
    f.call::<()>(()).map_err(|e| e.to_string())?;
    let fails: HashSet<String> = lua
        .globals()
        .get::<Table>("__cov_fails")
        .map(|t| {
            t.pairs::<String, Value>()
                .filter_map(|p| p.ok().map(|(k, _)| k))
                .collect()
        })
        .unwrap_or_default();
    let _ = rt.leave();

    // Gaps the probe itself saw (a stub answering counts against "real"),
    // plus calls without behaviour from real runs.
    let mut flagged: HashSet<(String, String)> = HashSet::new();
    let mut take = |v: &serde_json::Value, kinds: &[&str]| {
        for g in v["gaps"].as_array().into_iter().flatten() {
            if kinds.contains(&g["kind"].as_str().unwrap_or("")) {
                flagged.insert((
                    g["class"].as_str().unwrap_or("").to_string(),
                    g["member"].as_str().unwrap_or("").to_string(),
                ));
            }
        }
    };
    take(&crate::telemetry::snapshot(), &["missing", "default-only", "stub"]);
    for path in gap_files {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                take(&v, &["call-unimplemented", "stub-call"]);
            }
        }
    }

    #[derive(Default)]
    struct Tally {
        total: usize,
        exists: usize,
        done: usize,
        game_total: usize,
        game_exists: usize,
        game_done: usize,
    }
    let mut tally: BTreeMap<Kind, Tally> = BTreeMap::new();
    let mut missing: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut missing_events: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut missing_deprecated = 0usize;
    for (c, m, kind, security) in &entries {
        let exists = !fails.contains(&format!("{c}.{m}"));
        // A callback can't be read, so it has real behaviour only when the
        // runtime calls it (crate::runtime::stubs::REAL_CALLBACKS).
        let ok = exists
            && !flagged.contains(&(c.to_string(), m.to_string()))
            && (*kind != Kind::Callback || crate::runtime::stubs::REAL_CALLBACKS.contains(m));
        if !ok && api.classes[*c].members[*m].deprecated {
            missing_deprecated += 1;
        }
        let t = tally.entry(*kind).or_default();
        let game = *security == "None";
        t.total += 1;
        t.exists += usize::from(exists);
        t.done += usize::from(ok);
        if game {
            t.game_total += 1;
            t.game_exists += usize::from(exists);
            t.game_done += usize::from(ok);
        }
        if !ok && game && matches!(kind, Kind::Function | Kind::Property) {
            missing
                .entry(c.to_string())
                .or_default()
                .push(format!("{} {m}", kind.label()));
        }
        if !ok && game && *kind == Kind::Event {
            missing_events.entry(c.to_string()).or_default().push(m.to_string());
        }
    }
    let mut row = |k: Kind| {
        let t = tally.remove(&k).unwrap_or_default();
        serde_json::json!({
            "exists": t.exists, "implemented": t.done, "total": t.total,
            "game_scripts_exists": t.game_exists,
            "game_scripts_implemented": t.game_done, "game_scripts_total": t.game_total,
        })
    };
    let report = serde_json::json!({
        "dump": {
            "classes": api.classes.len(),
            "functions": api.count(Kind::Function, false),
            "properties": api.count(Kind::Property, false),
            "events": api.count(Kind::Event, false),
        },
        "missing_deprecated": missing_deprecated,
        "functions": row(Kind::Function),
        "properties": row(Kind::Property),
        "events": row(Kind::Event),
        "callbacks": row(Kind::Callback),
        "missing_game_members_by_class": missing,
        "missing_game_events_by_class": missing_events,
        "method": "every member of api-dump.json touched in pcall through the runtime on an instance of its class (functions indexed, not called); exists = no error (a generated stub with its typed default counts, src/api_stubs.rs); implemented (real behaviour) = exists and no gap telemetry (missing / default-only / stub), minus call-unimplemented and stub-call from the given real-run gap files",
    });
    std::fs::write(
        out,
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(report)
}
