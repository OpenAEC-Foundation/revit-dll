//! Semantic readers: elements of a Revit file to the neutral model, behind a queryable [`Document`].

mod compound;
mod curve_elements;
mod curves;
mod families;
mod hosted;
mod instances;
mod levels;
mod mep;
mod railings;
mod slabs;
mod solids;
mod stairs;
mod util;
mod walls;

use std::collections::{HashMap, HashSet};

use rvt_core::{read_partitions, vtag, Cfb, Chain, Element, Partition};
use rvt_model::{CompoundType, DocumentInfo, ElementInfo, Floor, Instance, LegacyStair, Level, MepSystem, Railing, RailingTypeInfo, Roof, Segment, Stair, Wall};

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
