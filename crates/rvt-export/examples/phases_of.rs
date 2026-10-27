// Created/demolished phase per element (reader `element_phases`) and the phase elements
// (category -2000140: id, name order as in the file) (branch open-research, step 2308).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    for e in doc.elements_in_category(-2000140) {
        println!("phase\t{}\t{}#{}", e.id, e.partition, e.chain);
    }
    for (id, (c, d)) in doc.element_phases() {
        println!("elem\t{id}\t{c}\t{}", d.map_or("-".to_string(), |d| d.to_string()));
    }
}
