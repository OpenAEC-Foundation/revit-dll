//! Semantic readers: elements of a Revit file to the neutral model, behind a queryable [`Document`].

mod levels;
mod util;

use std::collections::{HashMap, HashSet};

use rvt_core::{read_partitions, vtag, Cfb, Chain, Element, Partition};
use rvt_model::{DocumentInfo, ElementInfo, Level};

pub use rvt_core::{Error, Result};

/// Canonical (2026) class tags used for chain classification (reference reader line `TAG_SEG, …`).
pub(crate) const TAG_SEG: u16 = 3442;
pub(crate) const TAG_SYS: u16 = 3481;
pub(crate) const TAG_FAMILY: u16 = 1967;
pub(crate) const TAG_LEVEL: u16 = 2535;
pub(crate) const TAG_LEVEL_TYPE: u16 = 2536;
const TAG_HDR: u16 = 765;
const CAT_WALLS: i32 = -2000011;

/// An opened `.rvt` file.
pub struct Document {
    partitions: Vec<Partition>,
    /// (partition index, chain index) of the project chains.
    project: Vec<(usize, usize)>,
    index: HashMap<u64, (usize, usize, usize)>,
    /// LevelType ids over all partitions (step 1001).
    pub(crate) level_types: HashSet<u64>,
}

impl Document {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let cfb = Cfb::open(path)?;
        Ok(Self::from_partitions(read_partitions(&cfb)?))
    }

    pub fn from_partitions(partitions: Vec<Partition>) -> Self {
        let mut project = Vec::new();
        let mut index = HashMap::new();
        let mut level_types = HashSet::new();
        for (pi, p) in partitions.iter().enumerate() {
            for (ci, ch) in p.chains.iter().enumerate() {
                if ch.bits.is_none() {
                    continue;
                }
                if is_project(ch) {
                    project.push((pi, ci));
                }
                let lt = vtag(ch.version, TAG_LEVEL_TYPE);
                for (ei, e) in ch.elements.iter().enumerate() {
                    index.entry(e.id).or_insert((pi, ci, ei));
                    if e.tag.is_some() && e.tag == lt {
                        level_types.insert(e.id);
                    }
                }
            }
        }
        Document { partitions, project, index, level_types }
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
fn hdr_marker(ch: &Chain) -> Option<Vec<u8>> {
    let t = vtag(ch.version, TAG_HDR)?;
    let mut m = vec![0xff; 4];
    m.extend_from_slice(&t.to_le_bytes());
    m.extend_from_slice(&1u32.to_le_bytes());
    Some(m)
}
