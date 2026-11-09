# gui_input parity notes (2026-11-07/08)

Domain code: `src/domains/gui_input.rs` (hooks, property store for classes
the reflection database lacks, Rust ↔ Luau glue, engine messages to the
server), `gui_input_path2d.rs` (Path2D + Path2DControlPoint),
`gui_input.luau` (everything else). Viewport side: `src/gui.rs`
(`GuiShare`: focus, hover / press, loaded images, drags, page scroll shared
with the scripts), `src/gui_input.rs` (focus / drag / wheel / InputChanged
events, ScreenGui-held BillboardGuis), `src/gui_walk.rs` (UIPageLayout
scroll, drag flags). Shared files touched surgically: `play.rs` (engine
message hook), `play_game.rs` (wheel InputObjects), `runtime_ext.rs`
(GetFocusedTextBox moved here), `doc.rs` (classes newer than the reflection
database: API-dump hierarchy, creatable).

Probe: `checks/parity/gui_input/{place.rbxlx,client.luau,server.luau}`; each
line of client.luau carries Studio's answer.

    gamescope --backend headless -- env RUSTUDIO_NO_AUDIO=1 ./target/release/rustudio \
      checks/parity/gui_input/place.rbxlx --play --screenshot /tmp/gi.png --shot-after 22 \
      --luau server checks/parity/gui_input/server.luau@3 \
      --luau client checks/parity/gui_input/client.luau@4 --log /tmp/gi.log

## Measured in Studio (offline Studio, own instance, edit + Play)

GuiService
- CoreGuiNavigationEnabled true, PreferredTextSize Medium,
  PreferredTransparency 1, ReducedMotionEnabled false, TopbarInset
  `0, 0, 0, 0` (also in Play), ViewportDisplaySize Medium.
- GetInsetArea (Play, view 862×675): None / DeviceSafe / TopbarSafe →
  `0, -58, 862, 617`; CoreUISafeInsets → `0, 0, 862, 617`. Edit: whole view.
- SendNotification / DismissNotification → `GuiService:X is not yet enabled!`
- Select(x) off a BasePlayerGui → `Calling GuiService:Select() on an
  instance that is not a descendant of a BasePlayerGui`; else picks the
  top-most, left-most selectable descendant.
- SelectedObject set: old.SelectionLost, new.SelectionGained, Changed, then
  SelectionChanged(amISelected, prev, new) on each GuiBase2d up from new
  (and old). A non-Selectable object warns `Setting GuiService.SelectedObject
  to invalid GuiObject`, is set, then moves to its first selectable
  descendant.
- InspectPlayerFromUserId opens the inspect menu (MenuIsOpen true);
  Get/SetGameplayPausedNotificationEnabled default true; SetEmotesMenuOpen
  round-trips through GetEmotesMenuOpen.

UserInputService (no sensors / VR / touch / on-screen keyboard)
- GetDeviceAcceleration / Gravity / Rotation warn `Trying to get
  acceleration|gravity data on a device without a accelerometer.` /
  `…rotation data on a device without a gyroscope.` and return an
  InputObject (Accelerometer / Gyro, Change, zero) (+ identity CFrame).
- OnScreenKeyboardPosition / Size 0,0; UserHeadCFrame / GetUserCFrame
  identity; CreateVirtualInput → typeof "Object", ClassName VirtualInput, no
  Name / Parent; SendMouseDelta with a free cursor → `VirtualInput::
  SendMouseDelta: cursor is not locked.`
- SetNavigationGamepad(Gamepad1, true) → IsNavigationGamepad true.

TextBox (Play)
- Defaults CursorPosition 1, SelectionStart -1. CaptureFocus: (ClearTextOnFocus
  empties) cursor → #Text+1, Focused, then UIS.TextBoxFocused. ReleaseFocus(x):
  cursor -1, selection -1, FocusLost(x, nil), UIS.TextBoxFocusReleased.
  Destroying a focused box → FocusLost(false). Unparented CaptureFocus: nothing.

GetGuiObjectsAtPosition (Play)
- PlayerGui: GUI coordinates (AbsolutePosition's), top-most first, hidden
  objects excluded. A ScreenGui in the PlayerGui answers `{}` (measured).

Path2D
- Max 100 points (`Path2D:SetControlPoints() only supports 100 control
  points.` — the first 100 are kept), index errors `Path2D:X() at index N
  access out of bounds`, empty `attempting to query with zero segments`,
  unparented / non-GUI parent `Attempting to use Path2D with invalid parent`,
  zero-area parent warns `Path2D parent rectangle has zero area.` (offset
  UDim2). Segment shape from the tangents (both zero: line, one: quadratic,
  both: cubic); GetLength's adaptive halving measures a symmetric S curve as
  its chord (141.42 for a 149.5 curve) and the arc-length queries walk that
  many pixels along the real curve (arc 1.0 → (91.97, 99.07)). Bounding rect
  padded ceil(Thickness/2); ±FLT_MAX with no segment. ControlPointChanged once
  per mutation.

UIPageLayout (Play)
- CurrentPage starts at the first page; Next/Previous honour Circular;
  JumpToIndex is 0-based and wraps (10 of 3 → 2nd); JumpTo(non-GuiObject) →
  `Workspace passed to PageLayout JumpTo is not a GuiObject`. Stopped(page)
  after the tween when Animated, none when not. PageEnter / PageLeave did not
  fire in the (hidden, non-rendering) Studio, so rustudio doesn't fire them.

Styles
- StyleRule.SetProperty accepts any name; nil removes; SelectorError is ""
  even for garbage selectors. SetPropertyTransition(non-TweenInfo) warns
  ``Type `double` is not supported as a StyleRule property transition``;
  the default transition shows as `*` in GetPropertyTransitions;
  SetPropertyTransitions replaces all. SetStyleRules warns `Unparenting
  existing styles in SetStyles().`; SetDerives(non-sheet) → `Expected
  StyleSheet at index 0, got Workspace`. StyleQuery.IsActive false
  unparented. The engine applies rules (selectors: class, .tag, #name,
  :state, descendant / `>` child, `,` lists, nested rules, `$token`
  attributes, Priority, transitions through TweenService) and restores
  properties no rule sets any more.

Input action system (Play)
- Fire(true): Pressed, StateChanged(true); Fire(false): Released,
  StateChanged(false); repeats nothing. Bool + non-bool → `Action:Fire() on
  InputActionType.Bool should be called with a bool state`; other types
  ignore wrong types silently. Type change resets to the type's zero
  (ViewportPosition (-1, -1)) with StateChanged, Released first when a Bool
  takes over from a non-zero state. Disabled action / context: Fire does
  nothing, GetState is the zero. InputBinding:Fire unless Scriptable →
  `InputBinding:Fire() can only be called when Type is Scriptable`.
  PreferredBinding: the binding for the last input device. InputActionLabel
  ResolvedText "" / ResolvedImageContent none (as measured).

Haptics / VR / gamepad
- Gamepad1 connected in that Studio: IsVibrationSupported true for connected
  pads only, IsMotorSupported Large / Small only. SetMotor stores any values
  (GetMotor returns them as given); no values → Output error `Error in
  HapticService:SetMotor no values found for vibration.`
- HapticEffect: Ended only when in the DataModel, after UIHover 35 ms /
  UIClick 55 ms / UINotification 0.4 s / Collision 0.3 s / Explosion 0.8 s /
  Custom next frame; SetWaveformKeys on non-Custom → `Cannot modify
  non-custom haptic effect!`, bad keys `Variant cast failed`.
- VRService touchpad mode default VirtualThumbstick; GuiInputUserCFrame Head.
- GamepadService Enable/DisableGamepadCursor: no cursor without a pad.

ProximityPrompt (Play)
- PPS.PromptShown fires before the prompt's; for the rest the prompt's
  first. Hold: PromptButtonHoldBegan → (HoldDuration) PromptButtonHoldEnded,
  Triggered → (release) TriggerEnded; early release: HoldEnded only;
  HoldDuration 0: Triggered / TriggerEnded. Fired on the client and the
  server (engine message). The UI is Studio's own CoreScripts/
  ProximityPrompt.lua, read from the Studio install and run with its
  CoreUtility / AppFonts requires shimmed.

Others: default ParticleEmitter2D / UIBlur / UIDragDetector.Selectable /
InputBinding.Deadzone values in gui_input.rs `STORED` (the API dump's are
placeholders); TextGenerator unparented → `TextGenerator must be parented
into the DataModel.`; Dialog.InUse false, GetCurrentPlayers {}.

## Second round (2026-11-08)

ParticleEmitter2D / UIBlur (src/gui_fx.rs)
- Studio 0.744 draws neither: its FFlags `UIParticles` and `UIBlur` are off
  (game:GetFastFlag says false). Turned on (FFlagUIParticles / FFlagUIBlur,
  plus UIBoundsTracking / UITextureUsingNewJobFlow) through a briefly edited
  ClientAppSettings.json for one an offline Studio instance (restored right after), it
  still drew nothing, and with the last flag its viewport stopped
  presenting. So there is no Studio picture to compare with; rustudio
  follows the properties (ParticleEmitter's rules in 2D; a 12-tap disc blur
  of the 3D view and the GUI drawn before it, Light 4 / Medium 8 / Heavy 16
  px). Probe: gui_input/fx.luau (blurred stripes and studs, sparkles).

Handle adornments (gui_input.luau, src/gui_adorn.rs)
- an offline Studio, Play: BoxHandleAdornment MouseEnter → MouseButton1Down →
  MouseButton1Up → MouseLeave (no arguments); Handles pass the NormalId and
  MouseDrag(face, studs along the face axis since the press) (50 px drag ≈
  2 studs); hovering keeps updating during a drag (Enter / Leave mid-drag).
  ArcHandles: MouseDrag(axis, relative angle in radians, radius change),
  measured on the ring plane. Geometry: Handles spheres at half size + 2
  studs per face; ArcHandles rings of radius |half size| + 2, spheres for
  Z on ±Y, Y on ±X, X on ±Z, coloured X red / Y green / Z blue.
- rustudio draws them over the view without depth (Studio depth-tests the
  non-AlwaysOnTop ones). The harness clicks press and release in one frame;
  the hit-test reads both edges (GuiShare.mouse_edges).

Dialog (Studio's MainBotChatScript2 with stand-ins)
- an offline Studio: clicking the "?" bubble sets InUse true on the client, then the
  server (GetCurrentPlayers lists the player); a choice fires
  DialogChoiceSelected(player, choice) on the client only (the server never
  hears it); the end of the conversation sets InUse false on both.
- Studio's RobloxGui.ControlFrame.BottomLeftControl is at (0, 0, 1, -46),
  130 × 46. Frame.Style ChatBlue / Green / Red: textures/ui/dialog_*.png
  (40 px, 10 px corners) 9-sliced over the box, children inset 17 px.
- The NPC's speech bubbles (Chat:Chat) are the chat domain's.

TextGenerator:GenerateTextAsync
- POST https://apis.roblox.com/user/cloud/v2/universes/{GameId}:generateText   with the session (CSRF), body userPrompt / systemPrompt / temperature   (+ contextToken, maxTokens, responseFormatType JSON_SCHEMA + jsonSchemaStr);   reply generatedText / contextToken / model → GeneratedText / ContextToken /   Model. Checks and errors from Studio strings. Tested with   3 calls on the scratch place (two generations, one using the context   token). That run's game.GameId read 0 (the harness started Play before the   opened place arrived) and the endpoint still answered.

Inspect menu (src/game_menu.rs)
- an offline Studio, Play: InspectPlayerFromUserId opens Studio's InspectAndBuy panel
  even after SetInspectMenuEnabled(false); MenuOpened and MenuIsOpen true at
  the call; CloseInspectMenu → MenuClosed; -5 → "GuiService:
  InspectPlayerFromUserId - Got invalid userId."; FromHumanoidDescription
  titles it "<name>'s Avatar". Layout (862 × 677 view): panel inset 31 / 92
  / 31 / 35 px, a 608 px column, title 208 px down, Buy items 120 × 40, item
  rows 100 px with 16 px gaps. rustudio lists the worn items (avatar API)
  with GetProductInfoAsync prices; there's no 3D avatar preview, and Buy
  items does nothing (no purchases).

## Left, and why

- Event timing: rustudio queues every event and dispatches after the call
  (engine-wide), Studio fires these synchronously (e.g. Focused before
  CaptureFocus returns). Same order, later.
- Handle adornments draw without depth testing; ArcHandles' rings
  themselves aren't hit targets (only their spheres).
- Inspect menu: no 3D avatar preview; Buy items is inert.
- ProximityPrompt IndicatorShown / Hidden: never fired in the Studio
  measured (gamepad / off-screen indicators).
- Touch*, Device*Changed, UserCFrameChanged / Enabled, NavigationRequested,
  ReturnPressedFromOnScreenKeyboard: no such device here, so (as Studio
  without one) never fired.
