-- AvatarEditorService and AvatarCreationService (avatar.rs installs this).
-- Web reads go through the signed-in session to the endpoints Studio uses
-- (avatar_web.rs; checks/parity/avatar.md); each *Async call polls its
-- request and yields with task.wait, so it yields like Studio's. Replies
-- are handed to scripts with their keys capitalised (camelCase JSON →
-- PascalCase), as Studio's AvatarEditorService converts them.
--
-- Prompts: Studio fires an internal event that its AvatarEditorPrompts
-- CoreScript turns into a dialog; the dialog's buttons call back into the
-- engine, which does the account write and fires Prompt…Completed. rustudio
-- has no CoreScripts, so the dialog is built here (in the LocalPlayer's
-- PlayerGui, the GUI rustudio draws). Its confirm button never writes to the
-- account: the result is reported, nothing is saved, favourited, renamed or
-- deleted on Roblox. PromptAllowInventoryReadAccess's Allow grants read
-- access for this session, which GetInventory / GetOutfits require.

local web, poll, httperr, uid, is_client, fire, create, rules, warnf = ...
local M = __rs_lua_methods
-- Services are fetched on first use: this file loads while the runtime is
-- binding its globals, before `game` exists.
local HttpService = setmetatable({}, {
	__index = function(_, k)
		local svc = game:GetService("HttpService")
		return function(_, ...)
			return svc[k](svc, ...)
		end
	end,
})

local AVATAR = "https://avatar.roblox.com"
local CATALOG = "https://catalog.roblox.com"

local function request(method, url, body)
	local id = web(method, url, body)
	while true do
		local done, status, text = poll(id)
		if done == true then
			if status >= 200 and status < 300 then
				return text
			end
			error(httperr(status), 0)
		elseif done == nil then
			error(status, 0)
		end
		task.wait()
	end
end

local function decode(text)
	if text == nil or text == "" or text == "null" then
		return nil
	end
	return HttpService:JSONDecode(text)
end

local function get(url)
	return decode(request("GET", url))
end

local function pascal(v)
	if type(v) ~= "table" then
		return v
	end
	local out = {}
	for k, x in v do
		if type(k) == "string" then
			k = k:sub(1, 1):upper() .. k:sub(2)
		end
		out[k] = pascal(x)
	end
	return out
end

local function argcheck(v, i)
	if v == nil then
		error("Argument " .. i .. " missing or nil", 0)
	end
end

local function cast(v, i, want)
	argcheck(v, i)
	local t = typeof(v)
	if want == "int64" then
		local n = tonumber(v)
		if n == nil then
			error("Unable to cast " .. (t == "string" and "string" or t) .. " to int64", 0)
		end
		return math.floor(n)
	elseif want == "Array" or want == "Dictionary" then
		if t ~= "table" then
			error("Unable to cast " .. (t == "number" and "double" or t) .. " to " .. want, 0)
		end
		return v
	end
	return v
end

-- Whether v is an item of `enum` (compared by name: an Enum's identity isn't
-- stable across lookups in rustudio).
local function is_item(v, enum)
	return typeof(v) == "EnumItem" and tostring(v.EnumType) == tostring(enum)
end

local function enumarg(v, i, enum, default)
	if v == nil then
		if default then
			return default
		end
		error("Argument " .. i .. " missing or nil", 0)
	end
	if is_item(v, enum) then
		return v
	end
	if type(v) == "number" then
		for _, it in enum:GetEnumItems() do
			if it.Value == v then
				return it
			end
		end
	elseif type(v) == "string" then
		local ok, it = pcall(function()
			return enum[v]
		end)
		if ok and it then
			return it
		end
	end
	error("Unable to cast value to " .. tostring(enum), 0)
end

local function query(s)
	return HttpService:UrlEncode(tostring(s))
end

----------------------------------------------------------------- Pages
-- InventoryPages / OutfitPages / CatalogPages go through the shared Pages
-- (src/domains/pages.rs): fetch(cursor) → (page array, next cursor or nil),
-- the first page fetched before the Pages is returned, the next one on
-- AdvanceToNextPageAsync with the reply's cursor.
local function make_pages(class, fetch)
	local cursor
	return __rs_pages_new(class, function()
		local page, nextc = fetch(cursor)
		cursor = nextc
		return page, nextc == nil or nextc == ""
	end)
end

-- For other domains (Luau): __rs_avatar_get(url) is a yielding GET through
-- the login session that returns the decoded JSON (nil for "null"), or
-- errors "HTTP n (reason)".
__rs_avatar_get = get

---------------------------------------------------------- permissions
local read_access = false
local prompt_open = {}

local function need_access()
	if not read_access then
		error("Permission must be obtained with PromptAllowInventoryReadAccess before calling this method", 0)
	end
	if not is_client() then
		error("This method can only be called on the client", 0)
	end
end

------------------------------------------------------------- the dialog
local answer_open -- the open dialog's answer function (tests answer through __rs_avatar_prompt_answer)

local function dialog(title, body, confirm, field, done)
	local players = game:GetService("Players")
	local lp = players.LocalPlayer
	local pg = lp and lp:FindFirstChildOfClass("PlayerGui")
	local gui
	local function finish(ok, text)
		if gui then
			gui:Destroy()
			gui = nil
		end
		answer_open = nil
		done(ok, text)
	end
	answer_open = finish
	if pg == nil then
		return
	end
	gui = Instance.new("ScreenGui")
	gui.Name = "AvatarEditorPrompts"
	gui.DisplayOrder = 1000
	gui.ResetOnSpawn = false
	gui.IgnoreGuiInset = true
	local shade = Instance.new("Frame")
	shade.Size = UDim2.fromScale(1, 1)
	shade.BackgroundColor3 = Color3.new(0, 0, 0)
	shade.BackgroundTransparency = 0.5
	shade.BorderSizePixel = 0
	shade.Parent = gui
	local box = Instance.new("Frame")
	box.AnchorPoint = Vector2.new(0.5, 0.5)
	box.Position = UDim2.fromScale(0.5, 0.5)
	box.Size = UDim2.fromOffset(400, field and 240 or 200)
	box.BackgroundColor3 = Color3.fromRGB(57, 59, 61)
	box.BorderSizePixel = 0
	box.Parent = shade
	local function label(text, y, h, size, bold)
		local l = Instance.new("TextLabel")
		l.BackgroundTransparency = 1
		l.Position = UDim2.fromOffset(20, y)
		l.Size = UDim2.new(1, -40, 0, h)
		l.Font = bold and Enum.Font.GothamBold or Enum.Font.Gotham
		l.TextSize = size
		l.TextWrapped = true
		l.TextColor3 = Color3.new(1, 1, 1)
		l.Text = text
		l.Parent = box
		return l
	end
	label(title, 16, 28, 20, true)
	label(body, 52, 70, 15, false)
	local input
	if field then
		input = Instance.new("TextBox")
		input.Name = "Name"
		input.Position = UDim2.fromOffset(20, 128)
		input.Size = UDim2.new(1, -40, 0, 32)
		input.BackgroundColor3 = Color3.fromRGB(35, 37, 39)
		input.BorderSizePixel = 0
		input.TextColor3 = Color3.new(1, 1, 1)
		input.PlaceholderText = field
		input.Text = ""
		input.Font = Enum.Font.Gotham
		input.TextSize = 15
		input.Parent = box
	end
	local function button(name, text, x, primary)
		local b = Instance.new("TextButton")
		b.Name = name
		b.AnchorPoint = Vector2.new(0, 1)
		b.Position = UDim2.new(x, x == 0 and 20 or 10, 1, -16)
		b.Size = UDim2.new(0.5, -30, 0, 40)
		b.BackgroundColor3 = primary and Color3.fromRGB(0, 162, 255) or Color3.fromRGB(85, 87, 89)
		b.BorderSizePixel = 0
		b.TextColor3 = Color3.new(1, 1, 1)
		b.Font = Enum.Font.GothamBold
		b.TextSize = 16
		b.Text = text
		b.Parent = box
		return b
	end
	button("Cancel", "Cancel", 0, false).Activated:Connect(function()
		finish(false)
	end)
	button("Confirm", confirm, 0.5, true).Activated:Connect(function()
		finish(true, input and input.Text or nil)
	end)
	gui.Parent = pg
end

function __rs_avatar_prompt_answer(ok, text)
	if answer_open then
		answer_open(ok, text)
		return true
	end
	return false
end

local aes_inst
local function aes()
	aes_inst = aes_inst or game:GetService("AvatarEditorService")
	return aes_inst
end

local R = setmetatable({}, { __index = function(_, k) return Enum.AvatarPromptResult[k] end })
local function start_prompt(name)
	if not is_client() then
		error(name .. " can only be called on the client", 0)
	end
	if prompt_open[name] then
		error(name .. " can not be called again while currently in progress", 0)
	end
	prompt_open[name] = true
end
local function end_prompt(name, event, ...)
	prompt_open[name] = nil
	fire(aes(), event, ...)
end

local function clone_desc(name, d)
	if typeof(d) ~= "Instance" or not d:IsA("HumanoidDescription") then
		error("Unable to cast value to Object", 0)
	end
	local ok, c = pcall(function()
		return d:Clone()
	end)
	if not ok or c == nil then
		error(name .. ": HumanoidDescription could not be cloned. Is HumanoidDescription.Archivable set to false?", 0)
	end
	return c
end

M.PromptAllowInventoryReadAccess = function(self)
	start_prompt("PromptAllowInventoryReadAccess")
	dialog("Allow Inventory Access", "Allow " .. tostring(game.Name) .. " to see your inventory and outfits?", "Allow", nil, function(ok)
		if ok then
			read_access = true
		end
		end_prompt("PromptAllowInventoryReadAccess", "PromptAllowInventoryReadAccessCompleted", ok and R.Success or R.PermissionDenied)
	end)
end

M.PromptSaveAvatar = function(self, desc, rig)
	argcheck(desc, 1)
	local c = clone_desc("PromptSaveAvatar", desc)
	rig = enumarg(rig, 2, Enum.HumanoidRigType)
	start_prompt("PromptSaveAvatar")
	dialog("Save Avatar", "Save this " .. rig.Name .. " avatar as your Roblox avatar? (rustudio never changes your avatar.)", "Save", nil, function(ok)
		end_prompt("PromptSaveAvatar", "PromptSaveAvatarCompleted", ok and R.Success or R.PermissionDenied, ok and c or nil)
	end)
end

M.PromptCreateOutfit = function(self, desc, rig, options, outfitType)
	argcheck(desc, 1)
	local c = clone_desc("PromptCreateOutfit", desc)
	rig = enumarg(rig, 2, Enum.HumanoidRigType)
	if options ~= nil then
		cast(options, 3, "Dictionary")
		for k in options do
			error(string.format("PromptCreateOutfit: unknown key in outfitOptions: %s", tostring(k)), 0)
		end
	end
	if outfitType ~= nil then
		if not is_item(outfitType, Enum.OutfitType) then
			error("PromptCreateOutfit: outfitType must be an Enum.OutfitType or nil", 0)
		end
		if outfitType ~= Enum.OutfitType.Avatar and outfitType ~= Enum.OutfitType.Makeup then
			error("PromptCreateOutfit: outfitType is not supported. Only Avatar and Makeup are accepted.", 0)
		end
	end
	start_prompt("PromptCreateOutfit")
	dialog("Create Outfit", "Create a new outfit from this avatar? (rustudio never creates outfits.)", "Create", "Outfit name", function(ok)
		c:Destroy()
		if ok then
			end_prompt("PromptCreateOutfit", "PromptCreateOutfitCompleted", R.Success, nil)
		else
			end_prompt("PromptCreateOutfit", "PromptCreateOutfitCompleted", R.PermissionDenied, nil)
		end
	end)
end

local function outfit_prompt(name, event, title, body, confirm, field)
	return function(self, outfitId, desc, rig)
		outfitId = cast(outfitId, 1, "int64")
		if name == "PromptUpdateOutfit" then
			argcheck(desc, 2)
			clone_desc(name, desc):Destroy()
			enumarg(rig, 3, Enum.HumanoidRigType)
		end
		start_prompt(name)
		dialog(title, body, confirm, field, function(ok)
			end_prompt(name, event, ok and R.Success or R.PermissionDenied)
		end)
	end
end
M.PromptDeleteOutfit = outfit_prompt("PromptDeleteOutfit", "PromptDeleteOutfitCompleted", "Delete Outfit", "Delete this outfit? (rustudio never deletes outfits.)", "Delete")
M.PromptRenameOutfit = outfit_prompt("PromptRenameOutfit", "PromptRenameOutfitCompleted", "Rename Outfit", "Rename this outfit? (rustudio never renames outfits.)", "Rename", "New name")
M.PromptUpdateOutfit = outfit_prompt("PromptUpdateOutfit", "PromptUpdateOutfitCompleted", "Update Outfit", "Update this outfit to the current avatar? (rustudio never changes outfits.)", "Update")

M.PromptSetFavorite = function(self, itemId, itemType, shouldFavorite)
	itemId = cast(itemId, 1, "int64")
	itemType = enumarg(itemType, 2, Enum.AvatarItemType)
	argcheck(shouldFavorite, 3)
	start_prompt("PromptSetFavorite")
	local verb = shouldFavorite and "Favorite" or "Unfavorite"
	dialog(verb .. " Item", verb .. " this " .. itemType.Name:lower() .. "? (rustudio never changes your favorites.)", verb, nil, function(ok)
		end_prompt("PromptSetFavorite", "PromptSetFavoriteCompleted", ok and R.Success or R.PermissionDenied)
	end)
end

------------------------------------------------------------------ reads
local function assettype_name(id)
	for _, it in Enum.AvatarAssetType:GetEnumItems() do
		if it.Value == id then
			return it.Name
		end
	end
	return tostring(id)
end

M.GetInventoryAsync = function(self, assetTypes)
	cast(assetTypes, 1, "Array")
	local cats = {}
	for i, t in assetTypes do
		if not is_item(t, Enum.AvatarAssetType) then
			error("Unexpected value in assetTypes, must be a memeber of Enum.AvatarAssetType", 0)
		end
		local k = i - 1
		table.insert(cats, string.format("&itemCategories[%d].itemType=1&itemCategories[%d].itemSubType=%d", k, k, t.Value))
	end
	need_access()
	local base = AVATAR .. "/v1/avatar-inventory?pageLimit=25&sortOption=1&availabilityStatus=0" .. table.concat(cats)
	return make_pages("InventoryPages", function(cursor)
		local url = base .. (cursor and ("&pageToken=" .. query(cursor)) or "")
		local r = get(url) or {}
		local page = {}
		for _, it in r.avatarInventoryItems or {} do
			local cat = it.itemCategory or {}
			table.insert(page, {
				AssetId = it.itemId,
				AssetType = assettype_name(cat.itemSubType),
				Name = it.itemName,
				Created = it.acquisitionTime,
			})
		end
		return page, r.nextPageToken
	end)
end
M.GetInventory = M.GetInventoryAsync

M.GetOutfitsAsync = function(self, source, outfitType)
	source = enumarg(source, 1, Enum.OutfitSource, Enum.OutfitSource.All)
	outfitType = enumarg(outfitType, 2, Enum.OutfitType, Enum.OutfitType.All)
	need_access()
	local editable = source == Enum.OutfitSource.Created and "true" or source == Enum.OutfitSource.Purchased and "false" or ""
	local base = string.format("%s/v2/avatar/users/%d/outfits?itemsPerPage=%d&isEditable=%s", AVATAR, uid(), 50, editable)
	if outfitType ~= Enum.OutfitType.All then
		base ..= "&outfitType=" .. outfitType.Name
	end
	return make_pages("OutfitPages", function(cursor)
		local r = get(base .. (cursor and ("&paginationToken=" .. query(cursor)) or "")) or {}
		return pascal(r.data or {}), r.paginationToken
	end)
end
M.GetOutfits = M.GetOutfitsAsync

local function hexcolor(s)
	if type(s) ~= "string" then
		error("AvatarEditorService hex color was not a string when converting to Color3", 0)
	end
	return Color3.fromHex(s)
end

M.GetOutfitDetailsAsync = function(self, outfitId)
	outfitId = cast(outfitId, 1, "int64")
	local r = get(string.format("%s/v3/outfits/%d/details", AVATAR, outfitId))
	if type(r) ~= "table" then
		error("GetOutfitDetails response missing or invalid outfitModel", 0)
	end
	local out = pascal(r)
	out.BodyColor3s = nil
	if r.bodyColor3s then
		local bc = {}
		for k, v in r.bodyColor3s do
			bc[(k:sub(1, 1):upper() .. k:sub(2)):gsub("Color3$", "Color")] = hexcolor(v)
		end
		out.BodyColors = bc
	end
	return out
end
M.GetOutfitDetails = M.GetOutfitDetailsAsync

M.GetItemDetailsAsync = function(self, itemId, itemType)
	itemId = cast(itemId, 1, "int64")
	itemType = enumarg(itemType, 2, Enum.AvatarItemType)
	local r = get(string.format("%s/v1/catalog/items/%d/details?itemType=%s", CATALOG, itemId, itemType.Name))
	if type(r) ~= "table" then
		error("Data returned was not in the expected format", 0)
	end
	return pascal(r)
end
M.GetItemDetails = M.GetItemDetailsAsync

local MAX_BATCH = 100
M.GetBatchItemDetailsAsync = function(self, itemIds, itemType)
	cast(itemIds, 1, "Array")
	itemType = enumarg(itemType, 2, Enum.AvatarItemType)
	if #itemIds > MAX_BATCH then
		error(string.format("A maximum of %d item details can be requested at once", MAX_BATCH), 0)
	end
	local items = {}
	for _, id in itemIds do
		if type(id) ~= "number" or id % 1 ~= 0 then
			error("Unexpected value in itemIds, must be an int", 0)
		end
		table.insert(items, { itemType = itemType.Name, id = id })
	end
	local r = decode(request("POST", CATALOG .. "/v1/catalog/items/details", HttpService:JSONEncode({ items = items })))
	if type(r) ~= "table" or type(r.data) ~= "table" then
		error("Unexpected value in AvatarEditorService:GetBatchItemDetails result: expected ValueTable", 0)
	end
	return pascal(r.data)
end
M.GetBatchItemDetails = M.GetBatchItemDetailsAsync

M.GetFavoriteAsync = function(self, itemId, itemType)
	itemId = cast(itemId, 1, "int64")
	itemType = enumarg(itemType, 2, Enum.AvatarItemType)
	local kind = itemType == Enum.AvatarItemType.Bundle and "bundles" or "assets"
	local r = get(string.format("%s/v1/favorites/users/%d/%s/%d/favorite", CATALOG, uid(), kind, itemId))
	return r ~= nil
end
M.GetFavorite = M.GetFavoriteAsync

M.GetRecommendedAssetsAsync = function(self, assetType, contextAssetId)
	assetType = enumarg(assetType, 1, Enum.AvatarAssetType)
	contextAssetId = contextAssetId == nil and 0 or cast(contextAssetId, 2, "int64")
	local r = get(string.format("%s/v2/recommendations/assets?assetTypeId=%d&assetId=%d&numItems=%d&details=%s", CATALOG, assetType.Value, contextAssetId, 50, "true"))
	if type(r) ~= "table" then
		error("Unexpected value in AvatarEditorService:GetRecommendationsSuccess result: expected ValueTable", 0)
	end
	return pascal(r.data or {})
end
M.GetRecommendedAssets = M.GetRecommendedAssetsAsync

M.GetRecommendedBundlesAsync = function(self, bundleId)
	bundleId = cast(bundleId, 1, "int64")
	local r = get(string.format("%s/v2/recommendations/bundles?bundleId=%d&numItems=%d&details=%s", CATALOG, bundleId, 50, "true"))
	if type(r) ~= "table" then
		error("Unexpected value in AvatarEditorService:GetRecommendationsSuccess result: expected ValueTable", 0)
	end
	return pascal(r.data or {})
end
M.GetRecommendedBundles = M.GetRecommendedBundlesAsync

M.GetBundlesByAssetIdAsync = function(self, assetId, limit)
	assetId = cast(assetId, 1, "int64")
	if assetId <= 0 then
		error("Invalid assetId: must be a positive integer.", 0)
	end
	limit = limit == nil and 10 or cast(limit, 2, "int64")
	local base = string.format("%s/v2/assets/%d/bundles?limit=%d&sortOrder=Asc", CATALOG, assetId, limit)
	return make_pages("CatalogPages", function(cursor)
		local r = get(base .. (cursor and ("&cursor=" .. query(cursor)) or "")) or {}
		return pascal(r.data or {}), r.nextPageCursor
	end)
end

local SORT = { Relevance = 0, MostFavorited = 1, Bestselling = 2, RecentlyCreated = 3, PriceLowToHigh = 4, PriceHighToLow = 5 }
local AGG = { Past12Hours = 0, PastDay = 1, Past3Days = 2, PastWeek = 3, PastMonth = 4, AllTime = 5 }

M.SearchCatalogAsync = function(self, params)
	argcheck(params, 1)
	if typeof(params) ~= "CatalogSearchParams" then
		error("Unable to cast value to CatalogSearchParams", 0)
	end
	local q = { "model.keyword=" .. query(params.SearchKeyword) }
	if params.MinPrice > 0 then
		table.insert(q, string.format("&model.minPrice=%d", params.MinPrice))
	end
	if params.MaxPrice < 2147483647 then
		table.insert(q, string.format("&model.maxPrice=%d", params.MaxPrice))
	end
	table.insert(q, string.format("&limit=%d", math.clamp(params.Limit, 1, 120) <= 30 and 30 or math.clamp(params.Limit, 1, 120)))
	table.insert(q, "&model.salesTypeFilter=" .. (params.SalesTypeFilter.Value + 1))
	if params.CategoryFilter ~= Enum.CatalogCategoryFilter.None then
		table.insert(q, "&model.categoryFilter=" .. params.CategoryFilter.Name)
	end
	table.insert(q, "&model.sortType=" .. (SORT[params.SortType.Name] or 0))
	table.insert(q, "&model.sortAggregation=" .. (AGG[params.SortAggregation.Name] or 5))
	if params.CreatorName ~= "" then
		table.insert(q, "&model.creatorName=" .. query(params.CreatorName))
	end
	for _, b in params.BundleTypes do
		table.insert(q, "&model.bundleTypeIds=" .. (b.Value + 1))
	end
	for _, a in params.AssetTypes do
		table.insert(q, "&model.assetTypeIds=" .. a.Value)
	end
	table.insert(q, "&model.includeNotForSale=" .. tostring(params.IncludeOffSaleItems))
	if params.CreatorId ~= 0 then
		table.insert(q, "&model.creatorTargetId=" .. params.CreatorId)
		table.insert(q, "&model.creatorType=" .. (params.CreatorType == Enum.CreatorTypeFilter.Group and 2 or 1))
	end
	local base = CATALOG .. "/v2/search/items/details?" .. table.concat(q)
	return make_pages("CatalogPages", function(cursor)
		local r = get(base .. (cursor and ("&cursor=" .. query(cursor)) or ""))
		if type(r) ~= "table" then
			error("Unexpected value in AvatarEditorService:SearchCatalog result: expected ValueTable", 0)
		end
		return pascal(r.data or {}), r.nextPageCursor
	end)
end
M.SearchCatalog = M.SearchCatalogAsync

M.GetAvatarRulesAsync = function(self)
	local r = get(AVATAR .. "/v1/avatar-rules")
	if type(r) ~= "table" then
		error("Data returned was not in the expected format", 0)
	end
	return pascal(r)
end
M.GetAvatarRules = M.GetAvatarRulesAsync

M.GetHeadShapesAsync = function(self)
	local r = get(AVATAR .. "/v1/avatar-inventory?itemCategories[0].itemSubType=1&itemCategories[0].itemType=headshape&sortOption=1&pageLimit=120&pageToken=0")
	if r == nil then
		error("GetHeadShapesAsync received empty response", 0)
	end
	if type(r.avatarInventoryItems) ~= "table" then
		error("GetHeadShapesAsync response missing AvatarInventoryItems", 0)
	end
	local out = {}
	for _, it in r.avatarInventoryItems do
		if it.headShape then
			table.insert(out, it.headShape)
		end
	end
	return out
end

-------------------------------------------------- rules applied locally
local function desc_arg(name, d)
	if typeof(d) ~= "Instance" or not d:IsA("HumanoidDescription") then
		error(name .. " got invalid value for humanoidDescription", 0)
	end
	local ok, c = pcall(function()
		return d:Clone()
	end)
	if not ok or c == nil then
		error(name .. ": HumanoidDescription could not be cloned. Is HumanoidDescription.Archivable set to false?", 0)
	end
	return c
end

-- CIE76 ΔE between two Color3s (Studio compares the torso and leg colours
-- with the rules' MinimumDeltaEBodyColorDifference).
local function lab(c)
	local function lin(u)
		return u <= 0.04045 and u / 12.92 or ((u + 0.055) / 1.055) ^ 2.4
	end
	local r, g, b = lin(c.R), lin(c.G), lin(c.B)
	local x = (r * 0.4124 + g * 0.3576 + b * 0.1805) / 0.95047
	local y = r * 0.2126 + g * 0.7152 + b * 0.0722
	local z = (r * 0.0193 + g * 0.1192 + b * 0.9505) / 1.08883
	local function f(t)
		return t > 0.008856 and t ^ (1 / 3) or 7.787 * t + 16 / 116
	end
	return 116 * f(y) - 16, 500 * (f(x) - f(y)), 200 * (f(y) - f(z))
end
local function delta_e(a, b)
	local l1, a1, b1 = lab(a)
	local l2, a2, b2 = lab(b)
	return math.sqrt((l1 - l2) ^ 2 + (a1 - a2) ^ 2 + (b1 - b2) ^ 2)
end

local cached_rules
local function avatar_rules()
	cached_rules = cached_rules or get(AVATAR .. "/v1/avatar-rules")
	return cached_rules
end

M.CheckApplyDefaultClothingAsync = function(self, desc)
	argcheck(desc, 1)
	local c = desc_arg("CheckApplyDefaultClothing", desc)
	local r = avatar_rules() or {}
	local min = r.minimumDeltaEBodyColorDifference or 11.4
	local legs = math.min(delta_e(c.TorsoColor, c.LeftLegColor), delta_e(c.TorsoColor, c.RightLegColor))
	if (c.Shirt ~= 0 and c.Pants ~= 0) or legs >= min then
		c:Destroy()
		return nil
	end
	local lists = r.defaultClothingAssetLists or {}
	if c.Shirt == 0 and lists.defaultShirtAssetIds and lists.defaultShirtAssetIds[1] then
		c.Shirt = lists.defaultShirtAssetIds[1]
	end
	if c.Pants == 0 and lists.defaultPantAssetIds and lists.defaultPantAssetIds[1] then
		c.Pants = lists.defaultPantAssetIds[1]
	end
	return c
end
M.CheckApplyDefaultClothing = M.CheckApplyDefaultClothingAsync

M.ConformToAvatarRulesAsync = function(self, desc)
	argcheck(desc, 1)
	local c = desc_arg("ConformToAvatarRules", desc)
	local r = avatar_rules()
	if type(r) ~= "table" or type(r.scales) ~= "table" then
		error("ConformToAvatarRules unexpected error parsing scale values", 0)
	end
	for prop, key in { HeightScale = "height", WidthScale = "width", HeadScale = "head", ProportionScale = "proportion", BodyTypeScale = "bodyType" } do
		local s = r.scales[key]
		if s then
			c[prop] = math.clamp(c[prop], s.min, s.max)
		end
	end
	-- Rigid accessories: at most each type's maxNumber, refinements clamped.
	local max = {}
	for _, w in r.wearableAssetTypes or {} do
		max[w.id] = w.maxNumber
	end
	local accs = c:GetAccessories(true)
	local count, kept = {}, {}
	local lo, hi = r.accessoryRefinementLowerBounds or {}, r.accessoryRefinementUpperBounds or {}
	for _, a in accs do
		local at = a.IsLayered and a.AccessoryType.Name .. "Accessory" or (a.AccessoryType.Name == "Hat" and "Hat" or a.AccessoryType.Name .. "Accessory")
		local ok, item = pcall(function()
			return Enum.AvatarAssetType[at]
		end)
		local limit = ok and item and max[item.Value] or nil
		count[at] = (count[at] or 0) + 1
		if limit == nil or count[at] <= limit then
			local l, h = lo[at], hi[at]
			if l and h then
				local function clampv(v, a0, b0, ka, kb, kc)
					return Vector3.new(
						math.clamp(v.X, a0[ka], b0[ka]),
						math.clamp(v.Y, a0[kb], b0[kb]),
						math.clamp(v.Z, a0[kc], b0[kc])
					)
				end
				a.Position = clampv(a.Position, l.position, h.position, "xPosition", "yPosition", "zPosition")
				a.Rotation = clampv(a.Rotation, l.rotation, h.rotation, "xRotation", "yRotation", "zRotation")
				a.Scale = clampv(a.Scale, l.scale, h.scale, "xScale", "yScale", "zScale")
			end
			table.insert(kept, a)
		end
	end
	c:SetAccessories(kept, true)
	-- Body colours snap to the palette.
	local palette = {}
	for _, p in r.bodyColorsPalette or {} do
		table.insert(palette, BrickColor.new(p.brickColorId).Color)
	end
	if #palette > 0 then
		for _, prop in { "HeadColor", "TorsoColor", "LeftArmColor", "RightArmColor", "LeftLegColor", "RightLegColor" } do
			local best, bd = nil, math.huge
			for _, p in palette do
				local d = delta_e(c[prop], p)
				if d < bd then
					best, bd = p, d
				end
			end
			c[prop] = best
		end
	end
	return c
end
M.ConformToAvatarRules = M.ConformToAvatarRulesAsync

------------------------------------------------- AvatarCreationService
local function server_only(name)
	if is_client() then
		error("AvatarCreationService::" .. name .. " - Can Only be Called on the Server.", 0)
	end
end
local function player_arg(p, i)
	argcheck(p, i)
	if typeof(p) ~= "Instance" or not p:IsA("Player") then
		error("Unable to cast value to Object", 0)
	end
	return p
end

local acs_inst
local function acs()
	acs_inst = acs_inst or game:GetService("AvatarCreationService")
	return acs_inst
end

M.GetValidationRules = function(self)
	return rules()
end

M.GetBatchTokenDetailsAsync = function(self, tokenIds)
	cast(tokenIds, 1, "Array")
	if #tokenIds == 0 then
		error("AvatarCreationService:getTokenDetailsAsync - tokenIds must not be empty", 0)
	end
	for _, t in tokenIds do
		if type(t) ~= "string" then
			error("Unexpected value in tokenIds, must be a string", 0)
		end
		if not t:match("^%x%x%x%x%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%x%x%x%x%x%x%x%x$") or t:match("%u") then
			error("Unexpected value in tokenIds, string must be a valid creation token", 0)
		end
	end
	-- Tokens belong to a published experience's universe; a Play test of a
	-- local or unpublished place has none, and Studio fails the lookup.
	if game.GameId == 0 then
		error("AvatarCreationService:getTokenDetailsAsync - Failed Unexpectedly", 0)
	end
	error("AvatarCreationService:getTokenDetailsAsync - Failed Unexpectedly", 0)
end

M.LoadGeneratedAvatarAsync = function(self, generationId)
	argcheck(generationId, 1)
	-- Only ids GenerateAvatarAsync handed out in this session exist.
	error("Unable to find id.", 0)
end
M.LoadAvatar2DPreviewAsync = function(self, previewId)
	argcheck(previewId, 1)
	error("Unable to find id.", 0)
end

M.GenerateAvatarAsync = function(self, params, cb)
	cast(params, 1, "Dictionary")
	local n = 0
	for _ in params do
		n += 1
	end
	if n < 2 then
		error("Incorrect number of arguments, expected 2.", 0)
	end
	error("AvatarCreationService:GenerateAvatarAsync - no avatar generation session; call RequestAvatarGenerationSessionAsync first", 0)
end
M.GenerateAvatar2DPreviewAsync = function(self, params, cb)
	cast(params, 1, "Dictionary")
	local n = 0
	for _ in params do
		n += 1
	end
	if n < 3 then
		error("Incorrect number of arguments, expected 3.", 0)
	end
	error("AvatarCreationService:GenerateAvatar2DPreviewAsync - no avatar generation session; call RequestAvatarGenerationSessionAsync first", 0)
end

M.RequestAvatarGenerationSessionAsync = function(self, player, callback)
	player_arg(player, 1)
	argcheck(callback, 2)
	-- Avatar generation is a Roblox service for published experiences; Studio
	-- answers a Play test that has no universe with a refused session.
	return false, "Avatar generation is not available for this experience"
end

M.AutoSetupAvatarAsync = function(self, player, params, cb)
	player_arg(player, 1)
	cast(params, 2, "Dictionary")
	error("AvatarCreationService:AutoSetupAvatarAsync - Avatar auto setup is not available for this experience", 0)
end

M.PrepareAvatarForPreviewAsync = function(self, model)
	argcheck(model, 1)
	if typeof(model) ~= "Instance" or not model:FindFirstChildOfClass("Humanoid") then
		error("AvatarCreationService:PrepareAvatarForPreviewAsync - Humanoid not found in model.", 0)
	end
	task.wait()
end

local function acs_prompt(name, title, body, done)
	dialog(title, body .. " (rustudio never publishes or buys anything.)", "Create", nil, done)
end

M.PromptCreateAvatarAsync = function(self, tokenId, player, desc)
	argcheck(tokenId, 1)
	player_arg(player, 2)
	argcheck(desc, 3)
	server_only("PromptCreateAvatarAsync")
	if typeof(desc) ~= "Instance" or not desc:IsA("HumanoidDescription") then
		error("Unable to cast value to Object", 0)
	end
	if #desc:GetChildren() == 0 then
		return Enum.PromptCreateAvatarResult.InvalidHumanoidDescription, "AvatarCreationService::PromptCreateAvatarAsync invalid humanoid description - no children"
	end
	local result = Instance.new("BindableEvent")
	local out
	acs_prompt("PromptCreateAvatarAsync", "Create Avatar", "Create this avatar and add it to your inventory?", function(ok)
		out = ok and Enum.PromptCreateAvatarResult.UploadFailed or Enum.PromptCreateAvatarResult.PermissionDenied
		result:Fire()
	end)
	result.Event:Wait()
	return out, out == Enum.PromptCreateAvatarResult.UploadFailed and "AvatarCreationService:PromptCreateAvatarAsync Unable to Publish Assets" or nil
end

M.PromptCreateAvatarAssetAsync = function(self, tokenId, player, assetInstance, assetType)
	argcheck(tokenId, 1)
	player_arg(player, 2)
	argcheck(assetInstance, 3)
	assetType = enumarg(assetType, 4, Enum.AvatarAssetType)
	server_only("PromptCreateAvatarAssetAsync")
	local result = Instance.new("BindableEvent")
	local out
	acs_prompt("PromptCreateAvatarAssetAsync", "Create Item", "Create this " .. assetType.Name .. " and add it to your inventory?", function(ok)
		out = ok and Enum.PromptCreateAssetResult.UploadFailed or Enum.PromptCreateAssetResult.PermissionDenied
		result:Fire()
	end)
	result.Event:Wait()
	return out, nil
end

M.PromptCreateMakeupAsync = function(self, player, entries, headColor)
	player_arg(player, 1)
	cast(entries, 2, "Array")
	server_only("PromptCreateMakeupAsync")
	local result = Instance.new("BindableEvent")
	local out
	acs_prompt("PromptCreateMakeupAsync", "Create Makeup", "Create this makeup look?", function(ok)
		out = ok and Enum.PromptCreateAvatarResult.UploadFailed or Enum.PromptCreateAvatarResult.PermissionDenied
		result:Fire()
	end)
	result.Event:Wait()
	return out, nil
end

M.PromptSelectAvatarGenerationImageAsync = function(self, player)
	player_arg(player, 1)
	-- No camera or file picker in a Play test: Studio's picker returns no id.
	return ""
end

-- UGC validation against GetValidationRules (sizes, attachments, triangle
-- budgets that can be checked from the instance tree).
local function size_of(inst)
	if inst:IsA("BasePart") then
		return inst.Size
	end
	if inst:IsA("Model") then
		local _, s = inst:GetBoundingBox()
		return s
	end
	return Vector3.zero
end

M.ValidateUGCAccessoryAsync = function(self, player, accessory, accessoryType)
	player_arg(player, 1)
	if accessory == nil then
		error("AvatarCreationService: Accessory Instance is nil.", 0)
	end
	accessoryType = enumarg(accessoryType, 3, Enum.AccessoryType)
	server_only("ValidateUGCAccessoryAsync")
	local reasons = {}
	local handle = accessory:FindFirstChild("Handle")
	if not accessory:IsA("Accessory") then
		table.insert(reasons, "Expected an Accessory")
	elseif handle == nil or not handle:IsA("MeshPart") then
		table.insert(reasons, "Accessory is missing a MeshPart named Handle")
	else
		local rule
		for at, rr in rules().AccessoryRules do
			if aes():GetAccessoryType(at) == accessoryType then
				rule = rr
			end
		end
		if rule then
			local att
			for _, a in rule.Attachments do
				att = att or handle:FindFirstChild(a.Name)
				if att then
					local s = handle.Size
					if s.X > a.Size.X or s.Y > a.Size.Y or s.Z > a.Size.Z then
						table.insert(reasons, string.format("Accessory is too big for %s (max %s)", a.Name, tostring(a.Size)))
					end
				end
			end
			if att == nil then
				table.insert(reasons, "Handle has no attachment this accessory type uses")
			end
			if handle:FindFirstChildOfClass("WrapLayer") == nil and not rule.RigidAllowed then
				table.insert(reasons, "This accessory type must be layered (WrapLayer)")
			end
		end
	end
	if #reasons > 0 then
		return false, reasons
	end
	return true, nil
end

M.ValidateUGCBodyPartAsync = function(self, player, instance, bodyPart)
	player_arg(player, 1)
	argcheck(instance, 2)
	bodyPart = enumarg(bodyPart, 3, Enum.BodyPart)
	server_only("ValidateUGCBodyPartAsync")
	local reasons = {}
	local key = ({ Head = "DynamicHead", Torso = "Torso", LeftArm = "LeftArm", RightArm = "RightArm", LeftLeg = "LeftLeg", RightLeg = "RightLeg" })[bodyPart.Name]
	local rule = key and rules().BodyPartRules[Enum.AssetType[key]]
	if rule then
		for sub in rule.SubParts do
			if instance.Name ~= sub and instance:FindFirstChild(sub) == nil then
				table.insert(reasons, "Missing " .. sub)
			end
		end
		local s, b = size_of(instance), rule.Bounds.Classic
		if s.X > b.MaxSize.X or s.Y > b.MaxSize.Y or s.Z > b.MaxSize.Z or s.X < b.MinSize.X or s.Y < b.MinSize.Y or s.Z < b.MinSize.Z then
			table.insert(reasons, string.format("Body part size %s is outside [%s, %s]", tostring(s), tostring(b.MinSize), tostring(b.MaxSize)))
		end
	end
	if #reasons > 0 then
		return false, reasons
	end
	return true, nil
end

M.ValidateUGCFullBodyAsync = function(self, player, desc)
	player_arg(player, 1)
	argcheck(desc, 2)
	server_only("ValidateUGCFullBodyAsync")
	local reasons = {}
	local r = avatar_rules() or {}
	for prop, key in { HeightScale = "height", WidthScale = "width", HeadScale = "head", ProportionScale = "proportion", BodyTypeScale = "bodyType" } do
		local s = r.scales and r.scales[key]
		if s and (desc[prop] < s.min or desc[prop] > s.max) then
			table.insert(reasons, string.format("%s scale (%s) out of bounds. Valid range: [%s - %s]", key, tostring(desc[prop]), tostring(s.min), tostring(s.max)))
		end
	end
	if #reasons > 0 then
		return false, reasons
	end
	return true, nil
end
