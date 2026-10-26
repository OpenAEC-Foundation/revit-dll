// For given ids: in-place member? secondary design option? (step 2297 analysis)
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let inplace = doc.inplace_ids();
    let opt = doc.secondary_option_elements();
    for id in &a[2..] {
        let id: u64 = id.parse().unwrap();
        println!("{id} inplace {} secondary_option {}", inplace.contains(&id), opt.contains(&id));
    }
}
