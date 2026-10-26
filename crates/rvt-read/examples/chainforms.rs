// Forms (extrusions, sweeps, blends, revolves) of given chains: id, kind, void flag (format analysis).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let part = &a[2];
    let want: Vec<usize> = a[3..].iter().map(|s| s.parse().unwrap()).collect();
    for (st, ix, v) in doc.family_solids_by_chain() {
        if &st == part && want.contains(&ix) {
            for s in v {
                println!("chain {ix} extrusion {} void {:?}", s.id, s.void);
            }
        }
    }
    for (st, ix, v) in doc.family_sweeps_by_chain() {
        if &st == part && want.contains(&ix) {
            for s in v {
                println!("chain {ix} sweep {} void {:?}", s.id, s.void);
            }
        }
    }
    for (st, ix, b, r) in doc.family_blends_revolves_by_chain() {
        if &st == part && want.contains(&ix) {
            for s in b {
                println!("chain {ix} blend {} void {:?}", s.id, s.void);
            }
            for s in r {
                println!("chain {ix} revolve {} void {:?}", s.id, s.void);
            }
        }
    }
}
