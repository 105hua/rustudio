//! Self-test step for round 27, "cloud-place": a place from Roblox.
//!
//! - Links and IDs parse (roblox.com/games/…, create.roblox.com/…/places/…).
//! - Signed in: Roblox's public Baseplate template (place 95206881,
//!   universe 28220420, by @Templates 998796) opens from Roblox through
//!   the app's own path, lands in the cache and in Recent, and knows its
//!   origin; the Team Create status is asked for; a place the account
//!   can't read (100000005) gives a clear error and leaves the document.
//! - Play reports the place's ids (game.PlaceId, GameId, CreatorId,
//!   CreatorType, PlaceVersion).
//! - Ctrl+S against a local mock of Roblox (nothing is sent to Roblox):
//!   someone saved in between, so it stops with the conflict; Overwrite
//!   saves the next version; without a publishing key it says why.
//! - Signed out, the cached copy reopens.
//!
//! Signed out, the Baseplate part is skipped and a made-up origin is used.

use std::sync::{Arc, Mutex};

use rbx_dom_weak::InstanceBuilder;
use rustudio_account::mock::{MockReply, MockRequest, MockServer};
use rustudio_account::{
    parse_place_ref, CloudApi, CloudOrigin, Cookie, CreatorKind, Endpoints, PlaceCache, Profile,
    Session, VersionType,
};

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;

const BASEPLATE: u64 = 95206881;
const BASEPLATE_UNIVERSE: u64 = 28220420;
const TEMPLATES: u64 = 998796;
/// Another account's private place.
const NO_ACCESS: u64 = 100000005;

#[derive(Default)]
pub(super) struct CloudTest {
    stage: u32,
    started: u64,
    t0: Option<std::time::Instant>,
    notes: Vec<String>,
    ok: bool,
    mark: usize,
    /// The version Baseplate opened at.
    version: u64,
    mock: Option<MockServer>,
    server_version: Option<Arc<Mutex<u64>>>,
}

fn mock_api(base: &str, key: Option<&str>) -> crate::cloud_ui::ApiFactory {
    let base = base.to_string();
    let key = key.map(str::to_string);
    Arc::new(move || {
        let s = Session::unchecked(
            Cookie::new("rustudio-selftest-mock-session")?,
            Profile {
                id: 1,
                name: "selftest".into(),
                display_name: "selftest".into(),
            },
        );
        Some(CloudApi::new(s, Endpoints::mock(&base), key.clone()))
    })
}

/// A pretend Roblox for the save: asset delivery reports `server`,
/// Place Publishing answers the next version.
fn mock_roblox(place: u64, universe: u64, server: Arc<Mutex<u64>>) -> Option<MockServer> {
    MockServer::start(Arc::new(move |r: &MockRequest| {
        let v = *server.lock().unwrap();
        if r.path.starts_with(&format!("/assetdelivery/v2/assetId/{place}")) {
            return MockReply::json(200, r#"{"locations":[{"location":"http://127.0.0.1:9/none"}]}"#)
                .with_header("roblox-assetversionnumber", &v.to_string());
        }
        if r.method == "POST" && r.path.starts_with(&format!("/apis/universes/v1/{universe}/places/{place}/versions")) {
            let mut g = server.lock().unwrap();
            *g += 1;
            return MockReply::json(200, &format!("{{\"versionNumber\":{}}}", *g));
        }
        MockReply::json(404, r#"{"errors":[{"code":0,"message":"mock: no route"}]}"#)
    }))
    .ok()
}

impl SelfTest {
    fn cloud_wait(&self) -> u64 {
        self.cloud.t0.map_or(0, |t| t.elapsed().as_secs())
    }

    fn cloud_next(&mut self, f: u64, stage: u32) {
        self.cloud.stage = stage;
        self.cloud.started = f;
        self.cloud.t0 = Some(std::time::Instant::now());
    }

    fn cloud_note(&mut self, ok: bool, note: String) {
        self.cloud.ok &= ok;
        self.cloud.notes.push(note);
    }

    /// Phase 90.
    pub(super) fn tick_cloud(&mut self, f: u64, ed: &mut Editor) -> bool {
        let since = f.saturating_sub(self.cloud.started);
        let waited = self.cloud_wait();
        match self.cloud.stage {
            0 => {
                self.cloud.ok = true;
                // A throwaway cache (RUSTUDIO_CACHE_DIR is the self-test's).
                ed.cloud.cache = PlaceCache::default();
                let _ = std::fs::remove_dir_all(&ed.cloud.cache.root);
                let cases = [
                    ("95206881", Some(BASEPLATE)),
                    ("https://www.roblox.com/games/95206881/Baseplate", Some(BASEPLATE)),
                    ("https://create.roblox.com/dashboard/creations/experiences/28220420/places/95206881/configure", Some(BASEPLATE)),
                    ("roblox.com/games/start?placeId=95206881", Some(BASEPLATE)),
                    ("https://example.com/games/95206881", None),
                    ("Baseplate", None),
                ];
                let bad: Vec<&str> = cases
                    .iter()
                    .filter(|(s, want)| parse_place_ref(s).ok().map(|p| p.place_id) != *want)
                    .map(|(s, _)| *s)
                    .collect();
                self.cloud_note(bad.is_empty(), format!("place IDs and links parse ({} cases; wrong: {bad:?})", cases.len()));
                if ed.account.signed_in() {
                    ed.perform_open_cloud(BASEPLATE, None, false);
                    self.cloud_next(f, 1);
                } else {
                    self.cloud.notes.push("signed out: the Baseplate download is skipped; a made-up origin stands in".into());
                    let mut d = Document::new_place();
                    d.title = "Baseplate".into();
                    d.cloud = Some(CloudOrigin {
                        universe_id: BASEPLATE_UNIVERSE,
                        place_id: BASEPLATE,
                        version: 71,
                        name: "Baseplate".into(),
                        experience: "Baseplate".into(),
                        root_place_id: BASEPLATE,
                        creator_id: TEMPLATES,
                        creator_type: CreatorKind::User,
                        creator_name: "Templates".into(),
                        ..Default::default()
                    });
                    self.cloud.version = 71;
                    ed.set_document(d);
                    self.cloud_next(f, 4);
                }
            }
            // The download.
            1 => {
                let got = ed.doc.cloud.as_ref().is_some_and(|o| o.place_id == BASEPLATE);
                if !got && ed.cloud.open_error.is_none() && waited < 60 {
                    return false;
                }
                match ed.doc.cloud.clone().filter(|_| got) {
                    Some(o) => {
                        let ok = o.universe_id == BASEPLATE_UNIVERSE
                            && o.creator_id == TEMPLATES
                            && o.creator_type == CreatorKind::User
                            && o.version > 0
                            && o.name == "Baseplate";
                        self.cloud_note(ok, format!(
                            "opened from Roblox: {:?} place {} version {} universe {} by {} ({} {}); tab title {:?}",
                            o.name, o.place_id, o.version, o.universe_id, o.creator_label(), o.creator_type.as_str(), o.creator_id, ed.doc.title
                        ));
                        self.cloud.version = o.version;
                        let cached = ed.cloud.cache.cached_versions(BASEPLATE);
                        let recent = ed.cloud.cache.recent().first().map(|r| r.origin.place_id);
                        self.cloud_note(
                            cached.contains(&o.version) && recent == Some(BASEPLATE),
                            format!("cached versions {cached:?} at {}; first in Recent: {recent:?}", ed.cloud.cache.dir(BASEPLATE).display()),
                        );
                        let parts = ed.doc.dom.descendants().count();
                        self.cloud_note(parts > 5, format!("{parts} instances"));
                    }
                    None => self.cloud_note(false, format!(
                        "Baseplate didn't open in {waited} s: {:?} (cloud jobs running: {}, signed in: {})",
                        ed.cloud.open_error, ed.cloud.busy(), ed.account.signed_in()
                    )),
                }
                self.cloud_next(f, 2);
            }
            // The Team Create status of its universe.
            2 => {
                let st = ed.cloud.status.clone().filter(|(u, _)| *u == BASEPLATE_UNIVERSE);
                if st.is_none() && waited < 30 {
                    return false;
                }
                match st {
                    // Baseplate isn't ours: Roblox doesn't say whether Team
                    // Create is on, and we may not edit it.
                    Some((_, s)) => self.cloud_note(
                        s.can_edit() == Some(false) && s.can_collaborate == Some(false),
                        format!("Team Create: {:?} (can collaborate {:?}, {:?})", s.describe(), s.can_collaborate, s.reason),
                    ),
                    None => self.cloud_note(false, "no Team Create status within 30 s".into()),
                }
                ed.cloud.open_error = None;
                ed.perform_open_cloud(NO_ACCESS, None, false);
                self.cloud_next(f, 3);
            }
            // A place this account can't read.
            3 => {
                if ed.cloud.open_error.is_none() && waited < 45 {
                    return false;
                }
                let e = ed.cloud.open_error.clone().unwrap_or_default();
                let still = ed.doc.cloud.as_ref().is_some_and(|o| o.place_id == BASEPLATE);
                self.cloud_note(
                    e.contains("won't give this account place 100000005") && still,
                    format!("no access: {e:?}; the open document stays: {still}"),
                );
                self.cloud_next(f, 4);
            }
            // Play reports the ids.
            4 => {
                let root = ed.doc.dom.root_ref();
                let sss = ed.doc.find_child(root, "ServerScriptService").unwrap_or_else(|| {
                    ed.doc.dom.insert(root, InstanceBuilder::new("ServerScriptService").with_name("ServerScriptService"))
                });
                ed.doc.insert(
                    sss,
                    InstanceBuilder::new("Script").with_name("CloudIds").with_property(
                        "Source",
                        "print(\"cloud ids\", game.PlaceId, game.GameId, game.CreatorId, game.CreatorType, game.PlaceVersion)".to_string(),
                    ),
                );
                self.cloud.mark = ed.log.len();
                ed.start_play(crate::runtime::RunMode::Run);
                self.cloud_next(f, 5);
            }
            5 if since >= 30 => {
                let want = format!(
                    "cloud ids {BASEPLATE} {BASEPLATE_UNIVERSE} {TEMPLATES} Enum.CreatorType.User {}",
                    self.cloud.version
                );
                let line = ed.log[self.cloud.mark.min(ed.log.len())..]
                    .iter()
                    .find(|l| l.text.contains("cloud ids"))
                    .map(|l| l.text.clone());
                self.cloud_note(line.as_deref().is_some_and(|l| l.contains(&want)), format!("Play printed {line:?}"));
                ed.stop_play();
                // Ctrl+S with someone else's newer version on (mock) Roblox.
                let server = Arc::new(Mutex::new(self.cloud.version + 1));
                self.cloud.mock = mock_roblox(BASEPLATE, BASEPLATE_UNIVERSE, server.clone());
                self.cloud.server_version = Some(server);
                let Some(base) = self.cloud.mock.as_ref().map(|m| m.base.clone()) else {
                    self.cloud_note(false, "the mock server didn't start".into());
                    self.cloud_next(f, 9);
                    return false;
                };
                ed.cloud.api_override = Some(mock_api(&base, Some("selftest-key")));
                ed.cloud.conflict = None;
                ed.cloud.last_saved = None;
                ed.run_action("save");
                self.cloud_next(f, 6);
            }
            6 => {
                if ed.cloud.conflict.is_none() && ed.cloud.save_error.is_none() && waited < 20 {
                    return false;
                }
                let posts = self.mock_posts();
                let c = ed.cloud.conflict;
                self.cloud_note(
                    c.is_some_and(|c| c.opened == self.cloud.version && c.server == self.cloud.version + 1) && posts.is_empty(),
                    format!("Ctrl+S after someone saved: conflict {c:?} (asked the user; {} upload(s) sent)", posts.len()),
                );
                // "Overwrite".
                ed.cloud.conflict = None;
                ed.save_cloud(VersionType::Saved, true);
                self.cloud_next(f, 7);
            }
            7 => {
                if ed.cloud.last_saved.is_none() && ed.cloud.save_error.is_none() && waited < 20 {
                    return false;
                }
                let posts = self.mock_posts();
                let want = self.cloud.version + 2;
                let v = ed.doc.cloud.as_ref().map(|o| o.version);
                let ok = ed.cloud.last_saved == Some((want, VersionType::Saved))
                    && v == Some(want)
                    && !ed.doc.dirty()
                    && posts.len() == 1
                    && posts[0].path.ends_with(&format!("/universes/v1/{BASEPLATE_UNIVERSE}/places/{BASEPLATE}/versions?versionType=Saved"))
                    && posts[0].header("x-api-key") == Some("selftest-key");
                self.cloud_note(ok, format!(
                    "Overwrite: saved version {:?}, document at version {v:?}, unsaved changes {}, {} upload(s) to the mock ({:?}); error {:?}",
                    ed.cloud.last_saved, ed.doc.dirty(), posts.len(), posts.first().map(|p| p.path.clone()), ed.cloud.save_error
                ));
                // Without a publishing key.
                let base = self.cloud.mock.as_ref().map(|m| m.base.clone()).unwrap_or_default();
                ed.cloud.api_override = Some(mock_api(&base, None));
                ed.cloud.save_error = None;
                ed.ui_action(&egui::Context::default(), "publish");
                self.cloud_next(f, 8);
            }
            8 => {
                if ed.cloud.save_error.is_none() && waited < 20 {
                    return false;
                }
                let e = ed.cloud.save_error.take().unwrap_or_default();
                let posts = self.mock_posts();
                self.cloud_note(
                    e.contains("Open Cloud API key") && posts.len() == 1,
                    format!("Publish without a key: {:?}…; nothing more sent: {}", e.chars().take(90).collect::<String>(), posts.len() == 1),
                );
                // Signed out: the cached copy reopens.
                ed.cloud.api_override = Some(Arc::new(|| None));
                self.cloud.mark = ed.log.len();
                ed.cloud.open_error = None;
                ed.perform_open_cloud(BASEPLATE, None, false);
                self.cloud_next(f, 9);
            }
            9 => {
                let reopened = ed.log[self.cloud.mark.min(ed.log.len())..]
                    .iter()
                    .any(|l| l.text.contains("this is the copy cached at"));
                if !reopened && ed.cloud.open_error.is_none() && waited < 20 && self.cloud.mock.is_some() {
                    return false;
                }
                if self.cloud.mock.is_some() {
                    let v = ed.doc.cloud.as_ref().map(|o| o.version);
                    self.cloud_note(
                        reopened && v == Some(self.cloud.version + 2),
                        format!("signed out, reopened from the cache: {reopened} (version {v:?}); error {:?}", ed.cloud.open_error),
                    );
                }
                ed.cloud.api_override = None;
                self.cloud.mock = None;
                self.steps.push(Step {
                    step: "cloud-place",
                    passed: self.cloud.ok,
                    detail: std::mem::take(&mut self.cloud.notes).join("; "),
                });
                ed.account.drop_session();
                self.r4.phase = 94;
            }
            _ => {}
        }
        false
    }

    fn mock_posts(&self) -> Vec<MockRequest> {
        self.cloud
            .mock
            .as_ref()
            .map(|m| m.requests().into_iter().filter(|r| r.method == "POST").collect())
            .unwrap_or_default()
    }
}
