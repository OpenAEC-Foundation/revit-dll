// Roofs by face on a mass with the mass's model lines (step 2290 analysis).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    for (r, cs) in doc.mass_roofs() {
        println!("roof {r}");
        for c in cs {
            println!("  {} {:?} {:?}", c.kind, c.p0.map(|p| p.map(|v| (v * 100.0).round() / 100.0)), c.p1.map(|p| p.map(|v| (v * 100.0).round() / 100.0)));
        }
    }
}
