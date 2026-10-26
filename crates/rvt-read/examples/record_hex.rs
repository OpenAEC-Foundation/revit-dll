// Hex of one channel record body between two offsets (branch open-research).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    let recs = rvt_core::records::file_channel(&cfb, &s, a[2].parse().unwrap()).expect("records");
    let r = &recs[&a[3].parse::<u64>().unwrap()];
    let (from, to): (usize, usize) = (a[4].parse().unwrap(), a[5].parse().unwrap());
    let to = to.min(r.body.len());
    for o in (from..to).step_by(16) {
        let row: Vec<String> = r.body[o..(o + 16).min(to)].iter().map(|b| format!("{b:02x}")).collect();
        println!("{o:6}: {}", row.join(" "));
    }
}
