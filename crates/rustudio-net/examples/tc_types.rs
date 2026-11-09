//! Lists schema properties and event args of given wire value types in a
//! recorded Team Create stream (RUSTUDIO_TC_STREAM_OUT).
//!   cargo run -p rustudio-net --example tc_types -- STREAM.bin 0x28 [0x27 ...]
use rustudio_net::replicator::{split_stream, Decoder};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: tc_types STREAM.bin TYPE...");
    let want: Vec<u8> = args.map(|a| u8::from_str_radix(a.trim_start_matches("0x"), 16).unwrap()).collect();
    let bytes = std::fs::read(&path).expect("read stream");
    let (_, _, msgs) = split_stream(&bytes).expect("not an rbx-rtcio stream");
    let mut dec = Decoder::new();
    for m in msgs.iter().filter(|m| !m.is_empty()) {
        let _ = dec.decode_frame(m[0], &m[1..]);
        if let Some(s) = &dec.schema {
            for c in &s.classes {
                for p in &c.props {
                    if want.contains(&p.ty) {
                        println!("prop {}.{} = {:#x}", c.name, p.name, p.ty);
                    }
                }
                for e in &c.events {
                    if e.arg_types.iter().any(|t| want.contains(t)) {
                        println!("event {}.{} {:x?}", c.name, e.name, e.arg_types);
                    }
                }
            }
            break;
        }
    }
    for e in dec.errors.iter().take(3) {
        println!("error: {e:?}");
    }
}
