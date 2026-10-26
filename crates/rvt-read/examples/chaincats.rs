// Element categories and class tags of one chain: count per (category, class tag).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let (part, ix): (&str, usize) = (&a[2], a[3].parse().unwrap());
    let mut c = std::collections::BTreeMap::new();
    for e in doc.elements().filter(|e| e.partition == part && e.chain == ix) {
        let k = (e.category, e.class_tag);
        let v = c.entry(k).or_insert((0, Vec::new()));
        v.0 += 1;
        if v.1.len() < 4 {
            v.1.push(e.id);
        }
    }
    for ((cat, tag), (n, ids)) in c {
        println!("cat {cat} tag {tag:?} n {n} e.g. {ids:?}");
    }
}
