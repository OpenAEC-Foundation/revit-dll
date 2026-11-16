//! Wall-    eprintln!("{} chains, {} MiB, largest {} MiB", chains.iter().map(|c| c.len()).sum::<usize>(), chains.iter().flatten().map(|c| c.data.len()).sum::<usize>() >> 20, chains.iter().flatten().map(|c| c.data.len()).max().unwrap_or(0) >> 20);lock time of the steps of opening a file and of reading its saved graphics.
//!
//!   cargo run --release -p rvt-read --example open_timing -- FILE.rvt

use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("usage: open_timing FILE.rvt");
    macro_rules! time {
        ($what:expr, $e:expr) => {{
            let t = Instant::now();
            let n = $e;
            eprintln!("{:<32} {:>7.2}s", $what, t.elapsed().as_secs_f64());
            n
        }};
    }
    let cfb = time!("compound file", rvt_core::Cfb::open(std::path::Path::new(&path)).expect("open"));
    let parts: Vec<_> = cfb.paths().into_iter().filter(|(p, _)| p.starts_with("Partitions/")).collect();
    eprintln!("{} partitions: {:?}", parts.len(), parts.iter().map(|(p, e)| (p.clone(), e.size >> 20)).collect::<Vec<_>>());
    let raws: Vec<Vec<u8>> = time!("streams", parts.iter().map(|(_, e)| cfb.entry_bytes(e).expect("bytes")).collect());
    let raws: Vec<Vec<u8>> = time!("strip", raws.iter().map(|r| rvt_core::strip_inserted(r)).collect());
    let mut chains: Vec<Vec<rvt_core::Chain>> = time!("chains", raws.iter().map(|r| rvt_core::chains(r).expect("chains")).collect());
    let sizes: Vec<usize> = chains.iter().flatten().map(|c| c.data.len()).collect();
    eprintln!("{} chains, {} MiB, largest {} MiB", sizes.len(), sizes.iter().sum::<usize>() >> 20, sizes.iter().max().copied().unwrap_or(0) >> 20);
    time!("element tables", for ch in chains.iter_mut().flatten() { let _ = rvt_core::section_a_b(ch); });
    let partitions = time!("partitions again", rvt_core::read_partitions(&cfb).expect("partitions"));
    let doc = time!("document", rvt_read::Document::from_partitions(partitions));
    let schema = time!("schema", rvt_core::Schema::read(&cfb).expect("schema"));
    let g = time!("channel 103", rvt_core::records::file_channel(&cfb, &schema, 103).expect("103"));
    let o = time!("channel 102", rvt_core::records::file_channel(&cfb, &schema, 102).expect("102"));
    eprintln!("{} graphics ({} MiB), {} objects ({} MiB)", g.len(), g.values().map(|r| r.body.len()).sum::<usize>() >> 20, o.len(), o.values().map(|r| r.body.len()).sum::<usize>() >> 20);
    let n = time!("object roots", o.values().filter(|r| rvt_core::graph::Graph::decode_root(&schema, &r.body).is_ok()).count());
    eprintln!("{n} roots decoded");
    drop(doc);
}
