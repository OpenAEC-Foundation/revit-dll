// Current values of a symbol's family chain (step 2266 check).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let sym: u64 = a[2].parse().expect("symbol");
    let (sf, fc) = (doc.symbol_family(), doc.family_chains());
    let Some((st, ix)) = sf.get(&sym).and_then(|f| fc.get(f)).cloned() else { return };
    let names = doc.parameter_names_of(&st, ix);
    match doc.family_current_values(&st, ix) {
        Some(vs) => {
            println!("{st}#{ix} current values (Family {}):", vs.owner);
            for r in &vs.rows {
                println!("  {:>10} {:<28} {:.4}", r.param, names.get(&(r.param as u64)).cloned().flatten().unwrap_or_default(), r.num);
            }
        }
        None => println!("{st}#{ix}: no current values"),
    }
}
