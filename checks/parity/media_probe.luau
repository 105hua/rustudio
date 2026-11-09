task.spawn(function()
local out = {}
local function P(...) local t = {} for i = 1, select("#", ...) do t[i] = tostring((select(i, ...))) end print("MP " .. table.concat(t, " ")) end
local f = Instance.new("Folder") f.Parent = workspace
local function W(a, b, sn, tn) local w = Instance.new("Wire") w.SourceInstance = a w.TargetInstance = b if sn then w.SourceName = sn end if tn then w.TargetName = tn end w.Parent = f return w end
-- pins
P("pins", table.concat(Instance.new("AudioCompressor"):GetInputPins(), ","), table.concat(Instance.new("AudioPlayer"):GetOutputPins(), ","))
-- wiring rules
local p = Instance.new("AudioPlayer") p.Name = "P1" p.Parent = f
local fd = Instance.new("AudioFader") fd.Parent = f
local log = {}
p.WiringChanged:Connect(function(c, pin, w, o) table.insert(log, "P " .. tostring(c) .. " " .. pin .. " " .. o.Name) end)
fd.WiringChanged:Connect(function(c, pin, w, o) table.insert(log, "F " .. tostring(c) .. " " .. pin .. " " .. o.Name) end)
local w = W(p, fd)
local d = W(p, fd)
P("connected", w.Connected, "dup", d.Connected, #fd:GetConnectedWires("Input"))
local loop = Instance.new("Wire") loop.Parent = f loop.SourceInstance = fd loop.TargetInstance = fd
P("selfloop", loop.Connected)
local bad = Instance.new("Wire") bad.Parent = f bad.SourceInstance = p bad.TargetInstance = fd bad.TargetName = "Bogus"
P("bogus", bad.Connected)
task.wait()
P("events", table.concat(log, " | "))
-- player + analyzer
local an = Instance.new("AudioAnalyzer") an.Parent = f
local pl = Instance.new("AudioPlayer") pl.Parent = f pl.Looping = true
W(pl, an)
local ev = {}
pl:GetPropertyChangedSignal("IsReady"):Connect(function() table.insert(ev, "ready " .. tostring(pl.IsReady)) end)
pl.Asset = "rbxassetid://8000000000099003"
P("ready", pl.IsReady, pl.TimeLength)
pl:Play()
task.wait(0.5)
local s = an:GetSpectrum()
local mx, mi = 0, 0 for i, v in s do if v > mx then mx, mi = v, i end end
P("analyzer", an.RmsLevel, an.PeakLevel, #s, mi, mx, pl.TimePosition, table.concat(ev, ","))
-- filter
local flt = Instance.new("AudioFilter") flt.FilterType = Enum.AudioFilterType.Lowpass24dB
P("gainat", flt:GetGainAt(1000), flt:GetGainAt(4000))
flt.Q = 99 P("Qclamp", flt.Q)
-- ended
local one = Instance.new("AudioPlayer") one.Parent = f one.Asset = "rbxassetid://8000000000099004" one.PlaybackSpeed = 10
local ended = false one.Ended:Connect(function() ended = true end)
one:Play() task.wait(1)
P("ended", ended, one.IsPlaying, one.TimePosition)
-- emitter/listener
local a = Instance.new("Part") a.Anchored = true a.Parent = f a.CFrame = CFrame.new()
local b = Instance.new("Part") b.Anchored = true b.Parent = f b.Position = Vector3.new(20, 0, 0)
local em = Instance.new("AudioEmitter") em.Parent = a local li = Instance.new("AudioListener") li.Parent = b
P("audibility", em:GetAudibilityFor(li), #em:GetInteractingListeners())
em:SetDistanceAttenuation({ [0] = 1, [10] = 0.5, [30] = 0.1 })
P("custom", em:GetAudibilityFor(li))
P("waveform", #pl:GetWaveformAsync(NumberRange.new(0, 1), 8))
P("rec", pcall(function() return Instance.new("AudioRecorder"):CanRecordAsync() end))
P("mixer", game:GetService("SoundService"):GetMixerTime() > 0)
P("done")
end)
