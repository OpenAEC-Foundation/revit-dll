// Nested instances of a symbol's family chain (step 2246 check).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let sym: u64 = a[2].parse().expect("symbol");
    let (sf, fc, nsf, nfc) = (doc.symbol_family(), doc.family_chains(), doc.nested_symbol_family(), doc.nested_family_chains());
    let f = sf.get(&sym).or(nsf.get(&sym)).copied();
    let key = f.and_then(|f| fc.get(&f).or(nfc.get(&f)).cloned());
    println!("symbol {sym} family {f:?} chain {key:?}");
    for (st, ix, v) in doc.family_chain_instances() {
        if Some((st.clone(), ix)) != key {
            continue;
        }
        for i in v {
            let f2 = i.symbol.and_then(|s| sf.get(&s).or(nsf.get(&s)).copied());
            let k2 = f2.and_then(|f| fc.get(&f).or(nfc.get(&f)).cloned());
            println!("  nested {} cat {} sym {:?} fam {:?} chain {:?} O {:?}", i.id, i.cat, i.symbol, f2, k2, i.origin);
        }
    }
}
