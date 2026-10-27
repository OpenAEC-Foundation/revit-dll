// First bytes of the first inflated record group per channel of a file (branch open-research):
// for working out the record layout of a version.
use std::io::Read;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    let m = rvt_core::records::Markers::from_schema(&s).expect("markers");
    let want: u64 = a.get(2).and_then(|x| x.parse().ok()).unwrap_or(101);
    let n: usize = a.get(3).and_then(|x| x.parse().ok()).unwrap_or(96);
    for (path, _) in cfb.paths().into_iter().filter(|(p, _)| p.starts_with("Partitions/")) {
        let raw = rvt_core::chain::strip_inserted(&cfb.stream(&path).expect("stream"));
        let mut p = 8usize;
        while p + 2 <= raw.len() {
            let t = u16::from_le_bytes([raw[p], raw[p + 1]]);
            p += 2;
            if t == m.segment {
                let rd = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
                let (flags, count, size, unc) = (rd(p), rd(p + 4), rd(p + 8) as usize, rd(p + 12));
                if size == 0 {
                    p += 16;
                    continue;
                }
                let ch = u64::from_le_bytes(raw[p + 16..p + 24].try_into().unwrap());
                let member = &raw[p + 24..p + 24 + size - 8];
                p += 24 + size - 8;
                if ch == want {
                    let mut g = Vec::new();
                    let _ = flate2::read::GzDecoder::new(member).read_to_end(&mut g);
                    println!("{path} flags {flags} count {count} size {size} unc {unc} inflated {}", g.len());
                    for row in g[..g.len().min(n)].chunks(16) {
                        println!("  {}", row.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
                    }
                    return;
                }
            } else if t == m.checkback {
                p += 4;
            } else if t == m.content {
                let tok = u32::from_le_bytes(raw[p..p + 4].try_into().unwrap());
                p += 4 + if tok != 0 { 18 } else { 0 };
                if u32::from_le_bytes(raw[p..p + 4].try_into().unwrap()) == u32::MAX {
                    break;
                }
                p += 4;
            } else if Some(t) == m.signature {
                let nb = u32::from_le_bytes(raw[p..p + 4].try_into().unwrap()) as usize;
                p += 4 + nb;
                let ns = u32::from_le_bytes(raw[p..p + 4].try_into().unwrap()) as usize;
                p += 4 + 2 * ns;
            } else {
                break;
            }
        }
    }
}
