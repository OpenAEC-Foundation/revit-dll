//! Family instances (§13, §16, §30).

use std::collections::{HashMap, HashSet};

use rvt_core::{vtag, Chain, Element};
use rvt_model::{Connector, Instance};

use crate::util::{f64_at, find, i32_at, i64_at, u32_at, u64_at};

const TAG_INSTANCE: u16 = 1989;
const TAG_SYMBOL: u16 = 2022;
const TAG_MULLION_TYPE: u16 = 4245;
const TAG_PANEL_TYPE: u16 = 4247;
const TAG_SKETCH_PLANE: u16 = 3933;
const CAT_LEVELS: i32 = -2000240;
const CAT_GENERIC_ANNOTATION: i32 = -2000150;
const CAT_STRUCTURAL_FRAMING: i32 = -2001320;
const CAT_MULLIONS: i32 = -2000171;
const CAT_CURTAIN_PANELS: i32 = -2000170;

fn d9(b: &[u8], o: usize) -> [f64; 9] {
    std::array::from_fn(|i| f64_at(b, o + 8 * i))
}

fn d3(b: &[u8], o: usize) -> [f64; 3] {
    [f64_at(b, o), f64_at(b, o + 8), f64_at(b, o + 16)]
}

/// Orthonormal 3×3 (rows), entries finite and within ±1.
pub(crate) fn is_ortho(v: &[f64; 9]) -> bool {
    if !v.iter().all(|x| x.is_finite() && x.abs() <= 1.0000001) {
        return false;
    }
    (0..3).all(|i| {
        (0..3).all(|j| {
            let d: f64 = (0..3).map(|k| v[3 * i + k] * v[3 * j + k]).sum();
            (d - if i == j { 1.0 } else { 0.0 }).abs() < 1e-9
        })
    })
}

/// §15.3: records of 136 bytes from the end of the body while the 3×3 is orthonormal.
fn connectors(bd: &[u8]) -> Vec<Connector> {
    let mut out = Vec::new();
    let e = bd.len();
    for i in 0..64 {
        let Some(s) = e.checked_sub(136 * (i + 1)) else { break };
        let m = d9(bd, s + 4);
        if !is_ortho(&m) {
            break;
        }
        let (f1, f3) = (f64_at(bd, s + 100), f64_at(bd, s + 116));
        let round = u32_at(bd, s + 128) == 0;
        out.push(Connector {
            position: d3(bd, s + 76),
            dir: [m[2], m[5], m[8]],
            xdir: [m[0], m[3], m[6]],
            index: u32_at(bd, s + 124),
            shape: if round { "round" } else { "rectangular" },
            r: round.then_some(f1),
            width: (!round).then_some(f3),
            height: (!round).then_some(f1),
        });
    }
    out
}

/// Per-chain lookups shared by all instances.
struct Lookups<'a> {
    bits32: bool,
    w: usize,
    symbols: HashSet<u64>,
    cat: HashMap<u64, i32>,
    byid: HashMap<u64, &'a Element>,
    levels: HashSet<i64>,
    curtain_types: HashMap<i64, i32>,
    sketch_plane: Option<u16>,
}

impl Lookups<'_> {
    fn rd(&self, b: &[u8], o: usize) -> i64 {
        if self.bits32 {
            i32_at(b, o) as i64
        } else {
            i64_at(b, o)
        }
    }
}

/// Steps 2069/2093 anchor: own id (i64) directly after `01 00 00 00`, searched in [40, hi).
fn id_anchor(bd: &[u8], id: u64, hi: usize) -> Option<usize> {
    (40..hi).find(|&o| i64_at(bd, o) == id as i64 && bd[o - 4..o] == [1, 0, 0, 0])
}

fn instance(ch: &Chain, e: &Element, lk: &Lookups) -> Instance {
    let bd = ch.body(e);
    let l = bd.len();
    let w = lk.w;
    // §13.2: first orthonormal 3×3 + origin
    let mut rotation = None;
    let mut origin = None;
    let mut scale = None;
    for o in 0..l.saturating_sub(96) {
        let v = d9(bd, o);
        if is_ortho(&v) {
            rotation = Some(v);
            origin = Some(d3(bd, o + 72));
            break;
        }
    }
    if rotation.is_none() {
        // step 1162: annotation in a view: matrix = rotation × view scale
        for o in 0..l.saturating_sub(96) {
            let v = d9(bd, o);
            if !v.iter().all(|t| t.is_finite() && t.abs() < 1e5) {
                continue;
            }
            let sc = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if 1.5 < sc && sc < 1e5 {
                let n = v.map(|t| t / sc);
                if is_ortho(&n) {
                    rotation = Some(n);
                    origin = Some(d3(bd, o + 72));
                    scale = Some(sc);
                    break;
                }
            }
        }
    }
    // §16: symbols in order of appearance (step 2051: not in the header; step 1327: 32-bit u32)
    let mut syms: Vec<u64> = Vec::new();
    for o in 64..l.saturating_sub(w) {
        let v = if lk.bits32 { u32_at(bd, o) as u64 } else { u64_at(bd, o) };
        if lk.symbols.contains(&v) && !syms.contains(&v) {
            syms.push(v);
        }
    }
    let (mut typ, mut ityp) = (syms.first().copied(), syms.get(1).copied());
    // step 667: type = first symbol with the instance's category
    let same: Vec<u64> = syms.iter().copied().filter(|v| lk.cat.get(v) == Some(&e.cat)).collect();
    if !same.is_empty() && same[0] != syms[0] {
        typ = Some(same[0]);
        ityp = same.get(1).copied();
    }
    if typ.is_none() && (e.cat == CAT_CURTAIN_PANELS || e.cat == CAT_MULLIONS) {
        // step 2001: mullion/panel type = first reference (from @200) to a MullionType/PanelType of the same category
        typ = (200..=l.saturating_sub(w)).map(|o| lk.rd(bd, o)).find(|v| lk.curtain_types.get(v) == Some(&e.cat)).map(|v| v as u64);
    }
    // step 2003: level = first Level ref right after the header (64-bit @62.., 32-bit @54..)
    let lo = if lk.bits32 { 54 } else { 62 };
    let mut level = (lo..(lo + 24).min((l + 1).saturating_sub(w))).map(|o| lk.rd(bd, o)).find(|v| lk.levels.contains(v));
    let mut schedule_level = None;
    if level.is_none() && !lk.bits32 {
        // step 2093: longer header: level right after the own id
        if let Some(ip) = id_anchor(bd, e.id, 120.min(l.saturating_sub(16))) {
            let v = i64_at(bd, ip + 8);
            if lk.levels.contains(&v) {
                level = Some(v);
            }
        }
    }
    if level.is_none() && lk.bits32 {
        // step 2048: 32-bit schedule level = first Level i32 in [280, 330) right after ff×8
        schedule_level = (280..330.min(l.saturating_sub(3))).find(|&o| bd[o - 8..o] == [0xff; 8] && lk.levels.contains(&(i32_at(bd, o) as i64))).map(|o| i32_at(bd, o) as i64);
    }
    if level.is_none() && !lk.bits32 {
        // step 2004: face-based instance: second level ref at +279 from the −1 level field
        if let Some(m1) = (lo..(lo + 24).min(l.saturating_sub(7))).find(|&o| i64_at(bd, o) == -1) {
            if m1 + 287 <= l && lk.levels.contains(&i64_at(bd, m1 + 279)) {
                schedule_level = Some(i64_at(bd, m1 + 279));
            }
        }
        // step 2043: same field anchored on `fd 02 01 00 00 00` + 293
        if schedule_level.is_none() {
            if let Some(mk) = find(&bd[..l.min(200)], &[0xfd, 2, 1, 0, 0, 0]) {
                if mk + 301 <= l && lk.levels.contains(&i64_at(bd, mk + 293)) {
                    schedule_level = Some(i64_at(bd, mk + 293));
                }
            }
        }
        // steps 2069/2151: version-independent anchor = own id; level @id+287 or @id+289
        if schedule_level.is_none() {
            if let Some(ip) = id_anchor(bd, e.id, 200.min(l.saturating_sub(8))) {
                if ip + 295 <= l && lk.levels.contains(&i64_at(bd, ip + 287)) {
                    schedule_level = Some(i64_at(bd, ip + 287));
                } else if ip + 297 <= l && lk.levels.contains(&i64_at(bd, ip + 289)) {
                    schedule_level = Some(i64_at(bd, ip + 289));
                }
            }
        }
    }
    // step 2038: beam/brace without level: reference level via the work plane, else between ff blocks
    let (mut reference_level, mut reference_level_source) = (None, None);
    if e.cat == CAT_STRUCTURAL_FRAMING && level.is_none() {
        let k = if lk.bits32 { 8 } else { 12 };
        if let Some(t) = typ {
            let start = if lk.bits32 { 150 } else { 200 };
            let to = (start..420.min((l + 1).saturating_sub(w))).find(|&o| lk.rd(bd, o) == t as i64);
            if let Some(to) = to {
                if to + 3 * w <= l {
                    let sp = lk.rd(bd, to + 2 * w);
                    if let Some(spe) = (sp >= 0).then(|| lk.byid.get(&(sp as u64))).flatten() {
                        if spe.tag.is_some() && spe.tag == lk.sketch_plane {
                            let sb = ch.body(spe);
                            if let Some(v) = (0..=sb.len().saturating_sub(w)).map(|o| lk.rd(sb, o)).find(|v| lk.levels.contains(v)) {
                                reference_level = Some(v);
                                reference_level_source = Some("sketch_plane");
                            }
                        }
                    }
                }
            }
        }
        if reference_level.is_none() && l >= w + 2 * k {
            let ff = vec![0xffu8; k];
            if let Some(o) = (k..l - w - k).find(|&o| bd[o - k..o] == ff[..] && bd[o + w..o + w + k] == ff[..] && lk.levels.contains(&lk.rd(bd, o))) {
                reference_level = Some(lk.rd(bd, o));
                reference_level_source = Some("ff_anchor");
            } else if ch.version == Some(2014) && l >= 16 {
                // step 2193: 2014 lacks the second ff block: `ff×8 | i32 level | u32 5`
                if let Some(o) = (8..l - 8).find(|&o| bd[o - 8..o] == [0xff; 8] && u32_at(bd, o + 4) == 5 && lk.levels.contains(&(i32_at(bd, o) as i64))) {
                    reference_level = Some(i32_at(bd, o) as i64);
                    reference_level_source = Some("ff_anchor");
                }
            }
        }
    }
    Instance {
        id: e.id,
        cat: e.cat,
        rotation,
        origin,
        symbol: typ,
        instance_type: ityp,
        connectors: connectors(bd),
        scale,
        level,
        schedule_level,
        reference_level,
        reference_level_source,
        work_plane_host: None,
        system: None,
    }
}

/// All family instances (tag 1989) of a chain, with the step 1161 placement of generic annotations.
/// `extra_symbols`: symbol ids of other chains (step 2191: the 2014 document chain's instances
/// reference symbols in the project chain).
pub(crate) fn instances(ch: &Chain, extra_symbols: &HashSet<u64>) -> Vec<Instance> {
    let bits32 = ch.bits == Some(32);
    let curtain: HashSet<Option<u16>> = [vtag(ch.version, TAG_MULLION_TYPE), vtag(ch.version, TAG_PANEL_TYPE)].into_iter().collect();
    let sym = vtag(ch.version, TAG_SYMBOL);
    let lk = Lookups {
        bits32,
        w: if bits32 { 4 } else { 8 },
        symbols: ch.elements.iter().filter(|x| x.tag.is_some() && x.tag == sym).map(|x| x.id).chain(extra_symbols.iter().copied()).collect(),
        cat: ch.elements.iter().map(|x| (x.id, x.cat)).collect(),
        byid: ch.elements.iter().map(|x| (x.id, x)).collect(),
        levels: ch.elements.iter().filter(|x| x.cat == CAT_LEVELS).map(|x| x.id as i64).collect(),
        curtain_types: ch.elements.iter().filter(|x| x.tag.is_some() && curtain.contains(&x.tag)).map(|x| (x.id as i64, x.cat)).collect(),
        sketch_plane: vtag(ch.version, TAG_SKETCH_PLANE),
    };
    let Some(ti) = vtag(ch.version, TAG_INSTANCE) else { return vec![] };
    let mut out: Vec<Instance> = ch.elements.iter().filter(|e| e.tag == Some(ti)).map(|e| instance(ch, e, &lk)).collect();
    // step 1161: generic annotation symbols (545-byte form): placement = 3 f64 at len − 61
    for r in out.iter_mut() {
        if r.origin.is_none() && r.cat == CAT_GENERIC_ANNOTATION {
            let bd = ch.body(lk.byid[&r.id]);
            if bd.len() == 545 {
                let p = d3(bd, bd.len() - 61);
                if p.iter().all(|v| v.is_finite() && v.abs() < 1e6) {
                    r.origin = Some(p);
                }
            }
        }
    }
    out
}

/// Step 2045: work-plane based instances without any level: the SketchPlane (tag 3933) referenced
/// from the body gives (a) a Level → reference level, or (b) the work plane's host (64-bit @150,
/// 32-bit @110, 2014 @111) → `work_plane_host`, plus its level when the host record has one.
pub(crate) fn work_plane_levels(ch: &Chain, insts: &mut [Instance], host_levels: &HashMap<u64, i64>) {
    let bits32 = ch.bits == Some(32);
    let w = if bits32 { 4 } else { 8 };
    let rd = |b: &[u8], o: usize| if bits32 { i32_at(b, o) as i64 } else { i64_at(b, o) };
    let byid: HashMap<u64, &Element> = ch.elements.iter().map(|x| (x.id, x)).collect();
    let levels: HashSet<i64> = ch.elements.iter().filter(|x| x.cat == CAT_LEVELS).map(|x| x.id as i64).collect();
    let sp_tag = vtag(ch.version, TAG_SKETCH_PLANE);
    for r in insts.iter_mut() {
        if r.level.is_some_and(|v| v != 0) || r.schedule_level.is_some_and(|v| v != 0) || r.reference_level.is_some_and(|v| v != 0) {
            continue;
        }
        let Some(e) = byid.get(&r.id) else { continue };
        let bd = ch.body(e);
        let sp = (0..=bd.len().saturating_sub(w)).map(|o| rd(bd, o)).find_map(|v| {
            (v >= 0).then(|| byid.get(&(v as u64))).flatten().filter(|x| x.tag.is_some() && x.tag == sp_tag).copied()
        });
        let Some(sp) = sp else { continue };
        let sb = ch.body(sp);
        if let Some(v) = (0..=sb.len().saturating_sub(w)).map(|o| rd(sb, o)).find(|v| levels.contains(v)) {
            r.reference_level = Some(v);
            r.reference_level_source = Some("sketch_plane");
            continue;
        }
        let (min_len, at) = if bits32 { (115, if ch.version == Some(2014) { 111 } else { 110 }) } else { (158, 150) };
        if sb.len() >= min_len {
            let h = rd(sb, at);
            if h >= 0 && byid.contains_key(&(h as u64)) && h as u64 != sp.id {
                r.work_plane_host = Some(h);
                if let Some(&lv) = host_levels.get(&(h as u64)) {
                    r.reference_level = Some(lv);
                    r.reference_level_source = Some("work_plane_host");
                }
            }
        }
    }
}
