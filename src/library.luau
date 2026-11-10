-- rustudio engine library: services and members written in Luau, following
-- the public Roblox docs. Loaded once per runtime after the prelude.
local create, fire = ...
local M = __rs_lua_methods
-- The running thread's identity (src/security.rs; the global goes after load).
local rs_identity = __rs_identity

-- Studio's micro-profiler markers (no profiler here). Their stack lives
-- for one resumption of a thread (__rs_profile: +1 / -1, false when there
-- was nothing open); an end after a yield warns as Studio does (Place B's
-- RbxCharacterSounds AtomicBinding, whose bound function waits).
pcall(function()
    function debug.profilebegin() __rs_profile(1) end
    function debug.profileend()
        if __rs_profile(-1) then return end
        local src, line = debug.info(2, "sl")
        warn(string.format("debug.profileEnd() - No active profile annotation. At: Stack Begin\nScript '%s', Line %d\nStack End",
            tostring(src), tonumber(line) or 0))
    end
end)

------------------------------------------------------------ legacy globals
ypcall = pcall
Spawn = spawn
Delay = delay
ElapsedTime = elapsedTime
function version() return "0.1.0 (rustudio)" end
function printidentity(prefix) print((prefix or "Current identity is") .. " " .. rs_identity()) end
function LoadLibrary(name)
    error("LoadLibrary is no longer available in Roblox ('" .. tostring(name) .. "' can't be loaded)", 2)
end
local function settings_table()
    local mt = {}
    mt.__index = function(t, k)
        local v = setmetatable({}, mt)
        rawset(t, k, v)
        return v
    end
    return setmetatable({}, mt)
end
local the_settings = settings_table()
local function fake_signal()
    return { Connect = function() return { Disconnect = function() end, Connected = true } end, Wait = function() return task.wait() end }
end
-- A plain Luau signal for objects that aren't Instances.
local LuaSignal = {}
LuaSignal.__index = LuaSignal
-- A userdata like the engine's signals (typeof "RBXScriptSignal").
function LuaSignal.new() return __rs_make_signal({ handlers = {} }, LuaSignal) end
function LuaSignal:Connect(fn)
    local c = { Connected = true, fn = fn }
    local list = self.handlers
    function c.Disconnect() c.Connected = false; local i = table.find(list, c); if i then table.remove(list, i) end end
    c.disconnect = c.Disconnect
    table.insert(list, c)
    return c
end
LuaSignal.connect = LuaSignal.Connect
function LuaSignal:Once(fn)
    local c
    c = self:Connect(function(...) c.Disconnect(); fn(...) end)
    return c
end
function LuaSignal:Wait()
    local th = coroutine.running()
    self:Once(function(...) task.spawn(th, ...) end)
    return coroutine.yield()
end
function LuaSignal:Fire(...)
    for _, c in table.clone(self.handlers) do
        if c.Connected then task.spawn(c.fn, ...) end
    end
end

-- UserSettings():GetService("UserGameSettings"): the player's client
-- settings with Studio's defaults (MouseSensitivity measured in an offline Studio);
-- changes fire Changed / GetPropertyChangedSignal like an Instance. The
-- engine reads RotationType (shift-lock and first person turn the
-- character with the camera).
local ugs_values = {
    MouseSensitivity = 0.52,
    ComputerCameraMovementMode = Enum.ComputerCameraMovementMode.Default,
    ComputerMovementMode = Enum.ComputerMovementMode.Default,
    TouchCameraMovementMode = Enum.TouchCameraMovementMode.Default,
    TouchMovementMode = Enum.TouchMovementMode.Default,
    ControlMode = Enum.ControlMode.MouseLockSwitch,
    RotationType = Enum.RotationType.MovementRelative,
    SavedQualityLevel = Enum.SavedQualitySetting.Automatic,
    MasterVolume = 1,
    Fullscreen = false,
    GamepadCameraSensitivity = 0.2,
    CameraYInverted = false,
    VRSmoothRotationEnabled = false,
    VignetteEnabled = true,
    HasEverUsedVR = false,
    ChatVisible = true,
    PlayerHeight = 5,
}
-- Studio's are Instances: a
-- UserSettings named "GlobalBasicSettings" holding the UserGameSettings;
-- their properties read and write `ugs_values` (src/domains/services.rs),
-- Changed fires as for any Instance. Made on first use (the library loads
-- before the runtime has a document).
local user_settings, user_game_settings
local function make_user_settings()
    -- Made again if the first one went away (destroyed or collected while
    -- unparented): UserSettings() always answers a live object.
    if user_settings ~= nil and not pcall(function() return user_settings.Name end) then
        user_settings = nil
    end
    if user_settings == nil then
        user_settings = create("UserSettings")
        user_settings.Name = "GlobalBasicSettings"
        user_game_settings = create("UserGameSettings")
        user_game_settings.Name = "UserGameSettings"
        user_game_settings.Parent = user_settings
    end
    return user_settings
end
__rs_user_game_settings = ugs_values
-- User feature flags as Studio answers them (offline Studio, UserSettings():
-- IsUserFeatureEnabled, 2026-11-01); unknown names error like Studio's.
local user_features = {
    UserAllowAbilityControls = true, UserAllowAbilityControlsBonus = true, UserAnimateRemoveEmoteChatHook = false,
    UserAnimationAbilityManagerFixed = false, UserClickToMoveSupportAgentCanClimb2 = true, UserCurrentCameraUpdate2 = true,
    UserDynamicThumbstickSafeAreaUpdate = true, UserExcludeNonCollidableForPathfinding = true, UserFixCameraFPError = true,
    UserFixOrbitalCameraAzimuth = true, UserFixStuckShiftLock = true, UserFixVRCameraGamepadReset = true,
    UserFixVRImmersionAutoRotate = false, UserPlayerConnectionMemoryLeak = true, UserPlayerScriptsSupportMicroGamepad = true,
    UserPlayerScriptsSupportTVRemoteKeycodes = true, UserPSFixCameraControllerReset = true, UserPSSinkUnknownTouchEvents = false,
    UserPSTextboxResetCameraInput = true, UserRaycastUpdateAPI2 = true, UserVRRecenterOnExternalTeleport = true,
    -- Round 19, asked by the corpus games' own PlayerModules (Place B, Place C
    -- Run, Place A); UserAllowDisableCustomAnims2 and UserPortraitPopperFix
    -- error in Studio too. UserNewPoppercam4 picks the Popper camera.
    UserAnimateScaleRun = true, UserBetterHandlingVehicleInputStates = true, UserClickToMoveFollowPathRefactor = true,
    UserFixMovementCameraStraightDown = true, UserFixZoomClampingIssues = true, UserMakeThumbstickDynamic = false,
    UserNavigationClickToMoveSkipPassedWaypoints = true, UserNewPoppercam4 = true, UserNoMoreKeyboardPan = true,
    UserNoUpdateOnLoop = true, UserPointerActionsInPlayerScripts = false, UserPoppercamLooseOpacityThreshold = true,
    UserTheMovementModeInquisition = true, UserThirdGamepadZoomStep = true, UserTouchSensitivityAdjust = true,
}
function M.IsUserFeatureEnabled(self, name)
    local v = user_features[name]
    if v == nil then error("Flag " .. tostring(name) .. " does not exist", 0) end
    return v
end
-- UserSettings():GetService / FindService (only UserSettings dispatches here).
function M.GetService(self, name)
    if name == "UserGameSettings" then make_user_settings() return user_game_settings end
    return the_settings[name]
end
M.FindService = M.GetService
function the_settings:GetService(name) return the_settings[name] end
function the_settings:GetFFlag() return false end
function the_settings:GetFVariable() return "" end
-- settings() wants the Plugin capability, which game scripts lack (an admin script
-- kicks a client whose pcall(settings) succeeds, "Tampering with Client
-- [read rt0001]"). src/security.rs replaces this with a C function that
-- checks the calling thread's identity; this one only hands it the value.
function settings() return the_settings end
function UserSettings() return make_user_settings() end
-- The Stats service (created on first use, as GetService does).
function stats() return game:GetService("Stats") end
-- Roblox's collectgarbage takes only "count" and wants its argument.
do
    local cg = collectgarbage
    function collectgarbage(opt, ...)
        if opt == nil then error("missing argument #1 to 'collectgarbage' (string expected)", 2) end
        return cg(opt, ...)
    end
end

------------------------------------------------------------------- Debris
function M.AddItem(self, item, lifetime)
    lifetime = tonumber(lifetime) or 10
    delay(lifetime, function()
        if item and item.Parent ~= nil then
            pcall(item.Destroy, item)
        end
    end)
end

------------------------------------------------------------- TweenService
local PI = math.pi
local function bounce_out(t)
    if t < 1 / 2.75 then return 7.5625 * t * t
    elseif t < 2 / 2.75 then t -= 1.5 / 2.75; return 7.5625 * t * t + 0.75
    elseif t < 2.5 / 2.75 then t -= 2.25 / 2.75; return 7.5625 * t * t + 0.9375
    else t -= 2.625 / 2.75; return 7.5625 * t * t + 0.984375 end
end
local ease_in = {
    Linear = function(t) return t end,
    Sine = function(t) return 1 - math.cos(t * PI / 2) end,
    Quad = function(t) return t * t end,
    Cubic = function(t) return t * t * t end,
    Quart = function(t) return t ^ 4 end,
    Quint = function(t) return t ^ 5 end,
    Exponential = function(t) return t == 0 and 0 or 2 ^ (10 * (t - 1)) end,
    Circular = function(t) return 1 - math.sqrt(math.max(0, 1 - t * t)) end,
    Back = function(t) local s = 1.70158; return t * t * ((s + 1) * t - s) end,
    Elastic = function(t)
        if t == 0 or t == 1 then return t end
        return -(2 ^ (10 * (t - 1))) * math.sin((t - 1.075) * (2 * PI) / 0.3)
    end,
    Bounce = function(t) return 1 - bounce_out(1 - t) end,
}
local function ease(alpha, style, direction)
    local f = ease_in[style] or ease_in.Quad
    alpha = math.clamp(alpha, 0, 1)
    if style == "Linear" then return alpha end
    if direction == "In" then return f(alpha)
    elseif direction == "InOut" then
        if alpha < 0.5 then return f(alpha * 2) / 2 end
        return 1 - f((1 - alpha) * 2) / 2
    end
    return 1 - f(1 - alpha) -- Out (the default)
end

-- TweenService runs natively (src/domains/anim_tween.rs): Create, Play /
-- Pause / Cancel, PlaybackState / Instance / TweenInfo, the frame step.
-- `ease` above stays for pose easing (its Elastic period is 0.3).
M.GetValue = function(self, alpha, style, direction) return __rs_tween_ease(alpha, style, direction) end
function M.Create(self, inst, info, goals)
    if typeof(inst) ~= "Instance" then error("TweenService:Create: Unable to cast value to Object", 2) end
    if type(goals) ~= "table" then error("TweenService:Create: property table expected", 2) end
    -- Frame events (and with them the tween step) go to RunService.
    game:GetService("RunService")
    return __rs_tween_create(inst, info, goals)
end

-- GuiObject:TweenPosition / TweenSize / TweenSizeAndPosition (Roblox docs):
-- (end, easingDirection = Out, easingStyle = Quad, time = 1, override =
-- false, callback(Enum.TweenStatus)). A running GUI tween of the object is
-- only replaced when `override` is true; returns whether it started.
local gui_tweens = setmetatable({}, { __mode = "k" })
local function gui_tween(obj, goals, dir, style, t, override, cb)
    local running = gui_tweens[obj]
    if running and not running.done then
        if not override then return false end
        running.tween:Cancel()
    end
    local function enum_of(e, v, default)
        if typeof(v) == "EnumItem" then return v end
        if type(v) == "string" then return Enum[e][v] end
        return default
    end
    local info = TweenInfo.new(tonumber(t) or 1, enum_of("EasingStyle", style, Enum.EasingStyle.Quad),
        enum_of("EasingDirection", dir, Enum.EasingDirection.Out))
    local tw = game:GetService("TweenService"):Create(obj, info, goals)
    local rec = { tween = tw, done = false }
    gui_tweens[obj] = rec
    tw.Completed:Connect(function(state)
        rec.done = true
        if type(cb) == "function" then
            task.spawn(cb, state == Enum.PlaybackState.Completed and Enum.TweenStatus.Completed or Enum.TweenStatus.Canceled)
        end
    end)
    tw:Play()
    return true
end
function M.TweenPosition(self, pos, dir, style, t, override, cb)
    return gui_tween(self, { Position = pos }, dir, style, t, override, cb)
end
function M.TweenSize(self, size, dir, style, t, override, cb)
    return gui_tween(self, { Size = size }, dir, style, t, override, cb)
end
function M.TweenSizeAndPosition(self, size, pos, dir, style, t, override, cb)
    return gui_tween(self, { Size = size, Position = pos }, dir, style, t, override, cb)
end

-- Humanoid:GetLimb(part) → Enum.Limb (R6 names), GetBodyPartR15(part) →
-- Enum.BodyPartR15, GetMoveVelocity() = MoveDirection × WalkSpeed.
local limbs = { Head = "Head", Torso = "Torso", ["Left Arm"] = "LeftArm", ["Right Arm"] = "RightArm",
    ["Left Leg"] = "LeftLeg", ["Right Leg"] = "RightLeg" }
function M.GetLimb(self, part)
    return Enum.Limb[limbs[part and part.Name or ""] or "Unknown"]
end
function M.GetBodyPartR15(self, part)
    local ok, e = pcall(function() return Enum.BodyPartR15[part.Name] end)
    return ok and e or Enum.BodyPartR15.Unknown
end
function M.GetMoveVelocity(self)
    return self.MoveDirection * self.WalkSpeed
end
-- Instance:GetActor(): the nearest Actor ancestor (or itself), else nil.
function M.GetActor(self)
    local cur = self
    while cur do
        if cur:IsA("Actor") then return cur end
        cur = cur.Parent
    end
    return nil
end

-- Actor messaging. One Luau state per side here, so a parallel handler
-- runs like a serial one (task.desynchronize / synchronize are no-ops);
-- either kind runs at the next resumption point after SendMessage.
local actor_topics = setmetatable({}, { __mode = "k" })
local function bind_message(actor, topic, fn)
    if type(topic) ~= "string" then error("Unable to cast value to std::string", 3) end
    if type(fn) ~= "function" then error("Attempt to bind a non-function to message '" .. topic .. "'", 3) end
    local topics = actor_topics[actor]
    if not topics then topics = {}; actor_topics[actor] = topics end
    local list = topics[topic]
    if not list then list = {}; topics[topic] = list end
    local c = { Connected = true, fn = fn }
    function c.Disconnect() c.Connected = false; local i = table.find(list, c); if i then table.remove(list, i) end end
    c.disconnect = c.Disconnect
    table.insert(list, c)
    return c
end
function M.BindToMessage(self, topic, fn) return bind_message(self, topic, fn) end
function M.BindToMessageParallel(self, topic, fn) return bind_message(self, topic, fn) end
function M.SendMessage(self, topic, ...)
    if type(topic) ~= "string" then error("Unable to cast value to std::string", 2) end
    local actor, args = self, table.pack(...)
    task.defer(function()
        local list = actor_topics[actor] and actor_topics[actor][topic]
        if not list then return end
        for _, c in table.clone(list) do
            if c.Connected then task.spawn(c.fn, table.unpack(args, 1, args.n)) end
        end
    end)
end
-- (Named: Studio's read "synchronize"/"desynchronize" in debug.info.)
if not task.synchronize then function task.synchronize() end end
if not task.desynchronize then function task.desynchronize() end end

------------------------------------------------------------ AnimationTrack
-- Tracks play KeyframeSequences (registered with KeyframeSequenceProvider,
-- or an animation asset: the local asset store, the cache, else public asset
-- delivery in the background) and write each joint's Motor6D.Transform every
-- frame before physics, as Roblox's Animator does. Priorities layer from
-- Core up: each level's tracks blend by weight (normalised when they add up
-- past 1) and cover the levels below by their total weight (up to all of
-- it). Play / Stop / AdjustWeight fade over fadeTime (0.1 s default). A
-- track whose asset is still downloading has Length 0 and starts once it
-- arrives; one that can't be loaded stays at Length 0 and moves nothing.
local registered, reg_seq = {}, 0 -- "rbxtemp://kfs/n" → KeyframeSequence
local sequences = {} -- AnimationId → parsed sequence, or false

function M.RegisterKeyframeSequence(self, kfs)
    if typeof(kfs) ~= "Instance" or not kfs:IsA("KeyframeSequence") then
        error("RegisterKeyframeSequence: KeyframeSequence expected", 2)
    end
    reg_seq += 1
    local id = "rbxtemp://kfs/" .. reg_seq
    registered[id] = kfs
    return id
end
function M.RegisterAnimationClip(self, clip)
    if typeof(clip) ~= "Instance" or not clip:IsA("AnimationClip") then
        error("RegisterAnimationClip: AnimationClip expected", 2)
    end
    reg_seq += 1
    local id = "rbxtemp://clip/" .. reg_seq
    registered[id] = clip
    return id
end
M.RegisterActiveKeyframeSequence = M.RegisterKeyframeSequence
-- The anim domain (src/domains/anim_lib.luau) registers and looks up ids too.
function __rs_anim_register(id, clip) registered[id] = clip; sequences[id] = nil; __rs_anim_forget(id) end
function __rs_anim_registered(id) return registered[id] end

-- FloatCurve's version-2 wire records: u16 mode, f32 value/left/right;
-- a version-1 time array follows, in 1/2400 second ticks (Studio 0.746).
local function curve_keys(curve)
    local ok, data = pcall(function() return curve.ValuesAndTimes end)
    if not ok or type(data) ~= "string" or #data < 16 then return {} end
    local b = buffer.fromstring(data)
    local n = buffer.readu32(b, 4)
    if buffer.readu32(b, 0) ~= 2 or n > (#data - 16) / 18 then return {} end
    local times = 8 + n * 14
    if buffer.readu32(b, times) ~= 1 or buffer.readu32(b, times + 4) ~= n then return {} end
    local keys = {}
    for i = 0, n - 1 do
        local off = 8 + i * 14
        table.insert(keys, {t = buffer.readi32(b, times + 8 + i * 4) / 2400,
            mode = bit32.band(buffer.readu16(b, off), 0xff), v = buffer.readf32(b, off + 2),
            left = buffer.readf32(b, off + 6), right = buffer.readf32(b, off + 10)})
    end
    return keys
end
local function curve_value(keys, t)
    if #keys == 0 then return 0 end
    if t <= keys[1].t then return keys[1].v end
    for i = 1, #keys - 1 do
        local a, b = keys[i], keys[i + 1]
        if t < b.t then
            if a.mode == 0 then return a.v end
            local dt = b.t - a.t
            local u = (t - a.t) / dt
            if a.mode == 1 then return a.v + (b.v - a.v) * u end
            local u2, u3 = u * u, u * u * u
            return (2*u3-3*u2+1)*a.v + (u3-2*u2+u)*dt*a.right
                + (-2*u3+3*u2)*b.v + (u3-u2)*dt*b.left
        end
    end
    return keys[#keys].v
end
local function marker_frames(curve)
    local ok, data = pcall(function() return curve.ValuesAndTimes end)
    if not ok or type(data) ~= "string" or #data < 16 then return {} end
    local b, values, offset = buffer.fromstring(data), {}, 9
    if buffer.readu32(b, 0) ~= 2 then return {} end
    local n = buffer.readu32(b, 4)
    if n > #data - 16 then return {} end
    for _ = 1, n do
        local finish = string.find(data, "\0", offset, true)
        if not finish then return {} end
        table.insert(values, string.sub(data, offset, finish - 1))
        offset = finish + 1
    end
    local timeoff = offset - 1
    if timeoff + 8 + n * 4 > #data or buffer.readu32(b, timeoff) ~= 1
        or buffer.readu32(b, timeoff + 4) ~= n then return {} end
    local frames = {}
    for i, value in values do
        table.insert(frames, {t = buffer.readi32(b, timeoff + 8 + (i-1)*4) / 2400,
            name = "Keyframe", markers = {{curve.Name, value}}})
    end
    return frames
end
local function parse_curve(clip)
    local joints, frames, length = {}, {}, 0
    for _, folder in clip:GetDescendants() do
        if folder:IsA("MarkerCurve") then
            for _, frame in marker_frames(folder) do
                table.insert(frames, frame)
                length = math.max(length, frame.t)
            end
        elseif folder:IsA("Folder") then
            local channels = {}
            for _, channel in folder:GetChildren() do
                if channel:IsA("RotationCurve") then
                    -- Its quaternion keys (anim domain); length from the last.
                    local keys = __rs_anim_rotation_list and __rs_anim_rotation_list(channel)
                    if keys and #keys > 0 then
                        length = math.max(length, keys[#keys].t)
                        channels[channel.Name] = {quat = keys}
                    end
                elseif channel:IsA("Vector3Curve") or channel:IsA("EulerRotationCurve") then
                    local axes = {}
                    for _, axis in channel:GetChildren() do
                        if axis:IsA("FloatCurve") then
                            axes[axis.Name] = curve_keys(axis)
                            local keys = axes[axis.Name]
                            if #keys > 0 then length = math.max(length, keys[#keys].t) end
                        end
                    end
                    channels[channel.Name] = {axes = axes, rotation = channel:IsA("EulerRotationCurve"),
                        order = channel:IsA("EulerRotationCurve") and channel.RotationOrder or nil}
                end
            end
            if next(channels) then joints[folder.Name] = {curves = channels} end
        end
    end
    table.sort(frames, function(a,b) return a.t < b.t end)
    return {joints = joints, frames = frames, length = length, loop = clip.Loop, priority = clip.Priority}
end

local function parse_sequence(kfs)
    if kfs:IsA("CurveAnimation") then return parse_curve(kfs) end
    -- Every Pose with Weight > 0 under each Keyframe (by joint name, in tree
    -- order) and each Keyframe's markers, read natively (__rs_kfs_parts).
    local joints, frames, length = __rs_kfs_parts(kfs)
    for _, list in joints do table.sort(list, function(x, y) return x.t < y.t end) end
    table.sort(frames, function(x, y) return x.t < y.t end)
    return { joints = joints, frames = frames, length = length, loop = kfs.Loop, priority = kfs.Priority }
end

-- The sequence for an AnimationId: parsed, nil + true while its asset is
-- still downloading, or nil when it can't be had.
local function load_sequence(id)
    if sequences[id] ~= nil then return sequences[id] or nil end
    local kfs = registered[id]
    if not kfs and not string.find(id, "rbxtemp", 1, true) then
        local n = tonumber(string.match(id, "(%d+)%s*$"))
        if n then
            local state = __rs_asset_ready(n)
            if state == false then return nil, true end
            if state == true then
                -- Decoded natively (anim domain): Studio's Animator loads its
                -- clips on either side, without InsertService.
                local ok, model = pcall(__rs_anim_asset, n)
                if ok and model then
                    kfs = model:FindFirstChildWhichIsA("KeyframeSequence", true)
                        or model:FindFirstChildWhichIsA("CurveAnimation", true)
                    if model:IsA("AnimationClip") then kfs = model end
                end
            end
        end
    end
    local seq = kfs and parse_sequence(kfs) or false
    sequences[id] = seq
    return seq or nil
end

-- The track step, poses, blending and events run natively
-- (src/domains/anim_tracks.rs); it loads sequences through `__rs_anim_load`
-- and samples CurveAnimation joints through `__rs_anim_sample`.
__rs_anim_load = load_sequence
function __rs_anim_sample(list, t)
    local position, rotation = Vector3.zero, CFrame.new()
    for name, c in list.curves do
        if c.quat then
            if name == "Rotation" then rotation = __rs_anim_rotation_value(c.quat, t) or rotation end
            continue
        end
        local a = c.axes
        local x, y, z = curve_value(a.X or {}, t), curve_value(a.Y or {}, t), curve_value(a.Z or {}, t)
        if name == "Position" then position = Vector3.new(x, y, z)
        elseif name == "Rotation" and c.rotation then rotation = CFrame.fromEulerAngles(x, y, z, c.order) end
    end
    return CFrame.new(position) * rotation
end

-- The model a Humanoid / Animator / AnimationController animates.
local function rig_of(owner)
    local p = owner.Parent
    if owner:IsA("Animator") and p and (p:IsA("Humanoid") or p:IsA("AnimationController")) then p = p.Parent end
    return p
end

-- Humanoid / AnimationController:LoadAnimation go through their Animator,
-- made on first use as Roblox does.
local function animator_of(owner)
    if owner:IsA("Animator") then return owner end
    local a = owner:FindFirstChildOfClass("Animator")
    if not a then
        a = create("Animator")
        a.Parent = owner
    end
    return a
end

function M.LoadAnimation(self, anim)
    if typeof(anim) ~= "Instance" or not anim:IsA("Animation") then
        error("LoadAnimation requires an Animation object", 2)
    end
    local animator = animator_of(self)
    -- Frame events (and with them the Animator step) go to RunService.
    game:GetService("RunService")
    local track = create("AnimationTrack")
    track.Name = anim.Name
    __rs_track_new(track, anim, animator, rig_of(animator), anim.AnimationId)
    return track
end

function M.GetPlayingAnimationTracks(self)
    local owner = if self:IsA("Animator") then self else self:FindFirstChildOfClass("Animator")
    if not owner then return {} end
    return __rs_track_playing(owner)
end

-- Play / Pause / Cancel / Stop are shared by Tweens and AnimationTracks.
function M.Play(self, fade, weight, speed)
    if __rs_tween_play(self) then return end
    local ok, anim, rig = __rs_track_play(self, tonumber(fade), tonumber(weight), tonumber(speed))
    if not ok then
        warn(string.format("Failed to play animation: %s. AnimationTrack limit of 256 tracks exceeded for '%s', new animations will not be played.", anim.AnimationId, rig and rig.Name or ""))
    end
end

function M.Pause(self) __rs_tween_pause(self) end

function M.Cancel(self) __rs_tween_cancel(self) end

function M.Stop(self, fade) __rs_track_stop(self, tonumber(fade)) end

function M.AdjustSpeed(self, speed) __rs_track_speed(self, tonumber(speed)) end
function M.AdjustWeight(self, weight, fade) __rs_track_weight(self, tonumber(weight), tonumber(fade)) end
function M.GetMarkerReachedSignal(self, name)
    return __rs_new_signal(self, "MarkerReached:" .. tostring(name))
end
function M.GetTimeOfKeyframe(self, name)
    local t = __rs_track_keyframe_time(self, tostring(name))
    if t then return t end
    error("GetTimeOfKeyframe: no keyframe named " .. tostring(name), 2)
end

------------------------------------------------------------- ParticleEmitter
-- Emit(count = 16) and Clear() as Roblox's API has them: handed to the
-- viewport's particle simulation (src/effects.rs, round 25).
function M.Emit(self, count)
    __rs_particles(self, math.max(0, math.floor(tonumber(count) or 16)))
end
function M.Clear(self)
    __rs_particles(self, -1)
end

----------------------------------------------------- LocalizationService
-- (Translators and LocalizationTables: src/domains/players_chat*.)

------------------------------------------------------------ TextService
-- FilterStringAsync gives a TextFilterResult; Studio doesn't filter, so
-- each Get…Async gives the text back (an admin script filters every UI string
-- through it: "attempt to index nil with 'GetNonChatStringForUserAsync'").
local filtered = setmetatable({}, { __mode = "k" }) -- result → text
M.FilterStringAsync = function(self, text, from_user_id)
    if type(text) ~= "string" then error("Argument 1 missing or nil", 2) end
    if tonumber(from_user_id) == nil then error("Argument 2 missing or nil", 2) end
    local r = create("TextFilterResult")
    filtered[r] = text
    return r
end
local function filtered_text(self) return filtered[self] or "" end
M.GetNonChatStringForUserAsync = filtered_text
M.GetNonChatStringForBroadcastAsync = filtered_text
M.GetChatForUserAsync = filtered_text

------------------------------------------------------- MarketplaceService
-- GetProductInfo(id, infoType = Asset) yields until Roblox's catalog
-- answers (round 26): its record as a table (Name, Creator, AssetTypeId,
-- …), or an error with the HTTP status. It used to return {}, so Place B
-- Newer's music GUI set its title to nil.
local function product_info(self, id, info_type)
    local n = tonumber(id)
    if not n then
        error("Argument 1 missing or nil", 2)
    end
    local kind = typeof(info_type) == "EnumItem" and info_type.Name or "Asset"
    while true do
        local body, err = __rs_product_info(n, kind)
        if body then
            return game:GetService("HttpService"):JSONDecode(body)
        elseif body == nil then
            error(err, 0)
        end
        task.wait()
    end
end
M.GetProductInfo = product_info
M.GetProductInfoAsync = product_info

-- DataStoreService is installed by domains/data.rs.

------------------------------------------------------------ PhysicsService
-- Collision groups live in the engine (one table per Play session, shared by
-- the server, the client and the physics, as Roblox replicates them):
-- parts in groups set not to collide pass through each other, and raycasts
-- and box queries with a CollisionGroup skip those groups.
local function group_list() return __rs_cg("list") end
function M.RegisterCollisionGroup(self, name) __rs_cg("register", tostring(name)) end
M.CreateCollisionGroup = M.RegisterCollisionGroup
function M.UnregisterCollisionGroup(self, name) __rs_cg("unregister", tostring(name)) end
M.RemoveCollisionGroup = M.UnregisterCollisionGroup
function M.RenameCollisionGroup(self, from, to) __rs_cg("rename", tostring(from), tostring(to)) end
function M.CollisionGroupSetCollidable(self, a, b, on)
    __rs_cg("set", tostring(a), tostring(b), on and true or false)
end
function M.CollisionGroupsAreCollidable(self, a, b) return __rs_cg("collidable", tostring(a), tostring(b)) end
function M.IsCollisionGroupRegistered(self, name) return __rs_cg("registered", tostring(name)) end
function M.GetRegisteredCollisionGroups(self)
    local out = {}
    for i, g in group_list() do out[i] = { name = g.name, mask = g.mask } end
    return out
end
M.GetCollisionGroups = function(self)
    local out = {}
    for i, g in group_list() do out[i] = { name = g.name, id = g.id, mask = g.mask } end
    return out
end
function M.GetCollisionGroupId(self, name) return __rs_cg("id", tostring(name)) end
function M.SetPartCollisionGroup(self, part, name) part.CollisionGroup = name end
-- Roblox allows 32 collision groups (Default included).
function M.GetMaxCollisionGroups(self) return 32 end
function M.GetCollisionGroupName(self, id) return __rs_cg("name", tostring(math.floor(tonumber(id) or -1))) end
function M.CollisionGroupContainsPart(self, name, part)
    return part.CollisionGroup == name
end

------------------------------------------------------ ContextActionService
-- Bound actions see input before the rest of the game: highest priority
-- first (Enum.ContextActionPriority; the newest binding first among equal
-- ones); a handler returning nil or Sink stops it there, Pass hands it on.
-- End / Change go to the actions that took the Begin. (Roblox docs.)
local actions, action_seq = {}, 0
-- Enum.PlayerActions → keys (Roblox's default bindings; the Left / Right
-- arrows turn the camera instead, through CameraInput's own binding).
local player_action_keys = {
    CharacterForward = { "W", "Up" },
    CharacterBackward = { "S", "Down" },
    CharacterLeft = { "A" },
    CharacterRight = { "D" },
    CharacterJump = { "Space" },
}
local function action_matches(k, input)
    if typeof(k) ~= "EnumItem" then
        return type(k) == "string" and input.KeyCode.Name:lower() == k:lower()
    end
    if k == input.KeyCode or k == input.UserInputType then return true end
    local keys = player_action_keys[k.Name]
    if keys and Enum.PlayerActions[k.Name] == k then
        for _, name in keys do
            if input.KeyCode.Name == name then return true end
        end
    end
    return false
end
local function bind(name, fn, touch, priority, ...)
    action_seq += 1
    actions[name] = { fn = fn, inputs = { ... }, priority = priority, seq = action_seq, touch = touch }
end
function M.BindAction(self, name, fn, touch, ...)
    bind(name, fn, touch, Enum.ContextActionPriority.Medium.Value, ...)
end
function M.BindActionAtPriority(self, name, fn, touch, priority, ...)
    bind(name, fn, touch, tonumber(priority) or Enum.ContextActionPriority.Medium.Value, ...)
end
function M.UnbindAction(self, name) actions[name] = nil end
function M.UnbindAllActions(self) table.clear(actions) end
function M.GetBoundActionInfo(self, name)
    local a = actions[name]
    if not a then return {} end
    return { inputTypes = a.inputs, priorityLevel = a.priority, stackOrder = a.seq, createTouchButton = a.touch == true }
end
function M.GetAllBoundActionInfo(self)
    local out = {}
    for name in actions do out[name] = M.GetBoundActionInfo(self, name) end
    return out
end
function M.GetButton() return nil end
for _, n in { "SetTitle", "SetDescription", "SetImage", "SetPosition", "BindActivate", "UnbindActivate" } do
    M[n] = function() end
end

-- Runs the bound actions for one input; true when one sank it.
local took = {} -- InputObject → the actions that took its Begin
local function run_actions(input, state)
    local list
    if state == Enum.UserInputState.Begin or not took[input] then
        list = {}
        for name, a in actions do
            for _, k in a.inputs do
                if action_matches(k, input) then
                    table.insert(list, { name = name, a = a })
                    break
                end
            end
        end
        table.sort(list, function(x, y)
            if x.a.priority ~= y.a.priority then return x.a.priority > y.a.priority end
            return x.a.seq > y.a.seq
        end)
    else
        list = took[input]
    end
    local got, sunk = {}, false
    for _, e in list do
        if actions[e.name] == e.a then
            local ok, r = pcall(e.a.fn, e.name, state, input)
            if not ok then
                task.spawn(error, r, 0)
            end
            table.insert(got, e)
            if r == nil or r == Enum.ContextActionResult.Sink then
                sunk = true
                break
            end
        end
    end
    if state == Enum.UserInputState.Begin then
        took[input] = got
    elseif state == Enum.UserInputState.End or state == Enum.UserInputState.Cancel then
        took[input] = nil
    end
    return sunk
end

-------------------------------------------------------------- InputObject
-- One InputObject per key / button / device, reused for its Begin, Change
-- and End like Roblox's (CameraInput keys tables by them).
-- A userdata like Roblox's (typeof gives "InputObject"; games tell their own
-- table-based inputs apart that way).
local InputObject = {}
function InputObject:IsModifierKeyDown(m)
    local uis = game:GetService("UserInputService")
    local n = typeof(m) == "EnumItem" and m.Name or tostring(m)
    if n == "Shift" then return uis:IsKeyDown(Enum.KeyCode.LeftShift) or uis:IsKeyDown(Enum.KeyCode.RightShift) end
    if n == "Ctrl" then return uis:IsKeyDown(Enum.KeyCode.LeftControl) or uis:IsKeyDown(Enum.KeyCode.RightControl) end
    if n == "Alt" then return uis:IsKeyDown(Enum.KeyCode.LeftAlt) or uis:IsKeyDown(Enum.KeyCode.RightAlt) end
    return false
end
local input_objects = {}
-- KeyCode 0 (Unknown) for mouse input, whatever the reflection data calls it.
local keycode_unknown = nil
for _, k in Enum.KeyCode:GetEnumItems() do
    if k.Value == 0 then keycode_unknown = k break end
end
local function new_input_object(fields)
    local d = {
        KeyCode = keycode_unknown,
        UserInputType = Enum.UserInputType.None,
        UserInputState = Enum.UserInputState.None,
        Position = Vector3.zero,
        Delta = Vector3.zero,
    }
    for k, v in fields or {} do d[k] = v end
    return __rs_make_input_object(d, InputObject)
end
__rs_new_input_object = new_input_object
local mouse_delta = Vector2.zero
-- Called by the engine (in its own thread) for every input on the client:
-- the bound actions first, then UserInputService's events.
local last_input_type = "Keyboard"
function __rs_input_event(state, uit, key, x, y, z, dx, dy, gpe)
    local id = uit .. "/" .. key
    local io = input_objects[id]
    if not io then
        local ok, kc = pcall(function() return Enum.KeyCode[key] end)
        io = new_input_object({ UserInputType = Enum.UserInputType[uit], KeyCode = ok and kc or keycode_unknown })
        input_objects[id] = io
    end
    io.UserInputState = Enum.UserInputState[state]
    io.Position = Vector3.new(x, y, z)
    io.Delta = Vector3.new(dx, dy, 0)
    if uit == "MouseMovement" then mouse_delta = Vector2.new(dx, dy) end
    local uis = game:GetService("UserInputService")
    local sunk = false
    if not gpe then sunk = run_actions(io, io.UserInputState) end
    local ev = state == "Begin" and "InputBegan" or state == "End" and "InputEnded" or "InputChanged"
    -- UserInputService.LastInputTypeChanged: the kind of input the player
    -- last used changed (Roblox: keyboard / mouse movement / a button…).
    if uit ~= last_input_type then
        last_input_type = uit
        fire(uis, "LastInputTypeChanged", Enum.UserInputType[uit])
    end
    fire(uis, ev, io, gpe or false)
    if uit == "MouseWheel" then
        fire(uis, "PointerAction", z, Vector2.zero, 0, gpe or false)
    end
    return sunk
end
function __rs_input_frame_end() mouse_delta = Vector2.zero end
function M.GetLastInputType(self) return Enum.UserInputType[last_input_type] end
-- The game window gaining / losing focus (UserInputService.WindowFocused /
-- WindowFocusReleased), sent by the play session when it changes.
local window_focused = true
function __rs_window_focus(on)
    if on == window_focused then return end
    window_focused = on
    fire(game:GetService("UserInputService"), on and "WindowFocused" or "WindowFocusReleased")
end
function M.GetMouseDelta(self) return mouse_delta end
function M.GetGamepadState() return {} end
function M.GetNavigationGamepads() return {} end
function M.GetSupportedGamepadKeyCodes() return {} end
function M.IsGamepadButtonDown() return false end
function M.GamepadSupports() return false end
function M.IsNavigationGamepad() return false end
function M.GetStringForKeyCode(self, k) return typeof(k) == "EnumItem" and k.Name or "" end
function M.GetImageForKeyCode() return "" end
function M.GetDeviceRotation() return Vector3.zero, CFrame.new() end
function M.GetDeviceGravity() return nil end

------------------------------------------------- VRService / GuiService
function M.GetUserCFrame() return CFrame.new() end
function M.GetUserCFrameEnabled() return false end
function M.RecenterUserHeadCFrame() end
function M.RequestNavigation() end
function M.IsTenFootInterface() return false end
-- Studio's Play reports a 58 px top bar (offline Studio, GuiService:GetGuiInset()).
function M.GetGuiInset() return Vector2.new(0, 58), Vector2.zero end
function M.GetEmotesMenuOpen() return false end
function M.GetInspectMenuEnabled() return false end
-- PlayerScripts: the camera / movement mode lists shown in the settings menu.
for _, n in { "RegisterTouchCameraMovementMode", "RegisterComputerCameraMovementMode", "RegisterTouchMovementMode", "RegisterComputerMovementMode",
    "ClearComputerCameraMovementModes", "ClearComputerMovementModes", "ClearTouchCameraMovementModes", "ClearTouchMovementModes" } do
    M[n] = function() end
end

-------------------------------------------------------------------- Camera
-- Rays and projections from the camera's CFrame, FieldOfView (vertical) and
-- ViewportSize; "Screen" coordinates start under the 58 px top bar.
local INSET = 58
local function cam_basis(cam)
    local vs = cam.ViewportSize
    local th = math.tan(math.rad(cam.FieldOfView) / 2)
    return cam.CFrame, vs, th, vs.X / math.max(vs.Y, 1)
end
-- ProjectionType Orthographic (Studio with SupportOrthoCameraApi): half the
-- view's width and height in studs from OrthographicSize / Mode and
-- ViewStretch; rays start on the camera plane and run along LookVector.
local function ortho_half(cam, vs)
    if cam.ProjectionType ~= Enum.ProjectionType.Orthographic then return nil end
    local a = vs.X / math.max(vs.Y, 1)
    local s = cam.OrthographicSize / 2
    local m = cam.OrthographicMode
    local h = s
    if m == Enum.OrthographicMode.Horizontal or (m == Enum.OrthographicMode.MaxAxis and a >= 1) then
        h = s / a
    elseif m == Enum.OrthographicMode.Diagonal then
        h = s / math.sqrt(1 + a * a)
    end
    return h * a * cam.ViewStretch, h
end
function M.ViewportPointToRay(self, x, y, depth)
    local cf, vs, th, asp = cam_basis(self)
    local ow, oh = ortho_half(self, vs)
    if ow then
        local o = cf.Position + cf.RightVector * ((2 * x / vs.X - 1) * ow) + cf.UpVector * ((1 - 2 * y / vs.Y) * oh)
        return Ray.new(o + cf.LookVector * (tonumber(depth) or 0), cf.LookVector)
    end
    local nx = (2 * x / vs.X - 1) * th * asp
    local ny = (1 - 2 * y / vs.Y) * th
    local dir = (cf.RightVector * nx + cf.UpVector * ny + cf.LookVector).Unit
    return Ray.new(cf.Position + dir * (tonumber(depth) or 0), dir)
end
function M.ScreenPointToRay(self, x, y, depth) return M.ViewportPointToRay(self, x, y + INSET, depth) end
function M.WorldToViewportPoint(self, p)
    local cf, vs, th, asp = cam_basis(self)
    local lp = cf:PointToObjectSpace(p)
    local z = -lp.Z
    local ow, oh = ortho_half(self, vs)
    if ow then
        local x = (lp.X / ow + 1) / 2 * vs.X
        local y = (1 - lp.Y / oh) / 2 * vs.Y
        return Vector3.new(x, y, z), z > 0 and x >= 0 and x <= vs.X and y >= 0 and y <= vs.Y
    end
    local sz = math.abs(z) > 1e-6 and z or 1e-6
    local x = (lp.X / (sz * th * asp) + 1) / 2 * vs.X
    local y = (1 - lp.Y / (sz * th)) / 2 * vs.Y
    return Vector3.new(x, y, z), z > 0 and x >= 0 and x <= vs.X and y >= 0 and y <= vs.Y
end
function M.WorldToScreenPoint(self, p)
    local v, on = M.WorldToViewportPoint(self, p)
    return Vector3.new(v.X, v.Y - INSET, v.Z), on
end
function M.GetRenderCFrame(self) return self.CFrame end
function M.GetPartsObscuringTarget(self, points, ignore)
    -- Parts hit between the camera and each point (Invisicam).
    local params = RaycastParams.new()
    params.FilterType = Enum.RaycastFilterType.Exclude
    params.FilterDescendantsInstances = ignore or {}
    local out, seen = {}, {}
    local from = self.CFrame.Position
    for _, p in points do
        local origin = from
        for _ = 1, 16 do
            local r = workspace:Raycast(origin, p - origin, params)
            if not r then break end
            if not seen[r.Instance] then seen[r.Instance] = true; table.insert(out, r.Instance) end
            origin = r.Position + (p - origin).Unit * 0.01
        end
    end
    return out
end
function M.GetLargestCutoffDistance() return 0 end
function M.GetRoll() return 0 end
function M.SetRoll() end
function M.ZoomToExtents() end
function M.Interpolate(self, cf, focus) self.CFrame = cf; self.Focus = focus end

------------------------------------------------- Player:Move / Humanoid:Move
-- Humanoid:Move sets MoveDirection (at most length 1); relative to the
-- camera it turns with the camera's heading. Player:Move moves its character.
local move_warned = {} -- by UserId
function M.Move(self, dir, relative)
    if typeof(dir) ~= "Vector3" then error("Move: Vector3 expected", 2) end
    local h = self
    if self:IsA("Player") then
        local ch = self.Character
        h = ch and ch:FindFirstChildOfClass("Humanoid")
        if not h then
            -- Studio's warning (Place A's menu moves before the first spawn), once
            -- per player here: Studio printed it once in Place A's capture.
            if not ch and not move_warned[self.UserId] then
                move_warned[self.UserId] = true
                warn("Player:Move called, but player currently has no character.")
            end
            return
        end
    end
    -- Swimming, the move turns with the whole camera (look down to dive),
    -- as in Roblox; the engine keeps mostly-level moves level.
    local swimming = relative and h:GetState() == Enum.HumanoidStateType.Swimming
    if swimming and workspace.CurrentCamera then
        dir = workspace.CurrentCamera.CFrame:VectorToWorldSpace(dir)
    elseif relative then
        local cam = workspace.CurrentCamera
        if cam then
            local look = cam.CFrame.LookVector
            local flat = Vector3.new(look.X, 0, look.Z)
            if flat.Magnitude < 1e-4 then flat = Vector3.new(cam.CFrame.UpVector.X, 0, cam.CFrame.UpVector.Z) * (look.Y > 0 and -1 or 1) end
            if flat.Magnitude > 1e-4 then
                local f = flat.Unit
                local r = Vector3.new(-f.Z, 0, f.X)
                dir = r * dir.X + Vector3.yAxis * dir.Y - f * dir.Z
            end
        end
    end
    if dir.Magnitude > 1 then dir = dir.Unit end
    __rs_set_raw(h, "MoveDirection", dir)
end

-- WorldRoot:BulkMoveTo: each part to its CFrame (the mode only changes which
-- events Roblox fires; Changed fires here either way).
function M.BulkMoveTo(self, parts, cframes, _mode)
    if type(parts) ~= "table" or type(cframes) ~= "table" then
        error("BulkMoveTo: tables of parts and CFrames expected", 2)
    end
    for i, p in parts do
        local cf = cframes[i]
        if cf ~= nil then p.CFrame = cf end
    end
end

-- The Content datatype (asset references): kept as its URI string here;
-- Content properties take strings.
Content = {
    none = "",
    fromUri = function(uri) return tostring(uri) end,
    fromAssetId = function(id) return "rbxassetid://" .. tostring(id) end,
}

------------------------------------------------- SharedTableRegistry
-- SharedTable itself is src/sharedtable.rs. One Luau state per side here,
-- so a SharedTable is shared by reference across every script and Actor of
-- that side; the registry hands out the same object by name.
local shared_tables = {}
function M.GetSharedTable(self, name)
    local t = shared_tables[name]
    if not t then t = SharedTable.new(); shared_tables[name] = t end
    return t
end
function M.SetSharedTable(self, name, t) shared_tables[name] = t end

---------------------------------------------------------------- RunService
-- Bound functions run every frame in priority order (lowest first), before
-- RenderStepped's own connections; one that yields doesn't hold up the rest.
local render_steps, render_order, render_conn = {}, {}, nil
local function render_sort()
    render_order = {}
    for name, e in render_steps do table.insert(render_order, { name = name, p = e.p, n = e.n, fn = e.fn }) end
    table.sort(render_order, function(a, b) if a.p ~= b.p then return a.p < b.p end return a.n < b.n end)
end
local render_seq = 0
function M.BindToRenderStep(self, name, priority, fn)
    render_seq += 1
    local p = if typeof(priority) == "EnumItem" then priority.Value else tonumber(priority)
    render_steps[name] = { p = p or 0, n = render_seq, fn = fn }
    render_sort()
    if not render_conn then
        render_conn = self.RenderStepped:Connect(function(dt)
            for _, e in render_order do
                if render_steps[e.name] and render_steps[e.name].fn == e.fn then task.spawn(e.fn, dt) end
            end
        end)
    end
end
function M.UnbindFromRenderStep(self, name)
    render_steps[name] = nil
    render_sort()
end

------------------------------------------------------------ seats
-- What Roblox does when a Humanoid sits in a Seat / VehicleSeat (the engine
-- decides when, see play.rs update_seat): a SeatWeld from the seat to the
-- HumanoidRootPart, Seat.Occupant, Humanoid.SeatPart and Humanoid.Sit.
-- The weld as Studio builds it: C0 = (0, Seat.Size.Y × 0.5, 0)
-- and C1 = (0, y, 0), both turned by the basis look (0, -1, 0) / up (0, 0, -1)
-- (Angles(-π/2, 0, 0)). R6: y = -(max(root.Size.Y, 0.05) × 0.5 + 0.5).
-- R15: y = root RootRigAttachment.Y - LowerTorso RootRigAttachment.Y
-- + LowerTorso LeftHipRigAttachment.Y + LeftUpperLeg LeftHipRigAttachment.Z
-- - LeftUpperLeg.Size.Z × 0.5 (the thigh lies flat, so its Z is its height).
local function seat_att(part, name)
    local a = part and part:FindFirstChild(name)
    return if a and a:IsA("Attachment") then a.Position else Vector3.zero
end
function __rs_seat(humanoid, seat, root)
    local turn = CFrame.Angles(-math.pi / 2, 0, 0)
    local y
    if humanoid.RigType == Enum.HumanoidRigType.R6 then
        y = -(math.max(root.Size.Y, 0.05) * 0.5 + 0.5)
    else
        local char = humanoid.Parent
        local lower = char and char:FindFirstChild("LowerTorso")
        local leg = char and char:FindFirstChild("LeftUpperLeg")
        y = seat_att(root, "RootRigAttachment").Y
        if lower then
            y = y - seat_att(lower, "RootRigAttachment").Y + seat_att(lower, "LeftHipRigAttachment").Y
        end
        if leg then
            y = y + seat_att(leg, "LeftHipRigAttachment").Z - leg.Size.Z * 0.5
        end
    end
    local w = Instance.new("Weld")
    w.Name = "SeatWeld"
    w.Part0 = seat
    w.Part1 = root
    w.C0 = CFrame.new(0, seat.Size.Y * 0.5, 0) * turn
    w.C1 = CFrame.new(0, y, 0) * turn
    w.Parent = seat
    __rs_set_raw(seat, "Occupant", humanoid)
    __rs_set_raw(humanoid, "SeatPart", seat)
    humanoid.Sit = true
end
function __rs_unseat(humanoid, seat)
    if seat then
        for _, w in seat:GetChildren() do
            if w.Name == "SeatWeld" and w:IsA("Weld") then w:Destroy() end
        end
        if seat.Occupant == humanoid then __rs_set_raw(seat, "Occupant", nil) end
    end
    __rs_set_raw(humanoid, "SeatPart", nil)
    humanoid.Sit = false
end
