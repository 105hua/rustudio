//! One icon per class for the Explorer, Insert Object and Properties: a
//! Tabler glyph (src/ti.rs) chosen for what the class is, coloured by its
//! family (parts grey-blue, scripts by kind, GUI pink, values violet, lights
//! amber, sound blue, physics teal...). Classes without their own entry take
//! their family's glyph through the class hierarchy (`doc::is_a`).

use egui::Color32;

const PART: Color32 = Color32::from_rgb(160, 178, 204);
const SERVICE: Color32 = Color32::from_rgb(110, 170, 255);
const SERVER: Color32 = Color32::from_rgb(70, 196, 120);
const CLIENT: Color32 = Color32::from_rgb(255, 196, 70);
const SCRIPT: Color32 = Color32::from_rgb(120, 205, 90);
const LOCAL: Color32 = Color32::from_rgb(90, 160, 255);
const MODULE: Color32 = Color32::from_rgb(190, 130, 255);
const GUI: Color32 = Color32::from_rgb(255, 110, 150);
const VALUE: Color32 = Color32::from_rgb(170, 140, 255);
const LIGHT: Color32 = Color32::from_rgb(255, 214, 90);
const SOUND: Color32 = Color32::from_rgb(80, 180, 255);
const PHYS: Color32 = Color32::from_rgb(70, 210, 200);
const ANIM: Color32 = Color32::from_rgb(255, 150, 70);
const FX: Color32 = Color32::from_rgb(255, 120, 220);
const NET: Color32 = Color32::from_rgb(255, 170, 60);
const AVATAR: Color32 = Color32::from_rgb(255, 160, 110);
const MODEL: Color32 = Color32::from_rgb(200, 200, 210);
const FOLDER: Color32 = Color32::from_rgb(232, 196, 100);
const MISC: Color32 = Color32::from_rgb(170, 175, 185);

/// Each class's own icon: (class, Tabler icon, colour).
const TABLE: &[(&str, &str, Color32)] = &[
    // Services
    ("Workspace", "world", SERVICE),
    ("Players", "users", Color32::from_rgb(120, 210, 140)),
    ("Lighting", "bulb", LIGHT),
    ("MaterialService", "texture", Color32::from_rgb(230, 150, 90)),
    ("ReplicatedFirst", "arrow-big-down-lines", SERVICE),
    ("ReplicatedStorage", "packages", SERVICE),
    ("ServerScriptService", "server-cog", SERVER),
    ("ServerStorage", "server-2", SERVER),
    ("StarterGui", "layout-dashboard", CLIENT),
    ("StarterPack", "backpack", CLIENT),
    ("StarterPlayer", "user-cog", CLIENT),
    ("StarterPlayerScripts", "user-code", CLIENT),
    ("StarterCharacterScripts", "user-bolt", CLIENT),
    ("Teams", "users-group", Color32::from_rgb(240, 120, 120)),
    ("SoundService", "volume", SOUND),
    ("TextChatService", "messages", Color32::from_rgb(200, 200, 220)),
    ("Chat", "message-circle", Color32::from_rgb(200, 200, 220)),
    ("LocalizationService", "language", MISC),
    ("TestService", "test-pipe", MISC),
    ("VoiceChatService", "microphone", SOUND),
    ("CollectionService", "tags", MISC),
    ("RunService", "run", MISC),
    ("TweenService", "wave-sine", ANIM),
    ("HttpService", "world-www", NET),
    ("DataStoreService", "database", NET),
    ("MarketplaceService", "shopping-cart", NET),
    ("PhysicsService", "atom", PHYS),
    ("PathfindingService", "route", PHYS),
    ("UserInputService", "keyboard", MISC),
    ("ContextActionService", "hand-click", MISC),
    ("GuiService", "app-window", GUI),
    ("Debris", "trash", MISC),
    ("InsertService", "file-import", MISC),
    ("BadgeService", "award", NET),
    ("TeleportService", "transfer", NET),
    ("MessagingService", "send", NET),
    ("AssetService", "photo", NET),
    ("ContentProvider", "cloud-download", NET),
    ("ProximityPromptService", "hand-finger", MISC),
    ("Terrain", "mountain", Color32::from_rgb(110, 190, 110)),
    ("Camera", "camera", MISC),
    // Parts
    ("Part", "cube", PART),
    ("MeshPart", "box-model-2", PART),
    ("WedgePart", "triangle-square-circle", PART),
    ("CornerWedgePart", "triangle", PART),
    ("TrussPart", "building-bridge-2", PART),
    ("SpawnLocation", "map-pin", Color32::from_rgb(120, 210, 140)),
    ("Seat", "armchair", PART),
    ("VehicleSeat", "steering-wheel", PART),
    ("UnionOperation", "squares-diagonal", PART),
    ("NegateOperation", "square-minus", Color32::from_rgb(240, 120, 120)),
    ("IntersectOperation", "squares-selected", PART),
    ("PartOperation", "squares", PART),
    ("SkateboardPlatform", "skateboarding", PART),
    // Containers
    ("Model", "rs-model", MODEL),
    ("Folder", "folder", FOLDER),
    ("Configuration", "settings", MISC),
    ("WorldModel", "world", MODEL),
    ("Actor", "cpu", MODULE),
    ("Tool", "tool", Color32::from_rgb(230, 170, 110)),
    ("HopperBin", "tool", Color32::from_rgb(230, 170, 110)),
    ("Accessory", "hanger", AVATAR),
    ("Hat", "crown", AVATAR),
    ("Backpack", "backpack", CLIENT),
    ("PlayerGui", "layout-dashboard", CLIENT),
    ("PlayerScripts", "user-code", CLIENT),
    ("Player", "user", Color32::from_rgb(120, 210, 140)),
    ("Team", "flag", Color32::from_rgb(240, 120, 120)),
    // Scripts
    ("Script", "file-code", SCRIPT),
    ("LocalScript", "file-code-2", LOCAL),
    ("ModuleScript", "file-function", MODULE),
    // Humanoid / avatar
    ("Humanoid", "user-circle", AVATAR),
    ("HumanoidDescription", "id-badge", AVATAR),
    ("Animator", "player-play", ANIM),
    ("AnimationController", "adjustments-horizontal", ANIM),
    ("Animation", "movie", ANIM),
    ("AnimationTrack", "timeline", ANIM),
    ("KeyframeSequence", "timeline-event", ANIM),
    ("Keyframe", "point", ANIM),
    ("Pose", "accessible", ANIM),
    ("BodyColors", "palette", AVATAR),
    ("Shirt", "shirt", AVATAR),
    ("Pants", "hanger-2", AVATAR),
    ("ShirtGraphic", "shirt-sport", AVATAR),
    ("CharacterMesh", "user-square", AVATAR),
    ("WrapLayer", "layers-intersect", AVATAR),
    ("WrapTarget", "layers-subtract", AVATAR),
    ("FaceControls", "mood-smile", AVATAR),
    // Meshes and surfaces
    ("SpecialMesh", "cylinder", PART),
    ("BlockMesh", "cube-3d-sphere", PART),
    ("CylinderMesh", "cylinder", PART),
    ("FileMesh", "file-3d", PART),
    ("Decal", "photo", Color32::from_rgb(120, 200, 255)),
    ("Texture", "texture", Color32::from_rgb(120, 200, 255)),
    ("SurfaceAppearance", "brush", Color32::from_rgb(120, 200, 255)),
    ("MaterialVariant", "palette", Color32::from_rgb(230, 150, 90)),
    ("TerrainDetail", "trees", Color32::from_rgb(110, 190, 110)),
    // Lights
    ("PointLight", "bulb", LIGHT),
    ("SpotLight", "lamp", LIGHT),
    ("SurfaceLight", "lamp-2", LIGHT),
    ("Sky", "cloud", Color32::from_rgb(140, 200, 255)),
    ("Atmosphere", "haze", Color32::from_rgb(140, 200, 255)),
    ("Clouds", "cloud-fog", Color32::from_rgb(200, 210, 230)),
    ("BloomEffect", "sun-high", FX),
    ("BlurEffect", "blur", FX),
    ("ColorCorrectionEffect", "color-filter", FX),
    ("ColorGradingEffect", "chart-histogram", FX),
    ("DepthOfFieldEffect", "focus-2", FX),
    ("SunRaysEffect", "sun", FX),
    // Effects
    ("ParticleEmitter", "sparkles", FX),
    ("Beam", "line", FX),
    ("Trail", "route-2", FX),
    ("Fire", "flame", Color32::from_rgb(255, 120, 60)),
    ("Smoke", "cloud-fog", MISC),
    ("Sparkles", "sparkles", LIGHT),
    ("Explosion", "bomb", Color32::from_rgb(255, 120, 60)),
    ("ForceField", "shield", Color32::from_rgb(120, 200, 255)),
    ("Highlight", "highlight", LIGHT),
    ("SelectionBox", "box-margin", Color32::from_rgb(25, 153, 255)),
    ("SelectionSphere", "circle-dashed", Color32::from_rgb(25, 153, 255)),
    ("BoxHandleAdornment", "box-padding", Color32::from_rgb(25, 153, 255)),
    ("ArcHandles", "rotate-360", Color32::from_rgb(25, 153, 255)),
    ("Handles", "arrows-move", Color32::from_rgb(25, 153, 255)),
    // Sound
    ("Sound", "music", SOUND),
    ("SoundGroup", "playlist", SOUND),
    ("ReverbSoundEffect", "wave-square", SOUND),
    ("EqualizerSoundEffect", "adjustments-alt", SOUND),
    ("CompressorSoundEffect", "arrows-minimize", SOUND),
    ("EchoSoundEffect", "repeat", SOUND),
    ("ChorusSoundEffect", "waveform", SOUND),
    ("DistortionSoundEffect", "wave-saw-tool", SOUND),
    ("PitchShiftSoundEffect", "chart-arrows-vertical", SOUND),
    ("TremoloSoundEffect", "activity", SOUND),
    ("FlangeSoundEffect", "infinity", SOUND),
    // Physics
    ("Attachment", "circle-dot", PHYS),
    ("Bone", "bone", PHYS),
    ("Weld", "link", PHYS),
    ("WeldConstraint", "link", PHYS),
    ("Motor6D", "engine", PHYS),
    ("Motor", "engine", PHYS),
    ("Snap", "magnet", PHYS),
    ("Glue", "droplet", PHYS),
    ("ManualWeld", "link", PHYS),
    ("BallSocketConstraint", "circle-dot", PHYS),
    ("HingeConstraint", "door", PHYS),
    ("PrismaticConstraint", "arrows-horizontal", PHYS),
    ("CylindricalConstraint", "cylinder", PHYS),
    ("RodConstraint", "minus", PHYS),
    ("RopeConstraint", "line-dashed", PHYS),
    ("SpringConstraint", "spiral", PHYS),
    ("AlignPosition", "focus-centered", PHYS),
    ("AlignOrientation", "compass", PHYS),
    ("LinearVelocity", "arrow-right", PHYS),
    ("AngularVelocity", "rotate-clockwise", PHYS),
    ("VectorForce", "arrow-up-right", PHYS),
    ("Torque", "rotate-2", PHYS),
    ("LineForce", "arrows-right", PHYS),
    ("NoCollisionConstraint", "ban", PHYS),
    ("PlaneConstraint", "square-rotated", PHYS),
    ("UniversalConstraint", "arrows-cross", PHYS),
    ("RigidConstraint", "lock", PHYS),
    ("BodyVelocity", "arrow-big-right", PHYS),
    ("BodyPosition", "target", PHYS),
    ("BodyGyro", "compass", PHYS),
    ("BodyForce", "arrow-bar-up", PHYS),
    ("BodyAngularVelocity", "rotate", PHYS),
    ("ClickDetector", "click", Color32::from_rgb(120, 200, 255)),
    ("DragDetector", "drag-drop", Color32::from_rgb(120, 200, 255)),
    ("ProximityPrompt", "hand-finger", Color32::from_rgb(120, 200, 255)),
    ("Dialog", "message-dots", Color32::from_rgb(120, 200, 255)),
    ("DialogChoice", "message-2", Color32::from_rgb(120, 200, 255)),
    // Values
    ("StringValue", "letter-case", VALUE),
    ("IntValue", "number", VALUE),
    ("NumberValue", "decimal", VALUE),
    ("BoolValue", "toggle-left", VALUE),
    ("ObjectValue", "pointer", VALUE),
    ("Vector3Value", "axis-x", VALUE),
    ("CFrameValue", "3d-cube-sphere", VALUE),
    ("Color3Value", "color-swatch", VALUE),
    ("BrickColorValue", "palette", VALUE),
    ("RayValue", "line", VALUE),
    ("IntConstrainedValue", "number", VALUE),
    ("DoubleConstrainedValue", "decimal", VALUE),
    ("LocalizationTable", "language", MISC),
    // Networking / events
    ("RemoteEvent", "bolt", NET),
    ("UnreliableRemoteEvent", "bolt-off", NET),
    ("RemoteFunction", "arrows-exchange", NET),
    ("BindableEvent", "bell", Color32::from_rgb(200, 160, 255)),
    ("BindableFunction", "arrows-right-left", Color32::from_rgb(200, 160, 255)),
    // GUI
    ("ScreenGui", "app-window", GUI),
    ("SurfaceGui", "device-desktop", GUI),
    ("BillboardGui", "badge", GUI),
    ("Frame", "square", GUI),
    ("ScrollingFrame", "layout-list", GUI),
    ("CanvasGroup", "stack-2", GUI),
    ("ViewportFrame", "box", GUI),
    ("VideoFrame", "video", GUI),
    ("TextLabel", "typography", GUI),
    ("TextButton", "click", GUI),
    ("TextBox", "forms", GUI),
    ("ImageLabel", "photo", GUI),
    ("ImageButton", "photo-check", GUI),
    ("UIListLayout", "list", Color32::from_rgb(160, 200, 255)),
    ("UIGridLayout", "layout-grid", Color32::from_rgb(160, 200, 255)),
    ("UIPageLayout", "files", Color32::from_rgb(160, 200, 255)),
    ("UITableLayout", "table", Color32::from_rgb(160, 200, 255)),
    ("UIFlexItem", "arrows-horizontal", Color32::from_rgb(160, 200, 255)),
    ("UIPadding", "box-padding", Color32::from_rgb(160, 200, 255)),
    ("UICorner", "border-corner-rounded", Color32::from_rgb(160, 200, 255)),
    ("UIStroke", "border-outer", Color32::from_rgb(160, 200, 255)),
    ("UIGradient", "gradienter", Color32::from_rgb(160, 200, 255)),
    ("UIScale", "arrows-maximize", Color32::from_rgb(160, 200, 255)),
    ("UIAspectRatioConstraint", "aspect-ratio", Color32::from_rgb(160, 200, 255)),
    ("UISizeConstraint", "dimensions", Color32::from_rgb(160, 200, 255)),
    ("UITextSizeConstraint", "text-size", Color32::from_rgb(160, 200, 255)),
    // Misc
    ("PackageLink", "package", MISC),
];

/// The icon and colour for `class`: its own entry, else its family's.
pub fn icon_for(class: &str) -> (&'static str, Color32) {
    if let Some((_, i, c)) = TABLE.iter().find(|(k, _, _)| *k == class) {
        return (i, *c);
    }
    let is = |b: &str| crate::doc::is_a(class, b);
    let family: &[(&str, &str, Color32)] = &[
        ("BasePart", "cube", PART),
        ("LuaSourceContainer", "file-code", SCRIPT),
        ("ValueBase", "variable", VALUE),
        ("GuiButton", "click", GUI),
        ("GuiObject", "square", GUI),
        ("LayerCollector", "app-window", GUI),
        ("UIComponent", "adjustments", Color32::from_rgb(160, 200, 255)),
        ("Light", "bulb", LIGHT),
        ("PostEffect", "color-filter", FX),
        ("Constraint", "link", PHYS),
        ("JointInstance", "link", PHYS),
        ("BodyMover", "arrow-big-right", PHYS),
        ("SoundEffect", "adjustments-alt", SOUND),
        ("DataModelMesh", "cylinder", PART),
        ("FaceInstance", "photo", Color32::from_rgb(120, 200, 255)),
        ("Clothing", "shirt", AVATAR),
        ("Accoutrement", "hanger", AVATAR),
        ("PVAdornment", "box-margin", Color32::from_rgb(25, 153, 255)),
        ("GuiBase3d", "box-margin", Color32::from_rgb(25, 153, 255)),
        ("PVInstance", "box", MODEL),
        ("ServiceProvider", "server", SERVICE),
    ];
    for (base, i, c) in family {
        if is(base) {
            return (i, *c);
        }
    }
    if crate::api::get().is_some_and(|a| a.classes.get(class).is_some_and(|k| k.service)) {
        return ("settings-2", SERVICE);
    }
    ("point", MISC)
}

#[cfg(test)]
mod tests {
    /// Every icon the table and the families name exists in the bundled set.
    #[test]
    fn icons_exist() {
        let missing: Vec<&str> = super::TABLE
            .iter()
            .map(|(_, i, _)| *i)
            .chain(["cube", "file-code", "variable", "click", "square", "app-window", "adjustments",
                "bulb", "color-filter", "link", "arrow-big-right", "adjustments-alt", "cylinder",
                "photo", "shirt", "hanger", "box-margin", "box", "server", "settings-2", "point"])
            .filter(|i| !crate::ti::has(i))
            .collect();
        assert!(missing.is_empty(), "not in Tabler: {missing:?}");
    }
}
