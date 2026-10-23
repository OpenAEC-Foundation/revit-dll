//! Levels (§18).

use std::collections::HashSet;

use rvt_core::{vtag, Chain};
use rvt_model::Level;

use crate::util::{f64_at, i32_at, i64_at, rfind_before, u32_at};
use crate::{TAG_FAMILY, TAG_LEVEL};

/// Datum plane class markers `u32 5 | u16 class` per version: 2026/2025 (step 368), 2020–2024
/// (step 1344), 2014 (step 1451), 2016–2019 (step 1479), 2015 (step 1511).
const DATUM_CLASSES: [u16; 13] = [0x0265, 0x025D, 0x01EA, 0x01FB, 0x020A, 0x0235, 0x0248, 0x01D4, 0x01D7, 0x01D8, 0x01E3, 0x01E4, 0x01D2];

/// Step 368: datum plane `u32 5 | u16 class | 4×f64 uv box | u8 01 | O 3×f64 | …`; the last
/// occurrence with flag 01 over all markers. Returns (uv box, origin).
pub(crate) fn datum_origin(bd: &[u8]) -> Option<([f64; 4], [f64; 3])> {
    let mut best: Option<usize> = None;
    for &k in &DATUM_CLASSES {
        let mut mk = [0u8; 6];
        mk[..4].copy_from_slice(&5u32.to_le_bytes());
        mk[4..].copy_from_slice(&k.to_le_bytes());
        let mut end = bd.len();
        while let Some(o) = rfind_before(bd, &mk, end) {
            if o + 63 <= bd.len() && bd[o + 38] == 1 {
                best = Some(best.map_or(o, |b| b.max(o)));
                break;
            }
            end = o; // next match must end before o (Python rfind(mk, 0, o))
        }
    }
    let o = best?;
    let uv = [f64_at(bd, o + 6), f64_at(bd, o + 14), f64_at(bd, o + 22), f64_at(bd, o + 30)];
    let origin = [f64_at(bd, o + 39), f64_at(bd, o + 47), f64_at(bd, o + 55)];
    Some((uv, origin))
}

/// §18 (steps 261/368): Level = class tag 2535. Name = first `u32 n | UTF-16` (printable ASCII)
/// from @120 (32-bit: @90, step 1344); elevation = origin z of the datum plane.
pub(crate) fn levels(ch: &Chain, level_types: &HashSet<u64>) -> Vec<Level> {
    let Some(tag) = vtag(ch.version, TAG_LEVEL) else { return vec![] };
    let bits32 = ch.bits == Some(32);
    // step 1456: 2014 keeps family content inline in the project chain (reference to a Family @40–90)
    let fam14: Option<HashSet<u64>> = (ch.version == Some(2014)).then(|| {
        let ft = vtag(ch.version, TAG_FAMILY);
        ch.elements.iter().filter(|x| x.tag.is_some() && x.tag == ft).map(|x| x.id).collect()
    });
    // step 1001: LevelType = first id (offset 200…) that is a LevelType, local or global
    let ltt = vtag(ch.version, crate::TAG_LEVEL_TYPE);
    let mut lt: HashSet<u64> = ch.elements.iter().filter(|x| x.tag.is_some() && x.tag == ltt).map(|x| x.id).collect();
    lt.extend(level_types);

    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag == Some(tag)) {
        let bd = ch.body(e);
        let l = bd.len();
        if let Some(f) = &fam14 {
            if !f.is_empty() && (40..(l.saturating_sub(3)).min(90)).any(|o| i32_at(bd, o) >= 0 && f.contains(&(i32_at(bd, o) as u64))) {
                continue;
            }
        }
        let mut name = None;
        for o in (if bits32 { 90 } else { 120 })..(l.saturating_sub(4)).min(200) {
            let n = u32_at(bd, o) as usize;
            if (1..=100).contains(&n) && o + 4 + 2 * n <= l {
                let t = &bd[o + 4..o + 4 + 2 * n];
                if t.chunks_exact(2).all(|c| (32..127).contains(&c[0]) && c[1] == 0) {
                    name = Some(t.chunks_exact(2).map(|c| c[0] as char).collect::<String>());
                    break;
                }
            }
        }
        let (Some(name), Some((uv, origin))) = (name, datum_origin(bd)) else { continue };
        if !origin[2].is_finite() {
            continue;
        }
        let w = if bits32 { 4 } else { 8 };
        let level_type = (200..=l.saturating_sub(w)).find_map(|o| {
            let q = if bits32 { i32_at(bd, o) as i64 } else { i64_at(bd, o) };
            (q >= 0 && lt.contains(&(q as u64))).then_some(q as u64)
        });
        out.push(Level { id: e.id, name, elevation: origin[2], origin, uv_box: uv, level_type });
    }
    out
}
