//! Roblox web lookups for Player / Players, through the signed-in login
//! session (the `.ROBLOSECURITY` session, never an API key), on worker
//! threads so the calling script yields (players_chat.luau polls). Only
//! reads: groups, friends, users. Endpoints are the ones Studio's engine
//! names (Studio strings: `/v1/users/%s/friends/online`,
//! `/v1/users/%s/friends/statuses`, `/v1/usernames/users`, `v1/users/%lld`,
//! the groups roles list), on their public hosts.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value as J;

pub enum Poll {
    Ready(J),
    Pending,
    Failed(String),
}

type Job = Box<dyn FnOnce(&rustudio_account::Client) -> Result<J, String> + Send>;

static STATE: Mutex<Option<HashMap<String, Option<Result<J, String>>>>> = Mutex::new(None);

/// Starts `job` once per `key` (answers are kept for the process, as
/// Studio caches them) and reports where it is.
pub fn poll(key: &str, job: Job) -> Poll {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    match map.get(key) {
        Some(Some(Ok(v))) => return Poll::Ready(v.clone()),
        // A failure is reported once; the next call asks again.
        Some(Some(Err(e))) => {
            let e = e.clone();
            map.remove(key);
            return Poll::Failed(e);
        }
        Some(None) => return Poll::Pending,
        None => {}
    }
    map.insert(key.to_string(), None);
    let key = key.to_string();
    std::thread::spawn(move || {
        let r = (|| {
            if !crate::assets::wait_for_session(std::time::Duration::from_secs(20)) {
                return Err("HTTP 401 (Unauthorized)".to_string());
            }
            let shared = crate::assets::session_client().ok_or("HTTP 401 (Unauthorized)")?;
            let guard = shared.lock().map_err(|_| "account busy".to_string())?;
            let c = guard.as_ref().ok_or("HTTP 401 (Unauthorized)")?;
            job(c)
        })();
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let map = g.get_or_insert_with(HashMap::new);
        map.insert(key, Some(r));
    });
    Poll::Pending
}

fn status_text(code: u16) -> String {
    let reason = match code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    };
    format!("HTTP {code} ({reason})")
}

pub fn get_json(c: &rustudio_account::Client, url: &str) -> Result<J, String> {
    let (status, body) = c.session.get(url).map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(status_text(status));
    }
    serde_json::from_str(&body).map_err(|_| "Invalid response received".to_string())
}

pub fn post_json(c: &rustudio_account::Client, url: &str, body: &J) -> Result<J, String> {
    let bytes = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let (status, text) = c
        .session
        .post_csrf(url, "application/json", &bytes)
        .map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(status_text(status));
    }
    serde_json::from_str(&text).map_err(|_| "Invalid response received".to_string())
}

/// The user's groups: [{ id, rank, role }].
pub fn groups(uid: i64) -> Job {
    Box::new(move |c| {
        let v = get_json(c, &format!("https://groups.roblox.com/v2/users/{uid}/groups/roles"))?;
        let rows = v["data"].as_array().cloned().unwrap_or_default();
        Ok(J::Array(
            rows.iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r["group"]["id"],
                        "rank": r["role"]["rank"],
                        "role": r["role"]["name"],
                    })
                })
                .collect(),
        ))
    })
}

/// Whether `uid` and `other` are friends (friends/statuses).
pub fn friend_status(uid: i64, other: i64) -> Job {
    Box::new(move |c| {
        let v = get_json(
            c,
            &format!("https://friends.roblox.com/v1/users/{uid}/friends/statuses?userIds={other}"),
        )?;
        let friends = v["data"]
            .as_array()
            .and_then(|a| a.iter().find(|r| r["id"].as_i64() == Some(other)))
            .is_some_and(|r| r["status"].as_str() == Some("Friends"));
        Ok(J::Bool(friends))
    })
}

/// Player:GetFriendsOnline's records, as the engine builds them from
/// `/v1/users/%s/friends/online` (VisitorId, UserName, DisplayName,
/// LastOnline, IsOnline, LastLocation, PlaceId, GameId, LocationType).
pub fn friends_online(uid: i64) -> Job {
    Box::new(move |c| {
        let v = get_json(
            c,
            &format!("https://friends.roblox.com/v1/users/{uid}/friends/online?origin=getFriendsOnline"),
        )?;
        let rows = v["data"].as_array().cloned().unwrap_or_default();
        let mut out = Vec::new();
        for r in rows {
            let p = &r["userPresence"];
            // Enum UserLocationType: 0 MobileWebsite, 1 MobileInGame,
            // 2 Webpage, 3 Studio, 4 InGame, 5 XBoxApp, 6 TeamCreate.
            let loc = match p["UserPresenceType"].as_str().unwrap_or("") {
                "InGame" => 4,
                "InStudio" => 3,
                _ => 2,
            };
            let mut e = serde_json::json!({
                "VisitorId": r["id"],
                "UserName": r["name"],
                "DisplayName": r["displayName"],
                "LastOnline": p["lastOnline"].as_str().unwrap_or(""),
                "IsOnline": true,
                "LastLocation": p["lastLocation"].as_str().unwrap_or(""),
                "LocationType": loc,
            });
            if let Some(pl) = p["placeId"].as_i64() {
                e["PlaceId"] = pl.into();
            }
            if let Some(g) = p["gameInstanceId"].as_str() {
                e["GameId"] = g.into();
            }
            out.push(e);
        }
        Ok(J::Array(out))
    })
}

/// Every friend of `uid`: [{ Id, Username, DisplayName, IsOnline }]
/// (FriendPages' records), from friends/find (ids, by cursor), the users
/// lookup for names and friends/online for IsOnline.
pub fn friends(uid: i64) -> Job {
    Box::new(move |c| {
        let mut ids: Vec<i64> = Vec::new();
        let mut cursor = String::new();
        for _ in 0..40 {
            let url = format!(
                "https://friends.roblox.com/v1/users/{uid}/friends/find?limit=50{}",
                if cursor.is_empty() { String::new() } else { format!("&cursor={cursor}") }
            );
            let v = get_json(c, &url).map_err(|e| format!("FriendPages: Request Failed because {e}"))?;
            for r in v["PageItems"].as_array().into_iter().flatten() {
                if let Some(id) = r["id"].as_i64() {
                    ids.push(id);
                }
            }
            match v["NextCursor"].as_str() {
                Some(n) if !n.is_empty() => cursor = n.to_string(),
                _ => break,
            }
        }
        let mut names: HashMap<i64, (String, String)> = HashMap::new();
        for chunk in ids.chunks(100) {
            let v = post_json(
                c,
                "https://users.roblox.com/v1/users",
                &serde_json::json!({ "userIds": chunk, "excludeBannedUsers": false }),
            )
            .map_err(|e| format!("FriendPages: Request Failed because {e}"))?;
            for r in v["data"].as_array().into_iter().flatten() {
                if let Some(id) = r["id"].as_i64() {
                    names.insert(
                        id,
                        (
                            r["name"].as_str().unwrap_or("").to_string(),
                            r["displayName"].as_str().unwrap_or("").to_string(),
                        ),
                    );
                }
            }
        }
        let online: Vec<i64> = get_json(
            c,
            &format!("https://friends.roblox.com/v1/users/{uid}/friends/online"),
        )
        .ok()
        .and_then(|v| v["data"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r["id"].as_i64())
        .collect();
        Ok(J::Array(
            ids.iter()
                .map(|id| {
                    let (n, d) = names.get(id).cloned().unwrap_or_default();
                    serde_json::json!({ "Id": id, "Username": n, "DisplayName": d, "IsOnline": online.contains(id) })
                })
                .collect(),
        ))
    })
}

/// A user's id from their name (POST users.roblox.com/v1/usernames/users).
pub fn user_id_from_name(name: String) -> Job {
    Box::new(move |c| {
        let v = post_json(
            c,
            "https://users.roblox.com/v1/usernames/users",
            &serde_json::json!({ "usernames": [name], "excludeBannedUsers": false }),
        )?;
        v["data"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|r| r["id"].as_i64())
            .map(J::from)
            .ok_or_else(|| "Unknown user".to_string())
    })
}

/// A user's name from their id (users.roblox.com/v1/users/{id}).
pub fn name_from_user_id(id: i64) -> Job {
    Box::new(move |c| {
        let (status, body) = c
            .session
            .get(&format!("https://users.roblox.com/v1/users/{id}"))
            .map_err(|e| e.to_string())?;
        if status == 404 || status == 400 {
            return Err("Unknown user".into());
        }
        if status != 200 {
            return Err(status_text(status));
        }
        let v: J = serde_json::from_str(&body).map_err(|_| "Invalid response received".to_string())?;
        v["name"]
            .as_str()
            .map(|s| J::String(s.to_string()))
            .ok_or_else(|| "Unknown user".to_string())
    })
}

/// Days since 1970-01-01 for an ISO-8601 date ("2015-03-19T01:02:03.4Z").
fn days_of(iso: &str) -> Option<i64> {
    let d = iso.get(..10)?;
    let mut it = d.split('-');
    let (y, m, dd): (i64, i64, i64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    // (Howard Hinnant's days_from_civil.)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// The signed-in account's Player fields a signed-in Studio Play gives the
/// local player: AccountAge (whole days since `created`), HasVerifiedBadge
/// (users.roblox.com/v1/users/{id}) and MembershipType (Premium from
/// premiumfeatures' validate-membership, the check Studio's engine names
/// `/v1/users/%lld/validate-membership`).
pub fn identity(uid: i64) -> Job {
    Box::new(move |c| {
        let u = get_json(c, &format!("https://users.roblox.com/v1/users/{uid}"))?;
        let now_days = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 / 86400)
            .unwrap_or(0);
        let age = u["created"].as_str().and_then(days_of).map(|d| (now_days - d).max(0)).unwrap_or(0);
        let premium = get_json(c, &format!("https://premiumfeatures.roblox.com/v1/users/{uid}/validate-membership"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(serde_json::json!({
            "AccountAge": age,
            "HasVerifiedBadge": u["hasVerifiedBadge"].as_bool().unwrap_or(false),
            "Premium": premium,
        }))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_days() {
        assert_eq!(super::days_of("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(super::days_of("2000-03-01T00:00:00.000Z"), Some(11017));
        assert_eq!(super::days_of("2026-11-08T12:00:00Z"), Some(20765));
    }
}
