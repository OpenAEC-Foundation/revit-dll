// Host walls of family documents (step 2257 check).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let only: Option<usize> = a.get(2).and_then(|v| v.parse().ok());
    let mut v: Vec<_> = doc.family_host_walls().into_iter().collect();
    v.sort_by(|x, y| x.0.cmp(&y.0));
    for ((st, ix), ws) in v {
        if only.is_some_and(|o| o != ix) {
            continue;
        }
        for (w, t) in ws {
            println!("{st}#{ix} wall {} type {} width {t:?} p0 {:?} p1 {:?} h {:?}", w.id, w.wall_type, w.p0, w.p1, w.height);
        }
    }
}
