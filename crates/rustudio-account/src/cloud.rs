//! Places stored on Roblox, the way Studio opens them (docs/MECHANICS.md, round
//! 27): finding experiences (owned, group, shared with the account, recent),
//! resolving a place to its experience and creator, downloading a version,
//! a local cache so a place reopens offline, the Team Create status, and
//! saving a new version after checking nobody saved in between.
//!
//! Every host goes through [`Endpoints`], so tests point them at a local
//! [`crate::mock::MockServer`]: nothing here writes to Roblox in a test.
//! The only state-changing call is [`CloudApi::save_place`] (Open Cloud
//! Place Publishing, publishing key only).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::VersionType;
use crate::session::Session;
use crate::{Error, Result};

/// The hosts, as base URLs without a trailing slash.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub apis: String,
    pub develop: String,
    pub games: String,
    pub groups: String,
    pub thumbnails: String,
    pub assetdelivery: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            apis: "https://apis.roblox.com".into(),
            develop: "https://develop.roblox.com".into(),
            games: "https://games.roblox.com".into(),
            groups: "https://groups.roblox.com".into(),
            thumbnails: "https://thumbnails.roblox.com".into(),
            assetdelivery: "https://assetdelivery.roblox.com".into(),
        }
    }
}

impl Endpoints {
    /// Every host under one local server: `{base}/apis/…`, `{base}/develop/…`.
    pub fn mock(base: &str) -> Endpoints {
        let h = |n: &str| format!("{base}/{n}");
        Endpoints {
            apis: h("apis"),
            develop: h("develop"),
            games: h("games"),
            groups: h("groups"),
            thumbnails: h("thumbnails"),
            assetdelivery: h("assetdelivery"),
        }
    }
}

/// Which experiences a list shows, as the tabs of Studio's Start Page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExperienceSource {
    Mine,
    Shared,
    Group(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CreatorKind {
    #[default]
    User,
    Group,
}

impl CreatorKind {
    pub fn parse(s: &str) -> CreatorKind {
        if s.eq_ignore_ascii_case("group") {
            CreatorKind::Group
        } else {
            CreatorKind::User
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CreatorKind::User => "User",
            CreatorKind::Group => "Group",
        }
    }

    /// `Enum.CreatorType` value (User 0, Group 1).
    pub fn enum_value(self) -> u32 {
        match self {
            CreatorKind::User => 0,
            CreatorKind::Group => 1,
        }
    }
}

/// Which server-side place an open document is: what Studio knows of a
/// place opened from Roblox, and what Team Create will build on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CloudOrigin {
    pub universe_id: u64,
    pub place_id: u64,
    /// The asset version that was opened (or last saved).
    pub version: u64,
    /// The place's name.
    pub name: String,
    /// The experience's name.
    #[serde(default)]
    pub experience: String,
    #[serde(default)]
    pub root_place_id: u64,
    pub creator_id: u64,
    pub creator_type: CreatorKind,
    #[serde(default)]
    pub creator_name: String,
    /// Team Create on for the universe when it was opened (None: unknown).
    #[serde(default)]
    pub team_create: Option<bool>,
    /// A downloaded copy of a Team Create place, not its live session: it
    /// saves to a file; Save to Roblox only when the person says so.
    #[serde(default)]
    pub local_copy: bool,
}

impl CloudOrigin {
    pub fn creator_label(&self) -> String {
        match (self.creator_type, self.creator_name.is_empty()) {
            (CreatorKind::User, false) => format!("@{}", self.creator_name),
            (CreatorKind::Group, false) => format!("{} (group)", self.creator_name),
            (k, true) => format!("{} {}", k.as_str(), self.creator_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Experience {
    pub id: u64,
    pub name: String,
    pub root_place_id: Option<u64>,
    pub creator_type: CreatorKind,
    pub creator_id: u64,
    pub creator_name: String,
    /// ISO time of the last update, as Roblox gives it.
    pub updated: String,
    /// apis universes/v1/search says whether Team Create is on.
    pub team_create: Option<bool>,
}

impl Experience {
    fn from_json(v: &serde_json::Value) -> Option<Experience> {
        // develop.roblox.com universes and apis universes/v1/search share
        // this shape; games.roblox.com nests the creator.
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        Some(Experience {
            id: v.get("id")?.as_u64()?,
            name: s("name"),
            root_place_id: v.get("rootPlaceId").and_then(|x| x.as_u64()),
            creator_type: CreatorKind::parse(&s("creatorType")),
            creator_id: v.get("creatorTargetId").and_then(|x| x.as_u64()).unwrap_or(0),
            creator_name: s("creatorName"),
            updated: s("updated"),
            team_create: v.get("isTeamCreateEnabled").and_then(|x| x.as_bool()),
        })
    }
}

/// Most recently updated first, as Studio lists them. `updated` is ISO time,
/// so comparing the strings compares the times.
fn newest_first(mut list: Vec<Experience>) -> Vec<Experience> {
    list.sort_by(|a, b| b.updated.cmp(&a.updated));
    list
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloudPlace {
    pub id: u64,
    pub universe_id: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupInfo {
    pub id: u64,
    pub name: String,
    pub role: String,
    /// The role may edit the group's experiences (`manageGroupGames`).
    pub can_edit: Option<bool>,
}

/// One saved version of a place.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionInfo {
    pub version: u64,
    /// ISO time.
    pub created: String,
    pub published: bool,
    /// "Auto", "Manual", "Published" (place-version-history) when known.
    pub kind: String,
}

/// Team Create, as Studio's Start Page finds it for a universe.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TeamCreateStatus {
    /// develop `universes/multiget/teamcreate` (None: Roblox shows it only
    /// to people who can edit the experience).
    pub enabled: Option<bool>,
    /// team-create-service `GetCanCollaborate`.
    pub can_collaborate: Option<bool>,
    /// Its `Error` when it says no ("NotAuthorized", …).
    pub reason: Option<String>,
    /// develop `universes/{id}/permissions`.
    pub can_manage: Option<bool>,
    pub can_cloud_edit: Option<bool>,
}

impl TeamCreateStatus {
    /// One line for the Output and the place's info.
    pub fn describe(&self) -> String {
        let tc = match self.enabled {
            Some(true) => "Team Create is on: live co-editing isn't supported yet; saving replaces the server copy",
            Some(false) => "Team Create is off: saving to Roblox makes a new version of the place",
            None => "Team Create status unknown (Roblox only shows it to people who can edit this experience)",
        };
        if self.can_edit() == Some(false) {
            format!("{tc}. This account can't edit this experience, so Roblox will refuse a save")
        } else {
            tc.to_string()
        }
    }

    pub fn can_edit(&self) -> Option<bool> {
        match (self.can_manage, self.can_cloud_edit) {
            (Some(a), Some(b)) => Some(a || b),
            (Some(a), None) | (None, Some(a)) => Some(a),
            _ => None,
        }
    }
}

/// How a place opens, as Studio decides: a universe with Team Create on
/// is joined live (its saved file isn't what others are editing); any
/// other place downloads its latest saved version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    Download,
    JoinTeamCreate,
}

pub fn open_mode(team_create: Option<bool>, as_copy: bool) -> OpenMode {
    if team_create == Some(true) && !as_copy {
        OpenMode::JoinTeamCreate
    } else {
        OpenMode::Download
    }
}

/// What saving did.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveOutcome {
    Saved { version: u64 },
    /// Someone saved since the document was opened: nothing was sent.
    Conflict { opened: u64, server: u64 },
}

/// Why Save to Roblox can't work without the publishing key (shown as is).
pub const NEEDS_PUBLISH_KEY: &str = "Save to Roblox needs an Open Cloud API key: Roblox refuses the session cookie for place uploads (docs/AUTH.md, \"Publishing\"). Create a key at create.roblox.com > Open Cloud > API Keys with the universe-places:write permission for this experience, and save it as rustudio's publishing key (Roblox account window, \"Save / publish the open place\"). Nothing was saved; File > Save to File As keeps a local copy.";

/// Calls for cloud places over a session (a downloader copy, so long
/// downloads don't hold the account's lock).
pub struct CloudApi {
    pub session: Session,
    pub ep: Endpoints,
    /// The Open Cloud publishing key, when one is saved.
    pub publish_key: Option<String>,
}

fn json(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str(text).ok()
}

/// Roblox's error message in a reply body, if it has one.
fn error_message(text: &str) -> Option<String> {
    let v = json(text)?;
    v.pointer("/errors/0/message")
        .or_else(|| v.get("message"))
        .or_else(|| v.get("Message"))
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .map(str::to_string)
}

impl CloudApi {
    pub fn new(session: Session, ep: Endpoints, publish_key: Option<String>) -> CloudApi {
        CloudApi {
            session,
            ep,
            publish_key,
        }
    }

    fn get_json(&self, url: &str, what: &str) -> Result<serde_json::Value> {
        let (status, text) = self.session.get(url)?;
        if status != 200 {
            let why = error_message(&text).map(|m| format!(": {m}")).unwrap_or_default();
            return Err(Error(format!("{what} answered HTTP {status}{why}")));
        }
        json(&text).ok_or_else(|| Error(format!("{what}: unexpected reply")))
    }

    /// develop pages (`data` + `nextPageCursor`) until the last.
    fn develop_pages(&self, base: &str, what: &str) -> Result<Vec<serde_json::Value>> {
        let mut out = Vec::new();
        let mut cursor = String::new();
        for _ in 0..50 {
            let sep = if base.contains('?') { '&' } else { '?' };
            let mut url = format!("{base}{sep}limit=50&sortOrder=Asc");
            if !cursor.is_empty() {
                url.push_str(&format!("&cursor={cursor}"));
            }
            let v = self.get_json(&url, what)?;
            out.extend(v.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default());
            match v.get("nextPageCursor").and_then(|c| c.as_str()) {
                Some(c) if !c.is_empty() => cursor = c.to_string(),
                _ => break,
            }
        }
        Ok(out)
    }

    /// My Experiences: develop `v1/user/universes` (owned by the account).
    pub fn my_experiences(&self) -> Result<Vec<Experience>> {
        let base = format!("{}/v1/user/universes", self.ep.develop);
        let rows = self.develop_pages(&base, "develop.roblox.com/v1/user/universes")?;
        Ok(newest_first(rows.iter().filter_map(Experience::from_json).collect()))
    }

    /// One page of a list, most recently updated first, as Studio's Start
    /// Page asks for it (apis `universes/v1/search`, docs/TEAMCREATE.md).
    /// Returns the page and whether Roblox has more after it.
    pub fn experiences_page(&self, src: ExperienceSource, page: u32, page_size: u32) -> Result<(Vec<Experience>, bool)> {
        let uid = self.session.profile.id;
        let (creator_type, target) = match src {
            ExperienceSource::Mine => ("User", uid),
            ExperienceSource::Shared => ("Team", uid),
            ExperienceSource::Group(g) => ("Group", g),
        };
        let url = format!(
            "{}/universes/v1/search?creatorType={creator_type}&creatorTargetId={target}&isArchived=false&sortParam=LastUpdated&sortOrder=Desc&pageSize={page_size}&surface=StudioStartPage&PageIndex={page}",
            self.ep.apis
        );
        let v = self.get_json(&url, "apis.roblox.com/universes/v1/search")?;
        let rows: Vec<Experience> = v
            .get("data")
            .and_then(|d| d.as_array())
            .into_iter()
            .flatten()
            .filter_map(Experience::from_json)
            .collect();
        let total = v.get("totalResults").and_then(|t| t.as_u64()).unwrap_or(0);
        let more = !rows.is_empty() && ((page as u64 + 1) * page_size as u64) < total;
        Ok((rows, more))
    }

    /// The account's groups (groups `v2/users/{id}/groups/roles`), each
    /// with whether its role may edit the group's experiences: the role's
    /// `manageGroupGames` from groups `v1/groups/{id}/membership` (None
    /// when Roblox doesn't say). Groups whose role can't are kept, so the
    /// picker can say why their experiences don't open.
    pub fn groups(&self) -> Result<Vec<GroupInfo>> {
        let uid = self.session.profile.id;
        let v = self.get_json(
            &format!("{}/v2/users/{uid}/groups/roles", self.ep.groups),
            "groups.roblox.com/v2/users/{id}/groups/roles",
        )?;
        let mut groups: Vec<GroupInfo> = v
            .get("data")
            .and_then(|d| d.as_array())
            .into_iter()
            .flatten()
            .filter_map(|g| {
                Some(GroupInfo {
                    id: g.pointer("/group/id")?.as_u64()?,
                    name: g.pointer("/group/name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                    role: g.pointer("/role/name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                    can_edit: None,
                })
            })
            .collect();
        for g in &mut groups {
            if let Ok(m) = self.get_json(
                &format!("{}/v1/groups/{}/membership", self.ep.groups, g.id),
                "group membership",
            ) {
                let p = |k: &str| m.pointer(&format!("/permissions/groupEconomyPermissions/{k}")).and_then(|b| b.as_bool());
                g.can_edit = p("manageGroupGames");
            }
        }
        // Editable first, as Studio's group menu.
        groups.sort_by_key(|g| g.can_edit != Some(true));
        Ok(groups)
    }

    /// A group's experiences: develop `v1/groups/{groupId}/universes`.
    pub fn group_experiences(&self, group_id: u64) -> Result<Vec<Experience>> {
        let base = format!("{}/v1/groups/{group_id}/universes", self.ep.develop);
        let rows = self.develop_pages(&base, "develop.roblox.com/v1/groups/{id}/universes")?;
        Ok(newest_first(rows.iter().filter_map(Experience::from_json).collect()))
    }

    /// Shared with me: experiences the account collaborates on, as Studio's
    /// Start Page asks for them (apis `universes/v1/search`, creatorType
    /// Team with the account's own id).
    pub fn shared_experiences(&self) -> Result<Vec<Experience>> {
        let uid = self.session.profile.id;
        let mut out = Vec::new();
        for page in 0..20 {
            let url = format!(
                "{}/universes/v1/search?creatorType=Team&creatorTargetId={uid}&isArchived=false&sortParam=LastUpdated&sortOrder=Desc&pageSize=50&surface=StudioStartPage&PageIndex={page}",
                self.ep.apis
            );
            let v = self.get_json(&url, "apis.roblox.com/universes/v1/search (shared)")?;
            let rows = v.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default();
            out.extend(rows.iter().filter_map(Experience::from_json));
            let total = v.get("totalResults").and_then(|t| t.as_u64()).unwrap_or(0);
            if rows.is_empty() || out.len() as u64 >= total {
                break;
            }
        }
        Ok(out)
    }

    /// A universe's places: develop `v1/universes/{id}/places`.
    pub fn places(&self, universe_id: u64) -> Result<Vec<CloudPlace>> {
        let base = format!("{}/v1/universes/{universe_id}/places", self.ep.develop);
        let rows = self.develop_pages(&base, "develop.roblox.com/v1/universes/{id}/places")?;
        Ok(rows
            .iter()
            .filter_map(|p| {
                Some(CloudPlace {
                    id: p.get("id")?.as_u64()?,
                    universe_id,
                    name: p.get("name").and_then(|n| n.as_str()).unwrap_or("Place").to_string(),
                })
            })
            .collect())
    }

    /// Experience icons (thumbnails `v1/games/icons`, 150x150 PNG URLs).
    pub fn icon_urls(&self, universe_ids: &[u64]) -> Result<Vec<(u64, String)>> {
        let mut out = Vec::new();
        for chunk in universe_ids.chunks(50) {
            let ids: Vec<String> = chunk.iter().map(|i| i.to_string()).collect();
            let v = self.get_json(
                &format!(
                    "{}/v1/games/icons?universeIds={}&size=150x150&format=Png&isCircular=false",
                    self.ep.thumbnails,
                    ids.join(",")
                ),
                "thumbnails.roblox.com/v1/games/icons",
            )?;
            for t in v.get("data").and_then(|d| d.as_array()).into_iter().flatten() {
                if let (Some(id), Some(url)) = (
                    t.get("targetId").and_then(|x| x.as_u64()),
                    t.get("imageUrl").and_then(|x| x.as_str()),
                ) {
                    if t.get("state").and_then(|s| s.as_str()) == Some("Completed") {
                        out.push((id, url.to_string()));
                    }
                }
            }
        }
        Ok(out)
    }

    /// An image from the thumbnail CDN (no cookie).
    pub fn image(&self, url: &str) -> Result<Vec<u8>> {
        let (status, bytes) = self.session.fetch_bytes(url, false)?;
        if status != 200 {
            return Err(Error(format!("thumbnail answered HTTP {status}")));
        }
        Ok(bytes)
    }

    /// Which experience a place belongs to, its name and creator: apis
    /// `universes/v1/places/{id}/universe`, develop `v1/universes/{id}`,
    /// games `v1/games/multiget-place-details` (any place, even private).
    /// `version` is 0 until a download fills it.
    pub fn resolve(&self, place_id: u64) -> Result<CloudOrigin> {
        let (status, text) = self
            .session
            .get(&format!("{}/universes/v1/places/{place_id}/universe", self.ep.apis))?;
        let universe_id = json(&text)
            .and_then(|v| v.get("universeId").and_then(|u| u.as_u64()))
            .filter(|u| *u > 0 && status == 200)
            .ok_or_else(|| Error(format!("Roblox has no place {place_id} (HTTP {status})")))?;
        let u = self.get_json(
            &format!("{}/v1/universes/{universe_id}", self.ep.develop),
            "develop.roblox.com/v1/universes/{id}",
        )?;
        let exp = Experience::from_json(&u).ok_or_else(|| Error("unexpected universe reply".into()))?;
        let mut origin = CloudOrigin {
            universe_id,
            place_id,
            version: 0,
            name: String::new(),
            experience: exp.name.clone(),
            root_place_id: exp.root_place_id.unwrap_or(0),
            creator_id: exp.creator_id,
            creator_type: exp.creator_type,
            creator_name: exp.creator_name,
            team_create: None,
            local_copy: false,
        };
        if let Ok((200, text)) = self.session.get(&format!(
            "{}/v1/games/multiget-place-details?placeIds={place_id}",
            self.ep.games
        )) {
            if let Some(n) = json(&text).and_then(|v| v.pointer("/0/name").and_then(|n| n.as_str()).map(str::to_string)) {
                origin.name = n;
            }
        }
        if origin.name.is_empty() {
            origin.name = self
                .places(universe_id)
                .ok()
                .and_then(|ps| ps.into_iter().find(|p| p.id == place_id).map(|p| p.name))
                .unwrap_or_else(|| {
                    if place_id == origin.root_place_id {
                        exp.name
                    } else {
                        format!("Place {place_id}")
                    }
                });
        }
        Ok(origin)
    }

    /// The asset-delivery descriptor of a place version: (version number,
    /// CDN location). Studio-style errors for places the account can't read.
    fn descriptor(&self, place_id: u64, version: Option<u64>) -> Result<(u64, String)> {
        let url = match version {
            Some(v) => format!("{}/v2/assetId/{place_id}/version/{v}", self.ep.assetdelivery),
            None => format!("{}/v2/assetId/{place_id}", self.ep.assetdelivery),
        };
        let (status, header, text) = self.session.get_with_header(&url, "roblox-assetversionnumber")?;
        let v = json(&text)
            .ok_or_else(|| Error(format!("place {place_id}: asset delivery answered HTTP {status}")))?;
        let Some(location) = v.pointer("/locations/0/location").and_then(|l| l.as_str()) else {
            let why = v
                .pointer("/errors/0/message")
                .and_then(|m| m.as_str())
                .unwrap_or("no download location");
            let code = v.pointer("/errors/0/code").and_then(|c| c.as_u64()).unwrap_or(status as u64);
            let hint = match code {
                401 | 403 | 409 => ": only the owner, the group's editors and collaborators can open a place that isn't uncopylocked",
                404 => ": no such place or version",
                _ => "",
            };
            return Err(Error(format!(
                "Roblox won't give this account place {place_id}{} ({why}){hint}",
                version.map(|v| format!(" version {v}")).unwrap_or_default()
            )));
        };
        let number = header
            .and_then(|h| h.trim().parse().ok())
            .or(version)
            .unwrap_or(0);
        Ok((number, location.to_string()))
    }

    /// The newest version number on Roblox (only the descriptor is fetched,
    /// not the file).
    pub fn latest_version(&self, place_id: u64) -> Result<u64> {
        Ok(self.descriptor(place_id, None)?.0)
    }

    /// A place file: the latest version, or the one asked for. Returns the
    /// bytes and the version they are.
    pub fn download(&self, place_id: u64, version: Option<u64>) -> Result<(Vec<u8>, u64)> {
        let (number, location) = self.descriptor(place_id, version)?;
        let (status, bytes) = self.session.fetch_bytes(&location, false)?;
        if !(200..300).contains(&status) {
            return Err(Error(format!("place {place_id}: download answered HTTP {status}")));
        }
        Ok((bytes, number))
    }

    /// The version list, newest first. Studio's version history
    /// (apis `place-version-history-api/v1/{placeId}/history`), else
    /// develop `v1/assets/{id}/saved-versions`. Both are only for people
    /// who can edit the place; any version still opens by number.
    pub fn versions(&self, place_id: u64) -> Result<Vec<VersionInfo>> {
        let first = self.get_json(
            &format!("{}/place-version-history-api/v1/{place_id}/history?pageSize=50", self.ep.apis),
            "place version history",
        );
        let rows_of = |v: &serde_json::Value, key: &str| -> Vec<VersionInfo> {
            v.get(key)
                .and_then(|d| d.as_array())
                .into_iter()
                .flatten()
                .filter_map(|r| {
                    // place-version-history gives the number as a string ("757").
                    let num = ["version", "assetVersionNumber", "versionNumber"].iter().find_map(|k| {
                        let x = r.get(*k)?;
                        x.as_u64().or_else(|| x.as_str()?.parse().ok())
                    })?;
                    let s = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let created = if r.get("createdTime").is_some() { s("createdTime") } else { s("created") };
                    // saveType is a number there (its names aren't known yet).
                    let kind = s("saveType");
                    let published = r.get("isPublished").and_then(|x| x.as_bool()).unwrap_or(false);
                    Some(VersionInfo {
                        version: num,
                        created,
                        published,
                        kind,
                    })
                })
                .collect()
        };
        let err = match first {
            Ok(v) => return Ok(rows_of(&v, "placeVersions")),
            Err(e) => e,
        };
        match self.get_json(
            &format!("{}/v1/assets/{place_id}/saved-versions?limit=50&sortOrder=Desc", self.ep.develop),
            "develop saved-versions",
        ) {
            Ok(v) => Ok(rows_of(&v, "data")),
            Err(_) => Err(Error(format!(
                "Roblox lists versions only to people who can edit this place ({err}); a version can still be opened by its number"
            ))),
        }
    }

    /// Team Create and edit access for a universe (all read-only):
    /// develop `v1/universes/multiget/teamcreate` (Studio's own call),
    /// team-create-service `GetCanCollaborate` (a POST that only reads),
    /// develop `v1/universes/{id}/permissions`.
    pub fn team_create(&self, universe_id: u64) -> TeamCreateStatus {
        let mut st = TeamCreateStatus::default();
        if let Ok(v) = self.get_json(
            &format!("{}/v1/universes/multiget/teamcreate?ids={universe_id}", self.ep.develop),
            "teamcreate",
        ) {
            st.enabled = v
                .get("data")
                .and_then(|d| d.as_array())
                .into_iter()
                .flatten()
                .find(|r| r.get("id").and_then(|i| i.as_u64()) == Some(universe_id))
                .and_then(|r| r.get("isEnabled").and_then(|b| b.as_bool()));
        }
        let body = format!("{{\"universeId\":{universe_id}}}");
        if let Ok((200, text)) = self.session.post_csrf(
            &format!("{}/team-create-service/v1/GetCanCollaborate", self.ep.apis),
            "application/json",
            body.as_bytes(),
        ) {
            if let Some(v) = json(&text) {
                st.can_collaborate = v.get("CanCollaborate").and_then(|b| b.as_bool());
                st.reason = v
                    .get("Error")
                    .and_then(|e| e.as_str())
                    .filter(|e| !e.is_empty() && *e != "None")
                    .map(str::to_string);
            }
        }
        if let Ok(v) = self.get_json(
            &format!("{}/v1/universes/{universe_id}/permissions", self.ep.develop),
            "permissions",
        ) {
            st.can_manage = v.get("canManage").and_then(|b| b.as_bool());
            st.can_cloud_edit = v.get("canCloudEdit").and_then(|b| b.as_bool());
        }
        st
    }

    /// Saves (Saved) or publishes (Published) the place as a new version.
    /// First the server's newest version is compared with the one opened:
    /// if someone saved in between, nothing is sent and the conflict comes
    /// back, unless `overwrite`. Needs the publishing key (Open Cloud Place
    /// Publishing); without it the reason is the error, nothing is sent.
    pub fn save_place(
        &self,
        origin: &CloudOrigin,
        bytes: &[u8],
        is_xml: bool,
        vt: VersionType,
        overwrite: bool,
    ) -> Result<SaveOutcome> {
        let Some(key) = self.publish_key.as_deref() else {
            return Err(Error(NEEDS_PUBLISH_KEY.into()));
        };
        let server = self.latest_version(origin.place_id)?;
        if !overwrite && server != origin.version {
            return Ok(SaveOutcome::Conflict {
                opened: origin.version,
                server,
            });
        }
        let url = format!(
            "{}/universes/v1/{}/places/{}/versions?versionType={}",
            self.ep.apis,
            origin.universe_id,
            origin.place_id,
            vt.as_str()
        );
        let content_type = if is_xml { "application/xml" } else { "application/octet-stream" };
        let mut r = crate::api::agent()
            .post(&url)
            .header("x-api-key", key)
            .header("Content-Type", content_type)
            .send(bytes)?;
        let status = r.status().as_u16();
        let text = r.body_mut().read_to_string()?;
        if status >= 400 {
            let detail: String = text.chars().take(300).collect();
            let hint = match status {
                401 | 403 => " (the key needs universe-places:write for this experience, and the IP it allows)",
                409 => " (Roblox has a newer version or the place is locked)",
                _ => "",
            };
            return Err(Error(format!(
                "Place Publishing answered HTTP {status}{hint}: {}",
                detail.trim()
            )));
        }
        let version = json(&text)
            .and_then(|v| v.get("versionNumber").and_then(|n| n.as_u64()))
            .or_else(|| text.trim().trim_matches('"').parse().ok())
            .ok_or_else(|| Error(format!("Place Publishing: no version number in {:?}", text.chars().take(120).collect::<String>())))?;
        Ok(SaveOutcome::Saved { version })
    }
}

/// A place reference typed or pasted by the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceRef {
    pub place_id: u64,
    pub universe_id: Option<u64>,
}

/// Reads a bare place id, `roblox.com/games/<id>/…` (any locale prefix,
/// `games/start?placeId=<id>`), or `create.roblox.com/…/experiences/<u>/places/<id>/…`.
pub fn parse_place_ref(input: &str) -> std::result::Result<PlaceRef, String> {
    let s = input.trim().trim_matches(|c| c == '<' || c == '>' || c == '"');
    let bad = || {
        format!(
            "{s:?} isn't a place ID or link: paste a roblox.com/games/… or create.roblox.com/…/places/… link, or the number"
        )
    };
    let num = |t: &str| t.parse::<u64>().ok().filter(|n| *n > 0);
    if let Some(n) = num(s) {
        return Ok(PlaceRef {
            place_id: n,
            universe_id: None,
        });
    }
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    // A `#fragment` is never part of the path or query (the game page's
    // tabs add `#!/about`, `#!/store`, ...).
    let rest = rest.split_once('#').map_or(rest, |(r, _)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_ascii_lowercase();
    if !(host == "roblox.com" || host.ends_with(".roblox.com")) {
        return Err(bad());
    }
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    for kv in query.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            if k.eq_ignore_ascii_case("placeId") {
                if let Some(n) = num(v) {
                    return Ok(PlaceRef {
                        place_id: n,
                        universe_id: None,
                    });
                }
            }
        }
    }
    let segs: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
    let after = |name: &str| {
        segs.iter()
            .position(|x| x.eq_ignore_ascii_case(name))
            .and_then(|i| segs.get(i + 1))
            .and_then(|x| num(x))
    };
    if let Some(p) = after("places") {
        return Ok(PlaceRef {
            place_id: p,
            universe_id: after("experiences"),
        });
    }
    if let Some(p) = after("games") {
        return Ok(PlaceRef {
            place_id: p,
            universe_id: None,
        });
    }
    Err(bad())
}

/// A cloud place opened before (File > Open from Roblox > Recent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentPlace {
    pub origin: CloudOrigin,
    /// Unix seconds.
    pub opened_at: u64,
}

/// Downloaded places: `~/.cache/rustudio/places/<placeId>/v<N>.rbxl` with
/// `origin.json`, and `places/recent.json`. A place reopens from here when
/// Roblox can't be reached.
#[derive(Debug, Clone)]
pub struct PlaceCache {
    pub root: PathBuf,
}

/// Versions kept per place.
const KEEP_VERSIONS: usize = 5;
const KEEP_RECENT: usize = 20;

impl Default for PlaceCache {
    fn default() -> Self {
        let base = std::env::var("RUSTUDIO_CACHE_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::cache_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("rustudio")
            });
        PlaceCache {
            root: base.join("places"),
        }
    }
}

fn is_xml(bytes: &[u8]) -> bool {
    bytes.starts_with(b"<roblox") && !bytes.starts_with(b"<roblox!")
}

impl PlaceCache {
    pub fn dir(&self, place_id: u64) -> PathBuf {
        self.root.join(place_id.to_string())
    }

    fn file(&self, place_id: u64, version: u64, xml: bool) -> PathBuf {
        self.dir(place_id)
            .join(format!("v{version}.{}", if xml { "rbxlx" } else { "rbxl" }))
    }

    /// Keeps a version's bytes and the origin; returns the file.
    pub fn store(&self, origin: &CloudOrigin, bytes: &[u8]) -> Result<PathBuf> {
        let dir = self.dir(origin.place_id);
        std::fs::create_dir_all(&dir)?;
        let path = self.file(origin.place_id, origin.version, is_xml(bytes));
        std::fs::write(&path, bytes)?;
        std::fs::write(dir.join("origin.json"), serde_json::to_string_pretty(origin)?)?;
        let mut vs = self.cached_versions(origin.place_id);
        while vs.len() > KEEP_VERSIONS {
            let old = vs.remove(vs.len() - 1);
            if old != origin.version {
                for xml in [false, true] {
                    let _ = std::fs::remove_file(self.file(origin.place_id, old, xml));
                }
            }
        }
        Ok(path)
    }

    /// Cached version numbers, newest first.
    pub fn cached_versions(&self, place_id: u64) -> Vec<u64> {
        let mut out: Vec<u64> = std::fs::read_dir(self.dir(place_id))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                let stem = n.strip_suffix(".rbxl").or_else(|| n.strip_suffix(".rbxlx"))?;
                stem.strip_prefix('v')?.parse().ok()
            })
            .collect();
        out.sort_unstable_by(|a, b| b.cmp(a));
        out.dedup();
        out
    }

    /// A cached copy: the version asked for, else the newest. The origin
    /// carries the version of the bytes.
    pub fn load(&self, place_id: u64, version: Option<u64>) -> Option<(CloudOrigin, Vec<u8>, PathBuf)> {
        let text = std::fs::read_to_string(self.dir(place_id).join("origin.json")).ok()?;
        let mut origin: CloudOrigin = serde_json::from_str(&text).ok()?;
        let v = match version {
            Some(v) => v,
            None => *self.cached_versions(place_id).first()?,
        };
        let path = [false, true]
            .into_iter()
            .map(|xml| self.file(place_id, v, xml))
            .find(|p| p.exists())?;
        let bytes = std::fs::read(&path).ok()?;
        origin.version = v;
        Some((origin, bytes, path))
    }

    fn recent_path(&self) -> PathBuf {
        self.root.join("recent.json")
    }

    /// Recently opened cloud places, newest first.
    pub fn recent(&self) -> Vec<RecentPlace> {
        std::fs::read_to_string(self.recent_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Puts a place at the top of Recent.
    pub fn remember(&self, origin: &CloudOrigin) -> Result<()> {
        let mut list = self.recent();
        list.retain(|r| r.origin.place_id != origin.place_id);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        list.insert(
            0,
            RecentPlace {
                origin: origin.clone(),
                opened_at: now,
            },
        );
        list.truncate(KEEP_RECENT);
        std::fs::create_dir_all(&self.root)?;
        std::fs::write(self.recent_path(), serde_json::to_string_pretty(&list)?)?;
        Ok(())
    }

    pub fn forget(&self, place_id: u64) -> Result<()> {
        let mut list = self.recent();
        list.retain(|r| r.origin.place_id != place_id);
        std::fs::create_dir_all(&self.root)?;
        std::fs::write(self.recent_path(), serde_json::to_string_pretty(&list)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockReply, MockRequest, MockServer};
    use crate::session::{Cookie, Profile};
    use std::sync::{Arc, Mutex};

    #[test]
    fn team_create_places_join_instead_of_downloading() {
        assert_eq!(open_mode(Some(true), false), OpenMode::JoinTeamCreate);
        assert_eq!(open_mode(Some(true), true), OpenMode::Download);
        assert_eq!(open_mode(Some(false), false), OpenMode::Download);
        assert_eq!(open_mode(None, false), OpenMode::Download);
    }

    #[test]
    fn place_links_parse() {
        let p = |s: &str| parse_place_ref(s).map(|r| (r.place_id, r.universe_id));
        assert_eq!(p("95206881"), Ok((95206881, None)));
        assert_eq!(p("  95206881 \n"), Ok((95206881, None)));
        assert_eq!(p("https://www.roblox.com/games/95206881/Baseplate"), Ok((95206881, None)));
        assert_eq!(p("roblox.com/games/95206881"), Ok((95206881, None)));
        assert_eq!(p("https://www.roblox.com/de/games/95206881/Baseplate?privateServerLinkCode=1"), Ok((95206881, None)));
        assert_eq!(p("https://www.roblox.com/games/start?placeId=95206881&launchData=x"), Ok((95206881, None)));
        assert_eq!(
            p("https://create.roblox.com/dashboard/creations/experiences/28220420/places/95206881/configure"),
            Ok((95206881, Some(28220420)))
        );
        assert_eq!(
            p("create.roblox.com/dashboard/creations/experiences/28220420/places/95206881/versions"),
            Ok((95206881, Some(28220420)))
        );
        assert!(p("").is_err());
        assert!(p("0").is_err());
        assert!(p("https://example.com/games/1").is_err());
        assert!(p("https://www.roblox.com/catalog").is_err());
        assert!(p("Baseplate").is_err());
    }

    #[test]
    fn place_links_ignore_the_fragment() {
        let p = |s: &str| parse_place_ref(s).map(|r| (r.place_id, r.universe_id));
        // The game page's tabs (About, Store, Servers) put `#!/...` in the URL.
        assert_eq!(p("https://www.roblox.com/games/95206881#!/about"), Ok((95206881, None)));
        assert_eq!(p("https://www.roblox.com/games/95206881/Baseplate#!/game-instances"), Ok((95206881, None)));
        assert_eq!(p("https://www.roblox.com/games/start?placeId=95206881#top"), Ok((95206881, None)));
        assert_eq!(
            p("https://create.roblox.com/dashboard/creations/experiences/28220420/places/95206881#versions"),
            Ok((95206881, Some(28220420)))
        );
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustudio-cloud-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn origin(version: u64) -> CloudOrigin {
        CloudOrigin {
            universe_id: 28220420,
            place_id: 95206881,
            version,
            name: "Baseplate".into(),
            experience: "Baseplate".into(),
            root_place_id: 95206881,
            creator_id: 998796,
            creator_type: CreatorKind::User,
            creator_name: "Templates".into(),
            ..Default::default()
        }
    }

    #[test]
    fn cache_keeps_versions_and_recent() {
        let root = tmp("cache");
        let c = PlaceCache { root: root.clone() };
        assert!(c.load(95206881, None).is_none());
        for v in 1..=7 {
            c.store(&origin(v), format!("<roblox!bytes{v}").as_bytes()).unwrap();
        }
        assert_eq!(c.cached_versions(95206881), vec![7, 6, 5, 4, 3]);
        let (o, b, path) = c.load(95206881, None).unwrap();
        assert_eq!((o.version, b), (7, b"<roblox!bytes7".to_vec()));
        assert!(path.ends_with("95206881/v7.rbxl"));
        assert_eq!(c.load(95206881, Some(5)).unwrap().0.version, 5);
        assert!(c.load(95206881, Some(1)).is_none());
        // XML bytes keep their extension.
        c.store(&origin(8), b"<roblox version=\"4\"></roblox>").unwrap();
        assert!(c.load(95206881, Some(8)).unwrap().2.ends_with("v8.rbxlx"));
        let mut other = origin(3);
        other.place_id = 42;
        c.remember(&origin(7)).unwrap();
        c.remember(&other).unwrap();
        c.remember(&origin(8)).unwrap();
        let r: Vec<u64> = c.recent().iter().map(|r| r.origin.place_id).collect();
        assert_eq!(r, vec![95206881, 42]);
        assert_eq!(c.recent()[0].origin.version, 8);
        c.forget(42).unwrap();
        assert_eq!(c.recent().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A pretend Roblox: place 95206881 is at `server` version; Open Cloud
    /// publishing answers the next version.
    fn roblox(server: Arc<Mutex<u64>>) -> MockServer {
        MockServer::start(Arc::new(move |r: &MockRequest| {
            let p = r.path.as_str();
            let v = *server.lock().unwrap();
            if p.starts_with("/assetdelivery/v2/assetId/95206881") {
                let base = p.trim_end_matches(|c: char| c.is_ascii_digit());
                let n = if base.ends_with("/version/") {
                    p.rsplit('/').next().unwrap().parse().unwrap()
                } else {
                    v
                };
                return MockReply::json(
                    200,
                    &format!("{{\"locations\":[{{\"location\":\"http://{}/cdn/{n}\"}}]}}", r.header("host").unwrap_or("")),
                )
                .with_header("roblox-assetversionnumber", &n.to_string());
            }
            if p.starts_with("/assetdelivery/v2/assetId/100000005") {
                return MockReply::json(200, r#"{"errors":[{"code":409,"message":"User is not authorized to access Asset.","customErrorCode":1}]}"#);
            }
            if let Some(n) = p.strip_prefix("/cdn/") {
                return MockReply {
                    status: 200,
                    headers: vec![],
                    body: format!("<roblox!v{n}").into_bytes(),
                };
            }
            if p.starts_with("/apis/universes/v1/28220420/places/95206881/versions") {
                let mut g = server.lock().unwrap();
                *g += 1;
                return MockReply::json(200, &format!("{{\"versionNumber\":{}}}", *g));
            }
            if p == "/apis/universes/v1/places/95206881/universe" {
                return MockReply::json(200, r#"{"universeId":28220420}"#);
            }
            if p == "/develop/v1/universes/28220420" {
                return MockReply::json(200, r#"{"id":28220420,"name":"Baseplate","rootPlaceId":95206881,"creatorType":"User","creatorTargetId":998796,"creatorName":"Templates"}"#);
            }
            if p.starts_with("/games/v1/games/multiget-place-details") {
                return MockReply::json(200, r#"[{"placeId":95206881,"name":"Baseplate","universeId":28220420}]"#);
            }
            if p.starts_with("/develop/v1/universes/multiget/teamcreate") {
                return MockReply::json(200, r#"{"data":[{"id":28220420,"isEnabled":true}]}"#);
            }
            if p == "/apis/team-create-service/v1/GetCanCollaborate" {
                if r.header("x-csrf-token").is_none() {
                    return MockReply::json(403, r#"{"errors":[{"code":0,"message":"XSRF token invalid"}]}"#)
                        .with_header("x-csrf-token", "tok");
                }
                return MockReply::json(200, r#"{"CanCollaborate":true,"Error":"None"}"#);
            }
            if p == "/develop/v1/universes/28220420/permissions" {
                return MockReply::json(200, r#"{"canManage":true,"canCloudEdit":true}"#);
            }
            if p.starts_with("/apis/universes/v1/search") {
                return MockReply::json(200, r#"{"data":[{"id":5,"name":"Shared One","rootPlaceId":6,"creatorType":"User","creatorTargetId":7,"creatorName":"friend","updated":"2026-01-01T00:00:00Z","isTeamCreateEnabled":true}],"totalResults":1}"#);
            }
            if p.starts_with("/groups/v2/users/1/groups/roles") {
                return MockReply::json(200, r#"{"data":[{"group":{"id":10,"name":"Builders"},"role":{"name":"Dev","rank":200}},{"group":{"id":11,"name":"Fans"},"role":{"name":"Member","rank":1}}]}"#);
            }
            if p == "/groups/v1/groups/10/membership" {
                return MockReply::json(200, r#"{"permissions":{"groupEconomyPermissions":{"manageGroupGames":true}}}"#);
            }
            if p == "/groups/v1/groups/11/membership" {
                return MockReply::json(200, r#"{"permissions":{"groupEconomyPermissions":{"manageGroupGames":false}}}"#);
            }
            if p.starts_with("/apis/place-version-history-api/v1/95206881/history") {
                return MockReply::json(200, r#"{"placeVersions":[{"version":"72","createdTime":"2026-11-04T00:00:00Z","saveType":1,"isPublished":false,"publishStatus":0},{"version":"71","createdTime":"2026-11-03T00:00:00Z","saveType":2,"isPublished":true,"publishStatus":1}],"hasMore":false}"#);
            }
            MockReply::json(404, r#"{"errors":[{"code":0,"message":"mock: no route"}]}"#)
        }))
        .unwrap()
    }

    fn api(server: &MockServer, key: Option<&str>) -> CloudApi {
        let session = Session::unchecked(
            Cookie::new("mock-session-not-a-real-cookie").unwrap(),
            Profile {
                id: 1,
                name: "tester".into(),
                display_name: "tester".into(),
            },
        );
        CloudApi::new(session, Endpoints::mock(&server.base), key.map(str::to_string))
    }

    #[test]
    fn open_resolves_downloads_and_reads_team_create() {
        let v = Arc::new(Mutex::new(71));
        let s = roblox(v.clone());
        let a = api(&s, None);
        let o = a.resolve(95206881).unwrap();
        assert_eq!((o.universe_id, o.creator_id, o.creator_type), (28220420, 998796, CreatorKind::User));
        assert_eq!((o.name.as_str(), o.experience.as_str(), o.root_place_id), ("Baseplate", "Baseplate", 95206881));
        assert_eq!(a.download(95206881, None).unwrap(), (b"<roblox!v71".to_vec(), 71));
        assert_eq!(a.download(95206881, Some(3)).unwrap(), (b"<roblox!v3".to_vec(), 3));
        assert_eq!(a.latest_version(95206881).unwrap(), 71);
        let e = a.download(100000005, None).unwrap_err().0;
        assert!(e.contains("won't give this account place 100000005") && e.contains("not authorized"), "{e}");
        let tc = a.team_create(28220420);
        assert_eq!((tc.enabled, tc.can_collaborate, tc.can_edit()), (Some(true), Some(true), Some(true)));
        assert!(tc.describe().starts_with("Team Create is on: live co-editing isn't supported yet"));
        let vs = a.versions(95206881).unwrap();
        assert_eq!(vs.iter().map(|v| v.version).collect::<Vec<_>>(), vec![72, 71]);
        assert!(vs[1].published && !vs[0].published);
        let shared = a.shared_experiences().unwrap();
        assert_eq!((shared[0].id, shared[0].name.as_str(), shared[0].team_create), (5, "Shared One", Some(true)));
        let groups = a.groups().unwrap();
        assert_eq!(groups.iter().map(|g| (g.id, g.can_edit)).collect::<Vec<_>>(), vec![(10, Some(true)), (11, Some(false))]);
        // The shared search asks as Studio's Start Page does.
        assert!(s.requests().iter().any(|r| r.path.contains("creatorType=Team&creatorTargetId=1&")));
    }

    #[test]
    fn experience_pages_ask_by_source_and_stop_at_the_total() {
        let s = roblox(Arc::new(Mutex::new(71)));
        let a = api(&s, None);
        let (rows, more) = a.experiences_page(ExperienceSource::Mine, 0, 25).unwrap();
        assert_eq!((rows.len(), rows[0].id, more), (1, 5, false));
        a.experiences_page(ExperienceSource::Shared, 0, 25).unwrap();
        a.experiences_page(ExperienceSource::Group(10), 2, 25).unwrap();
        let paths: Vec<String> = s.requests().into_iter().map(|r| r.path).collect();
        for want in [
            "creatorType=User&creatorTargetId=1&",
            "creatorType=Team&creatorTargetId=1&",
            "creatorType=Group&creatorTargetId=10&",
        ] {
            assert!(paths.iter().any(|p| p.contains(want) && p.contains("sortParam=LastUpdated&sortOrder=Desc")), "{want}");
        }
        assert!(paths.iter().any(|p| p.contains("creatorType=Group") && p.contains("PageIndex=2")));
    }

    #[test]
    fn save_checks_the_server_version_first() {
        let v = Arc::new(Mutex::new(71));
        let s = roblox(v.clone());
        let posts = |s: &MockServer| s.requests().into_iter().filter(|r| r.method == "POST" && r.path.contains("/versions")).collect::<Vec<_>>();
        // No key: a clear reason, nothing sent anywhere.
        let none = api(&s, None);
        let e = none.save_place(&origin(71), b"<roblox!x", false, VersionType::Saved, false).unwrap_err().0;
        assert!(e.contains("Open Cloud API key") && e.contains("universe-places:write"), "{e}");
        assert!(posts(&s).is_empty());
        // Same version on the server: saved as the next version.
        let a = api(&s, Some("test-key"));
        let out = a.save_place(&origin(71), b"<roblox!x", false, VersionType::Saved, false).unwrap();
        assert_eq!(out, SaveOutcome::Saved { version: 72 });
        let p = posts(&s);
        assert_eq!(p.len(), 1);
        assert!(p[0].path.ends_with("versionType=Saved"));
        assert_eq!(p[0].header("x-api-key"), Some("test-key"));
        assert_eq!(p[0].header("content-type"), Some("application/octet-stream"));
        assert_eq!(p[0].body, b"<roblox!x");
        // Someone else saved (server 73, opened 72): conflict, nothing sent.
        *v.lock().unwrap() = 73;
        let out = a.save_place(&origin(72), b"<roblox!y", false, VersionType::Saved, false).unwrap();
        assert_eq!(out, SaveOutcome::Conflict { opened: 72, server: 73 });
        assert_eq!(posts(&s).len(), 1);
        // Overwrite: sent anyway, as a published version.
        let out = a.save_place(&origin(72), b"<roblox!y", false, VersionType::Published, true).unwrap();
        assert_eq!(out, SaveOutcome::Saved { version: 74 });
        let p = posts(&s);
        assert_eq!(p.len(), 2);
        assert!(p[1].path.ends_with("versionType=Published"));
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::session::{load_cookie, Session};

    /// Read-only probe of every cloud call with the person's session
    /// (`cargo test -p rustudio-account -- --ignored cloud::live`). Nothing
    /// is saved: `save_place` is never called. Prints names and numbers only.
    #[test]
    #[ignore]
    fn probe_cloud_places() {
        let Some((c, _)) = load_cookie(false).expect("cookie") else {
            println!("no session");
            return;
        };
        let s = Session::validate(c).expect("session");
        println!("signed in as {} ({})", s.profile.display(), s.profile.id);
        let a = CloudApi::new(s, Endpoints::default(), None);
        let show = |what: &str, r: Result<Vec<Experience>>| match r {
            Ok(v) => println!("{what}: {:?}", v.iter().map(|e| (e.id, e.name.as_str(), e.root_place_id, e.team_create)).collect::<Vec<_>>()),
            Err(e) => println!("{what}: {e}"),
        };
        show("my experiences", a.my_experiences());
        match a.groups() {
            Ok(g) => {
                println!("groups: {:?}", g.iter().map(|g| (g.id, &g.name, &g.role, g.can_edit)).collect::<Vec<_>>());
                for g in g {
                    show(&format!("group {}", g.id), a.group_experiences(g.id));
                }
            }
            Err(e) => println!("groups: {e}"),
        }
        let shared = a.shared_experiences();
        show("shared with me", shared.clone());
        let o = a.resolve(95206881).expect("Baseplate");
        println!("Baseplate: {o:?}, latest version {:?}", a.latest_version(95206881));
        println!("Baseplate versions: {:?}", a.versions(95206881).map(|v| v.len()));
        println!("Baseplate v1: {:?}", a.download(95206881, Some(1)).map(|(b, v)| (b.len(), v)));
        println!("Baseplate team create: {:?}", a.team_create(28220420));
        println!("icons: {:?}", a.icon_urls(&[28220420]).map(|v| v.len()));
        println!("no access: {:?}", a.download(100000005, None).map(|(b, _)| b.len()));
        println!("no such place: {:?}", a.resolve(1).map(|o| o.universe_id));
        for e in shared.unwrap_or_default() {
            let Some(root) = e.root_place_id else { continue };
            println!("{} places: {:?}", e.name, a.places(e.id).map(|p| p.len()));
            println!("{} latest version: {:?}", e.name, a.latest_version(root));
            match a.versions(root) {
                Ok(v) => println!("{} versions: {} listed, newest {:?}", e.name, v.len(), v.first()),
                Err(err) => println!("{} versions: {err}", e.name),
            }
            println!("{} team create: {:?}", e.name, a.team_create(e.id));
        }
    }
}
