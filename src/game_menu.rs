//! The in-game menu (Esc while playing): Roblox's SettingsHub, drawn with
//! the CoreScripts' own layout and theme values (Studio's
//! ExtraContent/scripts/CoreScripts/Modules/Settings: SettingsHub.lua,
//! SettingsPageFactory.lua, Utility.lua, Theme.lua, Pages/*, and UIBlox's
//! DarkTheme / Colors). Open, it frees a locked cursor (SettingsHub pushes
//! OverrideMouseIconBehavior.ForceShow and a Modal button) and keeps the
//! game's input (RbxSettingsHubStopCharacter sinks movement), while
//! UserInputService.MouseBehavior keeps whatever the game set.

use egui::{Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::*;

/// The hub's tabs, in SettingsHub's AddPage order, and its confirmation
/// pages (ResetCharacter / LeaveGame).
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuPage {
    #[default]
    People,
    Settings,
    Captures,
    Report,
    Help,
    ConfirmReset,
    ConfirmLeave,
}

const TABS: [(MenuPage, &str, &str); 5] = [
    (MenuPage::People, "People", "users"),
    (MenuPage::Settings, "Settings", "settings"),
    (MenuPage::Captures, "Captures", "camera"),
    (MenuPage::Report, "Report", "flag"),
    (MenuPage::Help, "Help", "help-circle"),
];

/// The menu's own state for a play session.
#[derive(Default)]
pub struct MenuState {
    pub page: MenuPage,
    /// The tab a confirmation page was opened from (Esc / Don't goes back:
    /// SettingsHub's MenuStack).
    back: Option<MenuPage>,
    /// When it last opened or closed (egui time), for the shield tween.
    changed_at: Option<f64>,
    /// Scroll of the page view, per tab.
    scroll: [f32; 5],
    /// Where each button was drawn last frame (self-test clicks them).
    pub rects: HashMap<&'static str, Rect>,
    /// Client Luau to run next tick (UserGameSettings changes).
    pub lua: Vec<String>,
    /// The Volume slider's level before the first change (MasterVolume × 10).
    volume: Option<u32>,
    /// The view has let go of the cursor for this opening of the menu
    /// (viewport.rs: free_cursor_for_menu); false again once it closes.
    pub cursor_freed: bool,
}

// ---------------------------------------------------------------- theme
// Theme.lua + UIBlox DarkTheme / Colors.
/// Overlay: Black at 0.5 transparency (DarkenBackground).
const DARKEN_ALPHA: f32 = 0.5;
/// MenuContainer → BackgroundUIContrast: Black, transparency 0.3.
const CONTAINER_ALPHA: f32 = 0.7;
/// Theme.MenuContainerCornerRadius / DefaultCornerRadius.
const CONTAINER_RADIUS: f32 = 10.0;
const RADIUS: f32 = 8.0;
/// Theme.HubPadding (desktop): left/right 20, bottom 14, top 0.
const PAD_X: f32 = 20.0;
const PAD_BOTTOM: f32 = 14.0;
/// HubBar 800 × 60 (desktop), BottomButtonFrame 80 tall.
const HUB_W: f32 = 800.0;
const BAR_H: f32 = 60.0;
const BOTTOM_H: f32 = 80.0;
/// addBottomBarButtonOld: 260 × LargeButtonHeight 48, 12 apart.
const BUTTON: Vec2 = Vec2::new(260.0, 48.0);
const BUTTON_GAP: f32 = 12.0;
/// IGM_ButtonHover (56, 57, 59).
const HOVER: Color32 = Color32::from_rgb(56, 57, 59);
/// SecondaryDefault / SecondaryContent: White at 0.3 transparency.
const SECONDARY_A: f32 = 0.7;
/// Divider: White at 0.8 transparency.
const DIVIDER_A: f32 = 0.2;
/// Colors.Slate (BackgroundDefault: player rows), Flint (BackgroundUIDefault).
const SLATE: Color32 = Color32::from_rgb(35, 37, 39);
const FLINT: Color32 = Color32::from_rgb(57, 59, 61);
/// IGM_Selected (217, 217, 217): slider steps up to the value.
const SELECTED: Color32 = Color32::from_rgb(217, 217, 217);
/// Utility.lua's ARROW_COLOR / ARROW_COLOR_HOVER.
const ARROW: Color32 = Color32::from_rgb(204, 204, 204);
/// nominalSizeFactor: Roblox → Builder Sans text sizes.
const NOMINAL: f32 = 0.794;
/// SettingsHub's shield tween: up from 36 px below the screen in 0.5 s
/// (Quart InOut), down in 0.4 s (Quad In); the darkening fades with it.
const OPEN_S: f64 = 0.5;
const CLOSE_S: f64 = 0.4;

const ESC_ICON: &str = "rbxasset://textures/ui/Settings/Help/EscapeIcon.png";
const LEAVE_ICON: &str = "rbxasset://textures/ui/Settings/Help/LeaveIcon.png";
const RESET_ICON: &str = "rbxasset://textures/ui/Settings/Help/ResetIcon.png";
const LESS: &str = "rbxasset://textures/ui/Settings/Slider/Less.png";
const MORE: &str = "rbxasset://textures/ui/Settings/Slider/More.png";
const LEFT: &str = "rbxasset://textures/ui/Settings/Slider/Left.png";
const RIGHT: &str = "rbxasset://textures/ui/Settings/Slider/Right.png";

#[derive(Clone, Copy)]
enum Weight {
    Regular,
    Medium,
    Bold,
}

/// Builder Sans at Roblox TextSize `px` (a line is `px` tall, as gui_walk sizes it).
fn font(ctx: &egui::Context, px: f32, w: Weight) -> FontId {
    let fam = match w {
        Weight::Regular => FontFamily::Proportional,
        Weight::Medium => FontFamily::Name("medium".into()),
        Weight::Bold => FontFamily::Name("bold".into()),
    };
    let line = ctx.fonts_mut(|f| f.row_height(&FontId::new(100.0, fam.clone()))) / 100.0;
    FontId::new(px * mech::GUI_FONT_EM_PER_TEXT_SIZE / line.max(0.5), fam)
}

fn white(a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn quart_in_out(t: f32) -> f32 {
    if t < 0.5 {
        8.0 * t.powi(4)
    } else {
        1.0 - (-2.0 * t + 2.0).powi(4) / 2.0
    }
}

/// GameSettings.lua's Camera Sensitivity slider ↔ MouseSensitivity: the
/// curve through (0, 0.2), (5, 1), (10, 4).
fn sensitivity_to_step(s: f64) -> u32 {
    if s <= 0.2 {
        return 0;
    }
    let x = if s <= 1.0 { (s - 0.2) / 0.16 + 0.5 } else { (s + 2.0) / 0.6 + 0.5 };
    (x.floor() as i64).clamp(0, 10) as u32
}

fn step_to_sensitivity(x: u32) -> f64 {
    let x = x as f64;
    let y = if x <= 5.0 { 0.16 * x + 0.2 } else { 0.6 * x - 2.0 };
    y.max(0.2)
}

/// What a frame of the menu asked for.
#[derive(Default)]
struct Asked {
    page: Option<MenuPage>,
    close: bool,
    reset: bool,
    leave: bool,
    sensitivity: Option<u32>,
    volume: Option<u32>,
    inverted: Option<bool>,
    fullscreen: Option<bool>,
}

impl Editor {
    /// Esc during Play opens (and closes) the in-game menu, as in Roblox; a
    /// focused TextBox or the chat box take Esc first. On a confirmation
    /// page Esc goes back to the tab it came from (SettingsHub:PopMenu).
    pub fn toggle_game_menu(&mut self) {
        if self.gui.focus.is_some() {
            return;
        }
        // Esc closes the inspect menu first (its back arrow does too).
        {
            let mut sh = crate::gui::share();
            if sh.inspect.is_some() {
                sh.inspect = None;
                sh.inspect_closed = true;
                return;
            }
        }
        let now = self.menu_clock();
        if let Some(s) = self.play.as_mut().filter(|s| s.mode == RunMode::Play && !s.game.chat_open) {
            let g = &mut s.game;
            if g.menu_open {
                if let Some(back) = g.menu.back.take() {
                    g.menu.page = back;
                    return;
                }
            }
            g.menu_open = !g.menu_open;
            g.menu_events.push(g.menu_open);
            g.menu.changed_at = Some(now);
            if g.menu_open {
                // SettingsHub opens on its first tab (People).
                g.menu.page = MenuPage::People;
                g.menu.back = None;
            }
        }
    }

    /// Keys while the menu is open: R and L open Reset Character / Leave
    /// (the bottom bar's hotkeys, bound only while it shows). True when the
    /// menu has the keyboard: the game hears none of these keys.
    pub fn game_menu_keys(&mut self, taps: &[egui::Key]) -> bool {
        let Some(g) = self.play.as_mut().map(|s| &mut s.game).filter(|g| g.menu_open) else {
            return false;
        };
        let on_tab = g.menu.back.is_none();
        for k in taps {
            match k {
                egui::Key::R if on_tab => {
                    g.menu.back = Some(g.menu.page);
                    g.menu.page = MenuPage::ConfirmReset;
                }
                egui::Key::L if on_tab => {
                    g.menu.back = Some(g.menu.page);
                    g.menu.page = MenuPage::ConfirmLeave;
                }
                _ => {}
            }
        }
        true
    }

    /// FILE.cursor.json beside a harness shot: the menu and the cursor grab
    /// (what the pause-menu captures check).
    pub fn write_cursor_state(&self, shot: &std::path::Path) {
        let s = self.play.as_ref();
        let body = serde_json::json!({
            "menu_open": s.is_some_and(|s| s.game.menu_open),
            "page": s.map(|s| format!("{:?}", s.game.menu.page)),
            "mouse_behavior": s.map(|s| s.mouse_behavior),
            "grabbed": self.game_grab.is_some(),
            "cursor_hidden": self.game_cursor_hidden,
            "cursor_log": self.cursor_log,
        });
        let _ = std::fs::write(shot.with_extension("cursor.json"), body.to_string());
    }

    /// GuiService's inspect menu (InspectPlayerFromUserId / …FromHumanoid-
    /// Description), laid out as Studio's InspectAndBuy WideView measured in
    /// an offline Studio (862 × 677 view): a dark rounded panel 31 px in from the
    /// sides, 92 px from the top and 35 from the bottom; a back arrow at
    /// (28, 28) in it; a 608 px column centred with "<name>'s Avatar" (bold
    /// 20) 208 px down, the Robux total under it, "Buy items" (120 × 40) on
    /// the right; then a 100 px row per item, 16 px apart (placeholder rows
    /// while loading). Buying is not wired: it never spends Robux here.
    fn inspect_menu(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let Some(view) = crate::gui::share().inspect.clone() else { return };
        let ctx = ui.ctx().clone();
        let painter = ui.painter_at(rect);
        let panel = Rect::from_min_max(
            Pos2::new(rect.left() + 31.0, rect.top() + 92.0),
            Pos2::new(rect.right() - 31.0, rect.bottom() - 35.0),
        );
        painter.rect_filled(panel, CornerRadius::same(8), Color32::from_rgba_unmultiplied(17, 18, 21, 217));
        // Back arrow.
        let a = Pos2::new(panel.left() + 28.0, panel.top() + 28.0);
        painter.line_segment([a + Vec2::new(4.0, -7.0), a + Vec2::new(-3.0, 0.0)], Stroke::new(2.0, white(1.0)));
        painter.line_segment([a + Vec2::new(-3.0, 0.0), a + Vec2::new(4.0, 7.0)], Stroke::new(2.0, white(1.0)));
        let back = Rect::from_center_size(a, Vec2::splat(32.0));
        let resp = ui.interact(back, ui.id().with("inspect_back"), Sense::click());
        if resp.clicked() {
            let mut sh = crate::gui::share();
            sh.inspect = None;
            sh.inspect_closed = true;
            return;
        }
        // Wheel scrolls the list.
        let hovered = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| panel.contains(p));
        let wheel = if hovered { ctx.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
        let x0 = panel.center().x - 304.0;
        let rows = view.items.len().max(if view.loading { 3 } else { 0 });
        let content_h = 257.0 + rows as f32 * 116.0;
        let max_scroll = (content_h - panel.height()).max(0.0);
        let scroll = (view.scroll - wheel).clamp(0.0, max_scroll);
        if scroll != view.scroll {
            if let Some(v) = crate::gui::share().inspect.as_mut() {
                v.scroll = scroll;
            }
        }
        let clip = painter.with_clip_rect(panel.shrink(4.0));
        let top = panel.top() - scroll;
        clip.text(
            Pos2::new(x0, top + 208.0),
            egui::Align2::LEFT_CENTER,
            &view.title,
            font(&ctx, 20.0, Weight::Bold),
            white(1.0),
        );
        clip.text(
            Pos2::new(x0 + 2.0, top + 231.0),
            egui::Align2::LEFT_CENTER,
            format!("\u{23E3}{}", view.robux),
            font(&ctx, 16.0, Weight::Regular),
            white(0.85),
        );
        let buy = Rect::from_min_size(Pos2::new(x0 + 488.0, top + 199.0), Vec2::new(120.0, 40.0));
        clip.rect_filled(buy, CornerRadius::same(8), Color32::from_rgb(44, 64, 140));
        clip.text(buy.center(), egui::Align2::CENTER_CENTER, "Buy items", font(&ctx, 14.0, Weight::Medium), white(0.7));
        for i in 0..rows {
            let r = Rect::from_min_size(Pos2::new(x0, top + 257.0 + i as f32 * 116.0), Vec2::new(608.0, 100.0));
            clip.rect_filled(r, CornerRadius::ZERO, white(0.08));
            if let Some((name, price)) = view.items.get(i) {
                clip.text(
                    Pos2::new(r.left() + 16.0, r.center().y - 10.0),
                    egui::Align2::LEFT_CENTER,
                    name,
                    font(&ctx, 16.0, Weight::Medium),
                    white(1.0),
                );
                let p = match price {
                    Some(p) => format!("\u{23E3}{p}"),
                    None => "Off sale".to_string(),
                };
                clip.text(
                    Pos2::new(r.left() + 16.0, r.center().y + 12.0),
                    egui::Align2::LEFT_CENTER,
                    p,
                    font(&ctx, 14.0, Weight::Regular),
                    white(0.7),
                );
            }
        }
    }

    fn menu_clock(&self) -> f64 {
        self.play.as_ref().map(|s| s.time).unwrap_or(0.0)
    }

    /// The menu over the game view, sliding in and out.
    pub(super) fn game_menu(&mut self, ui: &mut egui::Ui, rect: Rect) {
        self.inspect_menu(ui, rect);
        let now = self.menu_clock();
        let Some((open, changed)) = self
            .play
            .as_ref()
            .map(|s| (s.game.menu_open, s.game.menu.changed_at))
        else {
            return;
        };
        let since = changed.map(|t| (now - t).max(0.0)).unwrap_or(f64::MAX);
        if !open && since >= CLOSE_S {
            return;
        }
        let ctx = ui.ctx().clone();
        // How far in it is: 0 hidden (below the screen), 1 in place.
        let k = if open {
            quart_in_out((since / OPEN_S).min(1.0) as f32)
        } else {
            let t = (since / CLOSE_S).min(1.0) as f32;
            1.0 - t * t
        };
        let dark = if open {
            let t = (since / OPEN_S).min(1.0) as f32;
            1.0 - (1.0 - t) * (1.0 - t)
        } else {
            let t = (since / CLOSE_S).min(1.0) as f32;
            (1.0 - t).powi(4)
        };
        if since < OPEN_S.max(CLOSE_S) {
            ctx.request_repaint();
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_black_alpha((DARKEN_ALPHA * dark * 255.0) as u8));
        let dy = (1.0 - k) * (rect.height() + 36.0);

        let page = self.play.as_ref().map(|s| s.game.menu.page).unwrap_or_default();
        let confirm = matches!(page, MenuPage::ConfirmReset | MenuPage::ConfirmLeave);
        // SettingsHub's sizes: the page view is as tall as the screen allows
        // (5% margins, the bars), at most 600 and at least 150.
        let hub_w = HUB_W.min(rect.width() - 2.0 * PAD_X - 24.0).max(320.0);
        let buffer = 0.05 * rect.height();
        let page_h = (rect.height() - 2.0 * buffer - BAR_H - BOTTOM_H).clamp(150.0, 600.0);
        let size = if confirm {
            // The confirmation page: its question (200 tall) and two 200×50 buttons.
            Vec2::new(hub_w + 2.0 * PAD_X, 250.0 + PAD_BOTTOM + 20.0)
        } else {
            Vec2::new(hub_w + 2.0 * PAD_X, BAR_H + page_h + BOTTOM_H + PAD_BOTTOM)
        };
        // Centred, 10 px low (Theme.MenuContainerPosition).
        let container = Rect::from_center_size(rect.center() + Vec2::new(0.0, 10.0 + dy), size);
        painter.rect_filled(
            container,
            CornerRadius::same(CONTAINER_RADIUS as u8),
            Color32::from_black_alpha((CONTAINER_ALPHA * 255.0) as u8),
        );
        // The menu takes the mouse over the whole view (its Modal shield):
        // nothing under it is hovered or clicked.
        let shield = ui.interact(rect, ui.id().with("game_menu_shield"), Sense::click());
        let _ = shield;
        let inner = Rect::from_min_max(
            container.min + Vec2::new(PAD_X, 0.0),
            container.max - Vec2::new(PAD_X, PAD_BOTTOM),
        );
        let mut asked = Asked::default();
        let mut rects: HashMap<&'static str, Rect> = HashMap::new();
        let interactive = open && k > 0.5;
        // (the self-test sweeps the pointer over these)
        rects.insert("container", container);
        if confirm {
            self.menu_confirm(ui, &painter, inner, page, interactive, &mut asked, &mut rects);
        } else {
            // HubBar: five tabs, 1/5 wide each, a divider underneath.
            let bar = Rect::from_min_size(inner.min, Vec2::new(inner.width(), BAR_H));
            self.menu_tabs(ui, &painter, bar, page, interactive, &mut asked);
            let view = Rect::from_min_size(
                Pos2::new(inner.min.x, bar.max.y + 1.0),
                Vec2::new(inner.width(), page_h - 1.0),
            );
            rects.insert("page", view);
            self.menu_page(ui, &painter, view, page, interactive, &mut asked, &mut rects);
            let bottom = Rect::from_min_size(
                Pos2::new(inner.min.x, view.max.y),
                Vec2::new(inner.width(), BOTTOM_H),
            );
            self.menu_bottom(ui, &painter, bottom, interactive, &mut asked, &mut rects);
        }
        self.menu_apply(&ctx, asked, rects);
    }

    fn menu_apply(&mut self, ctx: &egui::Context, asked: Asked, rects: HashMap<&'static str, Rect>) {
        let now = self.menu_clock();
        if let Some(on) = asked.fullscreen {
            if on != self.ui.fullscreen {
                self.ui.fullscreen = on;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
            }
        }
        let Some(s) = self.play.as_mut() else { return };
        let g = &mut s.game;
        g.menu.rects = rects;
        if let Some(p) = asked.page {
            match p {
                MenuPage::ConfirmReset | MenuPage::ConfirmLeave => {
                    g.menu.back = Some(g.menu.page);
                }
                _ => g.menu.back = None,
            }
            g.menu.page = p;
        }
        if let Some(x) = asked.sensitivity {
            g.menu.lua.push(format!(
                "UserSettings():GetService(\"UserGameSettings\").MouseSensitivity = {}",
                step_to_sensitivity(x)
            ));
        }
        if let Some(v) = asked.volume {
            g.menu.volume = Some(v);
            let f = v as f32 / 10.0;
            g.menu.lua.push(format!(
                "UserSettings():GetService(\"UserGameSettings\").MasterVolume = {f}"
            ));
            match g.audio.as_mut() {
                Some(a) => a.set_master(f),
                None => crate::audio::set_master_global(f),
            }
        }
        if let Some(on) = asked.inverted {
            g.menu.lua.push(format!(
                "UserSettings():GetService(\"UserGameSettings\").CameraYInverted = {on}"
            ));
        }
        if asked.close || asked.reset || asked.leave {
            g.menu_open = false;
            g.menu_events.push(false);
            g.menu.changed_at = Some(now);
            g.menu.back = None;
            g.reset_request |= asked.reset;
            g.leave_request |= asked.leave;
        }
    }

    /// A Utility:MakeStyledButton: clear, a 1 px White 0.3 stroke, 8 px
    /// corners, IGM_ButtonHover under the pointer.
    #[allow(clippy::too_many_arguments)]
    fn menu_button(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        r: Rect,
        id: &'static str,
        label: &str,
        hint: Option<&str>,
        text_px: f32,
        interactive: bool,
        rects: &mut HashMap<&'static str, Rect>,
    ) -> bool {
        let ctx = ui.ctx().clone();
        let resp = ui.interact(r, ui.id().with(("game_menu", id)), Sense::click());
        rects.insert(id, r);
        let hovered = interactive && resp.hovered();
        if hovered {
            painter.rect_filled(r, CornerRadius::same(RADIUS as u8), HOVER);
        }
        painter.rect_stroke(
            r,
            CornerRadius::same(RADIUS as u8),
            Stroke::new(1.0, white(SECONDARY_A)),
            egui::StrokeKind::Inside,
        );
        let mut text_rect = r;
        if let Some(icon) = hint {
            // The key hint: 33 × 33, 2.5% in from the left, tinted like the text.
            let pad = r.width() * 0.025;
            let ir = Rect::from_min_size(
                Pos2::new(r.min.x + pad, r.center().y - 16.5),
                Vec2::splat(33.0),
            );
            if let Some((tex, _)) = self.gui.texture(&ctx, &mut self.assets, None, icon) {
                painter.image(
                    tex,
                    ir,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    white(SECONDARY_A),
                );
            }
            text_rect.min.x = ir.max.x;
            text_rect.max.x -= pad;
        }
        painter.text(
            text_rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            font(&ctx, text_px, Weight::Medium),
            white(SECONDARY_A),
        );
        interactive && resp.clicked()
    }

    fn menu_tabs(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        bar: Rect,
        page: MenuPage,
        interactive: bool,
        asked: &mut Asked,
    ) {
        let ctx = ui.ctx().clone();
        let w = bar.width() / TABS.len() as f32;
        let title = font(&ctx, 24.0 * NOMINAL, Weight::Medium);
        for (n, (p, name, icon)) in TABS.iter().enumerate() {
            let r = Rect::from_min_size(bar.min + Vec2::new(w * n as f32, 0.0), Vec2::new(w, bar.height()));
            let resp = ui.interact(r, ui.id().with(("game_menu_tab", n)), Sense::click());
            if interactive && resp.clicked() && *p != page {
                asked.page = Some(*p);
            }
            let on = *p == page;
            // Unselected tabs are at half transparency; the open one has a
            // 2 px white bar along the bottom (TabSelection).
            let c = white(if on { 1.0 } else { 0.5 });
            let galley = painter.layout_no_wrap(name.to_string(), title.clone(), c);
            // Icon 36 × 2/3 = 24, 10 px, the title; centred together.
            let icon_s = 24.0;
            let total = icon_s + 10.0 + galley.size().x;
            let x0 = r.center().x - total / 2.0;
            crate::ti::paint(
                painter,
                Rect::from_min_size(Pos2::new(x0, r.center().y - icon_s / 2.0), Vec2::splat(icon_s)),
                icon,
                c,
            );
            painter.galley(
                Pos2::new(x0 + icon_s + 10.0, r.center().y - galley.size().y / 2.0 - 2.0),
                galley,
                c,
            );
            if on {
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(r.min.x + 3.0, r.max.y - 2.0),
                        Vec2::new(r.width() - 2.0, 2.0),
                    ),
                    0.0,
                    Color32::WHITE,
                );
            }
        }
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(bar.min.x, bar.max.y - 1.0), Vec2::new(bar.width(), 1.0)),
            0.0,
            white(DIVIDER_A),
        );
    }

    /// BottomButtonFrame: Reset Character (R), Leave (L), Resume (Esc).
    fn menu_bottom(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        bottom: Rect,
        interactive: bool,
        asked: &mut Asked,
        rects: &mut HashMap<&'static str, Rect>,
    ) {
        let bw = BUTTON.x.min((bottom.width() - 2.0 * BUTTON_GAP) / 3.0);
        let total = 3.0 * bw + 2.0 * BUTTON_GAP;
        let mut x = bottom.center().x - total / 2.0;
        let y = bottom.center().y - BUTTON.y / 2.0;
        // (addBottomBarButtonOld's text is Size24 under a Button-font
        // UITextSizeConstraint: 22 × 0.794.)
        let px = 22.0 * NOMINAL;
        for (id, label, icon) in [
            ("reset", "Reset Character", RESET_ICON),
            ("leave", "Leave", LEAVE_ICON),
            ("resume", "Resume", ESC_ICON),
        ] {
            let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(bw, BUTTON.y));
            if self.menu_button(ui, painter, r, id, label, Some(icon), px, interactive, rects) {
                match id {
                    "reset" => asked.page = Some(MenuPage::ConfirmReset),
                    "leave" => asked.page = Some(MenuPage::ConfirmLeave),
                    _ => asked.close = true,
                }
            }
            x += bw + BUTTON_GAP;
        }
    }

    /// ResetCharacter.lua / LeaveGame.lua: the question in Bold 28 over a
    /// 200 px band, then two 200 × 50 buttons 20 apart.
    #[allow(clippy::too_many_arguments)]
    fn menu_confirm(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        inner: Rect,
        page: MenuPage,
        interactive: bool,
        asked: &mut Asked,
        rects: &mut HashMap<&'static str, Rect>,
    ) {
        let ctx = ui.ctx().clone();
        let (question, yes, no) = if page == MenuPage::ConfirmReset {
            ("Are you sure you want to reset your character?", "Reset", "Don't Reset")
        } else {
            ("Are you sure you want to leave the experience?", "Leave", "Don't Leave")
        };
        let band = Rect::from_min_size(inner.min + Vec2::new(0.0, 10.0), Vec2::new(inner.width(), 200.0));
        let galley = painter.layout(
            question.to_string(),
            font(&ctx, 28.0, Weight::Bold),
            Color32::WHITE,
            band.width() - 40.0,
        );
        painter.galley(band.center() - galley.size() / 2.0, galley, Color32::WHITE);
        let y = band.max.y;
        let x = inner.center().x - 210.0;
        let a = Rect::from_min_size(Pos2::new(x, y), Vec2::new(200.0, 50.0));
        let b = Rect::from_min_size(Pos2::new(x + 220.0, y), Vec2::new(200.0, 50.0));
        let px = 22.0 * NOMINAL;
        if self.menu_button(ui, painter, a, "confirm", yes, None, px, interactive, rects) {
            if page == MenuPage::ConfirmReset {
                asked.reset = true;
            } else {
                asked.leave = true;
            }
        }
        if self.menu_button(ui, painter, b, "cancel", no, None, px, interactive, rects) {
            let back = self
                .play
                .as_ref()
                .and_then(|s| s.game.menu.back)
                .unwrap_or_default();
            asked.page = Some(back);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn menu_page(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        view: Rect,
        page: MenuPage,
        interactive: bool,
        asked: &mut Asked,
        rects: &mut HashMap<&'static str, Rect>,
    ) {
        let ctx = ui.ctx().clone();
        let tab = TABS.iter().position(|t| t.0 == page).unwrap_or(0);
        let scroll = self.play.as_ref().map(|s| s.game.menu.scroll[tab]).unwrap_or(0.0);
        let p = painter.with_clip_rect(view);
        // (5 px top padding: PageViewInnerFrame's UIPadding)
        let top = view.min + Vec2::new(0.0, 5.0 - scroll);
        let height = match page {
            MenuPage::People => self.menu_people(&ctx, &p, top, view.width()),
            MenuPage::Settings => self.menu_settings(ui, &p, view, top, interactive, asked),
            MenuPage::Captures => {
                let c = Pos2::new(view.center().x, top.y + 120.0);
                crate::ti::paint(&p, Rect::from_center_size(c, Vec2::splat(48.0)), "photo", white(0.5));
                p.text(
                    c + Vec2::new(0.0, 48.0),
                    egui::Align2::CENTER_CENTER,
                    "No captures yet",
                    font(&ctx, 18.0, Weight::Medium),
                    Color32::WHITE,
                );
                p.text(
                    c + Vec2::new(0.0, 76.0),
                    egui::Align2::CENTER_CENTER,
                    "Screenshots you take in this experience show up here.",
                    font(&ctx, 16.0 * NOMINAL, Weight::Regular),
                    white(0.7),
                );
                240.0
            }
            MenuPage::Report => self.menu_report(ui, &p, view, top, interactive, rects),
            MenuPage::Help => self.menu_help(&ctx, &p, top, view.width()),
            _ => 0.0,
        };
        // The wheel scrolls the page view (PageView, a ScrollingFrame).
        if interactive {
            let over = ctx.input(|i| i.pointer.latest_pos()).is_some_and(|q| view.contains(q));
            let wheel: f32 = if over {
                ctx.input(|i| {
                    i.raw
                        .events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                                egui::MouseWheelUnit::Line => delta.y * 40.0,
                                egui::MouseWheelUnit::Point => delta.y,
                                egui::MouseWheelUnit::Page => delta.y * view.height(),
                            }),
                            _ => None,
                        })
                        .sum()
                })
            } else {
                0.0
            };
            let max = (height + 10.0 - view.height()).max(0.0);
            if let Some(s) = self.play.as_mut() {
                let v = &mut s.game.menu.scroll[tab];
                *v = (*v - wheel).clamp(0.0, max);
            }
            if max > 0.0 {
                // DefaultScrollBarThickness 1.
                let frac = view.height() / (height + 10.0);
                let h = view.height() * frac;
                let y = view.min.y + (view.height() - h) * (scroll / max).clamp(0.0, 1.0);
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(view.max.x - 1.0, y), Vec2::new(1.0, h)),
                    0.0,
                    white(0.5),
                );
            }
        }
    }

    /// Players.lua: a 62 px row per player, 80 apart (Slate, 8 px corners),
    /// headshot 36 at 12,12, display name over @name from x = 60.
    fn menu_people(&self, ctx: &egui::Context, p: &egui::Painter, top: Pos2, w: f32) -> f32 {
        let players: Vec<(String, String)> = self
            .play
            .as_ref()
            .and_then(|s| {
                let d = if s.showing == Side::Client { Some(&self.doc) } else { s.other_doc.as_ref() }?;
                let ps = d.find_child(d.dom.root_ref(), "Players")?;
                Some(
                    d.dom
                        .get_by_ref(ps)?
                        .children()
                        .iter()
                        .filter_map(|c| d.dom.get_by_ref(*c))
                        .filter(|i| i.class == "Player")
                        .map(|i| {
                            let display = match i.properties.get(&rbx_dom_weak::ustr("DisplayName")) {
                                Some(Variant::String(t)) if !t.is_empty() => t.clone(),
                                _ => i.name.clone(),
                            };
                            (display, i.name.clone())
                        })
                        .collect(),
                )
            })
            .unwrap_or_default();
        let name_f = font(ctx, 18.0, Weight::Medium);
        let user_f = font(ctx, 14.0, Weight::Regular);
        for (n, (display, user)) in players.iter().enumerate() {
            let r = Rect::from_min_size(top + Vec2::new(0.0, 80.0 * n as f32), Vec2::new(w, 62.0));
            p.rect_filled(r, CornerRadius::same(RADIUS as u8), SLATE);
            let head = Rect::from_min_size(r.min + Vec2::new(12.0, 12.0), Vec2::splat(36.0));
            p.circle_filled(head.center(), 18.0, FLINT);
            p.text(
                head.center(),
                egui::Align2::CENTER_CENTER,
                display.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(),
                font(ctx, 18.0, Weight::Bold),
                Color32::WHITE,
            );
            p.text(
                Pos2::new(r.min.x + 60.0, r.center().y - 10.0),
                egui::Align2::LEFT_CENTER,
                display,
                name_f.clone(),
                Color32::WHITE,
            );
            p.text(
                Pos2::new(r.min.x + 60.0, r.center().y + 12.0),
                egui::Align2::LEFT_CENTER,
                format!("@{user}"),
                user_f.clone(),
                Color32::from_rgb(162, 162, 162),
            );
        }
        if players.is_empty() {
            p.text(
                top + Vec2::new(w / 2.0, 40.0),
                egui::Align2::CENTER_CENTER,
                "No one else is here",
                name_f,
                white(0.7),
            );
            return 80.0;
        }
        80.0 * players.len() as f32
    }

    /// GameSettings.lua rows (Utility AddNewRow, 50 tall): the label on the
    /// left 40%, the control on the right 60%.
    fn menu_settings(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        view: Rect,
        top: Pos2,
        interactive: bool,
        asked: &mut Asked,
    ) -> f32 {
        let ctx = ui.ctx().clone();
        let ugs = |name: &str| -> Option<mlua::Value> {
            let c = self.play.as_ref()?.client.as_ref()?;
            c.runtime
                .lua
                .globals()
                .get::<mlua::Table>("__rs_user_game_settings")
                .ok()?
                .get::<mlua::Value>(name)
                .ok()
        };
        let num = |v: Option<mlua::Value>| match v {
            Some(mlua::Value::Number(n)) => Some(n),
            Some(mlua::Value::Integer(n)) => Some(n as f64),
            _ => None,
        };
        let sens = num(ugs("MouseSensitivity")).unwrap_or(0.52);
        let vol = num(ugs("MasterVolume")).unwrap_or(crate::audio::master_volume() as f64);
        let inverted = matches!(ugs("CameraYInverted"), Some(mlua::Value::Boolean(true)));
        let full = self.ui.fullscreen;
        let label_f = font(&ctx, 16.0 * NOMINAL, Weight::Medium);
        let w = view.width();
        let row = |n: usize| Rect::from_min_size(top + Vec2::new(0.0, 50.0 * n as f32), Vec2::new(w, 50.0));
        let mut n = 0;
        let label = |p: &egui::Painter, n: usize, text: &str| {
            let r = row(n);
            p.text(
                Pos2::new(r.min.x + 10.0, r.center().y),
                egui::Align2::LEFT_CENTER,
                text,
                label_f.clone(),
                Color32::WHITE,
            );
            r
        };
        // Camera Sensitivity: 10 steps and the value box (60 wide, 20 apart).
        let r = label(p, n, "Camera Sensitivity");
        let step = sensitivity_to_step(sens);
        let slider = Rect::from_min_max(
            Pos2::new(r.max.x - 0.6 * w, r.min.y + 3.0),
            Pos2::new(r.max.x - 80.0, r.max.y - 3.0),
        );
        if let Some(x) = self.menu_slider(ui, p, slider, "sens", step, interactive) {
            asked.sensitivity = Some(x);
        }
        let tb = Rect::from_min_size(
            Pos2::new(slider.max.x + 10.0, r.center().y - 0.4 * 50.0),
            Vec2::new(60.0, 0.8 * 50.0),
        );
        p.rect_filled(tb, CornerRadius::same(RADIUS as u8), Color32::from_gray(51));
        p.text(
            tb.center(),
            egui::Align2::CENTER_CENTER,
            format!("{:.2}", sens),
            font(&ctx, 18.0 * NOMINAL, Weight::Regular),
            Color32::WHITE,
        );
        n += 1;
        // Volume: MasterVolume × 10 on 10 steps.
        let r = label(p, n, "Volume");
        let level = (vol * 10.0 + 0.5).floor().clamp(0.0, 10.0) as u32;
        let slider = Rect::from_min_max(Pos2::new(r.max.x - 0.6 * w, r.min.y), r.max);
        if let Some(x) = self.menu_slider(ui, p, slider, "vol", level, interactive) {
            asked.volume = Some(x);
        }
        n += 1;
        let r = label(p, n, "Camera Inverted");
        if let Some(i) = self.menu_selector(ui, p, r, w, "inv", &["Off", "On"], inverted as usize, interactive) {
            asked.inverted = Some(i == 1);
        }
        n += 1;
        let r = label(p, n, "Fullscreen");
        if let Some(i) = self.menu_selector(ui, p, r, w, "full", &["On", "Off"], if full { 0 } else { 1 }, interactive) {
            asked.fullscreen = Some(i == 0);
        }
        n += 1;
        50.0 * n as f32
    }

    /// Utility CreateNewSlider: Less / More arrows (32 wide) round `steps`
    /// bars 24 tall and 4 apart, IGM_Selected up to the value, Flint after;
    /// the end bars rounded. Returns a new value when clicked.
    fn menu_slider(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        r: Rect,
        id: &'static str,
        value: u32,
        interactive: bool,
    ) -> Option<u32> {
        const STEPS: u32 = 10;
        let ctx = ui.ctx().clone();
        let arrow_w = 32.0; // Theme.SelectorArrowButtonWidth
        let less = Rect::from_min_size(Pos2::new(r.min.x, r.min.y), Vec2::new(arrow_w, r.height()));
        let more = Rect::from_min_size(Pos2::new(r.max.x - arrow_w, r.min.y), Vec2::new(arrow_w, r.height()));
        let steps = Rect::from_center_size(r.center(), Vec2::new(r.width() - 100.0, r.height()));
        let mut out = None;
        for (ar, img, dir) in [(less, LESS, -1i32), (more, MORE, 1)] {
            let resp = ui.interact(ar, ui.id().with(("game_menu_slider", id, dir)), Sense::click());
            let c = if interactive && resp.hovered() { Color32::WHITE } else { ARROW };
            if let Some((tex, _)) = self.gui.texture(&ctx, &mut self.assets, None, img) {
                p.image(
                    tex,
                    Rect::from_center_size(ar.center(), Vec2::splat(30.0)),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    c,
                );
            }
            if interactive && resp.clicked() {
                out = Some((value as i32 + dir).clamp(0, STEPS as i32) as u32);
            }
        }
        let sw = steps.width() / STEPS as f32;
        let resp = ui.interact(steps, ui.id().with(("game_menu_steps", id)), Sense::click_and_drag());
        if interactive && (resp.clicked() || resp.dragged()) {
            if let Some(q) = resp.interact_pointer_pos() {
                let i = ((q.x - steps.min.x) / sw).floor() as i32 + 1;
                out = Some(i.clamp(1, STEPS as i32) as u32);
            }
        }
        let shown = out.unwrap_or(value);
        for i in 1..=STEPS {
            let b = Rect::from_min_size(
                Pos2::new(steps.min.x + (i - 1) as f32 * sw + 2.0, steps.center().y - 12.0),
                Vec2::new(sw - 4.0, 24.0),
            );
            let c = if i <= shown { SELECTED } else { FLINT };
            let cr = match i {
                1 => CornerRadius { nw: RADIUS as u8, sw: RADIUS as u8, ne: 0, se: 0 },
                STEPS => CornerRadius { nw: 0, sw: 0, ne: RADIUS as u8, se: RADIUS as u8 },
                _ => CornerRadius::ZERO,
            };
            p.rect_filled(b, cr, c);
        }
        out
    }

    /// Utility CreateSelector: the value between Left / Right arrows (18 × 30,
    /// in 32-wide buttons); clicking cycles it.
    #[allow(clippy::too_many_arguments)]
    fn menu_selector(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        row: Rect,
        w: f32,
        id: &'static str,
        values: &[&str],
        index: usize,
        interactive: bool,
    ) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let r = Rect::from_min_max(Pos2::new(row.max.x - 0.6 * w, row.min.y), row.max);
        let n = values.len();
        let mut out = None;
        let body = ui.interact(r, ui.id().with(("game_menu_sel", id)), Sense::click());
        for (dx, img, dir) in [(0.0, LEFT, n - 1), (r.width() - 32.0, RIGHT, 1)] {
            let ar = Rect::from_min_size(Pos2::new(r.min.x + dx, r.min.y), Vec2::new(32.0, r.height()));
            let resp = ui.interact(ar, ui.id().with(("game_menu_sel_arrow", id, dir)), Sense::click());
            let c = if interactive && resp.hovered() { Color32::WHITE } else { ARROW };
            if let Some((tex, _)) = self.gui.texture(&ctx, &mut self.assets, None, img) {
                p.image(
                    tex,
                    Rect::from_center_size(ar.center(), Vec2::new(18.0, 30.0)),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    c,
                );
            }
            if interactive && resp.clicked() {
                out = Some((index + dir) % n);
            }
        }
        if out.is_none() && interactive && body.clicked() {
            out = Some((index + 1) % n);
        }
        p.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            values[out.unwrap_or(index).min(n - 1)],
            font(&ctx, 22.0 * NOMINAL, Weight::Regular),
            Color32::WHITE,
        );
        out
    }

    /// ReportAbuseMenuContainerPage: what to report, then Submit (a test
    /// session sends nothing).
    fn menu_report(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        view: Rect,
        top: Pos2,
        interactive: bool,
        rects: &mut HashMap<&'static str, Rect>,
    ) -> f32 {
        let ctx = ui.ctx().clone();
        let w = view.width();
        let label_f = font(&ctx, 16.0 * NOMINAL, Weight::Medium);
        let rows = [
            ("Game or Player?", "Experience"),
            ("Which Player?", "Choose One"),
            ("Type Of Abuse", "Choose One"),
            ("Short Description (Optional)", ""),
        ];
        for (n, (name, value)) in rows.iter().enumerate() {
            let r = Rect::from_min_size(top + Vec2::new(0.0, 50.0 * n as f32), Vec2::new(w, 50.0));
            p.text(
                Pos2::new(r.min.x + 10.0, r.center().y),
                egui::Align2::LEFT_CENTER,
                *name,
                label_f.clone(),
                Color32::WHITE,
            );
            // DropDownFrame: ControlInputBackground (Slate), Divider stroke.
            let d = Rect::from_min_size(
                Pos2::new(r.max.x - 0.6 * w, r.center().y - 20.0),
                Vec2::new(0.6 * w, 40.0),
            );
            p.rect_filled(d, CornerRadius::same(RADIUS as u8), SLATE);
            p.rect_stroke(
                d,
                CornerRadius::same(RADIUS as u8),
                Stroke::new(1.0, white(DIVIDER_A)),
                egui::StrokeKind::Inside,
            );
            let dim = n == 1;
            p.text(
                Pos2::new(d.min.x + 12.0, d.center().y),
                egui::Align2::LEFT_CENTER,
                *value,
                font(&ctx, 22.0 * NOMINAL, Weight::Regular),
                white(if dim { 0.5 } else { 1.0 }),
            );
            if n < 3 {
                crate::ti::paint(
                    p,
                    Rect::from_center_size(Pos2::new(d.max.x - 20.0, d.center().y), Vec2::splat(18.0)),
                    "chevron-down",
                    white(0.7),
                );
            }
        }
        let y = top.y + 50.0 * rows.len() as f32 + 20.0;
        let r = Rect::from_min_size(Pos2::new(view.center().x - 100.0, y), Vec2::new(200.0, 48.0));
        self.menu_button(ui, p, r, "submit", "Submit", None, 22.0 * NOMINAL, interactive, rects);
        y + 48.0 - top.y
    }

    /// Help.lua (keyboard): groups of action / key rows, 42 tall, 2 apart,
    /// on BackgroundUIDefault with 3 px corners, three columns.
    fn menu_help(&self, ctx: &egui::Context, p: &egui::Painter, top: Pos2, w: f32) -> f32 {
        let groups: [(&str, &[(&str, &str)]); 5] = [
            (
                "Character Movement",
                &[
                    ("Move Forward", "W/Up Arrow"),
                    ("Move Backward", "S/Down Arrow"),
                    ("Move Left", "A/Left Arrow"),
                    ("Move Right", "D/Right Arrow"),
                    ("Jump", "Space"),
                ],
            ),
            (
                "Accessories",
                &[
                    ("Equip Tools", "1,2,3..."),
                    ("Unequip Tools", "1,2,3..."),
                    ("Drop Tool", "Backspace"),
                    ("Use Tool", "Left Mouse Button"),
                ],
            ),
            (
                "Misc",
                &[
                    ("UI Selection Toggle", "\\"),
                    ("Screenshot", "Alt + 1"),
                    ("Record Video", "F12"),
                    ("Dev Console", "F9"),
                    ("Mouselock", "Shift"),
                    ("Graphics Level", "F10"),
                    ("Fullscreen", "F11"),
                    ("Perf. Stats", "Ctrl + Shift + F7"),
                ],
            ),
            (
                "Camera Movement",
                &[
                    ("Rotate", "Right Mouse Button"),
                    ("Zoom In/Out", "Mouse Wheel"),
                    ("Zoom In", "I"),
                    ("Zoom Out", "O"),
                ],
            ),
            ("Menu Items", &[("Roblox Menu", "Esc"), ("Backpack", "~"), ("Playerlist", "Tab"), ("Chat", "/")]),
        ];
        let gap = 10.0; // PC_TABLE_SPACING
        let col_w = (w - 2.0 * gap) / 3.0;
        let title_f = font(ctx, 14.0, Weight::Medium);
        let text_f = font(ctx, 14.0, Weight::Regular);
        let mut bottom: f32 = 0.0;
        let mut col_y = [0.0f32; 3];
        for (n, (title, rows)) in groups.iter().enumerate() {
            // Character Movement, Accessories, Misc across; Camera Movement
            // under the first, Menu Items under the second (rowOffset 50).
            let col = n % 3;
            let y = if n < 3 { 0.0 } else { col_y[col] + 50.0 };
            let x = top.x + col as f32 * (col_w + gap);
            p.text(
                Pos2::new(x + 10.0, top.y + y + 15.0),
                egui::Align2::LEFT_CENTER,
                *title,
                title_f.clone(),
                Color32::WHITE,
            );
            let mut ry = top.y + y + 30.0;
            for (action, keys) in rows.iter() {
                let r = Rect::from_min_size(Pos2::new(x, ry), Vec2::new(col_w, 42.0));
                p.rect_filled(r, CornerRadius::same(3), FLINT);
                p.text(
                    Pos2::new(r.min.x + 10.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    *action,
                    title_f.clone(),
                    Color32::WHITE,
                );
                p.text(
                    Pos2::new(r.min.x + col_w * 0.5 - 4.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    *keys,
                    text_f.clone(),
                    Color32::WHITE,
                );
                ry += 44.0;
            }
            col_y[col] = ry - top.y;
            bottom = bottom.max(ry - top.y);
        }
        bottom
    }
}
