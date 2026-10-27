// Created / demolished phase and design option of every element from its channel-102 object
// graph root (branch open-research, step 2308): id, created, demolished, design option.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    let recs = rvt_core::records::file_channel(&cfb, &s, 102).expect("records");
    for (id, r) in &recs {
        let Ok(g) = rvt_core::graph::Graph::decode(&s, &r.body) else { continue };
        let Some(root) = g.objects.first() else { continue };
        let f = |n: &str| root.get(n).and_then(|v| v.element_id()).map_or("-".to_string(), |x| x.to_string());
        println!("{id}\t{}\t{}\t{}", f("m_createdPhaseId"), f("m_demolishedPhaseId"), f("m_designOptionId"));
    }
}
