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
  rvt railings <file.rvt>              railings (path, type, height, level)
  rvt stairs <file.rvt>                component stairs (riser, tread, levels, height, runs)
  rvt stairs_runs <file.rvt>           stairs runs (width, base and top elevation)
  rvt wall_types <file.rvt>            type catalogue with layers (wall/floor/roof/ceiling types)
  rvt elements <file.rvt>              element tables per partition and chain
  rvt mesh <file.rvt> [scene.bin]      viewer meshes per category (summary; optional binary)
  rvt project_info <file.rvt>          project information with all its parameters
  rvt views <file.rvt>                 views, sheets, schedules, legends
  rvt families <file.rvt>              loaded families
  rvt types <file.rvt>                 element types with their parameters
  rvt parameters <file.rvt>            parameter definitions and bindings
  rvt params <file.rvt> <id>           parameter values of one element
  rvt object <file.rvt> <id>           one element fully decoded with the file's schema
  rvt class <file.rvt> <class>         all project elements of a schema class, fully decoded
  rvt classes <file.rvt>               schema classes in use, with element counts
  rvt schema <file.rvt>                the file's schema
  rvt check <file.rvt>                 decode every element; report what does not read to its end
  rvt dump <file.rvt> <dir>            streams, inflated chains and body index, for format analysis
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

/// Raw material for format analysis: every stream of the compound file, the inflated chains and an
/// index with each element's body location. The output is derived from the model: keep it out of git.
fn dump(path: &str, dir: &str) -> rvt_core::Result<Value> {
    let cfb = rvt_core::Cfb::open(path)?;
    std::fs::create_dir_all(dir)?;
    let safe = |s: &str| s.chars().map(|c| if c.is_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect::<String>();
    let mut streams = Vec::new();
    for (name, entry) in cfb.paths() {
        if entry.kind == 2 {
            let file = format!("stream_{}", safe(&name));
            let bytes = cfb.entry_bytes(&entry)?;
            std::fs::write(std::path::Path::new(dir).join(&file), &bytes)?;
            streams.push(json!({"name": name, "size": bytes.len(), "file": file}));
        }
    }
    let mut chains = Vec::new();
    for p in rvt_core::read_partitions(&cfb)? {
        for ch in &p.chains {
            let file = format!("chain_{}_{}.bin", safe(&p.stream), ch.index);
            std::fs::write(std::path::Path::new(dir).join(&file), &ch.data)?;
            chains.push(json!({
                "stream": p.stream, "index": ch.index, "file": file, "version": ch.version, "bits": ch.bits,
                "elements": ch.elements.iter().map(|e| json!({
                    "id": e.id, "cat": e.cat, "tag": e.tag, "view": e.view, "owner32": e.owner32, "o": e.body.0, "l": e.body.1,
                })).collect::<Vec<_>>(),
            }));
        }
    }
    let index = json!({"streams": streams, "chains": chains});
    std::fs::write(std::path::Path::new(dir).join("index.json"), serde_json::to_string(&index).expect("json"))?;
    Ok(json!({"dir": dir}))
}

fn run(args: &[String]) -> Result<Value, String> {
    let cmd = args.first().map(String::as_str).ok_or(USAGE)?;
    let path = args.get(1).ok_or(USAGE)?;
    let err = |e: rvt_core::Error| format!("{path}: {e}");
    let to = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
    match cmd {
        "elements" => elements(path).map_err(err),
        "dump" => dump(path, args.get(2).ok_or(USAGE)?).map_err(err),
        "project_info" => to(serde_json::to_value(rvt_read::Document::open(path).map_err(err)?.project_info())),
        "views" => to(serde_json::to_value(rvt_read::Document::open(path).map_err(err)?.views())),
        "families" => to(serde_json::to_value(rvt_read::Document::open(path).map_err(err)?.families())),
        "types" => to(serde_json::to_value(rvt_read::Document::open(path).map_err(err)?.element_types())),
        "parameters" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let p = doc.parameters();
            Ok(json!({"definitions": p.definitions, "bindings": p.bindings}))
        }
        "params" => {
            let id: u64 = args.get(2).and_then(|s| s.parse().ok()).ok_or(USAGE)?;
            to(serde_json::to_value(rvt_read::Document::open(path).map_err(err)?.element_parameters(id)))
        }
        "object" => {
            let id: u64 = args.get(2).and_then(|s| s.parse().ok()).ok_or(USAGE)?;
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let d = doc.decoded(id).ok_or("no such element, or no schema")?.map_err(err)?;
            Ok(rvt_read::object_json(&d))
        }
        "class" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            doc.class_objects(args.get(2).ok_or(USAGE)?).map_err(err)
        }
        "classes" => to(serde_json::to_value(rvt_read::Document::open(path).map_err(err)?.class_counts())),
        "schema" => Ok(rvt_read::Document::open(path).map_err(err)?.schema_json()),
        "check" => Ok(rvt_read::Document::open(path).map_err(err)?.decode_check()),
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
        "mesh" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let t = std::time::Instant::now();
            let scene = rvt_export::mesh::scene(&doc);
            let ms = t.elapsed().as_millis();
            if let Some(bin) = args.get(2) {
                std::fs::write(bin, rvt_export::mesh::encode(&scene)).map_err(|e| e.to_string())?;
            }
            let batches: Vec<Value> = scene
                .batches
                .iter()
                .map(|b| {
                    // per-batch bounds (view coordinates: metres, Y up)
                    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
                    for p in b.positions.chunks_exact(3) {
                        for k in 0..3 {
                            lo[k] = lo[k].min(p[k]);
                            hi[k] = hi[k].max(p[k]);
                        }
                    }
                    json!({"name": b.name, "vertices": b.positions.len() / 3, "triangles": b.indices.len() / 3, "min": lo, "max": hi})
                })
                .collect();
            Ok(json!({"batches": batches, "bounds_min": scene.bounds_min, "bounds_max": scene.bounds_max, "mesh_ms": ms}))
        }
        "segments" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.segments_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "segments": v})).collect();
            Ok(json!({"chains": chains, "systems": doc.systems()}))
        }
        "stairs" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.stairs_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "stairs": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "stairs_runs" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.stairs_runs_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "stairs_runs": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "railings" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.railings_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "railings": v})).collect();
            Ok(json!({"chains": chains}))
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
