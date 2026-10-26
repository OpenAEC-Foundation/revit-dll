// Chain curves of a symbol's family as JSON (step 2285 analysis).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let (part, ix): (&str, usize) = (&a[2], a[3].parse().unwrap());
    for (id, c) in doc.chain_curves(part, ix) {
        println!("{id} {c:?}");
    }
}
