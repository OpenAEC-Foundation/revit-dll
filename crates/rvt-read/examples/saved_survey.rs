// Per file: version, schema, records per channel 101/102/103, channel-103 decoding (branch
// open-research, par B: saved graphics of older files).
fn main() {
    for f in std::env::args().skip(1) {
        let name = std::path::Path::new(&f).file_name().unwrap().to_string_lossy().to_string();
        let cfb = match rvt_core::Cfb::open(&f) {
            Ok(c) => c,
            Err(e) => {
                println!("{name}\topen error {e}");
                continue;
            }
        };
        let parts = rvt_core::read_partitions(&cfb).ok();
        let ver = parts.as_ref().and_then(|p| p.iter().flat_map(|p| &p.chains).find_map(|c| c.version.map(|v| (v, c.bits))));
        let s = match rvt_core::Schema::read(&cfb) {
            Ok(s) => s,
            Err(e) => {
                println!("{name}\t{ver:?}\tschema error {e}");
                continue;
            }
        };
        let markers = rvt_core::records::Markers::from_schema(&s);
        let mut line = format!("{name}\t{ver:?}\tclasses {}\tGElement {:?}\tmarkers {}", s.classes.len(), s.tag("GElement"), markers.is_some());
        for ch in [101u64, 102, 103] {
            match rvt_core::records::file_channel(&cfb, &s, ch) {
                Ok(r) => {
                    line += &format!("\tch{ch} {}", r.len());
                    if ch == 103 {
                        let (mut ok, mut ge) = (0, 0);
                        let g = s.tag("GElement");
                        for r in r.values() {
                            if rvt_core::graph::Graph::decode(&s, &r.body).is_ok() {
                                ok += 1;
                            }
                            if g.is_some_and(|g| r.body.len() >= 2 && u16::from_le_bytes([r.body[0], r.body[1]]) == g) {
                                ge += 1;
                            }
                        }
                        line += &format!(" (decoded {ok}, GElement root {ge})");
                        if std::env::var_os("SURVEY_ROOTS").is_some() {
                            let mut h = std::collections::BTreeMap::<String, (usize, usize)>::new();
                            for r in r.values() {
                                let k = if r.body.len() < 2 { format!("<{} bytes>", r.body.len()) } else { s.class(u16::from_le_bytes([r.body[0], r.body[1]])).map_or("?".into(), |c| c.name.clone()) };
                                let e = h.entry(k).or_default();
                                e.0 += 1;
                                e.1 += r.body.len();
                            }
                            let mut v: Vec<_> = h.into_iter().collect();
                            v.sort_by_key(|x| std::cmp::Reverse(x.1 .0));
                            line += &format!("\n  roots {:?}", &v[..v.len().min(12)]);
                        }
                    }
                }
                Err(e) => line += &format!("\tch{ch} ERR {}", e.to_string().chars().take(80).collect::<String>()),
            }
        }
        println!("{line}");
    }
}
