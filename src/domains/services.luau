-- Engine API domain "services": the parts written in Luau (see services.rs).
-- Behaviour measured against Studio 0.744 (offline Studio) in Run/Play; notes in
-- checks/parity/services.md. Called from Rust (services.rs `call`) through
-- the table this chunk returns, and from `__rs_lua_methods` for the methods
-- that yield or throw non-string errors.
local rs = ...
local M = {}
local LM = __rs_lua_methods

local function fail(msg) error(msg, 0) end
local function svc(name) return game:GetService(name) end

------------------------------------------------------------ LogService / Logger
-- "hello {name}" with context {name = "bob"} → "hello bob"; unknown keys stay.
local function render(template, context)
	if type(context) ~= "table" then return template end
	return (string.gsub(template, "{([^{}]*)}", function(key)
		local v = context[key]
		if v == nil then return nil end
		return tostring(v)
	end))
end

local MESSAGE_TYPES = {
	[0] = "MessageOutput", [1] = "MessageInfo", [2] = "MessageWarning", [3] = "MessageError",
}
local function message_type(v)
	if typeof(v) == "EnumItem" then
		if not string.find(tostring(v), "^Enum%.MessageType%.") then fail("Unable to cast EnumItem to MessageType") end
		return v
	end
	if type(v) == "number" then
		local n = MESSAGE_TYPES[v]
		if not n then fail("Unable to cast double to MessageType") end
		return Enum.MessageType[n]
	end
	if type(v) == "string" then
		local ok, item = pcall(function() return Enum.MessageType[v] end)
		if not ok then fail(string.format('Unable to cast string "%s" to MessageType', v)) end
		return item
	end
	fail("Argument 1 missing or nil")
end

local function message_arg(v, i)
	if v == nil then fail(string.format("Argument %d missing or nil", i)) end
	return tostring(v)
end

-- The calling script's frames as Studio prints them after an error.
local function stack_text(level)
	local lines = { "Stack Begin" }
	local l = level
	while true do
		local src, line, fname = debug.info(l, "sln")
		if src == nil then break end
		if src ~= "[C]" and not string.find(src, "rustudio.", 1, true) then
			src = string.gsub(src, "^[=@]", "")
			if fname and fname ~= "" then
				table.insert(lines, string.format("Script '%s', Line %d - function %s", src, line, fname))
			else
				table.insert(lines, string.format("Script '%s', Line %d", src, line))
			end
		end
		l += 1
	end
	table.insert(lines, "Stack End")
	return table.concat(lines, "\n")
end

local LEVEL = { MessageOutput = "print", MessageInfo = "info", MessageWarning = "warn", MessageError = "error" }

-- Logger objects (LogService:GetLogger): one per full path.
local loggers = {}
local logger_path = setmetatable({}, { __mode = "k" })

local function emit(logger, mtype, message, context)
	local text = render(message, context)
	local ctx = if type(context) == "table" then context else {}
	-- A logger's MessageOut fires at once, on it and on every logger above it.
	if logger then
		local path = logger_path[logger]
		while path do
			local l = loggers[path]
			if l then rs.fire_sync(l, "MessageOut", text, mtype, ctx) end
			path = string.match(path, "^(.*)%.[^.]*$")
		end
	end
	rs.output(LEVEL[mtype.Name], text)
	return text, ctx
end

local function get_logger(parent_path, name)
	if type(name) ~= "string" then fail("Argument 1 missing or nil") end
	if name == "" then fail("Logger name cannot be empty") end
	if string.find(name, ".", 1, true) then fail("Logger name cannot contain '.'") end
	local path = if parent_path then parent_path .. "." .. name else name
	local l = loggers[path]
	if not l then
		l = rs.create_logger(name, path)
		loggers[path] = l
		logger_path[l] = path
	end
	return l
end

local function log_call(self, kind, a, b, c)
	local logger = if self.ClassName == "Logger" then self else nil
	if kind == "Log" then
		local mtype = message_type(a)
		emit(logger, mtype, message_arg(b, 2), c)
		return
	end
	local mtype = Enum.MessageType[({ Info = "MessageInfo", Warn = "MessageWarning", Output = "MessageOutput", Error = "MessageError" })[kind]]
	local text, ctx = emit(logger, mtype, message_arg(a, 1), b)
	if kind == "Error" then
		-- Studio raises a table: {message, stack, template, context}, printed as its message.
		local err = { message = text, stack = stack_text(3), template = a, context = ctx }
		error(setmetatable(err, { __tostring = function(e) return e.message end }), 0)
	end
end

function M.log(self, kind, ...)
	return log_call(self, kind, ...)
end
function M.GetLogger(self, name)
	local parent = if self.ClassName == "Logger" then logger_path[self] else nil
	return get_logger(parent, name)
end

-- LogService:Error / Logger:Error throw a table, so they can't go through
-- the Rust call path (which turns errors into strings).
do
	local prev = LM.Error
	LM.Error = function(self, ...)
		if (typeof(self) == "Instance" or typeof(self) == "Object") and (self.ClassName == "LogService" or self.ClassName == "Logger") then
			return log_call(self, "Error", ...)
		end
		if prev then return prev(self, ...) end
		fail("Error is not a valid member")
	end
end
function LM.GetLogHistoryAsync(self, user)
	if user == nil then fail("Argument 1 missing or nil") end
	-- The User object belongs to the experience's own log upload backend,
	-- which a Play test doesn't have: the session's own history.
	task.wait()
	return self:GetLogHistory()
end

--------------------------------------------------------------- Instance members
local defaults = {}
local function default_instance(class)
	local d = defaults[class]
	if d == nil then
		local ok, inst = pcall(Instance.new, class)
		d = ok and inst or false
		defaults[class] = d
	end
	return d or nil
end

local function same(a, b)
	if a == b then return true end
	-- NaN-free numbers compare by ==; Enum / datatypes by their own __eq.
	return false
end

-- (exists, read_only) for a property of the class, from Studio's API dump.
local function prop_check(self, fname, prop)
	if type(prop) ~= "string" then fail("Argument 1 missing or nil") end
	local kind, ro = rs.member(self.ClassName, prop)
	if kind ~= "Property" then
		fail(string.format("Invalid argument to %s(): Class %s has no property named `%s`", fname, self.ClassName, prop))
	end
	return ro
end

function M.IsPropertyModified(self, prop)
	prop_check(self, "IsPropertyModified", prop)
	local d = default_instance(self.ClassName)
	if not d then return false end
	if prop == "Name" then return self.Name ~= d.Name end
	if prop == "Parent" then return self.Parent ~= nil end
	local ok1, cur = pcall(function() return self[prop] end)
	local ok2, def = pcall(function() return d[prop] end)
	if not (ok1 and ok2) then return false end
	return not same(cur, def)
end

function M.ResetPropertyToDefault(self, prop)
	local ro = prop_check(self, "ResetPropertyToDefault", prop)
	if ro then
		fail(string.format("Invalid argument to ResetPropertyToDefault(): %s.%s is read-only", self.ClassName, prop))
	end
	if prop == "Parent" then self.Parent = nil return end
	local d = default_instance(self.ClassName)
	if not d then return end
	local ok, def = pcall(function() return d[prop] end)
	if ok then self[prop] = def end
end

-- No StyleSheet applies to the instance: its styled value is its own.
function M.GetStyled(self, prop, selector)
	if type(prop) ~= "string" then fail("Argument 1 missing or nil") end
	if rs.member(self.ClassName, prop) ~= "Property" then
		fail(string.format("GetStyled(): No such property named `%s`", prop))
	end
	return self[prop]
end
function M.GetStyledPropertyChangedSignal(self, prop)
	if type(prop) ~= "string" then fail("Argument 1 missing or nil") end
	if rs.member(self.ClassName, prop) ~= "Property" then
		fail(prop .. " is not a valid property name.")
	end
	return self:GetPropertyChangedSignal(prop)
end

-- QueryDescendants: Studio's style selector language over descendants.
-- Compound selectors ("Part.Tag#Name[attr]"), descendant (" ") and child
-- (">") combinators, and "," lists.
local function parse_compound(s, i)
	local c = { tags = {}, attrs = {}, props = {} }
	local n = #s
	local any = false
	while i <= n do
		local ch = string.sub(s, i, i)
		if string.match(ch, "[%a_]") then
			local name, j = string.match(s, "^([%w_]+)()", i)
			c.class = name; i = j; any = true
		elseif ch == "." then
			local name, j = string.match(s, "^%.([%w_%-]+)()", i)
			if not name then return nil end
			table.insert(c.tags, name); i = j; any = true
		elseif ch == "#" then
			local name, j = string.match(s, "^#([%w_%-]+)()", i)
			if not name then return nil end
			c.name = name; i = j; any = true
		elseif ch == "[" then
			local body, j = string.match(s, "^%[([^%]]*)%]()", i)
			if not body then return nil end
			local key, op, val = string.match(body, "^%s*(%$?[%w_]+)%s*(=?)%s*(.-)%s*$")
			if not key then return nil end
			if op == "" then val = nil else
				val = string.gsub(val, '^"(.*)"$', "%1")
				val = string.gsub(val, "^'(.*)'$", "%1")
			end
			table.insert(c.attrs, { key = key, val = val }); i = j; any = true
		else
			break
		end
	end
	return any and c or nil, i
end

local function parse_selector(sel)
	local alts = {}
	for part in string.gmatch(sel .. ",", "([^,]*),") do
		local steps = {}
		local i, n = 1, #part
		local comb = " "
		local s = part
		while true do
			local ws = string.match(s, "^%s*()", i); i = ws
			if i > n then break end
			if string.sub(s, i, i) == ">" then
				comb = ">"; i += 1
				i = string.match(s, "^%s*()", i)
			end
			local c, j = parse_compound(s, i)
			if not c then return nil end
			c.comb = comb
			table.insert(steps, c)
			comb = " "
			i = j
		end
		if #steps == 0 then return nil end
		table.insert(alts, steps)
	end
	return alts
end

local function matches(inst, c)
	if c.class and not inst:IsA(c.class) then return false end
	if c.name and inst.Name ~= c.name then return false end
	for _, t in c.tags do
		if not inst:HasTag(t) then return false end
	end
	for _, a in c.attrs do
		local v
		if string.sub(a.key, 1, 1) == "$" then
			v = inst:GetAttribute(string.sub(a.key, 2))
		else
			local ok, pv = pcall(function() return inst[a.key] end)
			v = if ok and typeof(pv) ~= "RBXScriptSignal" and type(pv) ~= "function" then pv else nil
		end
		if a.val == nil then
			if v == nil or v == false then return false end
		elseif tostring(v) ~= a.val and not (typeof(v) == "EnumItem" and v.Name == a.val) then
			return false
		end
	end
	return true
end

-- Does `inst` (under `root`) match the steps, last step first?
local function match_steps(inst, steps, k, root)
	local c = steps[k]
	if not matches(inst, c) then return false end
	if k == 1 then
		if c.comb == ">" then return inst.Parent == root end
		return true
	end
	if c.comb == ">" then
		local p = inst.Parent
		return p ~= nil and p ~= root and match_steps(p, steps, k - 1, root)
	end
	local p = inst.Parent
	while p and p ~= root do
		if match_steps(p, steps, k - 1, root) then return true end
		p = p.Parent
	end
	return false
end

function M.QueryDescendants(self, selector)
	if type(selector) ~= "string" then fail("Argument 1 missing or nil") end
	local alts = parse_selector(selector)
	if not alts then fail("Expected at least one filter") end
	local out = {}
	for _, d in self:GetDescendants() do
		for _, steps in alts do
			if match_steps(d, steps, #steps, self) then
				table.insert(out, d)
				break
			end
		end
	end
	return out
end

---------------------------------------------------------------- RunService binds
-- BindToSimulation / BindToAnimation: run inside the physics step, between
-- PreSimulation and PostSimulation (animation binds first, then simulation
-- binds, each by priority, lowest first), at a fixed StepFrequency; the
-- callback gets the step's nominal period (Hz30 → 1/30).
local HZ = { [0] = 60, [1] = 30, [2] = 15, [3] = 10, [4] = 5, [5] = 1 }
local binds = { anim = {}, sim = {} }
local bind_seq = 0
local bind_hooked = false

local function run_binds(list, dt_frame)
	-- Snapshot, so a bind that unbinds itself doesn't skip the next one.
	for _, b in table.clone(list) do
		if not b.conn.Connected then
			local i = table.find(list, b)
			if i then table.remove(list, i) end
		else
			b.acc += dt_frame
			local period = 1 / b.hz
			if b.acc + 1e-6 >= period then
				b.acc = math.min(b.acc - period, period)
				task.spawn(b.fn, period)
			end
		end
	end
end

local function sort_binds(list)
	table.sort(list, function(a, b)
		if a.priority ~= b.priority then return a.priority < b.priority end
		return a.seq < b.seq
	end)
end

function M.on_sim_step(dt)
	run_binds(binds.anim, dt)
	run_binds(binds.sim, dt)
end

local function bind(self, kind, fname, fn, freq, priority)
	if type(fn) ~= "function" then
		fail(string.format("Unable to cast %s to Function", if fn == nil then "nil" elseif type(fn) == "number" then "double" else type(fn)))
	end
	local hz = 30
	if freq ~= nil then
		local v
		if typeof(freq) == "EnumItem" then
			if not string.find(tostring(freq), "^Enum%.StepFrequency%.") then fail("Unable to cast EnumItem to StepFrequency") end
			v = freq.Value
		elseif type(freq) == "number" then
			v = freq
		elseif type(freq) == "string" then
			local ok, item = pcall(function() return Enum.StepFrequency[freq] end)
			if not ok then fail(string.format('Unable to cast string "%s" to StepFrequency', freq)) end
			v = item.Value
		else
			fail("Unable to cast " .. typeof(freq) .. " to StepFrequency")
		end
		hz = HZ[v] or fail("Unable to cast double to StepFrequency")
	end
	local p = 2000
	if priority ~= nil then
		p = tonumber(priority) or fail("Unable to cast " .. typeof(priority) .. " to int")
		p = math.floor(p)
	end
	if not bind_hooked then
		bind_hooked = true
		__rs_new_signal(self, "__svc_sim"):Connect(M.on_sim_step)
		rs.hook_sim()
	end
	bind_seq += 1
	-- The returned connection is a real one (on a private signal): its
	-- Disconnect / Connected are the bind's.
	local conn = __rs_new_signal(self, "__svc_bind" .. bind_seq):Connect(fn)
	local b = { fn = fn, hz = hz, priority = p, seq = bind_seq, acc = 1 / hz, conn = conn }
	local list = binds[kind]
	table.insert(list, b)
	sort_binds(list)
	return conn
end
function M.BindToSimulation(self, fn, freq, priority) return bind(self, "sim", "BindToSimulation", fn, freq, priority) end
function M.BindToAnimation(self, fn, freq, priority) return bind(self, "anim", "BindToAnimation", fn, freq, priority) end

----------------------------------------------------------- TweenService.SmoothDamp
-- Studio's own (decompiled,  scalar /  vector /
--  CFrame): Unity's critically damped spring, except that an
-- overshoot lands on the target with zero velocity. smoothTime < 1e-4 gives
-- (target, zero); dt < 1e-5 gives (current, velocity) unchanged. All in
-- single precision.
local function f32(x) return (string.unpack("f", string.pack("f", x))) end
local function decay(omega, dt)
	local x = f32(omega * dt)
	return f32(1 / (((x * 0.235 + 0.48) * x + 1) * x + 1))
end

local function damp_scalar(cur, target, vel, smooth, max_speed, dt)
	local omega = f32(2 / smooth)
	local max_change = f32(smooth * max_speed)
	local change = math.clamp(f32(cur - target), -max_change, max_change)
	local temp = f32(f32(change * omega + vel) * dt)
	local e = decay(omega, dt)
	local out = f32((temp + change) * e + (cur - change))
	vel = f32((vel - temp * omega) * e)
	if (out - target) * (target - cur) > 0 then
		return target, 0
	end
	return out, vel
end

-- Vectors as {x, y, z} component lists (Vector2: z = 0).
local function damp_vec(cur, target, vel, smooth, max_speed, dt)
	local omega = f32(2 / smooth)
	local max_change = f32(smooth * max_speed)
	local ch = { cur[1] - target[1], cur[2] - target[2], cur[3] - target[3] }
	local len2 = ch[1] * ch[1] + ch[2] * ch[2] + ch[3] * ch[3]
	if max_change * max_change < len2 then
		local k = max_change / math.sqrt(len2)
		ch = { ch[1] * k, ch[2] * k, ch[3] * k }
	end
	local e = decay(omega, dt)
	local out, nv = {}, {}
	for i = 1, 3 do
		local temp = f32((ch[i] * omega + vel[i]) * dt)
		out[i] = f32((temp + ch[i]) * e + (cur[i] - ch[i]))
		nv[i] = f32((vel[i] - temp * omega) * e)
	end
	local over = (out[1] - target[1]) * (target[1] - cur[1]) + (target[2] - cur[2]) * (out[2] - target[2])
		+ (out[3] - target[3]) * (target[3] - cur[3])
	if over > 0 then
		return { target[1], target[2], target[3] }, { 0, 0, 0 }
	end
	return out, nv
end

-- Quaternions {x, y, z, w}.
local function quat_of(cf)
	local _, _, _, m00, m01, m02, m10, m11, m12, m20, m21, m22 = cf:GetComponents()
	local tr = m00 + m11 + m22
	local x, y, z, w
	if tr > 0 then
		local s = math.sqrt(tr + 1) * 2
		w = 0.25 * s; x = (m21 - m12) / s; y = (m02 - m20) / s; z = (m10 - m01) / s
	elseif m00 > m11 and m00 > m22 then
		local s = math.sqrt(1 + m00 - m11 - m22) * 2
		w = (m21 - m12) / s; x = 0.25 * s; y = (m01 + m10) / s; z = (m02 + m20) / s
	elseif m11 > m22 then
		local s = math.sqrt(1 + m11 - m00 - m22) * 2
		w = (m02 - m20) / s; x = (m01 + m10) / s; y = 0.25 * s; z = (m12 + m21) / s
	else
		local s = math.sqrt(1 + m22 - m00 - m11) * 2
		w = (m10 - m01) / s; x = (m02 + m20) / s; y = (m12 + m21) / s; z = 0.25 * s
	end
	return { x, y, z, w }
end
local function qmul(a, b)
	local ax, ay, az, aw = a[1], a[2], a[3], a[4]
	local bx, by, bz, bw = b[1], b[2], b[3], b[4]
	return {
		aw * bx + ax * bw + ay * bz - az * by,
		aw * by - ax * bz + ay * bw + az * bx,
		aw * bz + ax * by - ay * bx + az * bw,
		aw * bw - ax * bx - ay * by - az * bz,
	}
end
-- Rotation vector of a quaternion, the shorter way round (w < 0 flips).
local function qlog(q)
	if q[4] < 0 then q = { -q[1], -q[2], -q[3], -q[4] } end
	local s = math.sqrt(q[1] * q[1] + q[2] * q[2] + q[3] * q[3])
	if s < 1e-12 then return { 2 * q[1], 2 * q[2], 2 * q[3] } end
	local k = 2 * math.atan2(s, math.clamp(q[4], -1, 1)) / s
	return { q[1] * k, q[2] * k, q[3] * k }
end
local function qexp(v)
	local a = math.sqrt(v[1] * v[1] + v[2] * v[2] + v[3] * v[3])
	if a < 1e-12 then return { v[1] * 0.5, v[2] * 0.5, v[3] * 0.5, 1 } end
	local k = math.sin(a / 2) / a
	return { v[1] * k, v[2] * k, v[3] * k, math.cos(a / 2) }
end

local SMOOTH_TYPES = { number = true, Vector2 = true, Vector3 = true, CFrame = true }
local function smooth_unsupported(v)
	fail(string.format("Unsupported type (%s) for argument #1 in TweenService.SmoothDamp. SmoothDamp supports number, Vector2, Vector3, and CFrame.", typeof(v)))
end
local function cast(v, ty)
	if typeof(v) ~= ty then
		local from = if type(v) == "number" then "double" elseif v == nil then "nil" else typeof(v)
		fail(string.format("Unable to cast %s to %s", from, if ty == "number" then "float" else ty))
	end
	return v
end
local function v3(v) return { v.X, v.Y, v.Z } end
local function v2(v) return { v.X, v.Y, 0 } end

function M.SmoothDamp(self, cur, target, vel, smooth, max_speed, dt)
	local ty = typeof(cur)
	if not SMOOTH_TYPES[ty] then smooth_unsupported(cur) end
	cast(target, ty); cast(vel, ty)
	if type(smooth) ~= "number" then fail("Argument 4 missing or nil") end
	max_speed = if max_speed == nil then math.huge else (tonumber(max_speed) or fail("Unable to cast to float"))
	dt = if dt == nil then rs.frame_dt() else (tonumber(dt) or fail("Unable to cast to float"))
	smooth, max_speed, dt = f32(smooth), f32(max_speed), f32(dt)
	if smooth < f32(1e-4) then
		local zero = if ty == "number" then 0 elseif ty == "Vector2" then Vector2.zero elseif ty == "Vector3" then Vector3.zero else CFrame.new()
		return target, zero
	end
	if dt < f32(1e-5) then return cur, vel end
	if ty == "number" then
		return damp_scalar(f32(cur), f32(target), f32(vel), smooth, max_speed, dt)
	elseif ty == "Vector2" then
		local o, v = damp_vec(v2(cur), v2(target), v2(vel), smooth, max_speed, dt)
		return Vector2.new(o[1], o[2]), Vector2.new(v[1], v[2])
	elseif ty == "Vector3" then
		local o, v = damp_vec(v3(cur), v3(target), v3(vel), smooth, max_speed, dt)
		return Vector3.new(o[1], o[2], o[3]), Vector3.new(v[1], v[2], v[3])
	end
	-- CFrame: the position damps as a Vector3. The rotation damps the
	-- rotation vector of cur·conj(target) toward zero, the result being
	-- exp(v)·target; its angular velocity is kept in the velocity CFrame's
	-- first matrix row (R00, R01, R02), the rest of that matrix passed
	-- through untouched (so an identity velocity starts at (1, 0, 0), as in
	-- Studio).
	local vc = { vel:GetComponents() }
	local tq = quat_of(target)
	local rel = qmul(quat_of(cur), { -tq[1], -tq[2], -tq[3], tq[4] })
	local rv, av = damp_vec(qlog(rel), { 0, 0, 0 }, { vc[4], vc[5], vc[6] }, smooth, max_speed, dt)
	local q = qmul(qexp(rv), tq)
	local n = math.sqrt(q[1] * q[1] + q[2] * q[2] + q[3] * q[3] + q[4] * q[4])
	local po, pv = damp_vec(v3(cur.Position), v3(target.Position), { vc[1], vc[2], vc[3] }, smooth, max_speed, dt)
	local out = CFrame.new(po[1], po[2], po[3], q[1] / n, q[2] / n, q[3] / n, q[4] / n)
	local nvel = CFrame.new(pv[1], pv[2], pv[3], av[1], av[2], av[3], vc[7], vc[8], vc[9], vc[10], vc[11], vc[12])
	return out, nvel
end

------------------------------------------------------------ CustomEvent(Receiver)
-- A receiver follows its Source CustomEvent; values are clamped to 0..1.
local event_value = setmetatable({}, { __mode = "k" })
function M.custom_event_set(self, v)
	if type(v) ~= "number" then fail("Argument 1 missing or nil") end
	v = math.clamp(v, 0, 1)
	event_value[self] = v
	for _, r in self:GetAttachedReceivers() do
		rs.fire(r, "SourceValueChanged", v)
	end
end
function M.custom_event_value(ev)
	return if ev then (event_value[ev] or 0) else 0
end

------------------------------------------------------------ AnalyticsService
-- The calls are fire-and-forget telemetry: a Play test sends nothing, but
-- Studio still checks the player, logging (not throwing) when it isn't one.
function M.analytics_player(player)
	if typeof(player) ~= "Instance" or not player:IsA("Player") then
		rs.output("error", "AnalyticsService: instance must be a Player object.")
		return false
	end
	return true
end
function LM.GetPlayerSegmentsAsync(self, player)
	if player == nil then fail("Argument 1 missing or nil") end
	if typeof(player) ~= "Instance" or not player:IsA("Player") then fail("Unable to cast value to Object") end
	task.wait()
	-- Segments come from the published experience's analytics backend: a
	-- local Play test has none.
	return {}
end

------------------------------------------------------------ CollectionService
-- TagAdded: a tag coming into use in the DataModel for the first time
-- (services.rs `step` passes the tags touched).
local known_tags = nil
function M.tag_check(added, removed)
	local cs = game:GetService("CollectionService")
	if known_tags == nil then
		known_tags = {}
		for _, t in cs:GetAllTags() do known_tags[t] = true end
		for _, t in added do known_tags[t] = nil end
	end
	for _, t in added do
		if not known_tags[t] then
			known_tags[t] = true
			rs.fire(cs, "TagAdded", t)
		end
	end
	-- Studio keeps a tag once it has been used: removing it from its last
	-- instance doesn't fire TagRemoved (checked in a Run test).
	local _ = removed
end

------------------------------------------------------------ yielding misc
function LM.SavePlace(self, filter)
	task.wait()
	fail("Game:SavePlace placeID is not valid!")
end
function LM.RequestLimitedAsync(self, options)
	fail("RequestLimitedAsync is disabled")
end

return M
