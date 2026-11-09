//! Avatar descriptions from Roblox's documented avatar web API (public; no
//! cookie): `GET https://avatar.roblox.com/v2/avatar/users/{userId}/avatar`
//! (rig type, scales, body colours, worn assets), and the username → id
//! lookup `POST https://users.roblox.com/v1/usernames/users`.

use serde::Serialize;

use crate::api::agent;
use crate::{Error, Result};

pub const AVATAR_URL: &str = "https://avatar.roblox.com/v2/avatar/users";
pub const USERNAMES_URL: &str = "https://users.roblox.com/v1/usernames/users";

#[derive(Debug, Clone, Serialize)]
pub struct WornAsset {
    pub id: u64,
    pub name: String,
    /// Enum.AssetType value (e.g. 11 Shirt, 12 Pants, 8 Hat).
    pub type_id: u32,
    pub type_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AvatarInfo {
    pub user_id: u64,
    /// playerAvatarType: R15 (3) or R6 (1).
    pub r15: bool,
    pub height: f32,
    pub width: f32,
    pub depth: f32,
    pub head: f32,
    /// head, torso, left arm, right arm, left leg, right leg (RGB).
    pub colors: [[u8; 3]; 6],
    pub assets: Vec<WornAsset>,
}

fn hex(v: Option<&serde_json::Value>) -> Option<[u8; 3]> {
    let s = v?.as_str()?.trim_start_matches('#');
    let n = u32::from_str_radix(s, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// Parses the v2 avatar JSON (also accepts the older v1 field names).
pub fn parse_avatar(user_id: u64, v: &serde_json::Value) -> AvatarInfo {
    // Studio's avatar-fetch reply names the rig Roblox resolved for the
    // place as resolvedAvatarType ("R6" / "R15"; AvatarFetchParser::parseUseR15
    //); the other avatar endpoints say playerAvatarType.
    let r15 = match v
        .get("resolvedAvatarType")
        .or_else(|| v.get("playerAvatarType"))
    {
        Some(serde_json::Value::Number(n)) => n.as_u64() != Some(1),
        Some(serde_json::Value::String(s)) => s != "R6",
        _ => true,
    };
    let scale = |k: &str| {
        v.pointer(&format!("/scales/{k}"))
            .and_then(|x| x.as_f64())
            .unwrap_or(1.0) as f32
    };
    let c = v.get("bodyColor3s").or_else(|| v.get("bodyColors"));
    let col = |k3: &str, k: &str| {
        hex(c.and_then(|c| c.get(k3).or_else(|| c.get(k)))).unwrap_or([163, 162, 165])
    };
    let colors = [
        col("headColor3", "headColor"),
        col("torsoColor3", "torsoColor"),
        col("leftArmColor3", "leftArmColor"),
        col("rightArmColor3", "rightArmColor"),
        col("leftLegColor3", "leftLegColor"),
        col("rightLegColor3", "rightLegColor"),
    ];
    // avatar-fetch lists worn items as assetAndAssetTypeIds (ids only).
    let fetched: Vec<WornAsset> = v
        .get("assetAndAssetTypeIds")
        .and_then(|a| a.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(WornAsset {
                        id: a.get("assetId")?.as_u64()?,
                        name: String::new(),
                        type_id: a.get("assetTypeId").and_then(|i| i.as_u64()).unwrap_or(0) as u32,
                        type_name: String::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let assets: Vec<WornAsset> = v
        .get("assets")
        .and_then(|a| a.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(WornAsset {
                        id: a.get("id")?.as_u64()?,
                        name: a
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string(),
                        type_id: a
                            .pointer("/assetType/id")
                            .and_then(|i| i.as_u64())
                            .unwrap_or(0) as u32,
                        type_name: a
                            .pointer("/assetType/name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    AvatarInfo {
        user_id,
        r15,
        height: scale("height"),
        width: scale("width"),
        depth: scale("depth"),
        head: scale("head"),
        colors,
        assets: if assets.is_empty() { fetched } else { assets },
    }
}

/// Studio's avatar request (sub_1407db6c0):
/// `GET avatar.roblox.com/v2/avatar/avatar-fetch/?placeId={p}&userId={u}`.
pub fn avatar_fetch_url(place_id: u64, user_id: u64) -> String {
    format!("https://avatar.roblox.com/v2/avatar/avatar-fetch/?placeId={place_id}&userId={user_id}")
}

/// Fetches a user's current avatar description.
pub fn fetch_avatar(user_id: u64) -> Result<AvatarInfo> {
    let mut r = agent()
        .get(&format!("{AVATAR_URL}/{user_id}/avatar"))
        .call()?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string()?;
    if status >= 400 {
        return Err(Error(format!(
            "avatar lookup for user {user_id} returned HTTP {status}"
        )));
    }
    Ok(parse_avatar(user_id, &serde_json::from_str(&text)?))
}

/// Resolves a username to a user id.
pub fn user_id_for(username: &str) -> Result<u64> {
    let body = serde_json::json!({ "usernames": [username], "excludeBannedUsers": true });
    let mut r = agent().post(USERNAMES_URL).send_json(&body)?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string()?;
    if status >= 400 {
        return Err(Error(format!("username lookup returned HTTP {status}")));
    }
    let v: serde_json::Value = serde_json::from_str(&text)?;
    v.pointer("/data/0/id")
        .and_then(|i| i.as_u64())
        .ok_or_else(|| Error(format!("no Roblox user named {username}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-only probe of an avatar web endpoint with the signed-in session:
    /// `AVATAR_PROBE_URL='https://…{uid}…' cargo test -p rustudio-account
    /// probe_avatar_endpoint -- --ignored --nocapture` (AVATAR_PROBE_BODY
    /// makes it a POST, for read-only batch lookups). Prints the status and
    /// the start of the reply; never the cookie.
    #[test]
    #[ignore]
    fn probe_avatar_endpoint() {
        let Ok(url) = std::env::var("AVATAR_PROBE_URL") else { return };
        let (cookie, _) = crate::session::load_cookie(true).unwrap().expect("signed in");
        let s = crate::session::Session::validate(cookie).unwrap();
        let url = url.replace("{uid}", &s.profile.id.to_string());
        let (status, text) = match std::env::var("AVATAR_PROBE_BODY") {
            Ok(b) => s.post_csrf(&url, "application/json", b.as_bytes()).unwrap(),
            Err(_) => s.get(&url).unwrap(),
        };
        let n = std::env::var("AVATAR_PROBE_CHARS").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        println!("STATUS {status}\n{}", text.chars().take(n).collect::<String>());
    }

    #[test]
    fn parses_v2_avatar() {
        let v = serde_json::json!({
            "scales": {"height": 1.05, "width": 0.9, "depth": 1.0, "head": 1.0, "proportion": 0, "bodyType": 0},
            "playerAvatarType": 3,
            "bodyColor3s": {"headColor3": "#FFCC99", "torsoColor3": "#0D69AC", "rightArmColor3": "FFCC99", "leftArmColor3": "#FFCC99", "rightLegColor3": "#A4BD47", "leftLegColor3": "#A4BD47"},
            "assets": [{"id": 607785314, "name": "Shirt", "assetType": {"id": 11, "name": "Shirt"}, "currentVersionId": 1}]
        });
        let a = parse_avatar(7, &v);
        assert!(a.r15);
        assert_eq!(a.colors[1], [0x0d, 0x69, 0xac]);
        assert_eq!(a.assets[0].type_id, 11);
        assert!((a.height - 1.05).abs() < 1e-6);
    }
}
