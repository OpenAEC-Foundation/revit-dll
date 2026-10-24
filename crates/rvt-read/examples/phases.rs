// Phase created/demolished per project element (step 2260 check): id cat created demolished.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let mut v: Vec<_> = doc.element_phases().into_iter().collect();
    v.sort();
    for (id, (c, d)) in v {
        let cat = doc.element(id).map(|e| e.category).unwrap_or(0);
        println!("{id} {cat} {c} {}", d.unwrap_or(-1));
    }
}
