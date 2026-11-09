//! Legacy in-memory DataStores, isolated for explicit local testing.
pub const SOURCE: &str = r###"local local_mode = ...
local M = __rs_lua_methods
---------------------------------------------------------- DataStoreService
-- Explicit local test mode only; never used as a successful live backend.
local stores = {}
local warned_store = false
local function copy(v, depth)
    if type(v) ~= "table" or (depth or 0) > 32 then return v end
    local t = {}
    for k, x in pairs(v) do t[k] = copy(x, (depth or 0) + 1) end
    return t
end
local DataStore = {}
DataStore.__index = DataStore
function DataStore:GetAsync(key) return copy(self.data[tostring(key)]) end
local function notify(self,key,value)
    local event=self.events and self.events[tostring(key)];if event then event:Fire(copy(value))end
end
function DataStore:SetAsync(key, value) self.data[tostring(key)] = copy(value);notify(self,key,value) end
function DataStore:OnUpdate(key,callback)
    self.events=self.events or{};key=tostring(key)
    if not self.events[key]then self.events[key]=Instance.new('BindableEvent')end
    local connection=self.events[key].Event:Connect(function(v)callback(copy(v))end)
    if self.data[key]~=nil then local value=copy(self.data[key]);task.defer(function()if connection.Connected then callback(value)end end)end
    return connection
end
function DataStore:BatchGetAsync(keys)
    if string.sub(self.name,1,8)~='ordered:'then error('{"errorType":"BatchGetUnsupportedOperation","errorMessage":"BatchGetAsync is not supported.","callbackMessage":"BatchGetAsync is not supported."}',0)end
    if #keys==0 then error('OrderedDataStore BatchGetAsync keys array cannot be empty.',0)end
    if #keys>100 then error('OrderedDataStore BatchGetAsync exceeded the 100 key limit.',0)end
    local out={};for _,key in ipairs(keys)do if type(key)~='string'then error('513: Attribute keys format is invalid',0)end;if self.data[key]~=nil then out[key]={value=copy(self.data[key])}end end;return out
end
function DataStore:UpdateAsync(key, fn)
    local new = fn(copy(self.data[tostring(key)]))
    if new ~= nil then self.data[tostring(key)] = copy(new);notify(self,key,new) end
    return new
end
function DataStore:IncrementAsync(key, delta)
    local v = (tonumber(self.data[tostring(key)]) or 0) + (tonumber(delta) or 1)
    self.data[tostring(key)] = v;notify(self,key,v)
    return v
end
function DataStore:RemoveAsync(key)
    local old = self.data[tostring(key)]
    self.data[tostring(key)] = nil;notify(self,key,nil)
    return old
end
function DataStore:GetSortedAsync(ascending, page_size)
    local rows = {}
    for k, v in pairs(self.data) do table.insert(rows, { key = k, value = v }) end
    table.sort(rows, function(a, b)
        local x, y = tonumber(a.value) or 0, tonumber(b.value) or 0
        if ascending then return x < y end
        return x > y
    end)
    -- A real DataStorePages through the shared Pages (src/domains/pages.rs).
    local size, pages = tonumber(page_size) or 50, {}
    for i = 1, #rows, size do
        local page = {}
        for j = i, math.min(i + size - 1, #rows) do table.insert(page, rows[j]) end
        table.insert(pages, page)
    end
    return __rs_pages_list("DataStorePages", pages)
end
local function get_store(name, scope)
    if not warned_store then
        warned_store = true
        warn("DataStores in rustudio keep data in memory for this play session only")
    end
    local key = tostring(name) .. "/" .. tostring(scope or "global")
    stores[key] = stores[key] or setmetatable({ name = name, data = {} }, DataStore)
    return stores[key]
end
-- Like Studio: an unpublished place (PlaceId 0) can't use DataStores.
-- The message has no position, as an engine error (Studio: "DataStore
-- unavailable: You must publish …" from a pcall).
local function published()
    if local_mode then return end
    if (tonumber(game.PlaceId) or 0) == 0 then
        error("You must publish this place to the web to access DataStore.", 0)
    end
    error("rustudio: session DataStore backend is not implemented", 0)
end
function M.GetDataStore(self, name, scope) published() return get_store(name, scope) end
function M.GetGlobalDataStore(self) published() return get_store("__global") end
function M.GetOrderedDataStore(self, name, scope) published() return get_store("ordered:" .. tostring(name), scope) end

"###;
