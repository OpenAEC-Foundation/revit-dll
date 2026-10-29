# Architecture and port plan

## Goal

One Rust library that opens a Revit `.rvt` file without Revit and

1. lets a caller **query every object**: elements by id, by category or by kind, with their
   properties: walls, floors, roofs, ceilings, columns, beams, family instances, MEP
   elements, rooms, levels, grids, materials, types, families and their geometry, views,
   sheets, schedules, annotation, settings;
2. **converts** the model to open formats: IFC, STEP and DXF, plus 3D DWG (see "Open
   decisions");
3. is usable from Rust, from C/C# and other languages through `rvt.dll`, and from the command line.

## Reference implementation and verification

The rules come from the clean-room project (`rvt-cleanroom`, private). Its Python reader
`rvtread.py` is the reference, and every rule there has a notebook step number. The Rust code
cites the same step numbers.

A port counts as done only when the Rust output matches the reference export:

- `tools/parity_elements.py` compares element tables (container layer); `tools/parity_lists.py` compares one project list (levels, walls, …).
- Each further reader gets its own parity check against the same reference JSON.
- Test files are never committed (see README). The checks run locally against the private
  sample set.

| Layer | Status | Parity |
|---|---|---|
| Compound file, partitions, chains, section A/B, version tags | **done** | 100/100 rvt100 + 7 samples 2014/2015, ~1.12 M elements, 0 mismatches |
| Levels (§18) | **done** | 111 files (rvt100 + 11 samples 2014/2015), 171 levels, 0 mismatches |
| Walls (§29: line/arc location, type, levels, height, stacked, curtain panels, in-place, phases) | **done** | 111 files, 1416 walls, 0 mismatches |
| Type catalogue with compound structures (§25: wall/floor/roof/ceiling types, layers, materials, core) | **done** | 111 files, 1487 types, 0 mismatches |
| Curve tables (§7: lines, arcs, ellipses, full circles; 64-bit and 32-bit) | **done** | via floors/ceilings/roofs |
| Floors, ceilings (§31) and roofs (§32: footprint, extrusion, edge slopes) | **done** | 111 files: 161 floors, 395 ceilings, 31 roofs, 0 mismatches |
| Family instances (§13 placement, §16 type, levels, schedule/reference level, work-plane host, connectors, MEP system; 2014 document chain merge) | **done** (fam_values, fam_flags still open) | 111 files, 41 008 instances, 0 mismatches |
| MEP systems (§15.5) and straight segments (§15.1: ducts, pipes, trays, conduits; size, round, type, system, level) | **done** | 111 files: 2182 segments; systems 103/103 files equal |
| Railings (§35: path, type, host, placement offset, level, height, top rail height/type; `top_rail` sub-record still open) | **done** (core) | 112 files, 246 railings, 0 mismatches |
| Stairs runs (step 722: width, base/top elevation) | **done** | 111 files, 56 runs, 0 mismatches |
| Stairs (§33: riser, tread, base/top level and offset, height, risers, base z, runs, run geometry: sketch lines + cached walk path incl. helix; 2014–2017 layout; `type` still open) | **done** (core + run geometry) | 111 files, 39 stairs, 0 mismatches |
| Generic objects (par C): every element as its schema class with typed fields (channel 102 + header 101, `Formats/Latest`; extensible storage via the ES schemas in `Global/Latest`) | **done** | 117 files 2020–2026 + 2014–2021 sample: 101/102/103 100 % decoded (1 empty record per file excepted); levels 332/332, walls 1622/1622, materials 763/763 against typed readers / reference (see `docs/par-generic-objects.md`) |
| Schedules (`rvt_export::schedules`, open-research): definition of every ViewSchedule (fields, filters, sort/group, totals) through the generic objects; rows computed from the category's elements, stored parameters (instance, type, family type `m_pParams`) and built-ins (wall length/area/volume, levels, views, sheets, revisions) | **started** | rvt100 045 'RE Wandstaat' as built (2 rows); snowdon arch 2025: 43 schedules, 90 % of visible cells filled; Revit check of rows and filter operators open |
| Other semantic readers (≈ 200 lists of the reference export) | not started (raw fields reachable through the generic objects) | — |
| Query API: `Document::open`, `info`, `element(id)`, `elements()`, `elements_in_category`, `levels`, `walls`; `object(id)`, `objects_of_class`, `classes`, `object_coverage` (par C) | **started** | CLI `rvt info/element/category/levels/object/objects/classes/coverage` |
| Exporters | not started | — |
| C ABI | **started**: `rvt_open`, `rvt_query` (info, object, classes, objects), `rvt_free`, `rvt_close` | unit tests |

## Crates

| Crate | Role |
|---|---|
| `rvt-core` | MS-CFB reader, partition streams, gzip members, chains, element tables, `vtag` (version tags) |
| `rvt-model` | Neutral, serialisable model: one struct per object kind, plus a generic `Element` view (id, category, class, properties) so that every object is reachable even before it has a typed struct |
| `rvt-read` | Ports of the reference readers: element bodies to `rvt-model` |
| `rvt-export` | Writers: IFC 4 (STEP physical file), STEP AP242 (faceted B-rep), DXF (3D faces, polyface meshes) |
| `rvt-ffi` | `rvt.dll`: `rvt_open`, `rvt_query` (JSON in/out), `rvt_element`, `rvt_export`, `rvt_free`, `rvt_close` |
| `rvt-cli` | `rvt` command: `elements`, later `query`, `export` |

## Query API (target)

```rust
let doc = rvt::Document::open("model.rvt")?;
doc.info();                         // version, project information, units
doc.element(id)?;                   // any element: category, class, properties, geometry if any
doc.elements().category(Category::Walls);
doc.walls(); doc.floors(); doc.materials(); doc.levels(); doc.views(); doc.sheets(); ...
doc.families(); doc.family(id)?.types(); doc.family(id)?.geometry();
doc.export_ifc("model.ifc")?;
```

The C ABI exposes the same queries as JSON strings, so that C#, Python and DynLex can use them
without bindings for every struct.

## Port order

1. Container layer (done).
2. Project context: levels, phases, worksets, project information, units, categories and
   subcategories, materials (+ assets, patterns).
3. 3D elements, in the clean-room's 3D-first order: walls (+ layers, openings), floors, roofs,
   ceilings, columns, beams, family instances (placement + type + host + level), MEP curves and
   fittings, stairs/railings, curtain walls, topography, masses.
4. Family documents: parameters, types, shapes (extrusion, sweep, blend, revolve, swept blend,
   void), nested instances. This gives instance geometry.
5. Exporters on top of 2–4: IFC first (extrusions native), then STEP and DXF (tessellation).
6. 2D: views, sheets, viewports, schedules, tags, dimensions, text, filled regions, line styles.
7. Settings and everything else in the reference export.

## Open decisions

- **DXF and DWG output — decided 2026-10-23: [`acadrust`](https://crates.io/crates/acadrust)**
  (pure Rust, reads and writes DXF ASCII/binary and DWG; MPL-2.0). MPL-2.0 is file-level
  copyleft: linking it into the proprietary `rvt.dll` is allowed; changes to acadrust's own
  files must be published under MPL-2.0. `rvt-export` uses it behind the `cad` feature, so
  there is no separate DXF writer of our own.
- **Licence of this crate.** Proprietary for now (`publish = false`).

## Porting rules (learned from parity checks)

- Totals of floats use `util::py_sum` (CPython 3.12+ `sum()` is Neumaier-compensated); a plain
  fold differs in the last bit.
- Strings `u32 n | UTF-16` decode strictly (invalid surrogates = no string), and `is_printable`
  approximates Python's `str.isprintable`.
- Byte searches follow Python semantics exactly: `rfind(x, 0, o)` means the match must end before
  `o`; regex `finditer` matches do not overlap.
- Sets of ids used across partitions (level types, wall types, phases, owners) are built once in
  `Document::from_partitions`, as the reference export builds its globals.
