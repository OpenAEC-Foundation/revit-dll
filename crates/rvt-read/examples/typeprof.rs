// i64 references of a type record that are family symbols, with their profile chain curves (format analysis).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let t: u64 = a[2].parse().unwrap();
    let b = doc.element_body(t).expect("body");
    let (sf, fc) = (doc.symbol_family(), doc.family_chains());
    println!("len {}", b.len());
    for o in 0..b.len().saturating_sub(7) {
        let v = if std::env::var("I32").is_ok() { i32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as i64 } else { i64::from_le_bytes(b[o..o + 8].try_into().unwrap()) };
        if v <= 0 {
            continue;
        }
        if let Some(key) = sf.get(&(v as u64)).and_then(|f| fc.get(f)) {
            println!("@{o} symbol {v} chain {key:?}");
            for (id, c) in doc.chain_curves(&key.0, key.1) {
                println!("   {id} {} {:?} {:?} {:?}", c.kind, c.p0, c.p1, c.center);
            }
        }
    }
}
