//! Read-only DataModel view of a recorded Team Create session.
//!
//! Feeds a decrypted, reassembled server->client `rbx-rtcio` stream (from
//! `tools/tc/rig/quic_decode.py ... OUTDIR`, e.g. `OUTDIR/stream_23.bin`)
//! through `replicator::Decoder` into `model::LiveModel`, then prints the
//! instance count, decode errors and the first levels of the tree. No network.
//!
//!   cargo run --release --offline -p rustudio-net --example tc_view -- STREAM.bin [DEPTH]
//! Set RUSTUDIO_SNAPSHOT_OUT to export the replayed model without a live join.
use rustudio_net::model::LiveModel;
use rustudio_net::replicator::{split_stream, Decoder};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: tc_view STREAM.bin [DEPTH]");
    let depth: usize = args.next().map(|d| d.parse().unwrap()).unwrap_or(1);
    let bytes = std::fs::read(&path).expect("read stream");
    let (channel, _, msgs) = split_stream(&bytes).expect("not an rbx-rtcio stream");
    let mut dec = Decoder::new();
    let mut model = LiveModel::new();
    let mut ops = 0usize;
    for (index, m) in msgs.iter().enumerate() {
        if m.is_empty() {
            continue;
        }
        let before = dec.errors.len();
        for op in dec.decode_frame(m[0], &m[1..]) {
            ops += 1;
            model.apply(op);
        }
        if dec.errors.len() > before {
            println!(
                "decode failure at frame {index}, length {}, packet {:02x}",
                m.len(),
                m[0]
            );
        }
    }
    println!(
        "channel {channel}: {} messages, {ops} ops, {} instances in the model, {} shared strings, {} decode errors",
        msgs.len(),
        model.instance_count(),
        dec.shared.len(),
        dec.errors.len()
    );
    for e in dec.errors.iter().take(20) {
        println!("  error: {e}");
    }
    if let Ok(class) = std::env::var("RUSTUDIO_SCHEMA_CLASS") {
        if let Some(schema) = &dec.schema {
            for (global, &(ci, pi)) in schema.global_props.iter().enumerate() {
                let cls = &schema.classes[ci as usize];
                let p = &cls.props[pi as usize];
                if class.split(',').any(|wanted|cls.name==wanted) || p.name == "Parent" {
                    println!("schema {global} {pi} {} {:02x}", p.name, p.ty);
                }
            }
        }
    }
    if let Ok(class) = std::env::var("RUSTUDIO_SCHEMA_CLASS") {
        if let Some(schema) = &dec.schema {
            for (global, &(ci,ei)) in schema.global_events.iter().enumerate() {
                let cls=&schema.classes[ci as usize];let event=&cls.events[ei as usize];
                if class.split(',').any(|wanted|cls.name==wanted) {println!("event {global} {} {:?}",event.name,event.arg_types);}
            }
        }
        for inst in model.dom().descendants().filter(|i| class.split(',').any(|wanted|i.class.as_str()==wanted)) {
            println!("service {} id {:?}",inst.name,model.server_id(inst.referent()));
        }
    }
    if let Ok(query) = std::env::var("RUSTUDIO_FIND_INSTANCE") {
        for inst in model.dom().descendants().filter(|i| i.name.to_lowercase().contains(&query.to_lowercase())) {
            println!("instance {} {:?} id {:?}",inst.class,inst.name,model.server_id(inst.referent()));
        }
    }
    if std::env::var("RUSTUDIO_EMITTERS").is_ok() {
        let mut on = 0;
        let mut off = 0;
        let mut missing = 0;
        for inst in model.dom().descendants().filter(|i| i.class.as_str() == "ParticleEmitter") {
            match inst.properties.get(&rbx_dom_weak::ustr("Enabled")) {
                Some(rbx_types::Variant::Bool(true)) => on += 1,
                Some(rbx_types::Variant::Bool(false)) => off += 1,
                other => {
                    missing += 1;
                    if missing <= 3 {
                        println!("emitter {:?} Enabled = {other:?}; props: {:?}", inst.name, inst.properties.keys().map(|k| k.as_str()).collect::<Vec<_>>());
                    }
                }
            }
        }
        if let Ok(name) = std::env::var("RUSTUDIO_EMITTERS") {
            for inst in model.dom().descendants().filter(|i| i.class.as_str() == "ParticleEmitter" && i.name == name).take(2) {
                let mut path = vec![inst.name.clone()];
                let mut p = inst.parent();
                while let Some(pi) = model.dom().get_by_ref(p) { path.push(pi.name.clone()); p = pi.parent(); }
                path.reverse();
                println!("== {}", path.join("."));
                let mut props: Vec<_> = inst.properties.iter().collect();
                props.sort_by_key(|(k, _)| k.as_str().to_string());
                for (k, v) in props { println!("   {} = {:?}", k.as_str(), v); }
            }
        }
        println!("ParticleEmitter Enabled: true {on}, false {off}, missing/other {missing}");
    }
    if std::env::var("RUSTUDIO_LIGHTING").is_ok() {
        for inst in model.dom().descendants().filter(|i| i.class.as_str() == "Lighting") {
            let mut props: Vec<_> = inst.properties.iter().collect();
            props.sort_by_key(|(k, _)| k.as_str().to_string());
            for (k, v) in props { println!("Lighting.{} = {:?}", k.as_str(), v); }
        }
    }
    if std::env::var("RUSTUDIO_UNIONS").is_ok() {
        let mut c = std::collections::BTreeMap::new();
        for inst in model.dom().descendants().filter(|i| i.class.as_str() == "UnionOperation") {
            let k = format!("holder={} md2={}",
                inst.properties.get(&rbx_dom_weak::ustr("SolidMeshHolder")).map(|v| format!("{:?}", v.ty())).unwrap_or("-".into()),
                inst.properties.get(&rbx_dom_weak::ustr("MeshData2")).map(|v| match v { rbx_types::Variant::SharedString(s) => format!("{} bytes", s.data().len()), o => format!("{:?}", o.ty()) }).unwrap_or("-".into()));
            *c.entry(k).or_insert(0) += 1;
        }
        for (k, v) in c { println!("{v} {k}"); }
        if let Ok(path) = std::env::var("RUSTUDIO_DUMP_HOLDER") {
            if let Some(rbx_types::Variant::NetAssetRef(h)) = model.dom().descendants()
                .filter(|i| i.class.as_str() == "UnionOperation")
                .find_map(|i| i.properties.get(&rbx_dom_weak::ustr("SolidMeshHolder")).cloned().filter(|v| matches!(v, rbx_types::Variant::NetAssetRef(_))))
            {
                std::fs::write(&path, h.data()).unwrap();
            }
        }
        for inst in model.dom().descendants().filter(|i| i.class.as_str() == "UnionOperation").take(8) {
            if let Some(rbx_types::Variant::BinaryString(b)) = inst.properties.get(&rbx_dom_weak::ustr("SolidMeshHolder")) {
                let b: &[u8] = b.as_ref();
                println!("holder {:?} id {:?}: {}", inst.name, model.server_id(inst.referent()), b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" "));
            }
        }
    }
    let dom = model.dom();
    if let Ok(path) = std::env::var("RUSTUDIO_SNAPSHOT_OUT") {
        let out = std::io::BufWriter::new(std::fs::File::create(&path).expect("create snapshot"));
        rbx_binary::to_writer(out, dom, dom.root().children()).expect("write snapshot");
        println!("wrote snapshot {path}");
    }
    fn walk(dom: &rbx_dom_weak::WeakDom, r: rbx_dom_weak::types::Ref, d: usize, max: usize) {
        let inst = dom.get_by_ref(r).unwrap();
        println!(
            "{}{} {:?} ({} children)",
            "  ".repeat(d),
            inst.class,
            inst.name,
            inst.children().len()
        );
        if d < max {
            for &c in inst.children() {
                walk(dom, c, d + 1, max);
            }
        }
    }
    let mut by_class = std::collections::BTreeMap::<String, usize>::new();
    for &c in dom.root().children() {
        *by_class
            .entry(dom.get_by_ref(c).unwrap().class.to_string())
            .or_default() += 1;
    }
    let mut v: Vec<_> = by_class.into_iter().collect();
    v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("root children by class: {:?}", &v[..v.len().min(8)]);
    walk(dom, dom.root_ref(), 0, depth);
}
