# UI parity with Studio (rounds 16–18)

The reference is an offline Studio (hidden, 1280×891), looked at over
its a Studio MCP bridge MCP: screenshots of the window, every menu popup and submenu,
every ribbon tab, the test drop-down, the account menu, the Explorer and its
context menus are in `checks/ui/studio/`; `list_actions` gave every item's
text and shortcut. rustudio's side is `rustudio PLACE --window 1280x891
--ui-shots DIR`, and `tools/ui_compare.py DIR` writes the pairs to
`checks/ui/side-*.png` (`checks/ui/index.md`). Names and icon art stay
rustudio's own (Tabler Icons, MIT, CREDITS.md); layout, words, order and
behaviour follow Studio.

Status: **done** = matches Studio; **partial** = as noted; **open** = not yet.

## Window chrome

| Item | Studio | rustudio | Status |
|---|---|---|---|
| Menu bar | File, Edit, View, Plugins, Test, Window, Help | the same, in that order (src/studio_ui.rs) | done |
| Account | round avatar at the top right; menu: Add Account, Log Out | the same; Add Account opens the sign-in window, Log Out drops the session; the status line on top | done |
| Publish | File > Publish (Alt+P), Publish As, Save to Roblox (As) | the same items, opening the publish window; Account / Publish left the ribbon | done |
| Collaborate, share, notifications | top right | shown, disabled with "not in rustudio yet" (Team Create is parked, TEAMCREATE.md) | done (disabled) |
| Ribbon tabs | Home, Model, Avatar, UI, Script, Plugins, + (centred) | the same, centred; + disabled | done |
| Title | place name in the window title | place name at the menu bar's right, and on the viewport's document tab | done |
| Document tab | the place's file name over the 3D view | the place's name (was "Viewport") | done |
| Panel headers | full-width bar, name centred, float / close at the right | full-width centred title with close; no float button | partial |
| Command bar | Script > Command Bar at the bottom (on by default) | not built: Window > Script > Command Bar disabled | open |
| Status bar | none in this Studio | rustudio keeps its status bar (selection, parts, camera, snap, fps; rebuild_check reads it) | different on purpose |
| Fonts | Builder Sans (ribbon, panels) | Builder Sans read from Studio's content folder at run time, else egui's | done |
| Animations | menus open at once | egui fades turned off | done |

## Menus (items, order, shortcuts)

Every item below is in Studio's order with Studio's shortcut; what rustudio
can't do is greyed, never left out. rustudio adds one item: File > Open
Folder… (round 15's project folders).

| Menu | Works in rustudio | Disabled (not yet) | Status |
|---|---|---|---|
| File | New, Open from File, Open from Roblox (account window), Open Folder…, Recent (real list, ~/.config/rustudio/recent.txt), Close Place (to a new place: rustudio has no start page), Import Roblox Model, Save to File (As), Save to Roblox (As), Publish (As), Avatar Settings, About rustudio, Exit | Import (3D importer), Export as .obj / glTF, Manage Audience, Experience Settings, Open Configs, Studio Settings, Beta Features, Customize Shortcuts, Open Auto Saves | done |
| Edit | Undo, Redo, Cut, Copy, Paste, Paste Into (Ctrl+Shift+V), Paste at Original Location and Paste into at Original Location (rustudio's paste keeps positions, so they equal Paste / Paste Into), Duplicate, Delete, Group as Model (Ctrl+G), Group as Folder (Ctrl+Alt+G), Ungroup, Lock Selection (Alt+L), Unlock All, Anchor Selection (Alt+A), Select All (Workspace's children) | Lock Tool, Anchor Tool, Convert to Package, Get Latest Package, Save as Default Instance, Format Document / Selection, Indent, Unindent | done |
| View | Zoom In / Out (UI zoom), Full Screen (F11), Show Grid (Alt+G) and 2 / 4 / 16 studs, Show UI, Screenshot (to ~/Pictures), Mute | Show View Selector, Show Wind Direction, Show Grid Material, Show Wireframe Rendering, Show Welds, Show Constraint Details, Expand / Collapse Script Folds, Toggle Comment | done |
| Plugins | — | Manage Plugins, Plugins Folder, Save as Local Plugin, Publish as Plugin (rustudio runs no Studio plugins; the person's own Dervex Tools / Place B Weathering entries are not listed) | done (disabled) |
| Test | Start Test Session > Test (F5), Test Here, Run (F8); Toggle Client/Server View; Stop (Shift+F5) | Server and Clients (F7), Team Test, Device Simulator, Controller / Player Emulator, Debug Error (Never shown checked), Reload Script, Show Client / Server Viewport, Add Clients, Pause, Pause Client/Server, End Session, Exit Client | done |
| Window | Properties, Explorer (Alt+X), Output toggles with checkmarks, Insert Object, Reset Layout | Toolbox, Activity History, Asset Manager, every 3D / Avatar / Collaboration / Debug / Localization / Performance Summary / Script entry (captured order) | done |
| Help | Documentation, Tutorials, Developer Forum (open in the browser) | Start Studio Tour, Open Logs Folder | done |
| Menu look | 233-px File menu; check column; shortcuts left-aligned in a column after the longest label | the same metrics (studio_actions.rs `menu_metrics`) | done |

## Ribbon

| Tab | Studio's buttons (in order) | rustudio | Status |
|---|---|---|---|
| Home | Select, Move, Scale, Rotate, Transform · Geometric, snap (☐ 1 studs, ☐ 45°) · Part, Terrain, Character, GUI, Script, Import · Material, Color, Group, Lock, Anchor · Explorer, Properties, Toolbox, Assets | all present in order. Working: the four tools, snap, Part (+ shape arrow), GUI (ScreenGui in StarterGui), Script (into the selection, else ServerScriptService), Import, Group, Lock, Anchor, Explorer, Properties. Disabled: Transform, Geometric, Terrain, Character, Material, Color, Toolbox, Assets | done |
| Model | tools, snap · Pivot, Reset · Part, Effect · Attachment, Constraint, Weld · Material, Color, Texture, Group, Lock, Anchor · Align (more past 1280 px not seen) | in order; Effect inserts ParticleEmitter / Beam / Trail / Fire / Smoke / Sparkles / Highlight into the selected parts, Attachment into each selected part, Weld joins two selected parts with a WeldConstraint; Pivot, Reset, Constraint, Material, Color, Texture, Align disabled | partial (the buttons right of Align weren't visible in the 1280-px reference) |
| Avatar | tools, snap · Settings · Character, Setup, Accessory, Adaptive…, UGC Validati… · Clip Editor, Graph Editor | Settings opens Avatar Settings; the rest disabled | done |
| UI | GUI, Frame, Label, Input, Appearance, Layout, Constraint · Style Editor | GUI works; Frame / Label / Input insert into a selected GUI object (greyed otherwise, as in Studio); Appearance / Layout / Constraint list their UI classes; Style Editor disabled | done |
| Script | Back, Fwd · Script · Format, Find, Go to Line · Command, Output · Breakpoints, Call Stack, Watch · Analysis, Script Activity | Script and Output work; the rest disabled | done |
| Plugins | the installed plugins' buttons | Manage Plugins, Plugins Folder (disabled), "rustudio doesn't run Studio plugins yet" | done |
| Ribbon icons | Studio's own art | Tabler glyphs, one family, active tool highlighted | done (own art) |

## Test controls

| Item | Studio | rustudio | Status |
|---|---|---|---|
| Run-type drop-down | Test, Test Here, Run, Server & Clients | the same; Server & Clients disabled (needs several clients) | done |
| Play button | starts the selected type | starts the selected type (self-test `run-type-dropdown`) | done |
| Test Here | the character starts at the camera | first spawn at the camera's position | done |
| Pause | pauses the session | shown disabled | open |
| Stop | stops it | stops it | done |
| R15 / R6 | File > Avatar Settings (a panel; needs a published game in Studio) | Avatar Settings panel (File menu, Avatar tab > Settings), docked under the view: R15 / R6 / Player Choice, saved with the place as StarterPlayer.GameSettingsAvatar, undoable; the toolbar picker is gone | done |

## Camera keys

| Item | Studio (measured in an offline Studio) | rustudio | Status |
|---|---|---|---|
| Q / E | along the camera's own up axis: E moved the camera along CFrame.UpVector (0, 0.914, −0.406) with the camera pitched down | along the camera's up (camera.rs `fly`); self-test `camera-qe-relative` | done |
| WASD / Q / E anywhere | fly wherever the mouse is | yes, unless a text field / the script editor / a game TextBox has the keyboard, Insert Object or a rename is open, the game has the input (Play), or Ctrl is held; self-test `camera-keys-anywhere` | done |

## Explorer

| Item | Studio | rustudio | Status |
|---|---|---|---|
| Order | ReflectionMetadata ExplorerOrder, then name (seen: Camera, Terrain, SpawnLocation, then parts A–Z) | the same, from Studio's ReflectionMetadata.xml read at run time | done |
| Start state | every service collapsed | the same (Workspace no longer opens expanded) | done |
| Search | "Search" field | "Search" field (name or class) | done |
| Drag and drop | above / into / below with an indicator, several at once, auto-scroll, auto-expand | indicator line (above / below) or box (into), the selection drags together, scrolls at the list's edges, a collapsed row expands after 0.6 s; above / below put the objects in that row's parent (siblings are sorted, as in Studio) | done |
| Row "+" | a + just right of the hovered row's name opens Insert Object for that object | the same, placed after the name (round 17; self-tests `explorer-plus-insert`, `explorer-plus-name`) | done |
| Insert Object popup | search field, classes by category, icons | the same: Studio's ReflectionMetadata ClassCategory groups, search, Enter inserts the first match | done |
| Shortcuts | F2, Ctrl+D, Del, Ctrl+G, Ctrl+U, Ctrl+Shift+V, arrows, Shift-click | all; arrows when the Explorer has the keyboard (last click in it): Up / Down move (Shift extends), Left collapses or goes to the parent, Right expands or goes to the first child | done |
| Ctrl+Shift+V | Paste Into | works from a real keyboard (egui-winit sends it as a Paste event with Shift; copy now puts a line on the system clipboard, without which egui-winit sent no Ctrl+V at all) — self-test `paste-into-shortcut` | done |
| Context menu (part) | Cut, Copy, Copy As ▸ · Duplicate, Delete, Rename · Group as Model, Group as Folder, Solid Modeling ▸, Insert ▸, Hierarchy ▸ · Zoom to, Show Orientation Indicator · Convert to Package, Save / Export ▸ · Help | the same order; Copy As, Solid Modeling, Show Orientation Indicator, Convert to Package disabled; Insert ▸ Insert Part / Insert Object… / Import Roblox Model; Hierarchy ▸ Select Children; Save / Export ▸ Save to File…; Help opens the class's docs | done |
| Context menu (service) | Copy As, Rename, Insert, Script Sync, Save / Export, Show Services…, Hide Service, Help | uses the part menu | open |
| Class icons | one per class | one Tabler glyph per class (about 230 classes listed, the rest by family) | partial (classes outside the list share their family's icon) |

## 3D view (round 17)

| Item | Studio (measured in an offline Studio) | rustudio | Status |
|---|---|---|---|
| Drag a part | with Select or Move, press on a part and drag: it slides over the surfaces under the cursor (a 4-stud cube from (−12, 2, −16) to (−9.33, 2, −1.98) with snap off, (−7, 2, −2) with 1-stud snap), lands on top of a part it is dragged onto, an unselected part is selected first, the selection moves together, one Undo puts it back | the same (src/part_drag.rs): the grab point follows the surface under the cursor (else the ground plane), the part rests on it by its half-extent along the surface normal, the axes along the surface snap to the Move increment, Locked parts stay, one "Drag" undo step; self-test `part-drag` | done |
| Box selection |  | the same (src/viewport.rs `marquee_pick`); self-test `box-select-live` | done |

## Playtests (round 17)

| Item | Studio | rustudio | Status |
|---|---|---|---|
| "/ to chat" hint | none at the bottom left of this Studio's playtest | removed (play_game.rs); self-test `play-no-chat-hint` | done |

## Properties (round 17)

Captured: Studio's Properties for a Part, a Model, a Script, a ScreenGui and
Lighting, scrolled top to bottom (`checks/ui/studio/props-*.png`), next to
rustudio's in `checks/ui/side-props-*.png`. The row model is checked against
all five by the unit test `props_layout::matches_studio_captures`, and the
Part's first 25 rows by self-test `properties-studio-order`.

| Item | Studio | rustudio | Status |
|---|---|---|---|
| Header | "Properties - Class "Name"" | the same, in the panel title | done |
| Filter | "Filter Properties (Ctrl+Shift+P)", always shown | the same; Ctrl+Shift+P focuses it; filters row names | done |
| Categories and order | Appearance, Data, Transform, Pivot, Behavior, Collision, Part, Assembly, Surface Inputs, Surface, the rest A–Z, Permissions, then Tags and Attributes | the same (props_layout.rs). From Studio's data: each property's category (API dump), hidden ones (Hidden / NotBrowsable / Deprecated, ReflectionMetadata Browsable), the order within a category (ReflectionMetadata PropertyOrder, then A–Z ignoring case). From the captures: the category order, Archivable under Data, the rows left out (Rotation, Disabled and Source of scripts, PredictionMode, Capabilities, Sandboxed, ModelStreamingMode…), CanQuery hidden while CanCollide is on, Permissions only for Models and scripts, Surface collapsed | done |
| Transform / Pivot rows | Size; CFrame, Origin, PivotOffset, WorldPivot as fold rows with Position / Orientation under them | the same, editable | done |
| Layout | two columns, divider at 48 %, rows 21 px + 1 px line, category bands 25 px, names indented 25 px | the same metrics | done |
| Colours | text (188, 190, 200), read-only (73, 77, 90), bands / lines (32, 34, 39), background (25, 26, 31), unticked box (40, 41, 49), ticked white box with a dark check | the same | done |
| Value editors | checkboxes; colour swatch + "[r, g, b]" (and the BrickColor name); BrickColor palette, Select Color dialog and sliders (round 18, below); enum drop-downs; vectors "x, y, z" that fold open to X / Y / Z; read-only rows greyed | the same | done |
| Worked-out rows | Mass 6.72 (4 × 1.2 × 2 × 0.7), AssemblyMass inf when anchored, AssemblyRootPart, AssemblyCenterOfMass; Current / CustomPhysicalProperties as ticks; a ScreenGui's AbsoluteSize = the view's size; every instance has a UniqueId | the same (Custom starts from Plastic's values when ticked); a UniqueId not saved yet shows the instance's session id | done |
| Tags / Attributes | own sections with +, "No Tags have been added yet" / "No Attribute has been added yet" | the same, with + to add and × to remove | done |
| Panel width | 407 px at 1280 px | 298 px (rustudio's dock; the fidelity harness's viewport size depends on it), so long names clip earlier | different on purpose |
| Model "Scale" reset arrow | a ↶ reset button at the row's right | not drawn | open |

## Properties editors (round 18)

Captured in an offline Studio by opening each editor on a Part
(`checks/ui/studio/brickcolor-popup.png`, `color-dialog.png`,
`color-dialog-picked.png`, `props-slider.png`, `props-color-row.png`);
pairs in `checks/ui/side-brickcolor-popup.png`, `side-color-dialog.png`,
`side-props-slider.png` (`tools/ui_compare.py … --editors DIR`, from
`RUSTUDIO_UI_SHOTS=editors`). Self-tests `brickcolor-palette`,
`color-dialog`, `property-slider`.

| Item | Studio | rustudio | Status |
|---|---|---|---|
| BrickColor editor | clicking the value opens `ColorPickerPopup` (368 × 400): a honeycomb of pointy-top hexagons (26 × 29, 27.75 px apart, rows 24 px apart, each row offset half a cell), rows of 7, 8 … 13 … 8, 7 holding `BrickColor.palette(0 … 126)` in order (read from Studio and matched cell by cell to the capture), a row of 12 greys (1003, 199, 194, 1002, 325, 348, 26, 302, 311, 320, 335, 1001), a "More Colors" button; the current colour ringed red outside yellow; each cell edged at about 0.64 × its colour | the same layout, colours and order (`src/color_editors.rs`); click sets the BrickColor (a part's Color follows) in one undo step; Esc or a click outside closes; More Colors opens Select Color | done |
| Hovered colour's name | the cells carry Studio's BrickColor names (Studio.App.RobloxCustomWidgets.BrickColor<n>); no visible hover change reached the hidden Studio's synthetic pointer, so the tooltip wasn't captured | the hovered cell gets a white ring and its name beside the pointer | done (look of the name not captured) |
| Color editor | clicking the row shows a swatch button and "r, g, b" text; the swatch opens Qt's `QColorDialog` "Select Color" (522 × 399): 48 basic colours (18 × 14 cells on a 28 × 24 pitch, Qt's fixed set), Pick Screen Color, 16 custom colours, Add to Custom Colors, the 220 × 200 hue × saturation field (value 200, hue falling left to right) with a black cross, the 13 × 200 value strip with its arrow, one 58 × 106 swatch of the current colour, Hue / Sat / Val and Red / Green / Blue spin boxes (43 × 21), Hex (93 × 21), OK / Cancel (75 × 25) | the same layout and sizes; the field, strip, cells, spin boxes (type, arrows, wheel), Hex and Pick Screen Color (click anywhere in rustudio) all edit; a click in the field at the same point gave Studio's colour exactly (109, 32, 165) | done |
| Before / after swatch | Studio's dialog has one swatch (the current colour) | one swatch, as Studio | as Studio (no before / after) |
| Live editing | the part recolours while the dialog is open; OK keeps it as one undo step (two field clicks, OK, one Undo → the old colour); Cancel puts the old colour back | the same | done |
| Typing a colour | the row's "r, g, b" text | the same | done |
| Slider rows | a property gets a slider when ReflectionMetadata gives UIMinimum and UIMaximum (303 members: Transparency, Reflectance, Lighting Brightness / ClockTime / ShadowSoftness / ExposureCompensation, Sound Volume / PlaybackSpeed, ParticleEmitter Rate / Speed / Drag…); step = range / UINumTicks, 20 steps without it (measured: Transparency 0.05, ParticleEmitter.Drag 0–5 in 0.25, ShadowSoftness 100 ticks in 0.01); SliderScaling "Square" | the same rules, read from Studio's ReflectionMetadata.xml (`src/prop_slider.rs`); Square spaces values quadratically | done |
| Slider look | on the picked row: a 65-px spin box (value, up / down arrows) then the slider (9 × 14 handle (53, 55, 65), 2-px groove), on (40, 41, 49); the name cell in the selection blue (19, 35, 93) | the same; shown on hover too (the person's request; Studio shows it once the row is clicked) | done |
| Slider editing | dragging sets the value live; arrows step by one tick; typing in the spin box | the same; a drag is one undo step | done |

## Output (round 17)

Captured: Studio's Output after a script that prints, warns and errors, with
and without timestamps, in Run and in Play (`checks/ui/studio/output*.png`);
pair in `checks/ui/side-output.png`.

| Item | Studio | rustudio | Status |
|---|---|---|---|
| Colours | by message type from the theme: Log MainText (188, 190, 200); Information InfoText (143, 180, 255); Warning (245, 118, 48) and Error (231, 87, 80) DemiBold; System SensitiveText (223, 106, 247) | the same, Builder Mono (Bold: no semi-bold face ships) | done |
| Line form | "message - Script:line"; errors "Full.Name:3: boom - Script:3", then "Stack Begin", one "Script 'Full.Name', Line N - function f" (or without " - function") per Luau frame, "Stack End", in blue | the same, every Luau frame from `debug.traceback` | done |
| Timestamps | "HH:MM:SS.mmm  " local time, off by default (⋯ > Show Timestamp) | the same | done |
| Context bar | green (0, 204, 103) for server lines (Run included), blue (52, 154, 213) for client lines; none on stack lines | the same | done |
| Toolbar | All Messages, All Contexts, Filter…, clear, ⋯ | the same; ⋯ holds Show Timestamp / Show Context / Show Source | done |
| Line pitch | 15 px | 15 px | done |
