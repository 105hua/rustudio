-- rustudio engine library, players_chat domain (src/domains/players_chat.rs):
-- the members of Player / Players / Chat / TextChatService that yield, and
-- the TextChatService message flow, as measured in Studio's Play test
-- (checks/parity/players_chat.md). Loaded once per runtime.
local M, R = ...
-- R: Rust helpers (players_chat.rs `install`):
--   fire(inst, event, ...), create(class), side() -> "Server"|"Client"|"Run",
--   web(kind, a, b) -> 0, value | 1, error | 2 (pending), guid(),
--   load_character(player), kick(player, message), pages(inst, rows, size)

-- Services are looked up when used (this loads before `game` exists).
local svc = setmetatable({}, { __index = function(_, k) return game:GetService(k) end })
local add_bubble -- (part, message instance or nil, text), set further down

local function side() return R.side() end
local function on_server() return side() ~= "Client" end

-- Engine errors carry no position (pcall gets the bare text).
local function fail(msg) error(msg, 0) end

-- Polls a web lookup until it's answered, yielding between polls.
local function web(kind, a, b)
	while true do
		local state, v = R.web(kind, a, b)
		if state == 0 then return v end
		if state == 1 then return nil, v end
		task.wait()
	end
end

local function is_a(v, class)
	return typeof(v) == "Instance" and v:IsA(class)
end

local function user_id_arg(v, what)
	local n = tonumber(v)
	if n == nil then fail("Argument " .. (what or 1) .. " missing or nil") end
	return math.floor(n)
end

-------------------------------------------------------------- property store
-- TextChat objects keep their engine state here (players_chat.rs reads it
-- through __rs_pc_get / writes through __rs_pc_set).
local P = setmetatable({}, { __mode = "k" })
local function props(inst)
	local t = P[inst]
	if not t then t = {}; P[inst] = t end
	return t
end
function __rs_pc_get(inst, key)
	local t = P[inst]
	if t == nil then return false end
	local v = t[key]
	if v == nil then return false end
	return true, v
end
function __rs_pc_set(inst, key, value)
	props(inst)[key] = value
end

---------------------------------------------------------------- Player

-- Group membership through the session (groups roles list, as Studio's
-- engine asks the groups API), cached per user like Studio.
local function group_row(player, group, what)
	if not is_a(player, "Player") then fail("Player:" .. what .. " error: Player not in datamodel") end
	local gid = user_id_arg(group)
	local rows, err = web("groups", player.UserId)
	if rows == nil then fail("Player:" .. what .. " failed because " .. tostring(err)) end
	for _, r in rows do
		if r.id == gid then return r end
	end
	return nil
end
local function in_group(self, group) return group_row(self, group, "IsInGroup") ~= nil end
local function rank_in_group(self, group)
	local r = group_row(self, group, "GetRankInGroup")
	return r and r.rank or 0
end
local function role_in_group(self, group)
	local r = group_row(self, group, "GetRoleInGroup")
	return r and r.role or "Guest"
end
M.IsInGroup, M.IsInGroupAsync = in_group, in_group
M.GetRankInGroup, M.GetRankInGroupAsync = rank_in_group, rank_in_group
M.GetRoleInGroup, M.GetRoleInGroupAsync = role_in_group, role_in_group

local function friends_with(self, other)
	local uid = user_id_arg(other)
	if uid == self.UserId then return false end
	local v, err = web("friend", self.UserId, uid)
	if v == nil then fail(tostring(err)) end
	return v
end
M.IsFriendsWith, M.IsFriendsWithAsync = friends_with, friends_with
-- Roblox retired best friends: the friends lookup runs (its errors come
-- through, as in Studio) and nobody is a best friend.
M.IsBestFriendsWith = function(self, other)
	friends_with(self, other)
	return false
end

local function friends_online(self, max)
	max = tonumber(max) or 200
	if max < 0 then fail("maxFriends argument can not be negative") end
	local rows, err = web("friends_online", self.UserId)
	if rows == nil then fail(tostring(err)) end
	local out = {}
	for i, r in rows do
		if i > max then break end
		out[i] = table.clone(r)
	end
	return out
end
M.GetFriendsOnline, M.GetFriendsOnlineAsync = friends_online, friends_online

-- Friends in this server: Studio answers "Not Implemented".
M.GetFriendsInServerAsync = function(self) fail("Not Implemented") end
-- An unpublished / local place has no universe: no friend is in it or
-- played it (the lookups need the universe id).
M.GetFriendsInUniverseAsync = function(self)
	local rows = friends_online(self, 200)
	local out = {}
	local gid = game.GameId
	if gid ~= 0 then
		for _, r in rows do
			if r.GameId ~= nil and r.PlaceId == game.PlaceId then table.insert(out, r) end
		end
	end
	return out
end
M.GetFriendsWhoPlayedAsync = function(self) return {} end

-- LoadCharacter: server only; yields until the new character is added.
local function load_character(self, desc)
	if not on_server() then fail("LoadCharacter can only be called by the backend server") end
	if not is_a(self, "Player") then fail("Expected ':' not '.' calling member function LoadCharacter") end
	if self.Parent == nil then fail("Player:LoadCharacter() failed because the player is no longer in the game") end
	local done = false
	local conn
	conn = self.CharacterAdded:Connect(function(char)
		done = true
		conn:Disconnect()
		if desc then
			local hum = char:FindFirstChildOfClass("Humanoid")
			if hum then pcall(hum.ApplyDescription, hum, desc) end
		end
	end)
	R.load_character(self)
	local t0 = os.clock()
	while not done do
		task.wait()
		-- A Run (F8) session has no character pipeline.
		if side() == "Run" and os.clock() - t0 > 5 then conn:Disconnect(); return end
	end
end
M.LoadCharacter = function(self) load_character(self) end
M.LoadCharacterAsync = M.LoadCharacter
M.LoadCharacterBlocking = M.LoadCharacter
local function load_with_desc(self, desc)
	if not is_a(desc, "HumanoidDescription") then fail("Unable to cast value to Object") end
	load_character(self, desc)
end
M.LoadCharacterWithHumanoidDescription = load_with_desc
M.LoadCharacterWithHumanoidDescriptionAsync = load_with_desc
-- Avatar rules come from the experience's settings; a Studio Play test
-- uses the place's (StarterPlayer) ones, which the loader applies.
M.LoadCharacterWithAvatarRulesAsync = function(self) load_character(self) end

M.RequestStreamAroundAsync = function(self, pos, timeout)
	if typeof(pos) ~= "Vector3" then fail("Unable to cast value to Vector3") end
	if not workspace.StreamingEnabled then fail("Workspace.StreamingEnabled is false, cannot RequestStreamAroundAsync") end
	-- Everything is already there (one machine): the request completes.
	task.wait()
end

M.Kick = function(self, message)
	if not is_a(self, "Player") then fail("Expected ':' not '.' calling member function Kick") end
	if side() == "Client" and self ~= svc.Players.LocalPlayer then
		fail("Cannot kick a non-local Player from a LocalScript")
	end
	R.kick(self, message ~= nil and tostring(message) or "")
end

---------------------------------------------------------------- Players

local function lookup_id_by_name(self, name)
	if type(name) ~= "string" then fail("Argument 1 missing or nil") end
	for _, p in svc.Players:GetPlayers() do
		if p.Name:lower() == name:lower() then return p.UserId end
	end
	local v, err = web("id_by_name", name)
	if v == nil then fail("Players:GetUserIdFromNameAsync() failed: " .. tostring(err)) end
	return v
end
M.GetUserIdFromNameAsync = lookup_id_by_name
local function lookup_name_by_id(self, id)
	local uid = user_id_arg(id)
	local p = svc.Players:GetPlayerByUserId(uid)
	if p then return p.Name end
	local v, err = web("name_by_id", uid)
	-- (Studio's own message names the other function.)
	if v == nil then fail("Players:GetUserIdFromNameAsync() failed: " .. tostring(err)) end
	return v
end
M.GetNameFromUserIdAsync = lookup_name_by_id

M.GetFriendsAsync = function(self, id)
	local uid = user_id_arg(id)
	local rows, err = web("friends", uid)
	if rows == nil then fail(tostring(err)) end
	local pages = R.create("FriendPages")
	R.pages(pages, rows, 50)
	return pages
end

-- Bans: Studio validates the config, then skips the request in a test
-- session ("{}Async will succeed on production game servers…").
local function check_ban_config(cfg, unban)
	if type(cfg) ~= "table" then fail("Unable to cast value to Dictionary") end
	if type(cfg.UserIds) ~= "table" then fail("Config must contain field UserIds with type array") end
	if unban then
		if type(cfg.ApplyToUniverse) ~= "boolean" then fail("Config must contain field ApplyToUniverse with type boolean") end
	else
		if type(cfg.Duration) ~= "number" then fail("Config must contain field Duration with type number") end
		if type(cfg.DisplayReason) ~= "string" then fail("Config must contain field DisplayReason with type string") end
		if type(cfg.PrivateReason) ~= "string" then fail("Config must contain field PrivateReason with type string") end
		if cfg.Duration <= 0 and cfg.Duration ~= -1 then fail("Duration must be a positive number or -1") end
		if #cfg.DisplayReason > 400 then fail("DisplayReason exceeded the maximum length of 400") end
		if #cfg.PrivateReason > 1000 then fail("PrivateReason exceeded the maximum length of 1000") end
	end
	if #cfg.UserIds > 50 then fail("UserIds exceeded the maximum length of 50") end
	for i, id in cfg.UserIds do
		if type(id) ~= "number" then fail("Expected type number in UserIds at index " .. tostring(i)) end
		if id <= 0 then fail("Invalid UserId: " .. tostring(id) .. ". All UserIds must be positive") end
	end
end
M.BanAsync = function(self, cfg)
	if not on_server() then fail("BanAsync() can only be called on the server") end
	check_ban_config(cfg, false)
	task.wait()
	print("BanAsync will succeed on production game servers. Skipping request in test environment...")
end
M.UnbanAsync = function(self, cfg)
	if not on_server() then fail("UnbanAsync() can only be called on the server") end
	check_ban_config(cfg, true)
	task.wait()
	print("UnbanAsync will succeed on production game servers. Skipping request in test environment...")
end
M.GetBanHistoryAsync = function(self, id)
	if not on_server() then fail("GetBanHistoryAsync() can only be called on the server") end
	local uid = tonumber(id)
	if uid == nil or uid <= 0 then fail("UserId must be a positive number") end
	print("GetBanHistoryAsync will succeed on production game servers. Request will fail in test environment.")
	task.wait()
	fail("HTTP 403 (Forbidden)")
end

---------------------------------------------------------------- legacy Chat

local chat_callbacks = {}
M.RegisterChatCallback = function(self, kind, fn)
	if typeof(kind) ~= "EnumItem" or not tostring(kind):find("^Enum%.ChatCallbackType%.") then
		fail("Unable to cast value to Enum.ChatCallbackType")
	end
	if fn ~= nil and type(fn) ~= "function" then fail("Unable to cast value to function") end
	chat_callbacks[kind.Name] = fn
end
M.InvokeChatCallback = function(self, kind, ...)
	if typeof(kind) ~= "EnumItem" or not tostring(kind):find("^Enum%.ChatCallbackType%.") then
		fail("Unable to cast value to Enum.ChatCallbackType")
	end
	local fn = chat_callbacks[kind.Name]
	if fn then return fn(...) end
end

-- Studio's chat doesn't filter (Studio has no text filter); the text comes
-- back. From a LocalScript Studio warns that this is deprecated.
local warned_local_filter = false
local function warn_local_filter()
	if side() == "Client" and not warned_local_filter then
		warned_local_filter = true
		warn("Calling Chat:FilterStringAsync or Chat:FilterStringForBroadcast from LocalScripts is deprecated and will be disabled in the future. Text filtering should be done from server-side Scripts. For more info see: https://devforum.roblox.com/t/deprecating-text-filtering-from-localscripts/46403")
	end
end
-- (__rs_lua_methods is keyed by name: TextService's FilterStringAsync,
-- loaded before this, keeps answering for TextService.)
local other_filter = M.FilterStringAsync
M.FilterStringAsync = function(self, text, from, to)
	if typeof(self) == "Instance" and not self:IsA("Chat") and other_filter then
		return other_filter(self, text, from, to)
	end
	if type(text) ~= "string" then fail("Argument 1 missing or nil") end
	if not is_a(from, "Player") then fail("Argument 2 missing or nil") end
	warn_local_filter()
	task.wait()
	return text
end
M.FilterStringForPlayerAsync = M.FilterStringAsync
M.FilterStringForBroadcast = function(self, text, from)
	if type(text) ~= "string" then fail("Argument 1 missing or nil") end
	if not is_a(from, "Player") then fail("Argument 2 missing or nil") end
	warn_local_filter()
	return text
end
-- A Studio Play test's player may chat (Roblox's own settings allow the
-- signed-in account to chat in its own test session).
local function can_chat(self, a) user_id_arg(a); task.wait(); return true end
local function can_chat2(self, a, b) user_id_arg(a); user_id_arg(b, 2); task.wait(); return true end
M.CanUserChatAsync = can_chat
M.CanUsersChatAsync = can_chat2

local bubble_settings = nil
M.SetBubbleChatSettings = function(self, settings)
	if type(settings) ~= "table" then
		fail("Bad settings object passed to Chat:SetBubbleChatSettings:\ninside consolidated BubbleChat table expected, got " .. typeof(settings))
	end
	bubble_settings = settings
	-- TextChatService's bubbles take the settings, as ExpChat maps them.
	local cfg = game:GetService("TextChatService"):FindFirstChildOfClass("BubbleChatConfiguration")
	if cfg then
		for k, v in settings do
			if type(k) == "string" and k ~= "UserSpecificSettings" then pcall(function() cfg[k] = v end) end
		end
	end
end

-- Chat:Chat(part, message, color): a bubble over the part, and Chat.Chatted.
M.Chat = function(self, part, message, color)
	if typeof(part) ~= "Instance" then fail("Argument 1 missing or nil") end
	message = tostring(message or "")
	R.fire(self, "Chatted", part, message, color or Enum.ChatColor.Blue)
	if part:IsDescendantOf(workspace) then
		add_bubble(part, nil, message)
	end
end

------------------------------------------------------------ TextChatService

local STATUS = setmetatable({}, { __index = function(_, k) return Enum.TextChatMessageStatus[k] end })

local function tcs_enabled()
	local ok, v = pcall(function() return svc.TextChatService.ChatVersion end)
	return ok and v == Enum.ChatVersion.TextChatService
end

local function channel_ok(ch, what)
	if not ch:IsDescendantOf(svc.TextChatService) then
		fail(what .. " error: TextChannel not a descendant of TextChatService")
	end
end

local function source_of(ch, uid)
	for _, s in ch:GetChildren() do
		if s:IsA("TextSource") and s.UserId == uid then return s end
	end
end

local function new_message(fields)
	local m = R.create("TextChatMessage")
	local t = props(m)
	t.Text = fields.Text or ""
	t.TextChannel = fields.TextChannel
	t.TextSource = fields.TextSource
	t.Status = fields.Status or STATUS.Unknown
	t.MessageId = fields.MessageId or ""
	t.Metadata = fields.Metadata or ""
	t.PrefixText = fields.PrefixText or ""
	t.Translation = ""
	t.Timestamp = fields.Timestamp or DateTime.now()
	return m
end

-- OnIncomingMessage: the channel's, then the service's. A returned
-- TextChatMessageProperties changes the message (Studio copies its
-- PrefixText / Text / Translation onto it).
local function run_incoming(m)
	local ch = props(m).TextChannel
	for _, owner in { ch, svc.TextChatService } do
		if owner then
			local cb = __rs_get_callback(owner, "OnIncomingMessage")
			if cb then
				local ok, res = pcall(cb, m)
				if not ok then
					warn(tostring(res))
				elseif typeof(res) == "Instance" and res:IsA("TextChatMessageProperties") then
					local t = props(m)
					local function take(k)
						local v = res[k]
						if v ~= nil and v ~= "" then t[k] = v end
					end
					take("PrefixText"); take("Text"); take("Translation")
					for _, c in res:GetChildren() do
						if c:IsA("BubbleChatMessageProperties") then t.BubbleChatMessageProperties = c end
						if c:IsA("ChatWindowMessageProperties") then t.ChatWindowMessageProperties = c end
					end
				end
			end
		end
	end
end

-- The chat window's lines and the bubbles (drawn by play_game.rs, read
-- through __rs_pc_chat_view).
local view = { lines = {}, bubbles = {}, seq = 0, cleared = 0 }
local function view_line(m)
	local t = props(m)
	local src = t.TextSource
	local player = src and svc.Players:GetPlayerByUserId(src.UserId)
	local color = nil
	if player and player.Team and player.Neutral == false then color = player.TeamColor.Color end
	local wp = t.ChatWindowMessageProperties
	local cb = __rs_get_callback(svc.TextChatService, "OnChatWindowAdded")
	if cb then
		local ok, res = pcall(cb, m)
		if ok and typeof(res) == "Instance" and res:IsA("ChatWindowMessageProperties") then wp = res
		elseif not ok then warn(tostring(res)) end
	end
	view.seq += 1
	table.insert(view.lines, {
		id = view.seq,
		prefix = t.PrefixText or "",
		text = t.Text or "",
		system = src == nil,
		name_color = color,
		text_color = wp and wp.TextColor3 or nil,
		time = os.clock(),
	})
	while #view.lines > 50 do table.remove(view.lines, 1) end
end
local function view_bubble(adornee, m, text)
	local t = m and props(m) or {}
	local bp = t.BubbleChatMessageProperties
	local cb = m and __rs_get_callback(svc.TextChatService, "OnBubbleAdded")
	if cb then
		local ok, res = pcall(cb, m, adornee)
		if ok and typeof(res) == "Instance" and res:IsA("BubbleChatMessageProperties") then bp = res
		elseif not ok then warn(tostring(res)) end
	end
	view.seq += 1
	table.insert(view.bubbles, {
		id = view.seq,
		adornee = adornee,
		text = text,
		time = os.clock(),
		bg = bp and bp.BackgroundColor3 or nil,
		bg_t = bp and bp.BackgroundTransparency or nil,
		fg = bp and bp.TextColor3 or nil,
	})
end
add_bubble = view_bubble

-- The character a bubble hangs over (BubbleChatConfiguration.AdorneeName).
local function bubble_adornee(player)
	local char = player and player.Character
	if not char then return nil end
	local cfg = svc.TextChatService:FindFirstChildOfClass("BubbleChatConfiguration")
	local name = cfg and cfg.AdorneeName or "HumanoidRootPart"
	return char:FindFirstChild(name) or char:FindFirstChild("Head") or char.PrimaryPart
end

-- Commands: an enabled TextChatCommand whose alias starts the text fires
-- Triggered(textSource, text) (the message still goes out, as in Studio).
local function run_commands(text, source)
	local word = text:match("^(%S+)")
	if not word then return end
	for _, cmd in svc.TextChatService:GetDescendants() do
		if cmd:IsA("TextChatCommand") and cmd.Enabled then
			local a, b = cmd.PrimaryAlias, cmd.SecondaryAlias
			if (a ~= "" and word:lower() == a:lower()) or (b ~= "" and word:lower() == b:lower()) then
				R.fire(cmd, "Triggered", source, text)
			end
		end
	end
end

__rs_pc_outbox = {}

M.SendAsync = function(self, text, metadata)
	if side() ~= "Client" then fail("TextChannel:SendAsync can only be called on the client.") end
	channel_ok(self, "SendAsync")
	text = tostring(text or "")
	local lp = svc.Players.LocalPlayer
	local src = lp and source_of(self, lp.UserId)
	local m = new_message({
		Text = text,
		TextChannel = self,
		TextSource = src,
		Status = STATUS.Sending,
		MessageId = tostring(lp and lp.UserId or 0) .. "-" .. svc.HttpService:GenerateGUID(true),
		Metadata = metadata ~= nil and tostring(metadata) or "",
		PrefixText = lp and (lp.DisplayName .. ":") or "",
	})
	if src == nil or not src.CanSend then
		props(m).Status = STATUS.Floodchecked
		if src == nil then props(m).Status = STATUS.InvalidPrivacySettings end
		return m
	end
	-- (An empty line comes back Sending, as in Studio, and goes nowhere.)
	if text:match("^%s*$") then return m end
	run_incoming(m)
	R.fire(svc.TextChatService, "SendingMessage", m)
	run_commands(text, src)
	table.insert(__rs_pc_outbox, { channel = self, text = text, metadata = props(m).Metadata, id = props(m).MessageId })
	return m
end

M.DisplaySystemMessage = function(self, text, metadata)
	if side() ~= "Client" then fail("TextChannel:DisplaySystemMessage can only be called on the client.") end
	channel_ok(self, "DisplaySystemMessage")
	local m = new_message({
		Text = tostring(text or ""),
		TextChannel = self,
		Status = STATUS.Success,
		MessageId = "0-" .. svc.HttpService:GenerateGUID(true),
		Metadata = metadata ~= nil and tostring(metadata) or "",
	})
	run_incoming(m)
	R.fire(self, "MessageReceived", m)
	R.fire(svc.TextChatService, "MessageReceived", m)
	view_line(m)
	return m
end

M.DisplayBubble = function(self, part, text)
	if part == nil then fail("partOrCharacter must be non-nil") end
	if typeof(part) ~= "Instance" or not part:IsDescendantOf(workspace) then
		fail("partOrCharacter must be in the Workspace")
	end
	local m = new_message({ Text = tostring(text or ""), Status = STATUS.Success })
	R.fire(svc.TextChatService, "BubbleDisplayed", part, m)
	view_bubble(part, m, props(m).Text)
end

M.AddUserAsync = function(self, uid)
	if not on_server() then fail("TextChannel:AddUserAsync can only be called on the server.") end
	uid = user_id_arg(uid)
	task.wait()
	local existing = source_of(self, uid)
	if existing then return existing, false end
	local p = svc.Players:GetPlayerByUserId(uid)
	if not p then return nil, false end
	local s = R.create("TextSource")
	s.Name = p.Name
	__rs_set_raw(s, "UserId", uid)
	s.CanSend = true
	s.Parent = self
	return s, true
end

M.SetDirectChatRequester = function(self, player)
	if not is_a(player, "Player") then fail("Unable to cast value to Object") end
	for _, c in self:GetChildren() do
		if c:IsA("TextSource") then
			fail("TextChannel:SetDirectChatRequester: Cannot update direct chat requester when TextSources are already present")
		end
	end
	props(self).DirectChatRequester = player
end

M.CanUsersDirectChatAsync = function(self, requester, ids)
	user_id_arg(requester)
	if type(ids) ~= "table" then fail("Unable to cast value to std::vector<int64>") end
	task.wait()
	local out = {}
	for _, id in ids do
		if tonumber(id) and tonumber(id) ~= requester then table.insert(out, tonumber(id)) end
	end
	return out
end
-- Everyone in a Studio test may chat with everyone: one group.
M.GetChatGroupsAsync = function(self, players)
	if type(players) ~= "table" then fail("Unable to cast value to Objects") end
	task.wait()
	local group = {}
	for _, p in players do
		if is_a(p, "Player") then table.insert(group, p) end
	end
	return { group }
end

M.DeriveNewMessageProperties = function(self)
	local d = R.create("ChatWindowMessageProperties")
	for _, k in { "FontFace", "TextColor3", "TextSize", "TextStrokeColor3", "TextStrokeTransparency" } do
		pcall(function() d[k] = self[k] end)
	end
	return d
end

-- Server: a client's SendAsync arrived (play_game.rs carries it across).
-- Fires Player.Chatted and the server's commands, asks the channel's
-- ShouldDeliverCallback per TextSource, and returns the user ids it goes to.
function __rs_pc_server_receive(player, channel, text, metadata, id)
	R.fire(player, "Chatted", text)
	local src = source_of(channel, player.UserId)
	if src == nil or not src.CanSend then return {} end
	run_commands(text, src)
	local deliver = {}
	local cb = __rs_get_callback(channel, "ShouldDeliverCallback")
	local probe = cb and new_message({
		Text = text, TextChannel = channel, TextSource = src, Status = STATUS.Success,
		MessageId = id, Metadata = metadata,
	})
	for _, s in channel:GetChildren() do
		if s:IsA("TextSource") then
			local ok = true
			if cb then
				local good, res = pcall(cb, probe, s)
				ok = good and res ~= false
				if not good then warn(tostring(res)) end
			end
			if ok then table.insert(deliver, s.UserId) end
		end
	end
	return deliver
end

-- Client: the server delivered a message to this player.
function __rs_pc_client_receive(channel, sender_id, text, metadata, id)
	local src = source_of(channel, sender_id)
	local sender = svc.Players:GetPlayerByUserId(sender_id)
	local m = new_message({
		Text = text,
		TextChannel = channel,
		TextSource = src,
		Status = STATUS.Success,
		MessageId = id,
		Metadata = metadata,
		PrefixText = sender and (sender.DisplayName .. ":") or "",
	})
	run_incoming(m)
	R.fire(channel, "MessageReceived", m)
	R.fire(svc.TextChatService, "MessageReceived", m)
	view_line(m)
	local cfg = svc.TextChatService:FindFirstChildOfClass("BubbleChatConfiguration")
	if cfg and cfg.Enabled and sender then
		local a = bubble_adornee(sender)
		if a then view_bubble(a, m, props(m).Text) end
	end
	if sender then R.fire(sender, "Chatted", text) end
end

-- Default channels and commands (server), TextSources per player.
local DEFAULT_COMMANDS = {
	{ "RBXTeamCommand", "/team", "/t" },
	{ "RBXClearCommand", "/clear", "/cls" },
	{ "RBXMuteCommand", "/mute", "/m" },
	{ "RBXUnmuteCommand", "/unmute", "/um" },
	{ "RBXHelpCommand", "/help", "/?" },
	{ "RBXEmoteCommand", "/emote", "/e" },
	{ "RBXVersionCommand", "/version", "/v" },
	{ "RBXConsoleCommand", "/console", "" },
	{ "RBXWhisperCommand", "/whisper", "/w" },
}
local set_up, configs = false, false
local function server_setup()
	-- The service's configuration children (Studio always has them).
	if not configs then
		configs = true
		local T = svc.TextChatService
		for _, class in { "ChatWindowConfiguration", "ChatInputBarConfiguration", "BubbleChatConfiguration", "ChannelTabsConfiguration" } do
			if not T:FindFirstChildOfClass(class) then
				local c = R.create(class)
				c.Parent = T
			end
		end
	end
	if set_up or not tcs_enabled() then return end
	set_up = true
	if svc.TextChatService.CreateDefaultTextChannels then
		local folder = svc.TextChatService:FindFirstChild("TextChannels") or Instance.new("Folder")
		folder.Name = "TextChannels"
		for _, n in { "RBXSystem", "RBXGeneral" } do
			if not folder:FindFirstChild(n) then
				local c = Instance.new("TextChannel")
				c.Name = n
				c.Parent = folder
			end
		end
		folder.Parent = svc.TextChatService
	end
	if svc.TextChatService.CreateDefaultCommands then
		local folder = svc.TextChatService:FindFirstChild("TextChatCommands") or Instance.new("Folder")
		folder.Name = "TextChatCommands"
		for _, d in DEFAULT_COMMANDS do
			if not folder:FindFirstChild(d[1]) then
				local c = Instance.new("TextChatCommand")
				c.Name = d[1]
				c.PrimaryAlias = d[2]
				c.SecondaryAlias = d[3]
				c.Parent = folder
			end
		end
		folder.Parent = svc.TextChatService
	end
end
local sourced = setmetatable({}, { __mode = "k" })
local function server_sources()
	if not set_up or not svc.TextChatService.CreateDefaultTextChannels then return end
	local folder = svc.TextChatService:FindFirstChild("TextChannels")
	if not folder then return end
	for _, p in svc.Players:GetPlayers() do
		if not sourced[p] then
			sourced[p] = true
			for _, n in { "RBXSystem", "RBXGeneral" } do
				local ch = folder:FindFirstChild(n)
				if ch and not source_of(ch, p.UserId) then
					local s = R.create("TextSource")
					s.Name = p.Name
					__rs_set_raw(s, "UserId", p.UserId)
					s.CanSend = n == "RBXGeneral"
					s.Parent = ch
				end
			end
		end
	end
	for p in sourced do
		if p.Parent == nil then
			sourced[p] = nil
			for _, ch in folder:GetChildren() do
				local s = ch:IsA("TextChannel") and source_of(ch, p.UserId)
				if s then s:Destroy() end
			end
		end
	end
end
local client_ready = false
local function client_setup()
	if client_ready or not tcs_enabled() then return end
	local bar = svc.TextChatService:FindFirstChildOfClass("ChatInputBarConfiguration")
	local general = svc.TextChatService:FindFirstChild("TextChannels") and svc.TextChatService.TextChannels:FindFirstChild("RBXGeneral")
	if bar and general then
		client_ready = true
		if bar.TargetTextChannel == nil then bar.TargetTextChannel = general end
	end
end

-- CharacterAppearanceLoaded: once the new character's appearance is on
-- (the next frame after CharacterAdded, as Studio orders them).
local appearance = setmetatable({}, { __mode = "k" })
local function appearance_events()
	for _, p in svc.Players:GetPlayers() do
		local c = p.Character
		local st = appearance[p]
		if c ~= nil and (st == nil or st.char ~= c) then
			appearance[p] = { char = c, frames = 0 }
			-- The server's character keeps the description it was dressed
			-- from under its Humanoid (Humanoid:GetAppliedDescription copies
			-- it): the account's for CharacterAppearanceId, as in Studio.
			local hum = side() ~= "Client" and c:FindFirstChildOfClass("Humanoid")
			if hum and not hum:FindFirstChildOfClass("HumanoidDescription") then
				local ok, d = pcall(function()
					if not p.CanLoadCharacterAppearance then return R.create("HumanoidDescription") end
					return svc.Players:GetHumanoidDescriptionFromUserIdAsync(p.CharacterAppearanceId)
				end)
				if ok and d then
					d.Name = "HumanoidDescription"
					d.Parent = hum
				end
			end
		elseif st and st.frames >= 0 then
			st.frames += 1
			if st.frames >= 1 then
				st.frames = -1
				R.fire(p, "CharacterAppearanceLoaded", c)
			end
		end
	end
end

function __rs_pc_step()
	local s = side()
	if s ~= "Client" then
		server_setup()
		server_sources()
	end
	if s ~= "Server" then client_setup() end
	appearance_events()
end

-- What play_game.rs draws: the window's lines and the live bubbles.
function __rs_pc_chat_view()
	local cfg = svc.TextChatService:FindFirstChildOfClass("BubbleChatConfiguration")
	local duration = cfg and cfg.BubbleDuration or 15
	local now = os.clock()
	for i = #view.bubbles, 1, -1 do
		local b = view.bubbles[i]
		local alive = pcall(function() assert(b.adornee:IsDescendantOf(workspace)) end)
		if now - b.time > duration or not alive then
			table.remove(view.bubbles, i)
		end
	end
	view.now = now
	return view
end
function __rs_pc_clear_view()
	table.clear(view.lines)
end
-- The local player typed a line in the chat bar: SendAsync on the target
-- channel (TextChatService chat), as the default chat does.
function __rs_pc_chat_input(text)
	if not tcs_enabled() then return false end
	local bar = svc.TextChatService:FindFirstChildOfClass("ChatInputBarConfiguration")
	local ch = bar and bar.TargetTextChannel
	if ch == nil then return false end
	local word = text:match("^(%S+)")
	if word and (word:lower() == "/clear" or word:lower() == "/cls") then
		__rs_pc_clear_view()
	end
	task.spawn(function() ch:SendAsync(text) end)
	return true
end
