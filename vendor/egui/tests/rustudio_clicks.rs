use egui::*;

struct H { ctx: Context, t: f64, text: String, galley_pos: Pos2, galley: Option<std::sync::Arc<Galley>>, id: Id }

impl H {
    fn new(text: &str) -> Self {
        let mut h = H { ctx: Context::default(), t: 0.0, text: text.into(), galley_pos: Pos2::ZERO, galley: None, id: Id::new("ed") };
        h.frame(vec![]);
        h
    }
    fn frame(&mut self, events: Vec<Event>) {
        let input = RawInput { time: Some(self.t), events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut text = std::mem::take(&mut self.text);
        let id = self.id;
        let mut out = None;
        let mut fo = self.ctx.run_ui(input, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                out = Some(TextEdit::multiline(&mut text).id(id).code_editor().desired_width(f32::INFINITY).show(ui));
            });
        });
        fo.textures_delta.clear();
        let o = out.unwrap();
        self.galley_pos = o.galley_pos;
        self.galley = Some(o.galley);
        self.text = text;
        self.t += 1.0 / 60.0;
    }
    fn pos_of(&self, ch: usize) -> Pos2 {
        let g = self.galley.as_ref().unwrap();
        let r = g.pos_from_cursor(text::CCursor::new(ch));
        self.galley_pos + r.center().to_vec2() + vec2(2.0, 0.0)
    }
    fn mv(&mut self, p: Pos2) { self.frame(vec![Event::PointerMoved(p)]); }
    fn btn(&mut self, p: Pos2, pressed: bool) {
        self.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
    fn click(&mut self, p: Pos2) { self.btn(p, true); self.btn(p, false); }
    fn sel(&self) -> (usize, usize) {
        let s = text_edit::TextEditState::load(&self.ctx, self.id).unwrap();
        let r = s.cursor.char_range().unwrap();
        let (a, b) = (r.primary.index.0, r.secondary.index.0);
        (a.min(b), a.max(b))
    }
}

const T: &str = "local hello = workspace.Part\nprint(hello)\nreturn 1";

#[test]
fn double_click_word() {
    let mut h = H::new(T);
    let p = h.pos_of(8); // inside "hello"
    h.mv(p);
    h.click(p);
    h.click(p);
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.sel(), (6, 11), "double click");
}

#[test]
fn triple_click_line() {
    let mut h = H::new(T);
    let p = h.pos_of(8);
    h.mv(p);
    h.click(p);
    h.click(p);
    h.click(p);
    h.frame(vec![]);
    let (a, b) = h.sel();
    assert_eq!(a, 0);
    assert!(b == 28 || b == 29, "line {b}");
}

#[test]
fn double_drag_words() {
    let mut h = H::new(T);
    let p = h.pos_of(8);
    h.mv(p);
    h.click(p);
    h.btn(p, true);
    let q = h.pos_of(20); // in "workspace"
    for i in 1..=10 { h.mv(p + (q - p) * (i as f32 / 10.0)); }
    assert_eq!(h.sel(), (6, 23), "double drag");
    h.btn(q, false);
    h.frame(vec![]);
    assert_eq!(h.sel(), (6, 23), "after release");
}

#[test]
fn triple_drag_lines() {
    let mut h = H::new(T);
    let p = h.pos_of(8);
    h.mv(p);
    h.click(p);
    h.click(p);
    h.btn(p, true);
    let q = h.pos_of(33); // line 2
    for i in 1..=10 { h.mv(p + (q - p) * (i as f32 / 10.0)); }
    let (a, b) = h.sel();
    assert_eq!(a, 0);
    assert!(b >= 41, "triple drag {b}");
}

#[test]
fn single_drag_chars() {
    let mut h = H::new(T);
    let p = h.pos_of(8);
    h.mv(p);
    h.btn(p, true);
    let q = h.pos_of(20);
    for i in 1..=10 { h.mv(p + (q - p) * (i as f32 / 10.0)); }
    h.btn(q, false);
    let (a, b) = h.sel();
    assert!(a == 8 && (b == 20 || b == 21), "{a} {b}");
}

// ---- selectable labels (Output panel etc.) ----

struct L { ctx: Context, t: f64, rect: Rect, copied: Option<String> }

const LT: &str = "local hello = workspace.Part";

impl L {
    fn new() -> Self { let mut l = L { ctx: Context::default(), t: 0.0, rect: Rect::NOTHING, copied: None }; l.frame(vec![]); l }
    fn frame(&mut self, events: Vec<Event>) {
        let input = RawInput { time: Some(self.t), events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut rect = Rect::NOTHING;
        let mut fo = self.ctx.run_ui(input, |ui| {
            ui.style_mut().override_font_id = Some(FontId::monospace(13.0));
            rect = ui.label(LT).rect;
        });
        fo.textures_delta.clear();
        for c in fo.platform_output.commands {
            if let OutputCommand::CopyText(s) = c { self.copied = Some(s); }
        }
        self.rect = rect;
        self.t += 1.0 / 60.0;
    }
    fn pos_of(&self, ch: usize) -> Pos2 {
        let w = self.rect.width() / LT.chars().count() as f32;
        pos2(self.rect.left() + w * (ch as f32 + 0.5), self.rect.center().y)
    }
    fn btn(&mut self, p: Pos2, pressed: bool) {
        self.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
    fn copy(&mut self) -> String { self.copied = None; self.frame(vec![Event::Copy]); self.copied.clone().unwrap_or_default() }
}

#[test]
fn label_double_click_word() {
    let mut l = L::new();
    let p = l.pos_of(8);
    l.frame(vec![Event::PointerMoved(p)]);
    l.btn(p, true); l.btn(p, false);
    l.btn(p, true);
    l.frame(vec![]); l.frame(vec![]);
    l.btn(p, false);
    l.frame(vec![]);
    assert_eq!(l.copy(), "hello");
}

#[test]
fn label_double_drag_words() {
    let mut l = L::new();
    let p = l.pos_of(8);
    l.frame(vec![Event::PointerMoved(p)]);
    l.btn(p, true); l.btn(p, false);
    l.btn(p, true);
    let q = l.pos_of(18);
    for i in 1..=10 { l.frame(vec![Event::PointerMoved(p + (q - p) * (i as f32 / 10.0))]); }
    l.btn(q, false);
    assert_eq!(l.copy(), "hello = workspace");
}

#[test]
fn label_triple_click_line() {
    let mut l = L::new();
    let p = l.pos_of(8);
    l.frame(vec![Event::PointerMoved(p)]);
    for _ in 0..3 { l.btn(p, true); l.btn(p, false); }
    assert_eq!(l.copy(), LT);
}

#[test]
fn label_single_drag_chars() {
    let mut l = L::new();
    let p = l.pos_of(8);
    l.frame(vec![Event::PointerMoved(p)]);
    l.btn(p, true);
    let q = l.pos_of(18);
    for i in 1..=10 { l.frame(vec![Event::PointerMoved(p + (q - p) * (i as f32 / 10.0))]); }
    l.btn(q, false);
    let s = l.copy();
    assert!(s.ends_with("lo = work") && s.len() <= 10, "{s:?}");
}
