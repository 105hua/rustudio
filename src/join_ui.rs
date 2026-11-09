//! The centred "Opening / Joining Team Create" dialog: the place's icon and
//! name, each step of the open with its time, and live counters (instances,
//! batches, time since the last data) so a slow join is visibly alive and a
//! stuck one is visibly stuck.

use std::time::{Duration, Instant};

use egui::{Color32, RichText};
use rustudio_net::live::JoinStage;

use crate::app::Editor;
use crate::theme;

/// No data for this long during a step: say it may be stuck.
const STALL: Duration = Duration::from_secs(20);

pub struct JoinProgress {
    pub place: u64,
    pub universe: u64,
    pub name: String,
    pub experience: String,
    started: Instant,
    /// The lookup (resolving the place, Team Create check) took this long.
    lookup: Option<Duration>,
    /// The join step under way (None: still looking the place up).
    stage: Option<JoinStage>,
    stage_started: Instant,
    done: Vec<(JoinStage, Duration)>,
    waiting: Option<(u32, String)>,
    pub instances: usize,
    batches: usize,
    last_data: Option<Instant>,
    pub error: Option<String>,
    failed_after: Option<Duration>,
}

impl JoinProgress {
    pub fn new(place: u64) -> Self {
        let now = Instant::now();
        JoinProgress {
            place,
            universe: 0,
            name: String::new(),
            experience: String::new(),
            started: now,
            lookup: None,
            stage: None,
            stage_started: now,
            done: Vec::new(),
            waiting: None,
            instances: 0,
            batches: 0,
            last_data: None,
            error: None,
            failed_after: None,
        }
    }

    /// The place was looked up and has Team Create on.
    pub fn found(&mut self, universe: u64, name: &str, experience: &str) {
        self.universe = universe;
        self.name = name.to_string();
        self.experience = experience.to_string();
        if self.lookup.is_none() {
            self.lookup = Some(self.stage_started.elapsed());
            self.stage_started = Instant::now();
        }
    }

    pub fn stage(&mut self, s: JoinStage) {
        if self.lookup.is_none() {
            self.lookup = Some(self.stage_started.elapsed());
        }
        if self.stage.is_some_and(|c| c >= s) {
            return;
        }
        if let Some(c) = self.stage {
            self.done.push((c, self.stage_started.elapsed()));
        }
        self.stage = Some(s);
        self.stage_started = Instant::now();
        self.waiting = None;
    }

    pub fn waiting(&mut self, attempt: u32, message: String) {
        self.waiting = Some((attempt, message));
        self.last_data = Some(Instant::now());
    }

    pub fn data(&mut self, instances: usize) {
        self.stage(JoinStage::Receiving);
        self.batches += 1;
        self.instances = instances;
        self.last_data = Some(Instant::now());
    }

    /// The open failed: the step under way stops its clock.
    pub fn fail(&mut self, e: String) {
        if self.error.is_none() {
            self.failed_after = Some(self.stage_started.elapsed());
            self.error = Some(e);
        }
    }

    fn joining(&self) -> bool {
        self.stage.is_some() || self.universe != 0
    }

    /// How long nothing has happened in the current step.
    fn quiet(&self) -> Duration {
        let since = self.last_data.map_or(self.stage_started, |t| t.max(self.stage_started));
        since.elapsed()
    }
}

fn secs(d: Duration) -> String {
    let s = d.as_secs_f32();
    if s < 60.0 { format!("{s:.1}s") } else { format!("{}m {:02}s", s as u64 / 60, s as u64 % 60) }
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

enum Mark {
    Done(Duration),
    Now(Duration),
    Failed(Duration),
    Later,
}

fn step_row(ui: &mut egui::Ui, label: &str, mark: Mark, detail: Option<(String, Color32)>) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(egui::vec2(w, 24.0), egui::Sense::hover());
    let ir = egui::Rect::from_center_size(egui::pos2(r.min.x + 9.0, r.center().y), egui::vec2(16.0, 16.0));
    let p = ui.painter();
    let (text, time) = match &mark {
        Mark::Done(d) => {
            crate::ti::paint(p, ir, "circle-check-filled", theme::SELECT);
            (theme::TEXT_DIM, secs(*d))
        }
        Mark::Now(d) => {
            egui::Spinner::new().size(14.0).color(theme::TEXT).paint_at(ui, ir);
            (theme::TEXT, secs(*d))
        }
        Mark::Failed(d) => {
            crate::ti::paint(p, ir, "circle-x-filled", theme::ERROR);
            (theme::TEXT, secs(*d))
        }
        Mark::Later => {
            p.circle_stroke(ir.center(), 6.0, egui::Stroke::new(1.5, theme::DIVIDER));
            (theme::TEXT_DIM.gamma_multiply(0.7), String::new())
        }
    };
    let font = egui::FontId::proportional(13.0);
    p.text(egui::pos2(r.min.x + 28.0, r.center().y), egui::Align2::LEFT_CENTER, label, font.clone(), text);
    p.text(egui::pos2(r.max.x, r.center().y), egui::Align2::RIGHT_CENTER, time, egui::FontId::monospace(12.0), theme::TEXT_DIM);
    if let Some((d, c)) = detail {
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.add(egui::Label::new(RichText::new(d).size(12.0).color(c)).wrap());
        });
        ui.add_space(2.0);
    }
}

impl Editor {
    /// The dialog, while a place opens or a Team Create join runs.
    pub fn join_modal(&mut self, ctx: &egui::Context) {
        let Some(j) = &self.cloud.join else { return };
        let icon = self.cloud.icon(j.universe);
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("join_modal"))
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_DARK)
                    .stroke(egui::Stroke::new(1.0, theme::DIVIDER))
                    .corner_radius(8.0)
                    .inner_margin(22.0),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                // The place.
                ui.vertical_centered(|ui| {
                    let size = egui::vec2(96.0, 96.0);
                    match &icon {
                        Some(t) => {
                            ui.add(egui::Image::new(t).fit_to_exact_size(size).corner_radius(10.0));
                        }
                        None => {
                            let (r, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                            ui.painter().rect_filled(r, 10.0, theme::PANEL_ALT);
                            crate::ti::paint(ui.painter(), r.shrink(28.0), "world", theme::TEXT_DIM);
                        }
                    }
                    ui.add_space(14.0);
                    let title = if j.name.is_empty() { format!("Place {}", j.place) } else { j.name.clone() };
                    ui.label(RichText::new(title).size(17.0).strong().color(theme::TEXT));
                    if !j.experience.is_empty() && j.experience != j.name {
                        ui.add_space(3.0);
                        ui.label(RichText::new(&j.experience).size(12.5).color(theme::TEXT_DIM));
                    }
                    ui.add_space(14.0);
                    let head = match (&j.error, j.joining()) {
                        (Some(_), true) => "Couldn't join Team Create",
                        (Some(_), false) => "Couldn't open the place",
                        (None, true) => "Joining Team Create…",
                        (None, false) => "Opening place…",
                    };
                    ui.label(RichText::new(head).size(14.0).color(if j.error.is_some() { theme::ERROR } else { theme::TEXT }));
                });
                ui.add_space(16.0);

                // The steps.
                egui::Frame::new()
                    .fill(theme::BG)
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let failed = j.error.is_some();
                        let lookup = match j.lookup {
                            Some(d) => Mark::Done(d),
                            None if failed => Mark::Failed(j.failed_after.unwrap_or_default()),
                            None => Mark::Now(j.stage_started.elapsed()),
                        };
                        step_row(ui, "Looking up the place", lookup, None);
                        if !j.joining() {
                            return;
                        }
                        for s in JoinStage::ALL {
                            let mark = if let Some((_, d)) = j.done.iter().find(|(d, _)| *d == s) {
                                Mark::Done(*d)
                            } else if j.stage == Some(s) && failed {
                                Mark::Failed(j.failed_after.unwrap_or_default())
                            } else if j.stage == Some(s) {
                                Mark::Now(j.stage_started.elapsed())
                            } else {
                                Mark::Later
                            };
                            let now = matches!(mark, Mark::Now(_));
                            let detail = match s {
                                JoinStage::Matchmaking if now => Some(match &j.waiting {
                                    Some((n, m)) if !m.is_empty() => format!("Roblox is starting the server (check {n}): {m}"),
                                    Some((n, _)) => format!("Roblox is starting the server (check {n})"),
                                    None => "Requesting a server for this place".into(),
                                }),
                                JoinStage::Handshake if now => Some("Waiting for the server to start sending the place".into()),
                                JoinStage::Receiving if now || j.batches > 0 => Some(format!(
                                    "{} instances · {} batches{}",
                                    thousands(j.instances),
                                    thousands(j.batches),
                                    j.last_data.filter(|_| now).map(|t| format!(" · last data {} ago", secs(t.elapsed()))).unwrap_or_default()
                                )),
                                _ => None,
                            };
                            step_row(ui, s.label(), mark, detail.map(|d| (d, theme::TEXT_DIM)));
                        }
                        if !failed && j.stage.is_some() && j.quiet() > STALL {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!(
                                    "Nothing from Roblox for {}. The server may be slow to start; Cancel and try again if this keeps going.",
                                    secs(j.quiet())
                                ))
                                .size(12.0)
                                .color(theme::WARN),
                            );
                        }
                    });
                if let Some(e) = &j.error {
                    ui.add_space(10.0);
                    ui.add(egui::Label::new(RichText::new(e).size(12.5).color(theme::ERROR)).wrap());
                }

                // Footer.
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Elapsed {}", secs(j.started.elapsed()))).size(12.0).color(theme::TEXT_DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if j.error.is_some() { "Close" } else { "Cancel" };
                        if ui.add(egui::Button::new(label).min_size(egui::vec2(76.0, 26.0))).clicked() {
                            cancel = true;
                        }
                    });
                });
            });
        ctx.request_repaint_after(Duration::from_millis(100));
        if cancel {
            self.cancel_join();
        }
    }

    /// Stops a join or open still under way (dropping the session cancels
    /// its worker) and closes the dialog.
    pub fn cancel_join(&mut self) {
        let failed = self.cloud.join.take().is_some_and(|j| j.error.is_some());
        if !failed {
            if self.live_join_pending() {
                self.cloud.drop_live();
            }
            self.cloud.cancel_open();
            self.info("Cancelled opening the place");
        }
    }
}
