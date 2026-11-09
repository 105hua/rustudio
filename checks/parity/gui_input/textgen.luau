-- TextGenerator:GenerateTextAsync through the login session (gui_input.md).
-- Run on the scratch universe's place so game.GameId is published.
task.spawn(function()
print("TG gameid", game.GameId)
local g = Instance.new("TextGenerator")
print("TG unparented", pcall(function() return g:GenerateTextAsync({ UserPrompt = "hi" }) end))
g.Parent = workspace
print("TG bad", pcall(function() return g:GenerateTextAsync({ UserPrompt = 5 }) end))
print("TG maxtokens", pcall(function() return g:GenerateTextAsync({ UserPrompt = "hi", MaxTokens = 0 }) end))
g.SystemPrompt = "Answer in at most five words."
local ok, r = pcall(function() return g:GenerateTextAsync({ UserPrompt = "Name a colour.", MaxTokens = 20 }) end)
if ok then print("TG ok", r.GeneratedText, typeof(r.ContextToken), #r.ContextToken > 0, r.Model) else print("TG err", r) end
if ok then
  local ok2, r2 = pcall(function() return g:GenerateTextAsync({ UserPrompt = "Another one.", ContextToken = r.ContextToken, MaxTokens = 20 }) end)
  print("TG ctx", ok2, ok2 and r2.GeneratedText or r2)
end
end)
