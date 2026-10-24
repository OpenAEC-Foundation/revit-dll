// Raw bodies of elements, written as <dir>/<id>.bin (format analysis).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    for id in &a[3..] {
        let id: u64 = id.parse().expect("id");
        if let Some(b) = doc.element_body(id) {
            std::fs::write(format!("{}/{id}.bin", a[2]), b).expect("write");
        }
    }
}
