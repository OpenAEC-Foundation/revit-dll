// Total size of a channel's record bodies (branch open-research).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    let t = std::time::Instant::now();
    let r = rvt_core::records::file_channel(&cfb, &s, a[2].parse().unwrap()).expect("records");
    println!("{} records, {:.1} MB, {:?}", r.len(), r.values().map(|x| x.body.len()).sum::<usize>() as f64 / 1e6, t.elapsed());
}
