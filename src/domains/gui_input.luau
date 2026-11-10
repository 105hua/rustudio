-- rustudio engine domain gui_input (src/domains/gui_input.rs): GUI objects,
-- input services, the input action system, styles, haptics / VR / gamepad,
-- proximity prompts. Studio's behaviour measured in an offline Studio
-- (checks/parity/gui_input.md). Loaded once per runtime after the library.
local G = ...
local fire, log = G.fire, G.log
-- The current DataModel (refreshed every step; see G.game).
local game = game
local workspace = workspace
local function refresh_game()
    local ok, g = pcall(G.game)
    if ok and g then
        game = g
        local w = g:FindFirstChildOfClass("Workspace")
        if w then workspace = w end
    end
end

local methods, get, set = {}, {}, {}
local M = { methods = methods, get = get, set = set }
local LM = __rs_lua_methods -- library methods this domain takes over

-- A service, or nil when the DataModel's copy is gone (GetService can
-- hand back a destroyed one on a live client).
local function svc(name)
    local ok, s = pcall(game.GetService, game, name)
    if ok and s and pcall(function() return s.Parent end) then return s end
    return game:FindFirstChildOfClass(name)
end
local function changed(inst, prop)
    fire(inst, "Changed", prop)
    fire(inst, "Changed:" .. prop)
end
local function enum(e, name) return Enum[e][name] end
local function f32(x) return Vector3.new(x, 0, 0).X end
local function isa(v, c) return typeof(v) == "Instance" and v:IsA(c) end
local function is_enum(v, e)
    if typeof(v) ~= "EnumItem" then return false end
    local ok, item = pcall(function() return Enum[e][v.Name] end)
    return ok and item == v
end
local function argerr(n, fn, want, got)
    error(string.format("invalid argument #%d to '%s' (%s expected, got %s)", n, fn, want, typeof(got)), 0)
end
local function local_player()
    local ok, p = pcall(function() return svc("Players").LocalPlayer end)
    return ok and p or nil
end
local KEY_NONE
for _, k in Enum.KeyCode:GetEnumItems() do
    if k.Value == 0 then KEY_NONE = k break end
end
local function input_object(fields)
    local t = { KeyCode = KEY_NONE, UserInputState = Enum.UserInputState.None, UserInputType = Enum.UserInputType.None,
        Position = Vector3.zero, Delta = Vector3.zero }
    for k, v in fields do t[k] = v end
    return G.new_input_object(t)
end
-- Visible all the way up and in an enabled LayerCollector.
local function shown(o)
    local cur = o
    while cur do
        if cur:IsA("GuiObject") then
            if not cur.Visible then return false end
        elseif cur:IsA("LayerCollector") then
            return cur.Enabled
        elseif not cur:IsA("Folder") then
            return false
        end
        cur = cur.Parent
    end
    return false
end

-------------------------------------------------------- descendant tracking
-- Instances this domain keeps up with (StyleLinks, input contexts, prompts,
-- haptic effects), found once and then through DescendantAdded.
local tracked = { StyleLink = {}, InputContext = {}, ProximityPrompt = {}, HapticEffect = {}, Adorn = {} }
local tracking = false
local function track(i)
    local set_ = tracked[i.ClassName]
    if set_ then set_[i] = true end
    if i:IsA("HandleAdornment") or i:IsA("HandlesBase") then tracked.Adorn[i] = true end
end
local function start_tracking()
    if tracking then return end
    tracking = true
    for _, d in game:GetDescendants() do track(d) end
    -- Natively filtered DescendantAdded / Removing: no Lua call per instance.
    local added, removing = G.class_signals({ "StyleLink", "InputContext", "ProximityPrompt", "HapticEffect", "HandleAdornment", "HandlesBase" })
    added:Connect(track)
    removing:Connect(function(i)
        local set_ = tracked[i.ClassName]
        if set_ then set_[i] = nil end
        tracked.Adorn[i] = nil
    end)
end

------------------------------------------------------------------ GuiService
local gs_state = {
    paused_notification = true, emotes_open = false, inspect_enabled = true, inspect_open = false,
    nav = true, groups = {},
}
get["GuiService.CoreGuiNavigationEnabled"] = function() return gs_state.nav end
set["GuiService.CoreGuiNavigationEnabled"] = function(inst, _, v)
    if type(v) ~= "boolean" then error("Unable to assign property CoreGuiNavigationEnabled. bool expected, got " .. typeof(v), 0) end
    if gs_state.nav ~= v then gs_state.nav = v changed(inst, "CoreGuiNavigationEnabled") end
    return true
end
get["GuiService.PreferredTextSize"] = function() return Enum.PreferredTextSize.Medium end
get["GuiService.PreferredTransparency"] = function() return 1 end
get["GuiService.ReducedMotionEnabled"] = function() return false end
get["GuiService.TopbarInset"] = function() return Rect.new(0, 0, 0, 0) end
get["GuiService.ViewportDisplaySize"] = function() return Enum.DisplaySize.Medium end

methods["GuiService.AddSelectionParent"] = function(_, name, parent)
    gs_state.groups[tostring(name)] = { parent }
end
methods["GuiService.AddSelectionTuple"] = function(_, name, ...)
    gs_state.groups[tostring(name)] = { ... }
end
methods["GuiService.RemoveSelectionGroup"] = function(_, name) gs_state.groups[tostring(name)] = nil end
methods["GuiService.SendNotification"] = function() error("GuiService:SendNotification is not yet enabled!", 0) end
methods["GuiService.DismissNotification"] = function() error("GuiService:DismissNotification is not yet enabled!", 0) end
methods["GuiService.GetGameplayPausedNotificationEnabled"] = function() return gs_state.paused_notification end
methods["GuiService.SetGameplayPausedNotificationEnabled"] = function(_, on) gs_state.paused_notification = on == true end
methods["GuiService.SetEmotesMenuOpen"] = function(_, open) gs_state.emotes_open = open == true end
LM.GetEmotesMenuOpen = function() return gs_state.emotes_open end
methods["GuiService.SetInspectMenuEnabled"] = function(_, on) gs_state.inspect_enabled = on == true end
LM.GetInspectMenuEnabled = function() return gs_state.inspect_enabled end
-- The inspect menu (drawn by game_menu.rs). Studio opens it whatever
-- SetInspectMenuEnabled says (that only hides the player list's button):
-- MenuOpened and MenuIsOpen at the call, MenuClosed when it closes. It
-- lists the avatar's worn items with their prices (read-only lookups).
local inspect_seq = 0
local function close_inspect()
    if not gs_state.inspect_open then return end
    gs_state.inspect_open = false
    inspect_seq += 1
    G.inspect_set(nil)
    fire(svc("GuiService"), "MenuClosed")
end
local function open_inspect(title_fn, desc_fn)
    local was = gs_state.inspect_open
    gs_state.inspect_open = true
    inspect_seq += 1
    local seq = inspect_seq
    G.inspect_set({ title = "", robux = 0, items = {}, loading = true })
    if not was then fire(svc("GuiService"), "MenuOpened") end
    task.spawn(function()
        local title = select(2, pcall(title_fn)) or ""
        if seq ~= inspect_seq then return end
        G.inspect_set({ title = tostring(title) .. "'s Avatar", robux = 0, items = {}, loading = true })
        local ok, desc = pcall(desc_fn)
        local ids = {}
        if ok and type(desc) == "table" then
            for _, id in desc do table.insert(ids, id) end
        elseif ok and typeof(desc) == "Instance" then
            local function add(id) id = tonumber(id) if id and id > 0 and not table.find(ids, id) then table.insert(ids, id) end end
            for _, a in desc:GetAccessories(true) do add(a.AssetId) end
            for _, p in { "Shirt", "Pants", "GraphicTShirt", "Face", "Head", "Torso", "LeftArm", "RightArm", "LeftLeg", "RightLeg",
                "IdleAnimation", "WalkAnimation", "RunAnimation", "JumpAnimation", "FallAnimation", "ClimbAnimation", "SwimAnimation" } do
                add(desc[p])
            end
        end
        local items, total = {}, 0
        local mps = svc("MarketplaceService")
        for _, id in ids do
            if seq ~= inspect_seq then return end
            local okp, info = pcall(function() return mps:GetProductInfoAsync(id) end)
            if okp and type(info) == "table" then
                local price = info.IsForSale and tonumber(info.PriceInRobux) or nil
                if price then total += price end
                table.insert(items, { name = tostring(info.Name), price = price })
            end
        end
        if seq ~= inspect_seq then return end
        G.inspect_set({ title = tostring(title) .. "'s Avatar", robux = total, items = items, loading = false })
    end)
end
methods["GuiService.InspectPlayerFromUserId"] = function(_, id)
    id = tonumber(id)
    if id == nil or id <= 0 then error("GuiService:InspectPlayerFromUserId - Got invalid userId.", 0) end
    local players = svc("Players")
    open_inspect(function()
        local p = players:GetPlayerByUserId(id)
        if p then return p.DisplayName end
        return players:GetNameFromUserIdAsync(id)
    end, function()
        -- The worn assets, as the avatar API lists them.
        local job = G.web_get("https://avatar.roblox.com/v1/users/" .. math.floor(id) .. "/avatar")
        local status, text
        repeat
            task.wait()
            status, text = G.textgen_poll(job)
        until status ~= nil
        if status ~= 200 then error(text) end
        local data = svc("HttpService"):JSONDecode(text)
        local ids = {}
        for _, a in data.assets or {} do table.insert(ids, a.id) end
        return ids
    end)
end
methods["GuiService.InspectPlayerFromHumanoidDescription"] = function(_, desc, name)
    if not isa(desc, "HumanoidDescription") then error("Unable to cast value to Object", 0) end
    open_inspect(function() return tostring(name) end, function() return desc end)
end
methods["GuiService.CloseInspectMenu"] = function() close_inspect() end
get["GuiService.MenuIsOpen"] = function() return gs_state.inspect_open or G.menu_open() end
local function step_inspect()
    if G.inspect_closed() then close_inspect() end
end

-- GetInsetArea: the screen less what the given insets keep clear, in GUI
-- coordinates (Studio's Play: the 58 px top bar is above y = 0).
methods["GuiService.GetInsetArea"] = function(_, insets)
    if not is_enum(insets, "ScreenInsets") then
        error("Unable to cast " .. typeof(insets) .. " to token", 0)
    end
    local w, h = G.view()
    local inset = G.is_client() and 58 or 0
    if insets == Enum.ScreenInsets.CoreUISafeInsets then
        return Rect.new(0, 0, w, h - inset)
    end
    return Rect.new(0, -inset, w, h - inset)
end

local function selectable(o)
    return isa(o, "GuiObject") and o.Selectable and shown(o) and o.AbsoluteSize.X > 0 and o.AbsoluteSize.Y > 0
end
-- The first selectable object under `parent`, top-most then left-most.
local function first_selectable(parent)
    local best, by, bx
    for _, d in parent:GetDescendants() do
        if selectable(d) then
            local p = d.AbsolutePosition
            if not best or p.Y < by or (p.Y == by and p.X < bx) then
                best, by, bx = d, p.Y, p.X
            end
        end
    end
    return best
end
methods["GuiService.Select"] = function(self, parent)
    if typeof(parent) ~= "Instance" or not parent:FindFirstAncestorWhichIsA("BasePlayerGui") then
        error("Calling GuiService:Select() on an instance that is not a descendant of a BasePlayerGui", 0)
    end
    local pick = first_selectable(parent)
    if pick then self.SelectedObject = pick end
end

-- SelectedObject: SelectionLost on the old one, SelectionGained on the new,
-- the property's Changed, then SelectionChanged(amISelected, previous, new)
-- up through every GuiBase2d holding either (Studio's order). A selection
-- that can't be selected warns and moves to a selectable descendant.
local selected = nil
set["GuiService.SelectedObject"] = function(gs, _, v)
    if v ~= nil and not isa(v, "GuiObject") then
        error("Unable to assign property SelectedObject. Object expected, got " .. typeof(v), 0)
    end
    local old = gs.SelectedObject
    if old == v then return true end
    local invalid = v ~= nil and not v.Selectable
    if invalid then log("warn", "Setting GuiService.SelectedObject to invalid GuiObject") end
    if old and old.Parent then fire(old, "SelectionLost") end
    if v then fire(v, "SelectionGained") end
    __rs_set_raw(gs, "SelectedObject", v)
    selected = v
    local done = {}
    local function up(from)
        local cur = from
        while cur and cur ~= game do
            if cur:IsA("GuiBase2d") and not done[cur] then
                done[cur] = true
                fire(cur, "SelectionChanged", cur == v, old, v)
            end
            cur = cur.Parent
        end
    end
    if v then up(v) end
    if old then up(old) end
    if invalid then
        local pick = first_selectable(v)
        if pick then gs.SelectedObject = pick else gs.SelectedObject = nil end
    end
    return true
end

------------------------------------------------------------ UserInputService
get["UserInputService.OnScreenKeyboardPosition"] = function() return Vector2.zero end
get["UserInputService.OnScreenKeyboardSize"] = function() return Vector2.zero end
get["UserInputService.UserHeadCFrame"] = function() return CFrame.new() end
methods["UserInputService.GetUserCFrame"] = function(_, t)
    if typeof(t) ~= "EnumItem" then error("Unable to cast " .. typeof(t) .. " to token", 0) end
    return CFrame.new()
end
methods["UserInputService.RecenterUserHeadCFrame"] = function() end
-- No accelerometer or gyroscope (Studio's warnings, an empty InputObject).
methods["UserInputService.GetDeviceAcceleration"] = function()
    log("warn", "Trying to get acceleration data on a device without a accelerometer.")
    return input_object({ UserInputType = Enum.UserInputType.Accelerometer, UserInputState = Enum.UserInputState.Change })
end
LM.GetDeviceGravity = function()
    log("warn", "Trying to get gravity data on a device without a accelerometer.")
    return input_object({ UserInputType = Enum.UserInputType.Accelerometer, UserInputState = Enum.UserInputState.Change })
end
LM.GetDeviceRotation = function()
    log("warn", "Trying to get rotation data on a device without a gyroscope.")
    return input_object({ UserInputType = Enum.UserInputType.Gyro, UserInputState = Enum.UserInputState.Change }), CFrame.new()
end
methods["UserInputService.GetMouseButtonsPressed"] = function()
    local b1, b2, b3 = G.buttons()
    local _, _, mx, my = G.view()
    local out = {}
    for i, down in { b1, b2, b3 } do
        if down then
            table.insert(out, input_object({ UserInputType = Enum.UserInputType["MouseButton" .. i],
                UserInputState = Enum.UserInputState.Begin, Position = Vector3.new(mx, my - 58, 0) }))
        end
    end
    return out
end
local nav_pads = {}
methods["UserInputService.SetNavigationGamepad"] = function(_, pad, on)
    if typeof(pad) ~= "EnumItem" then error("Unable to cast " .. typeof(pad) .. " to token", 0) end
    nav_pads[pad] = on == true or nil
end
LM.IsNavigationGamepad = function(_, pad) return nav_pads[pad] == true end
LM.GetNavigationGamepads = function()
    local out = {}
    for p in nav_pads do table.insert(out, p) end
    table.sort(out, function(a, b) return a.Value < b.Value end)
    return out
end

-- VirtualInput: Studio's scriptable input source (typeof "Object"). Its
-- sends go through the same path as the player's input.
local function send(state, uit, key, x, y, z, dx, dy)
    if __rs_input_event then
        task.spawn(__rs_input_event, state, uit, key, x or 0, y or 0, z or 0, dx or 0, dy or 0, false)
    end
end
local vi_methods = {}
function vi_methods:SendKey(down, key)
    if typeof(key) ~= "EnumItem" then error("Unable to cast " .. typeof(key) .. " to token", 0) end
    send(down and "Begin" or "End", "Keyboard", key.Name)
end
function vi_methods:SendMouseButton(pos, button, down)
    if typeof(pos) ~= "Vector2" then error("Unable to cast " .. typeof(pos) .. " to Vector2", 0) end
    if typeof(button) ~= "EnumItem" then error("Unable to cast " .. typeof(button) .. " to token", 0) end
    send(down and "Begin" or "End", button.Name, "Unknown", pos.X, pos.Y)
end
function vi_methods:SendMouseDelta(d)
    if svc("UserInputService").MouseBehavior == Enum.MouseBehavior.Default then
        error("VirtualInput::SendMouseDelta: cursor is not locked.", 0)
    end
    send("Change", "MouseMovement", "Unknown", 0, 0, 0, d.X, d.Y)
end
function vi_methods:SendMousePosition(pos)
    if typeof(pos) ~= "Vector2" then error("Unable to cast " .. typeof(pos) .. " to Vector2", 0) end
    send("Change", "MouseMovement", "Unknown", pos.X, pos.Y)
end
function vi_methods:SendPointerAction(pos, action)
    action = type(action) == "table" and action or {}
    fire(svc("UserInputService"), "PointerAction", tonumber(action.Wheel) or 0,
        typeof(action.Pan) == "Vector2" and action.Pan or Vector2.zero, tonumber(action.Pinch) or 0, false)
end
local focused -- the TextBox with the keyboard (forward)
function vi_methods:SendTextInput(text)
    if focused and focused.Parent then
        focused.Text = focused.Text .. tostring(text)
    end
end
function vi_methods:IsA(c) return c == "VirtualInput" or c == "Object" end
for _, n in { "SendKey", "SendMouseButton", "SendMouseDelta", "SendMousePosition", "SendPointerAction", "SendTextInput" } do
    methods["VirtualInput." .. n] = vi_methods[n]
end
methods["UserInputService.CreateVirtualInput"] = function()
    return G.make_object("VirtualInput", vi_methods)
end

--------------------------------------------------------- ContextActionService
methods["ContextActionService.BindActionToInputTypes"] = function(self, name, fn, touch, ...)
    return self:BindAction(name, fn, touch, ...)
end
local equipped_tool = nil
methods["ContextActionService.GetCurrentLocalToolIcon"] = function()
    if equipped_tool and equipped_tool.Parent then return equipped_tool.TextureId end
    return ""
end
local function step_tools()
    local p = local_player()
    local char = p and p.Character
    local tool = char and char:FindFirstChildOfClass("Tool")
    if tool ~= equipped_tool then
        local cas = svc("ContextActionService")
        if equipped_tool then fire(cas, "LocalToolUnequipped", equipped_tool) end
        equipped_tool = tool
        if tool then fire(cas, "LocalToolEquipped", tool) end
    end
end

------------------------------------------------------------ GuiObject state
get["GuiObject.GuiState"] = function(o)
    if not o.Interactable then return Enum.GuiState.NonInteractable end
    if focused == o then return Enum.GuiState.Focus end
    local hovered, pressed = G.hover_state(o)
    if pressed then return Enum.GuiState.Press end
    if hovered then return Enum.GuiState.Hover end
    return Enum.GuiState.Idle
end
local sinks = setmetatable({}, { __mode = "k" })
get["GuiObject.InputSink"] = function(o) return sinks[o] or Enum.InputSink.None end
set["GuiObject.InputSink"] = function(o, _, v)
    if typeof(v) == "number" or typeof(v) == "string" then
        for _, e in Enum.InputSink:GetEnumItems() do
            if e.Value == v or e.Name == v then v = e end
        end
    end
    if not is_enum(v, "InputSink") then
        error("Unable to assign property InputSink. EnumItem expected, got " .. typeof(v), 0)
    end
    if (sinks[o] or Enum.InputSink.None) ~= v then
        sinks[o] = v
        changed(o, "InputSink")
    end
    return true
end

------------------------------------------------- GetGuiObjectsAtPosition
-- Every GuiObject drawn at (x, y) in GUI coordinates (AbsolutePosition's),
-- top-most first: ScreenGuis by DisplayOrder, ZIndexBehavior's order inside
-- each, clipped by ClipsDescendants and ScrollingFrames.
local function draw_order(gui, out)
    local global = gui:IsA("ScreenGui") and gui.ZIndexBehavior == Enum.ZIndexBehavior.Global
    local function kids(p)
        local list = {}
        for _, c in p:GetChildren() do
            if c:IsA("GuiObject") then table.insert(list, c)
            elseif c:IsA("Folder") then for _, k in kids(c) do table.insert(list, k) end end
        end
        return list
    end
    if global then
        local all, seq = {}, 0
        local function walk(p)
            for _, c in kids(p) do
                seq += 1
                table.insert(all, { c, c.ZIndex, seq })
                walk(c)
            end
        end
        walk(gui)
        table.sort(all, function(a, b)
            if a[2] ~= b[2] then return a[2] < b[2] end
            return a[3] < b[3]
        end)
        for _, e in all do table.insert(out, e[1]) end
    else
        local function walk(p)
            local list = kids(p)
            local idx = {}
            for i, c in list do idx[c] = i end
            table.sort(list, function(a, b)
                if a.ZIndex ~= b.ZIndex then return a.ZIndex < b.ZIndex end
                return idx[a] < idx[b]
            end)
            for _, c in list do
                table.insert(out, c)
                walk(c)
            end
        end
        walk(gui)
    end
end
local function inside(o, x, y)
    local p, s = o.AbsolutePosition, o.AbsoluteSize
    return x >= p.X and y >= p.Y and x < p.X + s.X and y < p.Y + s.Y
end
local function hit(o, x, y)
    if not inside(o, x, y) then return false end
    local cur = o.Parent
    while cur and cur:IsA("GuiObject") do
        if (cur.ClipsDescendants or cur:IsA("ScrollingFrame")) and not inside(cur, x, y) then return false end
        cur = cur.Parent
    end
    return true
end
local function objects_at(guis, x, y)
    local res = {}
    for _, g in guis do
        local order = {}
        draw_order(g, order)
        local here = {}
        for _, o in order do
            if shown(o) and hit(o, x, y) then table.insert(here, o) end
        end
        for i = #here, 1, -1 do table.insert(res, here[i]) end
    end
    return res
end
local function screen_guis(pg)
    local list = {}
    for _, c in pg:GetChildren() do
        if c:IsA("ScreenGui") and c.Enabled then table.insert(list, c) end
    end
    table.sort(list, function(a, b) return a.DisplayOrder > b.DisplayOrder end)
    return list
end
local function check_xy(x, y)
    if tonumber(x) == nil then error("Unable to cast " .. typeof(x) .. " to int", 0) end
    if tonumber(y) == nil then error("Unable to cast " .. typeof(y) .. " to int", 0) end
    return math.floor(tonumber(x)), math.floor(tonumber(y))
end
methods["BasePlayerGui.GetGuiObjectsAtPosition"] = function(self, x, y)
    x, y = check_xy(x, y)
    return objects_at(screen_guis(self), x, y)
end
-- A ScreenGui's own answer: Studio gives an empty list for the PlayerGui's
-- ScreenGuis (measured); other LayerCollectors answer for themselves.
methods["LayerCollector.GetGuiObjectsAtPosition"] = function(self, x, y)
    x, y = check_xy(x, y)
    if self:IsA("ScreenGui") and self:FindFirstAncestorWhichIsA("BasePlayerGui") then return {} end
    if not self.Enabled then return {} end
    return objects_at({ self }, x, y)
end
methods["LayerCollector.GetLayoutNodeTree"] = function(self)
    local function node(o)
        local t = { Instance = o, Children = {} }
        for _, c in o:GetChildren() do
            if c:IsA("GuiObject") then table.insert(t.Children, node(c)) end
        end
        return t
    end
    return node(self)
end

----------------------------------------------------------------- PlayerGui
local topbar = {}
get["PlayerGui.CurrentScreenOrientation"] = function() return Enum.ScreenOrientation.LandscapeLeft end
methods["PlayerGui.GetTopbarTransparency"] = function(self) return topbar[self] or 0.5 end
methods["PlayerGui.SetTopbarTransparency"] = function(self, v)
    v = tonumber(v)
    if v == nil then error("Unable to cast value to float", 0) end
    v = f32(v)
    if topbar[self] ~= v then
        topbar[self] = v
        fire(self, "TopbarTransparencyChangedSignal", v)
    end
end
get["CoreGui.Version"] = function() return 0 end

--------------------------------------------------------------- Text objects
local function content(o)
    if o.RichText then return G.content_text(o.Text) end
    return o.Text
end
for _, c in { "TextLabel", "TextButton", "TextBox" } do
    get[c .. ".ContentText"] = content
    get[c .. ".OpenTypeFeaturesError"] = function() return "" end
    -- Fits when its text was drawn inside its box last frame.
    get[c .. ".TextFits"] = function(o)
        local x, y, w, h, _, _, tw, th = G.layout(o)
        if x == nil or tw < 0 then return false end
        return tw <= w + 0.5 and th <= h + 0.5
    end
end
get["TextLabel.LocalizedText"] = function(o) return o.Text end
get["TextButton.LocalizedText"] = function(o) return o.Text end
get["ImageLabel.IsLoaded"] = function(o) return G.image_loaded(o.Image) end
get["ImageButton.IsLoaded"] = function(o) return G.image_loaded(o.Image) end

------------------------------------------------------------------ TextBox
-- CaptureFocus: ClearTextOnFocus empties it, the cursor goes to the end,
-- Focused then UserInputService.TextBoxFocused. ReleaseFocus: cursor and
-- selection to -1, FocusLost(submitted, input) then TextBoxFocusReleased.
local cursor, selstart = setmetatable({}, { __mode = "k" }), setmetatable({}, { __mode = "k" })
local last_text = nil
local function set_cursor(box, v)
    if (cursor[box] or 1) ~= v then cursor[box] = v changed(box, "CursorPosition") end
end
local function set_sel(box, v)
    if (selstart[box] or -1) ~= v then selstart[box] = v changed(box, "SelectionStart") end
end
get["TextBox.CursorPosition"] = function(box) return cursor[box] or 1 end
get["TextBox.SelectionStart"] = function(box) return selstart[box] or -1 end
local function int_arg(prop, v)
    v = tonumber(v)
    if v == nil then error("Unable to assign property " .. prop .. ". int expected", 0) end
    return math.floor(v)
end
set["TextBox.CursorPosition"] = function(box, _, v)
    v = int_arg("CursorPosition", v)
    local n = utf8.len(box.Text) or #box.Text
    if focused == box then v = math.clamp(v, 1, n + 1) else v = math.clamp(v, -1, n + 1) end
    set_cursor(box, v)
    return true
end
set["TextBox.SelectionStart"] = function(box, _, v)
    v = int_arg("SelectionStart", v)
    local n = utf8.len(box.Text) or #box.Text
    set_sel(box, math.clamp(v, -1, n + 1))
    return true
end
local function gain(box)
    if box.ClearTextOnFocus then box.Text = "" end
    focused = box
    last_text = box.Text
    set_cursor(box, (utf8.len(box.Text) or #box.Text) + 1)
    set_sel(box, -1)
    fire(box, "Focused")
    fire(svc("UserInputService"), "TextBoxFocused", box)
end
local function lose(box, enter, input)
    if focused ~= box then return end
    focused = nil
    set_cursor(box, -1)
    set_sel(box, -1)
    fire(box, "FocusLost", enter == true, input)
    fire(svc("UserInputService"), "TextBoxFocusReleased", box)
end
methods["TextBox.CaptureFocus"] = function(box)
    if focused == box or not G.is_client() then return end
    if not box:IsDescendantOf(game) or not box:FindFirstAncestorWhichIsA("LayerCollector") then return end
    if focused then
        G.set_focus(nil)
        lose(focused, false, nil)
    end
    G.set_focus(box)
    gain(box)
end
methods["TextBox.ReleaseFocus"] = function(box, submitted)
    if focused ~= box then return end
    G.set_focus(nil)
    lose(box, submitted, nil)
end
methods["TextBox.IsFocused"] = function(box) return focused == box end
-- (moved here from runtime_ext.rs, which answered nil)
methods["UserInputService.GetFocusedTextBox"] = function() return focused end
local function step_focus()
    for _, e in G.focus_events() do
        if e.gained then
            if focused and focused ~= e.box then lose(focused, false, nil) end
            if focused ~= e.box then gain(e.box) end
        else
            local io
            if e.button == 0 then
                io = input_object({ UserInputType = Enum.UserInputType.Keyboard, UserInputState = Enum.UserInputState.Begin,
                    KeyCode = e.enter and Enum.KeyCode.Return or Enum.KeyCode.Escape })
            elseif e.button then
                io = input_object({ UserInputType = Enum.UserInputType["MouseButton" .. e.button],
                    UserInputState = Enum.UserInputState.Begin, Position = Vector3.new(e.x, e.y, 0) })
            end
            lose(e.box, e.enter, io)
        end
    end
    if focused then
        if not focused:IsDescendantOf(game) then
            local box = focused
            G.set_focus(nil)
            lose(box, false, nil)
        elseif focused.Text ~= last_text then
            last_text = focused.Text
            set_cursor(focused, (utf8.len(last_text) or #last_text) + 1)
        end
    end
end

------------------------------------------------------------- ScrollingFrame
get["ScrollingFrame.AbsoluteCanvasSize"] = function(sf)
    local x, y, w, h, cw, ch = G.layout(sf)
    if x ~= nil and cw >= 0 then return Vector2.new(cw, ch) end
    local s, c = sf.AbsoluteSize, sf.CanvasSize
    return Vector2.new(c.X.Scale * s.X + c.X.Offset, c.Y.Scale * s.Y + c.Y.Offset)
end
-- The part of the frame the canvas shows in: less a scroll bar's width when
-- its inset (ScrollBar: while it shows; Always) keeps it out.
get["ScrollingFrame.AbsoluteWindowSize"] = function(sf)
    local s = sf.AbsoluteSize
    local canvas = get["ScrollingFrame.AbsoluteCanvasSize"](sf)
    local t = sf.ScrollBarThickness
    local w, h = s.X, s.Y
    local vi, hi = sf.VerticalScrollBarInset, sf.HorizontalScrollBarInset
    if vi == Enum.ScrollBarInset.Always or (vi == Enum.ScrollBarInset.ScrollBar and canvas.Y > s.Y) then w -= t end
    if hi == Enum.ScrollBarInset.Always or (hi == Enum.ScrollBarInset.ScrollBar and canvas.X > s.X) then h -= t end
    return Vector2.new(math.max(w, 0), math.max(h, 0))
end
-- The wheel scrolls in whole steps here: no fling to slow down.
methods["ScrollingFrame.GetScrollVelocity"] = function() return Vector2.zero end
methods["ScrollingFrame.ResetScrollVelocity"] = function() end

-------------------------------------------------------------- UIGridLayout
local function grid_cells(g)
    local parent = g.Parent
    if not isa(parent, "GuiBase2d") then return Vector2.zero, Vector2.new(g.CellSize.X.Offset, g.CellSize.Y.Offset) end
    local s = parent.AbsoluteSize
    local cs, cp = g.CellSize, g.CellPadding
    local cw, ch = cs.X.Scale * s.X + cs.X.Offset, cs.Y.Scale * s.Y + cs.Y.Offset
    local px, py = cp.X.Scale * s.X + cp.X.Offset, cp.Y.Scale * s.Y + cp.Y.Offset
    local n = 0
    for _, c in parent:GetChildren() do
        if c:IsA("GuiObject") and c.Visible then n += 1 end
    end
    if n == 0 then return Vector2.zero, Vector2.new(cw, ch) end
    local vertical = g.FillDirection == Enum.FillDirection.Vertical
    local fit = vertical and math.floor((s.Y + py) / math.max(ch + py, 1e-3)) or math.floor((s.X + px) / math.max(cw + px, 1e-3))
    local per = math.max(fit, 1)
    if g.FillDirectionMaxCells > 0 then per = math.min(per, g.FillDirectionMaxCells) end
    local lines = math.ceil(n / per)
    local along = math.min(per, n)
    if vertical then return Vector2.new(lines, along), Vector2.new(cw, ch) end
    return Vector2.new(along, lines), Vector2.new(cw, ch)
end
get["UIGridLayout.AbsoluteCellCount"] = function(g) return (grid_cells(g)) end
get["UIGridLayout.AbsoluteCellSize"] = function(g) return select(2, grid_cells(g)) end
-- Deprecated: the layout already re-applies itself every frame; a custom
-- sort function is what SortOrder Custom used to call.
methods["UIGridStyleLayout.ApplyLayout"] = function() end
local sort_fns = setmetatable({}, { __mode = "k" })
methods["UIGridStyleLayout.SetCustomSortFunction"] = function(self, fn)
    if fn ~= nil and type(fn) ~= "function" then error("Unable to cast value to function", 0) end
    sort_fns[self] = fn
end

-------------------------------------------------------------- UIPageLayout
-- Pages are the parent's visible GuiObjects in SortOrder. Jumps set
-- CurrentPage at once; when Animated the view tweens over TweenTime and
-- Stopped(page) fires at its end (not when not animated). JumpToIndex is
-- 0-based and wraps (Studio: 10 of 3 pages is the second).
local pl_state = setmetatable({}, { __mode = "k" })
local function pages_of(pl)
    local parent = pl.Parent
    local list = {}
    if not parent then return list end
    for _, c in parent:GetChildren() do
        if c:IsA("GuiObject") and c.Visible then table.insert(list, c) end
    end
    local idx = {}
    for i, c in list do idx[c] = i end
    if pl.SortOrder == Enum.SortOrder.Name then
        table.sort(list, function(a, b) return a.Name < b.Name end)
    elseif pl.SortOrder == Enum.SortOrder.LayoutOrder then
        table.sort(list, function(a, b)
            if a.LayoutOrder ~= b.LayoutOrder then return a.LayoutOrder < b.LayoutOrder end
            return idx[a] < idx[b]
        end)
    end
    return list
end
local function current_page(pl)
    local st = pl_state[pl]
    local pages = pages_of(pl)
    if st and st.page and table.find(pages, st.page) then return st.page, pages end
    return pages[1], pages
end
get["UIPageLayout.CurrentPage"] = function(pl) return (current_page(pl)) end
local tweening = {}
local function go(pl, page)
    local cur = current_page(pl)
    if page == nil or page == cur then return end
    local st = pl_state[pl] or {}
    pl_state[pl] = st
    st.page = page
    changed(pl, "CurrentPage")
    if pl.Animated and pl.TweenTime > 0 then
        tweening[pl] = { from = cur, to = page, t = 0 }
        G.set_page(pl, cur, page, 0)
    else
        tweening[pl] = nil
        G.set_page(pl, page, page, 1)
    end
end
local function step_pages(dt)
    for pl, tw in tweening do
        tw.t += dt
        local dur = math.max(pl.TweenTime, 1e-3)
        local a = math.min(tw.t / dur, 1)
        local ok, alpha = pcall(function() return svc("TweenService"):GetValue(a, pl.EasingStyle, pl.EasingDirection) end)
        G.set_page(pl, tw.from, tw.to, ok and alpha or a)
        if a >= 1 then
            tweening[pl] = nil
            fire(pl, "Stopped", tw.to)
        end
    end
end
methods["UIPageLayout.JumpTo"] = function(pl, page)
    if not isa(page, "GuiObject") then
        error((typeof(page) == "Instance" and page.ClassName or typeof(page)) .. " passed to PageLayout JumpTo is not a GuiObject", 0)
    end
    local _, pages = current_page(pl)
    if table.find(pages, page) then go(pl, page) end
end
methods["UIPageLayout.JumpToIndex"] = function(pl, i)
    i = tonumber(i)
    if i == nil then error("Unable to cast value to int", 0) end
    local _, pages = current_page(pl)
    if #pages == 0 then return end
    go(pl, pages[(math.floor(i) % #pages) + 1])
end
local function shift(pl, d)
    local cur, pages = current_page(pl)
    if not cur then return end
    local i = table.find(pages, cur) + d
    if i < 1 or i > #pages then
        if not pl.Circular then return end
        i = (i - 1) % #pages + 1
    end
    go(pl, pages[i])
end
methods["UIPageLayout.Next"] = function(pl) shift(pl, 1) end
methods["UIPageLayout.Previous"] = function(pl) shift(pl, -1) end

------------------------------------------------------------ UIDragDetector
local dd_constraints = setmetatable({}, { __mode = "k" })
local dd_style = setmetatable({}, { __mode = "k" })
methods["UIDragDetector.AddConstraintFunction"] = function(self, priority, fn)
    if tonumber(priority) == nil then error("Unable to cast value to int", 0) end
    if type(fn) ~= "function" then error("Unable to cast value to function", 0) end
    -- A real RBXScriptConnection, its Disconnect removing the function.
    local holder = Instance.new("BindableEvent")
    local conn = holder.Event:Connect(fn)
    local list = dd_constraints[self] or {}
    dd_constraints[self] = list
    table.insert(list, { priority = tonumber(priority), fn = fn, conn = conn })
    table.sort(list, function(a, b) return a.priority > b.priority end)
    return conn
end
methods["UIDragDetector.SetDragStyleFunction"] = function(self, fn)
    if fn ~= nil and type(fn) ~= "function" then error("Unable to cast value to function", 0) end
    dd_style[self] = fn
end
methods["UIDragDetector.GetReferencePosition"] = function(self)
    local ref = self.ReferenceUIInstance
    if isa(ref, "GuiObject") then return ref.Position end
    local p = self.Parent
    if isa(p, "GuiObject") then return p.Position end
    return UDim2.new()
end
methods["UIDragDetector.GetReferenceRotation"] = function(self)
    local ref = self.ReferenceUIInstance
    if isa(ref, "GuiObject") then return ref.Rotation end
    local p = self.Parent
    if isa(p, "GuiObject") then return p.Rotation end
    return 0
end

-- Dragging (the viewport reports presses / moves / releases on objects
-- that are Draggable or hold an enabled UIDragDetector). Draggable:
-- DragBegin(Position at the start), the object follows the mouse,
-- DragStopped(x, y). UIDragDetector: DragStart / DragContinue / DragEnd
-- (input position), DragUDim2 / DragRotation, its DragStyle (TranslatePlane,
-- TranslateLine along DragAxis, Rotate, Scriptable through the style
-- function), constraint functions by priority, ResponseStyle (Offset /
-- Scale move the object; Custom* leave it to scripts) and BoundingUI.
local drag = nil
local function detector_of(o)
    for _, c in o:GetChildren() do
        if c:IsA("UIDragDetector") and c.Enabled then return c end
    end
    return nil
end
local function run_constraints(dd, v)
    for _, e in dd_constraints[dd] or {} do
        if e.conn.Connected then
            local ok, r = pcall(e.fn, v)
            if ok and r ~= nil then v = r end
        end
    end
    return v
end
local function drag_step(kind, x, y)
    if kind == 0 then
        local o = drag.obj
        drag.start_pos = o.Position
        drag.start_rot = o.Rotation
        drag.x0, drag.y0 = x, y
        local dd = detector_of(o)
        drag.dd = dd
        if dd then
            fire(dd, "DragStart", Vector2.new(x, y))
        elseif o.Draggable then
            fire(o, "DragBegin", o.Position)
        end
        return
    end
    local o, dd = drag.obj, drag.dd
    if not o.Parent then return end
    local dx, dy = x - drag.x0, y - drag.y0
    if dd then
        local style = dd.DragStyle
        local du, rot = UDim2.fromOffset(dx, dy), 0
        if style == Enum.UIDragDetectorDragStyle.TranslateLine then
            local axis = dd.DragAxis
            local m = axis.Magnitude > 0 and axis.Unit or Vector2.xAxis
            local along = dx * m.X + dy * m.Y
            du = UDim2.fromOffset(m.X * along, m.Y * along)
        elseif style == Enum.UIDragDetectorDragStyle.Rotate then
            local c = o.AbsolutePosition + o.AbsoluteSize / 2
            local a0 = math.atan2(drag.y0 - c.Y, drag.x0 - c.X)
            local a1 = math.atan2(y - c.Y, x - c.X)
            rot = math.deg(a1 - a0)
            du = UDim2.new()
        elseif style == Enum.UIDragDetectorDragStyle.Scriptable then
            local fn = dd_style[dd]
            local ok, r = pcall(function() return fn and fn(Vector2.new(x, y)) end)
            if ok and typeof(r) == "UDim2" then du = r - drag.start_pos
            elseif ok and type(r) == "number" then rot = r - drag.start_rot du = UDim2.new() end
        end
        if style == Enum.UIDragDetectorDragStyle.Rotate or (style == Enum.UIDragDetectorDragStyle.Scriptable and rot ~= 0) then
            rot = run_constraints(dd, rot)
            pcall(function() dd.DragRotation = rot end)
        else
            if dd.ResponseStyle == Enum.UIDragDetectorResponseStyle.Scale or dd.ResponseStyle == Enum.UIDragDetectorResponseStyle.CustomScale then
                local ps = o.Parent:IsA("GuiBase2d") and o.Parent.AbsoluteSize or Vector2.one
                du = UDim2.fromScale(du.X.Offset / math.max(ps.X, 1), du.Y.Offset / math.max(ps.Y, 1))
            end
            du = run_constraints(dd, du)
            pcall(function() dd.DragUDim2 = du end)
        end
        local rs = dd.ResponseStyle
        if rs == Enum.UIDragDetectorResponseStyle.Offset or rs == Enum.UIDragDetectorResponseStyle.Scale then
            if style == Enum.UIDragDetectorDragStyle.Rotate then
                o.Rotation = drag.start_rot + rot
            else
                local np = drag.start_pos + du
                local b = dd.BoundingUI
                if b and b:IsA("GuiObject") then
                    -- Keep the object inside the bounding object.
                    local bp, bs, os = b.AbsolutePosition, b.AbsoluteSize, o.AbsoluteSize
                    local cur = o.AbsolutePosition
                    local want = cur + Vector2.new((np.X.Offset - o.Position.X.Offset), (np.Y.Offset - o.Position.Y.Offset))
                    local cx = math.clamp(want.X, bp.X, bp.X + bs.X - os.X) - want.X
                    local cy = math.clamp(want.Y, bp.Y, bp.Y + bs.Y - os.Y) - want.Y
                    np = np + UDim2.fromOffset(cx, cy)
                end
                o.Position = np
            end
        end
        fire(dd, kind == 2 and "DragEnd" or "DragContinue", Vector2.new(x, y))
    elseif o.Draggable then
        if kind == 1 then
            o.Position = drag.start_pos + UDim2.fromOffset(dx, dy)
        else
            fire(o, "DragStopped", x, y)
        end
    end
end
local function step_drags()
    for _, e in G.drags() do
        if e.kind == 0 then drag = { obj = e.obj } end
        if drag and drag.obj == e.obj then
            local ok, err = pcall(drag_step, e.kind, e.x, e.y)
            if not ok then log("error", tostring(err)) end
            if e.kind == 2 then drag = nil end
        end
    end
    if drag and not G.buttons() then
        local _, _, mx, my = G.view()
        pcall(drag_step, 2, mx, my - 58)
        drag = nil
    end
end

------------------------------------------------------- 3D handle adornments
-- The mouse ray against HandleAdornments (their shape at Adornee * CFrame,
-- SizeRelativeOffset), Handles (a sphere per face at half size + 2 studs
-- out) and ArcHandles (two spheres per axis on a ring of radius |half
-- size| + 2: Z's on ±Y, Y's on ±X, X's on ±Z). The nearest one under the
-- mouse is hovered (parts in front hide those not AlwaysOnTop). Studio's
-- order: MouseEnter, MouseButton1Down, MouseDrag while held (Handles:
-- studs along the face's axis since the press; ArcHandles: angle in
-- radians and radius change on the ring's plane), MouseButton1Up on the
-- pressed one, MouseLeave; hovering keeps changing while dragging.
local HANDLE_R = 0.55
local function adornee_cf(a)
    local ad = a.Adornee
    if isa(ad, "BasePart") then return ad.CFrame, ad.Size end
    if isa(ad, "Model") then
        local ok, cf, size = pcall(function() return ad:GetBoundingBox() end)
        if ok then return cf, size end
    end
    return nil
end
local function ray_sphere(o, d, c, r)
    local oc = o - c
    local b = oc:Dot(d)
    local cc = oc:Dot(oc) - r * r
    local h = b * b - cc
    if h < 0 then return nil end
    local t = -b - math.sqrt(h)
    if t < 0 then t = -b + math.sqrt(h) end
    return t >= 0 and t or nil
end
local function ray_box(o, d, cf, size)
    local lo = cf:PointToObjectSpace(o)
    local ld = cf:VectorToObjectSpace(d)
    local h = size / 2
    local tmin, tmax = -math.huge, math.huge
    for _, ax in { "X", "Y", "Z" } do
        local oo, dd, hh = lo[ax], ld[ax], h[ax]
        if math.abs(dd) < 1e-9 then
            if oo < -hh or oo > hh then return nil end
        else
            local t1, t2 = (-hh - oo) / dd, (hh - oo) / dd
            if t1 > t2 then t1, t2 = t2, t1 end
            tmin, tmax = math.max(tmin, t1), math.min(tmax, t2)
            if tmin > tmax then return nil end
        end
    end
    if tmax < 0 then return nil end
    return math.max(tmin, 0)
end
local FACES = { Right = Vector3.xAxis, Left = -Vector3.xAxis, Top = Vector3.yAxis, Bottom = -Vector3.yAxis, Back = Vector3.zAxis, Front = -Vector3.zAxis }
local ARC_SPOTS = { X = { Vector3.zAxis, -Vector3.zAxis }, Y = { Vector3.xAxis, -Vector3.xAxis }, Z = { Vector3.yAxis, -Vector3.yAxis } }
local AXIS_VEC = { X = Vector3.xAxis, Y = Vector3.yAxis, Z = Vector3.zAxis }
-- Distance along the ray to `a` and which part of it (face / axis name).
local function adorn_hit(a, o, d)
    if not a.Visible then return nil end
    local cf, size = adornee_cf(a)
    if not cf then return nil end
    if a:IsA("Handles") then
        local best, which
        for name, n in FACES do
            if a.Faces[name] then
                local c = cf:PointToWorldSpace(n * (math.abs(n:Dot(size)) / 2 + 2))
                local t = ray_sphere(o, d, c, HANDLE_R)
                if t and (not best or t < best) then best, which = t, name end
            end
        end
        return best, which
    elseif a:IsA("ArcHandles") then
        local radius = (size / 2).Magnitude + 2
        local best, which
        for name, spots in ARC_SPOTS do
            if a.Axes[name] then
                for _, n in spots do
                    local t = ray_sphere(o, d, cf:PointToWorldSpace(n * radius), HANDLE_R)
                    if t and (not best or t < best) then best, which = t, name end
                end
            end
        end
        return best, which
    end
    local base = cf * CFrame.new(size * a.SizeRelativeOffset / 2) * a.CFrame
    if a:IsA("SphereHandleAdornment") then return ray_sphere(o, d, base.Position, a.Radius) end
    if a:IsA("BoxHandleAdornment") or a:IsA("ImageHandleAdornment") then
        local s = a.Size
        if a:IsA("ImageHandleAdornment") then s = Vector3.new(s.X, s.Y, 0.05) end
        return ray_box(o, d, base, s)
    end
    if a:IsA("CylinderHandleAdornment") or a:IsA("ConeHandleAdornment") then
        return ray_box(o, d, base, Vector3.new(a.Radius * 2, a.Radius * 2, a.Height))
    end
    if a:IsA("PyramidHandleAdornment") then
        return ray_box(o, d, base * CFrame.new(0, 0, -a.Height / 2), Vector3.new(a.Size, a.Size, a.Height))
    end
    if a:IsA("LineHandleAdornment") then
        return ray_box(o, d, base * CFrame.new(0, 0, -a.Length / 2), Vector3.new(0.2, 0.2, a.Length))
    end
    return nil
end
local hover_a, hover_part = nil, nil
local press = nil
local function enum_of(a, part)
    if part == nil then return nil end
    if a:IsA("Handles") then return Enum.NormalId[part] end
    return Enum.Axis[part]
end
local function adorn_fire(a, ev, part, ...)
    if a:IsA("HandleAdornment") then fire(a, ev) else fire(a, ev, enum_of(a, part), ...) end
end
-- Where the ray passes the axis line through p along n: studs along n.
local function along_axis(o, d, p, n)
    local w = o - p
    local b = d:Dot(n)
    local denom = 1 - b * b
    if denom < 1e-6 then return nil end
    return (w:Dot(n) - b * w:Dot(d)) / denom
end
local function ring_point(o, d, c, n)
    local dn = d:Dot(n)
    if math.abs(dn) < 1e-6 then return nil end
    local t = (c - o):Dot(n) / dn
    if t < 0 then return nil end
    return o + d * t
end
local function step_adorns()
    if next(tracked.Adorn) == nil then hover_a = nil press = nil return end
    local o, d = G.mouse_ray()
    if d.Magnitude < 0.5 then return end
    local best, ba, bp
    for a in tracked.Adorn do
        if a.Parent then
            local ok, t, part = pcall(adorn_hit, a, o, d)
            if ok and t and (not best or t < best) then
                local hidden = false
                if not (a:IsA("HandlesBase") or a.AlwaysOnTop) then
                    local params = RaycastParams.new()
                    params.FilterType = Enum.RaycastFilterType.Exclude
                    local ex = {}
                    local p = local_player()
                    if p and p.Character then table.insert(ex, p.Character) end
                    if a.Adornee then table.insert(ex, a.Adornee) end
                    params.FilterDescendantsInstances = ex
                    local r = workspace:Raycast(o, d * t, params)
                    hidden = r ~= nil and r.Distance < t - 0.01
                end
                if not hidden then best, ba, bp = t, a, part end
            end
        end
    end
    if ba ~= hover_a or bp ~= hover_part then
        if hover_a and hover_a.Parent then adorn_fire(hover_a, "MouseLeave", hover_part) end
        hover_a, hover_part = ba, bp
        if ba then adorn_fire(ba, "MouseEnter", bp) end
    end
    local function begin()
        local a, part = hover_a, hover_part
        press = { a = a, part = part }
        local cf, size = adornee_cf(a)
        if cf and a:IsA("Handles") then
            local n = cf:VectorToWorldSpace(FACES[part])
            press.p, press.n = cf.Position, n
            press.t0 = along_axis(o, d, cf.Position, n)
        elseif cf and a:IsA("ArcHandles") then
            local n = cf:VectorToWorldSpace(AXIS_VEC[part])
            press.c, press.n = cf.Position, n
            press.radius = (size / 2).Magnitude + 2
            local q = ring_point(o, d, cf.Position, n)
            press.v0 = q and (q - cf.Position)
        end
        adorn_fire(a, "MouseButton1Down", part)
    end
    local function finish()
        if press.a.Parent then adorn_fire(press.a, "MouseButton1Up", press.part) end
        press = nil
    end
    for _, e in G.mouse_edges() do
        if e.button == 1 then
            if e.down and not e.gui and hover_a and not press then begin()
            elseif not e.down and press then finish() end
        end
    end
    if press and G.buttons() then
        local a = press.a
        if a.Parent and press.n then
            if press.t0 then
                local t = along_axis(o, d, press.p, press.n)
                if t and t ~= press.last then
                    press.last = t
                    adorn_fire(a, "MouseDrag", press.part, t - press.t0)
                end
            elseif press.v0 then
                local q = ring_point(o, d, press.c, press.n)
                if q then
                    local v = q - press.c
                    local key = v.X .. "," .. v.Y .. "," .. v.Z
                    if key ~= press.last then
                        press.last = key
                        local ang = math.atan2(press.v0:Cross(v):Dot(press.n), press.v0:Dot(v))
                        adorn_fire(a, "MouseDrag", press.part, ang, v.Magnitude - press.radius)
                    end
                end
            end
        end
    elseif press and not G.buttons() then
        finish()
    end
end

-------------------------------------------------------------------- Styles
-- StyleRule properties / transitions, StyleSheet derives, StyleQuery
-- conditions, and the engine applying them: every StyleLink's sheet styles
-- the descendants of the link's parent (rules by Priority, later wins;
-- "$token" values are the sheet's attributes; a property no rule sets any
-- more gets back the value it had).
local rule_props = setmetatable({}, { __mode = "k" })
local rule_trans = setmetatable({}, { __mode = "k" })
local sheet_derives = setmetatable({}, { __mode = "k" })
local query_conds = setmetatable({}, { __mode = "k" })
local styles_dirty = true
local function props_of(r)
    local t = rule_props[r]
    if not t then t = {} rule_props[r] = t end
    return t
end
local function copy(t)
    local o = {}
    for k, v in t do o[k] = v end
    return o
end
methods["StyleRule.GetProperties"] = function(r) return copy(props_of(r)) end
methods["StyleRule.GetProperty"] = function(r, name) return props_of(r)[tostring(name)] end
methods["StyleRule.SetProperty"] = function(r, name, v)
    if type(name) ~= "string" then error("Unable to cast value to std::string", 0) end
    props_of(r)[name] = v
    styles_dirty = true
end
methods["StyleRule.SetProperties"] = function(r, t)
    if type(t) ~= "table" then error("Unable to cast value to Dictionary", 0) end
    local p = {}
    for k, v in t do p[tostring(k)] = v end
    rule_props[r] = p
    styles_dirty = true
end
local function trans_of(r)
    local t = rule_trans[r]
    if not t then t = {} rule_trans[r] = t end
    return t
end
local function check_tween(v)
    if v == nil or typeof(v) == "TweenInfo" then return true end
    local tn = ({ number = "double", string = "string", boolean = "bool" })[type(v)] or typeof(v)
    log("warn", "Type `" .. tn .. "` is not supported as a StyleRule property transition")
    return false
end
methods["StyleRule.GetDefaultPropertyTransition"] = function(r) return trans_of(r)["*"] end
methods["StyleRule.SetDefaultPropertyTransition"] = function(r, v)
    if check_tween(v) then trans_of(r)["*"] = v end
end
methods["StyleRule.GetPropertyTransitions"] = function(r) return copy(trans_of(r)) end
methods["StyleRule.SetPropertyTransition"] = function(r, name, v)
    if check_tween(v) then trans_of(r)[tostring(name)] = v end
end
methods["StyleRule.SetPropertyTransitions"] = function(r, t)
    if type(t) ~= "table" then error("Unable to cast value to Dictionary", 0) end
    local o = {}
    for k, v in t do
        if check_tween(v) then o[tostring(k)] = v end
    end
    rule_trans[r] = o
end
get["StyleRule.SelectorError"] = function() return "" end

local function rules_of(base)
    local list, idx = {}, {}
    for i, c in base:GetChildren() do
        if c:IsA("StyleRule") then table.insert(list, c) idx[c] = i end
    end
    table.sort(list, function(a, b)
        if a.Priority ~= b.Priority then return a.Priority < b.Priority end
        return idx[a] < idx[b]
    end)
    return list
end
methods["StyleBase.GetStyleRules"] = function(base) return rules_of(base) end
methods["StyleBase.InsertStyleRule"] = function(base, rule, priority)
    if not isa(rule, "StyleRule") then error("Unable to cast value to StyleRule", 0) end
    if priority ~= nil then rule.Priority = tonumber(priority) or 0 end
    rule.Parent = base
    styles_dirty = true
    fire(base, "StyleRulesChanged")
end
methods["StyleBase.SetStyleRules"] = function(base, rules)
    if type(rules) ~= "table" then error("Unable to cast value to Objects", 0) end
    for i, r in rules do
        if not isa(r, "StyleRule") then error("Expected StyleRule at index " .. (i - 1) .. ", got " .. (typeof(r) == "Instance" and r.ClassName or typeof(r)), 0) end
    end
    local existing = rules_of(base)
    if #existing > 0 then log("warn", "Unparenting existing styles in SetStyles().") end
    for _, r in existing do r.Parent = nil end
    for _, r in rules do r.Parent = base end
    styles_dirty = true
    fire(base, "StyleRulesChanged")
end
methods["StyleSheet.GetDerives"] = function(s) return copy(sheet_derives[s] or {}) end
methods["StyleSheet.SetDerives"] = function(s, list)
    if type(list) ~= "table" then error("Unable to cast value to Objects", 0) end
    for i, d in list do
        if not isa(d, "StyleSheet") then
            error("Expected StyleSheet at index " .. (i - 1) .. ", got " .. (typeof(d) == "Instance" and d.ClassName or typeof(d)), 0)
        end
    end
    sheet_derives[s] = copy(list)
    styles_dirty = true
end
local function conds_of(q)
    local t = query_conds[q]
    if not t then t = {} query_conds[q] = t end
    return t
end
methods["StyleQuery.GetCondition"] = function(q, name) return conds_of(q)[tostring(name)] end
methods["StyleQuery.GetConditions"] = function(q) return copy(conds_of(q)) end
methods["StyleQuery.SetCondition"] = function(q, name, v)
    conds_of(q)[tostring(name)] = v
    styles_dirty = true
end
methods["StyleQuery.SetConditions"] = function(q, t)
    if type(t) ~= "table" then error("Unable to cast value to Dictionary", 0) end
    local o = {}
    for k, v in t do o[tostring(k)] = v end
    query_conds[q] = o
    styles_dirty = true
end
-- Active when it is in a styled tree and its size conditions hold for the
-- screen (MinSize / MaxSize / AspectRatioRange).
local function query_active(q, screen)
    if not screen then return false end
    local any = false
    for k, v in conds_of(q) do
        any = true
        if k == "MinSize" and typeof(v) == "Vector2" then
            if screen.X < v.X or screen.Y < v.Y then return false end
        elseif k == "MaxSize" and typeof(v) == "Vector2" then
            if screen.X > v.X or screen.Y > v.Y then return false end
        elseif k == "AspectRatioRange" and typeof(v) == "NumberRange" then
            local ar = screen.X / math.max(screen.Y, 1)
            if ar < v.Min or ar > v.Max then return false end
        elseif k == "ReducedMotionEnabled" then
            if v ~= false then return false end
        end
    end
    return any
end
get["StyleQuery.IsActive"] = function(q)
    local sheet = q:FindFirstAncestorWhichIsA("StyleSheet")
    if not sheet or not G.is_client() then return false end
    local w, h = G.view()
    return query_active(q, Vector2.new(w, h - 58))
end

-- Selectors: "A, B" lists of compounds joined by descendant (space) or
-- child (">") steps; a compound is ClassName, .tag, #Name and :state
-- (GuiState: hover, press, focus, nonInteractable), in any combination.
local selector_cache = {}
local function parse_selector(s)
    if selector_cache[s] then return selector_cache[s] end
    local alts = {}
    for alt in string.gmatch(s, "[^,]+") do
        local steps = {}
        local rel = "desc"
        alt = alt:gsub(">", " > ")
        for tok in string.gmatch(alt, "%S+") do
            if tok == ">" then
                rel = "child"
            elseif tok == "&" then
                rel = "self"
            else
                local c = { rel = rel, tags = {}, states = {} }
                local rest = tok
                if rest:sub(1, 1) == "&" then c.rel = "self" rest = rest:sub(2) end
                c.class = rest:match("^([%a_][%w_]*)")
                for kind, name in string.gmatch(rest, "([%.#:])([%w_]+)") do
                    if kind == "." then table.insert(c.tags, name)
                    elseif kind == "#" then c.name = name
                    else table.insert(c.states, name:lower()) end
                end
                table.insert(steps, c)
                rel = "desc"
            end
        end
        if #steps > 0 then table.insert(alts, steps) end
    end
    selector_cache[s] = alts
    return alts
end
local function compound_matches(c, o)
    if c.class and not o:IsA(c.class) then return false end
    if c.name and o.Name ~= c.name then return false end
    for _, t in c.tags do
        if not o:HasTag(t) then return false end
    end
    if #c.states > 0 then
        if not o:IsA("GuiObject") then return false end
        local st = get["GuiObject.GuiState"](o).Name:lower()
        for _, s in c.states do
            if s ~= st then return false end
        end
    end
    return true
end
-- Does `steps` (right-most last) match o, with every step inside `root`?
-- `scope`: the instances a nested rule's parent matched (nil: the root).
local function steps_match(steps, o, root, scopes)
    local function from(i, node)
        local c = steps[i]
        if not compound_matches(c, node) then return false end
        if i == 1 then
            if scopes then
                if c.rel == "self" then return scopes[node] == true end
                local p = node.Parent
                if c.rel == "child" then return p ~= nil and scopes[p] == true end
                while p and p ~= root.Parent do
                    if scopes[p] then return true end
                    p = p.Parent
                end
                return false
            end
            if c.rel == "child" then return node.Parent == root end
            return node == root or node:IsDescendantOf(root)
        end
        local p = node.Parent
        if c.rel == "child" then return p ~= nil and from(i - 1, p) end
        while p and (p == root or p:IsDescendantOf(root)) do
            if from(i - 1, p) then return true end
            p = p.Parent
        end
        return false
    end
    return from(#steps, o)
end
local styled = {} -- inst → { prop → original value }
local style_targets = {} -- inst → { prop → value } last applied
local function resolve_token(v, sheets)
    if type(v) == "string" and v:sub(1, 1) == "$" then
        local name = v:sub(2)
        for _, s in sheets do
            local a = s:GetAttribute(name)
            if a ~= nil then return a end
        end
        return nil
    end
    return v
end
local function collect_rules(sheet, out, seen)
    if seen[sheet] then return end
    seen[sheet] = true
    for _, d in sheet_derives[sheet] or {} do collect_rules(d, out, seen) end
    for _, d in sheet:GetChildren() do
        if d:IsA("StyleDerive") and d.StyleSheet then collect_rules(d.StyleSheet, out, seen) end
    end
    local function add(base, parent_rule, depth)
        for _, r in rules_of(base) do
            table.insert(out, { rule = r, parent = parent_rule, depth = depth })
            add(r, r, depth + 1)
        end
    end
    add(sheet, nil, 0)
end
local function step_styles()
    local links = tracked.StyleLink
    if next(links) == nil and next(styled) == nil then return end
    local want = {} -- inst → { prop → { value, transition } }
    local w, h = G.view()
    local screen = Vector2.new(w, h - 58)
    for link in links do
        local sheet, root = link.StyleSheet, link.Parent
        if sheet and root and link:IsDescendantOf(game) then
            local rules = {}
            collect_rules(sheet, rules, {})
            local sheets = { sheet }
            for _, d in sheet_derives[sheet] or {} do table.insert(sheets, d) end
            local nodes = root:GetDescendants()
            table.insert(nodes, 1, root)
            -- Matches per rule (nested rules match inside their parent's).
            local matched = {}
            local order = {}
            for i, e in rules do order[i] = e end
            for _, e in order do
                local r = e.rule
                local active = true
                local q = r:FindFirstAncestorWhichIsA("StyleQuery")
                if q and not query_active(q, screen) then active = false end
                local m = {}
                if active and r.Selector ~= "" then
                    local scopes = e.parent and matched[e.parent] or nil
                    if e.parent == nil or scopes then
                        for _, alt in parse_selector(r.Selector) do
                            for _, n in nodes do
                                if not m[n] and steps_match(alt, n, root, scopes) then m[n] = true end
                            end
                        end
                    end
                end
                matched[r] = m
            end
            table.sort(order, function(a, b)
                if a.rule.Priority ~= b.rule.Priority then return a.rule.Priority < b.rule.Priority end
                return false
            end)
            for _, e in order do
                local r = e.rule
                local props = rule_props[r]
                if props and next(props) then
                    local tr = rule_trans[r] or {}
                    for n in matched[r] do
                        local t = want[n] or {}
                        want[n] = t
                        for k, v in props do
                            local val = resolve_token(v, sheets)
                            if val ~= nil then t[k] = { val, tr[k] or tr["*"] } end
                        end
                    end
                end
            end
        end
    end
    -- Apply, remembering originals; restore what nothing sets any more.
    for inst, props in want do
        local orig = styled[inst] or {}
        styled[inst] = orig
        local last = style_targets[inst] or {}
        style_targets[inst] = last
        for k, e in props do
            if orig[k] == nil then
                local ok, cur = pcall(function() return inst[k] end)
                if ok then orig[k] = { cur } end
            end
            if orig[k] and last[k] ~= e[1] then
                last[k] = e[1]
                if e[2] then
                    pcall(function() svc("TweenService"):Create(inst, e[2], { [k] = e[1] }):Play() end)
                else
                    pcall(function() inst[k] = e[1] end)
                end
            end
        end
    end
    for inst, orig in styled do
        local props = want[inst] or {}
        for k, o in orig do
            if props[k] == nil then
                if inst.Parent then pcall(function() inst[k] = o[1] end) end
                orig[k] = nil
                if style_targets[inst] then style_targets[inst][k] = nil end
            end
        end
        if next(orig) == nil then
            styled[inst] = nil
            style_targets[inst] = nil
        end
    end
end

---------------------------------------------------------- ParticleEmitter2D
-- Emit(n) adds n particles now, Clear() removes them (drawn by
-- src/gui_fx.rs, which also runs Rate).
methods["ParticleEmitter2D.Emit"] = function(self, n)
    n = n == nil and 16 or tonumber(n)
    if n == nil then error("Unable to cast value to int", 0) end
    if n >= 1 then G.p2d(self, math.floor(n)) end
end
methods["ParticleEmitter2D.Clear"] = function(self) G.p2d(self, 0) end

-------------------------------------------------------- Input action system
-- InputContext (Enabled, Priority, Sink) > InputAction (Type, Enabled) >
-- InputBinding (KeyCode or Up/Down/Left/Right/Forward/Backward, modifiers,
-- Scale). An action's state starts at its type's zero; Bool actions fire
-- Pressed / Released, every change fires StateChanged(state).
local action_state = setmetatable({}, { __mode = "k" })
local action_type = setmetatable({}, { __mode = "k" })
local function zero_of(t)
    if t == Enum.InputActionType.Bool then return false end
    if t == Enum.InputActionType.Direction1D then return 0 end
    if t == Enum.InputActionType.Direction2D then return Vector2.zero end
    if t == Enum.InputActionType.Direction3D then return Vector3.zero end
    if t == Enum.InputActionType.ViewportPosition then return Vector2.new(-1, -1) end
    return false
end
local function state_of(a)
    local s = action_state[a]
    if s == nil then return zero_of(a.Type) end
    return s
end
local function action_live(a)
    local ctx = a.Parent
    return a.Enabled and isa(ctx, "InputContext") and ctx.Enabled and a:IsDescendantOf(game)
end
local function set_state(a, v)
    local old = state_of(a)
    if old == v then return end
    action_state[a] = v
    if a.Type == Enum.InputActionType.Bool then
        fire(a, v and "Pressed" or "Released")
    end
    fire(a, "StateChanged", v)
end
methods["InputAction.GetState"] = function(a)
    if not action_live(a) then return zero_of(a.Type) end
    return state_of(a)
end
methods["InputAction.Fire"] = function(a, v)
    local t = a.Type
    if t == Enum.InputActionType.Bool and type(v) ~= "boolean" then
        error("Action:Fire() on InputActionType.Bool should be called with a bool state", 0)
    end
    if not action_live(a) then return end
    local ok = (t == Enum.InputActionType.Bool and type(v) == "boolean")
        or (t == Enum.InputActionType.Direction1D and type(v) == "number")
        or ((t == Enum.InputActionType.Direction2D or t == Enum.InputActionType.ViewportPosition) and typeof(v) == "Vector2")
        or (t == Enum.InputActionType.Direction3D and typeof(v) == "Vector3")
    if ok then set_state(a, v) end
end
methods["InputBinding.Fire"] = function(b, v)
    if b.Type ~= Enum.InputBindingType.Scriptable then
        error("InputBinding:Fire() can only be called when Type is Scriptable", 0)
    end
    local a = b.Parent
    if isa(a, "InputAction") then methods["InputAction.Fire"](a, v) end
end
-- Type changes reset the state to the new type's zero (Released first when
-- a Bool action takes over from a non-zero state).
set["InputAction.Type"] = function(a, _, v)
    local before_t = a.Type
    local before = state_of(a)
    if typeof(v) ~= "EnumItem" then return false end
    if v == before_t then return false end
    __rs_set_raw(a, "Type", v)
    action_state[a] = nil
    local z = zero_of(v)
    if before ~= z then
        if v == Enum.InputActionType.Bool and before ~= zero_of(before_t) then fire(a, "Released") end
        fire(a, "StateChanged", z)
    end
    return true
end
local function reset(a)
    local z = zero_of(a.Type)
    if state_of(a) ~= z then set_state(a, z) end
end
set["InputAction.Enabled"] = function(a, _, v)
    if v == false then reset(a) end
    return false
end
set["InputContext.Enabled"] = function(ctx, _, v)
    if v == false then
        for _, a in ctx:GetChildren() do
            if a:IsA("InputAction") then reset(a) end
        end
    end
    return false
end
local last_device = "Keyboard"
local function binding_device(b)
    local k = b.KeyCode
    if k ~= KEY_NONE and k.Name:find("^Button") or k.Name:find("^Thumbstick") or k.Name:find("^DPad") then return "Gamepad" end
    return "Keyboard"
end
get["InputAction.PreferredBinding"] = function(a)
    local first
    for _, b in a:GetChildren() do
        if b:IsA("InputBinding") then
            first = first or b
            if binding_device(b) == last_device then return b end
        end
    end
    return first
end
get["InputActionLabel.ResolvedText"] = function() return "" end
get["InputActionLabel.ResolvedImageContent"] = function() return Content.none end
-- Held keys / buttons per binding, for the action's value.
local held = {}
local function mods_ok(b)
    local uis = svc("UserInputService")
    for _, m in { b.PrimaryModifier, b.SecondaryModifier } do
        if m ~= KEY_NONE and not uis:IsKeyDown(m) then return false end
    end
    return true
end
local function binding_value(a, b)
    local t = a.Type
    local s = b.Scale
    local function h(k) return k ~= KEY_NONE and held[k] == true end
    if t == Enum.InputActionType.Bool then
        return h(b.KeyCode) and mods_ok(b)
    elseif t == Enum.InputActionType.Direction1D then
        local v = 0
        if h(b.KeyCode) then v = 1 end
        if h(b.Up) then v += 1 end
        if h(b.Down) then v -= 1 end
        return v * s
    elseif t == Enum.InputActionType.Direction2D then
        local v = Vector2.new((h(b.Right) and 1 or 0) - (h(b.Left) and 1 or 0), (h(b.Up) and 1 or 0) - (h(b.Down) and 1 or 0))
        if b.ClampMagnitudeToOne and v.Magnitude > 1 then v = v.Unit end
        return v * b.Vector2Scale
    elseif t == Enum.InputActionType.Direction3D then
        local v = Vector3.new((h(b.Right) and 1 or 0) - (h(b.Left) and 1 or 0), (h(b.Up) and 1 or 0) - (h(b.Down) and 1 or 0),
            (h(b.Backward) and 1 or 0) - (h(b.Forward) and 1 or 0))
        if b.ClampMagnitudeToOne and v.Magnitude > 1 then v = v.Unit end
        return v * b.Vector3Scale
    end
    return nil
end
local function binding_uses(b, key)
    if b.Type == Enum.InputBindingType.Scriptable then return false end
    for _, k in { b.KeyCode, b.Up, b.Down, b.Left, b.Right, b.Forward, b.Backward } do
        if k == key then return true end
    end
    return false
end
-- Runs the input action system for one key / button; true when a Sink
-- context took it.
local function actions_input(key, down)
    held[key] = down or nil
    local ctxs = {}
    for c in tracked.InputContext do
        if c.Enabled and c:IsDescendantOf(game) then table.insert(ctxs, c) end
    end
    table.sort(ctxs, function(a, b) return a.Priority > b.Priority end)
    for _, ctx in ctxs do
        local took = false
        for _, a in ctx:GetChildren() do
            if a:IsA("InputAction") and a.Enabled then
                local uses, value = false, nil
                for _, b in a:GetChildren() do
                    if b:IsA("InputBinding") and binding_uses(b, key) then
                        uses = true
                        local v = binding_value(a, b)
                        if a.Type == Enum.InputActionType.Bool then
                            value = value or v
                        elseif v ~= nil then
                            value = value and (value + v) or v
                        end
                    end
                end
                if uses then
                    took = true
                    if value ~= nil then set_state(a, value) end
                end
            end
        end
        if took and ctx.Sink then return true end
    end
    return false
end
local mouse_keys = { MouseButton1 = "MouseLeftButton", MouseButton2 = "MouseRightButton", MouseButton3 = "MouseMiddleButton" }

------------------------------------------------------- Mouse / UIS extras
-- The legacy Mouse's KeyDown / KeyUp (lower-case character), Move,
-- WheelForward / WheelBackward and Idle; UserInputService.JumpRequest.
local mouse_moved = false
local prompt_key
local function on_input(state, uit, key, x, y, z, gpe)
    local mouse = G.mouse()
    if uit == "Keyboard" then
        local kc = Enum.KeyCode[key]
        if kc and not gpe then
            local down = state == "Begin"
            if state == "Begin" or state == "End" then
                actions_input(kc, down)
                if prompt_key then prompt_key(kc, down) end
            end
        end
        if mouse and not gpe and (state == "Begin" or state == "End") then
            local kc2 = Enum.KeyCode[key]
            local ch = kc2 and kc2.Value < 256 and string.char(kc2.Value):lower() or key:lower()
            fire(mouse, state == "Begin" and "KeyDown" or "KeyUp", ch)
        end
        if key == "Space" and state == "Begin" and not gpe then
            fire(svc("UserInputService"), "JumpRequest")
        end
        last_device = "Keyboard"
    elseif mouse_keys[uit] then
        if not gpe and (state == "Begin" or state == "End") then
            actions_input(Enum.KeyCode[mouse_keys[uit]], state == "Begin")
        end
        last_device = "Keyboard"
    elseif uit == "MouseMovement" then
        mouse_moved = true
        if mouse then fire(mouse, "Move") end
    elseif uit == "MouseWheel" then
        if mouse then fire(mouse, (z or 0) > 0 and "WheelForward" or "WheelBackward") end
    elseif uit:find("^Gamepad") then
        local kc = Enum.KeyCode[key]
        if kc and not gpe and (state == "Begin" or state == "End") then actions_input(kc, state == "Begin") end
        last_device = "Gamepad"
    end
end
do
    local inner = __rs_input_event
    if inner then
        __rs_input_event = function(state, uit, key, x, y, z, dx, dy, gpe)
            local ok, err = pcall(on_input, state, uit, key, x, y, z, gpe)
            if not ok then log("error", "gui_input: " .. tostring(err)) end
            return inner(state, uit, key, x, y, z, dx, dy, gpe)
        end
    end
end
local function step_mouse()
    local mouse = G.mouse()
    if mouse and not mouse_moved then fire(mouse, "Idle") end
    mouse_moved = false
end

-------------------------------------------------------- Gamepad / haptics
methods["GamepadService.EnableGamepadCursor"] = function(self, obj)
    if obj ~= nil and typeof(obj) ~= "Instance" then error("Unable to cast value to Object", 0) end
    -- No cursor without a gamepad (Studio: GamepadCursorEnabled stays false).
    if G.gamepads() > 0 then
        __rs_set_raw(self, "GamepadCursorEnabled", true)
        if isa(obj, "GuiObject") then svc("GuiService").SelectedObject = obj end
    end
end
methods["GamepadService.DisableGamepadCursor"] = function(self)
    if self.GamepadCursorEnabled then __rs_set_raw(self, "GamepadCursorEnabled", false) end
end
local function pad_index(t)
    if typeof(t) ~= "EnumItem" then return nil end
    local n = t.Name:match("^Gamepad(%d)$")
    return n and tonumber(n)
end
local function pad_connected(t)
    local i = pad_index(t)
    return i ~= nil and i <= G.gamepads()
end
methods["HapticService.IsVibrationSupported"] = function(_, t)
    if typeof(t) ~= "EnumItem" then error("Unable to cast value to token", 0) end
    return pad_connected(t)
end
methods["HapticService.IsMotorSupported"] = function(_, t, motor)
    if typeof(t) ~= "EnumItem" or typeof(motor) ~= "EnumItem" then error("Unable to cast value to token", 0) end
    return pad_connected(t) and (motor == Enum.VibrationMotor.Large or motor == Enum.VibrationMotor.Small)
end
local motors = {}
methods["HapticService.SetMotor"] = function(_, t, motor, ...)
    if typeof(t) ~= "EnumItem" or typeof(motor) ~= "EnumItem" then error("Unable to cast value to token", 0) end
    if select("#", ...) == 0 then
        log("error", "Error in HapticService:SetMotor no values found for vibration.")
        return
    end
    motors[t.Name .. "/" .. motor.Name] = table.pack(...)
end
methods["HapticService.GetMotor"] = function(_, t, motor)
    if typeof(t) ~= "EnumItem" or typeof(motor) ~= "EnumItem" then error("Unable to cast value to token", 0) end
    local v = motors[t.Name .. "/" .. motor.Name]
    if not v then return end
    return table.unpack(v, 1, v.n)
end
-- HapticEffect: in the DataModel, a play ends after its type's length
-- (Studio: UIHover 35 ms, UIClick 55, UINotification 0.4 s, collision 0.3,
-- explosion 0.8, Custom at once); Looped plays until Stop.
local effect_len = { UIHover = 0.035, UIClick = 0.055, UINotification = 0.4, GameplayCollision = 0.3, GameplayExplosion = 0.8, Custom = 0 }
local playing_fx = {}
local fx_keys = setmetatable({}, { __mode = "k" })
methods["HapticEffect.Play"] = function(fx)
    if not fx:IsDescendantOf(game) then return end
    if fx.Looped then playing_fx[fx] = math.huge return end
    playing_fx[fx] = G.time() + (effect_len[fx.Type.Name] or 0)
end
methods["HapticEffect.Stop"] = function(fx) playing_fx[fx] = nil end
methods["HapticEffect.SetWaveformKeys"] = function(fx, keys)
    if fx.Type ~= Enum.HapticEffectType.Custom then error("Cannot modify non-custom haptic effect!", 0) end
    if type(keys) ~= "table" then error("Variant cast failed", 0) end
    for _, k in keys do
        if typeof(k) ~= "FloatCurveKey" then error("Variant cast failed", 0) end
    end
    fx_keys[fx] = copy(keys)
end
local function step_fx()
    local now = G.time()
    for fx, at in playing_fx do
        if now >= at then
            playing_fx[fx] = nil
            if fx.Parent then fire(fx, "Ended") end
        end
    end
end

------------------------------------------------------------------- VRService
local touchpad = {}
methods["VRService.GetTouchpadMode"] = function(_, pad)
    if typeof(pad) ~= "EnumItem" then error("Unable to cast value to token", 0) end
    return touchpad[pad] or Enum.VRTouchpadMode.VirtualThumbstick
end
methods["VRService.SetTouchpadMode"] = function(vr, pad, mode)
    if typeof(pad) ~= "EnumItem" or typeof(mode) ~= "EnumItem" then error("Unable to cast value to token", 0) end
    if (touchpad[pad] or Enum.VRTouchpadMode.VirtualThumbstick) ~= mode then
        touchpad[pad] = mode
        fire(vr, "TouchpadModeChanged", pad, mode)
    end
end
local gui_input_cf = Enum.UserCFrame.Head
get["VRService.GuiInputUserCFrame"] = function() return gui_input_cf end
set["VRService.GuiInputUserCFrame"] = function(vr, _, v)
    if not is_enum(v, "UserCFrame") then
        error("Unable to assign property GuiInputUserCFrame. EnumItem expected, got " .. typeof(v), 0)
    end
    if v ~= gui_input_cf then gui_input_cf = v changed(vr, "GuiInputUserCFrame") end
    return true
end

------------------------------------------------------------ Dialog
local dialog_use = setmetatable({}, { __mode = "k" })
get["Dialog.InUse"] = function(d) return dialog_use[d] == true end
set["Dialog.InUse"] = function(d, _, v)
    v = v == true
    if (dialog_use[d] == true) ~= v then dialog_use[d] = v changed(d, "InUse") end
    return true
end
local dialog_players = setmetatable({}, { __mode = "k" })
methods["Dialog.GetCurrentPlayers"] = function(d)
    local out = {}
    for p in dialog_players[d] or {} do
        if p.Parent then table.insert(out, p) end
    end
    return out
end

------------------------------------------------------- BillboardGui distance
get["BillboardGui.CurrentDistance"] = function(b)
    if not G.is_client() then return 0 end
    local x, y, z, ok = G.camera()
    if not ok then return 0 end
    local a = b.Adornee or b.Parent
    local pos
    if isa(a, "BasePart") then pos = a.Position
    elseif isa(a, "Attachment") then pos = a.WorldPosition
    elseif isa(a, "Model") then
        local okp, piv = pcall(function() return a:GetPivot() end)
        pos = okp and piv.Position or nil
    end
    if not pos then return 0 end
    return f32((pos - Vector3.new(x, y, z)).Magnitude)
end

------------------------------------------------------- TextGenerator
-- Studio generates through the place's universe (/user/cloud/v2/
-- universes/{id}:generateText) with the user's session; it must be in the
-- DataModel first.
LM.GenerateTextAsync = function(self, request)
    if typeof(self) ~= "Instance" or not self:IsA("TextGenerator") then
        error("Expected ':' not '.' calling member function GenerateTextAsync", 0)
    end
    if not self:IsDescendantOf(game) then error("TextGenerator must be parented into the DataModel.", 0) end
    if type(request) ~= "table" then error("Unable to cast value to Dictionary", 0) end
    local up = request.UserPrompt
    if type(up) ~= "string" then error("UserPrompt should be a string.", 0) end
    local body = { userPrompt = up, systemPrompt = self.SystemPrompt, temperature = self.Temperature }
    if request.ContextToken ~= nil then
        if type(request.ContextToken) ~= "string" then error("ContextToken should be a string.", 0) end
        body.contextToken = request.ContextToken
    end
    if request.MaxTokens ~= nil then
        if type(request.MaxTokens) ~= "number" then
            error("MaxTokens should be a number. Non-integral values will be rounded to the nearest integer.", 0)
        end
        local n = math.floor(request.MaxTokens + 0.5)
        if n < 1 then error("The value of MaxTokens must be at least 1.", 0) end
        body.maxTokens = n
    end
    if request.JsonSchema ~= nil then
        if type(request.JsonSchema) ~= "string" then error("JsonSchema should be a string.", 0) end
        body.responseFormatType = "JSON_SCHEMA"
        body.jsonSchemaStr = request.JsonSchema
    end
    local http = svc("HttpService")
    local id = G.textgen_start(game.GameId, http:JSONEncode(body))
    local status, text
    repeat
        task.wait()
        status, text = G.textgen_poll(id)
    until status ~= nil
    if status == 0 then error("Error while making HTTP request: " .. tostring(text), 0) end
    if status < 200 or status >= 300 then
        error("Error while making HTTP request: " .. G.http_error(status), 0)
    end
    local ok, r = pcall(function() return http:JSONDecode(text) end)
    if not ok or type(r) ~= "table" then error("Cannot parse response.", 0) end
    if type(r.generatedText) ~= "string" then error("Response does not include generated text.", 0) end
    if type(r.contextToken) ~= "string" then error("Response does not include a context token.", 0) end
    if r.model == nil then error("Response does not include the model.", 0) end
    return { GeneratedText = r.generatedText, ContextToken = r.contextToken, Model = r.model }
end

---------------------------------------------------------- ProximityPrompt
-- The client shows each enabled prompt in range (MaxActivationDistance from
-- the character, line of sight when asked), by Exclusivity; its key holds
-- it. Hold: PromptButtonHoldBegan, then at HoldDuration PromptButtonHoldEnded
-- and Triggered, TriggerEnded at release (an early release: HoldEnded only).
-- Each fires on the prompt, then ProximityPromptService (with the prompt),
-- on the client and the server (sent as an engine message).
local shown_prompts = {}
local holds = {}
local function prompt_pos(p)
    local a = p.Parent
    if isa(a, "BasePart") then return a.Position end
    if isa(a, "Attachment") then return a.WorldPosition end
    if isa(a, "Model") then
        local ok, piv = pcall(function() return a:GetPivot() end)
        return ok and piv.Position or nil
    end
    return nil
end
local function both(p, ev, svc_ev, player)
    fire(p, ev, player)
    local pps = svc("ProximityPromptService")
    if pps then fire(pps, svc_ev, p, player) end
end
local function send_server(p, kind)
    if G.send then G.send(p, kind) end
end
local function hold_begin(p)
    local player = local_player()
    if not player or holds[p] then return end
    if p.HoldDuration > 0 then
        holds[p] = { t0 = G.time(), done = false }
        both(p, "PromptButtonHoldBegan", "PromptButtonHoldBegan", player)
        send_server(p, "PromptButtonHoldBegan")
    else
        holds[p] = { t0 = G.time(), done = true }
        both(p, "Triggered", "PromptTriggered", player)
        send_server(p, "Triggered")
    end
end
local function hold_end(p)
    local h = holds[p]
    if not h then return end
    holds[p] = nil
    local player = local_player()
    if h.done then
        both(p, "TriggerEnded", "PromptTriggerEnded", player)
        send_server(p, "TriggerEnded")
    else
        both(p, "PromptButtonHoldEnded", "PromptButtonHoldEnded", player)
        send_server(p, "PromptButtonHoldEnded")
    end
end
methods["ProximityPrompt.InputHoldBegin"] = function(p)
    if G.is_client() then hold_begin(p) end
end
methods["ProximityPrompt.InputHoldEnd"] = function(p)
    if G.is_client() then hold_end(p) end
end
prompt_key = function(kc, down)
    for p in shown_prompts do
        if p.KeyboardKeyCode == kc then
            if down then hold_begin(p) else hold_end(p) end
        end
    end
end
-- Legacy Dialogs: Studio's MainBotChatScript2 with stand-ins for what only
-- CoreGui has. Measured (offline Studio): InUse goes true on the client then the
-- server (GetCurrentPlayers then lists the player), DialogChoiceSelected
-- (player, choice) fires on the client only, InUse false at the end.
local dialog_set = {}
local dialog_added, dialog_removed
local function dialog_track(i)
    if i:IsA("Dialog") and not dialog_set[i] then
        dialog_set[i] = true
        dialog_added:Fire(i)
    end
end
local dialog_ui_started = false
local function start_dialog_ui()
    if dialog_ui_started then return end
    local p = local_player()
    local pg = p and p:FindFirstChildOfClass("PlayerGui")
    if not pg then return end
    dialog_ui_started = true
    dialog_added, dialog_removed = Instance.new("BindableEvent"), Instance.new("BindableEvent")
    -- RobloxGui stand-in: Studio's has ControlFrame.BottomLeftControl at
    -- (0, 0, 1, -46), 130 × 46.
    local rg = Instance.new("ScreenGui")
    rg.Name = "RobloxGui"
    rg.ResetOnSpawn = false
    rg.DisplayOrder = 100000
    local cf = Instance.new("Frame")
    cf.Name = "ControlFrame"
    cf.Size = UDim2.fromScale(1, 1)
    cf.BackgroundTransparency = 1
    cf.Parent = rg
    local bl = Instance.new("Frame")
    bl.Name = "BottomLeftControl"
    bl.Position = UDim2.new(0, 0, 1, -46)
    bl.Size = UDim2.fromOffset(130, 46)
    bl.BackgroundTransparency = 1
    bl.Parent = cf
    local holder = Instance.new("ScreenGui")
    holder.Name = "DialogBubbles"
    holder.ResetOnSpawn = false
    holder.Parent = pg
    rg.Parent = pg
    local rrs = {}
    local remote = {}
    function remote:FireServer(dialog, on)
        if G.send then G.send(dialog, "SetDialogInUse", on == true) end
    end
    function rrs:WaitForChild() return remote end
    for _, d in game:GetDescendants() do dialog_track(d) end
    local dialog_added_sig, dialog_removing_sig = G.class_signals({ "Dialog" })
    dialog_added_sig:Connect(dialog_track)
    dialog_removing_sig:Connect(function(i)
        if dialog_set[i] then dialog_set[i] = nil dialog_removed:Fire(i) end
    end)
    local D = {
        coregui = holder,
        robloxgui = rg,
        rrs = rrs,
        analytics = { TrackEvent = function() end },
        tenfoot = { IsEnabled = function() return false end },
        utility = { IsSmallTouchScreen = function() return false end },
        -- GameTranslator: the text as written (no localization table here).
        translator = { TranslateAndRegister = function(_, label, _, text) label.Text = text end },
        signal = function(dialog, player, choice)
            fire(dialog, "DialogChoiceSelected", player, choice)
        end,
        chat_local = function(...) svc("Chat"):Chat(...) end,
        item_added = dialog_added.Event,
        item_removed = dialog_removed.Event,
        dialogs = function()
            local out = {}
            for d in dialog_set do table.insert(out, d) end
            return out
        end,
        -- GuiService:GetClosestDialogToPosition: the nearest dialog on a part.
        closest = function(_, pos)
            local best, bd
            for d in dialog_set do
                local part = d.Parent
                if part and part:IsA("BasePart") and d:IsDescendantOf(workspace) then
                    local dist = (part.Position - pos).Magnitude
                    if not bd or dist < bd then best, bd = d, dist end
                end
            end
            return best
        end,
    }
    local ok, f = pcall(G.dialog_ui)
    if ok and f then
        task.spawn(f, D)
    elseif not ok then
        log("warn", "Dialog UI: " .. tostring(f))
    end
end

local prompt_ui_started = false
local function step_prompts()
    if not dialog_ui_started then start_dialog_ui() end
    if not prompt_ui_started and local_player() then
        prompt_ui_started = true
        local ok, f = pcall(G.prompt_ui)
        if ok and f then
            task.spawn(f)
        elseif not ok then
            log("warn", "ProximityPrompt UI: " .. tostring(f))
        end
    end
    local now = G.time()
    for p, h in holds do
        if not h.done and now - h.t0 >= p.HoldDuration then
            h.done = true
            local player = local_player()
            both(p, "PromptButtonHoldEnded", "PromptButtonHoldEnded", player)
            send_server(p, "PromptButtonHoldEnded")
            both(p, "Triggered", "PromptTriggered", player)
            send_server(p, "Triggered")
        end
    end
    local player = local_player()
    local char = player and player.Character
    local root = char and (char:FindFirstChild("HumanoidRootPart") or char.PrimaryPart)
    local want = {}
    if root then
        local origin = root.Position
        local best_key, best_all = {}, nil
        local cands = {}
        for p in tracked.ProximityPrompt do
            if p.Enabled and p:IsDescendantOf(workspace) then
                local pos = prompt_pos(p)
                if pos then
                    local d = (pos - origin).Magnitude
                    local ok = d <= p.MaxActivationDistance
                    if ok and p.RequiresLineOfSight then
                        local params = RaycastParams.new()
                        params.FilterType = Enum.RaycastFilterType.Exclude
                        local excl = { char }
                        local holder = p.Parent
                        if isa(holder, "Attachment") then holder = holder.Parent end
                        if holder then table.insert(excl, holder) end
                        params.FilterDescendantsInstances = excl
                        local hitr = workspace:Raycast(origin, pos - origin, params)
                        if hitr then ok = false end
                    end
                    if ok then table.insert(cands, { p = p, d = d }) end
                end
            end
        end
        for _, c in cands do
            local ex = c.p.Exclusivity
            if ex == Enum.ProximityPromptExclusivity.AlwaysShow then
                want[c.p] = true
            elseif ex == Enum.ProximityPromptExclusivity.OneGlobally then
                if not best_all or c.d < best_all.d then best_all = c end
            else
                local k = c.p.KeyboardKeyCode
                if not best_key[k] or c.d < best_key[k].d then best_key[k] = c end
            end
        end
        for _, c in best_key do want[c.p] = true end
        if best_all then
            local clash = false
            for _, c in best_key do
                if c.d < best_all.d then clash = true end
            end
            if not clash then want[best_all.p] = true end
        end
    end
    local pps = svc("ProximityPromptService")
    if pps and not pps.Enabled then want = {} end
    for p in shown_prompts do
        if not want[p] then
            shown_prompts[p] = nil
            if holds[p] then hold_end(p) end
            if pps then fire(pps, "PromptHidden", p) end
            fire(p, "PromptHidden")
        end
    end
    for p in want do
        if not shown_prompts[p] then
            shown_prompts[p] = true
            if pps then fire(pps, "PromptShown", p, Enum.ProximityPromptInputType.Keyboard) end
            fire(p, "PromptShown", Enum.ProximityPromptInputType.Keyboard)
        end
    end
end
-- The server hears the client's prompt input (gui_input.rs engine_message).
function M.remote_in(p, player, kind, ...)
    if kind == "SetDialogInUse" and typeof(p) == "Instance" and p:IsA("Dialog") then
        local on = ...
        local set_ = dialog_players[p] or {}
        dialog_players[p] = set_
        set_[player] = on and true or nil
        p.InUse = on == true
        return
    end
    if typeof(p) ~= "Instance" or not p:IsA("ProximityPrompt") then return end
    local svc_ev = ({ Triggered = "PromptTriggered", TriggerEnded = "PromptTriggerEnded",
        PromptButtonHoldBegan = "PromptButtonHoldBegan", PromptButtonHoldEnded = "PromptButtonHoldEnded" })[kind]
    if svc_ev then both(p, kind, svc_ev, player) end
end

---------------------------------------------------------------------- step
-- Drops tracked instances that were destroyed (no Parent read possible).
local function prune()
    for _, set_ in tracked do
        for i in set_ do
            if not pcall(function() return i.Parent end) then set_[i] = nil end
        end
    end
    for i in shown_prompts do
        if not pcall(function() return i.Parent end) then shown_prompts[i] = nil holds[i] = nil end
    end
    for d in dialog_set do
        if not pcall(function() return d.Parent end) then dialog_set[d] = nil end
    end
    if hover_a and not pcall(function() return hover_a.Parent end) then hover_a = nil end
    if press and not pcall(function() return press.a.Parent end) then press = nil end
    if focused and not pcall(function() return focused.Parent end) then focused = nil G.set_focus(nil) end
    if equipped_tool and not pcall(function() return equipped_tool.Parent end) then equipped_tool = nil end
    for fx in playing_fx do
        if not pcall(function() return fx.Parent end) then playing_fx[fx] = nil end
    end
    for pl in tweening do
        if not pcall(function() return pl.Parent end) then tweening[pl] = nil end
    end
    for inst in styled do
        if not pcall(function() return inst.Parent end) then styled[inst] = nil style_targets[inst] = nil end
    end
end

local function step_all(dt)
    refresh_game()
    prune()
    step_fx()
    if not G.is_client() then return end
    step_inspect()
    start_tracking()
    step_focus()
    step_tools()
    step_pages(dt)
    step_drags()
    step_adorns()
    step_mouse()
    step_prompts()
    step_styles()
end

function M.step(dt)
    local ok, err = xpcall(step_all, debug.traceback, dt)
    if not ok then log("error", "gui_input step: " .. tostring(err):gsub("\n", " | ")) end
end

return M
