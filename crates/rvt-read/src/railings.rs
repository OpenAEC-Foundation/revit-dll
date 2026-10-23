//! Railings (§35, steps 433–445, 1373): path from the railing's own sketch, placement fields
//! anchored on the RailingType id, height and top rail from the type.

use std::collections::{HashMap, HashSet};

use rvt_core::{vtag, Chain, Element};
use rvt_model::{Railing, RailingTypeInfo};

use crate::curves::curves_in;
use crate::slabs::CURVE_CAT;
use crate::util::{f64_at, find, i32_at, i64_at};

const TAG_SKETCH: u16 = 4419;
const TAG_RAILING: u16 = 579;
pub(crate) const TAG_RAILING_TYPE: u16 = 4036;
const BIP_RAILING_HEIGHT: i64 = -1008602;
const BIP_TOP_RAIL: i64 = -1008629;

fn pack(bits32: bool, v: i64) -> Vec<u8> {
    if bits32 {
        (v as i32).to_le_bytes().to_vec()
    } else {
        v.to_le_bytes().to_vec()
    }
}

/// Railing Height = f64 before BIP −1008602 (step 440); top rail `f64 height | type` before
/// BIP −1008629 (64-bit @−45/−37, 32-bit @−49/−17; steps 443–444, 1373).
pub(crate) fn rail_type_info(bd: &[u8], bits32: bool) -> RailingTypeInfo {
    let height = find(bd, &pack(bits32, BIP_RAILING_HEIGHT)).filter(|&k| k >= 8).map(|k| f64_at(bd, k - 8));
    let (toprail_height, toprail_type) = match find(bd, &pack(bits32, BIP_TOP_RAIL)) {
        Some(a) if bits32 && a >= 49 => (Some(f64_at(bd, a - 49)), Some(i32_at(bd, a - 17) as i64)),
        Some(a) if !bits32 && a >= 45 => (Some(f64_at(bd, a - 45)), Some(i64_at(bd, a - 37))),
        _ => (None, None),
    };
    RailingTypeInfo { height, toprail_height, toprail_type }
}

/// RailingType id -> type values, over every chain of the file (reference `grt`).
pub(crate) fn rail_types<'a>(chains: impl Iterator<Item = &'a Chain>) -> HashMap<i64, RailingTypeInfo> {
    let mut out = HashMap::new();
    for ch in chains.filter(|c| c.bits.is_some()) {
        let t = vtag(ch.version, TAG_RAILING_TYPE);
        for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == t) {
            out.insert(e.id as i64, rail_type_info(ch.body(e), ch.bits == Some(32)));
        }
    }
    out
}

/// Railings of one chain: sketches (tag 4419) whose owner (@127, 32-bit @91) is a Railing.
pub(crate) fn railings(ch: &Chain, rail_types: &HashMap<i64, RailingTypeInfo>) -> Vec<Railing> {
    let bits32 = ch.bits == Some(32);
    let (w, own) = if bits32 { (4usize, 91usize) } else { (8, 127) };
    let byid: HashMap<u64, &Element> = ch.elements.iter().map(|e| (e.id, e)).collect();
    let cur: HashSet<u64> = ch.elements.iter().filter(|e| e.cat == CURVE_CAT).map(|e| e.id).collect();
    let (sketch_tag, railing_tag) = (vtag(ch.version, TAG_SKETCH), vtag(ch.version, TAG_RAILING));
    let q = |b: &[u8], k: usize| if bits32 { i32_at(b, k) as i64 } else { i64_at(b, k) };
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == sketch_tag) {
        let sb = ch.body(e);
        if sb.len() < if bits32 { 95 } else { 135 } {
            continue;
        }
        let rid = q(sb, own);
        let Some(re) = (rid >= 0).then(|| byid.get(&(rid as u64))).flatten() else { continue };
        if re.tag.is_none() || re.tag != railing_tag {
            continue;
        }
        let bd = ch.body(re);
        let offsets = 0..(bd.len() + 1).saturating_sub(w);
        let tp: Vec<usize> = offsets.clone().filter(|&k| rail_types.contains_key(&q(bd, k))).collect();
        let sp: Vec<usize> = offsets.filter(|&k| q(bd, k) == rid).collect();
        let (lo, n) = e.body;
        let mut r = Railing {
            id: rid as u64,
            sketch: e.id,
            railing_type: None,
            host: None,
            placement_offset: None,
            level: None,
            path: curves_in(bits32, &ch.data, &cur, lo, lo + n),
            type_info: None,
        };
        if let Some(&t) = tp.last().filter(|&&t| t >= 32) {
            r.railing_type = Some(q(bd, t));
            r.host = Some(q(bd, if bits32 { t - 20 } else { t - 32 }));
            r.placement_offset = Some(f64_at(bd, if bits32 { t - 32 } else { t - 24 }));
            r.type_info = rail_types.get(&q(bd, t)).cloned();
        }
        if sp.len() >= 2 && sp[1] + 2 * w <= bd.len() {
            r.level = Some(q(bd, sp[1] + w));
        }
        out.push(r);
    }
    out
}
