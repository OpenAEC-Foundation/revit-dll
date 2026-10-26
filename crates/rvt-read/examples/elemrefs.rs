// i32/i64 values in an element's body that are element ids, with category and class (format analysis).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let id: u64 = a[2].parse().unwrap();
    let b = doc.element_body(id).expect("body");
    let mut seen = std::collections::HashSet::new();
    for o in 0..b.len().saturating_sub(3) {
        let v = i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        if v <= 0 || v as u64 == id || !seen.insert(v) {
            continue;
        }
        if let Some(e) = doc.element(v as u64) {
            let len = doc.element_body(v as u64).map_or(0, |x| x.len());
            println!("@{o} {v} cat {} class {:?} len {len} bbox {}", e.category, e.class_tag, e.bbox.is_some());
        }
    }
}
