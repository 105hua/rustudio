-- Humanoid:ApplyDescription / ApplyDescriptionReset (and their *Async
-- names), AccessoryDescription / MakeupDescription:GetAppliedInstance.
-- The character is really rebuilt: body colours, R15 scales (parts,
-- attachments, joints, HipHeight, the scale NumberValues), classic
-- clothing, face, rigid and layered accessories (downloaded with the
-- session through InsertService:LoadAsset, attached with an AccessoryWeld
-- like Humanoid:AddAccessory), the Animate script's animations; then the
-- applied copy becomes the Humanoid's HumanoidDescription child and
-- ApplyDescriptionFinished fires.
-- ApplyDescription changes only what differs from the description applied
-- before (accessories a game added itself stay); ApplyDescriptionReset
-- removes everything the description doesn't list. Body-part meshes
-- (Head / Torso / limbs ids) and Proportion / BodyType scales aren't
-- rebuilt (they swap the rig's MeshParts); see checks/parity/avatar.md.

local is_client, fire, load_start, load_poll = ...
local M = __rs_lua_methods

local applied = setmetatable({}, { __mode = "k" }) -- AccessoryDescription / MakeupDescription → instance
local loaded = {} -- asset id → Model (a template, cloned per use)

-- Downloads yield (the worker fetches, this polls each frame), as Studio's
-- ApplyDescription yields while it loads the character's assets.
local function fetch(id, format)
	local job = load_start(id, format)
	while true do
		local m, err = load_poll(job)
		if m then
			return m
		elseif m == nil then
			return nil, err
		end
		task.wait()
	end
end
__rs_avatar_load = fetch

local function load(id, format)
	if id == nil or id <= 0 then
		return nil
	end
	local key = tostring(id) .. "|" .. (format or "")
	local m = loaded[key]
	if m == nil then
		local r, err = fetch(id, format)
		m = r or false
		loaded[key] = m
		if not r then
			warn(string.format("Humanoid::ApplyDescription() could not load asset %d: %s", id, tostring(err)))
		end
	end
	return m and m:Clone() or nil
end

local function first(model, class)
	if model == nil then
		return nil
	end
	for _, d in model:GetDescendants() do
		if d:IsA(class) then
			return d
		end
	end
	return nil
end

local COLORS = {
	HeadColor = { "Head" },
	TorsoColor = { "Torso", "UpperTorso", "LowerTorso" },
	LeftArmColor = { "Left Arm", "LeftUpperArm", "LeftLowerArm", "LeftHand" },
	RightArmColor = { "Right Arm", "RightUpperArm", "RightLowerArm", "RightHand" },
	LeftLegColor = { "Left Leg", "LeftUpperLeg", "LeftLowerLeg", "LeftFoot" },
	RightLegColor = { "Right Leg", "RightUpperLeg", "RightLowerLeg", "RightFoot" },
}

local function apply_colors(char, d)
	local bc = char:FindFirstChildOfClass("BodyColors")
	if bc == nil then
		bc = Instance.new("BodyColors")
		bc.Name = "Body Colors"
		bc.Parent = char
	end
	for prop, parts in COLORS do
		local c = d[prop]
		bc[prop .. "3"] = c
		for _, n in parts do
			local p = char:FindFirstChild(n)
			if p and p:IsA("BasePart") then
				p.Color = c
			end
		end
	end
end

local SCALES = {
	{ "HeightScale", "BodyHeightScale" },
	{ "WidthScale", "BodyWidthScale" },
	{ "DepthScale", "BodyDepthScale" },
	{ "HeadScale", "HeadScale" },
	{ "ProportionScale", "BodyProportionScale" },
	{ "BodyTypeScale", "BodyTypeScale" },
}

-- Studio's R15 body-type factors relative to Classic, measured part by part
-- (Players:CreateHumanoidModelFromDescription, Studio 0.744): BodyType 1
-- with Proportion 0 (ProportionsNormal) and with Proportion 1
-- (ProportionsSlender). A part's size is its OriginalSize times
-- lerp(1, lerp(Normal, Slender, Proportion), BodyType) over its own
-- AvatarPartScaleType's factor, times Width/Height/Depth (HeadScale for the
-- head); checked against every measured mix.
local KIND = {
	UpperTorso = "UpperTorso", LowerTorso = "LowerTorso", Head = "Head",
	LeftUpperArm = "Arm", LeftLowerArm = "Arm", RightUpperArm = "Arm", RightLowerArm = "Arm",
	LeftHand = "Hand", RightHand = "Hand",
	LeftUpperLeg = "UpperLeg", RightUpperLeg = "UpperLeg", LeftLowerLeg = "LowerLeg", RightLowerLeg = "LowerLeg",
	LeftFoot = "Foot", RightFoot = "Foot",
}
local NORMAL, SLENDER
local function tables()
	if NORMAL then return end
	NORMAL = {
	UpperTorso = Vector3.new(1.033, 1.309, 1.14), LowerTorso = Vector3.new(1.033, 1.309, 1.14),
	Head = Vector3.new(0.942, 0.942, 0.942), Arm = Vector3.new(1.129, 1.342, 1.132),
	Hand = Vector3.new(1.066, 1.174, 1.231), UpperLeg = Vector3.new(1.023, 1.506, 1.023),
	LowerLeg = Vector3.new(1.023, 1.506, 1.023), Foot = Vector3.new(1.079, 1.267, 1.129),
}
SLENDER = {
	UpperTorso = Vector3.new(0.905346, 1.204232, 1.013333), LowerTorso = Vector3.new(0.985687, 1.004605, 1.013333),
	Head = Vector3.new(0.896289, 0.942, 0.896289), Arm = Vector3.new(1.003556, 1.207921, 1.006222),
	Hand = Vector3.new(0.947556, 1.174, 1.094222), UpperLeg = Vector3.new(0.976145, 1.40093, 0.909333),
	LowerLeg = Vector3.new(0.976145, 1.300518, 0.909333), Foot = Vector3.new(1.02958, 1.133274, 1.003556),
}
end

local function scale_value(h, name, default)
	local v = h:FindFirstChild(name)
	if v and v:IsA("NumberValue") then
		return v.Value
	end
	return default
end

-- The part's factor for scale values `sv` (relative to its unscaled asset).
local function part_factor(part, sv)
	local kind = KIND[part.Name]
	if kind == nil then
		return Vector3.one
	end
	tables()
	local n, sl = NORMAL[kind], SLENDER[kind]
	local target = Vector3.one:Lerp(n:Lerp(sl, sv.p), sv.b)
	local st = part:FindFirstChild("AvatarPartScaleType")
	local native = Vector3.one
	if st and st:IsA("StringValue") then
		if st.Value == "ProportionsNormal" then
			native = n
		elseif st.Value == "ProportionsSlender" then
			native = sl
		end
	end
	local hwd = part.Name == "Head" and Vector3.one * sv.head or Vector3.new(sv.w, sv.h, sv.d)
	return target / native * hwd
end

local function values_of(h)
	return {
		h = scale_value(h, "BodyHeightScale", 1), w = scale_value(h, "BodyWidthScale", 1),
		d = scale_value(h, "BodyDepthScale", 1), head = scale_value(h, "HeadScale", 1),
		p = scale_value(h, "BodyProportionScale", 0), b = scale_value(h, "BodyTypeScale", 0),
	}
end

-- The unscaled value kept in a Vector3Value child (OriginalSize /
-- OriginalPosition, as Roblox's rigs carry), made from `now / factor` the
-- first time.
local function original(inst, name, now, factor)
	local v = inst:FindFirstChild(name)
	if v == nil or not v:IsA("Vector3Value") then
		v = Instance.new("Vector3Value")
		v.Name = name
		v.Value = now / factor
		v.Parent = inst
	end
	return v.Value
end

local function rig_att(part, joint)
	local a = part and part:FindFirstChild(joint .. "RigAttachment")
	return a and a:IsA("Attachment") and a or nil
end

-- HipHeight from the rig's attachments: root centre down the left leg to the
-- sole, less half the root's height (how Roblox derives it for R15).
local function hip_from_rig(char)
	local root = char:FindFirstChild("HumanoidRootPart")
	local lt, ul, ll, ft = char:FindFirstChild("LowerTorso"), char:FindFirstChild("LeftUpperLeg"), char:FindFirstChild("LeftLowerLeg"), char:FindFirstChild("LeftFoot")
	local chain = {
		{ rig_att(root, "Root"), rig_att(lt, "Root") },
		{ rig_att(lt, "LeftHip"), rig_att(ul, "LeftHip") },
		{ rig_att(ul, "LeftKnee"), rig_att(ll, "LeftKnee") },
		{ rig_att(ll, "LeftAnkle"), rig_att(ft, "LeftAnkle") },
	}
	local y = 0
	for _, pair in chain do
		if pair[1] == nil or pair[2] == nil then
			return nil
		end
		y += pair[1].Position.Y - pair[2].Position.Y
	end
	y -= ft.Size.Y / 2
	return -y - root.Size.Y / 2
end

local function apply_scales(char, h, d, fresh)
	local old = values_of(h)
	for _, s in SCALES do
		local v = h:FindFirstChild(s[2])
		if v == nil then
			v = Instance.new("NumberValue")
			v.Name = s[2]
			v.Parent = h
		end
		v.Value = d[s[1]]
	end
	if h.RigType ~= Enum.HumanoidRigType.R15 then
		return
	end
	local new = values_of(h)
	for _, p in char:GetChildren() do
		if p:IsA("BasePart") and KIND[p.Name] then
			-- A part just swapped in is at its asset's own size.
			local f_old = fresh[p] and Vector3.one or part_factor(p, old)
			local f_new = part_factor(p, new)
			p.Size = original(p, "OriginalSize", p.Size, f_old) * f_new
			for _, a in p:GetChildren() do
				if a:IsA("Attachment") then
					a.Position = original(a, "OriginalPosition", a.Position, f_old) * f_new
				end
			end
		end
	end
	-- Joints sit on the rig attachments again (BuildRigFromAttachments).
	for _, j in char:GetDescendants() do
		if (j:IsA("Motor6D") or j:IsA("AnimationConstraint")) and j.Name ~= "" then
			local a0, a1 = rig_att(j.Part0, j.Name), rig_att(j.Part1, j.Name)
			if a0 and a1 and j:IsA("Motor6D") then
				j.C0 = a0.CFrame
				j.C1 = a1.CFrame
			end
		end
	end
	local hip1 = hip_from_rig(char)
	if hip1 and hip1 > 0 then
		h.HipHeight = hip1
	end
end

-- Body parts: a Torso / Left Arm … asset holds the R15 MeshParts in a
-- folder (R15ArtistIntent, else R15Fixed, else R15); a head asset is fetched
-- in Studio's MeshPart-head format. Each replaces its part through
-- Humanoid:ReplaceBodyPartR15 (joints rebuilt on the rig attachments).
local BODY = {
	{ "Torso", { "UpperTorso", "LowerTorso" } },
	{ "LeftArm", { "LeftUpperArm", "LeftLowerArm", "LeftHand" } },
	{ "RightArm", { "RightUpperArm", "RightLowerArm", "RightHand" } },
	{ "LeftLeg", { "LeftUpperLeg", "LeftLowerLeg", "LeftFoot" } },
	{ "RightLeg", { "RightUpperLeg", "RightLowerLeg", "RightFoot" } },
	{ "Head", { "Head" } },
}

local function apply_body_parts(h, d, base, reset, fresh)
	if h.RigType ~= Enum.HumanoidRigType.R15 then
		return
	end
	for _, b in BODY do
		local prop, parts = b[1], b[2]
		local id = d[prop]
		if id ~= 0 and (id ~= base[prop] or reset) then
			local m = load(id, prop == "Head" and "avatar_meshpart_head" or nil)
			if m then
				local folder = m:FindFirstChild("R15ArtistIntent") or m:FindFirstChild("R15Fixed") or m:FindFirstChild("R15") or m
				for _, name in parts do
					local src = folder:FindFirstChild(name, true)
					if src and src:IsA("BasePart") then
						src.Parent = nil
						local ok, done = pcall(function()
							return h:ReplaceBodyPartR15(Enum.BodyPartR15[name], src)
						end)
						if ok and done then
							fresh[src] = true
						else
							warn(string.format("Humanoid::ApplyDescription() could not replace %s with asset %d: %s", name, id, tostring(done)))
						end
					end
				end
				m:Destroy()
			end
		end
	end
end

local CLOTHES = {
	{ "Shirt", "Shirt", "Shirt", "ShirtTemplate" },
	{ "Pants", "Pants", "Pants", "PantsTemplate" },
	{ "GraphicTShirt", "ShirtGraphic", "Shirt Graphic", "Graphic" },
}

local function set_clothing(char, prop, class, name, field, id, old_id, reset)
	local cur = char:FindFirstChildOfClass(class)
	if id == old_id and not reset and (id == 0 or cur) then
		return
	end
	if cur then
		cur:Destroy()
	end
	if id == 0 then
		return
	end
	local m = load(id)
	local src = first(m, class)
	if src then
		local c = Instance.new(class)
		c.Name = name
		c[field] = src[field]
		c.Parent = char
	end
	if m then
		m:Destroy()
	end
end

local function set_face(char, id, old_id, reset)
	local head = char:FindFirstChild("Head")
	if head == nil or (id == old_id and not reset) then
		return
	end
	if id == 0 then
		return
	end
	local m = load(id)
	local src = first(m, "Decal")
	if src then
		local face = head:FindFirstChild("face")
		if face == nil then
			face = Instance.new("Decal")
			face.Name = "face"
			face.Face = Enum.NormalId.Front
			face.Parent = head
		end
		face.Texture = src.Texture
	end
	if m then
		m:Destroy()
	end
end

-- Humanoid:AddAccessory's attachment: the handle's attachment welded to the
-- body attachment of the same name.
local function attach(char, acc, a)
	local handle = acc:FindFirstChild("Handle")
	if handle and handle:IsA("BasePart") then
		handle.Anchored = false
		handle.CanCollide = false
		for _, att in handle:GetChildren() do
			if att:IsA("Attachment") then
				for _, part in char:GetChildren() do
					local body = part:IsA("BasePart") and part:FindFirstChild(att.Name)
					if body and body:IsA("Attachment") then
						local w = Instance.new("Weld")
						w.Name = "AccessoryWeld"
						w.Part0 = handle
						w.Part1 = part
						local offset = CFrame.new()
						if a and not a.IsLayered then
							offset = CFrame.new(a.Position) * CFrame.fromOrientation(math.rad(a.Rotation.X), math.rad(a.Rotation.Y), math.rad(a.Rotation.Z))
						end
						w.C0 = att.CFrame
						w.C1 = body.CFrame * offset
						w.Parent = handle
						acc.Parent = char
						return
					end
				end
			end
		end
	end
	acc.Parent = char
end

local function key(a)
	return tostring(a.AssetId) .. (a.IsLayered and "L" or "R")
end

local function apply(self, d, reset, name)
	if d == nil then
		error("Humanoid::ApplyDescription() got nil humanoidDescription", 0)
	end
	if typeof(d) ~= "Instance" or not d:IsA("HumanoidDescription") then
		error("Unable to cast value to Object", 0)
	end
	if is_client() then
		error("Humanoid::ApplyDescription() can only be called by the backend server", 0)
	end
	local ok, desc = pcall(function()
		return d:Clone()
	end)
	if not ok or desc == nil then
		error("Humanoid:ApplyDescription: HumanoidDescription could not be cloned. Is Archivable set to false?", 0)
	end
	local char = self.Parent
	if char == nil or char:FindFirstChild("HumanoidRootPart") == nil then
		error("Humanoid::ApplyDescription() Humanoid has no HumanoidRootPart", 0)
	end
	-- Same asset twice is refused, as Studio does.
	local seen = {}
	for _, a in desc:GetAccessories(true) do
		if seen[a.AssetId] then
			error("Humanoid::ApplyDescription() HumanoidDescription has the same asset used more than once: " .. a.AssetId, 0)
		end
		seen[a.AssetId] = true
	end
	local prev = self:FindFirstChildOfClass("HumanoidDescription")
	local base = prev or Instance.new("HumanoidDescription")

	local fresh = {}
	apply_body_parts(self, desc, base, reset, fresh)
	if next(fresh) then
		-- Accessories welded to a part that was swapped out attach again.
		for _, acc in char:GetChildren() do
			local handle = acc:IsA("Accoutrement") and acc:FindFirstChild("Handle")
			local w = handle and handle:FindFirstChild("AccessoryWeld")
			if w and (w.Part1 == nil or w.Part1.Parent ~= char) then
				w:Destroy()
				attach(char, acc, nil)
			end
		end
	end
	apply_colors(char, desc)
	apply_scales(char, self, desc, fresh)
	for _, c in CLOTHES do
		set_clothing(char, c[1], c[2], c[3], c[4], desc[c[1]], base[c[1]], reset)
	end
	set_face(char, desc.Face, base.Face, reset)

	-- Accessories: drop the ones the old description had (all of them on a
	-- reset) that the new one doesn't, add the new ones.
	local want = {}
	for _, a in desc:GetAccessories(true) do
		want[key(a)] = a
	end
	local had = {}
	for _, a in base:GetAccessories(true) do
		had[key(a)] = true
	end
	local present = {}
	for _, acc in char:GetChildren() do
		if acc:IsA("Accoutrement") then
			local id = acc:GetAttribute("RustudioAssetId")
			local k = id and (tostring(id) .. (acc:GetAttribute("RustudioLayered") and "L" or "R"))
			if k and want[k] then
				present[k] = acc
			elseif reset or (k and had[k]) then
				acc:Destroy()
			end
		end
	end
	local made = {}
	for k, a in want do
		local acc = present[k]
		if acc == nil then
			local m = load(a.AssetId)
			acc = first(m, "Accoutrement")
			if acc then
				acc.Parent = nil
				acc:SetAttribute("RustudioAssetId", a.AssetId)
				acc:SetAttribute("RustudioLayered", a.IsLayered or nil)
				attach(char, acc, a)
			end
			if m then
				m:Destroy()
			end
		end
		made[a.AssetId] = acc
	end

	-- Animations: the Animate script's StringValue / Animation ids.
	local animate = char:FindFirstChild("Animate")
	if animate then
		for prop, slot in { RunAnimation = "run", WalkAnimation = "walk", JumpAnimation = "jump", FallAnimation = "fall", IdleAnimation = "idle", ClimbAnimation = "climb", SwimAnimation = "swim", MoodAnimation = "mood" } do
			local id = desc[prop]
			if id ~= 0 and (id ~= base[prop] or reset) then
				local m = load(id)
				local sv = animate:FindFirstChild(slot)
				if m and sv then
					local src = m:FindFirstChild(slot, true) or m
					local anims = {}
					for _, x in src:GetDescendants() do
						if x:IsA("Animation") then
							table.insert(anims, x)
						end
					end
					local i = 0
					for _, x in sv:GetChildren() do
						if x:IsA("Animation") then
							i += 1
							local from = anims[i] or anims[1]
							if from then
								x.AnimationId = from.AnimationId
							end
						end
					end
				end
				if m then
					m:Destroy()
				end
			end
		end
	end

	-- The applied copy (GetAppliedDescription clones it).
	if prev then
		prev:Destroy()
	end
	desc.Name = "HumanoidDescription"
	desc.Parent = self
	for _, c in desc:GetChildren() do
		if c:IsA("AccessoryDescription") then
			applied[c] = made[c.AssetId]
		end
	end
	fire(self, "ApplyDescriptionFinished", desc)
end

M.ApplyDescription = function(self, d, verification)
	apply(self, d, false, "ApplyDescription")
end
M.ApplyDescriptionAsync = M.ApplyDescription
M.ApplyDescriptionReset = function(self, d, verification)
	apply(self, d, true, "ApplyDescriptionReset")
end
M.ApplyDescriptionResetAsync = M.ApplyDescriptionReset

local function applied_instance(kind)
	return function(self)
		local hd = self.Parent
		if hd == nil or not hd:IsA("HumanoidDescription") then
			error(kind .. " is not parented to a HumanoidDescription. Did you call ApplyDescription()?", 0)
		end
		if hd.Parent == nil or not hd.Parent:IsA("Humanoid") then
			if kind == "MakeupDescription" then
				error("MakeupDescription is not parented to a Humanoid. Did you call ApplyDescription()?", 0)
			end
			error("HumanoidDescription is not parented to a Humanoid. Did you call ApplyDescription()?", 0)
		end
		local inst = applied[self]
		if inst == nil or inst.Parent == nil then
			if kind == "MakeupDescription" then
				error("MakeupDescription's does not contain a MakeupDecal. Is this Makeup Descriptor parented correctly or did you ApplyDescription()?", 0)
			end
			error("Instance not found in the Humanoid. Check if you called ApplyDescription() on the server or client.", 0)
		end
		return inst
	end
end
M.GetAppliedInstance = function(self)
	if self:IsA("MakeupDescription") then
		return applied_instance("MakeupDescription")(self)
	end
	return applied_instance("AccessoryDescription")(self)
end

------------------------------------------------------------- wraps
-- WrapDeformer / WrapTarget / WrapTextureTransfer methods, with Studio's
-- precondition errors (read from a running Studio). The deformed CFrame
-- follows the cage: the target cage vertex nearest the point and the
-- deformer cage's vertex of the same index give the displacement (Roblox
-- pairs cage vertices by index). Texture projection between mesh and cage
-- UV spaces copies the image when both meshes load (Roblox's body cages
-- share the render mesh's UV layout); see checks/parity/avatar.md.

local function has_content(c)
	if c == nil then
		return false
	end
	if type(c) == "string" then
		return c ~= ""
	end
	local ok, uri = pcall(function()
		return c.Uri
	end)
	if ok and type(uri) == "string" and uri ~= "" then
		return true
	end
	local ok2, obj = pcall(function()
		return c.Object
	end)
	return ok2 and obj ~= nil
end

local function wrap_sibling(self)
	local part = self.Parent
	if part == nil or not part:IsA("MeshPart") then
		return nil
	end
	return part:FindFirstChildOfClass("WrapTarget") or part:FindFirstChildOfClass("WrapLayer"), part
end

local function deformer_check(self, name)
	local target = wrap_sibling(self)
	if target == nil then
		error("WrapDeformer:" .. name .. "(): WrapDeformer has no WrapTarget. Must be parented to a MeshPart with a WrapTarget or WrapLayer.", 0)
	end
	if not has_content(self.CageMeshContent) then
		error("WrapDeformer:" .. name .. "(): WrapDeformer has no cage mesh set.", 0)
	end
	return target
end

local function editable(content)
	return game:GetService("AssetService"):CreateEditableMeshAsync(content)
end

M.SetCageMeshContent = function(self, content, origin)
	if content == nil then
		error("Argument 1 missing or nil", 0)
	end
	self.CageMeshContent = content
	if origin ~= nil then
		self.CageOrigin = origin
	end
end

M.CreateEditableMeshAsync = function(self)
	deformer_check(self, "CreateEditableMeshAsync")
	return editable(self.CageMeshContent)
end

M.GetDeformedCFrameAsync = function(self, cf)
	if cf == nil then
		error("Argument 1 missing or nil", 0)
	end
	local target = deformer_check(self, "GetDeformedCFrameAsync")
	if not has_content(target.CageMeshContent) then
		return cf
	end
	local ok, a, b = pcall(function()
		return editable(target.CageMeshContent), editable(self.CageMeshContent)
	end)
	if not ok then
		error("WrapDeformer:GetDeformedCFrameAsync(): " .. tostring(a), 0)
	end
	local part = self.Parent
	local p = (target.CageOrigin:Inverse() * part.CFrame:Inverse() * cf).Position
	local best, bd = nil, math.huge
	for _, v in a:GetVertices() do
		local d = (a:GetPosition(v) - p).Magnitude
		if d < bd then
			best, bd = v, d
		end
	end
	if best == nil then
		return cf
	end
	local ids = b:GetVertices()
	local av = a:GetVertices()
	local index = table.find(av, best)
	local other = index and ids[index]
	if other == nil then
		return cf
	end
	local delta = b:GetPosition(other) - a:GetPosition(best)
	return cf + part.CFrame:VectorToWorldSpace(delta)
end

local function copy_image(img)
	local AssetService = game:GetService("AssetService")
	local out = AssetService:CreateEditableImage({ Size = img.Size })
	out:WritePixelsBuffer(Vector2.zero, img.Size, img:ReadPixelsBuffer(Vector2.zero, img.Size))
	return out
end

M.CreateTextureInCageSpaceAsync = function(self, texture, options)
	if texture == nil then
		error("Argument 1 missing or nil", 0)
	end
	local part = self.Parent
	if part == nil or not part:IsA("MeshPart") or not has_content(part.MeshContent) or not has_content(self.CageMeshContent) then
		error("CreateTextureInCageSpaceAsync: Failed to get mesh data for reference mesh part or cage.", 0)
	end
	local ok, err = pcall(function()
		editable(part.MeshContent)
		editable(self.CageMeshContent)
	end)
	if not ok then
		error("CreateTextureInCageSpaceAsync: Failed to get mesh data for reference mesh part or cage.", 0)
	end
	return copy_image(texture)
end

M.CreateTextureInTargetSpaceAsync = function(self, texture, transfer)
	if texture == nil then
		error("Argument 1 missing or nil", 0)
	end
	if transfer == nil then
		error("Argument 2 missing or nil", 0)
	end
	local part = self.Parent
	if part == nil or not part:IsA("MeshPart") or not has_content(part.MeshContent) or not has_content(transfer.ReferenceCageMeshContent) then
		error("CreateTextureInTargetSpaceAsync failed: Failed to fetch required mesh data.", 0)
	end
	local ok = pcall(function()
		editable(part.MeshContent)
		editable(transfer.ReferenceCageMeshContent)
	end)
	if not ok then
		error("CreateTextureInTargetSpaceAsync failed: Failed to fetch required mesh data.", 0)
	end
	return copy_image(texture)
end

M.PrepareProjectionMeshDataAsync = function(self)
	local decal = self.Parent
	local part = decal and decal.Parent
	local ok = decal and decal:IsA("Decal") and part and part:IsA("MeshPart") and has_content(part.MeshContent)
		and part:FindFirstChildOfClass("WrapTarget") ~= nil and part:IsDescendantOf(game)
	if not ok then
		error("PrepareProjectionMeshDataAsync: Expected a Decal on a MeshPart with mesh content and a valid WrapTarget in a DataModel.", 0)
	end
	local loaded_ok, err = pcall(function()
		editable(part.MeshContent)
		if has_content(self.ReferenceCageMeshContent) then
			editable(self.ReferenceCageMeshContent)
		end
	end)
	if not loaded_ok then
		error("PrepareProjectionMeshDataAsync: " .. tostring(err), 0)
	end
end
