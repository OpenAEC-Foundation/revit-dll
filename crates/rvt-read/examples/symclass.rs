// Class tags of the types (symbols) used by project instances (type value coverage check).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let mut c = std::collections::BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for i in doc.instances() {
        let Some(s) = i.symbol else { continue };
        if !seen.insert(s) {
            continue;
        }
        let t = doc.element(s).and_then(|e| e.class_tag);
        *c.entry(t).or_insert(0usize) += 1;
    }
    let tv: usize = doc.type_values_by_chain().iter().map(|(_, _, v)| v.len()).sum();
    println!("symbol class tags {c:?}; type value sets {tv}");
}
