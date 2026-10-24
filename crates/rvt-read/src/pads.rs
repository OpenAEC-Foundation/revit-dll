//! Building pads (step 1672): BuildingPad (category −2001263, 2026 tag 658, type 664). Structural:
//! level = first reference of category −2000240, phase −2000112, type −2001263 (not itself),
//! topography = first −2001340, cut surface = the second −2001340 (own id − 1); height offset f64
//! @topography + 2W + 4. Outline = curves of the Sketch (4419) owned by the pad.

use std::collections::{HashMap, HashSet};

use rvt_core::{vtag, Chain, Element};
use rvt_model::BuildingPad;

use crate::curves::curves_in;
use crate::util::{f64_at, i32_at, i64_at};

const TAG_PAD: u16 = 658;
const TAG_SKETCH: u16 = 4419;
const TAG_CURVE: u16 = 954;

pub(crate) fn building_pads(ch: &Chain) -> Vec<BuildingPad> {
    let Some(t) = vtag(ch.version, TAG_PAD) else { return vec![] };
    let xs: Vec<&Element> = ch.elements.iter().filter(|e| e.cat == -2001263 && e.tag == Some(t)).collect();
    if xs.is_empty() {
        return vec![];
    }
    let bits32 = ch.bits == Some(32);
    let w = if bits32 { 4 } else { 8 };
    let rd = |b: &[u8], o: usize| if bits32 { i32_at(b, o) as i64 } else { i64_at(b, o) };
    let by: HashMap<i64, &Element> = ch.elements.iter().map(|e| (e.id as i64, e)).collect();
    // sketch per owner (a later sketch replaces an earlier one, like the reference)
    let st = vtag(ch.version, TAG_SKETCH);
    let ow = if bits32 { 91 } else { 127 };
    let mut sk: HashMap<i64, &Element> = HashMap::new();
    for s in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == st) {
        let sb = ch.body(s);
        if sb.len() >= if bits32 { 95 } else { 135 } {
            sk.insert(rd(sb, ow), s);
        }
    }
    let ct = vtag(ch.version, TAG_CURVE);
    let cur: HashSet<u64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == ct).map(|e| e.id).collect();
    xs.into_iter()
        .map(|e| {
            let b = ch.body(e);
            let l = b.len();
            let mut pad = BuildingPad { id: e.id, ..Default::default() };
            let mut topo_at: Option<usize> = None;
            for o in 40..(l + 1).saturating_sub(w).min(400) {
                let q = rd(b, o);
                if q == e.id as i64 {
                    continue;
                }
                let Some(x) = by.get(&q) else { continue };
                match x.cat {
                    -2000240 if pad.level.is_none() => pad.level = Some(q),
                    -2000112 if pad.phase.is_none() => pad.phase = Some(q),
                    -2001263 if pad.r#type.is_none() => pad.r#type = Some(q),
                    -2001340 => match pad.topo {
                        None => {
                            pad.topo = Some(q);
                            topo_at = Some(o);
                        }
                        Some(tp) if pad.cut_surface.is_none() && q != tp => pad.cut_surface = Some(q),
                        _ => {}
                    },
                    _ => {}
                }
            }
            if let Some(o) = topo_at.filter(|o| o + 2 * w + 12 <= l) {
                pad.height_offset = Some(f64_at(b, o + 2 * w + 4));
            }
            if let Some(s) = sk.get(&(e.id as i64)) {
                pad.sketch = Some(s.id);
                let (lo, n) = s.body;
                pad.outline = Some(curves_in(bits32, &ch.data, &cur, lo, lo + n));
            }
            pad
        })
        .collect()
}
