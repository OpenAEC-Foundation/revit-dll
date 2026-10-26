// Classes of a file's schema (Formats/Latest), optionally one class with its fields (branch open-research).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    println!("classes {}", s.classes.len());
    for name in &a[2..] {
        let Some(t) = s.tag(name) else { println!("{name}: not found"); continue };
        println!("{name} tag {t} parent {:?}", s.class(t).and_then(|c| c.parent).and_then(|p| s.class(p)).map(|c| &c.name));
        for (c, f) in s.all_fields(t) {
            println!("   {}::{} desc {:#x} count {} class {:?}", c.name, f.name, f.desc, f.count, f.class.and_then(|t| s.class(t)).map(|c| &c.name));
        }
    }
}
