// Global parameter names for given ids.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let n = doc.parameter_names_global();
    for id in &a[2..] {
        let id: u64 = id.parse().unwrap();
        println!("{id} {:?}", n.get(&id));
    }
}
