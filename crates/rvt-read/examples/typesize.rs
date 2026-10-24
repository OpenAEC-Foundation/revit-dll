// Type values of a symbol next to its family chain's value sets, by parameter name (step 2253 check).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let sym: u64 = a[2].parse().expect("symbol");
    let (sf, fc) = (doc.symbol_family(), doc.family_chains());
    for (st, ix, tv) in doc.type_values_by_chain() {
        let Some(vs) = tv.get(&sym) else { continue };
        let names = doc.parameter_names_of(&st, ix);
        println!("type '{}' in {st}#{ix}:", vs.type_name);
        for r in &vs.rows {
            if r.num.abs() > 1e-9 {
                println!("  {:>10} {:<28} {:.4}", r.param, names.get(&(r.param as u64)).cloned().flatten().unwrap_or_default(), r.num);
            }
        }
    }
    let Some(f) = sf.get(&sym) else { return };
    let Some((st, ix)) = fc.get(f).cloned() else { return };
    let names = doc.parameter_names_of(&st, ix);
    for vs in doc.value_sets_of(&st, ix) {
        println!("family chain {st}#{ix} set owner {} type '{}':", vs.owner, vs.type_name);
        for r in vs.rows.iter().filter(|r| r.num.abs() > 1e-9) {
            println!("  {:>10} {:<28} {:.4}", r.param, names.get(&(r.param as u64)).cloned().flatten().unwrap_or_default(), r.num);
        }
    }
}
