// Records of one partition channel (101/102/103) for given element ids (branch open-research).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    let ch: u64 = a[2].parse().unwrap();
    let t = std::time::Instant::now();
    let recs = rvt_core::records::file_channel(&cfb, &s, ch).expect("records");
    println!("channel {ch}: {} records in {:?}", recs.len(), t.elapsed());
    for id in &a[3..] {
        let id: u64 = id.parse().unwrap();
        match recs.get(&id) {
            Some(r) => {
                let tag = u16::from_le_bytes([r.body[0], r.body[1]]);
                println!("{id}: {} bytes, root class {} ({:?})", r.body.len(), tag, s.class(tag).map(|c| &c.name));
            }
            None => println!("{id}: none"),
        }
    }
}
