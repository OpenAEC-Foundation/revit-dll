// Parameter names of one chain: id name.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let ix: usize = a[3].parse().unwrap();
    let mut v: Vec<_> = doc.parameter_names_of(&a[2], ix).into_iter().collect();
    v.sort();
    for (k, n) in v {
        println!("{k} {}", n.unwrap_or_default());
    }
}
