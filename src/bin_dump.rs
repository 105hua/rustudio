//! Development aid: prints a model file's instance tree with the
//! properties that matter for characters (`cargo test dump_model_file --
//! --ignored --nocapture` with RUSTUDIO_DUMP=path).

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn dump_model_file() {
        let Ok(path) = std::env::var("RUSTUDIO_DUMP") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let doc = crate::doc::Document::decode(&bytes).expect("decode");
        let keys: Vec<String> = std::env::var("RUSTUDIO_DUMP_PROPS")
            .unwrap_or_else(|_| {
                "MeshId,MeshContent,TextureID,Size,C0,C1,CFrame,Part0,Part1,Value,AnimationId,Scale,\
                 CageMeshId,ReferenceMeshId,Puffiness,Order,MeshType,BindOffset,AnimationContent"
                    .into()
            })
            .split(',')
            .map(String::from)
            .collect();
        fn walk(
            doc: &rbx_dom_weak::WeakDom,
            r: rbx_dom_weak::types::Ref,
            depth: usize,
            keys: &[String],
        ) {
            let Some(i) = doc.get_by_ref(r) else {
                return;
            };
            let mut props: Vec<String> = i
                .properties
                .iter()
                .filter(|(k, _)| keys.iter().any(|w| w == k.as_str()))
                .map(|(k, v)| {
                    let s = format!("{k}={v:?}");
                    let limit = std::env::var("RUSTUDIO_DUMP_CHARS")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(220);
                    s.chars().take(limit).collect()
                })
                .collect();
            props.sort();
            println!(
                "{}{} {:?} {}",
                "  ".repeat(depth),
                i.class,
                i.name,
                props.join(" ")
            );
            for c in i.children() {
                walk(doc, *c, depth + 1, keys);
            }
        }
        walk(&doc, doc.root_ref(), 0, &keys);
    }
}

#[cfg(test)]
mod mesh_dump {
    /// Prints a mesh file's bounds (RUSTUDIO_MESH=path; development aid).
    #[test]
    #[ignore]
    fn dump_mesh_bounds() {
        for p in std::env::var("RUSTUDIO_MESH")
            .unwrap_or_default()
            .split(',')
        {
            if let Ok(b) = std::fs::read(p) {
                match crate::mesh::parse(&b) {
                    Ok(m) => println!(
                        "{p}: min {:?} max {:?} verts {}",
                        m.min,
                        m.max,
                        m.positions.len()
                    ),
                    Err(e) => println!("{p}: {e}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod client_copy {
    /// Builds a place's first client copy (RUSTUDIO_PLACE=path) and reports
    /// what the client sees at its root (development aid).
    #[test]
    #[ignore]
    fn client_copy_root() {
        let Ok(path) = std::env::var("RUSTUDIO_PLACE") else { return };
        let doc = crate::doc::Document::open(std::path::Path::new(&path)).expect("open");
        let root = doc.dom.root_ref();
        let kids: Vec<String> = doc.dom.get_by_ref(root).unwrap().children().iter()
            .filter_map(|c| doc.dom.get_by_ref(*c)).map(|i| format!("{}:{}", i.class, i.name)).collect();
        println!("server root children ({}): {:?}", kids.len(), &kids[..kids.len().min(40)]);
        println!("server workspace {:?}", doc.workspace());
        let (c, _) = crate::replicate::initial_client(&doc, "c".into());
        let ckids: Vec<String> = c.dom.get_by_ref(c.dom.root_ref()).unwrap().children().iter()
            .filter_map(|x| c.dom.get_by_ref(*x)).map(|i| format!("{}:{}", i.class, i.name)).collect();
        println!("client root children ({}): {:?}", ckids.len(), &ckids[..ckids.len().min(40)]);
        println!("client workspace {:?}", c.workspace());
    }
}

#[cfg(test)]
mod light_dump {
    use rbx_dom_weak::types::Variant;
    use rbx_dom_weak::ustr;

    /// Lists a place's lights and its Lighting style (RUSTUDIO_PLACE=path;
    /// development aid for the fidelity render diff).
    #[test]
    #[ignore]
    fn dump_lights() {
        let Ok(path) = std::env::var("RUSTUDIO_PLACE") else { return };
        let doc = crate::doc::Document::open(std::path::Path::new(&path)).expect("open");
        let l = doc.find_child(doc.dom.root_ref(), "Lighting").unwrap();
        let li = doc.dom.get_by_ref(l).unwrap();
        for k in ["Technology", "LightingStyle", "PrioritizeLightingQuality"] {
            println!("Lighting.{k} = {:?}", li.properties.get(&ustr(k)));
        }
        for i in doc.dom.descendants() {
            if !matches!(i.class.as_str(), "PointLight" | "SpotLight" | "SurfaceLight") {
                continue;
            }
            let p = |k: &str| match i.properties.get(&ustr(k)) {
                Some(Variant::Float32(v)) => format!("{v:.2}"),
                Some(Variant::Bool(b)) => format!("{b}"),
                Some(Variant::Color3(c)) => format!("({:.2},{:.2},{:.2})", c.r, c.g, c.b),
                Some(Variant::Enum(e)) => format!("{}", e.to_u32()),
                _ => "-".into(),
            };
            let parent = doc.dom.get_by_ref(i.parent()).map(|p| p.name.clone()).unwrap_or_default();
            println!(
                "{} in {parent}: B {} R {} A {} face {} col {} shadows {} enabled {}",
                i.class,
                p("Brightness"),
                p("Range"),
                p("Angle"),
                p("Face"),
                p("Color"),
                p("Shadows"),
                p("Enabled")
            );
        }
    }
}

#[cfg(test)]
mod project_diff {
    /// Diffs a project folder's DataModel (RUSTUDIO_PROJECT) against a place
    /// file (RUSTUDIO_PLACE): paths only in one, and class / Source
    /// differences (development aid for the project reader).
    #[test]
    #[ignore]
    fn diff_project_place() {
        let (Ok(proj), Ok(place)) = (std::env::var("RUSTUDIO_PROJECT"), std::env::var("RUSTUDIO_PLACE")) else {
            return;
        };
        let files = crate::project::project_files(std::path::Path::new(&proj));
        let (a, _) = crate::project_sync::open(&files[0]).expect("project");
        let b = crate::doc::Document::open(std::path::Path::new(&place)).expect("place");
        use std::collections::BTreeMap;
        let paths = |d: &crate::doc::Document| {
            let mut m: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
            for i in d.dom.descendants() {
                let mut p = i.name.clone();
                let mut cur = i.parent();
                while let Some(x) = d.dom.get_by_ref(cur) {
                    if x.parent().is_none() {
                        break;
                    }
                    p = format!("{}/{}", x.name, p);
                    cur = x.parent();
                }
                let src = match i.properties.get(&rbx_dom_weak::ustr("Source")) {
                    Some(rbx_dom_weak::types::Variant::String(s)) => Some(s.clone()),
                    _ => None,
                };
                m.insert(p, (i.class.to_string(), src));
            }
            m
        };
        let (pa, pb) = (paths(&a), paths(&b));
        let only_a: Vec<_> = pa.keys().filter(|k| !pb.contains_key(*k)).collect();
        let only_b: Vec<_> = pb.keys().filter(|k| !pa.contains_key(*k)).collect();
        let differ: Vec<_> = pa
            .iter()
            .filter(|(k, v)| pb.get(*k).is_some_and(|w| w.0 != v.0 || w.1 != v.1))
            .map(|(k, v)| format!("{k}: {} vs {}", v.0, pb[k].0))
            .collect();
        println!("project {} / place {}", pa.len(), pb.len());
        println!("only in project ({}): {:?}", only_a.len(), &only_a[..only_a.len().min(25)]);
        println!("only in place ({}): {:?}", only_b.len(), &only_b[..only_b.len().min(40)]);
        println!("differ ({}): {:?}", differ.len(), &differ[..differ.len().min(25)]);
        for (k, v) in pa.iter().filter(|(k, v)| pb.get(*k).is_some_and(|w| w.1 != v.1)).take(2) {
            let (x, y) = (v.1.clone().unwrap_or_default(), pb[k].1.clone().unwrap_or_default());
            let at = x.chars().zip(y.chars()).position(|(a, b)| a != b).unwrap_or(x.len().min(y.len()));
            let ctx = |s: &str| s.chars().skip(at.saturating_sub(60)).take(140).collect::<String>();
            println!("source {k} differs at {at} (lengths {} / {}):\n  project: {:?}\n  place:   {:?}", x.len(), y.len(), ctx(&x), ctx(&y));
        }
        // Every property, by path (first instance of a path wins).
        let props = |d: &crate::doc::Document| {
            let mut m: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
            for i in d.dom.descendants() {
                let mut p = i.name.clone();
                let mut cur = i.parent();
                while let Some(x) = d.dom.get_by_ref(cur) {
                    if x.parent().is_none() {
                        break;
                    }
                    p = format!("{}/{}", x.name, p);
                    cur = x.parent();
                }
                let e = m.entry(p).or_default();
                if e.is_empty() {
                    for (k, v) in &i.properties {
                        if !matches!(v, rbx_dom_weak::types::Variant::Ref(_) | rbx_dom_weak::types::Variant::UniqueId(_) | rbx_dom_weak::types::Variant::SharedString(_)) {
                            e.insert(k.to_string(), format!("{v:?}").chars().take(80).collect());
                        }
                    }
                }
            }
            m
        };
        let (qa, qb) = (props(&a), props(&b));
        let mut n = 0;
        for (p, pa) in &qa {
            let Some(pb) = qb.get(p) else { continue };
            for (k, v) in pa {
                if pb.get(k) != Some(v) && n < 30 {
                    println!("prop {p}.{k}: project {v} / place {:?}", pb.get(k));
                    n += 1;
                }
            }
            // (Properties only the place stores are its written-out defaults.)
            let _ = pb;
        }
    }
}

#[cfg(test)]
mod attr_dump {
    /// Prints the raw attribute variants of instances named RUSTUDIO_NAME in
    /// RUSTUDIO_PLACE (development aid: which attribute types survive loading).
    #[test]
    #[ignore]
    fn dump_attributes() {
        let (Ok(path), Ok(name)) = (std::env::var("RUSTUDIO_PLACE"), std::env::var("RUSTUDIO_NAME")) else {
            return;
        };
        let doc = crate::doc::Document::open(std::path::Path::new(&path)).expect("open");
        for i in doc.dom.descendants() {
            if i.name != name {
                continue;
            }
            for (k, v) in &i.properties {
                if k.as_str().contains("ttribute") {
                    println!("{} {}: {:?}", i.class, k, v);
                }
            }
        }
    }
}
