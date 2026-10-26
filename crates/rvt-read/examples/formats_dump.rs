// Inflated Formats/Latest stream to a file (schema analysis, branch open-research).
use std::io::Read;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let raw = cfb.stream("Formats/Latest").expect("stream");
    let raw = rvt_core::chain::strip_inserted(&raw);
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut out).expect("gzip");
    std::fs::write(&a[2], &out).expect("write");
    println!("raw {} inflated {}", raw.len(), out.len());
}
