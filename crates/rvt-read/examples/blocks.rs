// All line/arc blocks (header 04/24, no denormals) in an element body, deduplicated (wall sweep check).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let id: u64 = a[2].parse().unwrap();
    let b = doc.element_body(id).expect("body");
    let mut seen: Vec<String> = Vec::new();
    for i in 0..b.len().saturating_sub(40) {
        if i + 8 >= b.len() || !matches!(b[i + 8], 0x04 | 0x24) {
            continue;
        }
        if let Some(c) = rvt_read::curve_block(&b, i) {
            let k = format!("{:?}", c);
            if !seen.contains(&k) {
                println!("@{i} {k}");
                seen.push(k);
            }
        }
    }
}
