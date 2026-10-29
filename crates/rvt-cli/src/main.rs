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
  rvt legacy_stairs <file.rvt>         old sketch-based stairs (levels, type, sketch lines)
  rvt topography <file.rvt>            topography surfaces (terrain points)
  rvt type_values <file.rvt>           type values (value sets) per symbol
  rvt wall_sweeps <file.rvt>           wall sweeps and reveals (type, distance, host, path)
  rvt slab_edges <file.rvt>            slab edges (type, host floor, offsets, path segments)
  rvt rails <file.rvt>                 top rails and handrails (type, host railing, path)
  rvt flex|mep_curves <file.rvt>       flex ducts; pipes, trays, conduits, flex pipes
  rvt columns <file.rvt>               columns: level block and height
  rvt inplace_members <file.rvt>       content of in-place families
  rvt wall_attachments <file.rvt>      walls attached to roofs/floors/ceilings
  rvt building_pads <file.rvt>         building pads (level, offset, outline, cut surface)
  rvt group_definition_members <file>  hidden group type definition copies
  rvt stairs_runs <file.rvt>           stairs runs (width, base and top elevation)
  rvt families <file.rvt>              family document chains and symbol -> family
  rvt solids <file.rvt>                extrusion solids per family chain
  rvt wall_types <file.rvt>            type catalogue with layers (wall/floor/roof/ceiling types)
  rvt elements <file.rvt>              element tables per partition and chain
  rvt mesh <file.rvt> [scene.bin]      viewer meshes per category (summary; optional binary)
  rvt object <file.rvt> <id>           any element as its schema class with typed fields
  rvt classes <file.rvt>               schema class -> number of elements
  rvt objects <file.rvt> --class <name> [--limit n]
                                       elements of a class and its subclasses, as objects
  rvt coverage <file.rvt>              decoded records per channel, top failing classes
  rvt views <file.rvt> [--all] [--json] model views, one line each (id, kind, name, camera,
                                       section box, level, cut); --all: templates, sheets, ...
  rvt schedules <file.rvt> [--json]    schedules: fields, filters, sorting and computed rows
Each command except `views` and `schedules` prints JSON; add `-o out.json` to write a file instead, `--pretty` to indent.";

/// The value after a `--name` option.
fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
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
        "mesh" => {
            // RVT_FROM_BYTES=1: open from memory, as the viewer does over IPC
            let doc = if std::env::var_os("RVT_FROM_BYTES").is_some() {
                rvt_read::Document::from_bytes(std::fs::read(path).map_err(|e| e.to_string())?).map_err(err)?
            } else {
                rvt_read::Document::open(path).map_err(err)?
            };
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
        "group_definition_members" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> = doc
                .group_definition_members_by_chain()
                .into_iter()
                .map(|(s, i, v)| json!({"stream": s, "index": i, "group_definition_members": v}))
                .collect();
            Ok(json!({"chains": chains}))
        }
        "building_pads" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.building_pads_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "building_pads": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "wall_attachments" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.wall_attachments_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "wall_attachments": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "inplace_members" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.inplace_members_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "inplace_members": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "columns" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.columns_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "columns": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "flex" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> = doc.flex_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "flex": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "mep_curves" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.mep_curves_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "mep_curves": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "stair_run_types" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let v: Vec<Value> = doc.stairs().iter().map(|s| json!({"stair": s.id, "run_type": doc.stair_run_type(s.id)})).collect();
            Ok(json!({"stair_run_types": v}))
        }
        "wall_sweeps" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            Ok(json!({"wall_sweeps": doc.wall_sweeps()}))
        }
        "slab_edges" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            Ok(json!({"slab_edges": doc.slab_edges()}))
        }
        "rails" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            Ok(json!({"rails": doc.rails()}))
        }
        "type_values" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> = doc
                .type_values_by_chain()
                .into_iter()
                .map(|(s, i, v)| {
                    let m: serde_json::Map<String, Value> = v
                        .into_iter()
                        .map(|(k, vs)| (k.to_string(), json!({"type": vs.type_name, "rows": vs.rows.iter().map(|r| json!({"param": r.param, "num": r.num, "elem": r.elem, "text": r.text})).collect::<Vec<_>>()})))
                        .collect();
                    json!({"stream": s, "index": i, "type_values": m})
                })
                .collect();
            Ok(json!({"chains": chains}))
        }
        "topography" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.topography_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "topography": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "legacy_stairs" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.legacy_stairs_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "legacy_stairs": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "stairs" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.stairs_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "stairs": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "blends" | "revolves" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> = doc
                .family_blends_revolves_by_chain()
                .into_iter()
                .map(|(s, i, b, r)| if cmd == "blends" { json!({"stream": s, "index": i, "blends": b}) } else { json!({"stream": s, "index": i, "revolves": r}) })
                .collect();
            Ok(json!({"chains": chains}))
        }
        "sweeps" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.family_sweeps_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "sweeps": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "hosted" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.hosted_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "hosted": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "solids" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let chains: Vec<Value> =
                doc.family_solids_by_chain().into_iter().map(|(s, i, v)| json!({"stream": s, "index": i, "solids": v})).collect();
            Ok(json!({"chains": chains}))
        }
        "families" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let fc: serde_json::Map<String, Value> =
                doc.family_chains().into_iter().map(|(f, (s, i))| (f.to_string(), json!([s, i]))).collect();
            let sf: serde_json::Map<String, Value> = doc.symbol_family().into_iter().map(|(s, f)| (s.to_string(), json!(f))).collect();
            Ok(json!({"family_chains": fc, "symbol_family": sf}))
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
        "object" => {
            let id: u64 = args.get(2).and_then(|s| s.parse().ok()).ok_or(USAGE)?;
            let doc = rvt_read::Document::open(path).map_err(err)?;
            match doc.object(id) {
                Some(o) => to(serde_json::to_value(o)),
                None if doc.object_store().is_none() => Err(format!("{path}: no object records (schema or partition records not readable)")),
                None => Err(format!("{path}: no object record for element {id}")),
            }
        }
        "classes" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            if doc.object_store().is_none() {
                return Err(format!("{path}: no object records (schema or partition records not readable)"));
            }
            Ok(json!({"classes": doc.classes()}))
        }
        "objects" => {
            let class = opt(args, "--class").ok_or(USAGE)?;
            let limit: usize = match opt(args, "--limit") {
                Some(n) => n.parse().map_err(|_| USAGE.to_string())?,
                None => usize::MAX,
            };
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let ids = doc.object_ids_of_class(class);
            let objects: Vec<_> = ids.iter().take(limit).filter_map(|&id| doc.object(id)).collect();
            Ok(json!({"class": class, "total": ids.len(), "objects": objects}))
        }
        "views" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            let all = args.iter().any(|a| a == "--all");
            let list = if all { rvt_export::views::all_views(&doc) } else { rvt_export::views::views(&doc) };
            to(serde_json::to_value(json!({"views": list})))
        }
        "schedules" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            to(serde_json::to_value(json!({"schedules": rvt_export::schedules::schedules(&doc)})))
        }
        "coverage" => {
            let doc = rvt_read::Document::open(path).map_err(err)?;
            to(serde_json::to_value(doc.object_coverage()))
        }
        _ => Err(USAGE.into()),
    }
}

/// Three numbers, short.
fn v3(v: [f64; 3]) -> String {
    format!("({:.3}, {:.3}, {:.3})", v[0], v[1], v[2])
}

/// `rvt views`: one line per view (feet, model coordinates).
fn views_text(doc: &rvt_read::Document, all: bool) -> String {
    let list = if all { rvt_export::views::all_views(doc) } else { rvt_export::views::views(doc) };
    let mut out = String::new();
    for v in &list {
        let mut line = format!("{:>9}  {:<14} {:?}", v.id, v.kind.name(), v.name);
        if v.is_template {
            line += "  template";
        }
        if v.owned {
            line += "  owned";
        }
        if let Some(c) = &v.camera {
            line += &format!("  eye {} fwd {} up {} {}", v3(c.eye), v3(c.forward), v3(c.up), if c.perspective { "persp" } else { "ortho" });
            if let Some([h, w]) = c.fov {
                line += &format!(" fov {:.1}x{:.1}deg", h.to_degrees(), w.to_degrees());
            }
        }
        if let Some(b) = &v.section_box {
            let (lo, hi) = b.world_bounds();
            line += &format!("  box {}..{}", v3(lo), v3(hi));
        }
        if let Some((id, z)) = v.level {
            line += &format!("  level {id} {:?} z {z:.3}", v.level_name.as_deref().unwrap_or(""));
        }
        if let Some(c) = v.cut_offset {
            line += &format!(" cut +{c:.3}");
        }
        if let Some(s) = v.scale {
            line += &format!("  1:{s}");
        }
        out += &line;
        out.push('\n');
    }
    out += &format!("{} views\n", list.len());
    out
}

/// `rvt schedules`: each schedule as a text table (visible columns), its filters and sorting.
fn schedules_text(doc: &rvt_read::Document) -> String {
    use rvt_export::schedules::{FieldSource, ScheduleKind};
    let list = rvt_export::schedules::schedules(doc);
    let mut out = String::new();
    for s in &list {
        out += &format!("== {} \"{}\"  category {}", s.id, s.name, s.category);
        if s.is_template {
            out += "  template";
        }
        if s.kind != ScheduleKind::Elements {
            out += &format!("  {:?}", s.kind);
        }
        if !s.itemize {
            out += "  not itemized";
        }
        out.push('\n');
        let heading = |f: i64| s.columns.iter().find(|c| c.field == f).map_or(format!("#{f}"), |c| c.heading.clone());
        for f in &s.filters {
            out += &format!("   filter: {} {:?} {:?}\n", heading(f.field), f.op, f.value);
        }
        for k in &s.sort {
            out += &format!("   sort: {}{}\n", heading(k.field), if k.descending { " descending" } else { "" });
        }
        let cols: Vec<usize> = (0..s.columns.len()).filter(|&i| !s.columns[i].hidden).collect();
        let mut table: Vec<Vec<String>> = vec![cols
            .iter()
            .map(|&i| {
                let c = &s.columns[i];
                match c.source {
                    FieldSource::Instance | FieldSource::Count => c.heading.clone(),
                    src => format!("{} ({src:?})", c.heading),
                }
            })
            .collect()];
        for r in &s.rows {
            table.push(cols.iter().map(|&i| r.cells[i].clone().unwrap_or_else(|| "?".into())).collect());
        }
        let widths: Vec<usize> = (0..cols.len()).map(|j| table.iter().map(|r| r[j].chars().count()).max().unwrap_or(0).min(40)).collect();
        for r in &table {
            let cells: Vec<String> = r.iter().zip(&widths).map(|(c, &w)| format!("{:<w$}", c.chars().take(40).collect::<String>())).collect();
            out += &format!("   | {} |\n", cells.join(" | "));
        }
        if s.computed {
            out += &format!("   {} rows, {} cells without a value\n", s.rows.len(), s.unresolved);
        } else {
            out += "   rows not computed\n";
        }
    }
    out += &format!("{} schedules\n", list.len());
    out
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("schedules") && !args.iter().any(|a| a == "--json") {
        let Some(path) = args.get(1) else {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        };
        return match rvt_read::Document::open(path) {
            Ok(doc) => {
                print!("{}", schedules_text(&doc));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{path}: {e}");
                ExitCode::from(2)
            }
        };
    }
    if args.first().map(String::as_str) == Some("views") && !args.iter().any(|a| a == "--json") {
        let Some(path) = args.get(1) else {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        };
        // RVT_FROM_BYTES=1: open from memory, as the viewer does
        let doc = if std::env::var_os("RVT_FROM_BYTES").is_some() {
            std::fs::read(path).map_err(rvt_core::Error::from).and_then(rvt_read::Document::from_bytes)
        } else {
            rvt_read::Document::open(path)
        };
        return match doc {
            Ok(doc) => {
                print!("{}", views_text(&doc, args.iter().any(|a| a == "--all")));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{path}: {e}");
                ExitCode::from(2)
            }
        };
    }
    args.retain(|a| a != "--json");
    let pretty = args.iter().any(|a| a == "--pretty");
    args.retain(|a| a != "--pretty");
    let out = args.iter().position(|a| a == "-o").map(|i| {
        let o = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
        o
    });
    match run(&args) {
        Ok(v) => {
            let s = if pretty { serde_json::to_string_pretty(&v) } else { serde_json::to_string(&v) }.expect("json");
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
