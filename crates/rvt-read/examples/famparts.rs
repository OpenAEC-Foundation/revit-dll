// Sweeps, blends and revolves of a symbol's family chain with their point bounds (door frame check).
fn bounds(pts: &[[f64; 3]]) -> String {
    let mut s = String::new();
    for k in 0..3 {
        let lo = pts.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
        let hi = pts.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
        s += &format!(" {}[{lo:.2},{hi:.2}]", ["x", "y", "z"][k]);
    }
    s
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let sym: u64 = a[2].parse().expect("symbol");
    let (sf, fc) = (doc.symbol_family(), doc.family_chains());
    let key = sf.get(&sym).and_then(|f| fc.get(f)).cloned();
    println!("symbol {sym} chain {key:?}");
    let pts = |cs: &[rvt_model::Curve]| -> Vec<[f64; 3]> { cs.iter().flat_map(|c| c.p0.into_iter().chain(c.p1).chain(c.center)).collect() };
    for (st, ix, v) in doc.family_sweeps_by_chain() {
        if Some((st.clone(), ix)) != key {
            continue;
        }
        for s in v {
            println!("sweep {} void {:?} X {:?} Y {:?} N {:?}", s.id, s.void, s.x, s.y, s.n);
            println!("   profile{} ({} curves)", bounds(&pts(&s.profile)), s.profile.len());
            println!("   path{} ({} curves)", bounds(&pts(&s.path)), s.path.len());
        }
    }
    for (st, ix, bl, rv) in doc.family_blends_revolves_by_chain() {
        if Some((st.clone(), ix)) != key {
            continue;
        }
        for b in bl {
            println!("blend {} void {:?} bottom{}", b.id, b.void, bounds(&pts(&b.bottom)));
        }
        for r in rv {
            println!("revolve {} void {:?} profile{}", r.id, r.void, bounds(&pts(&r.profile)));
        }
    }
}
