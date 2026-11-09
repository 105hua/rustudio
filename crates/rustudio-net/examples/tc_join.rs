//! Research aid: run the Team Create matchmaker for one place with rustudio's
//! keyring session and describe the join script's *shape* (keys, value types,
//! lengths). No secret value is printed.
//!
//!   cargo run --release --offline -p rustudio-net --example tc_join -- UNIVERSE PLACE [OUT.json]
//!
//! With OUT.json the raw join script is written there with mode 0600 (it holds
//! the tickets and tokens: keep it under ~/.cache/rustudio-dev/tc/keys and shred it).
use rustudio_net::matchmaker::{LiveJoinConsent, Matchmaker, PollBudget};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let universe: u64 = a[0].parse().unwrap();
    let place: u64 = a[1].parse().unwrap();
    let (cookie, _) = rustudio_account::session::load_cookie(false)
        .expect("keyring")
        .expect("signed in");
    let s = rustudio_account::Session::validate(cookie).expect("session");
    println!("signed in as {}", s.profile.name);
    let mm = Matchmaker::new(&s);
    let r = match mm.join(universe, place, LiveJoinConsent::user_requested_live_join(), PollBudget::default()) {
        Ok(r) => r,
        Err(e) => {
            println!("join failed: {e}");
            std::process::exit(1)
        }
    };
    let v: serde_json::Value = serde_json::from_str(r.raw.expose()).unwrap();
    describe(&v, 0);
    if let Some(out) = a.get(2) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(out)
            .unwrap();
        f.write_all(r.raw.expose().as_bytes()).unwrap();
        println!("wrote the join script (0600)");
    }
}

const PUBLIC: &[&str] = &[
    "MachineAddress", "ServerPort", "NetStackPort", "UdmuxEndpoints", "ServerConnections", "Address", "Port",
    "DirectServerReturn", "PlaceId", "UniverseId", "DataCenterId", "UserName", "RccVersion", "IsUnknownOrUnder13",
    "UserId", "GameId", "SessionId",
];

fn describe(v: &serde_json::Value, depth: usize) {
    let pad = "  ".repeat(depth);
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                match x {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        println!("{pad}{k}:");
                        describe(x, depth + 1);
                    }
                    serde_json::Value::String(s) if !PUBLIC.contains(&k.as_str()) => {
                        let b64 = b64len(s);
                        println!("{pad}{k}: string len {}{}", s.len(), b64.map(|n| format!(" (base64 -> {n} bytes)")).unwrap_or_default());
                    }
                    other => println!("{pad}{k}: {other}"),
                }
            }
        }
        serde_json::Value::Array(xs) => {
            for x in xs {
                describe(x, depth + 1);
                println!("{pad}-");
            }
        }
        other => println!("{pad}{other}"),
    }
}

fn b64len(s: &str) -> Option<usize> {
    let t = s.trim_end_matches('=');
    if t.is_empty() || !t.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/') {
        return None;
    }
    Some(t.len() * 3 / 4)
}
