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
        let n = m.element(e.id).map_or(0, |t| t.len());
        println!("{}\t{}\t{}\t{}", e.id, e.category, class, n);
    }
}
