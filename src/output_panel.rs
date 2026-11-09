//! The Output panel, as Studio's (checks/ui/studio/output*.png, the Studio
//! an offline Studio runs): a toolbar with "All Messages", "All Contexts", a Filter
//! field, the clear button and a "⋯" menu (Show Timestamp off, Show Context
//! off, Show Source on, Studio's defaults), then monospace lines:
//!
//!   [bar] [HH:MM:SS.mmm  ]message - Script:line
//!
//! Colours as Studio picks them (, see
//! docs/MECHANICS.md): a table of theme colours indexed by message type — Log
//! MainText (188, 190, 200), Information InfoText (143, 180, 255), Warning
//! WarningText (245, 118, 48), Error ErrorText (231, 87, 80), System
//! SensitiveText (223, 106, 247) — with types above Information DemiBold
//! (Builder Mono has no semi-bold face: Bold). The context bar
//! is one &nbsp; on #00CC67 (server) or #349AD5 (client),
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

/// The run of same-kind characters (word chars, spaces, or one other mark)
/// around `c` on line `c.0`.
fn word_at(text: &str, c: TextPos) -> (TextPos, TextPos) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (c, c);
    }
    let class = |ch: char| {
        if ch.is_alphanumeric() || ch == '_' {
            0
        } else if ch == '\n' {
            3
        } else if ch.is_whitespace() {
            1
        } else {
            2
        }
    };
    let mut at = c.1.min(chars.len() - 1);
    // Past the end of a line: the word just before it.
    if class(chars[at]) == 3 && at > 0 && class(chars[at - 1]) != 3 {
        at -= 1;
    }
    let k = class(chars[at]);
    if k >= 2 {
        return ((c.0, at), (c.0, at + 1));
    }
    let mut a = at;
    while a > 0 && class(chars[a - 1]) == k {
        a -= 1;
    }
    let mut b = at + 1;
    while b < chars.len() && class(chars[b]) == k {
        b += 1;
    }
    ((c.0, a), (c.0, b))
}

/// The line of a (maybe multi-line) message around `c`, its break included
/// when one follows.
fn line_at(text: &str, c: TextPos) -> (TextPos, TextPos) {
    let mut start = 0;
    for seg in text.split('\n') {
        let end = start + seg.chars().count();
        if c.1 <= end {
            let n = text.chars().count();
            return ((c.0, start), (c.0, (end + 1).min(n)));
        }
        start = end + 1;
    }
    ((c.0, 0), (c.0, text.chars().count()))
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

/// A place in the output: (log line index, char in the line as shown).
pub type TextPos = (usize, usize);

/// Text selected across Output lines, as a text editor's: the whole log is
/// one selectable text (click / double / triple + drag, Ctrl+A, Ctrl+C).
#[derive(Default)]
pub struct OutputSelection {
    /// Where the selection started and where it ends now (either order).
    pub anchor: TextPos,
    pub head: TextPos,
    /// Mouse held after a press in the panel: 1 chars, 2 words, 3 lines,
    /// with the word / line first picked.
    drag: Option<(u8, TextPos, TextPos)>,
    /// Quick presses near one spot: time, place, count (1..=3).
    press: Option<(f64, egui::Pos2, u8)>,
}

pub struct OutputState {
    pub messages: MessageFilter,
    pub contexts: ContextFilter,
    pub filter: String,
    pub show_timestamp: bool,
    pub show_context: bool,
    pub show_source: bool,
    pub sel: OutputSelection,
}

impl OutputState {
    /// The log dropped its first `n` lines: keep the selection on its text.
    pub fn log_drained(&mut self, n: usize) {
        let s = &mut self.sel;
        let shift = |p: &mut TextPos| *p = if p.0 >= n { (p.0 - n, p.1) } else { (0, 0) };
        shift(&mut s.anchor);
        shift(&mut s.head);
        s.drag = None;
    }
    pub fn log_cleared(&mut self) {
        let s = &mut self.sel;
        s.anchor = (0, 0);
        s.head = (0, 0);
        s.drag = None;
    }
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
            sel: OutputSelection::default(),
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
    let plain = |c: Color32| egui::TextFormat { font_id: mono.clone(), color: c, line_height: Some(LINE_H), ..Default::default() };
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
                    st.log_cleared();
                }
                ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("Filter...").desired_width(240.0));
            });
        });
        self.hud.output_lines = log_view(ui, &mut self.output_view, &self.log);
        let _ = RichText::new("");
    }
}

/// The lines, drawn and selectable as one text; returns how many were drawn.
pub fn log_view(ui: &mut egui::Ui, st: &mut OutputState, log: &[LogLine]) -> usize {
    // Unfiltered (the usual case) needs no per-line parse every frame.
    let unfiltered = st.messages == MessageFilter::All && st.contexts == ContextFilter::All && st.filter.is_empty();
    let shown: Vec<usize> = (0..log.len())
        .filter(|i| {
            if unfiltered {
                return true;
            }
            let l = &log[*i];
            wanted(st, &parse(l), l)
        })
        .collect();
    let text_of = |st: &OutputState, i: usize| line_job(st, &log[i], &parse(&log[i])).text;
    // A message can span lines (stack traces, banners): each takes
    // LINE_H per line; `tops` are the running offsets.
    let mut tops = Vec::with_capacity(shown.len() + 1);
    let mut y = 0.0;
    for &i in &shown {
        tops.push(y);
        y += LINE_H * (log[i].text.matches('\n').count() + 1) as f32;
    }
    tops.push(y);
    let id = egui::Id::new("output_text");
    let mut drawn = 0;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_viewport(ui, |ui, viewport| {
            let total = *tops.last().unwrap_or(&0.0);
            let (content, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), total), Sense::hover());
            let view = ui.clip_rect();
            // Under the lines (they only sense hover), above nothing: the
            // scroll bar is added after and stays on top.
            let resp = ui.interact(view, id, Sense::click_and_drag());
            let first = tops.partition_point(|&t| t <= viewport.min.y).saturating_sub(1);
            let last = tops.partition_point(|&t| t < viewport.max.y).min(shown.len());
            let space = ui.fonts_mut(|f| f.glyph_width(&FontId::monospace(13.0), ' '));
            // Each drawn message: log index, its top, where its text starts,
            // its layout, and a slot for the selection painted under it.
            let mut rows = Vec::new();
            for k in first..last {
                let i = shown[k];
                let l = &log[i];
                let s = parse(l);
                let galley = ui.fonts_mut(|f| f.layout_job(line_job(st, l, &s)));
                let top = content.min.y + tops[k];
                // One monospace space wide, then one more space.
                let bar = egui::Rect::from_min_size(egui::pos2(content.min.x, top), Vec2::new(space, LINE_H));
                let bar_color = match s.context {
                    Context::Server => Some(SERVER_BAR),
                    Context::Client => Some(CLIENT_BAR),
                    Context::None => None,
                };
                if let Some(c) = bar_color {
                    ui.painter().rect_filled(bar, 0.0, c);
                }
                let slot = ui.painter().add(egui::Shape::Noop);
                // (as the earlier row layout spaced them: bar, gap, space, gap)
                let pos = egui::pos2(content.min.x + 2.0 * space + 2.0 * ui.spacing().item_spacing.x, top);
                ui.painter().galley(pos, galley.clone(), PRINT);
                rows.push((i, tops[k + 1] - tops[k], top, pos, galley, slot, space));
                drawn += 1;
            }
            if resp.hovered() {
                ui.set_cursor_icon(egui::CursorIcon::Text);
            }

            // Pointer → (line, char) on the lines drawn this frame;
            // above / below them clamps to the first / last.
            let at = |p: egui::Pos2| -> Option<TextPos> {
                let (first, last) = (rows.first()?, rows.last()?);
                if p.y < first.2 {
                    return Some((first.0, 0));
                }
                let row = rows.iter().rev().find(|r| p.y >= r.2).unwrap_or(last);
                if p.y >= row.2 + row.1 && std::ptr::eq(row, last) {
                    return Some((row.0, row.4.text().chars().count()));
                }
                let c = row.4.cursor_from_pos(p - row.3).index.0;
                Some((row.0, c))
            };
            let (pressed, down, ptr, now, shift) = ui.input(|i| {
                (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos(), i.time, i.modifiers.shift)
            });
            let text_on = |line: usize| rows.iter().find(|r| r.0 == line).map(|r| r.4.text().to_string());
            let sel = &mut st.sel;
            let hit = ptr.and_then(|p| at(p).map(|c| (p, c)));
            if let (true, Some((p, c))) = (pressed && resp.hovered(), hit) {
                ui.memory_mut(|m| m.request_focus(id));
                let count = match sel.press {
                    Some((t, q, n)) if now - t < 0.4 && q.distance(p) < 6.0 => n % 3 + 1,
                    _ => 1,
                };
                sel.press = Some((now, p, count));
                let text = text_on(c.0).unwrap_or_default();
                let (a, b) = match count {
                    1 if shift => (sel.anchor, c),
                    1 => (c, c),
                    2 => word_at(&text, c),
                    _ => line_at(&text, c),
                };
                sel.anchor = a;
                sel.head = b;
                sel.drag = Some((count, a, b));
                if count == 1 && shift {
                    sel.drag = Some((1, a, a));
                }
            } else if let (true, Some((mode, a, b)), Some((p, c))) = (down, sel.drag, hit) {
                // Dragging: grow by chars, words or lines from the press.
                let (h0, h1) = match mode {
                    1 => (c, c),
                    2 => word_at(&text_on(c.0).unwrap_or_default(), c),
                    _ => line_at(&text_on(c.0).unwrap_or_default(), c),
                };
                (sel.anchor, sel.head) = if mode == 1 {
                    (a, c)
                } else if h0 < a {
                    (b, h0)
                } else {
                    (a, h1.max(b))
                };
                // Past the top / bottom edge: scroll along.
                let dy = if p.y < view.top() {
                    view.top() - p.y
                } else if p.y > view.bottom() {
                    view.bottom() - p.y
                } else {
                    0.0
                };
                if dy != 0.0 {
                    ui.scroll_with_delta(Vec2::new(0.0, dy.clamp(-LINE_H * 2.0, LINE_H * 2.0)));
                    ui.ctx().request_repaint();
                }
            }
            if !down {
                sel.drag = None;
            }
            // Clicking anywhere else gives the keyboard back.
            if ui.input(|i| i.pointer.any_pressed()) && !resp.hovered() && ui.memory(|m| m.has_focus(id)) {
                ui.memory_mut(|m| m.surrender_focus(id));
            }

            // Paint the selection under each drawn line.
            let (lo, hi) = if sel.anchor <= sel.head { (sel.anchor, sel.head) } else { (sel.head, sel.anchor) };
            if lo != hi {
                let fill = ui.visuals().selection.bg_fill;
                for (i, _, top, pos, galley, slot, space) in &rows {
                    if *i < lo.0 || *i > hi.0 {
                        continue;
                    }
                    let n = galley.text().chars().count();
                    let a = if *i == lo.0 { lo.1.min(n) } else { 0 };
                    let b = if *i == hi.0 { hi.1.min(n) } else { n };
                    let x = |c: usize| pos.x + galley.pos_from_cursor(egui::text::CCursor::new(c)).min.x;
                    // One rect per line of the message; a selection running on
                    // past a line covers its line break too.
                    let mut shapes = Vec::new();
                    let mut start = 0;
                    for (k, seg) in galley.text().split('\n').enumerate() {
                        let len = seg.chars().count();
                        let (sa, sb) = (a.max(start), b.min(start + len));
                        let runs_on = b > start + len && (start + len < n || *i < hi.0);
                        if sa <= sb && (sa < sb || runs_on) {
                            let y = top + LINE_H * k as f32;
                            let end = x(sb) + if runs_on { *space } else { 0.0 };
                            let r = egui::Rect::from_min_max(egui::pos2(x(sa), y), egui::pos2(end, y + LINE_H));
                            shapes.push(egui::Shape::rect_filled(r, 0.0, fill));
                        }
                        start += len + 1;
                    }
                    ui.painter().set(*slot, egui::Shape::Vec(shapes));
                }
            }
        });

    // Keys while the Output has the keyboard: Ctrl+A, Ctrl+C, Esc.
    if ui.memory(|m| m.has_focus(id)) {
        let (all, copy, esc) = ui.input_mut(|i| {
            (
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::A)),
                i.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut)),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if let (true, Some(&f), Some(&l)) = (all, shown.first(), shown.last()) {
            st.sel.anchor = (f, 0);
            st.sel.head = (l, text_of(st, l).chars().count());
        }
        if esc {
            st.sel.head = st.sel.anchor;
        }
        if copy {
            let (lo, hi) = if st.sel.anchor <= st.sel.head { (st.sel.anchor, st.sel.head) } else { (st.sel.head, st.sel.anchor) };
            if lo != hi {
                let mut out = String::new();
                for &i in shown.iter().filter(|&&i| i >= lo.0 && i <= hi.0) {
                    let t = text_of(st, i);
                    let n = t.chars().count();
                    let a = if i == lo.0 { lo.1.min(n) } else { 0 };
                    let b = if i == hi.0 { hi.1.min(n) } else { n };
                    if !out.is_empty() || i > lo.0 {
                        out.push('\n');
                    }
                    out.extend(t.chars().skip(a).take(b.saturating_sub(a)));
                }
                ui.ctx().copy_text(out.trim_start_matches('\n').to_string());
            }
        }
    }
    drawn
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

    /// Drives `log_view` headless with mouse / key events.
    struct View {
        ctx: egui::Context,
        t: f64,
        st: OutputState,
        log: Vec<LogLine>,
        copied: Option<String>,
    }

    impl View {
        fn new() -> Self {
            let log = vec![
                line(Level::Info, "[Server] hello world  —  ServerScriptService.A:1"),
                line(Level::Info, "[Client] second line here  —  Players.B:2"),
                line(Level::Warn, "[Server] third warning  —  ServerScriptService.C:3"),
            ];
            let ctx = egui::Context::default();
            crate::ti::install(&ctx);
            let mut v = View { ctx, t: 0.0, st: OutputState::default(), log, copied: None };
            v.frame(vec![]);
            v.frame(vec![]);
            v
        }
        fn frame(&mut self, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                time: Some(self.t),
                events,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 300.0))),
                ..Default::default()
            };
            let (st, log) = (&mut self.st, &self.log);
            let mut fo = self.ctx.run_ui(input, |ui| {
                log_view(ui, st, log);
            });
            fo.textures_delta.clear();
            for c in fo.platform_output.commands {
                if let egui::OutputCommand::CopyText(s) = c {
                    self.copied = Some(s);
                }
            }
            self.t += 1.0 / 60.0;
        }
        /// Screen point of char `c` on shown line `row` (monospace).
        fn at(&self, row: usize, c: usize) -> egui::Pos2 {
            let w = self.ctx.fonts_mut(|f| f.glyph_width(&FontId::monospace(13.0), 'x'));
            // Bar, gap, space, gap, then the text.
            let gap = self.ctx.global_style().spacing.item_spacing.x;
            egui::pos2(2.0 * w + 2.0 * gap + w * (c as f32 + 0.3), LINE_H * (row as f32 + 0.5))
        }
        fn btn(&mut self, p: egui::Pos2, down: bool) {
            self.frame(vec![egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: down,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) {
            for i in 1..=8 {
                self.frame(vec![egui::Event::PointerMoved(from + (to - from) * (i as f32 / 8.0))]);
            }
        }
        fn copy(&mut self) -> String {
            self.copied = None;
            self.frame(vec![egui::Event::Copy]);
            self.copied.clone().unwrap_or_default()
        }
    }

    #[test]
    fn select_across_lines_and_copy() {
        let mut v = View::new();
        let (p, q) = (v.at(0, 6), v.at(1, 6));
        v.frame(vec![egui::Event::PointerMoved(p)]);
        v.btn(p, true);
        v.drag(p, q);
        v.btn(q, false);
        assert_eq!(v.copy(), "world - A:1\nsecond");
    }

    #[test]
    fn double_click_word_and_drag_words() {
        let mut v = View::new();
        let p = v.at(0, 7);
        v.frame(vec![egui::Event::PointerMoved(p)]);
        v.btn(p, true);
        v.btn(p, false);
        v.btn(p, true);
        v.frame(vec![]);
        v.frame(vec![]);
        assert_eq!(v.copy(), "world");
        let q = v.at(1, 2);
        v.drag(p, q);
        v.btn(q, false);
        assert_eq!(v.copy(), "world - A:1\nsecond");
    }

    #[test]
    fn triple_click_line_and_drag_lines() {
        let mut v = View::new();
        let p = v.at(1, 3);
        v.frame(vec![egui::Event::PointerMoved(p)]);
        for _ in 0..2 {
            v.btn(p, true);
            v.btn(p, false);
        }
        v.btn(p, true);
        assert_eq!(v.copy(), "second line here - B:2");
        let q = v.at(2, 1);
        v.drag(p, q);
        v.btn(q, false);
        assert_eq!(v.copy(), "second line here - B:2\nthird warning - C:3");
    }

    #[test]
    fn ctrl_a_selects_everything() {
        let mut v = View::new();
        let p = v.at(0, 1);
        v.frame(vec![egui::Event::PointerMoved(p)]);
        v.btn(p, true);
        v.btn(p, false);
        v.frame(vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]);
        assert_eq!(v.copy(), "hello world - A:1\nsecond line here - B:2\nthird warning - C:3");
    }

    #[test]
    fn multi_line_messages_take_their_height() {
        let mut v = View::new();
        v.log.insert(1, line(Level::Info, "[Server] banner top\nbanner mid\nbanner end  —  ServerScriptService.D:4"));
        v.frame(vec![]);
        // Visual rows: 0 hello, 1-3 banner, 4 second line.
        let p = v.at(4, 1);
        v.frame(vec![egui::Event::PointerMoved(p)]);
        for _ in 0..3 {
            v.btn(p, true);
            v.btn(p, false);
        }
        assert_eq!(v.copy(), "second line here - B:2");
        // Triple-click the banner's middle line, drag to its last.
        let (p, q) = (v.at(2, 2), v.at(3, 2));
        v.frame(vec![egui::Event::PointerMoved(p)]);
        v.frame(vec![]);
        v.t += 1.0;
        for _ in 0..2 {
            v.btn(p, true);
            v.btn(p, false);
        }
        v.btn(p, true);
        assert_eq!(v.copy(), "banner mid\n");
        v.drag(p, q);
        v.btn(q, false);
        assert_eq!(v.copy(), "banner mid\nbanner end - D:4");
        // Char drag from the banner's top into the next message.
        let (p, q) = (v.at(1, 7), v.at(4, 6));
        v.t += 1.0;
        v.frame(vec![egui::Event::PointerMoved(p)]);
        v.btn(p, true);
        v.drag(p, q);
        v.btn(q, false);
        assert_eq!(v.copy(), "top\nbanner mid\nbanner end - D:4\nsecond");
    }
}
