// For given ids: group-definition copy? phases (created, demolished)? (format analysis)
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let g = doc.group_definition_ids();
    let ph = doc.element_phases();
    for id in &a[2..] {
        let id: u64 = id.parse().unwrap();
        println!("{id} groupdef {} phases {:?}", g.contains(&id), ph.get(&id));
    }
}
