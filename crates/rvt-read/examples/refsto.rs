// Elements whose body holds one of the given ids as an i64 (format analysis).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let want: Vec<i64> = a[2..].iter().map(|s| s.parse().expect("id")).collect();
    for e in doc.elements() {
        let Some(b) = doc.element_body(e.id) else { continue };
        for o in 0..b.len().saturating_sub(7) {
            let v = i64::from_le_bytes(b[o..o + 8].try_into().unwrap());
            if want.contains(&v) && v as u64 != e.id {
                println!("{} cat {} len {} @{} -> {} bbox {:?}", e.id, e.category, b.len(), o, v, e.bbox);
            }
        }
    }
}
