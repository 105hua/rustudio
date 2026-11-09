//! The Luau ↔ Instance/Enum bridge as scripts observe it (docs/RE-BRIDGE.md;
//! src/cmeta.rs): metatables, namecall, method functions, Lua-written
//! methods that yield, Enum types and the index/newindex errors.

use super::*;

/// Runs `src` as a client LocalScript for `frames` steps; Output lines
/// without their location suffix.
fn run(src: &str, frames: usize) -> Vec<String> {
    let mut d = Document::new_place();
    let sp = d.find_child(d.dom.root_ref(), "StarterPlayer").unwrap();
    let sps = d.find_child(sp, "StarterPlayerScripts").unwrap();
    let s = d.dom.insert(
        sps,
        InstanceBuilder::new("LocalScript").with_name("T").with_property("Source", src.to_string()),
    );
    let rt = Runtime::new().unwrap();
    let mut w = World::new(d, RunMode::Run);
    w.side = Side::Client;
    rt.enter(w);
    rt.bind_globals().unwrap();
    rt.run_script(s);
    for _ in 0..frames {
        rt.step(1.0 / 60.0);
    }
    rt.leave()
        .output
        .iter()
        .map(|(_, t)| t.split("  —").next().unwrap_or("").to_string())
        .collect()
}

fn expect(text: &[String], wants: &[&str]) {
    for want in wants {
        assert!(text.iter().any(|t| t == want), "{want:?} not in {text:#?}");
    }
}

#[test]
fn bridge_signal_connect() {
    let text = run(
        r#"
        local p = Instance.new("Part")
        local n = 0
        local c = p.Changed:Connect(function() n += 1 end)
        p.Name = "X"
        print("conn", typeof(c), tostring(c), c.Connected, n)
        c:Disconnect()
        print("disc", c.Connected)
        "#,
        2,
    );
    expect(&text, &["conn RBXScriptConnection Connection true 1", "disc false"]);
}

/// A Lua prelude for the tests: `e(f, ...)` is pcall's message with any
/// position as "LOC: ", `info(f)` debug.info's s/l/n/a.
const LIB: &str = r#"
local function e(f, ...)
    -- Called from a Lua function, as a script line would (pcall itself
    -- is C, and Luau's argument errors name their caller's position).
    local ok, m = pcall(function(...) return f(...) end, ...)
    if ok then return "ok" end
    return (string.gsub(tostring(m), "^[^:]*:%d+: ", "LOC: "))
end
local function info(f)
    local s, l, n, a, v = debug.info(f, "slna")
    return table.concat({ s, l, n, a, tostring(v) }, " ")
end
local function raiser(f)
    local m
    xpcall(f, function() m = debug.info(2, "f") end)
    return m
end
"#;

fn run_lib(src: &str, frames: usize) -> Vec<String> {
    run(&format!("{LIB}\n{src}"), frames)
}

/// docs/RE-BRIDGE.md 1: every bridge object's metatable is locked.
#[test]
fn bridge_metatables_locked() {
    let text = run_lib(
        r#"
        local p = Instance.new("Part")
        local c = p.Changed:Connect(function() end)
        for _, o in { game, p, p.Changed, c, Enum, Enum.Material, Enum.Material.Plastic } do
            print("mt", typeof(o), getmetatable(o))
        end
        "#,
        1,
    );
    for t in ["Instance", "RBXScriptSignal", "RBXScriptConnection", "Enums", "Enum", "EnumItem"] {
        expect(&text, &[&format!("mt {t} The metatable is locked")]);
    }
}

/// docs/RE-BRIDGE.md 2: the Instance __namecall is its own unnamed C function,
/// methods only; argument checks as Luau's.
#[test]
fn bridge_namecall() {
    let text = run_lib(
        r#"
        local p = Instance.new("Part")
        local idx = raiser(function() return p.____________ end)
        local nc = raiser(function() p:____________() end)
        print("nc", info(nc), nc ~= idx, raiser(function() game:XYZ() end) == nc)
        print("empty", e(nc))
        print("self", e(nc, p))
        print("prop", e(function() workspace:Name() end))
        print("event", e(function() p:Changed() end))
        print("case", e(function() game:getservice("Players") end))
        print("child", e(function() local f = Instance.new("Folder"); Instance.new("Folder", f).Name = "K"; f:K() end))
        print("ok", game:GetService("Players").ClassName, p:IsA("BasePart"), workspace:FindFirstChild("nope"))
        print("game", e(function() game:XYZ() end))
        "#,
        1,
    );
    expect(
        &text,
        &[
            "nc [C] -1  0 true true true",
            "empty LOC: missing argument #1 (Instance expected)",
            // The VM's last namecall atom (game:XYZ() above), as Studio's.
            "self XYZ is not a valid member of Part \"Part\"",
            "prop Name is not a valid member of Workspace \"Workspace\"",
            "event Changed is not a valid member of Part \"Part\"",
            "case getservice is not a valid member of DataModel \"Game\"",
            "child K is not a valid member of Folder \"Folder\"",
            "ok Players true nil",
            "game XYZ is not a valid member of DataModel \"Game\"",
        ],
    );
}

/// docs/RE-BRIDGE.md 3: method functions are named vararg C functions, one per
/// descriptor; bad self is "Expected ':' not '.'"; GetService converts.
#[test]
fn bridge_method_functions() {
    let text = run_lib(
        r#"
        local p = Instance.new("Part")
        print("info", info(game.GetService), info(p.WaitForChild))
        print("id", game.GetService == game.GetService, p.IsA == game.IsA, p.FindFirstChild == workspace.FindFirstChild)
        print("noself", e(game.GetService))
        print("wrongself", e(game.GetService, {}))
        print("wrongclass", e(game.GetService, workspace))
        print("args", e(game.GetService, game, {}), e(game.GetService, game, true), e(game.GetService, game, 3))
        print("missing", e(game.GetService, game))
        print("abstract", game:GetService("WorldRoot"))
        "#,
        1,
    );
    expect(
        &text,
        &[
            "info [C] -1 GetService 0 true [C] -1 WaitForChild 0 true",
            "id true true true",
            "noself Expected ':' not '.' calling member function GetService",
            "wrongself Expected ':' not '.' calling member function GetService",
            "wrongclass Expected ':' not '.' calling member function GetService",
            "args '{}' is not a valid Service name 'true' is not a valid Service name '3' is not a valid Service name",
            "missing Argument 1 missing or nil",
            "abstract nil",
        ],
    );
}

/// docs/RE-BRIDGE.md 6: Lua-written methods are named C functions that check
/// self first and still yield, called as methods, as functions, inside
/// pcall and inside coroutines.
#[test]
fn bridge_lua_methods_yield() {
    let text = run_lib(
        r#"
        local f = Instance.new("Folder", workspace)
        f.Name = "F"
        task.delay(0.05, function() Instance.new("Part", f).Name = "Late" end)
        print("wfc", f:WaitForChild("Late").Name)
        local bf = Instance.new("BindableFunction")
        bf.OnInvoke = function(x) task.wait(0.02) return x * 2 end
        print("invoke", bf:Invoke(21), bf.Invoke(bf, 4))
        print("pcall", pcall(bf.Invoke, bf, 5))
        -- A coroutine of the script's own, which the wait yields: the
        -- scheduler resumes it through the method's continuation.
        local co = coroutine.create(function() print("co", f:WaitForChild("Late2").Name, f.WaitForChild(f, "Late2").Name) end)
        print("resume", coroutine.resume(co), coroutine.status(co))
        task.delay(0.05, function() Instance.new("Part", f).Name = "Late2" end)
        task.wait(0.2)
        print("timeout", f:WaitForChild("Never", 0.05))
        print("noself", e(f.WaitForChild), e(f.WaitForChild, {}))
        print("noname", e(f.WaitForChild, f), e(function() f:WaitForChild() end))
        print("rf", e(Instance.new("RemoteFunction").InvokeServer, workspace))
        print("info", info(bf.Invoke), info(Instance.new("RemoteFunction").InvokeServer))
        local sig = f.ChildAdded
        print("sig", info(sig.Connect), sig.Connect == sig.Connect)
        print("sigself", e(sig.Connect), e(sig.Connect, {}), e(sig.Connect, workspace))
        task.delay(0.05, function() Instance.new("Part", f) end)
        print("wait", sig:Wait().ClassName)
        print("done")
        "#,
        60,
    );
    expect(
        &text,
        &[
            "wfc Late",
            "invoke 42 8",
            "pcall true 10",
            "resume true suspended",
            "co Late2 Late2",
            "timeout nil",
            "noself Expected ':' not '.' calling member function WaitForChild Expected ':' not '.' calling member function WaitForChild",
            "noname Argument 1 missing or nil Argument 1 missing or nil",
            "rf Expected ':' not '.' calling member function InvokeServer",
            "info [C] -1 Invoke 0 true [C] -1 InvokeServer 0 true",
            "sig [C] -1 Connect 0 true false",
            "sigself LOC: missing argument #1 to 'Connect' (RBXScriptSignal expected) LOC: invalid argument #1 to 'Connect' (RBXScriptSignal expected, got table) LOC: invalid argument #1 to 'Connect' (RBXScriptSignal expected, got Instance)",
            "wait Part",
            "done",
        ],
    );
}

/// docs/RE-BRIDGE.md 5: Enums, Enum and EnumItem.
#[test]
fn bridge_enums() {
    let text = run_lib(
        r#"
        local m, it = Enum.Material, Enum.Material.Plastic
        print("str", tostring(Enum), tostring(m), tostring(it))
        print("get", #Enum:GetEnums() > 600, info(Enum.GetEnums), Enum.GetEnums == Enum.GetEnums)
        print("from", m:FromName("Plastic"), m:FromValue(256), m:FromName("Nope"), m:FromValue(-3))
        print("eq", Enum.Material == Enum.Material, it == Enum.Material.Plastic, it == Enum.Material.Wood)
        print("cmp", e(function() return it < Enum.Material.Wood end))
        print("fields", it.Name, it.Value, it.EnumType == m)
        print("wname", e(function() it.Name = 3 end), e(function() it.XYZ = 3 end))
        print("rxyz", e(function() return it.XYZ end), e(function() return it[1] end))
        print("enum", e(function() return m.Nope end), e(function() return Enum.Nope end))
        print("items", #m:GetEnumItems(), m:GetEnumItems()[1] == it)
        print("self", e(m.FromName), e(Enum.GetEnums, {}), e(it.IsA, workspace))
        print("isa", it:IsA("Material"), it:IsA("KeyCode"))
        "#,
        1,
    );
    expect(
        &text,
        &[
            "str Enums Material Enum.Material.Plastic",
            "get true [C] -1 GetEnums 0 true false",
            "from Enum.Material.Plastic Enum.Material.Plastic nil nil",
            "eq true true false",
            "cmp LOC: attempt to compare EnumItem < EnumItem",
            "fields Plastic 256 true",
            "wname Name is not a valid member of EnumItem XYZ is not a valid member of EnumItem",
            "rxyz XYZ is not a valid member of \"Enum.Material.Plastic\" 1 is not a valid member of \"Enum.Material.Plastic\"",
            "enum Nope is not a valid member of \"Enum.Material\" Nope is not a valid member of \"Enum\"",
            "items 45 true",
            "self LOC: missing argument #1 to 'FromName' (Enum expected) LOC: invalid argument #1 to 'GetEnums' (Enums expected, got table) LOC: invalid argument #1 to 'IsA' (EnumItem expected, got Instance)",
            "isa true false",
        ],
    );
}

/// docs/RE-BRIDGE.md 4: index/newindex argument and assignment errors.
#[test]
fn bridge_index_errors() {
    let text = run_lib(
        r#"
        local p = Instance.new("Part")
        local idx = raiser(function() return p.____________ end)
        local nidx = raiser(function() p.____________ = 1 end)
        print("i", e(idx), e(idx, p), e(idx, p, {}), e(idx, p, nil), e(idx, {}, "Name"))
        print("n", e(nidx), e(nidx, p), e(nidx, p, {}), e(function() p[nil] = 1 end))
        print("num", e(function() return p[1] end))
        print("ro", e(function() p.ClassName = "X" end))
        print("name", e(function() p.Name = {} end))
        print("parent", e(function() p.Parent = 3 end))
        print("anch", e(function() p.Anchored = {} end), p.Anchored)
        local c = p.Changed:Connect(function() end)
        print("conn", e(function() return c.Nope end), e(function() c.Nope = 1 end))
        print("sig", e(function() return p.Changed.Nope end), e(function() p.Changed.Nope = 1 end))
        "#,
        1,
    );
    expect(
        &text,
        &[
            "i LOC: missing argument #1 (Instance expected) LOC: missing argument #2 (string expected) LOC: invalid argument #2 (string expected, got table) LOC: invalid argument #2 (string expected, got nil) LOC: invalid argument #1 (Instance expected, got table)",
            "n LOC: missing argument #1 (Instance expected) LOC: missing argument #2 (string expected) LOC: invalid argument #2 (string expected, got table) LOC: invalid argument #2 (string expected, got nil)",
            "num 1 is not a valid member of Part \"Part\"",
            "ro Unable to assign property ClassName. Property is read only",
            "name Unable to assign property Name. string expected, got table",
            "parent LOC: invalid argument #3 (Instance expected, got number)",
            "anch ok true",
            "conn Nope is not a valid member of RBXScriptConnection Nope is not a valid member of RBXScriptConnection",
            "sig Nope is not a valid member of RBXScriptSignal Nope is not a valid member of RBXScriptSignal",
        ],
    );
}

/// Errors inside a Lua-written method: a script pcall that catches one in
/// an OnInvoke (this crashed the resume when the method's C function
/// called into Lua nested), one that escapes through the method, and one
/// after a yield.
#[test]
fn bridge_lua_method_errors() {
    let text = run_lib(
        r#"
        local bf = Instance.new("BindableFunction")
        bf.OnInvoke = function(k)
            if k == "catch" then return pcall(function() error("x", 0) end) end
            if k == "nil" then local y = nil; return y.z end
            task.wait(0.02)
            error("late", 0)
        end
        print("catch", bf:Invoke("catch"))
        print("pcallerr", pcall(function() return pcall(error, "y", 0) end))
        print("escape", e(function() return bf:Invoke("nil") end))
        print("late", e(function() return bf:Invoke("late") end))
        print("after", bf.Invoke(bf, "catch"))
        "#,
        30,
    );
    expect(
        &text,
        &[
            "catch false x",
            "pcallerr true false y",
            "escape LOC: attempt to index nil with 'z'",
            "late late",
            "after false x",
        ],
    );
}

/// RE-DATATYPES RBXScriptSignal.values: every read of an event is a new
/// object (`==`, not rawequal, distinct table keys) and connections made
/// through any of them work.
#[test]
fn bridge_signal_reads_are_fresh() {
    let text = run(
        r#"
        local p = Instance.new("Part")
        local a, b = p.Changed, p.Changed
        local t = { [a] = 1 }
        local n = 0
        a:Connect(function() n += 1 end)
        b:Connect(function() n += 10 end)
        p.Name = "X"
        print("sig", a == b, rawequal(a, b), t[b], a == p.AncestryChanged, n)
        "#,
        2,
    );
    expect(&text, &["sig true false nil false 11"]);
}

/// RE-DATATYPES RaycastResult rows: read-only userdata, Studio's tostring,
/// `==` by content.
#[test]
fn bridge_raycast_result_userdata() {
    let text = run(
        r#"
        local part = Instance.new("Part")
        part.Anchored = true
        part.Position = Vector3.new(10000, 10000, 10000)
        part.Parent = workspace
        local function cast() return workspace:Raycast(Vector3.new(10000, 10010, 10000), Vector3.new(0, -20, 0)) end
        local r, r2 = cast(), cast()
        print("ray", type(r), typeof(r), tostring(r), getmetatable(r))
        print("ray2", r == r2, rawequal(r, r2), r.Instance == part, r.Distance, r.Material)
        print("ray3", select(2, pcall(function() r.Distance = 1 end)), select(2, pcall(function() return r.Nope end)))
        "#,
        2,
    );
    expect(
        &text,
        &[
            "ray userdata RaycastResult RaycastResult{Part @ 10000, 10000.5996, 10000; normal = 0, 1, 0; material = Plastic} The metatable is locked",
            "ray2 true false true 9.399999618530273 Enum.Material.Plastic",
            "ray3 Distance member of RaycastResult is read-only and cannot be assigned to 'Nope' is not a valid member of RaycastResult",
        ],
    );
}

/// RE-GLOBALS misc.stats / misc.UserSettings / misc.DebuggerManager.
#[test]
fn bridge_settings_globals_are_instances() {
    let text = run(
        r#"
        local us = UserSettings()
        local ugs = us:GetService("UserGameSettings")
        print("us", typeof(us), tostring(us), us.ClassName, us:IsA("Instance"), us == UserSettings())
        print("ugs", typeof(ugs), ugs.ClassName, ugs.Parent == us, us.GameSettings == ugs, ugs.MouseSensitivity)
        local changed
        ugs.Changed:Connect(function(k) changed = k end)
        ugs.MouseSensitivity = 1
        task.wait()
        print("ugs2", ugs.MouseSensitivity, changed, us:IsUserFeatureEnabled("UserNewPoppercam4"), select(2, pcall(us.IsUserFeatureEnabled, us, "Bogus")))
        print("stats", typeof(stats()), tostring(stats()), stats() == game:GetService("Stats"))
        print("dm", pcall(DebuggerManager))
        "#,
        3,
    );
    expect(
        &text,
        &[
            "us Instance GlobalBasicSettings UserSettings true true",
            "ugs Instance UserGameSettings true true 0.52",
            "ugs2 1 MouseSensitivity true Flag Bogus does not exist",
            "stats Instance Stats true",
            "dm false The current thread cannot call 'DebuggerManager' (lacking capability Plugin)",
        ],
    );
}
