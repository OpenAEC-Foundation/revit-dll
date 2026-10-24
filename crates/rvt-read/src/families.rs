//! Family documents (steps 492–494, 1350, 1953): which chain holds a family's own document, and
//! which family a symbol (family type) belongs to.

use std::collections::{BTreeMap, HashMap, HashSet};

use rvt_core::{vtag, Partition};

use crate::util::{i32_at, i64_at};

pub(crate) const TAG_FAMILY: u16 = 1967;
const TAG_SYMBOL: u16 = 2022;

/// Family id -> (partition index, chain index) of its document chain: the chain whose key occurs
/// inside the family element's body in chain 0 of a partition (step 492). First match in
/// (partition, other partition, chain) order wins, like the reference.
pub(crate) fn family_chains(parts: &[Partition]) -> BTreeMap<u64, (usize, usize)> {
    // every chain key -> (partition, chain) in reference iteration order
    let mut keys: HashMap<[u8; 16], Vec<(usize, usize)>> = HashMap::new();
    for (q, p) in parts.iter().enumerate() {
        for (xi, x) in p.chains.iter().enumerate() {
            if let Some(k) = x.key {
                keys.entry(k).or_default().push((q, xi));
            }
        }
    }
    let prefixes: HashSet<u32> = keys.keys().map(|k| u32::from_le_bytes([k[0], k[1], k[2], k[3]])).collect();
    // (order key, family) candidates
    let mut best: HashMap<u64, ((usize, usize, usize), (usize, usize))> = HashMap::new();
    for (pi, p) in parts.iter().enumerate() {
        let Some(ch) = p.chains.first().filter(|c| c.bits.is_some()) else { continue };
        scan_family_keys(ch, pi, 0, &keys, &prefixes, &mut best);
    }
    best.into_iter().map(|(f, (_, c))| (f, c)).collect()
}

/// Step 2246 (§10 nesting): families nested in a family document, i.e. Family records in a family
/// chain (index > 0) whose body holds the key of another chain; same rule as `family_chains`.
pub(crate) fn nested_family_chains(parts: &[Partition]) -> BTreeMap<u64, (usize, usize)> {
    let mut keys: HashMap<[u8; 16], Vec<(usize, usize)>> = HashMap::new();
    for (q, p) in parts.iter().enumerate() {
        for (xi, x) in p.chains.iter().enumerate() {
            if let Some(k) = x.key {
                keys.entry(k).or_default().push((q, xi));
            }
        }
    }
    let prefixes: HashSet<u32> = keys.keys().map(|k| u32::from_le_bytes([k[0], k[1], k[2], k[3]])).collect();
    let mut best: HashMap<u64, ((usize, usize, usize), (usize, usize))> = HashMap::new();
    for (pi, p) in parts.iter().enumerate() {
        for (xi, ch) in p.chains.iter().enumerate().skip(1).filter(|(_, c)| c.bits.is_some()) {
            scan_family_keys(ch, pi, xi, &keys, &prefixes, &mut best);
        }
    }
    best.into_iter().map(|(f, (_, c))| (f, c)).collect()
}

/// Family records of chain `ch` (partition `pi`, chain `self_xi`): the first key of another chain
/// inside a family body names that family's document chain.
fn scan_family_keys(
    ch: &rvt_core::Chain,
    pi: usize,
    self_xi: usize,
    keys: &HashMap<[u8; 16], Vec<(usize, usize)>>,
    prefixes: &HashSet<u32>,
    best: &mut HashMap<u64, ((usize, usize, usize), (usize, usize))>,
) {
    {
        let ft = vtag(ch.version, TAG_FAMILY);
        let mut fams: Vec<(usize, usize, u64)> =
            ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == ft).map(|e| (e.body.0, e.body.0 + e.body.1, e.id)).collect();
        if fams.is_empty() {
            return;
        }
        fams.sort();
        // family bodies do not overlap: the family containing i by binary search on the sorted starts
        let fam_at = |i: usize| -> Option<u64> {
            let k = fams.partition_point(|f| f.0 <= i);
            (k > 0 && i < fams[k - 1].1).then(|| fams[k - 1].2)
        };
        let (lo, hi) = (fams[0].0, fams.iter().map(|f| f.1).max().unwrap_or(0));
        let data = &ch.data;
        // first occurrence per key that lies in a family body
        let mut done: HashSet<[u8; 16]> = HashSet::new();
        if hi < 16 {
            return;
        }
        for i in lo..hi.min(data.len()).saturating_sub(15) {
            // step 2227: cheap 4-byte prefix check before the full key lookup
            if !prefixes.contains(&u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])) {
                continue;
            }
            let k: [u8; 16] = data[i..i + 16].try_into().expect("16 bytes");
            let Some(owners) = keys.get(&k) else { continue };
            if done.contains(&k) {
                continue;
            }
            let Some(f) = fam_at(i) else { continue };
            done.insert(k);
            for &(q, xi) in owners {
                if q == pi && xi == self_xi {
                    continue; // the chain itself
                }
                let order = (pi, q, xi);
                let e = best.entry(f).or_insert((order, (q, xi)));
                if order < e.0 {
                    *e = (order, (q, xi));
                }
            }
        }
    }
}

/// Symbol (tag 2022, chain 0) -> family: 64-bit the first family (in file order) whose i64 id
/// occurs in the symbol body; 32-bit the earliest i32 family id ≥ 0x10000, else the earliest
/// (steps 494, 1350, 1953).
pub(crate) fn symbol_family(parts: &[Partition]) -> HashMap<u64, u64> {
    let chains: Vec<&rvt_core::Chain> = parts.iter().filter_map(|p| p.chains.first().filter(|c| c.bits.is_some())).collect();
    symbol_family_in(&chains, false)
}

/// Step 2246: symbols of nested families (in family chains, index > 0) -> family, by the same rule.
pub(crate) fn nested_symbol_family(parts: &[Partition]) -> HashMap<u64, u64> {
    let chains: Vec<&rvt_core::Chain> = parts.iter().flat_map(|p| p.chains.iter().skip(1).filter(|c| c.bits.is_some())).collect();
    symbol_family_in(&chains, true)
}

/// `skip_header`: in a family chain the header slot (64-bit @76, 32-bit @60) of every record holds
/// the document's own Family; the symbol's family follows later in the body (rac_basic solar panel
/// symbol 1043463: own family 1043002 @76, its family 1044075 @467).
fn symbol_family_in(chains: &[&rvt_core::Chain], skip_header: bool) -> HashMap<u64, u64> {
    let mut fams: Vec<u64> = Vec::new();
    for ch in chains {
        let ft = vtag(ch.version, TAG_FAMILY);
        fams.extend(ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == ft).map(|e| e.id));
    }
    let fset: HashSet<i64> = fams.iter().map(|&f| f as i64).collect();
    let mut fam_index: HashMap<i64, usize> = HashMap::new();
    for (k, &f) in fams.iter().enumerate() {
        fam_index.entry(f as i64).or_insert(k);
    }
    let mut out = HashMap::new();
    for ch in chains {
        let st = vtag(ch.version, TAG_SYMBOL);
        let bits32 = ch.bits == Some(32);
        for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == st) {
            let b = ch.body(e);
            // step 2227: one scan of the body (same result as testing every family in file order)
            let fid = if bits32 {
                // a family found anywhere implies a hit at that offset; earliest i32 id >= 0x10000, else the earliest
                let hits: Vec<i64> =
                    (if skip_header { 80 } else { 0 }..b.len().saturating_sub(3)).map(|o| i32_at(b, o) as i64).filter(|v| fset.contains(v)).collect();
                hits.iter().copied().find(|&v| v >= 0x10000).or(hits.first().copied()).map(|v| v as u64)
            } else {
                // the family that comes first in file order among those whose id occurs in the body
                (if skip_header { 96 } else { 0 }..b.len().saturating_sub(7)).filter_map(|o| fam_index.get(&i64_at(b, o)).copied()).min().map(|k| fams[k])
            };
            if let Some(f) = fid {
                out.insert(e.id, f);
            }
        }
    }
    out
}
