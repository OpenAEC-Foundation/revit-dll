// Curves of one chain (profile family check).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let ix: usize = a[3].parse().unwrap();
    for (id, c) in doc.chain_curves(&a[2], ix) {
        let r = |p: Option<[f64; 3]>| p.map(|p| p.map(|v| (v * 1000.0).round() / 1000.0));
        println!("{id} {} p0 {:?} p1 {:?} c {:?} r {:?}", c.kind, r(c.p0), r(c.p1), r(c.center), c.r);
    }
}
