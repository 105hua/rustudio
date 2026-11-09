//! HttpService's real requests. Each request runs on its own thread (ureq);
//! the calling script yields on the task scheduler until the answer is in
//! (services.luau polls `poll` once a frame), as Studio's *Async calls yield.
//! Studio's rules, checked in a Run test (Studio 0.744, an offline Studio):
//! - HttpEnabled off: "HTTP requests are not enabled. Enable via Experience
//!   Settings", with "Go to Experience Settings and turn on Allow HTTP
//!   requests." printed once first (blue).
//! - Roblox's own domains: "HttpService is not allowed to access that Roblox
//!   resource"; not http(s): "URL must be http"; a GET / HEAD with a Body:
//!   "Invalid request options, Body should be empty for 'GET' or 'HEAD'
//!   method"; RequestAsync without Url: "Invalid request options".
//! - Get/PostAsync on a non-2xx answer: "HTTP 404 (Not Found)" over
//!   HTTP/1.1, "HTTP 404" over HTTP/2; transport failures "HttpError:
//!   ConnectFail" / "DnsResolve"; lower-case or unknown methods "HttpError:
//!   Aborted". Restricted headers, header and URL types, Body checks and
//!   the scheme are checked before HttpEnabled. (Measured in the real,
//!   signed-in Studio 0.746 against httpbin.org.)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

pub struct Response {
    pub status: u16,
    pub status_message: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

type Done = HashMap<u64, Result<Response, String>>;

fn done() -> &'static Mutex<Done> {
    static D: OnceLock<Mutex<Done>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Roblox's own web services are off limits to HttpService.
pub fn is_roblox_domain(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let host = host.split(':').next().unwrap_or("").to_ascii_lowercase();
    // (rbxcdn.com is reachable: Studio fetched tr.rbxcdn.com, 404.)
    host == "roblox.com" || host.ends_with(".roblox.com")
}

/// Headers scripts may not set (Studio 0.746: 'Header "Host" is not
/// allowed!'); Cookie, Origin, Authorization… are allowed.
pub fn restricted_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("roblox-")
        || matches!(n.as_str(), "user-agent" | "content-length" | "host" | "connection" | "accept-encoding" | "x-forwarded-for")
}

/// The script's headers plus what Studio adds to every request (seen at
/// httpbin.org/headers): Accept */*, Cache-Control no-cache, Roblox-Id (the
/// place id), a W3C Traceparent and Studio's User-Agent. (Studio also asks
/// for deflate / zstd; here only gzip, which the client decodes.)
pub fn studio_headers(mut h: Vec<(String, String)>, place_id: i64) -> Vec<(String, String)> {
    let has = |h: &[(String, String)], k: &str| h.iter().any(|(n, _)| n.eq_ignore_ascii_case(k));
    if !has(&h, "Accept") {
        h.push(("Accept".into(), "*/*".into()));
    }
    if !has(&h, "Cache-Control") {
        h.push(("Cache-Control".into(), "no-cache".into()));
    }
    h.push(("Roblox-Id".into(), place_id.to_string()));
    let r = |n: usize| -> String {
        let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u64
            ^ NEXT.load(Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        (0..n)
            .map(|i| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                format!("{:x}", (x >> (i % 8)) & 0xf)
            })
            .collect()
    };
    h.push(("Traceparent".into(), format!("00-{}-{}-00", r(32), r(16))));
    h
}

pub fn start(req: Request) -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    std::thread::Builder::new()
        .name("rustudio-http".into())
        .spawn(move || {
            let r = run(req);
            done().lock().unwrap().insert(id, r);
        })
        .ok();
    id
}

pub fn poll(id: u64) -> Option<Result<Response, String>> {
    done().lock().unwrap().remove(&id)
}

fn run(req: Request) -> Result<Response, String> {
    // Studio talks HTTP/2 over TLS where the server offers it, and HTTP/2
    // has no reason phrase (StatusMessage "", "HTTP 404"); plain http is
    // HTTP/1.1 ("Not Found"). This client is HTTP/1.1 only, so https
    // answers are given HTTP/2's empty message.
    let req_https = req.url.starts_with("https://");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent("RobloxStudio/WinInet RobloxApp/0.746.0.7461269 (GlobalDist; RobloxDirectDownload)")
        .build()
        .into();
    let mut b = ureq::http::Request::builder().method(req.method.as_str()).uri(&req.url);
    for (k, v) in &req.headers {
        b = b.header(k.as_str(), v.as_str());
    }
    let resp = match req.body {
        Some(body) => {
            let r = b.body(body).map_err(|e| e.to_string())?;
            agent.run(r)
        }
        None => {
            let r = b.body(()).map_err(|e| e.to_string())?;
            agent.run(r)
        }
    };
    let https = req_https;
    let mut resp = resp.map_err(describe)?;
    let status = resp.status();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();
    let body = resp
        .body_mut()
        .with_config()
        .limit(1 << 30)
        .read_to_vec()
        .map_err(describe)?;
    Ok(Response {
        status: status.as_u16(),
        status_message: if https { String::new() } else { status.canonical_reason().unwrap_or("").to_string() },
        headers,
        body,
    })
}

/// Studio's wording for transport failures.
fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::HostNotFound => "HttpError: DnsResolve".into(),
        ureq::Error::Timeout(_) => "HttpError: Timedout".into(),
        ureq::Error::ConnectionFailed => "HttpError: ConnectFail".into(),
        ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => "HttpError: ConnectFail".into(),
        // getaddrinfo failures come back as plain io errors.
        ureq::Error::Io(e) if e.to_string().contains("lookup address") || e.to_string().contains("Name or service not known") => {
            "HttpError: DnsResolve".into()
        }
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "HttpError: SslConnectFail".into(),
        ureq::Error::BadUri(_) => "HttpError: InvalidUrl".into(),
        other => format!("HttpError: Unknown ({other})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roblox_domains() {
        assert!(is_roblox_domain("https://users.roblox.com/v1/users/1"));
        assert!(is_roblox_domain("https://www.ROBLOX.com"));
        assert!(!is_roblox_domain("https://tr.rbxcdn.com/"));
        assert!(restricted_header("roblox-place-id") && restricted_header("Host") && !restricted_header("Cookie"));
        assert!(!is_roblox_domain("https://roproxy.com/x"));
        assert!(!is_roblox_domain("https://example.com/roblox.com"));
    }
    #[test]
    fn studio_adds_its_headers() {
        let h = studio_headers(vec![("Accept".into(), "text/html".into())], 7);
        let get = |k: &str| h.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("Accept").as_deref(), Some("text/html"));
        assert_eq!(get("Roblox-Id").as_deref(), Some("7"));
        assert_eq!(get("Cache-Control").as_deref(), Some("no-cache"));
        let tp = get("Traceparent").unwrap();
        assert_eq!(tp.len(), 55);
        assert!(tp.starts_with("00-") && tp.ends_with("-00"));
    }
}
