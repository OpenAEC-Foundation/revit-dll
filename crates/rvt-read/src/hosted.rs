//! Hosted family instances (§30, steps 374–377, 476–477, 1348): host element, position along the
//! host (HostParameter) and sill height of doors, windows and other hosted families.

use std::collections::HashMap;

use rvt_core::{vtag, Chain, Element};
use rvt_model::Hosted;

use crate::util::{f64_at, find, i32_at, i64_at};

const TAG_INSTANCE: u16 = 1989;
const TAG_SKETCH_PLANE: u16 = 3933;
const TAG_SILL: u16 = 2463;

/// FamilyInstances with the wall header (§29.2): 64-bit `id | level` @m+10, HostParameter f64
/// @m+18+77, host i64 @m+18+85; 32-bit id/level i32 @m+10/@m+14, HostParameter @m+63, host i32
/// @m+71. Not hosted (−1) is skipped. Sill = f64 @anchor+10 after `ffffffff | u16 tag 2463`.
pub(crate) fn hosted(ch: &Chain, link_ids: &[u64]) -> Vec<Hosted> {
    let Some(hdr) = crate::hdr_marker(ch) else { return vec![] };
    let bits32 = ch.bits == Some(32);
    let it = vtag(ch.version, TAG_INSTANCE);
    let byid: HashMap<u64, &Element> = ch.elements.iter().map(|e| (e.id, e)).collect();
    let sill_mark: Option<Vec<u8>> = vtag(ch.version, TAG_SILL).map(|t| {
        let mut m = vec![0xff; 4];
        m.extend_from_slice(&t.to_le_bytes());
        m
    });
    let sp_tag = vtag(ch.version, TAG_SKETCH_PLANE);
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == it) {
        let bd = ch.body(e);
        let Some(m) = find(bd, &hdr) else { continue };
        if m > 200 || m + 18 + 93 > bd.len() {
            continue;
        }
        let (sid, lvl, hp, host) = if bits32 {
            (i32_at(bd, m + 10) as i64, i32_at(bd, m + 14) as i64, f64_at(bd, m + 63), i32_at(bd, m + 71) as i64)
        } else {
            (i64_at(bd, m + 10), i64_at(bd, m + 18), f64_at(bd, m + 18 + 77), i64_at(bd, m + 18 + 85))
        };
        if sid != e.id as i64 || host == -1 {
            continue;
        }
        let sill = sill_mark.as_ref().and_then(|mk| find(bd, mk)).filter(|&a| a + 18 <= bd.len()).map(|a| f64_at(bd, a + 10));
        let mut r = Hosted { id: e.id, level: lvl, host: Some(host), host_param: hp, sill, work_plane: None };
        // step 377: host field = work plane (SketchPlane); a plane in a linked model -> link instance
        if host >= 0 && byid.get(&(host as u64)).is_some_and(|h| h.tag.is_some() && h.tag == sp_tag) {
            r.work_plane = Some(host);
            let pb = ch.body(byid[&(host as u64)]);
            r.host = link_ids.iter().find(|&&l| find(pb, &(l as i64).to_le_bytes()).is_some()).map(|&l| l as i64);
        }
        out.push(r);
    }
    out
}
