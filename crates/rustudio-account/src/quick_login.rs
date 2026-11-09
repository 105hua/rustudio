//! Sign in through the default browser with Roblox's Quick Sign-in (the
//! "cross-device login" the Roblox app uses): rustudio asks for a short code,
//! the person enters it at [`CONFIRM_URL`] in any browser signed in to
//! Roblox, and once they confirm, the code and its private key are traded for
//! a `.ROBLOSECURITY` session of rustudio's own. The browser's own cookie is
//! never read.

use std::time::Duration;

use crate::{session::session_agent, Result};

/// Where the person types the code (roblox.com asks them to log in first if
/// that browser isn't signed in).
pub const CONFIRM_URL: &str = "https://www.roblox.com/crossdevicelogin/ConfirmCode";

const CREATE: &str = "https://apis.roblox.com/auth-token-service/v1/login/create";
const STATUS: &str = "https://apis.roblox.com/auth-token-service/v1/login/status";
const LOGIN: &str = "https://auth.roblox.com/v2/login";
const COOKIE: &str = ".ROBLOSECURITY";

/// A pending Quick Sign-in. `private_key` proves this client asked for the
/// code; it never leaves this process except to Roblox.
pub struct QuickLogin {
    pub code: String,
    private_key: String,
}

impl std::fmt::Debug for QuickLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuickLogin").field("code", &self.code).finish_non_exhaustive()
    }
}

/// Where the code is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickStatus {
    /// Waiting for someone to enter it.
    Created,
    /// Entered; the browser is asking that account to confirm.
    UserLinked(Option<String>),
    /// Confirmed: [`QuickLogin::finish`] gets the session.
    Validated,
    /// Turned down in the browser.
    Cancelled,
    /// Ran out of time (codes last a few minutes).
    Expired,
}

fn err(msg: impl Into<String>) -> crate::Error {
    crate::Error(msg.into())
}

fn post(agent: &ureq::Agent, url: &str, body: &serde_json::Value, csrf: Option<&str>) -> Result<ureq::http::Response<ureq::Body>> {
    let mut req = agent.post(url).header("Content-Type", "application/json").header("Accept", "application/json");
    if let Some(t) = csrf {
        req = req.header("x-csrf-token", t);
    }
    Ok(req.send(body.to_string().as_bytes())?)
}

/// POSTs `body`, retrying once with the `x-csrf-token` a 403 hands back
/// (every one of these endpoints wants one, signed in or not).
fn call(agent: &ureq::Agent, url: &str, body: &serde_json::Value) -> Result<(u16, String)> {
    let r = post(agent, url, body, None)?;
    let token = r.headers().get("x-csrf-token").and_then(|v| v.to_str().ok()).map(str::to_owned);
    match token {
        Some(t) if r.status().as_u16() == 403 => Ok(read(post(agent, url, body, Some(&t))?)),
        _ => Ok(read(r)),
    }
}

fn read(mut r: ureq::http::Response<ureq::Body>) -> (u16, String) {
    let status = r.status().as_u16();
    let text = r.body_mut().with_config().limit(1 << 20).read_to_string().unwrap_or_default();
    (status, text)
}

/// Roblox's first error message in a reply, if any.
fn roblox_error(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.get("errors")?.get(0)?.get("message")?.as_str().map(str::to_owned)
}

impl QuickLogin {
    /// Asks Roblox for a new code.
    pub fn start() -> Result<QuickLogin> {
        let (status, text) = call(&session_agent(), CREATE, &serde_json::json!({}))?;
        if status != 200 {
            return Err(err(format!(
                "Roblox didn't give a sign-in code (HTTP {status}{})",
                roblox_error(&text).map(|m| format!(": {m}")).unwrap_or_default()
            )));
        }
        let v: serde_json::Value = serde_json::from_str(&text)?;
        let field = |k: &str| v.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
        match (field("code"), field("privateKey")) {
            (Some(code), Some(private_key)) => Ok(QuickLogin { code, private_key }),
            _ => Err(err("Roblox's sign-in code reply had no code")),
        }
    }

    fn body(&self) -> serde_json::Value {
        serde_json::json!({ "code": self.code, "privateKey": self.private_key })
    }

    /// Where the code is now.
    pub fn status(&self) -> Result<QuickStatus> {
        let (status, text) = call(&session_agent(), STATUS, &self.body())?;
        if status == 400 || status == 404 {
            // An unknown code is an expired one.
            return Ok(QuickStatus::Expired);
        }
        if status != 200 {
            return Err(err(format!("Roblox's sign-in check failed (HTTP {status})")));
        }
        let v: serde_json::Value = serde_json::from_str(&text)?;
        Ok(match v.get("status").and_then(|s| s.as_str()).unwrap_or("") {
            "UserLinked" => QuickStatus::UserLinked(
                v.get("accountName").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).map(str::to_owned),
            ),
            "Validated" => QuickStatus::Validated,
            "Cancelled" => QuickStatus::Cancelled,
            "Created" => QuickStatus::Created,
            _ => QuickStatus::Expired,
        })
    }

    /// Trades a confirmed code for a session: the `.ROBLOSECURITY` value.
    pub fn finish(&self) -> Result<String> {
        let agent = session_agent();
        let body = serde_json::json!({ "ctype": "AuthToken", "cvalue": self.code, "password": self.private_key });
        let mut csrf: Option<String> = None;
        for _ in 0..2 {
            let r = post(&agent, LOGIN, &body, csrf.as_deref())?;
            let fresh = r.headers().get("x-csrf-token").and_then(|v| v.to_str().ok()).map(str::to_owned);
            let cookie = r
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .find_map(|c| {
                    let (name, rest) = c.split_once('=')?;
                    let value = rest.split(';').next()?.trim();
                    (name.trim() == COOKIE && !value.is_empty()).then(|| value.to_owned())
                });
            let challenged = r.headers().contains_key("rblx-challenge-id");
            let (status, text) = read(r);
            if let Some(c) = cookie {
                return Ok(c);
            }
            if status == 403 && fresh.is_some() && fresh != csrf && !challenged {
                csrf = fresh;
                continue;
            }
            if challenged {
                return Err(err("Roblox wants an extra verification step for this sign-in; paste the cookie instead"));
            }
            return Err(err(format!(
                "Roblox didn't sign in (HTTP {status}{})",
                roblox_error(&text).map(|m| format!(": {m}")).unwrap_or_default()
            )));
        }
        Err(err("Roblox kept refusing the sign-in token"))
    }

    /// Checks every couple of seconds until the code is confirmed, turned
    /// down, expired, or `stop` says to give up; `on_status` hears each
    /// change. The session value on success, None if it ended without one
    /// (the reason went to `on_status`).
    pub fn wait(&self, stop: &dyn Fn() -> bool, on_status: &mut dyn FnMut(&QuickStatus)) -> Result<Option<String>> {
        let mut last = QuickStatus::Created;
        loop {
            if stop() {
                return Ok(None);
            }
            let now = self.status()?;
            if now != last {
                on_status(&now);
                last = now.clone();
            }
            match now {
                QuickStatus::Validated => return self.finish().map(Some),
                QuickStatus::Cancelled | QuickStatus::Expired => return Ok(None),
                _ => {}
            }
            // Short sleeps so Cancel takes effect quickly.
            for _ in 0..8 {
                if stop() {
                    return Ok(None);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}
