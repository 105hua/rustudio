-- rustudio anim domain (src/domains/anim.rs): the members written in Luau.
-- Every behaviour here was measured in Roblox Studio in an offline Studio
-- (2026-11-07); checks/parity/anim.md has the notes. Loaded once per Lua
-- state with (create, fire, setbin); returns (methods, props, writes).
local create, fire, setbin = ...
local A = {}

local function cast_name(v)
    local t = typeof(v)
    if t == "number" then return "double" elseif t == "boolean" then return "bool" end
    return t
end
local function toint(v, what)
    if type(v) == "number" then return if v >= 0 then math.floor(v) else math.ceil(v) end
    if type(v) == "string" and tonumber(v) then return math.floor(tonumber(v)) end
    error("Unable to cast " .. cast_name(v) .. " to " .. (what or "int"), 0)
end
local function tofloat(v)
    if type(v) == "number" then return v end
    if type(v) == "string" and tonumber(v) then return tonumber(v) end
    error("Unable to cast " .. cast_name(v) .. " to float", 0)
end
local function is_inst(v, class) return typeof(v) == "Instance" and (class == nil or v:IsA(class)) end

------------------------------------------------------------------ curves
-- FloatCurve.ValuesAndTimes (Studio 0.746 wire, as Studio saves it): u32 2,
-- u32 n, n × {u16 mode | flags << 8 (1 = left tangent, 2 = right tangent),
-- f32 value, f32 left, f32 right}, then u32 1, u32 n, n × i32 time in
-- 1/2400 s. The keys live in that property, so they save, replicate and
-- play (library.luau's CurveAnimation playback reads the same bytes).
local function curve_data(curve)
    local ok, data = pcall(function() return curve.ValuesAndTimes end)
    return if ok then data else nil
end
local function fc_read(curve)
    local data = curve_data(curve)
    if type(data) ~= "string" or #data < 16 then return {} end
    local b = buffer.fromstring(data)
    if buffer.readu32(b, 0) ~= 2 then return {} end
    local n = buffer.readu32(b, 4)
    local times = 8 + n * 14
    if times + 8 + n * 4 > #data or buffer.readu32(b, times) ~= 1 or buffer.readu32(b, times + 4) ~= n then
        return {}
    end
    local keys = table.create(n)
    for i = 0, n - 1 do
        local off = 8 + i * 14
        local word = buffer.readu16(b, off)
        local flags = bit32.rshift(word, 8)
        local k = { t = buffer.readi32(b, times + 8 + i * 4) / 2400, v = buffer.readf32(b, off + 2), mode = bit32.band(word, 0xff) }
        if bit32.band(flags, 1) ~= 0 then k.lt = buffer.readf32(b, off + 6) end
        if bit32.band(flags, 2) ~= 0 then k.rt = buffer.readf32(b, off + 10) end
        keys[i + 1] = k
    end
    return keys
end
local function fc_write(curve, keys)
    local n = #keys
    local b = buffer.create(16 + n * 18)
    buffer.writeu32(b, 0, 2)
    buffer.writeu32(b, 4, n)
    for i, k in keys do
        local off = 8 + (i - 1) * 14
        local flags = (if k.lt then 1 else 0) + (if k.rt then 2 else 0)
        buffer.writeu16(b, off, k.mode + flags * 256)
        buffer.writef32(b, off + 2, k.v)
        buffer.writef32(b, off + 6, k.lt or 0)
        buffer.writef32(b, off + 10, k.rt or 0)
    end
    local times = 8 + n * 14
    buffer.writeu32(b, times, 1)
    buffer.writeu32(b, times + 4, n)
    for i, k in keys do buffer.writei32(b, times + 8 + (i - 1) * 4, math.round(k.t * 2400)) end
    setbin(curve, "ValuesAndTimes", buffer.tostring(b))
end
local MODES = { [0] = "Constant", "Linear", "Cubic" }
local function mode_item(m) return Enum.KeyInterpolationMode[MODES[m] or "Cubic"] end

-- Hermite segment as Studio evaluates it (tangents in value per second,
-- missing tangents are 0): measured 0.25 / 0.1875 / 0.9375 cases.
local function seg_alpha(a, b, t)
    local dt = b.t - a.t
    if dt <= 0 then return 0 end
    return (t - a.t) / dt, dt
end
local function fc_value(keys, t)
    local n = #keys
    if n == 0 then return nil end
    if t <= keys[1].t then return keys[1].v end
    if t >= keys[n].t then return keys[n].v end
    for i = 1, n - 1 do
        local a, b = keys[i], keys[i + 1]
        if t < b.t then
            if a.mode == 0 then return a.v end
            local u, dt = seg_alpha(a, b, t)
            if a.mode == 1 then return a.v + (b.v - a.v) * u end
            local u2, u3 = u * u, u * u * u
            return (2*u3 - 3*u2 + 1) * a.v + (u3 - 2*u2 + u) * dt * (a.rt or 0)
                + (-2*u3 + 3*u2) * b.v + (u3 - u2) * dt * (b.lt or 0)
        end
    end
    return keys[n].v
end

-- Shared key-list operations (FloatCurve / RotationCurve / MarkerCurve).
local function find_time(keys, t)
    local tick = math.round(t * 2400)
    for i, k in keys do
        local kt = math.round(k.t * 2400)
        if kt == tick then return i, true end
        if kt > tick then return i, false end
    end
    return #keys + 1, false
end
local function insert_key(keys, k)
    local i, same = find_time(keys, k.t)
    if same then keys[i] = k else table.insert(keys, i, k) end
    return { not same, i - 1 }
end
local function indices_at(keys, t)
    local n = #keys
    if n == 0 then return { 0, 0 } end
    if n == 1 then return { 1, 1 } end
    local i = 1
    for j = 1, n do if keys[j].t <= t then i = j end end
    i = math.clamp(i, 1, n - 1)
    return { i, i + 1 }
end
local function remove_at(keys, start, count)
    start = toint(start)
    count = if count == nil then 1 else toint(count)
    if start < 1 then error("Invalid argument provided: startingIndex.", 0) end
    if count < 0 then error("Invalid argument provided: count.", 0) end
    local removed = 0
    while removed < count and keys[start] do
        table.remove(keys, start)
        removed += 1
    end
    return removed
end
-- Studio drops a left tangent whose segment isn't cubic (its warning
-- really says "%zu").
local function check_tangents(keys)
    for i, k in keys do
        if k.lt then
            if i == 1 then k.lt = 0
            elseif keys[i - 1].mode ~= 2 then
                warn("Key at index %zu defines a left-tangent, but the previous segment isn't cubic, therefore this tangent is ignored.")
                k.lt = nil
            end
        end
    end
end

local function fc_key(k)
    local key = FloatCurveKey.new(k.t, k.v, mode_item(k.mode))
    if k.lt then key.LeftTangent = k.lt end
    if k.rt then key.RightTangent = k.rt end
    return key
end
local function fc_from(key)
    if typeof(key) ~= "FloatCurveKey" then error("Unable to cast " .. cast_name(key) .. " to FloatCurveKey", 0) end
    return { t = key.Time, v = key.Value, mode = key.Interpolation.Value, lt = key.LeftTangent, rt = key.RightTangent }
end

local FC = {}
function FC.GetKeys(self)
    local out = {}
    for i, k in fc_read(self) do out[i] = fc_key(k) end
    return out
end
function FC.GetKeyAtIndex(self, index)
    index = toint(index)
    local k = fc_read(self)[index]
    if not k then error("FloatCurve index is out of range", 0) end
    return fc_key(k)
end
function FC.GetKeyIndicesAtTime(self, t) return indices_at(fc_read(self), tofloat(t)) end
function FC.GetValueAtTime(self, t) return fc_value(fc_read(self), tofloat(t)) end
function FC.InsertKey(self, key)
    local keys = fc_read(self)
    local r = insert_key(keys, fc_from(key))
    check_tangents(keys)
    fc_write(self, keys)
    return r
end
function FC.RemoveKeyAtIndex(self, start, count)
    local keys = fc_read(self)
    local n = remove_at(keys, start, count)
    if n > 0 then fc_write(self, keys) end
    return n
end
function FC.SetKeys(self, list)
    if type(list) ~= "table" then error("Unable to cast " .. cast_name(list) .. " to Array", 0) end
    local keys = {}
    for _, key in list do
        if typeof(key) ~= "FloatCurveKey" then error("Variant cast failed", 0) end
        local k = fc_from(key)
        local i, same = find_time(keys, k.t)
        if not same then table.insert(keys, i, k) end
    end
    check_tangents(keys)
    fc_write(self, keys)
    return #keys
end

-- RotationCurve.ValuesAndTimes (Studio 0.746): u32 1, u32 n, n × {u8 mode
-- | flags << 2 (4 = left tangent, 8 = right tangent; Studio sets both),
-- f32 quaternion x, y, z, w, f32 left, f32 right}, then u32 1, u32 n,
-- n × i32 time in 1/2400 s.
-- Read once per curve, kept while the session runs, written back on change.
local rot_keys = setmetatable({}, { __mode = "k" })
local function rc_decode(curve)
    local data = curve_data(curve)
    if type(data) ~= "string" or #data < 16 then return {} end
    local b = buffer.fromstring(data)
    if buffer.readu32(b, 0) ~= 1 then return {} end
    local n = buffer.readu32(b, 4)
    local times = 8 + n * 25
    if times + 8 + n * 4 > #data or buffer.readu32(b, times) ~= 1 or buffer.readu32(b, times + 4) ~= n then return {} end
    local keys = table.create(n)
    for i = 0, n - 1 do
        local off = 8 + i * 25
        local x, y, z, w = buffer.readf32(b, off + 1), buffer.readf32(b, off + 5), buffer.readf32(b, off + 9), buffer.readf32(b, off + 13)
        keys[i + 1] = { t = buffer.readi32(b, times + 8 + i * 4) / 2400, mode = bit32.band(buffer.readu8(b, off), 3),
            cf = CFrame.new(0, 0, 0, x, y, z, w), lt = buffer.readf32(b, off + 17), rt = buffer.readf32(b, off + 21) }
    end
    return keys
end
local function rc_write(curve, keys)
    local n = #keys
    local b = buffer.create(16 + n * 29)
    buffer.writeu32(b, 0, 1)
    buffer.writeu32(b, 4, n)
    for i, k in keys do
        local off = 8 + (i - 1) * 25
        local axis, angle = k.cf:ToAxisAngle()
        local sa = math.sin(angle / 2)
        buffer.writeu8(b, off, k.mode + 12)
        buffer.writef32(b, off + 1, axis.X * sa)
        buffer.writef32(b, off + 5, axis.Y * sa)
        buffer.writef32(b, off + 9, axis.Z * sa)
        buffer.writef32(b, off + 13, math.cos(angle / 2))
        buffer.writef32(b, off + 17, k.lt or 0)
        buffer.writef32(b, off + 21, k.rt or 0)
    end
    local times = 8 + n * 25
    buffer.writeu32(b, times, 1)
    buffer.writeu32(b, times + 4, n)
    for i, k in keys do buffer.writei32(b, times + 8 + (i - 1) * 4, math.round(k.t * 2400)) end
    setbin(curve, "ValuesAndTimes", buffer.tostring(b))
end
local function rc_list(c)
    local l = rot_keys[c]
    if not l then l = rc_decode(c); rot_keys[c] = l end
    return l
end
__rs_anim_rotation_list = rc_list
local function rc_key(k)
    local key = RotationCurveKey.new(k.t, k.cf, mode_item(k.mode))
    key.LeftTangent = k.lt
    key.RightTangent = k.rt
    return key
end
local function smooth(u) return u * u * (3 - 2 * u) end
local function rc_value(keys, t)
    local n = #keys
    if n == 0 then return nil end
    if t <= keys[1].t then return keys[1].cf end
    if t >= keys[n].t then return keys[n].cf end
    for i = 1, n - 1 do
        local a, b = keys[i], keys[i + 1]
        if t < b.t then
            if a.mode == 0 then return a.cf end
            local u, dt = seg_alpha(a, b, t)
            if a.mode == 2 then
                local u2, u3 = u * u, u * u * u
                u = smooth(u) + (u3 - 2*u2 + u) * dt * a.rt + (u3 - u2) * dt * b.lt
            end
            return a.cf:Lerp(b.cf, u)
        end
    end
    return keys[n].cf
end
__rs_anim_rotation_value = rc_value
local RC = {}
function RC.GetKeys(self)
    local out = {}
    for i, k in rc_list(self) do out[i] = rc_key(k) end
    return out
end
function RC.GetKeyAtIndex(self, index)
    local k = rc_list(self)[toint(index)]
    if not k then error("RotationCurve index is out of range", 0) end
    return rc_key(k)
end
function RC.GetKeyIndicesAtTime(self, t) return indices_at(rc_list(self), tofloat(t)) end
function RC.GetValueAtTime(self, t) return rc_value(rc_list(self), tofloat(t)) end
function RC.InsertKey(self, key)
    if typeof(key) ~= "RotationCurveKey" then error("Unable to cast " .. cast_name(key) .. " to RotationCurveKey", 0) end
    local keys = rc_list(self)
    local r = insert_key(keys, { t = key.Time, cf = key.Value, mode = key.Interpolation.Value, lt = key.LeftTangent, rt = key.RightTangent })
    rc_write(self, keys)
    return r
end
function RC.RemoveKeyAtIndex(self, start, count)
    local keys = rc_list(self)
    local n = remove_at(keys, start, count)
    if n > 0 then rc_write(self, keys) end
    return n
end
-- Studio's RotationCurve:SetKeys can't read RotationCurveKeys out of the
-- array: any non-empty array fails its cast; an empty one clears.
function RC.SetKeys(self, list)
    if type(list) ~= "table" then error("Unable to cast " .. cast_name(list) .. " to Array", 0) end
    if next(list) ~= nil then error("Variant cast failed", 0) end
    rot_keys[self] = {}
    rc_write(self, {})
    return 0
end

-- ValueCurve: Studio's ValueType stays "nil" for a script-made curve and
-- its key calls do nothing (InsertKey / InsertKeyValue return {}, SetKeys
-- 0, GetValueAtTime nil, GetKeyAtIndex out of range).
local VC = {}
function VC.GetKeys() return {} end
function VC.GetKeyAtIndex(self, index) toint(index); error("ValueCurve index is out of range", 0) end
function VC.GetKeyIndicesAtTime(self, t) tofloat(t); return { 0, 0 } end
function VC.GetValueAtTime(self, t) tofloat(t); return nil end
function VC.InsertKey() return {} end
function VC.InsertKeyValue(self, t) tofloat(t); return {} end
function VC.RemoveKeyAtIndex(self, start, count)
    start = toint(start); count = if count == nil then 1 else toint(count)
    if start < 1 then error("Invalid argument provided: startingIndex.", 0) end
    if count < 0 then error("Invalid argument provided: count.", 0) end
    return 0
end
function VC.SetKeys() return 0 end

-- MarkerCurve.ValuesAndTimes: u32 2, u32 n, n NUL-terminated strings,
-- u32 1, u32 n, n × i32 time in 1/2400 s.
local function mc_read(curve)
    local data = curve_data(curve)
    if type(data) ~= "string" or #data < 16 then return {} end
    local b = buffer.fromstring(data)
    if buffer.readu32(b, 0) ~= 2 then return {} end
    local n = buffer.readu32(b, 4)
    if n > #data then return {} end
    local values, offset = {}, 9
    for _ = 1, n do
        local finish = string.find(data, "\0", offset, true)
        if not finish then return {} end
        table.insert(values, string.sub(data, offset, finish - 1))
        offset = finish + 1
    end
    local timeoff = offset - 1
    if timeoff + 8 + n * 4 > #data or buffer.readu32(b, timeoff) ~= 1 then return {} end
    local out = {}
    for i, v in values do out[i] = { t = buffer.readi32(b, timeoff + 8 + (i - 1) * 4) / 2400, v = v } end
    return out
end
local function mc_write(curve, keys)
    local parts = { string.pack("<I4I4", 2, #keys) }
    for _, k in keys do table.insert(parts, k.v .. "\0") end
    table.insert(parts, string.pack("<I4I4", 1, #keys))
    for _, k in keys do table.insert(parts, string.pack("<i4", math.round(k.t * 2400))) end
    setbin(curve, "ValuesAndTimes", table.concat(parts))
end
local MC = {}
function MC.GetMarkers(self)
    local out = {}
    for i, k in mc_read(self) do out[i] = { Time = k.t, Value = k.v } end
    return out
end
function MC.GetMarkerAtIndex(self, index)
    local k = mc_read(self)[toint(index)]
    if not k then error("MarkerCurve index is out of range", 0) end
    return { Time = k.t, Value = k.v }
end
function MC.InsertMarkerAtTime(self, t, marker)
    local keys = mc_read(self)
    local r = insert_key(keys, { t = tofloat(t), v = tostring(marker) })
    mc_write(self, keys)
    return r
end
function MC.RemoveMarkerAtIndex(self, start, count)
    local keys = mc_read(self)
    local n = remove_at(keys, start, count)
    if n > 0 then mc_write(self, keys) end
    return n
end

-- EulerRotationCurve / Vector3Curve: X(), Y(), Z() give the axis
-- FloatCurve child, making it when there is none. GetAnglesAtTime /
-- GetValueAtTime return {x?, y?, z?} with holes for axes without keys.
local function axis(self, name)
    local c = self:FindFirstChild(name)
    if c and c:IsA("FloatCurve") then return c end
    c = create("FloatCurve")
    c.Name = name
    c.Parent = self
    return c
end
local function axes_at(self, t)
    t = tofloat(t)
    local out = {}
    for i, name in { "X", "Y", "Z" } do
        local c = self:FindFirstChild(name)
        if c and c:IsA("FloatCurve") then out[i] = fc_value(fc_read(c), t) end
    end
    return out
end
local AX = {}
function AX.X(self) return axis(self, "X") end
function AX.Y(self) return axis(self, "Y") end
function AX.Z(self) return axis(self, "Z") end
function AX.GetAnglesAtTime(self, t) return axes_at(self, t) end
function AX.GetValueAtTime(self, t) return axes_at(self, t) end
function AX.GetRotationAtTime(self, t)
    local a = axes_at(self, t)
    return CFrame.fromEulerAngles(a[1] or 0, a[2] or 0, a[3] or 0, self.RotationOrder)
end

-- CompositeValueCurve: off in Studio 0.746 (flag CompositeValueCurveEnabled):
-- no component curves for any CurveType, value nil.
local CV = {}
function CV.GetComponentCurves() return {} end
function CV.GetValueAtTime(self, t) tofloat(t); return nil end

local CURVES = { FloatCurve = FC, RotationCurve = RC, ValueCurve = VC, MarkerCurve = MC,
    EulerRotationCurve = AX, Vector3Curve = AX, CompositeValueCurve = CV }

----------------------------------------------- Keyframe / Pose / sequences
-- Add* parents whatever it is given (Studio checks nothing, not even the
-- class); Remove* unparents it only when it is a child; Get* lists the
-- children of the one class.
local function adder(self, child) if typeof(child) == "Instance" then child.Parent = self end end
local function remover(self, child) if typeof(child) == "Instance" and child.Parent == self then child.Parent = nil end end
local function lister(class) return function(self)
    local out = {}
    for _, c in self:GetChildren() do if c:IsA(class) then table.insert(out, c) end end
    return out
end end
local KF = { AddPose = adder, RemovePose = remover, GetPoses = lister("Pose"),
    AddMarker = adder, RemoveMarker = remover, GetMarkers = lister("KeyframeMarker") }
local KFS = { AddKeyframe = adder, RemoveKeyframe = remover, GetKeyframes = lister("Keyframe") }
local POSE = { AddSubPose = adder, RemoveSubPose = remover, GetSubPoses = lister("Pose") }

-- AnimationClip.Length: a KeyframeSequence's last Keyframe time; a
-- CurveAnimation's last key time over its curves (0 when empty).
local function clip_length(clip)
    local len = 0
    if clip:IsA("KeyframeSequence") then
        for _, k in clip:GetChildren() do if k:IsA("Keyframe") then len = math.max(len, k.Time) end end
        return len
    end
    for _, d in clip:GetDescendants() do
        if d:IsA("FloatCurve") then
            local keys = fc_read(d); if #keys > 0 then len = math.max(len, keys[#keys].t) end
        elseif d:IsA("MarkerCurve") then
            local keys = mc_read(d); if #keys > 0 then len = math.max(len, keys[#keys].t) end
        elseif d:IsA("RotationCurve") then
            local keys = rc_list(d); if #keys > 0 then len = math.max(len, keys[#keys].t) end
        end
    end
    return len
end

------------------------------------------------------------- providers
local active_seq = 0
local function provider_reg(self, clip, active)
    if not is_inst(clip, "AnimationClip") then
        error("Invalid AnimationClip instance passed to registerAnimationClip", 0)
    end
    if active then
        local id = "active://" .. active_seq
        active_seq += 1
        __rs_anim_register(id, clip)
        return id
    end
    return __rs_lua_methods.RegisterAnimationClip(self, clip)
end
local function user_check(self, user)
    local id = tonumber(user)
    if not id or id <= 0 then
        error(self.ClassName .. ":GetAnimations userId is not a positive number!", 0)
    end
    return id
end
local ACP = {}
function ACP.RegisterAnimationClip(self, clip) return provider_reg(self, clip, false) end
function ACP.RegisterActiveAnimationClip(self, clip) return provider_reg(self, clip, true) end
-- The clip behind an AnimationId: a registered one as it is, an asset
-- through InsertService (Studio: "Animation failed to load, assetId: …").
function ACP.GetAnimationClipAsync(self, id)
    id = tostring(id)
    local clip = __rs_anim_registered(id)
    if clip then return clip end
    local n = tonumber(string.match(id, "(%d+)%s*$"))
    local fail = "Animation failed to load, assetId: https://assetdelivery.roblox.com/v1/asset?id=" .. tostring(n or id) .. "&serverplaceid=0"
    if not n then error(fail, 0) end
    local state = __rs_asset_ready(n)
    while state == false do
        task.wait()
        state = __rs_asset_ready(n)
    end
    local ok, model = false, nil
    if state == true then ok, model = pcall(__rs_anim_asset, n) end
    if ok and model then
        local c = if model:IsA("AnimationClip") then model else model:FindFirstChildWhichIsA("AnimationClip", true)
        if c then c.Parent = nil; return c end
    end
    error(fail, 0)
end
function ACP.GetClipEvaluatorAsync() error("AnimationClipProvider:GetClipEvaluatorAsync is not currently enabled", 0) end
-- GetAnimations(Async)(userId): an InventoryPages over the user's animations
-- (asset type 24), Studio's request through the login session: GET
-- inventory.roblox.com/v2/users/{id}/inventory/24?limit=50&cursor=<cursor>
-- (the format string in Studio 0.746's exe; the offline Studio's log shows the empty
-- cursor= on the first page). Each page is the reply's "data" entries as
-- their JSON fields (assetId, name, assetType, created, ...), the next page
-- by nextPageCursor. A failed request raises Studio's "HTTP <code> (<text>)".
-- The Pages is the shared one (src/domains/pages.rs).
function ACP.GetAnimationsAsync(self, user)
    local id = user_check(self, user)
    local base = string.format("https://inventory.roblox.com/v2/users/%d/inventory/24?limit=50&cursor=", id)
    local cursor
    return __rs_pages_new("InventoryPages", function()
        local r = __rs_avatar_get(base .. (cursor and game:GetService("HttpService"):UrlEncode(cursor) or "")) or {}
        local page = {}
        for _, it in r.data or {} do table.insert(page, it) end
        cursor = r.nextPageCursor
        return page, cursor == nil or cursor == ""
    end)
end
ACP.GetAnimations = ACP.GetAnimationsAsync
local KSP = { GetAnimationsAsync = ACP.GetAnimationsAsync, GetAnimations = ACP.GetAnimationsAsync }

------------------------------------------------------------- Animator
local ANIM = {}
-- The track for an AnimationId among the tracks this Animator has played
-- and not yet let go (a loaded track that never played isn't found).
function ANIM.GetTrackByAnimationId(self, id)
    return __rs_track_by_id(self, tostring(id))
end
function ANIM.StepAnimations() return nil end
function ANIM.ApplyJointVelocities() return nil end
function ANIM.RegisterEvaluationParallelCallback() error("RegisterEvaluationParallelCallback is not enabled!", 0) end
-- AnimationController:GetPlayingAnimationTracks (deprecated): its Animator's.
local AC = {}
function AC.GetPlayingAnimationTracks(self)
    local a = self:FindFirstChildOfClass("Animator")
    if not a then return {} end
    return __rs_lua_methods.GetPlayingAnimationTracks(a)
end

-------------------------------------------------------- AnimationTrack
local params = setmetatable({}, { __mode = "k" })
local TRACK = {}
function TRACK.GetParameter(self, key) local p = params[self]; return p and p[tostring(key)] end
function TRACK.SetParameter(self, key, value)
    local p = params[self]; if not p then p = {}; params[self] = p end
    p[tostring(key)] = value
end
function TRACK.GetParameterDefaults() return {} end
function TRACK.GetTargetNames() error("AnimationTrack:GetTargetNames is not available yet!", 0) end
function TRACK.GetTargetInstance() error("AnimationTrack:GetTargetInstance is not available yet!", 0) end
function TRACK.SetTargetInstance() error("AnimationTrack:SetTargetInstance is not available yet!", 0) end

---------------------------------------------- AnimationNodeDefinition
-- Input pins are an ordered list (duplicates allowed); every call fires
-- InputPinsChanged, even one that changed nothing.
local pins = setmetatable({}, { __mode = "k" })
local function pin_list(self) local l = pins[self]; if not l then l = {}; pins[self] = l end; return l end
local NODE = {}
function NODE.AddInputPin(self, pin)
    table.insert(pin_list(self), tostring(pin))
    fire(self, "InputPinsChanged")
end
function NODE.RemoveInputPin(self, pin)
    local l = pin_list(self)
    local i = table.find(l, tostring(pin))
    if i then table.remove(l, i) end
    fire(self, "InputPinsChanged")
end
function NODE.GetOrderedInputPinNames(self) return table.clone(pin_list(self)) end
function NODE.SetOrderedInputPinNames(self, list)
    if type(list) ~= "table" then error("Unable to cast " .. cast_name(list) .. " to Array", 0) end
    local l = {}
    for _, v in list do table.insert(l, tostring(v)) end
    pins[self] = l
    fire(self, "InputPinsChanged")
end

---------------------------------------------------------- AnimationRigData
-- LoadFromModel: false for anything but a Model, and for a Model whose
-- Humanoid has no HumanoidRootPart; true otherwise (the rig data itself
-- isn't scriptable).
local RIG = {}
function RIG.LoadFromModel(self, model)
    if not is_inst(model, "Model") then return false end
    if model:FindFirstChildOfClass("Humanoid") and not model:FindFirstChild("HumanoidRootPart") then return false end
    return true
end

-------------------------------------------------------------- Humanoid
local HUM = {}
-- The status calls are dead in Studio: they all answer false / {} and
-- fire nothing (measured on parented and parentless Humanoids).
function HUM.AddStatus() return false end
function HUM.RemoveStatus() return false end
function HUM.HasStatus() return false end
function HUM.AddCustomStatus() return false end
function HUM.RemoveCustomStatus() return false end
function HUM.HasCustomStatus() return false end
function HUM.GetStatuses() return {} end
-- A copy of the HumanoidDescription the Humanoid keeps as a child (made
-- with the class defaults on first use).
function HUM.GetAppliedDescription(self)
    local d = self:FindFirstChildOfClass("HumanoidDescription")
    if not d then
        d = create("HumanoidDescription")
        d.Parent = self
    end
    return d:Clone()
end
-- The root's velocity less the velocity of the floor it stands on.
function HUM.GetRelativeVelocityAtFloor(self)
    local root = self.RootPart
    if not root then return Vector3.zero end
    local v = root.AssemblyLinearVelocity
    if self.FloorMaterial == Enum.Material.Air then return v end
    local params = RaycastParams.new()
    params.FilterType = Enum.RaycastFilterType.Exclude
    params.FilterDescendantsInstances = { self.Parent }
    local hit = workspace:Raycast(root.Position, Vector3.new(0, -(root.Size.Y / 2 + self.HipHeight + 1.5), 0), params)
    if hit and hit.Instance then
        return v - hit.Instance:GetVelocityAtPosition(hit.Position)
    end
    return v
end

-- The joint between two parts on matching rig attachments: a Motor6D, or
-- with StarterPlayer.AvatarJointUpgrade on (Studio 0.746's default) an
-- AnimationConstraint (kinematic, MaxTorque 3000) plus a
-- BallSocketConstraint "<name>BallSocket" with Studio's limits (rig.rs
-- upgrade_spec); the shoulders' ball socket sits on "JointRotation"
-- attachments. An existing joint of that name on the part is re-seated.
local function rot_cf(r)
    return CFrame.new(0, 0, 0, r[1], r[2], r[3], r[4], r[5], r[6], r[7], r[8], r[9])
end
local function joint_on(name, p0, a0, p1, a1)
    local upgrade = __rs_anim_joint_upgrade()
    local j
    for _, c in p1:GetChildren() do
        if c.Name == name and (c:IsA("JointInstance") or c:IsA("AnimationConstraint")) then j = c; break end
    end
    if j and upgrade and j:IsA("Motor6D") then j:Destroy(); j = nil end
    if upgrade then
        if not j then
            j = create("AnimationConstraint"); j.Name = name
            j.IsKinematic = true; j.MaxForce = 0; j.MaxTorque = 3000
        end
        j.Attachment0 = a0
        j.Attachment1 = a1
        j.Parent = p1
        local spec = __rs_anim_joint_spec(name)
        if spec and not p1:FindFirstChild(name .. "BallSocket") then
            local b0, b1 = a0, a1
            if spec.Rotation0 then
                b0 = create("Attachment"); b0.Name = "JointRotation"; b0.CFrame = rot_cf(spec.Rotation0); b0.Visible = false; b0.Parent = a0
                b1 = create("Attachment"); b1.Name = "JointRotation"; b1.CFrame = rot_cf(spec.Rotation1); b1.Visible = false; b1.Parent = a1
            end
            local bs = create("BallSocketConstraint"); bs.Name = name .. "BallSocket"
            bs.Attachment0 = b0; bs.Attachment1 = b1
            bs.LimitsEnabled = true; bs.UpperAngle = spec.UpperAngle
            bs.TwistLimitsEnabled = true; bs.TwistLowerAngle = spec.TwistLowerAngle; bs.TwistUpperAngle = spec.TwistUpperAngle
            bs.MaxFrictionTorque = spec.MaxFrictionTorque; bs.Restitution = 0; bs.Radius = 0.15
            bs.Parent = p1
        end
        return j
    end
    if not j then j = create("Motor6D"); j.Name = name end
    j.Part0 = p0
    j.Part1 = p1
    j.C0 = a0.CFrame
    j.C1 = a1.CFrame
    j.Parent = p1
    return j
end
local function rig_atts(p)
    local out = {}
    for _, c in p:GetChildren() do
        if c:IsA("Attachment") and string.sub(c.Name, -13) == "RigAttachment" then out[c.Name] = c end
    end
    return out
end
-- Humanoid:BuildRigFromAttachments: outward from the HumanoidRootPart,
-- each part's *RigAttachment meets the same-named one on a part not in
-- the rig yet, and a Motor6D named after it (less "RigAttachment") joins
-- them at the two attachments, kept and re-seated when it exists.
-- (Studio 0.746 builds AnimationConstraint + BallSocketConstraint joints
-- instead when StarterPlayer.AvatarJointUpgrade is on; rustudio's rigs
-- are Motor6D throughout.)
local function build_rig(humanoid)
    local model = humanoid.Parent
    if not model then return end
    local root = model:FindFirstChild("HumanoidRootPart")
    if not (root and root:IsA("BasePart")) then return end
    local parts = {}
    for _, c in model:GetChildren() do if c:IsA("BasePart") then table.insert(parts, c) end end
    local done, queue = { [root] = true }, { root }
    while #queue > 0 do
        local p = table.remove(queue, 1)
        for name, a0 in rig_atts(p) do
            for _, q in parts do
                if not done[q] then
                    local a1 = q:FindFirstChild(name)
                    if a1 and a1:IsA("Attachment") then
                        joint_on(string.sub(name, 1, -14), p, a0, q, a1)
                        done[q] = true
                        table.insert(queue, q)
                    end
                end
            end
        end
    end
end
function HUM.BuildRigFromAttachments(self) build_rig(self) end
-- ReplaceBodyPartR15: the new part needs every rig attachment the old one
-- had (else false and nothing changes); it keeps its own name, the old
-- part is destroyed with all it held, and the joints are rebuilt on the
-- attachments. Not an R15 rig, an unknown body part or none to replace:
-- false.
function HUM.ReplaceBodyPartR15(self, bodyPart, part)
    if typeof(bodyPart) ~= "EnumItem" or tostring(bodyPart.EnumType) ~= "BodyPartR15" then
        error("Unable to cast " .. cast_name(bodyPart) .. " to token", 0)
    end
    if not is_inst(part, "BasePart") then error("Unable to cast " .. cast_name(part) .. " to Object", 0) end
    local model = self.Parent
    if self.RigType ~= Enum.HumanoidRigType.R15 or not model then return false end
    if bodyPart == Enum.BodyPartR15.Unknown or bodyPart == Enum.BodyPartR15.RootPart then return false end
    local old = model:FindFirstChild(bodyPart.Name)
    if not (old and old:IsA("BasePart")) or old == part then return false end
    local need = rig_atts(old)
    for name in need do
        local a = part:FindFirstChild(name)
        if not (a and a:IsA("Attachment")) then return false end
    end
    -- The joints that held the old part: (name, other part, which side).
    local links = {}
    for _, d in model:GetDescendants() do
        if d:IsA("BallSocketConstraint") and d.Parent ~= old then
            local a0, a1 = d.Attachment0, d.Attachment1
            if (a0 and a0:IsDescendantOf(old)) or (a1 and a1:IsDescendantOf(old)) then d:Destroy() end
        elseif (d:IsA("JointInstance") or d:IsA("AnimationConstraint")) and d.Parent and (d.Part0 == old or d.Part1 == old) then
            local other = if d.Part0 == old then d.Part1 else d.Part0
            if other then table.insert(links, { name = d.Name, other = other, old_is_part1 = d.Part1 == old }) end
            if d.Parent ~= old then d:Destroy() end
        end
    end
    part.CFrame = old.CFrame
    part.Parent = model
    old:Destroy()
    for _, l in links do
        local att = l.name .. "RigAttachment"
        local mine, theirs = part:FindFirstChild(att), l.other:FindFirstChild(att)
        if mine and theirs then
            if l.old_is_part1 then joint_on(l.name, l.other, theirs, part, mine)
            else joint_on(l.name, part, mine, l.other, theirs) end
        end
    end
    return true
end

-- PlayEmote(name): the emote named in the Humanoid's HumanoidDescription
-- (else "No emote exists with name …"), handed as its Animation (nil when
-- the asset can't be had) to the character's Animate script's PlayEmote
-- BindableFunction; true only when that returns true, false with no
-- BindableFunction, "Character has no Animate script" with no Animate.
function HUM.PlayEmoteAsync(self, name)
    name = tostring(name)
    local desc = self:FindFirstChildOfClass("HumanoidDescription")
    local ids
    if desc then
        local ok, emotes = pcall(function() return desc:GetEmotes() end)
        ids = ok and type(emotes) == "table" and emotes[name] or nil
    end
    if not ids then error("No emote exists with name " .. name, 0) end
    local animate = self.Parent and self.Parent:FindFirstChild("Animate")
    if not animate then error("Character has no Animate script", 0) end
    local bf = animate:FindFirstChild("PlayEmote")
    if not (bf and bf:IsA("BindableFunction")) then return false end
    local anim
    local id = tonumber(ids[1])
    if id then
        local ok, model = pcall(function() return game:GetService("InsertService"):LoadAsset(id) end)
        if ok and model then anim = model:FindFirstChildWhichIsA("Animation", true) end
    end
    return bf:Invoke(anim) == true
end
HUM.PlayEmote = HUM.PlayEmoteAsync
-- ApplyAvatarRulesAsync(rules): Studio's argument checks, then the
-- AvatarSettings check (a Humanoid outside the DataModel has none) and the
-- sub-rules check; a rules object with sub-rules applies. The sub-rules'
-- own settings (animation sets, body scaling, collision) aren't carried
-- out yet: with their defaults Studio changes nothing on a standard rig.
-- AvatarBodyRules (Studio 0.746, measured on a spawned R15): BuildMode
-- CustomBuild clamps the applied description's scales into the CustomBuild*
-- ranges (a warning per clamp, Studio's text) and applies it; ScaleMode
-- CustomScale scales Height / Width / Head / Depth together so the body's
-- height lands in CustomWorldHeight (5.457 studs at scale 1: 5.5 gives
-- 1.0078301, 7 gives 1.2826929).
local BASE_HEIGHT = 5.5 / 1.0078301429748535
local function fmt8(x) return (string.format("%.8g", x)) end
local function apply_body_rules(h, b)
    local d = h:GetAppliedDescription()
    local changed = false
    if b.ScaleMode == Enum.AvatarSettingsScaleMode.CustomScale then
        local range = b.CustomWorldHeight
        local height = BASE_HEIGHT * d.HeightScale
        local target = math.clamp(height, range.Min, range.Max)
        local scale = target / BASE_HEIGHT
        d.HeightScale, d.WidthScale, d.HeadScale, d.DepthScale = scale, scale, scale, scale
        changed = true
    end
    if b.BuildMode == Enum.AvatarSettingsBuildMode.CustomBuild then
        for _, f in {
            { "BodyWidthScale", "WidthScale", b.CustomBuildWidth },
            { "BodyHeightScale", "HeightScale", b.CustomBuildHeight },
            { "HeadScale", "HeadScale", b.CustomBuildHead },
            { "BodyTypeScale", "BodyTypeScale", b.CustomBuildBodyType },
            { "BodyProportionScale", "ProportionScale", b.CustomBuildProportions },
        } do
            local v = d[f[2]]
            local c = math.clamp(v, f[3].Min, f[3].Max)
            if c ~= v then
                warn(string.format("Avatar rules clamped %s from %s to %s (allowed range [%g, %g])", f[1], fmt8(v), fmt8(c), f[3].Min, f[3].Max))
                d[f[2]] = c
            end
        end
        changed = true
    end
    if changed then h:ApplyDescription(d) end
end
-- AvatarCollisionRules: SingleCollider adds an invisible massless
-- "CollisionPart" (SingleColliderSize, welded to the root 0.765 studs up)
-- and leaves the body parts non-colliding and massless; UseCollider moves
-- touch and query onto it; Default puts R15's own collision back (root,
-- UpperTorso, LowerTorso collide) and Legacy the classic one (the head too).
local function apply_collision_rules(h, b)
    local model = h.Parent
    local root = h.RootPart
    if not (model and root) then return end
    local single = b.CollisionMode == Enum.AvatarSettingsCollisionMode.SingleCollider
    local collider = model:FindFirstChild("CollisionPart")
    if single then
        if not collider then
            collider = create("Part"); collider.Name = "CollisionPart"
            collider.Transparency = 1; collider.Massless = true
            collider.CFrame = root.CFrame * CFrame.new(0, 0.76484561, 0)
            local w = create("Weld"); w.Part0 = collider; w.Part1 = root
            w.C0 = CFrame.new(); w.C1 = CFrame.new(0, 0.76484561, 0); w.Parent = collider
            collider.Parent = model
        end
        collider.Size = b.SingleColliderSize
        collider.CanCollide = true
    elseif collider then
        collider:Destroy()
        collider = nil
    end
    local use_collider = single and b.HitAndTouchDetectionMode == Enum.AvatarSettingsHitAndTouchDetectionMode.UseCollider
    if collider then collider.CanTouch = use_collider; collider.CanQuery = use_collider end
    local colliding = { HumanoidRootPart = true, UpperTorso = true, LowerTorso = true }
    if b.CollisionMode == Enum.AvatarSettingsCollisionMode.Legacy then colliding.Head = true end
    for _, p in model:GetChildren() do
        if p:IsA("BasePart") and p ~= collider then
            p.CanCollide = not single and colliding[p.Name] == true
            p.Massless = single and p ~= root
            p.CanTouch = not use_collider
            p.CanQuery = not use_collider
        end
    end
end
local SUB_RULES = { AvatarAnimationRules = true, AvatarBodyRules = true, AvatarCollisionRules = true,
    AvatarAccessoryRules = true, AvatarClothingRules = true, AvatarAbilityRules = true }
function HUM.ApplyAvatarRulesAsync(self, rules)
    if rules == nil then error("Argument 1 missing or nil", 0) end
    if typeof(rules) ~= "Instance" then error("Unable to cast value to Object", 0) end
    if not rules:IsA("AvatarRules") then error("Argument 1 is invalid: expected 'AvatarRules' instance type", 0) end
    if not self:IsDescendantOf(game) then error("AvatarSettings service not found", 0) end
    local any = false
    for _, c in rules:GetChildren() do
        if SUB_RULES[c.ClassName] then any = true end
    end
    if not any then
        error("Humanoid::ApplyAvatarRulesAsync() got invalid avatarRules: AvatarRules carries no sub-rules children, so it would apply nothing", 0)
    end
    for _, c in rules:GetChildren() do
        if c:IsA("AvatarBodyRules") then apply_body_rules(self, c)
        elseif c:IsA("AvatarCollisionRules") then apply_collision_rules(self, c) end
    end
    return nil
end

local by_class = {
    Humanoid = HUM, Animator = ANIM, AnimationController = AC, AnimationTrack = TRACK,
    AnimationNodeDefinition = NODE, AnimationRigData = RIG,
    AnimationClipProvider = ACP, KeyframeSequenceProvider = KSP,
    Keyframe = KF, KeyframeSequence = KFS, Pose = POSE,
}
for k, v in CURVES do by_class[k] = v end

-- methods(name, self, ...) → results, dispatched by the instance's class.
local function call(name, self, ...)
    local t = by_class[self.ClassName]
    local f = t and t[name]
    if not f then error(name .. " is not a valid member of " .. self.ClassName, 0) end
    return f(self, ...)
end

------------------------------------------------------------ properties
-- props(self, class, key) → handled, value.
local stored = setmetatable({}, { __mode = "k" })
local DEFAULTS = {
    RTAnimationTracker = { Active = false, EnableFallbackAudioInput = false, SessionName = "",
        TrackerMode = Enum.TrackerMode.None, TrackerType = Enum.TrackerType.None },
    AnimationStreamTrack = { FACSDataLod = Enum.FACSDataLod.LOD0, IsPlaying = false,
        Priority = Enum.AnimationPriority.Core, WeightCurrent = 0, WeightTarget = 0 },
}
local WRITABLE = {
    RTAnimationTracker = { Active = "boolean", EnableFallbackAudioInput = "boolean", SessionName = "string", TrackerType = "EnumItem" },
    AnimationStreamTrack = { Priority = "EnumItem" },
}
local function props(self, class, key)
    if class == "FloatCurve" and key == "Length" then return true, #fc_read(self) end
    if class == "MarkerCurve" and key == "Length" then return true, #mc_read(self) end
    if class == "RotationCurve" and key == "Length" then return true, #rc_list(self) end
    if class == "ValueCurve" then
        if key == "Length" then return true, 0 end
        if key == "ValueType" then return true, "nil" end
    end
    if key == "Length" and self:IsA("AnimationClip") then return true, clip_length(self) end
    local d = DEFAULTS[class]
    if d then
        if key == "Animation" and class == "AnimationStreamTrack" then return true, nil end
        if d[key] ~= nil then
            local s = stored[self]
            if s and s[key] ~= nil then return true, s[key] end
            return true, d[key]
        end
    end
    return false
end
-- writes(self, class, key, value) → handled.
local function writes(self, class, key, value)
    local w = WRITABLE[class]
    local ty = w and w[key]
    if not ty then return false end
    if typeof(value) ~= ty then
        error("Unable to assign property " .. key .. ". " .. ty .. " expected, got " .. typeof(value), 0)
    end
    local s = stored[self]; if not s then s = {}; stored[self] = s end
    s[key] = value
    return true
end

-- Yielding members go straight into __rs_lua_methods (anim.rs LUA_METHODS),
-- dispatched here by class.
local YIELDING = { "PlayEmote", "PlayEmoteAsync", "ApplyAvatarRulesAsync", "GetAnimationClipAsync",
    "GetAnimations", "GetAnimationsAsync" }
for _, name in YIELDING do
    local prev = __rs_lua_methods[name]
    __rs_lua_methods[name] = function(self, ...)
        local t = by_class[self.ClassName]
        local f = t and t[name]
        if f then return f(self, ...) end
        if prev then return prev(self, ...) end
        error(name .. " is not a valid member of " .. self.ClassName, 2)
    end
end

return call, props, writes
