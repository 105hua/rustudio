//! Roblox account window (docs/AUTH.md): one sign-in, the session cookie from the
//! login keyring (imported once from ~/.config/rustudio/roblox-cookie).
//! Network and keyring work runs on background threads; results come back
//! over a channel and are polled each frame. The cookie never reaches this
//! module: it stays inside rustudio_account's Session.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui::RichText;
use rustudio_account::{
    AccountConfig, Client, CookieSource, Profile, PublishKeyStore, SavedAccount, SignedOut,
    UniverseInfo, VersionType,
};

use crate::app::{Editor, Level};
use crate::doc::DocKind;
use crate::theme;

enum Msg {
    Log(Level, String),
    SignedIn {
        profile: Profile,
        source: CookieSource,
        publish_key: bool,
    },
    Head(u64, egui::ColorImage),
    /// A switch or add that failed; the previous session was kept.
    Failed(String),
    SignedOut(String),
    PublishKey(bool),
    Universes(Vec<UniverseInfo>),
    Avatar(Box<crate::avatar::AvatarLook>),
    Done,
}

/// What the session check found.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    /// Not checked yet (or being checked); the cached name if any.
    Checking(Option<String>),
    SignedIn(String),
    /// "signed out" and why (never the cookie).
    SignedOut(String),
}

pub struct AccountUi {
    pub open: bool,
    pub show_publish: bool,
    config: Option<AccountConfig>,
    client: Arc<Mutex<Option<Client>>>,
    pub state: SessionState,
    source: Option<CookieSource>,
    universes: Vec<UniverseInfo>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    jobs: usize,
    publish_universe: String,
    publish_place: String,
    restored: bool,
    confirm_publish: Option<VersionType>,
    avatar: Option<crate::avatar::AvatarLook>,
    avatar_requested: bool,
    has_publish_key: bool,
    /// The add-account form: the pasted `.ROBLOSECURITY` value (cleared as
    /// soon as it is handed to the sign-in job).
    pub adding: bool,
    cookie_input: String,
    add_error: Option<String>,
    confirm_remove: Option<u64>,
    /// Who the active session belongs to (account switching).
    user: Option<u64>,
    /// Set when the signed-in account changes; the editor resets lists.
    changed: bool,
    heads: HashMap<u64, egui::TextureHandle>,
    head_images: Vec<(u64, egui::ColorImage)>,
    heads_asked: HashSet<u64>,
    restored_universes: bool,
    adding_pending: bool,
    /// A Quick Sign-in in progress (Sign in with your browser).
    quick: Option<QuickView>,
}

/// What the Quick Sign-in job shares with the panel: the code to show, a
/// line about where it is, and a flag that stops it.
#[derive(Default)]
struct QuickView {
    code: Arc<Mutex<Option<String>>>,
    note: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    /// The browser was sent to the confirm page for this code.
    opened: bool,
}

impl QuickView {
    fn cancel(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl AccountUi {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        AccountUi {
            open: false,
            show_publish: false,
            config: None,
            client: Arc::new(Mutex::new(None)),
            state: SessionState::Checking(None),
            source: None,
            universes: Vec::new(),
            tx,
            rx,
            jobs: 0,
            publish_universe: String::new(),
            publish_place: String::new(),
            restored: false,
            confirm_publish: None,
            avatar: None,
            avatar_requested: false,
            has_publish_key: false,
            adding: false,
            cookie_input: String::new(),
            add_error: None,
            confirm_remove: None,
            user: None,
            changed: false,
            heads: HashMap::new(),
            head_images: Vec::new(),
            heads_asked: HashSet::new(),
            restored_universes: false,
            adding_pending: false,
            quick: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.jobs > 0
    }

    /// The loaded avatar, if any.
    pub fn avatar(&self) -> Option<&crate::avatar::AvatarLook> {
        self.avatar.as_ref()
    }

    /// The signed-in client, for asset downloads (None when signed out).
    pub fn shared_client(&self) -> Option<crate::assets::SharedClient> {
        self.signed_in().then(|| self.client.clone())
    }

    /// Fetches the avatar in the background (once per sign-in unless asked again).
    pub fn load_avatar(&mut self) {
        if !self.signed_in() {
            return;
        }
        self.avatar_requested = true;
        self.spawn("Loading the avatar for Play", move |client, tx| {
            let txl = tx.clone();
            let mut log = |s: String| {
                let _ = txl.send(Msg::Log(Level::Info, s));
            };
            match crate::avatar::load(client, &mut log) {
                Ok(look) => {
                    let _ = tx.send(Msg::Avatar(Box::new(look)));
                    Ok(())
                }
                Err(e) => Err(rustudio_account::Error::new(e)),
            }
        });
    }

    /// Runs `f` with the signed-in client (self-test checks).
    pub fn with_client<R>(&self, f: impl FnOnce(&Client) -> R) -> Option<R> {
        let g = self.client.lock().ok()?;
        g.as_ref().map(f)
    }

    /// Forgets the session for this run only (self-test); the keyring keeps it.
    pub fn drop_session(&mut self) {
        if let Ok(mut g) = self.client.lock() {
            *g = None;
        }
        self.state = SessionState::SignedOut("signed out (session closed for this run)".into());
    }

    /// Starts a session check now (self-test).
    pub fn check_now(&mut self) {
        self.restored = true;
        self.check_session();
    }

    /// The signed-in account's user id (Play's local player takes it, as in
    /// Studio).
    pub fn user_id(&self) -> Option<u64> {
        if !self.signed_in() {
            return None;
        }
        self.with_client(|c| c.session.profile.id)
    }

    /// The signed-in account's username and display name (Play's local
    /// player takes them, as in signed-in Studio; offline it is "Player").
    pub fn names(&self) -> Option<(String, String)> {
        if !self.signed_in() {
            return None;
        }
        self.with_client(|c| {
            let p = &c.session.profile;
            let disp = if p.display_name.is_empty() { p.name.clone() } else { p.display_name.clone() };
            (p.name.clone(), disp)
        })
    }

    /// Before Play: lets a session check still running finish, then the
    /// avatar load, up to `max` (Studio only plays signed in, and its first
    /// character already wears the account's avatar). Returns the log lines.
    pub fn settle_for_play(&mut self, max: std::time::Duration) -> Vec<(Level, String)> {
        let t0 = std::time::Instant::now();
        let mut logs = Vec::new();
        loop {
            logs.extend(self.poll());
            let checking = matches!(self.state, SessionState::Checking(_)) && self.restored;
            let avatar_pending = self.signed_in() && self.avatar.is_none() && self.busy();
            if !(checking || avatar_pending) || t0.elapsed() >= max {
                return logs;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Signed in means Roblox accepted the session this run.
    pub fn signed_in(&self) -> bool {
        matches!(self.state, SessionState::SignedIn(_))
    }

    pub fn status_short(&self) -> String {
        match &self.state {
            SessionState::SignedIn(w) => format!("Roblox: {w}"),
            SessionState::Checking(Some(w)) => format!("Roblox: checking {w}…"),
            SessionState::Checking(None) => "Roblox: checking…".into(),
            SessionState::SignedOut(_) => "Roblox: signed out".into(),
        }
    }

    /// Drains worker messages; returns log lines for the Output panel.
    pub fn poll(&mut self) -> Vec<(Level, String)> {
        let mut logs = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Log(l, t) => logs.push((l, t)),
                Msg::SignedIn {
                    profile,
                    source,
                    publish_key,
                } => {
                    let who = profile.display();
                    logs.push((
                        Level::Info,
                        format!(
                            "Signed in as {who} (session from {})",
                            match source {
                                CookieSource::Keyring => "the login keyring".to_string(),
                                CookieSource::File { imported: true } =>
                                    "the cookie file, now in the login keyring".to_string(),
                                CookieSource::File { imported: false } =>
                                    "the cookie file (the keyring couldn't store it)".to_string(),
                            }
                        ),
                    ));
                    if self.user != Some(profile.id) {
                        self.changed = self.user.is_some() || self.changed;
                        self.user = Some(profile.id);
                        self.universes.clear();
                        self.avatar = None;
                        self.avatar_requested = false;
                    }
                    let cfg = self.config();
                    cfg.remember(&profile);
                    cfg.signed_out = false;
                    let _ = self.config().save();
                    if std::mem::take(&mut self.adding_pending) && self.add_error.is_none() {
                        self.adding = false;
                    }
                    self.state = SessionState::SignedIn(who);
                    self.source = Some(source);
                    self.has_publish_key = publish_key;
                }
                Msg::Head(id, img) => self.head_images.push((id, img)),
                Msg::Failed(e) => {
                    logs.push((Level::Error, format!("Roblox: {e}")));
                    self.add_error = Some(e);
                }
                Msg::SignedOut(why) => {
                    logs.push((Level::Info, format!("Roblox: {why}")));
                    if self.user.take().is_some() {
                        self.changed = true;
                    }
                    self.adding_pending = false;
                    self.state = SessionState::SignedOut(why);
                    self.source = None;
                    self.universes.clear();
                    self.avatar = None;
                    self.avatar_requested = false;
                }
                Msg::PublishKey(k) => self.has_publish_key = k,
                Msg::Universes(u) => {
                    logs.push((Level::Info, format!("Found {} experience(s)", u.len())));
                    self.universes = u;
                }
                Msg::Avatar(look) => {
                    logs.push((
                        Level::Info,
                        format!(
                            "Avatar ready for Play (user {}, {}): the next character uses it",
                            look.user_id,
                            if look.r15 { "R15" } else { "R6" }
                        ),
                    ));
                    self.avatar = Some(*look);
                }
                Msg::Done => {
                    self.jobs = self.jobs.saturating_sub(1);
                    if self.jobs == 0 {
                        self.quick = None;
                    }
                }
            }
        }
        if self.signed_in() && !self.avatar_requested {
            self.load_avatar();
        }
        logs
    }

    fn config(&mut self) -> &mut AccountConfig {
        if self.config.is_none() {
            self.config = Some(match AccountConfig::load() {
                Ok(c) => c,
                Err(e) => {
                    let _ = self
                        .tx
                        .send(Msg::Log(Level::Error, format!("Account config: {e}")));
                    AccountConfig::default()
                }
            });
        }
        self.config.as_mut().expect("loaded")
    }

    /// Runs `f` on a worker thread with access to the shared client.
    fn spawn(
        &mut self,
        label: &str,
        f: impl FnOnce(&Arc<Mutex<Option<Client>>>, &Sender<Msg>) -> rustudio_account::Result<()>
            + Send
            + 'static,
    ) {
        self.jobs += 1;
        let tx = self.tx.clone();
        let client = self.client.clone();
        let label = label.to_string();
        let _ = tx.send(Msg::Log(Level::Info, format!("{label}…")));
        std::thread::spawn(move || {
            if let Err(e) = f(&client, &tx) {
                let _ = tx.send(Msg::Log(Level::Error, format!("{label} failed: {e}")));
            }
            let _ = tx.send(Msg::Done);
        });
    }

    /// Checks the session once at start (and on "Check again").
    pub fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        self.check_session();
    }

    fn check_session(&mut self) {
        let cached = Client::cached_profile().map(|c| c.profile.display());
        self.state = SessionState::Checking(cached);
        self.connect("Checking the Roblox session", false, |cfg| {
            // Tokens an older (OAuth) rustudio left in the keyring go.
            PublishKeyStore::forget_oauth_tokens();
            Client::open(cfg)
        });
    }

    /// Runs a sign-in style job: on success its client becomes the session
    /// (kept for switching), else the editor is signed out.
    /// With `keep`, a failure reports itself and re-opens the active
    /// session (the keyring's active slot only changes on success).
    fn connect(
        &mut self,
        label: &str,
        keep: bool,
        open: impl FnOnce(AccountConfig) -> Result<Client, SignedOut> + Send + 'static,
    ) {
        let cfg = self.config().clone();
        crate::assets::session_checking(true);
        self.spawn(label, move |client, tx| {
            // Downloads waiting on the check (LoadAsset at Play start) go on
            // as soon as it ends, without waiting for the next frame.
            let mut r = open(cfg.clone());
            if keep {
                if let Err(e) = &r {
                    let _ = tx.send(Msg::Failed(e.describe()));
                    r = Client::open(cfg);
                }
            }
            match r {
                Ok(c) => {
                    let _ = c.remember_account();
                    let profile = c.session.profile.clone();
                    let source = c.source;
                    *client.lock().unwrap() = Some(c);
                    crate::assets::session_ready(Some(client.clone()));
                    let _ = tx.send(Msg::SignedIn {
                        profile,
                        source,
                        publish_key: Client::has_publish_key(),
                    });
                }
                Err(e) => {
                    *client.lock().unwrap() = None;
                    crate::assets::session_ready(None);
                    let _ = tx.send(Msg::SignedOut(e.describe()));
                }
            }
            Ok(())
        });
    }

    /// Accounts signed in on this computer before, most recent first.
    pub fn accounts(&mut self) -> Vec<SavedAccount> {
        self.config().accounts.clone()
    }

    /// The active account's user id (also while a check is running).
    pub fn active_id(&self) -> Option<u64> {
        self.user
    }

    /// True once after the signed-in account changed (switch, sign-out).
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Makes a saved account the active session.
    pub fn switch_to(&mut self, id: u64) {
        if self.busy() || (self.user == Some(id) && self.signed_in()) {
            return;
        }
        let name = self
            .config()
            .accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| format!("@{}", a.name))
            .unwrap_or_else(|| format!("user {id}"));
        self.state = SessionState::Checking(Some(name.clone()));
        self.add_error = None;
        self.connect(&format!("Switching to {name}"), true, move |cfg| {
            Client::switch_to(cfg, id)
        });
    }

    /// Signs in with the pasted session value (Add account).
    fn add_account(&mut self) {
        let raw = std::mem::take(&mut self.cookie_input);
        if raw.trim().is_empty() {
            return;
        }
        self.add_error = None;
        self.adding_pending = true;
        self.state = SessionState::Checking(None);
        self.connect("Signing in", true, move |cfg| {
            let c = Client::sign_in_with(cfg, &raw)?;
            Ok(c)
        });
    }

    /// Forgets a saved account that isn't the active one.
    fn remove_account(&mut self, id: u64) {
        self.config().accounts.retain(|a| a.id != id);
        let _ = self.config().save();
        self.heads.remove(&id);
        std::thread::spawn(move || Client::forget_account(id));
    }

    /// Sign in after a sign-out: the import file may be read again.
    fn sign_in(&mut self) {
        self.config().signed_out = false;
        let _ = self.config().save();
        self.check_session();
    }

    /// Sign in with the default browser (Roblox's Quick Sign-in): shows a
    /// code, sends the browser to Roblox's confirm page, and once the person
    /// confirms there, the session it yields is added like a pasted one.
    fn browser_sign_in(&mut self) {
        use rustudio_account::quick_login::{QuickLogin, QuickStatus};
        self.add_error = None;
        self.adding = true;
        self.adding_pending = true;
        let view = QuickView::default();
        let (code, note, stop) = (view.code.clone(), view.note.clone(), view.stop.clone());
        *note.lock().unwrap() = "Getting a code from Roblox…".into();
        self.quick = Some(view);
        self.connect("Signing in with the browser", true, move |cfg| {
            let fail = |m: String| SignedOut::Rejected(m);
            let q = QuickLogin::start().map_err(|e| fail(e.to_string()))?;
            *code.lock().unwrap() = Some(q.code.clone());
            *note.lock().unwrap() = "Waiting for you to enter the code…".into();
            let mut why = "the sign-in code expired; try again".to_string();
            let got = q
                .wait(&|| stop.load(Ordering::Relaxed), &mut |st| {
                    let line = match st {
                        QuickStatus::UserLinked(Some(name)) => format!("Confirm as @{name} in your browser…"),
                        QuickStatus::UserLinked(None) => "Confirm in your browser…".into(),
                        QuickStatus::Validated => "Confirmed, signing in…".into(),
                        QuickStatus::Cancelled => {
                            why = "the sign-in was turned down in the browser".into();
                            return;
                        }
                        _ => return,
                    };
                    *note.lock().unwrap() = line;
                })
                .map_err(|e| fail(e.to_string()))?;
            match got {
                Some(raw) => Client::sign_in_with(cfg, &raw),
                None if stop.load(Ordering::Relaxed) => Err(fail("sign-in cancelled".into())),
                None => Err(fail(why)),
            }
        });
    }

    /// Signs the active account out and forgets it on this computer; other
    /// saved accounts stay for switching.
    pub fn sign_out(&mut self) {
        let id = self.user;
        if let Some(id) = id {
            self.config().accounts.retain(|a| a.id != id);
        }
        self.config().signed_out = true;
        let cfg = self.config().clone();
        self.spawn("Signing out", move |client, tx| {
            client.lock().unwrap().take();
            crate::assets::session_ready(None);
            if let Some(id) = id {
                Client::forget_account(id);
            }
            Client::sign_out(cfg)?;
            let _ = tx.send(Msg::SignedOut("signed out".into()));
            Ok(())
        });
    }

    /// Fetches headshots for the saved accounts not asked for yet, and turns
    /// fetched ones into textures.
    fn heads(&mut self, ctx: &egui::Context) {
        for (id, img) in std::mem::take(&mut self.head_images) {
            let t = ctx.load_texture(format!("account-head-{id}"), img, egui::TextureOptions::LINEAR);
            self.heads.insert(id, t);
        }
        let user = self.user;
        let mut ids: Vec<u64> = self.config().accounts.iter().map(|a| a.id).chain(user).collect();
        ids.retain(|i| !self.heads_asked.contains(i));
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        self.heads_asked.extend(ids.iter().copied());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let Ok(list) = rustudio_account::headshots(&ids) else { return };
            for (id, bytes) in list {
                let Ok(img) = image::load_from_memory(&bytes) else { continue };
                let img = img.thumbnail(96, 96).to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let _ = tx.send(Msg::Head(id, egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw())));
            }
        });
    }

    /// The headshot (or an initial on a plate) painted into `r`.
    pub fn paint_head(&self, ui: &egui::Ui, r: egui::Rect, id: u64, name: &str) {
        let size = r.width();
        ui.painter().circle_filled(r.center(), size * 0.5, theme::PANEL_ALT);
        if let Some(t) = self.heads.get(&id) {
            egui::Image::new(t).corner_radius(size * 0.5).paint_at(ui, r);
        } else {
            let initial = name.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, initial, egui::FontId::proportional(size * 0.45), theme::TEXT_DIM);
        }
    }

    /// The headshot (or an initial on a plate) at `size`.
    pub fn head_widget(&self, ui: &mut egui::Ui, id: u64, name: &str, size: f32) {
        if let Some(t) = self.heads.get(&id) {
            ui.add(
                egui::Image::new(t)
                    .fit_to_exact_size(egui::vec2(size, size))
                    .corner_radius(size * 0.5)
                    .bg_fill(theme::PANEL_ALT),
            );
        } else {
            let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), size * 0.5, theme::PANEL_ALT);
            let initial = name.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                initial,
                egui::FontId::proportional(size * 0.45),
                theme::TEXT_DIM,
            );
        }
    }

    fn remove_publish_key(&mut self) {
        self.spawn("Removing the publishing key", move |_, tx| {
            PublishKeyStore::clear()?;
            let _ = tx.send(Msg::PublishKey(false));
            Ok(())
        });
    }

    fn refresh_universes(&mut self) {
        let bookmarks = self.config().bookmarks.clone();
        self.spawn("Listing experiences", move |client, tx| {
            let mut guard = client.lock().unwrap();
            let c = guard
                .as_mut()
                .ok_or_else(|| rustudio_account::Error::new("signed out"))?;
            c.config.bookmarks = bookmarks;
            let list = c.list_universes()?;
            let _ = tx.send(Msg::Universes(list));
            Ok(())
        });
    }

    fn publish(&mut self, universe: u64, place: u64, vt: VersionType, bytes: Vec<u8>) {
        self.spawn(
            &format!(
                "{} place {place}",
                if vt == VersionType::Published {
                    "Publishing"
                } else {
                    "Saving"
                }
            ),
            move |client, tx| {
                let guard = client.lock().unwrap();
                let c = guard
                    .as_ref()
                    .ok_or_else(|| rustudio_account::Error::new("signed out"))?;
                let r = c.publish_place(universe, place, vt, &bytes, false)?;
                let _ = tx.send(Msg::Log(
                    Level::Info,
                    format!(
                        "{} version {} of place {place} (with the {})",
                        vt.as_str(),
                        r.version_number,
                        r.via
                    ),
                ));
                Ok(())
            },
        );
    }
}

impl Editor {
    pub fn account_window(&mut self, ctx: &egui::Context) {
        if self.account.open || self.account.show_publish {
            self.account.restore();
        }
        if self.account.take_changed() {
            self.cloud.forget_lists();
            self.start.forget_lists();
        }
        if self.account.open {
            self.account.heads(ctx);
            self.account_popover(ctx);
        }
        if self.account.show_publish {
            let mut open = true;
            egui::Window::new("Publish to Roblox")
                .id(egui::Id::new("roblox_publish_window"))
                .open(&mut open)
                .default_width(400.0)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| self.publish_contents(ui));
            if !open {
                self.account.show_publish = false;
                self.account.confirm_publish = None;
            }
        }
    }

    /// The account dropdown under the top-right account button.
    fn account_popover(&mut self, ctx: &egui::Context) {
        const W: f32 = 300.0;
        let button = self.ui_rects.get("account").copied();
        let screen = ctx.content_rect();
        let pos = match button {
            Some(r) => egui::pos2((r.max.x - W).max(screen.min.x + 8.0), r.max.y + 6.0),
            None => egui::pos2(screen.max.x - W - 8.0, screen.min.y + 40.0),
        };
        let shown = egui::Area::new(egui::Id::new("account_popover"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::PANEL_DARK)
                    .stroke(egui::Stroke::new(1.0, theme::DIVIDER))
                    .corner_radius(6.0)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 6],
                        blur: 18,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(110),
                    })
                    .show(ui, |ui| {
                        ui.set_width(W);
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        self.account_contents(ui);
                    });
            });
        // Click outside (not on the button, which toggles) or Esc closes it.
        let outside = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer.interact_pos().is_some_and(|p| {
                    !shown.response.rect.contains(p) && !button.is_some_and(|b| b.contains(p))
                })
        });
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if outside || esc {
            self.account.open = false;
            if let Some(q) = &self.account.quick {
                q.cancel();
            }
            self.account.adding = false;
            self.account.cookie_input.clear();
            self.account.add_error = None;
        }
    }

    fn account_contents(&mut self, ui: &mut egui::Ui) {
        enum Do {
            Switch(u64),
            Remove(u64),
            SignOut,
            Retry,
            BrowserSignIn,
        }
        let mut todo: Option<Do> = None;
        let a = &mut self.account;
        let accounts = a.accounts();
        let signed = a.signed_in();
        let checking = matches!(a.state, SessionState::Checking(_));
        let active = a.user;
        let me = active.and_then(|id| accounts.iter().find(|x| x.id == id).cloned());

        // Who is signed in.
        pad(ui, 14.0, 14.0, |ui| {
            ui.horizontal(|ui| {
                match (&me, signed || checking) {
                    (Some(m), true) => a.head_widget(ui, m.id, &m.name, 44.0),
                    _ => icon_plate(ui, "user", 44.0),
                }
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.add_space(3.0);
                    match (&me, signed, checking) {
                        (Some(m), true, _) => {
                            ui.label(RichText::new(disp(m)).size(15.0).strong().color(theme::TEXT));
                            ui.add_space(2.0);
                            ui.label(RichText::new(format!("@{}", m.name)).size(12.0).color(theme::TEXT_DIM));
                        }
                        (_, _, true) => {
                            let who = match &a.state {
                                SessionState::Checking(Some(w)) => w.clone(),
                                _ => "Signing in".into(),
                            };
                            ui.label(RichText::new(who).size(15.0).strong().color(theme::TEXT));
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().size(11.0).color(theme::TEXT_DIM));
                                ui.add_space(6.0);
                                ui.label(RichText::new("Checking with Roblox…").size(12.0).color(theme::TEXT_DIM));
                            });
                        }
                        _ => {
                            ui.label(RichText::new("Not signed in").size(15.0).strong().color(theme::TEXT));
                            ui.add_space(2.0);
                            ui.label(RichText::new("Sign in to open your places").size(12.0).color(theme::TEXT_DIM));
                        }
                    }
                });
            });
            if let SessionState::SignedOut(why) = &a.state {
                if why != "signed out" && !why.starts_with("signed out (") {
                    ui.add_space(10.0);
                    ui.label(RichText::new(why).size(12.0).color(theme::WARN));
                }
            }
        });
        divider(ui);

        if a.adding || (!signed && !checking && accounts.is_empty()) {
            // Add account: one field, one button.
            pad(ui, 14.0, 12.0, |ui| {
                ui.label(RichText::new("Add account").size(13.0).strong().color(theme::TEXT));
                ui.add_space(8.0);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut a.cookie_input)
                        .password(true)
                        .hint_text("Paste .ROBLOSECURITY cookie")
                        .margin(egui::vec2(8.0, 6.0))
                        .desired_width(f32::INFINITY),
                );
                if a.adding && a.cookie_input.is_empty() && !field.has_focus() && a.add_error.is_none() {
                    field.request_focus();
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new("From a browser signed in to roblox.com")
                        .size(11.5)
                        .color(theme::TEXT_DIM),
                );
                if let Some(e) = &a.add_error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).size(12.0).color(theme::ERROR));
                }
                ui.add_space(10.0);
                if let Some(q) = &mut a.quick {
                    let code = q.code.lock().unwrap().clone();
                    let note = q.note.lock().unwrap().clone();
                    egui::Frame::new()
                        .fill(theme::PANEL_DARK)
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical_centered(|ui| {
                                match &code {
                                    Some(c) => {
                                        ui.label(RichText::new("Enter this code on Roblox").size(12.0).color(theme::TEXT_DIM));
                                        ui.add_space(4.0);
                                        ui.label(RichText::new(c).size(26.0).monospace().strong().color(theme::TEXT));
                                        ui.add_space(4.0);
                                        if ui.link(RichText::new("Open the page again").size(11.5)).clicked() {
                                            q.opened = false;
                                        }
                                    }
                                    None => {
                                        ui.add(egui::Spinner::new().size(14.0).color(theme::TEXT_DIM));
                                    }
                                }
                                ui.add_space(4.0);
                                ui.label(RichText::new(note).size(11.5).color(theme::TEXT_DIM));
                            });
                        });
                    if code.is_some() && !q.opened {
                        q.opened = true;
                        ui.ctx().open_url(egui::OpenUrl::new_tab(rustudio_account::quick_login::CONFIRM_URL));
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                } else {
                    let browser = ui.add_enabled(
                        !a.busy(),
                        egui::Button::new(RichText::new("Sign in with your browser").color(theme::TEXT))
                            .min_size(egui::vec2(ui.available_width(), 26.0)),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Shows a code to confirm on roblox.com, in the browser you're signed in to")
                            .size(11.5)
                            .color(theme::TEXT_DIM),
                    );
                    if browser.clicked() {
                        todo = Some(Do::BrowserSignIn);
                    }
                }
                ui.add_space(10.0);
                let mut go = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let row = egui::vec2(ui.available_width(), 26.0);
                ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let ready = !a.busy() && !a.cookie_input.trim().is_empty();
                    let label = if a.busy() { "Signing in…" } else { "Sign in" };
                    go |= ui
                        .add_enabled(
                            ready,
                            egui::Button::new(RichText::new(label).color(egui::Color32::WHITE))
                                .fill(theme::ACCENT)
                                .min_size(egui::vec2(76.0, 26.0)),
                        )
                        .clicked();
                    if (signed || !accounts.is_empty() || a.quick.is_some())
                        && ui
                            .add(egui::Button::new("Cancel").fill(egui::Color32::TRANSPARENT).min_size(egui::vec2(64.0, 26.0)))
                            .clicked()
                    {
                        if let Some(q) = &a.quick {
                            q.cancel();
                        }
                        a.adding = false;
                        a.cookie_input.clear();
                        a.add_error = None;
                    }
                });
                if go && !a.busy() && !a.cookie_input.trim().is_empty() {
                    a.add_account();
                }
            });
        } else {
            // Other accounts: click to switch.
            let others: Vec<&SavedAccount> =
                accounts.iter().filter(|x| Some(x.id) != active || !(signed || checking)).collect();
            if !others.is_empty() {
                ui.add_space(6.0);
                for x in &others {
                    let busy = a.busy();
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 42.0), egui::Sense::click());
                    let hover = resp.hovered() && !busy;
                    if hover {
                        ui.painter().rect_filled(r.shrink2(egui::vec2(6.0, 1.0)), 4.0, theme::HOVER_BG);
                    }
                    let head = egui::Rect::from_min_size(egui::pos2(r.min.x + 16.0, r.center().y - 14.0), egui::vec2(28.0, 28.0));
                    a.paint_head(ui, head, x.id, &x.name);
                    let tx = head.max.x + 12.0;
                    ui.painter().text(egui::pos2(tx, r.center().y - 7.0), egui::Align2::LEFT_CENTER, disp(x), egui::FontId::proportional(13.0), theme::TEXT);
                    ui.painter().text(egui::pos2(tx, r.center().y + 8.0), egui::Align2::LEFT_CENTER, format!("@{}", x.name), egui::FontId::proportional(11.5), theme::TEXT_DIM);
                    let xr = egui::Rect::from_center_size(egui::pos2(r.max.x - 26.0, r.center().y), egui::vec2(22.0, 22.0));
                    let mut on_x = false;
                    if a.confirm_remove == Some(x.id) {
                        let rr = egui::Rect::from_min_max(egui::pos2(r.max.x - 92.0, r.center().y - 12.0), egui::pos2(r.max.x - 14.0, r.center().y + 12.0));
                        let b = ui.put(rr, egui::Button::new(RichText::new("Remove").size(12.0).color(egui::Color32::WHITE)).fill(theme::ERROR.gamma_multiply(0.8)));
                        if b.clicked() {
                            todo = Some(Do::Remove(x.id));
                        }
                        on_x = true;
                    } else if hover || resp.hovered() {
                        let xh = ui.interact(xr, ui.id().with(("remove", x.id)), egui::Sense::click());
                        if xh.hovered() {
                            ui.painter().rect_filled(xr, 4.0, theme::ACTIVE_TAB);
                        }
                        crate::ti::paint(ui.painter(), xr.shrink(4.0), "x", if xh.hovered() { theme::TEXT } else { theme::TEXT_DIM });
                        if xh.clicked() {
                            a.confirm_remove = Some(x.id);
                        }
                        on_x = xh.hovered();
                        xh.on_hover_text("Remove from this computer");
                    }
                    if resp.clicked() && !on_x && !busy {
                        todo = Some(Do::Switch(x.id));
                    }
                }
                if let Some(e) = &a.add_error {
                    pad(ui, 16.0, 4.0, |ui| {
                        ui.label(RichText::new(e).size(12.0).color(theme::ERROR));
                    });
                }
                ui.add_space(2.0);
            } else {
                ui.add_space(6.0);
            }
            if pop_row(ui, "user-plus", "Add account", !a.busy()).clicked() {
                a.adding = true;
                a.add_error = None;
                a.confirm_remove = None;
            }
            ui.add_space(6.0);
            divider(ui);
            ui.add_space(6.0);
            if signed {
                if pop_row(ui, "logout", "Sign out", !a.busy()).clicked() {
                    todo = Some(Do::SignOut);
                }
            } else if pop_row(ui, "refresh", "Check the session again", !a.busy()).clicked() {
                todo = Some(Do::Retry);
            }
            ui.add_space(6.0);
        }

        // Footer: the account's facts in one quiet line.
        if signed {
            divider(ui);
            pad(ui, 14.0, 8.0, |ui| {
                let avatar = match &a.avatar {
                    Some(l) => if l.r15 { "R15 avatar" } else { "R6 avatar" },
                    None => "default body",
                };
                let id = active.map(|i| format!("ID {i}")).unwrap_or_default();
                let place = match a.source {
                    Some(CookieSource::Keyring) | Some(CookieSource::File { imported: true }) => "login keyring",
                    _ => "cookie file",
                };
                ui.label(RichText::new(format!("{id}  ·  {avatar}  ·  {place}")).size(11.0).color(theme::TEXT_DIM));
            });
        }

        match todo {
            Some(Do::Switch(id)) => {
                a.confirm_remove = None;
                a.switch_to(id);
            }
            Some(Do::Remove(id)) => {
                a.confirm_remove = None;
                a.remove_account(id);
            }
            Some(Do::SignOut) => a.sign_out(),
            Some(Do::Retry) => a.sign_in(),
            Some(Do::BrowserSignIn) => a.browser_sign_in(),
            None => {}
        }
    }

    fn publish_contents(&mut self, ui: &mut egui::Ui) {
        let is_place = self.doc.kind == DocKind::Place;
        let live_place = self.live_team_create_place();
        let origin = self.doc.cloud.clone();
        let a = &mut self.account;
        if a.publish_universe.is_empty() && a.publish_place.is_empty() {
            if let Some(o) = &origin {
                a.publish_universe = o.universe_id.to_string();
                a.publish_place = o.place_id.to_string();
            }
        }
        if a.signed_in() && a.universes.is_empty() && !a.busy() && !a.restored_universes {
            a.restored_universes = true;
            a.refresh_universes();
        }
        ui.label(RichText::new(a.status_short()).small().color(theme::TEXT_DIM));
        if !a.signed_in() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Sign in to publish.").color(theme::WARN));
                if ui.button("Account…").clicked() {
                    a.open = true;
                }
            });
        }
        // Pick a place from the account's experiences, or type the IDs.
        let current = a
            .universes
            .iter()
            .flat_map(|u| u.places.iter().map(move |p| (u, p)))
            .find(|(u, p)| a.publish_universe.trim() == u.id.to_string() && a.publish_place.trim() == p.id.to_string())
            .map(|(u, p)| format!("{} — {}", u.name, p.name))
            .unwrap_or_else(|| "Choose a place…".into());
        let mut pick: Option<(u64, u64)> = None;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("publish_target")
                .selected_text(current)
                .width(300.0)
                .show_ui(ui, |ui| {
                    if a.universes.is_empty() {
                        ui.label(RichText::new(if a.busy() { "Loading…" } else { "No experiences found" }).color(theme::TEXT_DIM));
                    }
                    for u in &a.universes {
                        ui.label(RichText::new(&u.name).small().strong().color(theme::TEXT_DIM));
                        for p in &u.places {
                            let label = format!("  {}{}", if p.is_root { "★ " } else { "" }, p.name);
                            if ui.selectable_label(false, label).clicked() {
                                pick = Some((u.id, p.id));
                            }
                        }
                    }
                });
            if ui.add_enabled(a.signed_in() && !a.busy(), egui::Button::new("Reload")).on_hover_text("Reload your experiences").clicked() {
                a.refresh_universes();
            }
        });
        if let Some((u, p)) = pick {
            a.publish_universe = u.to_string();
            a.publish_place = p.to_string();
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Universe ID").small().color(theme::TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut a.publish_universe).desired_width(110.0));
            ui.label(RichText::new("Place ID").small().color(theme::TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut a.publish_place).desired_width(110.0));
        });
        let ids = (a.publish_universe.trim().parse::<u64>(), a.publish_place.trim().parse::<u64>());
        let ready = a.signed_in() && is_place && ids.0.is_ok() && ids.1.is_ok() && !a.busy();
        if !is_place {
            ui.label(RichText::new("Open a place (.rbxl) to publish; models cannot be published as places.").color(theme::WARN));
        }
        let saves_live = live_place.is_some() && ids.1.as_ref().ok().copied() == live_place;
        let note = if saves_live {
            "Saved through the connected Team Create server."
        } else {
            "Uploaded with your signed-in session. The Output panel shows Roblox's answer."
        };
        ui.label(RichText::new(note).small().color(theme::TEXT_DIM));
        if a.has_publish_key {
            ui.horizontal(|ui| {
                ui.label(RichText::new("An older publishing key is stored and still used.").small().color(theme::TEXT_DIM));
                if ui.add_enabled(!a.busy(), egui::Button::new("Remove it").small()).clicked() {
                    a.remove_publish_key();
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(ready, egui::Button::new("Save")).on_hover_text("Stores a saved version without making it the published version").clicked() {
                a.confirm_publish = Some(VersionType::Saved);
            }
            if ui.add_enabled(ready, egui::Button::new(RichText::new("Publish").strong())).on_hover_text("Becomes the live version").clicked() {
                a.confirm_publish = Some(VersionType::Published);
            }
        });
        if let Some(vt) = self.account.confirm_publish {
            let (u, p) = (
                self.account.publish_universe.trim().to_string(),
                self.account.publish_place.trim().to_string(),
            );
            let mut decision: Option<bool> = None;
            ui.group(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} the open place to place {p} (universe {u})?",
                        if vt == VersionType::Published { "Publish (goes live)" } else { "Save" }
                    ))
                    .strong(),
                );
                ui.horizontal(|ui| {
                    if ui.button("Yes").clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
            if let Some(go) = decision {
                self.account.confirm_publish = None;
                if go {
                    if live_place.is_some() && p.parse::<u64>().ok() == live_place {
                        self.save_cloud(vt, false);
                        return;
                    }
                    match (u.parse(), p.parse(), self.doc.encode_all(false)) {
                        (Ok(u), Ok(p), Ok(bytes)) => self.account.publish(u, p, vt, bytes),
                        (_, _, Err(e)) => self.error(format!("Could not encode the place: {e}")),
                        _ => self.error("Universe and place IDs must be numbers"),
                    }
                }
            }
        }
    }
}

fn disp(a: &SavedAccount) -> &str {
    if a.display_name.is_empty() { &a.name } else { &a.display_name }
}

/// A section with side and vertical padding.
fn pad(ui: &mut egui::Ui, x: f32, y: f32, f: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(x as i8, y as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        f(ui);
    });
}

fn divider(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, theme::DIVIDER);
}

/// A round plate with an icon (no headshot).
fn icon_plate(ui: &mut egui::Ui, icon: &str, size: f32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), size * 0.5, theme::PANEL_ALT);
    crate::ti::paint(ui.painter(), r.shrink(size * 0.25), icon, theme::TEXT_DIM);
}

/// A full-width menu row: icon and label, hover highlight.
fn pop_row(ui: &mut egui::Ui, icon: &str, label: &str, enabled: bool) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), sense);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(r.shrink2(egui::vec2(6.0, 1.0)), 4.0, theme::HOVER_BG);
    }
    let c = if enabled { theme::TEXT } else { theme::TEXT_DIM };
    let ir = egui::Rect::from_center_size(egui::pos2(r.min.x + 30.0, r.center().y), egui::vec2(17.0, 17.0));
    crate::ti::paint(ui.painter(), ir, icon, if enabled { theme::TEXT_DIM } else { theme::DIVIDER });
    ui.painter().text(egui::pos2(r.min.x + 52.0, r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), c);
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default })
}
