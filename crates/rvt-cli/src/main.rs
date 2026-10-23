use std::process::ExitCode;

use serde_json::{json, Value};

fn usage() -> ExitCode {
    eprintln!("usage: rvt elements <file.rvt> [out.json]");
    ExitCode::from(2)
}

/// Element tables per partition and chain, in the shape of the reference export
/// (`partitions[].chains[].elements[] = {id, cat, tag, bbox}`).
fn elements(path: &str) -> rvt_core::Result<Value> {
    let cfb = rvt_core::Cfb::open(path)?;
    let parts = rvt_core::read_partitions(&cfb)?;
    let mut out = Vec::new();
    for p in parts {
        let chains: Vec<Value> = p
            .chains
            .iter()
            .map(|ch| {
                json!({
                    "index": ch.index,
                    "key": ch.key.map(|k| k.iter().map(|b| format!("{b:02x}")).collect::<String>()),
                    "version": ch.version,
                    "bits": ch.bits,
                    "elements": ch.elements.iter().map(|e| json!({
                        "id": e.id, "cat": e.cat, "tag": e.tag, "bbox": e.bbox,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        out.push(json!({"stream": p.stream, "chains": chains}));
    }
    Ok(json!({"partitions": out}))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("elements") if args.len() >= 3 => match elements(&args[2]) {
            Ok(v) => {
                let s = serde_json::to_string(&v).expect("json");
                match args.get(3) {
                    Some(o) => std::fs::write(o, s).map(|_| ExitCode::SUCCESS).unwrap_or_else(|e| {
                        eprintln!("{e}");
                        ExitCode::FAILURE
                    }),
                    None => {
                        println!("{s}");
                        ExitCode::SUCCESS
                    }
                }
            }
            Err(e) => {
                eprintln!("{path}: {e}", path = args[2]);
                ExitCode::FAILURE
            }
        },
        _ => usage(),
    }
}
