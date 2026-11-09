//! The REST matchmaker for a Team Create join (docs/TEAMCREATE.md §3).
//!
//! The call order is: `GetCanCollaborate` (with a
//! `robloxctx-authenticated-userid` header), then `POST
//! gamejoin.roblox.com/v1/team-create {gameJoinAttemptId, placeId}`, polled
//! until ready, yielding a join script.
//!
//! **Every call here changes server state** (it can start an RCC server), so
//! callers explicitly request a live join. [`Matchmaker::join`] requires
//! [`LiveJoinConsent`]; the live-session entry point supplies it for that request.

use crate::error::{NetError, Result};
use crate::joinscript::JoinScript;
use rustudio_account::Session;

/// `POST gamejoin.roblox.com/v1/team-create` — the matchmaker.
pub const GAMEJOIN_TEAM_CREATE: &str = "https://gamejoin.roblox.com/v1/team-create";
/// `POST .../team-create-service/v1/GetCanCollaborate`. Needs a
/// `robloxctx-authenticated-userid` header, supplied by the matchmaker.
pub const CAN_COLLABORATE: &str =
    "https://apis.roblox.com/team-create-service/v1/GetCanCollaborate";

/// The matchmaker request body (`TeamCreateRequest`). Only the two required
/// fields; the optional ones (gamerTag, joinOrigin, …) are omitted.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TeamCreateRequest {
    #[serde(rename = "gameJoinAttemptId")]
    pub game_join_attempt_id: String,
    #[serde(rename = "placeId")]
    pub place_id: u64,
}

impl TeamCreateRequest {
    pub fn new(place_id: u64, game_join_attempt_id: impl Into<String>) -> Self {
        TeamCreateRequest {
            game_join_attempt_id: game_join_attempt_id.into(),
            place_id,
        }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("TeamCreateRequest serializes")
    }
}

/// The matchmaker reply (`GameJoinResponse`). `status`: by Studio's game-join
/// convention, 0/1 mean wait-and-poll and 2 means ready (docs/TEAMCREATE.md §3.4,
/// recalled). `join_script` is the inner `JoinInformation` JSON (or fetched
/// from `join_script_url`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GameJoinResponse {
    #[serde(default, rename = "status")]
    pub status: i64,
    #[serde(default, rename = "message")]
    pub message: Option<String>,
    #[serde(default, rename = "jobId")]
    pub job_id: Option<String>,
    #[serde(default, rename = "joinScript", alias = "settings")]
    pub join_script: Option<serde_json::Value>,
    #[serde(default, rename = "joinScriptUrl")]
    pub join_script_url: Option<String>,
    #[serde(default, rename = "queuePosition")]
    pub queue_position: Option<i64>,
}

impl GameJoinResponse {
    pub fn is_ready(&self) -> bool {
        self.status == 2 && (self.join_script.is_some() || self.join_script_url.is_some())
    }
    pub fn is_waiting(&self) -> bool {
        matches!(self.status, 0 | 1)
    }
    /// Parse the inner join script, if present inline.
    pub fn parse_join_script(&self) -> Result<JoinScript> {
        let v = self
            .join_script
            .as_ref()
            .ok_or_else(|| NetError::JoinScript("no inline joinScript".into()))?;
        // The joinScript can be an object or a JSON string holding an object.
        let text = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        JoinScript::parse(&text).map_err(NetError::JoinScript)
    }
}

/// `GetCanCollaborate` request body.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CanCollaborateRequest {
    #[serde(rename = "universeId")]
    pub universe_id: u64,
}

impl CanCollaborateRequest {
    pub fn to_json(universe_id: u64) -> String {
        serde_json::to_string(&CanCollaborateRequest { universe_id })
            .expect("CanCollaborateRequest serializes")
    }
}

/// A capability token proving the caller means to open a live session (which
/// can start an RCC server). Construct it only at a real user action.
pub struct LiveJoinConsent(());
impl LiveJoinConsent {
    /// The caller affirms the user asked to join this live Team Create
    /// session and understands it contacts Roblox and may start a server.
    pub fn user_requested_live_join() -> Self {
        LiveJoinConsent(())
    }
}

/// Polling budget: Studio uses `FIntStudioTeamCreateGamejoinRequestTimeoutMs`
/// = 10000 per attempt (docs/TEAMCREATE.md §3.3).
#[derive(Debug, Clone, Copy)]
pub struct PollBudget {
    pub attempts: u32,
    pub delay_ms: u64,
}
impl Default for PollBudget {
    fn default() -> Self {
        PollBudget {
            // A cold Team Create server (nobody in the place) takes well
            // over 10 s to start; Studio keeps polling, so wait up to ~90 s.
            attempts: 90,
            delay_ms: 1000,
        }
    }
}

pub struct Matchmaker<'a> {
    session: &'a Session,
}

/// `POST gamejoin.roblox.com/v1/team-create-preemptive`: warms the RCC.
pub const GAMEJOIN_TEAM_CREATE_PREEMPTIVE: &str =
    "https://gamejoin.roblox.com/v1/team-create-preemptive";

/// What the matchmaker handed back: the join script plus the raw JSON (a
/// secret: it holds the tickets and tokens; never log or persist it except
/// to a 0600 research file the caller shreds).
pub struct JoinResult {
    pub join: JoinScript,
    pub raw: crate::Redacted,
}

impl<'a> Matchmaker<'a> {
    pub fn new(session: &'a Session) -> Self {
        Matchmaker { session }
    }

    fn post(&self, url: &str, body: &str) -> Result<(u16, String)> {
        let uid = self.session.profile.id.to_string();
        Ok(self.session.post_csrf_with(
            url,
            "application/json",
            body.as_bytes(),
            &[
                ("robloxctx-authenticated-userid", uid.as_str()),
                // gamejoin answers a browser user agent with HTTP 500
                // InternalServerError [verified: live 2026-11-05].
                ("User-Agent", "RobloxStudio/WinInet"),
            ],
        )?)
    }

    /// `GetCanCollaborate` for a universe. Read-only on Roblox's side.
    pub fn can_collaborate(&self, universe_id: u64) -> Result<()> {
        let (status, text) = self.post(CAN_COLLABORATE, &CanCollaborateRequest::to_json(universe_id))?;
        if status != 200 {
            return Err(NetError::Http { what: "GetCanCollaborate".into(), status });
        }
        let v: serde_json::Value = serde_json::from_str(&text)?;
        if v.get("CanCollaborate").and_then(|b| b.as_bool()) == Some(true) {
            Ok(())
        } else {
            Err(NetError::CannotCollaborate(
                v.get("Error").and_then(|e| e.as_str()).unwrap_or("unknown").to_string(),
            ))
        }
    }

    /// The whole Studio open sequence (docs/TEAMCREATE.md §3): GetCanCollaborate,
    /// team-create-preemptive, then `/v1/team-create` polled until ready.
    /// **State-changing** (it can start an RCC): requires [`LiveJoinConsent`].
    pub fn join(
        &self,
        universe_id: u64,
        place_id: u64,
        consent: LiveJoinConsent,
        budget: PollBudget,
    ) -> Result<JoinResult> {
        self.join_with_progress(universe_id, place_id, consent, budget, &mut |_, _| {})
    }

    /// [`Self::join`], reporting each poll: (attempt, Roblox's message).
    pub fn join_with_progress(
        &self,
        universe_id: u64,
        place_id: u64,
        _consent: LiveJoinConsent,
        budget: PollBudget,
        progress: &mut dyn FnMut(u32, &str),
    ) -> Result<JoinResult> {
        self.can_collaborate(universe_id)?;
        let attempt_id = new_attempt_id();
        let body = TeamCreateRequest::new(place_id, attempt_id.clone()).to_json();
        // The pre-warm reply is informational ({status, message}); Studio
        // carries on regardless of it.
        let _ = self.post(GAMEJOIN_TEAM_CREATE_PREEMPTIVE, &body);
        let mut last = String::new();
        for attempt in 0..budget.attempts {
            progress(attempt as u32 + 1, &last);
            let (status, text) = self.post(GAMEJOIN_TEAM_CREATE, &body)?;
            if status != 200 {
                return Err(NetError::Other(format!(
                    "gamejoin /v1/team-create: HTTP {status}: {}",
                    text.chars().take(300).collect::<String>()
                )));
            }
            let resp: GameJoinResponse = serde_json::from_str(&text)?;
            if resp.is_ready() {
                let raw = match resp.join_script.as_ref() {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => String::new(),
                };
                return Ok(JoinResult { join: resp.parse_join_script()?, raw: crate::Redacted::new(raw) });
            }
            if !resp.is_waiting() {
                return Err(NetError::Other(format!(
                    "matchmaker status {}: {}",
                    resp.status,
                    resp.message.unwrap_or_default()
                )));
            }
            last = resp.message.unwrap_or_default();
            std::thread::sleep(std::time::Duration::from_millis(budget.delay_ms));
        }
        if !last.is_empty() {
            eprintln!("rustudio: matchmaker still waiting: {last}");
        }
        Err(NetError::MatchmakerTimeout)
    }
}

/// A fresh GUID-shaped join-attempt id. Not a secret.
fn new_attempt_id() -> String {
    // A v4-looking id from the system clock and address entropy. The server
    // only needs it unique per attempt; it is not a credential.
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let a = (n & 0xffff_ffff) as u32;
    let b = ((n >> 32) & 0xffff) as u16;
    let c = 0x4000u16 | (((n >> 48) & 0x0fff) as u16);
    let d = 0x8000u16 | (((n >> 60) & 0x3fff) as u16);
    let e = n & 0xffff_ffff_ffff;
    format!("{a:08x}-{b:04x}-{c:04x}-{d:04x}-{e:012x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_shape() {
        let r = TeamCreateRequest::new(100000000000004, "ABC-123");
        let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(v["placeId"], 100000000000004u64);
        assert_eq!(v["gameJoinAttemptId"], "ABC-123");
        assert_eq!(v.as_object().unwrap().len(), 2);
    }

    #[test]
    fn can_collaborate_body() {
        let v: serde_json::Value =
            serde_json::from_str(&CanCollaborateRequest::to_json(10000000003)).unwrap();
        assert_eq!(v["universeId"], 10000000003u64);
    }

    #[test]
    fn response_ready_and_waiting() {
        let ready: GameJoinResponse = serde_json::from_str(
            r#"{"status":2,"joinScript":{"MachineAddress":"1.2.3.4","ServerPort":5}}"#,
        )
        .unwrap();
        assert!(ready.is_ready());
        let js = ready.parse_join_script().unwrap();
        assert_eq!(js.machine_address, "1.2.3.4");

        let waiting: GameJoinResponse = serde_json::from_str(r#"{"status":0}"#).unwrap();
        assert!(waiting.is_waiting() && !waiting.is_ready());
    }

    #[test]
    fn attempt_id_is_guid_shaped() {
        let id = new_attempt_id();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[4].len(), 12);
    }
}
