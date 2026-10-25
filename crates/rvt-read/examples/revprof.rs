// Profile curves of one revolve (step 2270 check).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let id: u64 = a[2].parse().unwrap();
    for (st, ix, _bl, rv) in doc.family_blends_revolves_by_chain() {
        for r in rv.iter().filter(|r| r.id == id) {
            println!("{st}#{ix} revolve {} axis {:?} angles {:.3}..{:.3}", r.id, r.axis, r.start_angle, r.end_angle);
            for c in &r.profile {
                println!("  {} p0 {:?} p1 {:?} center {:?}", c.kind, c.p0.map(|p| p.map(|v| (v * 1000.0).round() / 1000.0)), c.p1.map(|p| p.map(|v| (v * 1000.0).round() / 1000.0)), c.center);
            }
        }
    }
}
