// Elements that could be drawn from saved graphics: id, category, class, triangle count (branch
// open-research, step 2304).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let sg = doc.saved_graphics().expect("saved graphics");
    let mut m = rvt_export::saved::SavedMesher::new(sg);
    let mut skip = doc.group_definition_ids();
    skip.extend(doc.element_phases().into_iter().filter(|(_, (_, d))| d.is_some()).map(|(id, _)| id));
    skip.extend(doc.secondary_option_elements());
    let mut seen = std::collections::HashSet::new();
    for e in doc.elements() {
        if e.category == -1 || e.view.is_some_and(|v| v != -1) || !seen.insert(e.id) || skip.contains(&e.id) {
            continue;
        }
        let class = e.class_tag.and_then(|t| sg.schema.class(t)).map_or("?", |c| c.name.as_str());
        let t = m.element(e.id);
        let n = t.as_ref().map_or(0, |t| t.len());
        // the triangles' box and the stored box (xmin ymin zmin xmax ymax zmax)
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in t.iter().flatten().flatten() {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let bb = e.bbox.map_or(String::new(), |b| format!("{:.3?}", b));
        println!("{}\t{}\t{}\t{}\t{:.3?}\t{:.3?}\t{}\t{}", e.id, e.category, class, n, lo, hi, bb, e.chain);
    }
}
