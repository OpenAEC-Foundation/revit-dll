// Digest of every decoded partition record (branch open-research, par C): one line per record,
// `channel id ok|err digest`, to check that a decoder change leaves records that decoded before
// unchanged. Usage: graph_digest <file.rvt> [out.tsv]. An empty vector prints like a null pointer,
// so that a vector field once read as a single pointer (modifier 0x54, par C) compares equal when
// it is empty.
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    // par C: with RVT_DIGEST_ES=1 extensible-storage entities are decoded with the file's ES schemas
    let es = std::env::var_os("RVT_DIGEST_ES").map(|_| rvt_read::objects::es_catalog(&cfb, &s)).unwrap_or_default();
    let mut out = String::new();
    for ch in [101u64, 102, 103] {
        let Ok(recs) = rvt_core::records::file_channel(&cfb, &s, ch) else { continue };
        let mut ids: Vec<u64> = recs.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let (ok, text) = match rvt_core::graph::Graph::decode_es(&s, &recs[&id].body, &es) {
                Ok(g) => ("ok", format!("{:?}", g.objects).replace("List([])", "Ptr(None)")),
                Err(e) => ("err", e.to_string()),
            };
            let mut h = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut h);
            let _ = writeln!(out, "{ch}\t{id}\t{ok}\t{:016x}", h.finish());
        }
    }
    match a.get(2) {
        Some(p) => std::fs::write(p, out).expect("write"),
        None => print!("{out}"),
    }
}
