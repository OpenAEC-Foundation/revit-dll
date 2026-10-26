//! Semantic readers: elements of a Revit file to the neutral model, behind a queryable [`Document`].

mod compound;
mod curve_elements;
mod curves;
mod families;
mod groups;
mod hosted;
mod instances;
mod levels;
mod mep;
mod pads;
mod params;
mod printable_table;
mod railings;
mod slabs;
mod solids;
mod stairs;
mod topography;
mod util;
mod walls;

use std::collections::{HashMap, HashSet};

use rvt_core::{read_partitions, vtag, Cfb, Chain, Element, Partition};
use rvt_model::{CompoundType, DocumentInfo, ElementInfo, Floor, Instance, LegacyStair, Level, MepSystem, Railing, RailingTypeInfo, Roof, Segment, Stair, Topography, Wall};

pub use rvt_core::{Error, Result};

/// Canonical (2026) class tags used for chain classification (reference reader line `TAG_SEG, …`).
pub(crate) const TAG_SEG: u16 = 3442;
pub(crate) const TAG_SYS: u16 = 3481;
pub(crate) const TAG_FAMILY: u16 = 1967;
pub(crate) const TAG_LEVEL: u16 = 2535;
pub(crate) const TAG_LEVEL_TYPE: u16 = 2536;
const TAG_HDR: u16 = 765;
const TAG_SYMBOL: u16 = 2022;
const TAG_WALL_RECORD: u16 = 3842;
const CAT_WALLS: i32 = -2000011;
const CAT_PHASES: i32 = -2000112;
const CAT_VIEWS: i32 = -2000279;
const CAT_PROJECT_INFO: i32 = -2003101;
const CAT_LEVELS: i32 = -2000240;
const CAT_DUCT_TERMINALS: i32 = -2008013;
const CAT_PIPE_ACCESSORIES: i32 = -2008010;

/// An opened `.rvt` file.
pub struct Document {
    partitions: Vec<Partition>,
    /// (partition index, chain index) of the project chains.
    project: Vec<(usize, usize)>,
    /// Step 2179: 32-bit document chains (chain 0 with ProjectInformation but no project content,
    /// Revit 2014); their lists are merged into the first project chain by id.
    document: Vec<(usize, usize)>,
    index: HashMap<u64, (usize, usize, usize)>,
    /// LevelType ids over all partitions (step 1001).
    pub(crate) level_types: HashSet<u64>,
    /// Level id -> elevation, from chain 0 of every partition (reference `glev`, step 369).
    level_z: HashMap<i64, f64>,
    /// Wall type ids over all partitions (reference `gwt`).
    wall_types: HashSet<i64>,
    /// Phase ids over all partitions (step 1880).
    phases: HashSet<i64>,
    /// 32-bit: element -> owning family (owner32) and the families that own views (step 2163).
    owner32: HashMap<u64, i32>,
    view_owners: HashSet<i32>,
    /// Floor/roof/ceiling ids per category over chain 0 of all partitions (step 389).
    slab_cat_ids: HashMap<i32, HashSet<i64>>,
    /// Compound type id -> total thickness (step 426, reference `gtw`).
    type_width: HashMap<i64, f64>,
    /// MEP systems over chain 0 of all partitions (step 241), in file order.
    systems: Vec<MepSystem>,
    /// RailingType id -> height and top rail, over all chains (step 436, reference `grt`).
    rail_types: HashMap<i64, RailingTypeInfo>,
}

impl Document {
    /// Opens a file from memory (for example bytes received over IPC).
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let cfb = Cfb::from_bytes(data)?;
        Ok(Self::from_partitions(read_partitions(&cfb)?))
    }

    /// Elevation of a level (feet), over chain 0 of all partitions.
    pub fn level_elevation(&self, level: i64) -> Option<f64> {
        self.level_z.get(&level).copied()
    }

    /// Total thickness of a compound type (wall/floor/roof/ceiling type), in feet.
    pub fn type_thickness(&self, type_id: i64) -> Option<f64> {
        self.type_width.get(&type_id).copied()
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let cfb = Cfb::open(path)?;
        Ok(Self::from_partitions(read_partitions(&cfb)?))
    }

    pub fn from_partitions(partitions: Vec<Partition>) -> Self {
        let mut project = Vec::new();
        let mut document = Vec::new();
        let mut index = HashMap::new();
        let mut level_types = HashSet::new();
        let (mut wall_types, mut phases) = (HashSet::new(), HashSet::new());
        let (mut owner32, mut view_owners) = (HashMap::new(), HashSet::new());
        for (pi, p) in partitions.iter().enumerate() {
            for (ci, ch) in p.chains.iter().enumerate() {
                if ch.bits.is_none() {
                    continue;
                }
                if is_project(ch) {
                    project.push((pi, ci));
                } else if ch.bits == Some(32) && ch.index == 0 && ch.elements.iter().any(|e| e.cat == CAT_PROJECT_INFO) {
                    document.push((pi, ci));
                }
                let lt = vtag(ch.version, TAG_LEVEL_TYPE);
                let rec = vtag(ch.version, TAG_WALL_RECORD);
                for (ei, e) in ch.elements.iter().enumerate() {
                    index.entry(e.id).or_insert((pi, ci, ei));
                    if e.tag.is_some() && e.tag == lt {
                        level_types.insert(e.id);
                    }
                    if e.cat == CAT_WALLS && rec.map_or(true, |t| e.tag != Some(t)) {
                        wall_types.insert(e.id as i64);
                    }
                    if e.cat == CAT_PHASES && e.id > 0 {
                        phases.insert(e.id as i64);
                    }
                    if ch.bits == Some(32) {
                        let o = e.owner32.unwrap_or(-1);
                        owner32.insert(e.id, o);
                        if e.cat == CAT_VIEWS && o != -1 {
                            view_owners.insert(o);
                        }
                    }
                }
            }
        }
        let mut slab_cat_ids: HashMap<i32, HashSet<i64>> = HashMap::new();
        for p in &partitions {
            if let Some(ch) = p.chains.first().filter(|c| c.bits.is_some()) {
                for e in &ch.elements {
                    if [slabs::CAT_FLOORS, slabs::CAT_STRUCTURAL_FOUNDATION, slabs::CAT_ROOFS, slabs::CAT_CEILINGS].contains(&e.cat) {
                        slab_cat_ids.entry(e.cat).or_default().insert(e.id as i64);
                    }
                }
            }
        }
        let mut doc = Document {
            partitions,
            project,
            document,
            index,
            level_types,
            level_z: HashMap::new(),
            wall_types,
            phases,
            owner32,
            view_owners,
            slab_cat_ids,
            type_width: HashMap::new(),
            systems: Vec::new(),
            rail_types: HashMap::new(),
        };
        doc.rail_types = railings::rail_types(doc.partitions.iter().flat_map(|p| p.chains.iter()));
        let mut systems: Vec<MepSystem> = Vec::new();
        for p in &doc.partitions {
            if let Some(ch) = p.chains.first().filter(|c| c.bits.is_some()) {
                for s in mep::system_names(ch) {
                    match systems.iter_mut().find(|x| x.id == s.id) {
                        Some(x) => x.name = s.name,
                        None => systems.push(s),
                    }
                }
            }
        }
        doc.systems = systems;
        doc.type_width = doc.compound_types().into_iter().map(|t| (t.id as i64, t.width)).collect();
        let mut level_z = HashMap::new();
        for p in &doc.partitions {
            if let Some(ch) = p.chains.first().filter(|c| c.bits.is_some()) {
                for l in levels::levels(ch, &doc.level_types) {
                    level_z.insert(l.id as i64, l.elevation);
                }
            }
        }
        doc.level_z = level_z;
        doc
    }

    pub fn info(&self) -> DocumentInfo {
        let first = self.project.first().map(|&(p, c)| &self.partitions[p].chains[c]);
        DocumentInfo {
            version: first.and_then(|c| c.version),
            bits: first.and_then(|c| c.bits),
            element_count: self.index.len(),
        }
    }

    fn info_of(&self, pi: usize, ci: usize, e: &Element) -> ElementInfo {
        ElementInfo {
            id: e.id,
            category: e.cat,
            class_tag: e.tag,
            view: e.view,
            bbox: e.bbox,
            partition: self.partitions[pi].stream.clone(),
            chain: self.partitions[pi].chains[ci].index,
        }
    }

    /// Any element by id.
    pub fn element(&self, id: u64) -> Option<ElementInfo> {
        let &(pi, ci, ei) = self.index.get(&id)?;
        Some(self.info_of(pi, ci, &self.partitions[pi].chains[ci].elements[ei]))
    }

    /// Step 2260: Phase Created / Phase Demolished of any project element: the first i64 (32-bit:
    /// i32) in the header area (64-bit @96..124, 32-bit @70..96) that is a Phase, followed by a
    /// Phase or −1 (walls @102/@110, floors @104/@112; snowdon arch 2025 legend floors: created =
    /// demolished = 'Legends', not shown in {3D}).
    pub fn element_phases(&self) -> HashMap<u64, (i64, Option<i64>)> {
        let mut out = HashMap::new();
        for (_, ch) in self.project_chains() {
            let bits32 = ch.bits == Some(32);
            let (w, lo, hi) = if bits32 { (4, 70, 96) } else { (8, 96, 124) };
            let rd = |b: &[u8], o: usize| if bits32 { util::i32_at(b, o) as i64 } else { util::i64_at(b, o) };
            for e in ch.elements.iter().filter(|e| e.cat != CAT_PHASES) {
                let b = ch.body(e);
                let end = hi.min(b.len().saturating_sub(2 * w - 1));
                if let Some(o) = (lo..end).find(|&o| self.phases.contains(&rd(b, o)) && {
                    let d = rd(b, o + w);
                    d == -1 || self.phases.contains(&d)
                }) {
                    let d = rd(b, o + w);
                    out.entry(e.id).or_insert((rd(b, o), (d != -1).then_some(d)));
                }
            }
        }
        out
    }

    /// Step 2272: model/symbolic curve elements of one chain (a profile family's sketch).
    pub fn chain_curves(&self, stream: &str, index: usize) -> Vec<(u64, rvt_model::Curve)> {
        self.partitions
            .iter()
            .filter(|p| p.stream == stream)
            .flat_map(|p| p.chains.iter().filter(|c| c.index == index && c.bits.is_some()))
            .map(curve_elements::curve_elements)
            .next()
            .unwrap_or_default()
    }

    /// Step 2276: the run type of a stair: the stair record's first reference (@150..300) to a
    /// StairsType (cat −2000120, not the stair), whose first reference to a cat −2000919 record of
    /// the run-type layout gives it. 64-bit records with the type name at @247 (2025/2026): flags
    /// @243/@244/@245 = monolithic / treads / risers, f64 @143 structural depth, @159 tread
    /// thickness, @167 riser thickness, @175 nosing length (API: rac 2026 141730/141746/171672,
    /// snowdon 2025 1557288).
    pub fn stair_run_type(&self, stair: u64) -> Option<rvt_model::StairRunType> {
        let cat = |id: i64| if id > 0 { self.element(id as u64).map(|e| e.category) } else { None };
        let b = self.element_body(stair)?;
        let ty = (150..b.len().min(300).saturating_sub(8)).map(|o| util::i64_at(&b, o)).find(|&q| q != stair as i64 && cat(q) == Some(-2000120))?;
        let tb = self.element_body(ty as u64)?;
        (150..tb.len().saturating_sub(8)).map(|o| util::i64_at(&tb, o)).filter(|&q| cat(q) == Some(-2000919)).find_map(|q| {
            let r = self.element_body(q as u64)?;
            if r.len() < 251 || !(1..=200).contains(&util::u32_at(&r, 247)) || r[243] > 1 || r[244] > 1 || r[245] > 1 {
                return None;
            }
            let f = |o: usize| util::f64_at(&r, o);
            let ok = |v: f64| v.is_finite() && (0.0..20.0).contains(&v);
            (ok(f(143)) && ok(f(159)) && ok(f(167)) && ok(f(175))).then(|| rvt_model::StairRunType {
                id: q as u64,
                monolithic: r[243] == 1,
                treads: r[244] == 1,
                risers: r[245] == 1,
                structural_depth: f(143),
                tread_thickness: f(159),
                riser_thickness: f(167),
                nosing: f(175),
            })
        })
    }

    /// Raw record body of an element (for format analysis).
    pub fn element_body(&self, id: u64) -> Option<Vec<u8>> {
        let &(pi, ci, ei) = self.index.get(&id)?;
        let ch = &self.partitions[pi].chains[ci];
        Some(ch.body(&ch.elements[ei]).to_vec())
    }

    /// Every element of the file (all partitions and chains).
    pub fn elements(&self) -> impl Iterator<Item = ElementInfo> + '_ {
        self.partitions.iter().enumerate().flat_map(move |(pi, p)| {
            p.chains.iter().enumerate().flat_map(move |(ci, ch)| ch.elements.iter().map(move |e| self.info_of(pi, ci, e)))
        })
    }

    /// Elements of one built-in category (e.g. −2000011 = walls).
    pub fn elements_in_category(&self, category: i32) -> impl Iterator<Item = ElementInfo> + '_ {
        self.elements().filter(move |e| e.category == category)
    }

    pub(crate) fn project_chains(&self) -> impl Iterator<Item = (&Partition, &Chain)> + '_ {
        self.project.iter().map(|&(p, c)| (&self.partitions[p], &self.partitions[p].chains[c]))
    }

    /// Levels of all project chains.
    pub fn levels(&self) -> Vec<Level> {
        self.project_chains().flat_map(|(_, ch)| levels::levels(ch, &self.level_types)).collect()
    }

    /// Type catalogue with compound structures (wall, floor, roof, ceiling types), from chain 0 of
    /// every partition (step 426).
    pub fn compound_types(&self) -> Vec<CompoundType> {
        self.partitions
            .iter()
            .filter_map(|p| p.chains.first().filter(|c| c.bits.is_some()))
            .flat_map(compound::compound_types)
            .collect()
    }

    /// Wall instances of all project chains.
    pub fn walls(&self) -> Vec<Wall> {
        self.walls_by_chain().into_iter().flat_map(|(_, _, w)| w).collect()
    }

    /// Step 2257: the host walls of family documents (a door or window template's wall), per family
    /// chain, with the width of their compound type in the same chain.
    pub fn family_host_walls(&self) -> HashMap<(String, usize), Vec<(Wall, Option<f64>)>> {
        let ctx = walls::WallContext { level_z: &self.level_z, wall_types: &self.wall_types, phases: &self.phases };
        let proj: HashSet<(usize, usize)> = self.project.iter().chain(self.document.iter()).copied().collect();
        let mut out = HashMap::new();
        for (pi, p) in self.partitions.iter().enumerate() {
            for (ci, ch) in p.chains.iter().enumerate() {
                if ch.bits.is_none() || proj.contains(&(pi, ci)) {
                    continue;
                }
                let w = walls::walls(ch, &ctx);
                if w.is_empty() {
                    continue;
                }
                let types = compound::compound_types(ch);
                let v = w
                    .into_iter()
                    .map(|w| {
                        // step 2267: without a compound type in the chain (snowdon door family 439:
                        // type class 4399), the wall's stored box across an axis-aligned wall
                        let t = types.iter().find(|t| t.id as i64 == w.wall_type).map(|t| t.width).or_else(|| {
                            let bb = ch.elements.iter().find(|e| e.id == w.id)?.bbox?;
                            let (dx, dy) = ((w.p1[0] - w.p0[0]).abs(), (w.p1[1] - w.p0[1]).abs());
                            if dy < 1e-6 * dx.max(1.0) {
                                Some(bb[4] - bb[1])
                            } else if dx < 1e-6 * dy.max(1.0) {
                                Some(bb[3] - bb[0])
                            } else {
                                None
                            }
                        });
                        (w, t)
                    })
                    .collect();
                out.insert((p.stream.clone(), ch.index), v);
            }
        }
        out
    }

    /// Step 2285: top rails and handrails. The record references its type (same category, no
    /// box), its railing (cat −2000126) and, last, the path element whose body holds the path pieces.
    pub fn rails(&self) -> Vec<rvt_model::Rail> {
        let mut out = Vec::new();
        for cat in [-2000946, -2000947] {
            for e in self.elements_in_category(cat) {
                if e.bbox.is_none() {
                    continue; // a type
                }
                let Some(b) = self.element_body(e.id) else { continue };
                let mut seen = HashSet::new();
                let refs: Vec<u64> = (0..b.len().saturating_sub(7)).map(|o| util::i64_at(&b, o)).filter(|&q| q > 0 && q as u64 != e.id).map(|q| q as u64).filter(|q| seen.insert(*q)).collect();
                let info = |q: u64| self.element(q);
                let rail_type = refs.iter().copied().find(|q| info(*q).is_some_and(|x| x.category == cat && x.bbox.is_none()));
                let host = refs.iter().copied().find(|q| info(*q).is_some_and(|x| x.category == -2000126));
                // the last referenced element with pieces: the centre line (snowdon class 3140 after
                // the edge cache 929; rac 2026 class 3204 after the plan outline 947)
                let path = refs
                    .iter()
                    .rev()
                    .copied()
                    .filter(|q| info(*q).is_some_and(|x| x.category == -1))
                    .filter_map(|q| self.element_body(q))
                    .map(|pb| path_pieces(&pb))
                    .find(|v| !v.is_empty())
                    .unwrap_or_default();
                if path.is_empty() {
                    continue;
                }
                out.push(rvt_model::Rail { id: e.id, cat, rail_type, host, path });
            }
        }
        out
    }

    /// Step 2284: elements of a design option that is not its set's primary (API: a {3D} view
    /// shows the primary options). An option set (cat −2006112) lists its options (cat −2006114)
    /// and names the primary last (snowdon arch 2025 'Bleacher Seating': 2059967, 2059970, primary
    /// 2059970 'Middle Stair'); an element carries its option id in its header (first 300 bytes;
    /// snowdon floors 2062310.. and their slab edges in 2059967: absent from Revit's geometry).
    pub fn secondary_option_elements(&self) -> HashSet<u64> {
        let options: HashSet<u64> = self.elements_in_category(-2006114).map(|e| e.id).collect();
        let mut secondary: HashSet<u64> = HashSet::new();
        for set in self.elements_in_category(-2006112) {
            let Some(b) = self.element_body(set.id) else { continue };
            let found: Vec<u64> = (0..b.len().saturating_sub(7)).map(|o| util::i64_at(&b, o)).filter(|&q| q > 0 && options.contains(&(q as u64))).map(|q| q as u64).collect();
            let Some(&primary) = found.last() else { continue };
            secondary.extend(found.into_iter().filter(|q| *q != primary));
        }
        let mut out = HashSet::new();
        if secondary.is_empty() {
            return out;
        }
        for e in self.elements() {
            if options.contains(&e.id) || e.category == -2006112 {
                continue;
            }
            let Some(b) = self.element_body(e.id) else { continue };
            if (8..b.len().min(300).saturating_sub(7)).any(|o| {
                let q = util::i64_at(&b, o);
                q > 0 && secondary.contains(&(q as u64))
            }) {
                out.insert(e.id);
            }
        }
        out
    }

    /// Step 2284: slab edges (cat −2001392). The path is a run of 84-byte entries: f64 length, then
    /// start point and unit direction (snowdon arch 2025 629902: 7 floor edges of floor 629119,
    /// GT within 0.001 ft); the type is the first referenced slab-edge element without a box, the
    /// host the first referenced floor; the offsets are [value f64][parameter id i64] pairs.
    pub fn slab_edges(&self) -> Vec<rvt_model::SlabEdge> {
        let f = |b: &[u8], o: usize| f64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"));
        let mut out = Vec::new();
        for e in self.elements_in_category(-2001392) {
            let Some(b) = self.element_body(e.id) else { continue };
            if b.len() < 600 {
                continue; // a type
            }
            let best = path_pieces(&b);
            if best.is_empty() {
                continue;
            }
            let refs: Vec<u64> = (0..b.len().min(600).saturating_sub(8)).map(|o| util::i64_at(&b, o)).filter(|&q| q > 0 && q as u64 != e.id).map(|q| q as u64).collect();
            let is_type = |q: u64| q != e.id && self.element(q).is_some_and(|x| x.category == -2001392 && x.bbox.is_none()) && self.element_body(q).is_some_and(|tb| tb.len() < 600);
            // 2019 and older: the type as an i32 (rac_advanced 2019 217842: type 19887 @102)
            let edge_type = refs.iter().copied().find(|q| is_type(*q)).or_else(|| {
                (0..b.len().min(600).saturating_sub(4)).map(|o| i32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"))).filter(|&q| q > 0).map(|q| q as u64).find(|q| is_type(*q))
            });
            let host = refs.iter().copied().find(|q| self.element(*q).is_some_and(|x| x.category == -2000032));
            let param = |pid: i64| (8..b.len().saturating_sub(8)).find(|&o| util::i64_at(&b, o) == pid).map(|o| f(&b, o - 8)).filter(|v| v.is_finite() && v.abs() < 1e3);
            out.push(rvt_model::SlabEdge {
                id: e.id,
                edge_type,
                host,
                vertical_offset: param(-1012827).unwrap_or(0.0),
                horizontal_offset: param(-1012825).unwrap_or(0.0),
                segments: best,
            });
        }
        out
    }

    /// Step 2271: wall sweeps and reveals of every project chain (port of rvtread `wall_sweeps`).
    pub fn wall_sweeps(&self) -> Vec<rvt_model::WallSweep> {
        let is_element = |id: i64| id > 0 && self.index.contains_key(&(id as u64));
        self.project_chains().flat_map(|(_, ch)| walls::wall_sweeps(ch, &is_element)).collect()
    }

    /// Walls per project chain, keyed by (partition stream, chain index).
    pub fn walls_by_chain(&self) -> Vec<(String, usize, Vec<Wall>)> {
        let ctx = walls::WallContext { level_z: &self.level_z, wall_types: &self.wall_types, phases: &self.phases };
        self.project_chains()
            .map(|(p, ch)| {
                let mut w = walls::walls(ch, &ctx);
                if ch.bits == Some(32) {
                    w.retain(|r| !self.in_family_document(r.id));
                }
                (p.stream.clone(), ch.index, w)
            })
            .collect()
    }

    /// Step 2163 (32-bit): owned by an embedded family document (the owner owns views or is nested);
    /// such walls, floors, roofs and ceilings are not project content.
    fn in_family_document(&self, id: u64) -> bool {
        let o = self.owner32.get(&id).copied().unwrap_or(-1);
        o != -1 && (self.view_owners.contains(&o) || self.owner32.get(&(o as u64)).copied().unwrap_or(-1) != -1)
    }

    fn slab_ctx(&self) -> slabs::SlabContext<'_> {
        slabs::SlabContext { level_z: &self.level_z, type_width: &self.type_width, cat_ids: &self.slab_cat_ids }
    }

    /// Floors per project chain (§31; floors and structural foundation slabs).
    pub fn floors_by_chain(&self) -> Vec<(String, usize, Vec<Floor>)> {
        let ctx = self.slab_ctx();
        self.project_chains()
            .map(|(p, ch)| {
                let mut f = slabs::floors(ch, &ctx, &[slabs::CAT_FLOORS, slabs::CAT_STRUCTURAL_FOUNDATION], -1001951);
                if ch.bits == Some(32) {
                    f.retain(|r| !self.in_family_document(r.id));
                }
                (p.stream.clone(), ch.index, f)
            })
            .collect()
    }

    /// Ceilings per project chain (§31.6, step 393).
    pub fn ceilings_by_chain(&self) -> Vec<(String, usize, Vec<Floor>)> {
        let ctx = self.slab_ctx();
        self.project_chains()
            .map(|(p, ch)| {
                let mut f = slabs::floors(ch, &ctx, &[slabs::CAT_CEILINGS], -1002300);
                if ch.bits == Some(32) {
                    f.retain(|r| !self.in_family_document(r.id));
                }
                (p.stream.clone(), ch.index, f)
            })
            .collect()
    }

    /// Roofs per project chain (§32).
    pub fn roofs_by_chain(&self) -> Vec<(String, usize, Vec<Roof>)> {
        let ctx = self.slab_ctx();
        self.project_chains()
            .map(|(p, ch)| {
                let mut r = slabs::roofs(ch, &ctx);
                if ch.bits == Some(32) {
                    r.retain(|x| !self.in_family_document(x.id));
                }
                (p.stream.clone(), ch.index, r)
            })
            .collect()
    }

    /// Instances of one chain: placement, type, levels, work-plane levels (step 2045) and, for
    /// 32-bit files, without instances owned by a family (step 2163).
    fn chain_instances(&self, ch: &Chain, extra_symbols: &HashSet<u64>) -> Vec<Instance> {
        let mut insts = instances::instances(ch, extra_symbols);
        // step 2045 uses the level of any project record (level, base level or reference level) of this chain
        let lv: HashSet<i64> = ch.elements.iter().filter(|e| e.cat == CAT_LEVELS).map(|e| e.id as i64).collect();
        let mut host_levels: HashMap<u64, i64> = HashMap::new();
        let mut note = |id: u64, q: Option<i64>| {
            if let Some(q) = q.filter(|q| *q != 0 && lv.contains(q)) {
                host_levels.entry(id).or_insert(q);
            }
        };
        let wctx = walls::WallContext { level_z: &self.level_z, wall_types: &self.wall_types, phases: &self.phases };
        for w in walls::walls(ch, &wctx) {
            note(w.id, Some(w.base_level));
        }
        for i in &insts {
            note(i.id, i.level.filter(|v| *v != 0).or(i.reference_level));
        }
        let sctx = self.slab_ctx();
        for f in slabs::floors(ch, &sctx, &[slabs::CAT_FLOORS, slabs::CAT_STRUCTURAL_FOUNDATION], -1001951) {
            note(f.id, Some(f.level));
        }
        for f in slabs::floors(ch, &sctx, &[slabs::CAT_CEILINGS], -1002300) {
            note(f.id, Some(f.level));
        }
        for r in slabs::roofs(ch, &sctx) {
            note(r.id, Some(r.level));
        }
        instances::work_plane_levels(ch, &mut insts, &host_levels);
        // §15.5: system of duct and pipe terminals/accessories
        let sys_ids = self.system_ids();
        let byid: HashMap<u64, &Element> = ch.elements.iter().map(|e| (e.id, e)).collect();
        for r in insts.iter_mut().filter(|r| r.cat == CAT_DUCT_TERMINALS || r.cat == CAT_PIPE_ACCESSORIES) {
            r.system = Some(byid.get(&r.id).and_then(|e| mep::system_of(ch, e, &sys_ids)));
        }
        if ch.bits == Some(32) {
            insts.retain(|r| self.owner32.get(&r.id).copied().unwrap_or(-1) == -1);
        }
        insts
    }

    /// Family instances per project chain; the 2014 document chain's instances are merged into the
    /// first project chain (step 2179).
    pub fn instances_by_chain(&self) -> Vec<(String, usize, Vec<Instance>)> {
        let mut out: Vec<(String, usize, Vec<Instance>)> =
            self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, self.chain_instances(ch, &HashSet::new()))).collect();
        if let Some(first) = out.first_mut() {
            for &(pi, ci) in &self.document {
                let doc = &self.partitions[pi].chains[ci];
                // step 2191: symbols of the other 32-bit chains 0 of the file
                let syms: HashSet<u64> = self
                    .partitions
                    .iter()
                    .filter_map(|p| p.chains.first())
                    .filter(|c| c.bits == Some(32) && !std::ptr::eq(*c, doc))
                    .flat_map(|c| {
                        let t = vtag(c.version, TAG_SYMBOL);
                        c.elements.iter().filter(move |e| e.tag.is_some() && e.tag == t).map(|e| e.id)
                    })
                    .collect();
                let extra = self.chain_instances(doc, &syms);
                let have: HashSet<u64> = first.2.iter().map(|r| r.id).collect();
                if first.2.is_empty() {
                    first.2 = extra;
                } else {
                    first.2.extend(extra.into_iter().filter(|r| !have.contains(&r.id)));
                }
            }
        }
        out
    }

    /// Railings per project chain (§35); the 2014 document chain's railings are merged into the
    /// first project chain by id (step 2179).
    pub fn railings_by_chain(&self) -> Vec<(String, usize, Vec<Railing>)> {
        let mut out: Vec<(String, usize, Vec<Railing>)> =
            self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, railings::railings(ch, &self.rail_types))).collect();
        if let Some(first) = out.first_mut() {
            for &(pi, ci) in &self.document {
                let extra = railings::railings(&self.partitions[pi].chains[ci], &self.rail_types);
                let have: HashSet<u64> = first.2.iter().map(|r| r.id).collect();
                first.2.extend(extra.into_iter().filter(|r| !have.contains(&r.id)));
            }
        }
        out
    }

    /// Component stairs per project chain (§33); the 2014 document chain's stairs are merged into
    /// the first project chain by id (step 2179).
    pub fn stairs_by_chain(&self) -> Vec<(String, usize, Vec<Stair>)> {
        let mut out: Vec<(String, usize, Vec<Stair>)> =
            self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, stairs::stairs(ch, &self.level_z))).collect();
        if let Some(first) = out.first_mut() {
            for &(pi, ci) in &self.document {
                let extra = stairs::stairs(&self.partitions[pi].chains[ci], &self.level_z);
                let have: HashSet<u64> = first.2.iter().map(|r| r.id).collect();
                first.2.extend(extra.into_iter().filter(|r| !have.contains(&r.id)));
            }
        }
        out
    }

    /// Hidden group type definition copies per chain (step 2233), over every readable chain.
    pub fn group_definition_members_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::GroupDefinitionMember>)> {
        self.partitions
            .iter()
            .flat_map(|p| p.chains.iter().filter(|c| c.bits.is_some()).map(move |ch| (p.stream.clone(), ch.index, groups::group_definition_members(ch))))
            .filter(|(_, _, v)| !v.is_empty())
            .collect()
    }

    /// In-place family content per chain (step 2238), over every readable chain.
    pub fn inplace_members_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::InplaceMember>)> {
        self.partitions
            .iter()
            .flat_map(|p| p.chains.iter().filter(|c| c.bits.is_some()).map(move |ch| (p.stream.clone(), ch.index, groups::inplace_members(ch))))
            .filter(|(_, _, v)| !v.is_empty())
            .collect()
    }

    /// Ids of hidden group type definition copies (not drawn).
    pub fn group_definition_ids(&self) -> HashSet<u64> {
        self.group_definition_members_by_chain().into_iter().flat_map(|(_, _, v)| v.into_iter().map(|m| m.id)).collect()
    }

    /// Ids of in-place family content in project chains (step 2238): drawn through the family
    /// instance if at all, never as separate walls, floors or instances.
    pub fn inplace_ids(&self) -> HashSet<u64> {
        let project: HashSet<(String, usize)> = self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index)).collect();
        self.inplace_members_by_chain()
            .into_iter()
            .filter(|(s, i, _)| project.contains(&(s.clone(), *i)))
            .flat_map(|(_, _, v)| v.into_iter().map(|m| m.id))
            .collect()
    }

    /// Building pads per project chain (step 1672).
    pub fn building_pads_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::BuildingPad>)> {
        self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, pads::building_pads(ch))).collect()
    }

    pub fn building_pads(&self) -> Vec<rvt_model::BuildingPad> {
        self.building_pads_by_chain().into_iter().flat_map(|(_, _, v)| v).collect()
    }

    /// Wall attachments (top/bottom attached to a roof, floor or ceiling) per project chain (step 2237).
    pub fn wall_attachments_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::WallAttachment>)> {
        self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, walls::wall_attachments(ch))).collect()
    }

    pub fn wall_attachments(&self) -> Vec<rvt_model::WallAttachment> {
        self.wall_attachments_by_chain().into_iter().flat_map(|(_, _, v)| v).collect()
    }

    /// Columns (§30.5) per project chain: level block and height.
    pub fn columns_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::Column>)> {
        self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, instances::columns(ch, &self.level_z))).collect()
    }

    pub fn columns(&self) -> Vec<rvt_model::Column> {
        self.columns_by_chain().into_iter().flat_map(|(_, _, v)| v).collect()
    }

    /// Flex ducts per project chain (§15.6).
    pub fn flex_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::FlexCurve>)> {
        let sys = self.system_ids();
        self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, mep::flex_ducts(ch, &sys))).collect()
    }

    /// Pipes, cable trays, conduits and flex pipes per project chain (step 702).
    pub fn mep_curves_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::MepCurve>)> {
        let sys = self.system_ids();
        self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, mep::mep_curves(ch, &sys))).collect()
    }

    /// Type values (§9.2) of the symbols in each project chain: symbol id -> value set (the last
    /// set per owner, like the reference `type_values`).
    /// Step 2263: parameters of chain 0 of every partition count (snowdon arch 2025: a project over
    /// Partitions/122 and /124; family parameters in /122, symbols in /124).
    pub fn type_values_by_chain(&self) -> Vec<(String, usize, std::collections::BTreeMap<u64, rvt_model::ValueSet>)> {
        let global: HashSet<u64> = self
            .partitions
            .iter()
            .filter_map(|p| p.chains.first().filter(|c| c.bits.is_some()))
            .flat_map(|ch| params::parameter_names(ch).into_keys())
            .collect();
        self.project_chains()
            .map(|(p, ch)| {
                let mut prm: HashSet<u64> = params::parameter_names(ch).into_keys().collect();
                prm.extend(global.iter().copied());
                let sets = params::value_sets(ch, &prm, &[2022]);
                (p.stream.clone(), ch.index, sets.into_iter().map(|s| (s.owner, s)).collect())
            })
            .collect()
    }

    /// Step 2264: parameter names of chain 0 of every partition (a project can be spread over
    /// partitions, step 2263).
    pub fn parameter_names_global(&self) -> HashMap<u64, Option<String>> {
        self.partitions
            .iter()
            .filter_map(|p| p.chains.first().filter(|c| c.bits.is_some()))
            .flat_map(params::parameter_names)
            .collect()
    }

    /// Parameter names (§9.1) of a chain, by ParameterElement id.
    pub fn parameter_names_of(&self, stream: &str, index: usize) -> HashMap<u64, Option<String>> {
        self.partitions
            .iter()
            .filter(|p| p.stream == stream)
            .flat_map(|p| p.chains.iter().filter(|c| c.index == index && c.bits.is_some()))
            .map(params::parameter_names)
            .next()
            .unwrap_or_default()
    }

    /// Step 2268: parameter associations (built-in parameter → family parameter) of the elements of
    /// every family chain, e.g. a form's Visible (−1006205) bound to a yes/no family parameter.
    pub fn family_param_associations(&self) -> HashMap<u64, Vec<(i64, u64)>> {
        let proj: HashSet<(usize, usize)> = self.project.iter().chain(self.document.iter()).copied().collect();
        let mut out = HashMap::new();
        for (pi, p) in self.partitions.iter().enumerate() {
            for (ci, ch) in p.chains.iter().enumerate() {
                if ch.bits.is_none() || proj.contains(&(pi, ci)) {
                    continue;
                }
                let prm: HashSet<u64> = params::parameter_names(ch).into_keys().collect();
                out.extend(params::param_associations(ch, &prm));
            }
        }
        out
    }

    /// Step 2266: current values of a family chain (the type its forms are drawn at), if found.
    pub fn family_current_values(&self, stream: &str, index: usize) -> Option<rvt_model::ValueSet> {
        self.partitions
            .iter()
            .filter(|p| p.stream == stream)
            .flat_map(|p| p.chains.iter().filter(|c| c.index == index && c.bits.is_some()))
            .map(|ch| {
                let prm: HashSet<u64> = params::parameter_names(ch).into_keys().collect();
                params::current_values(ch, &prm)
            })
            .next()
            .and_then(|v| v.into_iter().next())
    }

    /// Value sets (§9.2) of the Family and FamilySymbol records of one chain (for a family chain:
    /// its own defaults and its nested types).
    pub fn value_sets_of(&self, stream: &str, index: usize) -> Vec<rvt_model::ValueSet> {
        self.partitions
            .iter()
            .filter(|p| p.stream == stream)
            .flat_map(|p| p.chains.iter().filter(|c| c.index == index && c.bits.is_some()))
            .map(|ch| {
                let prm: HashSet<u64> = params::parameter_names(ch).into_keys().collect();
                params::value_sets(ch, &prm, &[1967, 2022])
            })
            .next()
            .unwrap_or_default()
    }

    /// Topography surfaces per project chain (step 1532).
    pub fn topography_by_chain(&self) -> Vec<(String, usize, Vec<Topography>)> {
        self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, topography::topography(ch))).collect()
    }

    pub fn topography(&self) -> Vec<Topography> {
        self.topography_by_chain().into_iter().flat_map(|(_, _, t)| t).collect()
    }

    /// The class tag of canonical (2026) class `canonical` in this file's version.
    pub fn class_tag(&self, canonical: u16) -> Option<u16> {
        vtag(self.info().version, canonical)
    }

    /// Old sketch-based stairs per project chain (step 1455), with the chain's own levels.
    pub fn legacy_stairs_by_chain(&self) -> Vec<(String, usize, Vec<LegacyStair>)> {
        let lz = |ch: &Chain| -> HashMap<i64, f64> { levels::levels(ch, &self.level_types).into_iter().map(|l| (l.id as i64, l.elevation)).collect() };
        self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, stairs::legacy_stairs(ch, &lz(ch)))).collect()
    }

    pub fn legacy_stairs(&self) -> Vec<LegacyStair> {
        self.legacy_stairs_by_chain().into_iter().flat_map(|(_, _, s)| s).collect()
    }

    /// Stairs runs per project chain (step 722: width, base and top elevation).
    pub fn stairs_runs_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::StairsRun>)> {
        let mut out: Vec<(String, usize, Vec<rvt_model::StairsRun>)> =
            self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, stairs::stairs_runs(ch))).collect();
        if let Some(first) = out.first_mut() {
            for &(pi, ci) in &self.document {
                let extra = stairs::stairs_runs(&self.partitions[pi].chains[ci]);
                let have: HashSet<u64> = first.2.iter().map(|r| r.id).collect();
                first.2.extend(extra.into_iter().filter(|r| !have.contains(&r.id)));
            }
        }
        out
    }

    pub fn stairs_runs(&self) -> Vec<rvt_model::StairsRun> {
        self.stairs_runs_by_chain().into_iter().flat_map(|(_, _, s)| s).collect()
    }

    /// Family id -> (partition stream, chain index) of the chain holding the family's own
    /// document (step 492; reference `catalog.family_chains`).
    pub fn family_chains(&self) -> std::collections::BTreeMap<u64, (String, usize)> {
        families::family_chains(&self.partitions)
            .into_iter()
            .map(|(f, (q, xi))| (f, (self.partitions[q].stream.clone(), self.partitions[q].chains[xi].index)))
            .collect()
    }

    /// Step 2246 (§10): family id -> document chain for families nested in family documents.
    pub fn nested_family_chains(&self) -> std::collections::BTreeMap<u64, (String, usize)> {
        families::nested_family_chains(&self.partitions)
            .into_iter()
            .map(|(f, (q, xi))| (f, (self.partitions[q].stream.clone(), self.partitions[q].chains[xi].index)))
            .collect()
    }

    /// Step 2246: symbol -> family for nested families (symbols in family chains).
    pub fn nested_symbol_family(&self) -> HashMap<u64, u64> {
        families::nested_symbol_family(&self.partitions)
    }

    /// Step 2246: project instances linked to another instance (shared nested components).
    pub fn sub_component_links(&self) -> HashSet<u64> {
        self.project_chains().flat_map(|(_, ch)| instances::sub_component_links(ch)).collect()
    }

    /// Step 2246: family instances placed inside family documents (nested families), per family
    /// chain; placement in the host family's coordinates.
    pub fn family_chain_instances(&self) -> Vec<(String, usize, Vec<Instance>)> {
        let project: HashSet<(String, usize)> = self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index)).collect();
        self.partitions
            .iter()
            .flat_map(|p| p.chains.iter().skip(1).filter(|c| c.bits.is_some()).map(move |ch| (p, ch)))
            .filter(|(p, ch)| !project.contains(&(p.stream.clone(), ch.index)))
            .map(|(p, ch)| (p.stream.clone(), ch.index, instances::instances(ch, &HashSet::new())))
            .filter(|(_, _, v)| !v.is_empty())
            .collect()
    }

    /// Symbol (family type) id -> family id (step 494; reference `catalog.symbol_family`).
    pub fn symbol_family(&self) -> HashMap<u64, u64> {
        families::symbol_family(&self.partitions)
    }

    /// Extrusion solids of every family (non-project) chain, keyed by (partition stream, chain
    /// index); reference `solids` of family chains (§8.11).
    pub fn family_solids_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::ExtrusionSolid>)> {
        let proj: HashSet<(usize, usize)> = self.project.iter().chain(self.document.iter()).copied().collect();
        let mut out = Vec::new();
        for (pi, p) in self.partitions.iter().enumerate() {
            for (ci, ch) in p.chains.iter().enumerate() {
                if ch.bits.is_none() || proj.contains(&(pi, ci)) {
                    continue;
                }
                out.push((p.stream.clone(), ch.index, solids::extrusion_solids(ch)));
            }
        }
        out
    }

    /// Hosted family instances per project chain (§30); the 2014 document chain is merged by id.
    /// Work-plane hosts in linked models are not resolved yet (host = null).
    pub fn hosted_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::Hosted>)> {
        let mut out: Vec<(String, usize, Vec<rvt_model::Hosted>)> =
            self.project_chains()
                .map(|(p, ch)| {
                    let mut h = hosted::hosted(ch, &[]);
                    if ch.bits == Some(32) {
                        // step 2163: like instances, records owned by a family are not project content
                        h.retain(|r| self.owner32.get(&r.id).copied().unwrap_or(-1) == -1);
                    }
                    (p.stream.clone(), ch.index, h)
                })
                .collect();
        if let Some(first) = out.first_mut() {
            for &(pi, ci) in &self.document {
                let mut extra = hosted::hosted(&self.partitions[pi].chains[ci], &[]);
                extra.retain(|r| self.owner32.get(&r.id).copied().unwrap_or(-1) == -1);
                let have: HashSet<u64> = first.2.iter().map(|r| r.id).collect();
                first.2.extend(extra.into_iter().filter(|r| !have.contains(&r.id)));
            }
        }
        out
    }

    pub fn hosted(&self) -> Vec<rvt_model::Hosted> {
        self.hosted_by_chain().into_iter().flat_map(|(_, _, h)| h).collect()
    }

    /// Sweeps of every family (non-project) chain (§8.12; reference `sweeps`).
    pub fn family_sweeps_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::SweepSolid>)> {
        let proj: HashSet<(usize, usize)> = self.project.iter().chain(self.document.iter()).copied().collect();
        let mut out = Vec::new();
        for (pi, p) in self.partitions.iter().enumerate() {
            for (ci, ch) in p.chains.iter().enumerate() {
                if ch.bits.is_none() || proj.contains(&(pi, ci)) {
                    continue;
                }
                out.push((p.stream.clone(), ch.index, solids::sweep_solids(ch, &p.chains)));
            }
        }
        out
    }

    /// Blends and revolutions of every family (non-project) chain (§8.13/§8.14).
    pub fn family_blends_revolves_by_chain(&self) -> Vec<(String, usize, Vec<rvt_model::BlendSolid>, Vec<rvt_model::RevolveSolid>)> {
        let proj: HashSet<(usize, usize)> = self.project.iter().chain(self.document.iter()).copied().collect();
        let mut out = Vec::new();
        for (pi, p) in self.partitions.iter().enumerate() {
            for (ci, ch) in p.chains.iter().enumerate() {
                if ch.bits.is_none() || proj.contains(&(pi, ci)) {
                    continue;
                }
                out.push((p.stream.clone(), ch.index, solids::blend_solids(ch), solids::revolve_solids(ch)));
            }
        }
        out
    }

    pub fn stairs(&self) -> Vec<Stair> {
        self.stairs_by_chain().into_iter().flat_map(|(_, _, s)| s).collect()
    }

    pub fn railings(&self) -> Vec<Railing> {
        self.railings_by_chain().into_iter().flat_map(|(_, _, r)| r).collect()
    }

    /// MEP systems (id and name) of the file.
    pub fn systems(&self) -> &[MepSystem] {
        &self.systems
    }

    fn system_ids(&self) -> HashSet<u64> {
        self.systems.iter().map(|s| s.id).collect()
    }

    /// Straight MEP segments (ducts, pipes, cable trays, conduits) per project chain.
    pub fn segments_by_chain(&self) -> Vec<(String, usize, Vec<Segment>)> {
        let sys = self.system_ids();
        self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, mep::segments(ch, &sys))).collect()
    }

    pub fn segments(&self) -> Vec<Segment> {
        self.segments_by_chain().into_iter().flat_map(|(_, _, s)| s).collect()
    }

    pub fn instances(&self) -> Vec<Instance> {
        self.instances_by_chain().into_iter().flat_map(|(_, _, i)| i).collect()
    }

    pub fn floors(&self) -> Vec<Floor> {
        self.floors_by_chain().into_iter().flat_map(|(_, _, f)| f).collect()
    }

    pub fn ceilings(&self) -> Vec<Floor> {
        self.ceilings_by_chain().into_iter().flat_map(|(_, _, f)| f).collect()
    }

    pub fn roofs(&self) -> Vec<Roof> {
        self.roofs_by_chain().into_iter().flat_map(|(_, _, r)| r).collect()
    }

    /// Levels per project chain, keyed by (partition stream, chain index); used by parity checks.
    pub fn levels_by_chain(&self) -> Vec<(String, usize, Vec<Level>)> {
        self.project_chains().map(|(p, ch)| (p.stream.clone(), ch.index, levels::levels(ch, &self.level_types))).collect()
    }
}

/// A chain is a project chain when it has MEP segments or systems, or (chain 0 only) walls with
/// the project header field (steps 369/370).
fn is_project(ch: &Chain) -> bool {
    let seg = vtag(ch.version, TAG_SEG);
    let sys = vtag(ch.version, TAG_SYS);
    if ch.elements.iter().any(|e| e.tag.is_some() && (e.tag == seg || e.tag == sys)) {
        return true;
    }
    if ch.index != 0 {
        return false;
    }
    let Some(mk) = hdr_marker(ch) else { return false };
    ch.elements.iter().any(|e| {
        e.cat == CAT_WALLS && {
            let b = ch.body(e);
            util::find(&b[..b.len().min(200)], &mk).is_some()
        }
    })
}

/// Header field `ffffffff | u16 class 765 | u32 1` (§29.2, step 378); none if the class is unknown (step 1477).
pub(crate) fn hdr_marker(ch: &Chain) -> Option<Vec<u8>> {
    let t = vtag(ch.version, TAG_HDR)?;
    let mut m = vec![0xff; 4];
    m.extend_from_slice(&t.to_le_bytes());
    m.extend_from_slice(&1u32.to_le_bytes());
    Some(m)
}

/// A §6 line or arc block at `i` (no denormal coordinates), as a curve (for analysis tools).
pub fn curve_block(b: &[u8], i: usize) -> Option<rvt_model::Curve> {
    curves::nodenorm(curves::block(b, i)).filter(|c| matches!(c, curves::Block::Line { .. } | curves::Block::Arc { .. })).map(|c| c.to_curve(0))
}

/// Steps 2284/2285: path pieces of a sweep-like element (slab edge, rail path): [t0][t1] followed
/// by a line (origin, unit direction) or an arc (X, Y, radius, centre); pieces 84 (line) or 117
/// (arc) bytes apart. The longest chain in the record, the last one on a tie.
fn path_pieces(b: &[u8]) -> Vec<rvt_model::Curve> {
    let f = |b: &[u8], o: usize| f64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"));
    let unit3 = |o: usize| -> Option<[f64; 3]> {
        let v = [f(&b, o), f(&b, o + 8), f(&b, o + 16)];
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        (n.is_finite() && (n - 1.0).abs() < 1e-6).then_some(v)
    };
    let pt3 = |o: usize| -> Option<[f64; 3]> {
        let v = [f(&b, o), f(&b, o + 8), f(&b, o + 16)];
        v.iter().all(|x| x.is_finite() && x.abs() < 1e6).then_some(v)
    };
    // arc at s: t0 t1, X, Y (unit, orthogonal), r, C
    let arc = |s: usize| -> Option<rvt_model::Curve> {
        if s + 96 > b.len() {
            return None;
        }
        let (t0, t1) = (f(&b, s), f(&b, s + 8));
        let (x, y) = (unit3(s + 16)?, unit3(s + 40)?);
        let r = f(&b, s + 64);
        let c = pt3(s + 72)?;
        let ok = t0.is_finite() && t1.is_finite() && t1 > t0 && t1 - t0 < 7.0 && t0.abs() < 20.0 && r.is_finite() && r > 1e-3 && r < 1e5 && (x[0] * y[0] + x[1] * y[1] + x[2] * y[2]).abs() < 1e-6;
        let at = |t: f64| [c[0] + r * (t.cos() * x[0] + t.sin() * y[0]), c[1] + r * (t.cos() * x[1] + t.sin() * y[1]), c[2] + r * (t.cos() * x[2] + t.sin() * y[2])];
        ok.then(|| rvt_model::Curve { kind: "arc", p0: Some(at(t0)), p1: Some(at(t1)), t0: Some(t0), t1: Some(t1), center: Some(c), x: Some(x), y: Some(y), r: Some(r), ..Default::default() })
    };
    // line at s: t0 t1, origin, unit direction
    let line = |s: usize| -> Option<rvt_model::Curve> {
        if s + 64 > b.len() {
            return None;
        }
        let (t0, t1) = (f(&b, s), f(&b, s + 8));
        let (o, d) = (pt3(s + 16)?, unit3(s + 40)?);
        let ok = t0.is_finite() && t1.is_finite() && t1 - t0 > 1e-4 && t1 - t0 < 1e4;
        let at = |t: f64| [o[0] + t * d[0], o[1] + t * d[1], o[2] + t * d[2]];
        ok.then(|| rvt_model::Curve { kind: "line", p0: Some(at(t0)), p1: Some(at(t1)), t0: Some(t0), t1: Some(t1), ..Default::default() })
    };
    // pieces are 84 (line) or 117 (arc) bytes apart; the longest chain of pieces, the last
    // one on a tie (an arc is tried first: its X and Y also read as a line)
    let piece = |s: usize| -> Option<(rvt_model::Curve, usize)> { arc(s).map(|c| (c, 117)).or_else(|| line(s).map(|c| (c, 84))) };
    let mut best: Vec<rvt_model::Curve> = Vec::new();
    for s0 in 0..b.len().saturating_sub(64) {
        let mut run = Vec::new();
        let mut s = s0;
        while let Some((c, step)) = piece(s) {
            run.push(c);
            // the next piece is a line or an arc: stride of this piece, else of the other kind
            let next = [s + step, s + if step == 84 { 117 } else { 84 }];
            match next.into_iter().find(|n| piece(*n).is_some()) {
                Some(n) => s = n,
                None => break,
            }
        }
        if !run.is_empty() && run.len() >= best.len() {
            best = run;
        }
    }
    best
}

/// `path_pieces` for analysis tools (step 2285).
pub fn path_pieces_pub(b: &[u8]) -> Vec<rvt_model::Curve> {
    path_pieces(b)
}
