// Flag word of extrusion records: the u16 before `00 00 00 00 01 02 00 00 00` (visibility check).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    const PAT: [u8; 9] = [0, 0, 0, 0, 1, 2, 0, 0, 0];
    for (st, ix, v) in doc.family_solids_by_chain() {
        for s in v {
            let Some(b) = doc.element_body(s.id) else { continue };
            let Some(k) = (2..b.len().min(400).saturating_sub(9)).find(|&k| b[k..k + 9] == PAT) else {
                println!("{st}#{ix} {} none", s.id);
                continue;
            };
            println!("{st}#{ix} {} {:02x}{:02x} void {:?}", s.id, b[k - 2], b[k - 1], s.void);
        }
    }
}
