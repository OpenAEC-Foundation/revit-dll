// Current values of every family chain, one line per chain (step 2266 parity).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let mut keys: Vec<(String, usize)> = doc.family_chains().into_values().chain(doc.nested_family_chains().into_values()).collect();
    keys.sort();
    keys.dedup();
    for (st, ix) in keys {
        if let Some(vs) = doc.family_current_values(&st, ix) {
            let rows: Vec<String> = vs.rows.iter().map(|r| format!("{}:{:?}:{}", r.param, r.num, r.elem)).collect();
            println!("{st} {ix} {} {}", vs.owner, rows.join(","));
        } else {
            println!("{st} {ix} -");
        }
    }
}
