//! Read-only snapshot of a live Team Create place: join, wait for the initial
//! DataModel, write it to an .rbxl, disconnect. Sends no edits.
//!
//!   cargo run --release -p rustudio-net --example tc_snapshot -- UNIVERSE PLACE OUT.rbxl
use rustudio_net::{live::{LiveEvent, LiveSession}, model::LiveModel};
use std::time::{Duration, Instant};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (u, p, out) = (a[0].parse().unwrap(), a[1].parse().unwrap(), &a[2]);
    let session = LiveSession::connect(u, p).expect("connect");
    let mut model = LiveModel::new();
    let until = Instant::now() + Duration::from_secs(180);
    loop {
        if Instant::now() > until { panic!("initial model timed out"); }
        match session.events.recv_timeout(Duration::from_secs(1)) {
            Ok(LiveEvent::Batch(ops)) => for op in ops { model.apply(op) },
            Ok(LiveEvent::Ready) => break,
            Ok(LiveEvent::Error(e) | LiveEvent::Disconnected(e)) => panic!("{e}"),
            Ok(LiveEvent::Notice(m)) => eprintln!("notice: {m}"),
            _ => {}
        }
    }
    // drain briefly for late asset/shared-string resolutions
    let settle = Instant::now() + Duration::from_secs(5);
    while Instant::now() < settle {
        if let Ok(LiveEvent::Batch(ops)) = session.events.recv_timeout(Duration::from_millis(200)) {
            for op in ops { model.apply(op) }
        }
    }
    drop(session);
    let dom = model.dom();
    let root = dom.root();
    let kids: Vec<_> = root.children().to_vec();
    let f = std::io::BufWriter::new(std::fs::File::create(out).expect("create"));
    rbx_binary::to_writer(f, dom, &kids).expect("write rbxl");
    eprintln!("wrote {out}: {} instances", model.instance_count());
}
