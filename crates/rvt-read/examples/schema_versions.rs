// Version word of given schema classes and how many classes use each value (branch open-research).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    let mut hist = std::collections::BTreeMap::<u32, usize>::new();
    for c in &s.classes {
        *hist.entry(c.version).or_default() += 1;
    }
    println!("version histogram (top): {:?}", hist.iter().take(12).collect::<Vec<_>>());
    for n in &a[2..] {
        if let Some(c) = s.tag(n).and_then(|t| s.class(t)) {
            println!("{n}: version {:#x} parent {:?}", c.version, c.parent.and_then(|p| s.class(p)).map(|p| (&p.name, p.version)));
        }
    }
}
