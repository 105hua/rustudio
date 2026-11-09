-- Place A match probe (server): once round 1 is active, WATCH seconds of play are watched;
-- prints MATCH|{json} with how many enemies were spawned and the player's state.
-- (Place A simulates enemies as data in Match1.Game.ZombieSim; the session folder
-- carries Round / RoundState / ZombiesLeft.)
local WATCH = 60 -- seconds of play watched once round 1 is active
task.spawn(function()
	local H = game:GetService("HttpService")
	local RS = game:GetService("ReplicatedStorage")
	local t0 = os.clock()
	local session
	while os.clock() - t0 < 120 do
		local s = RS:FindFirstChild("Sessions")
		session = s and s:GetChildren()[1]
		if session and session:GetAttribute("RoundState") == "active" then break end
		task.wait(0.5)
	end
	if not (session and session:GetAttribute("RoundState") == "active") then
		print("MATCH|" .. H:JSONEncode({ started = false, waited = os.clock() - t0 }))
		return
	end
	local activeAt = os.clock() - t0
	local first = session:GetAttribute("ZombiesLeft") or 0
	local minLeft, peak, rounds, downs = first, 0, session:GetAttribute("Round") or 0, 0
	local ok, ZS = pcall(function()
		return require(game:GetService("ServerScriptService"):FindFirstChild("Match1"):FindFirstChild("Game"):FindFirstChild("ZombieSim"))
	end)
	local p = game.Players:GetPlayers()[1]
	local t1 = os.clock()
	while os.clock() - t1 < WATCH do
		local left = session:GetAttribute("ZombiesLeft") or minLeft
		minLeft = math.min(minLeft, left)
		rounds = math.max(rounds, session:GetAttribute("Round") or 0)
		if ok and ZS and ZS.count then peak = math.max(peak, ZS.count()) end
		if p and p:GetAttribute("Downed") then downs = 1 end
		task.wait(0.25)
	end
	print("MATCH|" .. H:JSONEncode({
		started = true, watched = WATCH, activeAfter = activeAt, firstLeft = first, minLeft = minLeft,
		peakAlive = peak, round = rounds, kills = p and p:GetAttribute("Kills") or 0,
		points = p and p:GetAttribute("Points") or 0, downed = downs == 1,
		state = session:GetAttribute("RoundState"),
	}))
end)
