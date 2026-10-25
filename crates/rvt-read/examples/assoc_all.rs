// Parameter associations of family elements, one line per element (step 2268 parity).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let mut v: Vec<_> = doc.family_param_associations().into_iter().collect();
    v.sort();
    for (id, l) in v {
        let s: Vec<String> = l.iter().map(|(b, f)| format!("{b}:{f}")).collect();
        println!("{id} {}", s.join(","));
    }
}
