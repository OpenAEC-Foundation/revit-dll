// Saved-graphics triangles of given elements: count, area (m²), bounding box (ft) (branch open-research).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let t0 = std::time::Instant::now();
    let sg = doc.saved_graphics().expect("saved graphics");
    eprintln!("load {:?}", t0.elapsed());
    let mut m = rvt_export::saved::SavedMesher::new(sg);
    // RVT_SAVED_OUT=file: the triangles as RGT1 (id, category 0, count, 9 f32 each)
    let mut rgt: Vec<u8> = b"RGT1".to_vec();
    // `@file`: the ids one per line
    let ids: Vec<String> = a[2..].iter().flat_map(|x| match x.strip_prefix('@') {
        Some(f) => std::fs::read_to_string(f).expect("ids").lines().map(str::to_string).collect(),
        None => vec![x.clone()],
    }).collect();
    let quiet = std::env::var("RVT_SAVED_QUIET").is_ok();
    for id in &ids {
        let id: u64 = id.trim().parse().unwrap();
        match m.element(id).or_else(|| rvt_export::rebar::element(sg, id)) {
            Some(t) => {
                let area: f64 = t.iter().map(|t| {
                    let (u, v) = ([t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]], [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]]);
                    let c = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                    (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() / 2.0
                }).sum::<f64>() * 0.0929;
                let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for p in t.iter().flatten() { for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } }
                if !quiet { println!("{id}: {} triangles, {area:.3} m2, box {:.2?} {:.2?}", t.len(), lo, hi); }
                rgt.extend((id as i64).to_le_bytes());
                rgt.extend(0i32.to_le_bytes());
                rgt.extend((t.len() as i32).to_le_bytes());
                for p in t.iter().flatten().flatten() {
                    rgt.extend((*p as f32).to_le_bytes());
                }
            }
            None => if !quiet { println!("{id}: none") },
        }
    }
    println!("faces {} skipped {} mesh {:?} decode {:?}", m.faces, m.skipped, t0.elapsed(), m.decode_time);
    if let Ok(path) = std::env::var("RVT_SAVED_OUT") {
        std::fs::write(path, rgt).expect("write");
    }
}
