//! Studio's session-authenticated experience API-access check.
//! Recovered from Studio  ->  (URL) and
//!  (isStudioAccessToApisAllowed boolean). No Open Cloud key.
use crate::{Error, Result, Session};

pub const UNPUBLISHED: &str = "You must publish this place to the web to access DataStore.";
pub const WRITE_DISABLED: &str =
    "403: Cannot write to DataStore from studio if API access is not enabled.";
/// A mockable read boundary. Session attaches its cookie internally; callers
/// never receive the credential and implementations must never log headers.
pub trait AccessHttp {
    fn get(&self, url: &str) -> Result<(u16, String)>;
}
impl AccessHttp for Session {
    fn get(&self, url: &str) -> Result<(u16, String)> {
        Session::get(self, url)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct StudioAccess {
    pub allowed: bool,
}
impl StudioAccess {
    pub fn fetch(http: &impl AccessHttp, place_id: u64, universe_id: u64) -> Result<Self> {
        if place_id == 0 {
            return Err(Error::new(UNPUBLISHED));
        }
        if universe_id == 0 {
            return Err(Error::new(
                "Invalid universe_id. universe_id cannot be 0 or negative",
            ));
        }
        let (status, body) = http.get(&format!(
            "https://develop.roblox.com/v1/universes/{universe_id}/configuration"
        ))?;
        if status != 200 {
            return Err(Error::new(format!(
                "Studio API access check failed (HTTP {status})"
            )));
        }
        let v: serde_json::Value = serde_json::from_str(&body)
            .map_err(|_| Error::new("Studio API access check returned invalid JSON"))?;
        // Studio treats a missing/nonboolean value as disabled, never as opt-in.
        Ok(Self {
            allowed: v
                .get("isStudioAccessToApisAllowed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }
    pub fn require_write(self) -> Result<()> {
        if self.allowed {
            Ok(())
        } else {
            Err(Error::new(WRITE_DISABLED))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Mock {
        status: u16,
        body: &'static str,
    }
    impl AccessHttp for Mock {
        fn get(&self, url: &str) -> Result<(u16, String)> {
            assert_eq!(
                url,
                "https://develop.roblox.com/v1/universes/123/configuration"
            );
            Ok((self.status, self.body.into()))
        }
    }
    #[test]
    fn studio_access_is_explicit_opt_in() {
        for (body, allowed) in [
            (r#"{"isStudioAccessToApisAllowed":true}"#, true),
            (r#"{"isStudioAccessToApisAllowed":false}"#, false),
            (r#"{}"#, false),
            (r#"{"isStudioAccessToApisAllowed":"true"}"#, false),
        ] {
            let a = StudioAccess::fetch(&Mock { status: 200, body }, 12, 123).unwrap();
            assert_eq!(a.allowed, allowed);
            assert_eq!(a.require_write().is_ok(), allowed);
        }
    }
    #[test]
    fn unpublished_does_not_request_http() {
        let e = StudioAccess::fetch(
            &Mock {
                status: 500,
                body: "",
            },
            0,
            123,
        )
        .unwrap_err();
        assert_eq!(e.0, UNPUBLISHED);
    }
    #[test]
    fn failed_checks_never_grant_access() {
        assert!(StudioAccess::fetch(
            &Mock {
                status: 403,
                body: "{}"
            },
            12,
            123
        )
        .is_err());
        assert!(StudioAccess::fetch(
            &Mock {
                status: 200,
                body: "not JSON"
            },
            12,
            123
        )
        .is_err());
    }
}

/// Context headers captured from an actual Studio server Play request.
#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub place: u64,
    pub universe: u64,
    pub players: usize,
}
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Request {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, deserialize_with = "request_headers")]
    pub headers: std::collections::BTreeMap<String, String>,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct Response {
    pub status: u16,
    pub headers: std::collections::BTreeMap<String, String>,
    pub body: String,
}
fn request_headers<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<std::collections::BTreeMap<String, String>, D::Error> {
    use serde::Deserialize;
    let v = serde_json::Value::deserialize(d)?;
    if v.as_array().is_some_and(|a| a.is_empty()) {
        return Ok(Default::default());
    }
    serde_json::from_value(v).map_err(serde::de::Error::custom)
}
impl Request {
    /// All credentials stay inside Session. A path cannot select a different
    /// universe from the engine's published DataModel context.
    pub fn execute(&self, session: &Session, context: Context) -> Result<Response> {
        use base64::Engine;
        use md5::{Digest, Md5};
        if context.place == 0 {
            return Err(Error::new(UNPUBLISHED));
        }
        if context.universe == 0 {
            return Err(Error::new(
                "Invalid universe_id. universe_id cannot be 0 or negative",
            ));
        }
        let prefix = format!("/v2/persistence/{}/datastores", context.universe);
        let memory = self.path.starts_with("/ums-service/studio/")
            && self.path.split('?').nth(1).is_some_and(|q| {
                q.split('&')
                    .any(|v| v == format!("universeId={}", context.universe))
            });
        if !self.path.starts_with(&prefix)
            && !self.path.starts_with("/v1/persistence/sorted")
            && !memory
        {
            return Err(Error::new("invalid engine data request context"));
        }
        if !matches!(self.method.as_str(), "GET" | "POST" | "DELETE") {
            return Err(Error::new("invalid engine data request method"));
        }
        if !memory && self.method != "GET" && !self.path.ends_with("/multi-get") {
            StudioAccess::fetch(session, context.place, context.universe)?.require_write()?;
        }
        let mut headers = std::collections::BTreeMap::from([
            (
                "User-Agent".into(),
                "RobloxStudio/WinInet RobloxApp/0.744.0.7441077 (GlobalDist; RobloxDirectDownload)"
                    .into(),
            ),
            (
                "Content-Type".into(),
                if memory || self.path.ends_with("/multi-get") {
                    "application/json".into()
                } else {
                    "application/octet-stream".into()
                },
            ),
            ("Roblox-Place-Id".into(), context.place.to_string()),
            ("Roblox-Universe-Id".into(), context.universe.to_string()),
            ("Requester".into(), "Client".into()),
            ("PlayerCount".into(), context.players.to_string()),
            ("rbx-use-universe-rate-limiting".into(), "True".into()),
        ]);
        for (k, v) in &self.headers {
            if !k.to_ascii_lowercase().starts_with("roblox-object-")
                && !matches!(
                    k.to_ascii_lowercase().as_str(),
                    "if-match" | "if-none-match"
                )
            {
                return Err(Error::new("invalid persistence object header"));
            }
            headers.insert(k.clone(), v.clone());
        }
        if !memory && self.method == "POST" && !self.body.is_empty() {
            headers.insert(
                "Content-MD5".into(),
                base64::engine::general_purpose::STANDARD.encode(Md5::digest(self.body.as_bytes())),
            );
        }
        let headers = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>();
        let (status, headers, body) = session.engine_data_request(
            &self.method,
            &format!(
                "https://{}{}",
                if memory {
                    "apis.roblox.com"
                } else {
                    "gamepersistence.roblox.com"
                },
                self.path
            ),
            self.body.as_bytes(),
            &headers,
        )?;
        if session.cookie().appears_in(&body) {
            return Err(Error::new(
                "persistence returned an invalid credential-bearing response",
            ));
        }
        Ok(Response {
            status,
            headers: headers.into_iter().collect(),
            body,
        })
    }
}

#[path = "data_messaging.rs"]
pub mod messaging;
