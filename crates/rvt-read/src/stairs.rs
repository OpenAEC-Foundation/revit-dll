//! Stairs (§33, steps 401–405, 722, 1328, 1453–1454, 1489–1490): riser, tread, levels, offsets,
//! height and runs of component stairs (tag 4012).

use std::collections::{HashMap, HashSet};

use rvt_core::{vtag, Chain};
use rvt_model::Stair;

use crate::util::{f64_at, find, i32_at, i64_at};

const TAG_STAIRS: u16 = 4012;
const TAG_RUN: u16 = 4039;
const TAG_MARKER: u16 = 1065;
const CAT_LEVELS: i32 = -2000240;

/// Stairs of one chain; `level_z` = level id -> elevation over the whole file (reference `glev`).
pub(crate) fn stairs(ch: &Chain, level_z: &HashMap<i64, f64>) -> Vec<Stair> {
    let mut out = Vec::new();
    let v = ch.version.unwrap_or(0);
    // step 1453/1489: 2014 header without the marker; 2015–2017 use the 2014 layout
    let s2014 = matches!(v, 2014..=2017);
    let k_tag = vtag(ch.version, TAG_MARKER);
    if k_tag.is_none() && !s2014 {
        return out; // step 1328: marker unknown in this version
    }
    let bits32 = ch.bits == Some(32);
    let pack = |id: u64| if bits32 { (id as i32).to_le_bytes().to_vec() } else { (id as i64).to_le_bytes().to_vec() };
    let seq: Option<Vec<u8>> = k_tag.map(|k| {
        let mut s = vec![0xff; 4];
        s.extend_from_slice(&k.to_le_bytes());
        s.extend_from_slice(&[0xff; 4]);
        s.extend_from_slice(&k.to_le_bytes());
        s
    });
    let (st_tag, run_tag) = (vtag(ch.version, TAG_STAIRS), vtag(ch.version, TAG_RUN));
    let runs: Vec<(u64, &[u8])> =
        ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == run_tag).map(|e| (e.id, ch.body(e))).collect();
    let levels: HashSet<i32> = ch.elements.iter().filter(|e| e.cat == CAT_LEVELS).map(|e| e.id as i32).collect();
    let run_ids = |id: u64| -> Vec<u64> {
        let pid = pack(id);
        runs.iter().filter(|(_, rb)| find(rb, &pid).is_some()).map(|(r, _)| *r).collect()
    };
    let derived = |bl: i64, bo: f64, tl: i64, to: f64, hh: f64| -> (f64, Option<f64>) {
        let ha = match (level_z.get(&bl), level_z.get(&tl)) {
            (Some(zb), Some(zt)) => (zt + to) - (zb + bo),
            _ => hh,
        };
        (ha, level_z.get(&bl).map(|z| z + bo))
    };
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == st_tag) {
        let bd = ch.body(e);
        let mut o = seq.as_ref().and_then(|s| find(bd, s));
        if s2014 && (o.is_none() || v != 2014) {
            // step 1489: in 2016/2017 the marker exists, but the block lies before it
            let k = (60..bd.len().saturating_sub(16).min(400)).find(|&q| {
                levels.contains(&i32_at(bd, q)) && levels.contains(&i32_at(bd, q + 4)) && i32_at(bd, q + 8) == -1
            });
            let Some(k) = k.filter(|&k| k >= 32) else { continue };
            let (hh, ri, tr) = (f64_at(bd, k - 32), f64_at(bd, k - 24), f64_at(bd, k - 16));
            let (bl, tl) = (i32_at(bd, k) as i64, i32_at(bd, k + 4) as i64);
            if !(ri > 0.0 && ri < 10.0 && tr > 0.0 && tr < 10.0) {
                continue;
            }
            // step 1490: base and top offset @k−48 / @k−40
            let (bo, to) = if k >= 48 { (f64_at(bd, k - 48), f64_at(bd, k - 40)) } else { (0.0, 0.0) };
            let (ha, base_z) = derived(bl, bo, tl, to, hh);
            out.push(Stair {
                id: e.id,
                riser: ri,
                tread: tr,
                base_level: bl,
                base_offset: bo,
                top_level: tl,
                top_offset: to,
                height: ha,
                desired_height: hh,
                risers: (ha / ri).round_ties_even() as i64,
                base_z,
                runs: run_ids(e.id),
            });
            continue;
        }
        if o.is_none() {
            // step 1454: `ffffffff w | u32 1 | ffffffff w` (list between the markers) -> block 4 B later
            if let Some(k) = k_tag {
                let mut s2 = vec![0xff; 4];
                s2.extend_from_slice(&k.to_le_bytes());
                s2.extend_from_slice(&[0x01, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
                s2.extend_from_slice(&k.to_le_bytes());
                o = find(bd, &s2).map(|x| x + 4);
            }
        }
        let Some(o) = o.filter(|&o| o + 72 <= bd.len()) else { continue };
        let (ri, tr, bl, bo, m1, hh, tl, to) = if bits32 {
            // step 1328: 5 × f64 (riser, tread, base offset, height, top offset), i32 count, base, −1, top
            let d = |i: usize| f64_at(bd, o + 16 + 8 * i);
            (d(0), d(1), i32_at(bd, o + 60) as i64, d(2), i32_at(bd, o + 64) as i64, d(3), i32_at(bd, o + 68) as i64, d(4))
        } else {
            let b = o + 16;
            (f64_at(bd, b), f64_at(bd, b + 8), i64_at(bd, b + 16), f64_at(bd, b + 24), i64_at(bd, b + 32), f64_at(bd, b + 40), i64_at(bd, b + 48), f64_at(bd, b + 56))
        };
        if m1 != -1 || !(ri > 0.0 && ri < 10.0 && tr > 0.0 && tr < 10.0) {
            continue;
        }
        let (ha, base_z) = derived(bl, bo, tl, to, hh);
        out.push(Stair {
            id: e.id,
            riser: ri,
            tread: tr,
            base_level: bl,
            base_offset: bo,
            top_level: tl,
            top_offset: to,
            height: ha,
            desired_height: hh,
            risers: (ha / ri).round_ties_even() as i64,
            base_z,
            runs: run_ids(e.id),
        });
    }
    out
}
