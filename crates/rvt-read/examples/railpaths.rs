// Referenced elements of a rail with their path pieces (step 2285 analysis).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let id: u64 = a[2].parse().unwrap();
    let b = doc.element_body(id).expect("body");
    let mut seen = std::collections::HashSet::new();
    for o in 0..b.len() - 7 {
        let q = i64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        if q <= 0 || q as u64 == id || !seen.insert(q) {
            continue;
        }
        let Some(e) = doc.element(q as u64) else { continue };
        let Some(pb) = doc.element_body(q as u64) else { continue };
        let pcs = rvt_read::path_pieces_pub(&pb);
        let zs: Vec<f64> = pcs.iter().flat_map(|c| [c.p0.unwrap()[2], c.p1.unwrap()[2]]).collect();
        let (lo, hi) = zs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), z| (l.min(*z), h.max(*z)));
        println!("@{o} {q} cat {} class {:?} len {} pieces {} z {lo:.2}..{hi:.2}", e.category, e.class_tag, pb.len(), pcs.len());
    }
}
