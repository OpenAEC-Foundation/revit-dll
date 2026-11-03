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
| File schema and generic decoding of every element and `Global/*` object (see "The file describes itself") | **done** | Formats 2009, 2012, 2019–2027 including workshared centrals, see "Tested files and limits" |
| Project info, views, sheets with viewports, schedule fields, families with types and values, element types, parameter definitions, bindings and values, worksets, element summary | **done** | Against the live oracle, `tools/parity_oracle.py`: 2025 models and families, and 2019 and 2023 files upgraded in memory |
| Other semantic readers (≈ 200 lists of the reference export) | reachable through `rvt class`, no typed struct yet | — |
| Query API: `Document::open`, `info`, `element(id)`, `decoded(id)`, `elements()`, `elements_in_category`, `levels`, `walls`, `views`, `families`, `parameters`, … | **started** | CLI, see `rvt` usage |
| Exporters | not started | — |
| C ABI | stub | — |

## The file describes itself

Stream `Formats/Latest` lists every class of the file with its parent and fields
(`rvt-core/src/schema.rs` has the layout). Class ids are serial from 12 in order of first
appearance, and the tags in the element tables are those ids. So tag -> class holds per file and per
version, and no version table is needed for it.

A field type with bit 1 set in its extra bytes is not saved and does not occur in bodies (seen from
2027). Revit 2012 and earlier write classes without a GUID list. Where the version code of a chain is
not known, the element id width in the schema decides the table layout, and the byte-level readers
leave that chain alone (`Chain::bits` stays empty).

An element body is the serialised object (`rvt-core/src/decode.rs`): 18-byte header (14 with 32-bit
ids), fields in schema order from the root class down, pointer targets after the object in first
in, first out order, then a 4-byte trailer. `Global/Latest` and other `Global/*` streams hold objects
in the same form, behind `u16 class`. Extensible storage is the one case outside the schema:
`ESEntity.m_blob` is `u32 marker | guid`, and its blob follows the storage schema with that GUID,
kept in the document root's `ESSchemaStorage`.

New readers should use the decoded fields (`Document::decoded`, `Node`), not byte patterns. The
older readers in `rvt-read` still search bytes and could move over. Use `rvt classes`, `rvt schema`,
`rvt object` and `rvt class` to explore; `rvt check` decodes everything in a file.

`Global/ElemTable` lists the live elements with their partition (workset) and save counters. Chain 0
of a partition can hold records that are no longer live, and an element can have a copy in several
partitions; so far the highest partition number held the newest copy (only 2 cases could tell).
`Global/PartitionTable` lists the worksets, `Global/ContentDocuments` holds per embedded document a
marker, `u32` size, `ADocument` and `u32` size: `m_ownerFamilyId` gives the family, the marker's key
the chain. A family's own types and values are in `Family.m_pFamilyTypes`. `RevitPreview4.0` wraps a
PNG, `PartAtom` (families) and the zip in `ProjectInformation` (projects) hold atom XML.

Chain 0 of a partition is the project document; other chains are embedded family documents. Chain 0
also holds content of loaded families (`m_famId` set) and of group types (`m_unplacedOwnerId` set),
and sub-types with `m_masterId` set. The public API lists none of these.

Stored parameter values are in the element's four `m_pParamValueSet*` tables, keyed by parameter id:
negative for built-in parameters, otherwise the id of a parameter element. Family types have their
own table `m_pParams` (values, formulas), which also gives the values an instance does not override.
Most built-in parameters the API shows are computed by Revit and not stored as such.

## Family intelligence

`Document::family_definition` (`rvt family`, `rvt-read/src/family.rs`) gives per family:

- parameters and values per type: `Family.m_familyParams`, `m_pFamilyTypes`; formula trees in
  `NamedParam.m_oExpression`;
- forms: classes derived from `GenSweep`, `m_cutting` = void; loops in `ExtrusionElemExtrusionHelper`
  or through `SketchMembership.m_groupId` of `CurveElem`s;
- parameter links: `FamilyParametrizedElemParamsCell.m_paramDrivenData` (`ParamIdPair`);
- labels: `Dimension.m_ArrSegInfo[].m_paramId`; driven values: `m_oFamDimConstrMgr.m_paramExprs`;
- nesting: `FamilyInstance` elements owned by the family;
- loaded families: `Family.m_oFamDoc` holds the key of the embedded document's chain, whose own
  `Family` element (`m_ownerFamilyId` of that document) has the definition; the types stay on the
  project's `Family` element.

## Tested files and limits

`rvt check` decodes every element. Limits known today:

- Files older than Revit 2009 keep their data in other streams (`Elements`, root `Latest`) and have
  no `Formats/Latest`: not readable. Header 4 partitions of about the same age are refused too.
- Extensible-storage blobs whose schema is not in the document stop that one element's decoding
  after its own fields (`Decoded::complete` false, `partial_unknown_storage` in `rvt check`).
- Formula codes come from recomputing 2 534 formulas of 256 families with the values stored per type;
  one formula was compared with the API text. Function 8 is unknown, and `>`/`<` could be `>=`/`<=`
  as far as the numbers tell. Constants in older files have no spec, so their unit is not known.
- `FamElemVisibility.m_flags` (detail levels, view directions) and `m_refName` of reference planes
  are passed on as stored; forms, reference planes, labels and nesting of `rvt family` were not
  compared with the API yet. Blend, sweep and swept blend profiles are only as far as sketches give
  them.
- Worksets were not compared with the oracle yet; no workshared file in the session's version was
  run. The partition choice rule above rests on few cases.
- Crop regions, and the type of elevations without `m_dbViewTypeId`, were not found. Names of some
  classes (detail lines, sketches) are labels from Revit and not stored.
- Parameter definitions before about 2021 store `m_unitType` (a number) instead of a spec id; the
  numbers are passed on as they are.
- Most built-in parameter values the API shows are computed and not stored.

## Live oracle

`tools/revit_oracle.py` opens a copy of a model in a running Revit session, exports public-API facts
to `exports/` and closes it without saving; `tools/parity_oracle.py` compares the readers with that
export. It addresses the session by port and process id and refuses a session with open documents,
unless they are named with `--beside`. Its own route runs on Revit's main thread: opening a document
from a web-server thread fails, and through the UI it crashes Revit. Files of an older format are
upgraded in memory, so the API then shows what the upgrade made of them; parity reports those
differences without failing. Repeated exports in one low-memory session have ended in a crash in
Revit itself: prefer a fresh session and enough free memory for long runs. `tools/gen_builtins.py` makes the table of built-in names, which the
file does not store.

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
