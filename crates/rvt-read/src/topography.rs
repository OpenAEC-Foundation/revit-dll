//! Topography (step 1532): TopographySurface (category −2001340, 2026 tag 3919) holds point lists
//! `u32 n | n × 3 f64` in its body; the largest list is the terrain points (rac_basic 607/515/146/162
//! points, identical 2014–2026), the smaller ones are probably boundary or helper geometry.

use std::collections::HashSet;

use rvt_core::{vtag, Chain};
use rvt_model::Topography;

use crate::util::f64_at;

const TAG_TOPOGRAPHY: u16 = 3919;
const CAT_TOPOGRAPHY: i32 = -2001340;

pub(crate) fn topography(ch: &Chain) -> Vec<Topography> {
    let Some(t) = vtag(ch.version, TAG_TOPOGRAPHY) else { return vec![] };
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.cat == CAT_TOPOGRAPHY && e.tag == Some(t)) {
        let bd = ch.body(e);
        let l = bd.len();
        let mut lists: Vec<Vec<[f64; 3]>> = Vec::new();
        let mut q = 0usize;
        // every offset (a false early hit must not skip over the real list)
        while q + 28 < l {
            let n = u32::from_le_bytes(bd[q..q + 4].try_into().expect("4 bytes")) as usize;
            if (3..=200_000).contains(&n) && q + 4 + 24 * n <= l {
                let mut pts = Vec::with_capacity(n);
                let mut ok = true;
                for k in 0..n {
                    let o = q + 4 + 24 * k;
                    let p = [f64_at(bd, o), f64_at(bd, o + 8), f64_at(bd, o + 16)];
                    if !p.iter().all(|v| v.is_finite() && v.abs() < 1e5) {
                        ok = false;
                        break;
                    }
                    pts.push(p);
                }
                if ok {
                    // distinct points as the reference counts them (0.0 == -0.0)
                    let key = |v: f64| if v == 0.0 { 0u64 } else { v.to_bits() };
                    let uniq: HashSet<[u64; 3]> = pts.iter().map(|p| [key(p[0]), key(p[1]), key(p[2])]).collect();
                    if uniq.len() as f64 > n as f64 * 0.9 {
                        lists.push(pts);
                    }
                }
            }
            q += 1;
        }
        // the first longest list
        let Some(best) = (0..lists.len()).fold(None, |b: Option<usize>, i| match b {
            Some(j) if lists[j].len() >= lists[i].len() => Some(j),
            _ => Some(i),
        }) else {
            continue;
        };
        let other_lists = lists.iter().enumerate().filter(|(i, _)| *i != best).map(|(_, x)| x.len()).collect();
        out.push(Topography { id: e.id, points: lists.swap_remove(best), other_lists });
    }
    out
}
