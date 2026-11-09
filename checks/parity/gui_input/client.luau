task.spawn(function()
-- rustudio --luau client probe (checks/parity/gui_input.md). Each line has
-- Studio's answer for the same code (offline Studio, 2026-11-07) after "| studio".
local function dump(v) if type(v)=="table" then local p={} for k,x in v do table.insert(p, tostring(k).."="..dump(x)) end table.sort(p) return "{"..table.concat(p,",").."}" end if typeof(v)=="Instance" then return v.Name end return tostring(v) end
local function t(name, f) local r = table.pack(pcall(f)) local s = {} for i=1,r.n do s[i]=dump(r[i]) end print("GI " .. name .. " => " .. table.concat(s, " | ")) end
local GS = game:GetService("GuiService")
local UIS = game:GetService("UserInputService")
local plr = game.Players.LocalPlayer
local pg = plr:WaitForChild("PlayerGui")
t("gs props", function() return GS.CoreGuiNavigationEnabled, GS.PreferredTextSize, GS.TopbarInset, GS.ViewportDisplaySize end) -- | true Medium 0,0,0,0 Medium
t("notif", function() return GS:SendNotification({}) end) -- | false GuiService:SendNotification is not yet enabled!
t("inset core", function() return GS:GetInsetArea(Enum.ScreenInsets.CoreUISafeInsets) end) -- | 0, 0, W, H-58
t("pg", function() return pg.CurrentScreenOrientation, pg:GetTopbarTransparency() end) -- | LandscapeLeft 0.5
t("topbar", function() local n = {} pg.TopbarTransparencyChangedSignal:Connect(function(v) table.insert(n, v) end) pg:SetTopbarTransparency(0.3) task.wait() return pg:GetTopbarTransparency(), n end) -- | 0.30000001192092896 {1=0.30000001192092896}
-- Path2D
local sg = Instance.new("ScreenGui") sg.Name = "SG" sg.Parent = pg
local f = Instance.new("Frame", sg) f.Size = UDim2.fromOffset(1000,1000) f.Position = UDim2.fromOffset(30,40)
local p = Instance.new("Path2D", f)
p:SetControlPoints({Path2DControlPoint.new(UDim2.fromOffset(0,0), UDim2.fromOffset(0,0), UDim2.fromOffset(50,0)), Path2DControlPoint.new(UDim2.fromOffset(100,100), UDim2.fromOffset(-50,0), UDim2.fromOffset(0,0))})
task.wait()
t("p2d len", function() return p:GetLength(), p:GetBoundingRect() end) -- | 141.42135620117188 29,39,131,141
t("p2d pos", function() return p:GetPositionOnCurve(0.5), p:GetTangentOnCurve(0.5) end) -- | {0.05,0},{0.05,0} 75, 150
t("p2d arc", function() return p:GetPositionOnCurveArcLength(1) end) -- | {0.0919652,0},{0.0990735,0}
t("p2d bad", function() return p:GetControlPoint(3) end) -- | false Path2D:GetControlPoint() at index 3 access out of bounds
t("p2d cp", function() return p:GetControlPoint(1) end)
-- GetGuiObjectsAtPosition
local a = Instance.new("Frame") a.Name = "A" a.Size = UDim2.fromOffset(200,200) a.Position = UDim2.fromOffset(10,10) a.Parent = sg
local b = Instance.new("TextButton") b.Name = "B" b.Size = UDim2.fromOffset(50,50) b.Position = UDim2.fromOffset(20,20) b.Parent = a
local c = Instance.new("Frame") c.Name = "C" c.Size = UDim2.fromOffset(100,100) c.ZIndex = 5 c.Parent = sg
f.Visible = false
task.wait()
t("at", function() return pg:GetGuiObjectsAtPosition(30, 30) end) -- | {C,B,A} (after the chat's)
t("sg at", function() return sg:GetGuiObjectsAtPosition(30, 30) end) -- | {}
-- TextBox focus
local tb = Instance.new("TextBox") tb.Name = "TB" tb.Text = "hello" tb.Size = UDim2.fromOffset(100,30) tb.Position = UDim2.fromOffset(300,300) tb.Parent = sg
task.wait()
local ev = {}
tb.Focused:Connect(function() table.insert(ev, "Focused c="..tb.CursorPosition.." s="..tb.SelectionStart) end)
tb.FocusLost:Connect(function(e, io) table.insert(ev, "FocusLost "..tostring(e).." "..tostring(io)) end)
UIS.TextBoxFocused:Connect(function(x) table.insert(ev, "UIS.Focused "..x.Name) end)
UIS.TextBoxFocusReleased:Connect(function(x) table.insert(ev, "UIS.Released "..x.Name) end)
tb:CaptureFocus()
table.insert(ev, "after capture focused="..tostring(tb:IsFocused()).." c="..tb.CursorPosition.." gf="..tostring(UIS:GetFocusedTextBox()))
task.wait()
tb:ReleaseFocus(true)
table.insert(ev, "after release c="..tb.CursorPosition.." s="..tb.SelectionStart)
task.wait()
t("tb ev", function() return table.concat(ev, "; ") end) -- | Focused c=1 s=-1; UIS.Focused TB; after capture focused=true c=1 gf=TB; FocusLost true nil; UIS.Released TB; after release c=-1 s=-1
-- UIPageLayout
local holder = Instance.new("Frame") holder.Name = "Holder" holder.Size = UDim2.fromOffset(300,200) holder.Parent = sg
local pl = Instance.new("UIPageLayout") pl.Parent = holder
local pages = {}
for i=1,3 do local q = Instance.new("Frame") q.Name = "P"..i q.Size = UDim2.fromScale(1,1) q.LayoutOrder = i q.Parent = holder pages[i]=q end
task.wait()
local pev = {}
pl.Stopped:Connect(function(q) table.insert(pev, "stopped "..q.Name) end)
pl:GetPropertyChangedSignal("CurrentPage"):Connect(function() table.insert(pev, "cur "..tostring(pl.CurrentPage)) end)
t("page cur", function() return pl.CurrentPage end) -- | P1
pl:Next() table.insert(pev, "after next "..tostring(pl.CurrentPage)) task.wait(1.5)
pl.Animated = false pl:JumpToIndex(2) task.wait(0.1) pl:JumpToIndex(10) task.wait(0.1)
t("page ev", function() return table.concat(pev, "; ") end) -- | cur P2; after next P2; stopped P2; cur P3; cur P2
t("page pos", function() return pages[2].AbsolutePosition.X - holder.AbsolutePosition.X end) -- | 0 (current page in view)
-- Selection
local h3 = Instance.new("Frame") h3.Name = "H" h3.Size = UDim2.fromOffset(300,200) h3.Position = UDim2.fromOffset(400,100) h3.Parent = sg
local b1 = Instance.new("TextButton") b1.Name = "B1" b1.Size = UDim2.fromOffset(50,50) b1.Position = UDim2.fromOffset(100,10) b1.Parent = h3
local b2 = Instance.new("TextButton") b2.Name = "B2" b2.Size = UDim2.fromOffset(50,50) b2.Position = UDim2.fromOffset(10,100) b2.Parent = h3
task.wait()
local sev = {}
GS:GetPropertyChangedSignal("SelectedObject"):Connect(function() table.insert(sev, "sel "..tostring(GS.SelectedObject)) end)
for _, x in {b1,b2} do x.SelectionGained:Connect(function() table.insert(sev, x.Name.." gained") end) x.SelectionLost:Connect(function() table.insert(sev, x.Name.." lost") end) end
h3.SelectionChanged:Connect(function(q,w,e) table.insert(sev, "hchanged "..tostring(q).." "..tostring(w).." "..tostring(e)) end)
GS:Select(h3) task.wait() GS.SelectedObject = b2 task.wait() GS.SelectedObject = nil task.wait()
t("sel ev", function() return table.concat(sev, "; ") end) -- | B1 gained; sel B1; hchanged false nil B1; B1 lost; B2 gained; sel B2; hchanged false B1 B2; B2 lost; sel nil; hchanged false B2 nil
-- Input actions
local ctx = Instance.new("InputContext") ctx.Parent = pg
local act = Instance.new("InputAction") act.Parent = ctx
local bind = Instance.new("InputBinding") bind.KeyCode = Enum.KeyCode.F bind.Parent = act
task.wait()
local aev = {}
act.Pressed:Connect(function() table.insert(aev,"P") end) act.Released:Connect(function() table.insert(aev,"R") end) act.StateChanged:Connect(function(s) table.insert(aev,"S"..tostring(s)) end)
act:Fire(true) task.wait() act:Fire(false) task.wait()
t("ia", function() return table.concat(aev, " "), act.PreferredBinding end) -- | P Strue R Sfalse InputBinding
t("ia bad", function() act:Fire(0.5) end) -- | false Action:Fire() on InputActionType.Bool should be called with a bool state
-- Haptics / VR / misc
local HS = game:GetService("HapticService")
t("haptic", function() return HS:IsVibrationSupported(Enum.UserInputType.Keyboard), HS:IsMotorSupported(Enum.UserInputType.Gamepad1, Enum.VibrationMotor.LeftTrigger) end) -- | false false
t("motor", function() HS:SetMotor(Enum.UserInputType.Gamepad2, Enum.VibrationMotor.Small, 0.25, 0.75) return HS:GetMotor(Enum.UserInputType.Gamepad2, Enum.VibrationMotor.Small) end) -- | 0.25 0.75
t("fx", function() local fx = Instance.new("HapticEffect") fx.Parent = workspace local n = 0 fx.Ended:Connect(function() n += 1 end) fx:Play() task.wait(0.3) return n end) -- | 1
t("vr", function() local VR = game:GetService("VRService") VR:SetTouchpadMode(Enum.VRTouchpad.Left, Enum.VRTouchpadMode.ABXY) return VR:GetTouchpadMode(Enum.VRTouchpad.Left), VR.GuiInputUserCFrame end) -- | ABXY Head
t("uis", function() return UIS.OnScreenKeyboardSize, UIS:GetDeviceAcceleration().UserInputType, #UIS:GetMouseButtonsPressed() end) -- | 0,0 Accelerometer 0
t("vi", function() local vi = UIS:CreateVirtualInput() return typeof(vi), tostring(vi), vi.ClassName end) -- | Object VirtualInput VirtualInput
t("blur", function() local x = Instance.new("UIBlur") x.Strength = Enum.UIBlurStrength.Light return x.Color, x.Enabled, x.Strength, x.Transparency end) -- | 1,1,1 true Light 0
t("pe2d", function() local e = Instance.new("ParticleEmitter2D") return e.Rate, e.Lifetime, e.Speed, e.ZIndex end) -- | 20 5 10 64 64 1
t("text", function() local l = Instance.new("TextLabel") l.Text = "<b>hi</b> &amp; x" l.RichText = true return l.ContentText, l.LocalizedText, l.TextFits end) -- | hi & x | <b>hi</b> &amp; x | false
t("guistate", function() return b1.GuiState, b1.InputSink end) -- | Idle None
t("style", function()
  local ss = Instance.new("StyleSheet") ss.Parent = sg
  local r = Instance.new("StyleRule") r.Selector = "#Styled" r.Parent = ss
  r:SetProperty("BackgroundColor3", Color3.new(1,0,0))
  local link = Instance.new("StyleLink") link.StyleSheet = ss link.Parent = sg
  local fr = Instance.new("Frame") fr.Name = "Styled" fr.Parent = sg
  task.wait() task.wait()
  local styled = fr.BackgroundColor3
  fr.Name = "NotStyled" task.wait() task.wait()
  return styled, fr.BackgroundColor3
end) -- | 1, 0, 0 | (back to the default)
-- ProximityPrompt (the server made it: server.luau)
local part = workspace:WaitForChild("PromptPart", 10)
local pp = part and part:FindFirstChildOfClass("ProximityPrompt")
local pev2 = {}
if pp then
  for _, n in {"PromptShown","Triggered","TriggerEnded","PromptButtonHoldBegan","PromptButtonHoldEnded"} do pp[n]:Connect(function() table.insert(pev2, n) end) end
  task.wait(0.5)
  pp:InputHoldBegin() task.wait(0.1) pp:InputHoldEnd() task.wait(0.1)
end
t("prompt", function() return table.concat(pev2, " ") end) -- | PromptShown Triggered TriggerEnded
print("GI done")
end)
