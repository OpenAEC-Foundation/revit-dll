use std::process::ExitCode;

use serde_json::{json, Value};

const USAGE: &str = "usage:
  rvt info <file.rvt>                  document facts
  rvt element <file.rvt> <id>          one element
  rvt category <file.rvt> <category>   elements of a built-in category (e.g. -2000011 = walls)
  rvt levels <file.rvt>                levels per project chain
  rvt walls <file.rvt>                 walls per project chain
  rvt floors|ceilings|roofs <file.rvt> slabs per project chain, with sketch curves
  rvt instances <file.rvt>             family instances (placement, type, levels, connectors)
  rvt segments <file.rvt>              MEP segments (ducts, pipes, trays, conduits) and systems
  rvt wall_types <file.rvt>            type catalogue with layers (wall/floor/roof/ceiling types)
  rvt elements <file.rvt>              element tables per partition and chain
Each command prints JSON; add `-o out.json` to write a file instead.";

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

fn run(args: &[String]) -> Result<Value, String> {
    let cmd = args.first().map(String::as_str).ok_or(USAGE)?;
    let path = args.get(1).ok_or(USAGE)?;
    let err = |e: rvt_core::Error| format!("{path}: {e}");
    let to = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
    match cmd {
        "elements" => elements(path).map_err(err),
        "info" => to(serde_json::to_value(rvt_read::Document::open(path).map_err(err)?.info())),
        "element" => {
            let id: u64 = args.get(2).and_then(|s| s.parse().ok()).ok_or(USAGE)?;
            to(serde_json::to_value(rvt_read::Document::open(path).map_err(err)?.element(id)))
        }
        "category" => {
            let cat: i32 = args.get(2).and_then(|s| s.parse().ok()).ok_or(USAGE)?;
            let doc = rvt_read::Document::open(path).map_err(err)?;
            to(serde_json::to_value(doc.elements_in_category(cat).collect::<Vec<_>>()))
        }
        "levels" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> = doc
                .levels_by_chain()
                .into_iter()
                .map(|(s, i, l)| json!({"stream": s, "index": i, "levels": l}))
                .collect();
            Ok(json!({"chains": chains}))
        }
        "wall_types" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            Ok(json!({"wall_types": doc.compound_types()}))
        }
        "segments" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.segments_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "segments": v})).collect();
            Ok(json!({"chains": chains, "systems": doc.systems()}))
        }
        "instances" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.instances_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "instances": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "floors" | "ceilings" | "roofs" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> = match cmd {
                "floors" => doc.floors_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, cmd: v})).collect(),
                "ceilings" => doc.ceilings_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, cmd: v})).collect(),
                _ => doc.roofs_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, cmd: v})).collect(),
            };
            Ok(json!({"chains": chains}))
        }
        "walls" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> = doc
                .walls_by_chain()
                .into_iter()
                .map(|(s, i, w)| json!({"stream": s, "index": i, "walls": w}))
                .collect();
            Ok(json!({"chains": chains}))
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.iter().position(|a| a == "-o").map(|i| {
        let o = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
        o
    });
    match run(&args) {
        Ok(v) => {
            let s = serde_json::to_string(&v).expect("json");
            match out.flatten() {
                Some(o) => match std::fs::write(&o, s) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("{o}: {e}");
                        ExitCode::FAILURE
                    }
                },
                None => {
                    println!("{s}");
                    ExitCode::SUCCESS
                }
            }
        }
        Err(m) => {
            eprintln!("{m}");
            ExitCode::from(2)
        }
    }
}
