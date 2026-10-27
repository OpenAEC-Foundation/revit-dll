// Rebar records of channel 102 (SavedGraphics::objects): class, decoding, triangles, and the field
// names along the path rebar.rs follows (branch open-research, par B: rebar of older files).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let sg = doc.saved_graphics().expect("saved graphics");
    let s = &sg.schema;
    let mut hist = std::collections::BTreeMap::<String, (usize, usize, usize)>::new();
    let mut shown = 0;
    let mut ids: Vec<&u64> = sg.objects.keys().collect();
    ids.sort();
    for id in ids {
        let body = &sg.objects[id];
        let root = s.class(u16::from_le_bytes([body[0], body[1]])).map_or("?".to_string(), |c| c.name.clone());
        let g = rvt_core::graph::Graph::decode(s, body);
        let tris = rvt_export::rebar::element(sg, *id).map_or(0, |t| t.len());
        let e = hist.entry(root.clone()).or_default();
        e.0 += 1;
        e.1 += g.is_ok() as usize;
        e.2 += (tris > 0) as usize;
        if std::env::var_os("PROBE_103").is_some() && (root == "Rebar" || root == "RebarInSystem") {
            // the channel-103 record: classes and the curve lengths
            let mut h = std::collections::BTreeMap::<String, usize>::new();
            let mut len = 0.0;
            if let Some(g) = sg.bodies.get(id).and_then(|b| rvt_core::graph::Graph::decode(s, b).ok()) {
                for (i, o) in g.objects.iter().enumerate() {
                    *h.entry(g.class_name(s, i).unwrap_or("?").to_string()).or_default() += 1;
                    if let Some(e) = o.get("m_endParams").and_then(|v| v.floats()) {
                        let r = o.get("m_radius").and_then(|v| v.as_f64()).unwrap_or(1.0);
                        len += (e[1] - e[0]).abs() * r;
                    }
                }
            }
            let nbars = g.as_ref().ok().and_then(|g| g.objects.iter().find_map(|o| o.get("m_numberOfBarPositions").and_then(|v| v.as_i64())));
            println!("103 {id} {root}: bars {nbars:?} curve length {len:.2} ft {h:?}");
        }
        if (root == "Rebar" || root == "RebarInSystem") && tris == 0 && shown < 3 {
            shown += 1;
            match &g {
                Err(err) => println!("{id} {root}: {err}"),
                Ok(g) => {
                    let names = |o: &rvt_core::graph::Object| o.fields.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>().join(",");
                    println!("{id} {root}: root fields {}", names(&g.objects[0]));
                    for (i, o) in g.objects.iter().enumerate().skip(1).take(8) {
                        println!("   #{i} {}: {}", g.class_name(s, i).unwrap_or("?"), names(o));
                    }
                }
            }
        }
    }
    for (k, (n, ok, t)) in hist {
        println!("{k}: {n} records, {ok} decoded, {t} with triangles");
    }
}
