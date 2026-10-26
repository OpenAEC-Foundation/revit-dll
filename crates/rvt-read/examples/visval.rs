// Value rows of a symbol (by name) and the current values of a chain, for one parameter id/name.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let sym: u64 = a[2].parse().unwrap();
    let (part, ix): (&str, usize) = (&a[3], a[4].parse().unwrap());
    let names = doc.parameter_names_global();
    for (_, _, m) in doc.type_values_by_chain() {
        if let Some(vs) = m.get(&sym) {
            for r in &vs.rows {
                let n = if r.param > 0 { names.get(&(r.param as u64)).cloned().flatten() } else { None };
                println!("sym row param {} {:?} int {} num {}", r.param, n, r.int, r.num);
            }
        }
    }
    if let Some(c) = doc.family_current_values(part, ix) {
        let fnames = doc.parameter_names_of(part, ix);
        for r in &c.rows {
            let n = if r.param > 0 { fnames.get(&(r.param as u64)).cloned().flatten() } else { None };
            println!("current param {} {:?} int {} num {}", r.param, n, r.int, r.num);
        }
    } else {
        println!("no current values");
    }
}
