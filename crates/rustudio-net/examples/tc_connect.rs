//! Research aid: join a Team Create place with rustudio's own client (the
//! matchmaker, then QUIC through the RUPP relay) and print the *structure* of
//! what the server sends: stream headers, message ids and lengths. No secret
//! is printed. Sends nothing above QUIC unless asked.
//!
//!   cargo run --release --offline -p rustudio-net --example tc_connect -- UNIVERSE PLACE [SECONDS]
use rustudio_net::matchmaker::{LiveJoinConsent, Matchmaker, PollBudget};
use rustudio_net::transport::{connect_quic, QuicTarget};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let universe: u64 = a[0].parse().unwrap();
    let place: u64 = a[1].parse().unwrap();
    let secs: u64 = a.get(2).map(|s| s.parse().unwrap()).unwrap_or(8);
    let (cookie, _) = rustudio_account::session::load_cookie(false).unwrap().unwrap();
    let s = rustudio_account::Session::validate(cookie).unwrap();
    let r = Matchmaker::new(&s)
        .join(universe, place, LiveJoinConsent::user_requested_live_join(), PollBudget::default())
        .unwrap_or_else(|e| panic!("join: {e}"));
    let t = QuicTarget::from_join(&r.join).unwrap();
    println!("relay {} rcc {}:{}", t.relay, t.rcc.0, t.rcc.1);
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async move {
        let (_ep, conn, sock) = match connect_quic(&t).await {
            Ok(x) => x,
            Err(e) => {
                println!("connect failed: {e}");
                return;
            }
        };
        println!("QUIC up: alpn ok, rtt {:?}", conn.rtt());
        let c2 = conn.clone();
        tokio::spawn(async move {
            while let Ok(mut rs) = c2.accept_uni().await {
                tokio::spawn(async move {
                    let id = rs.id();
                    let mut hdr = [0u8; 7];
                    if rs.read_exact(&mut hdr).await.is_err() {
                        println!("srv stream {id}: closed before header");
                        return;
                    }
                    println!("srv stream {id}: hdr {}", hex(&hdr));
                    loop {
                        let mut l = [0u8; 4];
                        if rs.read_exact(&mut l).await.is_err() {
                            println!("srv stream {id}: end");
                            return;
                        }
                        let n = (u32::from_be_bytes(l) & 0x7fff_ffff) as usize;
                        let mut m = vec![0u8; n];
                        if rs.read_exact(&mut m).await.is_err() {
                            return;
                        }
                        println!("  srv {id}: msg len {n} head {}", hex(&m[..m.len().min(6)]));
                    }
                });
            }
        });
        let c3 = conn.clone();
        tokio::spawn(async move {
            while let Ok(d) = c3.read_datagram().await {
                println!("  srv datagram len {} head {}", d.len(), hex(&d[..d.len().min(6)]));
            }
        });
        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
        println!("close reason: {:?}", conn.close_reason());
        println!("rupp stats in/out/token-switches {:?}", *sock.stats.lock().unwrap());
        conn.close(0u32.into(), b"bye");
    });
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
