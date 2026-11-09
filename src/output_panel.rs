//! The Output panel, as Studio's (checks/ui/studio/output*.png, the Studio
//! an offline Studio runs): a toolbar with "All Messages", "All Contexts", a Filter
//! field, the clear button and a "⋯" menu (Show Timestamp off, Show Context
//! off, Show Source on, Studio's defaults), then monospace lines:
//!
//!   [bar] [HH:MM:SS.mmm  ]message - Script:line
//!
//! Colours as Studio picks them (Studio , see
//! docs/MECHANICS.md): a table of theme colours indexed by message type — Log
//! MainText (188, 190, 200), Information InfoText (143, 180, 255), Warning
//! WarningText (245, 118, 48), Error ErrorText (231, 87, 80), System
//! SensitiveText (223, 106, 247) — with types above Information DemiBold
//! (Builder Mono has no semi-bold face: Bold). The context bar
//! () is one &nbsp; on #00CC67 (server) or #349AD5 (client),
//! then one more &nbsp;; lines 15 px apart. Log lines keep rustudio's
//! stored form ("[Server] msg  —  Path.To.Script:12", which the harnesses
//! read); this only draws them.

use egui::{Color32, FontId, RichText, Sense, Vec2};

use crate::app::{Editor, Level, LogLine};
use crate::theme;

pub const PRINT: Color32 = Color32::from_rgb(188, 190, 200);
pub const WARN: Color32 = Color32::from_rgb(245, 118, 48);
pub const ERROR: Color32 = Color32::from_rgb(231, 87, 80);
pub const INFO: Color32 = Color32::from_rgb(143, 180, 255);
pub const SERVER_BAR: Color32 = Color32::from_rgb(0, 204, 103);
pub const CLIENT_BAR: Color32 = Color32::from_rgb(52, 154, 213);
const LINE_H: f32 = 15.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Print,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Context {
    None,
    Server,
    Client,
}

/// One stored line taken apart the way Studio shows it.
pub struct Shown<'a> {
    pub kind: Kind,
    pub context: Context,
    pub message: &'a str,
    /// "Script:line" (the last part of the source path), when there is one.
    pub source: Option<String>,
}

/// Marks a stack-trace line (Studio's blue "Stack Begin" … "Stack End"
/// after an error); not shown.
pub const STACK_MARK: char = '\u{2063}';

/// "Full.Name" and line of an error message's "Full.Name:line: ..." prefix.
pub fn error_origin(msg: &str) -> Option<(&str, u32)> {
    let mut it = msg.splitn(3, ':');
    let path = it.next()?;
    let line = it.next()?.trim().parse().ok()?;
    it.next()?;
    (!path.is_empty() && !path.contains(' ')).then_some((path, line))
}

fn short_loc(loc: &str) -> String {
    // "Players.Player.PlayerScripts.Client.MapPreload:30" → "MapPreload:30"
    let (path, line) = match loc.rsplit_once(':') {
        Some((p, n)) if n.chars().all(|c| c.is_ascii_digit()) => (p, Some(n)),
        _ => (loc, None),
    };
    let short = path.rsplit('.').next().unwrap_or(path);
    match line {
        Some(n) => format!("{short}:{n}"),
        None => short.to_string(),
    }
}

pub fn parse(l: &LogLine) -> Shown<'_> {
    let mut t = l.text.as_str();
    let context = if let Some(rest) = t.strip_prefix("[Server] ") {
        t = rest;
        Context::Server
    } else if let Some(rest) = t.strip_prefix("[Client] ") {
        t = rest;
        Context::Client
    } else {
        Context::None
    };
    // Stack lines: blue, without the context bar (as Studio draws them).
    let (t, stack) = match t.strip_prefix(STACK_MARK) {
        Some(rest) => (rest, true),
        None => (t, false),
    };
    let (message, source) = match t.rsplit_once("  —  ") {
        Some((m, loc)) => (m, Some(short_loc(loc))),
        // An error names its script in front: "Full.Name:3: boom - Name:3".
        None if l.level == Level::Error => (t, error_origin(t).map(|(p, n)| short_loc(&format!("{p}:{n}")))),
        None => (t, None),
    };
    if stack {
        return Shown { kind: Kind::Info, context: Context::None, message, source };
    }
    // Script output (from a side, or with a source) prints in the normal
    // colour; rustudio's own lines are Studio's blue info lines.
    let kind = match l.level {
        Level::Warn => Kind::Warn,
        Level::Error => Kind::Error,
        Level::Info if context != Context::None || source.is_some() => Kind::Print,
        Level::Info => Kind::Info,
    };
    Shown { kind, context, message, source }
}

pub fn kind_color(k: Kind) -> Color32 {
    match k {
        Kind::Print => PRINT,
        Kind::Info => INFO,
        Kind::Warn => WARN,
        Kind::Error => ERROR,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageFilter {
    All,
    Output,
    Info,
    Warnings,
    Errors,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContextFilter {
    All,
    Server,
    Client,
}

pub struct OutputState {
    pub messages: MessageFilter,
    pub contexts: ContextFilter,
    pub filter: String,
    pub show_timestamp: bool,
    pub show_context: bool,
    pub show_source: bool,
}

impl Default for OutputState {
    fn default() -> Self {
        OutputState {
            messages: MessageFilter::All,
            contexts: ContextFilter::All,
            filter: String::new(),
            show_timestamp: false,
            show_context: false,
            show_source: true,
        }
    }
}

fn wanted(st: &OutputState, s: &Shown, l: &LogLine) -> bool {
    let kind_ok = match st.messages {
        MessageFilter::All => true,
        MessageFilter::Output => s.kind == Kind::Print,
        MessageFilter::Info => s.kind == Kind::Info,
        MessageFilter::Warnings => s.kind == Kind::Warn,
        MessageFilter::Errors => s.kind == Kind::Error,
    };
    let ctx_ok = match st.contexts {
        ContextFilter::All => true,
        ContextFilter::Server => s.context == Context::Server,
        ContextFilter::Client => s.context == Context::Client,
    };
    let text_ok = st.filter.is_empty() || l.text.to_lowercase().contains(&st.filter.to_lowercase());
    kind_ok && ctx_ok && text_ok
}

/// The line as one LayoutJob in Studio's colours (bold for warnings / errors).
pub fn line_job(st: &OutputState, l: &LogLine, s: &Shown) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mono = FontId::monospace(13.0);
    let plain = |c: Color32| egui::TextFormat { font_id: mono.clone(), color: c, ..Default::default() };
    if st.show_timestamp {
        job.append(&format!("{}  ", l.at), 0.0, plain(PRINT));
    }
    if st.show_context && s.context != Context::None {
        let c = if s.context == Context::Server { "Server" } else { "Client" };
        job.append(&format!("{c} - "), 0.0, plain(PRINT));
    }
    let mut fmt = plain(kind_color(s.kind));
    if matches!(s.kind, Kind::Warn | Kind::Error) {
        // (Studio draws them bold; egui's monospace has no bold face, so a
        // heavier look comes from the "bold" family when it is installed)
        fmt.font_id = FontId::new(13.0, egui::FontFamily::Name("mono-bold".into()));
    }
    job.append(s.message, 0.0, fmt);
    if st.show_source {
        if let Some(src) = &s.source {
            job.append(&format!(" - {src}"), 0.0, plain(PRINT));
        }
    }
    job
}

impl Editor {
    pub fn output_ui(&mut self, ui: &mut egui::Ui) {
        self.ui_rects.insert("output.panel", ui.max_rect());
        let st = &mut self.output_view;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("out_messages")
                .width(96.0)
                .selected_text(match st.messages {
                    MessageFilter::All => "All Messages",
                    MessageFilter::Output => "Output",
                    MessageFilter::Info => "Info",
                    MessageFilter::Warnings => "Warnings",
                    MessageFilter::Errors => "Errors",
                })
                .show_ui(ui, |ui| {
                    for (v, n) in [
                        (MessageFilter::All, "All Messages"),
                        (MessageFilter::Output, "Output"),
                        (MessageFilter::Info, "Info"),
                        (MessageFilter::Warnings, "Warnings"),
                        (MessageFilter::Errors, "Errors"),
                    ] {
                        ui.selectable_value(&mut st.messages, v, n);
                    }
                });
            egui::ComboBox::from_id_salt("out_contexts")
                .width(96.0)
                .selected_text(match st.contexts {
                    ContextFilter::All => "All Contexts",
                    ContextFilter::Server => "Server",
                    ContextFilter::Client => "Client",
                })
                .show_ui(ui, |ui| {
                    for (v, n) in [(ContextFilter::All, "All Contexts"), (ContextFilter::Server, "Server"), (ContextFilter::Client, "Client")] {
                        ui.selectable_value(&mut st.contexts, v, n);
                    }
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let more = ui.add(egui::Button::new("⋯").min_size(Vec2::new(24.0, 20.0)));
                egui::Popup::menu(&more).show(|ui| {
                    ui.checkbox(&mut st.show_timestamp, "Show Timestamp");
                    ui.checkbox(&mut st.show_context, "Show Context");
                    ui.checkbox(&mut st.show_source, "Show Source");
                });
                let (r, clear) = ui.allocate_exact_size(Vec2::new(24.0, 20.0), Sense::click());
                crate::ti::paint(ui.painter(), r.shrink(2.0), "clear-all", theme::TEXT);
                if clear.on_hover_text("Clear Output").clicked() {
                    self.log.clear();
                }
                ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("Filter...").desired_width(240.0));
            });
        });
        let st = &self.output_view;
        // Unfiltered (the usual case) needs no per-line parse every frame.
        let unfiltered = st.messages == MessageFilter::All && st.contexts == ContextFilter::All && st.filter.is_empty();
        let shown: Vec<usize> = (0..self.log.len())
            .filter(|i| {
                if unfiltered {
                    return true;
                }
                let l = &self.log[*i];
                wanted(st, &parse(l), l)
            })
            .collect();
        let mut drawn = 0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(ui, LINE_H, shown.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for &i in &shown[range] {
                    let l = &self.log[i];
                    let s = parse(l);
                    let job = line_job(st, l, &s);
                    ui.horizontal(|ui| {
                        // One monospace space wide, then one more space ().
                        let space = ui.fonts_mut(|f| f.glyph_width(&FontId::monospace(13.0), ' '));
                        let (bar, _) = ui.allocate_exact_size(Vec2::new(space, LINE_H), Sense::hover());
                        let bar_color = match s.context {
                            Context::Server => Some(SERVER_BAR),
                            Context::Client => Some(CLIENT_BAR),
                            Context::None => None,
                        };
                        if let Some(c) = bar_color {
                            ui.painter().rect_filled(bar, 0.0, c);
                        }
                        ui.add_space(space);
                        ui.label(job);
                    });
                    drawn += 1;
                }
            });
        self.hud.output_lines = drawn;
        let _ = RichText::new("");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(level: Level, text: &str) -> LogLine {
        LogLine { at: "20:50:23.591".into(), level, text: text.into() }
    }

    /// Lines take Studio's kinds, contexts and "Script:line" sources.
    #[test]
    fn studio_line_format() {
        let p = line(Level::Info, "[Server] hello from a script  —  ServerScriptService.RefScript:1");
        let s = parse(&p);
        assert!(s.kind == Kind::Print && s.context == Context::Server);
        assert_eq!(s.message, "hello from a script");
        assert_eq!(s.source.as_deref(), Some("RefScript:1"));
        let wl = line(Level::Warn, "[Client] careful  —  Players.Player.PlayerScripts.RefLocal:2");
        let w = parse(&wl);
        assert!(w.kind == Kind::Warn && w.context == Context::Client);
        let il = line(Level::Info, "Opened place.rbxl (39 instances)");
        let i = parse(&il);
        assert!(i.kind == Kind::Info && i.source.is_none());
        let el = line(Level::Error, "[Server] ServerScriptService.RefScript:3: boom");
        let e = parse(&el);
        assert!(e.kind == Kind::Error);
        assert_eq!(e.source.as_deref(), Some("RefScript:3"));
        let sl = line(Level::Info, &format!("[Server] {STACK_MARK}Script 'ServerScriptService.RefScript', Line 3  —  ServerScriptService.RefScript:3"));
        let k = parse(&sl);
        assert!(k.kind == Kind::Info && k.context == Context::None);
        assert_eq!(k.message, "Script 'ServerScriptService.RefScript', Line 3");
        let mut st = OutputState::default();
        let text: String = line_job(&st, &p, &s).text;
        assert_eq!(text, "hello from a script - RefScript:1");
        st.show_timestamp = true;
        assert_eq!(line_job(&st, &p, &s).text, "20:50:23.591  hello from a script - RefScript:1");
    }
}
