// Decode one channel record with the schema (branch open-research); RVT_GRAPH_TRACE=1 traces objects.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    let recs = rvt_core::records::file_channel(&cfb, &s, a[2].parse().unwrap()).expect("records");
    let r = &recs[&a[3].parse::<u64>().unwrap()];
    match rvt_core::graph::Graph::decode(&s, &r.body) {
        Ok(g) => println!("{} objects", g.objects.len()),
        Err(e) => println!("{e}"),
    }
}
