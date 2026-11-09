//! Research aid: join live, open the lanes the way Studio does and send the
//! opening messages built from the join script, using a decrypted Studio
//! capture (`…/c2s/stream_6.bin`) only as a template for the per-build parts.
//! Prints the structure of the server's answer. No secret is printed.
//!
//!   tc_probe UNIVERSE PLACE TEMPLATE_STREAM.bin SECONDS [a8|a8-cached|a8-sync|replay-crypto]
//! `a8` builds a fresh 0xa8 and sends an empty shared-string cache (0xa7);
//! `a8-cached` uses the template's hash list. `a8-sync` stops after 0x90.
//! `replay-crypto` replays both template messages verbatim.
use rustudio_net::handshake::*;
use rustudio_net::matchmaker::{LiveJoinConsent, Matchmaker, PollBudget};
use rustudio_net::rtcio::{self, *};
use rustudio_net::sessioncrypto::Hello;
use rustudio_net::transport::{QuicTarget, connect_quic};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let universe: u64 = a[0].parse().unwrap();
    let place: u64 = a[1].parse().unwrap();
    let tpl = std::fs::read(&a[2]).unwrap();
    let secs: u64 = a[3].parse().unwrap();
    let crypto_mode = a.get(4).map(String::as_str).unwrap_or("a8");
    let (_, _, msgs) = rustudio_net::replicator::split_stream(&tpl).unwrap();
    let find = |id: u8| {
        msgs.iter()
            .find(|m| m.first() == Some(&id))
            .map(|m| m.to_vec())
            .unwrap()
    };
    let captured90 = find(0x90);
    let ps = ProtocolSyncTemplate::from_capture(&captured90).expect("0x90 template");
    let captured8a = find(0x8a);
    let tk = TicketTemplate::from_capture(&captured8a).expect("0x8a template");
    let captured92 = find(0x92);
    let (ma8, ma7) = (find(0xa8), find(0xa7));
    if crypto_mode == "check" {
        verify_captured_hello(&ma8, &captured8a);
        return;
    }

    let (cookie, _) = rustudio_account::session::load_cookie(false)
        .unwrap()
        .unwrap();
    let s = rustudio_account::Session::validate(cookie).unwrap();
    let r = Matchmaker::new(&s)
        .join(
            universe,
            place,
            LiveJoinConsent::user_requested_live_join(),
            PollBudget::default(),
        )
        .unwrap_or_else(|e| panic!("join: {e}"));
    let raw: serde_json::Value = serde_json::from_str(r.raw.expose()).unwrap();
    let t = QuicTarget::from_join(&r.join).unwrap();
    let m90 = protocol_sync(&ps, &raw);
    let m8a = submit_ticket(&tk, &r.join, &raw);
    compare_player_json_shape(&captured90, &raw);
    compare_session_id_shape(&captured8a, &raw);
    let fresh_a8 = matches!(crypto_mode, "a8" | "a8-cached" | "a8-sync" | "a8-no92" | "a8-staged" | "a8-staged-noa7" | "a8-step" | "a8-isolate").then(|| {
        Hello::from_client_ticket(r.join.client_ticket.expose())
            .and_then(|h| h.encode())
            .unwrap_or_else(|e| panic!("build fresh 0xa8: {e}"))
    });
    let m92 = if fresh_a8.is_some() {
        fresh_place_verification().unwrap()
    } else {
        captured92
    };
    let empty_a7 = shared_string_cache_message(&[]).unwrap();
    println!("built 0x90 {} B, 0x8a {} B", m90.len(), m8a.len());
    if let Some(m) = &fresh_a8 {
        println!("built fresh 0xa8 {} B from this join's bundle", m.len());
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async move {
        let (_ep, conn, _sock) = connect_quic(&t).await.unwrap_or_else(|e| panic!("{e}"));
        println!("QUIC up");
        let c2 = conn.clone();
        let seen93 = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen93_rx = seen93.clone();
        let summary = std::sync::Arc::new(std::sync::Mutex::new((0usize, 0usize, 0usize, 0usize, Vec::<String>::new())));
        let summary_rx = summary.clone();
        tokio::spawn(async move {
            while let Ok(mut rs) = c2.accept_uni().await {
                let seen93_stream = seen93_rx.clone();
                let summary_stream = summary_rx.clone();
                tokio::spawn(async move {
                    let id = rs.id();
                    let Some((ch, tag)) = rtcio::read_header(&mut rs).await else {
                        return;
                    };
                    println!("srv {id}: ch {ch} tag {}", hex(&tag));
                    let mut n = 0;
                    let mut decoder = rustudio_net::replicator::Decoder::new();
                    let mut model = rustudio_net::model::LiveModel::new();
                    let mut capture = if ch == CH_REPLICATION && tag == lane_tag(1) {
                        std::env::var("RUSTUDIO_PROBE_STREAM_OUT").ok().map(|path| {
                            use std::io::Write;
                            let mut options = std::fs::OpenOptions::new();
                            options.write(true).create(true).truncate(true);
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::OpenOptionsExt;
                                options.mode(0o600);
                            }
                            let mut f = options.open(path).unwrap();
                            f.write_all(&[6, 1, ch, tag[0], tag[1], tag[2], tag[3]]).unwrap();
                            f
                        })
                    } else { None };
                    loop {
                        match rtcio::read_msg_result(&mut rs).await {
                            Ok(m) => {
                                n += 1;
                                if let Some(f) = capture.as_mut() {
                                    use std::io::Write;
                                    f.write_all(&rtcio::frame(&m)).unwrap();
                                }
                                if ch == CH_REPLICATION && tag == lane_tag(1) && !m.is_empty() {
                                    for op in decoder.decode_frame(m[0], &m[1..]) { model.apply(op); }
                                    *summary_stream.lock().unwrap() = (n, model.instance_count(), decoder.shared.len(), decoder.errors.len(), decoder.errors.clone());
                                }
                                if m.first() == Some(&0x93) {
                                    seen93_stream.store(true, std::sync::atomic::Ordering::Release);
                                }
                                if n <= 12 || n % 1000 == 0 {
                                    let printable = m.len() < 400
                                        && m.iter()
                                            .skip(1)
                                            .filter(|c| c.is_ascii_graphic() || **c == b' ')
                                            .count()
                                            * 10
                                            > m.len() * 8;
                                    println!(
                                        "  srv {id}#{n}: len {} head {}{}",
                                        m.len(),
                                        hex(&m[..m.len().min(12)]),
                                        if printable {
                                            format!(" text {:?}", String::from_utf8_lossy(&m))
                                        } else {
                                            String::new()
                                        }
                                    );
                                }
                            }
                            Err(e) => {
                                println!("srv {id}: {e} after {n} msgs");
                                break;
                            }
                        }
                    }
                });
            }
        });
        let c3 = conn.clone();
        tokio::spawn(async move {
            let mut n = 0;
            while let Ok(d) = c3.read_datagram().await {
                n += 1;
                if n <= 5 {
                    println!(
                        "  srv dgram len {} head {}",
                        d.len(),
                        hex(&d[..d.len().min(10)])
                    );
                }
            }
        });
        // lanes in Studio's order
        let _l0 = open_lane(&conn, CH_REPLICATION, lane_tag(0)).await.unwrap();
        let mut rep = open_lane(&conn, CH_REPLICATION, lane_tag(1)).await.unwrap();
        let mut lanes = vec![];
        for seq in [2u32, 4, 7, 8, 9, 10, 12, 13] {
            lanes.push(
                open_lane(&conn, CH_REPLICATION, lane_tag(seq))
                    .await
                    .unwrap(),
            );
        }
        let mut c4 = open_lane(&conn, CH_REPLICATION, TAG_CTRL).await.unwrap();
        rtcio::send(&mut c4, &[0, 0]).await.unwrap();
        for m in [
            [0x02, 0x04, 0, 0, 0, 0x03, 0, 0, 0, 0x02],
            [0x02, 0x04, 0, 0, 0, 0x05, 0, 0, 0, 0x04],
            [0x02, 0x04, 0, 0, 0, 0x06, 0, 0, 0, 0x06],
            [0x02, 0x04, 0, 0, 0, 0x0b, 0, 0, 0, 0x08],
        ] {
            rtcio::send(&mut c4, &m).await.unwrap();
        }
        let mut c6 = open_lane(&conn, CH_PING, TAG_CTRL).await.unwrap();
        rtcio::send(&mut c6, &[0, 0]).await.unwrap();
        for m in [
            [0x02, 0x06, 0, 0, 0, 0x02, 0, 0, 0, 0x0a],
            [0x02, 0x06, 0, 0, 0, 0x03, 0, 0, 0, 0x0c],
        ] {
            rtcio::send(&mut c6, &m).await.unwrap();
        }
        let mut c0 = open_lane(&conn, CH_CONTROL0, TAG_CTRL).await.unwrap();
        rtcio::send(&mut c0, &[0, 0]).await.unwrap();
        let mut ping = open_lane(&conn, CH_PING, lane_tag(1)).await.unwrap();
        if let Some(m) = &fresh_a8 {
            rtcio::send(&mut rep, m).await.unwrap();
            let cache = if crypto_mode == "a8-cached" {
                &ma7
            } else {
                &empty_a7
            };
            if !matches!(crypto_mode, "a8-staged-noa7" | "a8-isolate") {
                rtcio::send(&mut rep, cache).await.unwrap();
            }
        } else if crypto_mode == "replay-crypto" {
            rtcio::send(&mut rep, &ma8).await.unwrap();
            rtcio::send(&mut rep, &ma7).await.unwrap();
        }
        for m in [&m90, &m92, &m8a, &vec![0x8f, 0]] {
            if (matches!(crypto_mode, "a8-sync" | "a8-staged" | "a8-staged-noa7" | "a8-step" | "sync-only") && m.first() != Some(&0x90))
                || (crypto_mode == "a8-no92" && m.first() == Some(&0x92))
            {
                continue;
            }
            if crypto_mode != "a8-isolate" { rtcio::send(&mut rep, m).await.unwrap(); }
        }
        let rep_control = [0x83, 0x23, 0, 0, 0, 0, 0];
        println!("sent opening");
        let t0 = std::time::Instant::now();
        let mut last_tick = t0;
        let mut control_started = false;
        let mut step = 0;
        while t0.elapsed().as_secs() < secs {
            if crypto_mode == "a8-isolate" && step < 4 && t0.elapsed().as_secs() >= step + 1 {
                let m = [&m90, &m92, &m8a, &vec![0x8f, 0]][step as usize].clone();
                println!("isolate {} at {:?}: send {:02x}: {:?}", step, t0.elapsed(), m[0], rtcio::send(&mut rep, &m).await);
                step += 1;
            }
            if crypto_mode == "a8-step" && step < 3 && t0.elapsed().as_secs() >= step + 1 {
                let m = [&m92, &m8a, &vec![0x8f, 0]][step as usize].clone();
                println!("step {} at {:?}: send {:02x}: {:?}", step, t0.elapsed(), m[0], rtcio::send(&mut rep, &m).await);
                step += 1;
            }
            if seen93.swap(false, std::sync::atomic::Ordering::AcqRel) {
                if matches!(crypto_mode, "a8-staged" | "a8-staged-noa7") {
                    for m in [&m92, &m8a, &vec![0x8f, 0]] {
                        let _ = rtcio::send(&mut rep, m).await;
                    }
                    println!("sent 0x92/0x8a/0x8f after 0x93");
                }
                if !matches!(crypto_mode, "a8-step" | "a8-isolate") { let _ = rtcio::send(&mut rep, &rep_control).await; }
                control_started = !matches!(crypto_mode, "a8-step" | "a8-isolate");
                println!("sent replication control after 0x93");
            }
            if last_tick.elapsed() >= std::time::Duration::from_secs(1) {
                let us = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_micros() as u64;
                let _ = rtcio::send(&mut ping, &ping_message(us)).await;
                if control_started {
                    let _ = rtcio::send(&mut rep, &rep_control).await;
                }
                last_tick = std::time::Instant::now();
            }
            if let Some(r) = conn.close_reason() {
                println!("closed: {r}");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        println!("live model summary (messages, instances, shared strings, decode errors): {:?}", *summary.lock().unwrap());
        println!("close reason: {:?}", conn.close_reason());
        conn.close(0u32.into(), b"bye");
    });
}

fn verify_captured_hello(captured_a8: &[u8], submit: &[u8]) {
    let mut o = 1;
    get_varint(submit, &mut o).expect("captured user id");
    let n = get_varint(submit, &mut o).expect("captured ticket length") as usize;
    let ticket = std::str::from_utf8(&submit[o..o + n]).expect("captured ticket UTF-8");
    let produced = Hello::from_client_ticket(ticket)
        .and_then(|h| h.encode())
        .expect("decode captured ticket");
    if produced == captured_a8 {
        println!(
            "captured 0xa8 reproduction: exact ({} bytes)",
            produced.len()
        );
    } else {
        let first = produced.iter().zip(captured_a8).position(|(a, b)| a != b);
        println!(
            "captured 0xa8 reproduction: MISMATCH produced={}B captured={}B first_diff={first:?}",
            produced.len(),
            captured_a8.len()
        );
    }
}

fn compare_session_id_shape(captured: &[u8], raw: &serde_json::Value) {
    let Some(i) = captured.windows(11).position(|w| w == b"{\"SessionId") else {
        return;
    };
    let reference = serde_json::Deserializer::from_slice(&captured[i..])
        .into_iter::<serde_json::Value>()
        .next()
        .and_then(|v| v.ok());
    let current = raw
        .get("SessionId")
        .and_then(|v| v.as_str())
        .map(|s| serde_json::from_str::<serde_json::Value>(s));
    match (reference, current) {
        (Some(reference), Some(Ok(current))) => {
            let old = reference
                .as_object()
                .map(|m| m.keys().collect::<std::collections::BTreeSet<_>>());
            let new = current
                .as_object()
                .map(|m| m.keys().collect::<std::collections::BTreeSet<_>>());
            match (old, new) {
                (Some(old), Some(new)) => {
                    let missing: Vec<_> = old.difference(&new).map(|k| k.as_str()).collect();
                    let extra: Vec<_> = new.difference(&old).map(|k| k.as_str()).collect();
                    println!("session JSON keys: missing={missing:?} extra={extra:?}");
                }
                _ => println!("session JSON is not an object"),
            }
        }
        (_, Some(Err(_))) => println!("session JSON: live SessionId is not valid JSON"),
        _ => println!("session JSON: unavailable for comparison"),
    }
}

fn compare_player_json_shape(captured: &[u8], raw: &serde_json::Value) {
    let Some(i) = captured.windows(12).position(|w| w == b"{\"AccountAge") else {
        return;
    };
    let mut parsed = None;
    for k in 1..=3 {
        let mut o = i - k;
        if let Some(n) = get_varint(captured, &mut o) {
            if o == i && i + n as usize <= captured.len() {
                if let Ok(v) =
                    serde_json::from_slice::<serde_json::Value>(&captured[i..i + n as usize])
                {
                    parsed = Some(v);
                    break;
                }
            }
        }
    }
    let (Some(reference), Ok(current)) = (
        parsed,
        serde_json::from_str::<serde_json::Value>(&player_json(raw)),
    ) else {
        return;
    };
    let kind = |v: &serde_json::Value| match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    };
    let mut issues = vec![];
    let mut changed = vec![];
    if let (Some(reference), Some(current)) = (reference.as_object(), current.as_object()) {
        for (key, value) in current {
            if let Some(old) = reference.get(key) {
                let old_kind = kind(old);
                let new_kind = kind(value);
                if old_kind != new_kind {
                    issues.push(format!("{key}:{old_kind}->{new_kind}"));
                }
                if old != value {
                    changed.push(key.as_str());
                }
            } else {
                issues.push(format!("{key}:missing-in-capture"));
            }
        }
    }
    println!(
        "player JSON null/type differences: {}",
        if issues.is_empty() {
            "none".into()
        } else {
            issues.join(",")
        }
    );
    println!(
        "player JSON values differing from capture (keys only): {}",
        changed.join(",")
    );
    if let Some(token) = raw.get("APIsecurityToken").and_then(|v| v.as_str()) {
        println!(
            "API token metadata: chars={} percent-signs={}",
            token.len(),
            token.bytes().filter(|b| *b == b'%').count()
        );
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
