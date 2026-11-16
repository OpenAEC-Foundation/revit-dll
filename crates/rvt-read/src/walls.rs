//! Walls (§29).

use std::collections::{BTreeMap, HashMap, HashSet};

use rvt_core::{vtag, Chain};
use rvt_model::{ArcCurve, Wall};

use crate::curves::{arc_point, block, nodenorm, Block};
use crate::util::{f64_at, find, i32_at, i64_at, u32_at};

const CAT_WALLS: i32 = -2000011;
const CAT_STACKED: i32 = -2003500;
const CAT_CURTAIN_PANELS: i32 = -2000170;
const TAG_WALL_RECORD: u16 = 3842;
const TAG_INSTANCE: u16 = 1989;
const TAG_PHASE: u16 = 3351;
const WALL_LOC: [u8; 8] = [0xff; 8];

/// Wall location line (§6.2 block right after `u32 (0|2) | 8×ff`, step 365).
#[derive(Clone)]
enum Loc {
    Line { p0: [f64; 3], p1: [f64; 3] },
    Arc { p0: [f64; 3], p1: [f64; 3], curve: ArcCurve },
}

impl Loc {
    fn from_block(b: Block) -> Option<Loc> {
        match b {
            Block::Line { p0, p1, .. } => Some(Loc::Line { p0, p1 }),
            Block::Arc { t0, t1, x, y, r, c, .. } => Some(Loc::Arc {
                p0: arc_point(&c, r, &x, &y, t0),
                p1: arc_point(&c, r, &x, &y, t1),
                curve: ArcCurve { kind: "arc", center: c, r, x, y, t0, t1 },
            }),
            Block::Ellipse { .. } => None,
        }
    }
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..).and_then(|h| find(h, needle)).map(|o| o + from)
}

fn wall_line(bd: &[u8], bits32: bool) -> Option<Loc> {
    if bits32 {
        // steps 1321/1322: `ff×4 | 04 00 xx 01`, the block starts 4 bytes before the ff×4; lines only
        let mk = [0xff, 0xff, 0xff, 0xff, 0x04, 0x00];
        let mut k = find_from(bd, &mk, 4);
        while let Some(kk) = k {
            if kk + 8 <= bd.len() && bd[kk + 7] == 1 {
                if let Some(Block::Line { p0, p1, .. }) = block(bd, kk - 4) {
                    return Some(Loc::Line { p0, p1 });
                }
            }
            k = find_from(bd, &mk, kk + 1);
        }
        return None;
    }
    let mut o = find_from(bd, &WALL_LOC, 4);
    while let Some(oo) = o {
        let pre = &bd[oo - 4..oo];
        if pre == [0, 0, 0, 0] || pre == [2, 0, 0, 0] || pre == [0, 0, 0xf8, 0x3f] {
            // step 639: also after f64 1.5
            if let Some(l) = block(bd, oo + 8).and_then(Loc::from_block) {
                return Some(l);
            }
        }
        o = find_from(bd, &WALL_LOC, oo + 1);
    }
    // step 1422: an f64 before the 8×ff; take the first real line/arc block (header 04/24, no denormals)
    let mut o = find_from(bd, &WALL_LOC, 4);
    while let Some(oo) = o {
        if oo + 16 < bd.len() && matches!(bd[oo + 16], 0x04 | 0x24) {
            if let Some(l) = nodenorm(block(bd, oo + 8)).and_then(Loc::from_block) {
                return Some(l);
            }
        }
        o = find_from(bd, &WALL_LOC, oo + 1);
    }
    None
}

/// Inputs gathered over the whole file.
pub(crate) struct WallContext<'a> {
    pub level_z: &'a HashMap<i64, f64>,
    /// Wall type ids over all partitions.
    pub wall_types: &'a HashSet<i64>,
    /// Phase ids over all partitions (step 1880).
    pub phases: &'a HashSet<i64>,
}

/// §29 (steps 365–369): wall instances = category −2000011 with the project header field and a
/// location line (wall types have the header but no line).
pub(crate) fn walls(ch: &Chain, ctx: &WallContext) -> Vec<Wall> {
    let Some(hdr) = crate::hdr_marker(ch) else { return vec![] };
    let bits32 = ch.bits == Some(32);
    let w = if bits32 { 4 } else { 8 };
    let rd = |b: &[u8], o: usize| if bits32 { i32_at(b, o) as i64 } else { i64_at(b, o) };
    let pack = |v: i64| if bits32 { (v as i32).to_le_bytes().to_vec() } else { v.to_le_bytes().to_vec() };
    let tag_rec = vtag(ch.version, TAG_WALL_RECORD);
    let tag_inst = vtag(ch.version, TAG_INSTANCE);
    let tag_fam = vtag(ch.version, crate::TAG_FAMILY);
    let tag_phase = vtag(ch.version, TAG_PHASE);
    let cat_of: HashMap<u64, i32> = ch.elements.iter().map(|e| (e.id, e.cat)).collect();

    let mut phase_ids: HashSet<i64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == tag_phase).map(|e| e.id as i64).collect();
    phase_ids.extend(ctx.phases);

    // candidates: walls, stacked walls (step 718), wall records used as curtain panels (step 1423)
    let cand: BTreeMap<u64, &[u8]> = ch
        .elements
        .iter()
        .filter(|e| e.cat == CAT_WALLS || e.cat == CAT_STACKED || (e.cat == CAT_CURTAIN_PANELS && e.tag.is_some() && e.tag == tag_rec))
        .map(|e| (e.id, ch.body(e)))
        .collect();
    let mut lines: BTreeMap<u64, Option<Loc>> = cand
        .iter()
        .filter(|(_, bd)| find(&bd[..bd.len().min(200)], &hdr).is_some())
        .map(|(&i, bd)| (i, wall_line(bd, bits32)))
        .collect();

    // step 718: a stacked wall's part has no own line; a panel (tag 1989) holds the part id and the parent @151
    let mut parent: BTreeMap<u64, u64> = BTreeMap::new();
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == tag_inst) {
        let pb = ch.body(e);
        if pb.len() < 160 {
            continue;
        }
        let q = i64_at(pb, 151);
        if q < 0 {
            continue;
        }
        let q = q as u64;
        if matches!(lines.get(&q), Some(Some(_))) && cand.contains_key(&q) && cat_of.get(&q) == Some(&CAT_STACKED) {
            let refs = e.refs.as_deref().unwrap_or(&[]);
            for (&i, l) in lines.iter() {
                if l.is_none() && (refs.contains(&i) || find(pb, &(i as i64).to_le_bytes()).is_some()) {
                    parent.insert(i, q);
                }
            }
        }
    }
    for (&i, &q) in &parent {
        let l = lines[&q].clone();
        lines.insert(i, l);
    }

    let inst: Vec<u64> = lines.iter().filter(|(_, l)| l.is_some()).map(|(&i, _)| i).collect();
    let inst_set: HashSet<u64> = inst.iter().copied().collect();
    let mut wtypes: HashSet<i64> = ctx.wall_types.clone();
    wtypes.extend(cand.keys().filter(|i| !inst_set.contains(i)).map(|&i| i as i64));
    let fam: Vec<(u64, &[u8])> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == tag_fam).map(|e| (e.id, ch.body(e))).collect();
    // the family ids as a wall body holds them (32-bit files: their low four bytes)
    let as_stored = |f: u64| if bits32 { f as i64 as i32 as i64 } else { f as i64 };
    let fam_ids: HashSet<i64> = fam.iter().map(|(f, _)| as_stored(*f)).collect();

    let mut out = Vec::new();
    for i in inst {
        let bd = cand[&i];
        let l = bd.len();
        let Some(m) = find(bd, &hdr) else { continue };
        if m + 10 + 2 * w > l {
            continue;
        }
        let (sid, base) = (rd(bd, m + 10), rd(bd, m + 10 + w));
        if sid != i as i64 {
            continue;
        }
        let Some(to) = (m + 10 + 2 * w..l.saturating_sub(2 * w)).find(|&o| wtypes.contains(&rd(bd, o))) else { continue };
        let (typ, top) = (rd(bd, to), rd(bd, to + w));
        // parameter list `f64 value | id BuiltInParameter` from −1012829
        let mut prm: HashMap<i64, f64> = HashMap::new();
        if let Some(mut k) = find_from(bd, &pack(-1012829), to) {
            while 8 <= k && k + w <= l {
                let pid = rd(bd, k);
                if !(-1100000 < pid && pid < -1000000) {
                    break;
                }
                prm.insert(pid, f64_at(bd, k - 8));
                k += 8 + w;
            }
        }
        let bo = prm.get(&-1001108).copied().unwrap_or(0.0);
        let tof = prm.get(&-1001109).copied().unwrap_or(0.0);
        let height = if top == -1 {
            prm.get(&-1001105).copied()
        } else if !ctx.level_z.is_empty() && ctx.level_z.contains_key(&top) && ctx.level_z.contains_key(&base) {
            Some((ctx.level_z[&top] + tof) - (ctx.level_z[&base] + bo))
        } else {
            None
        };
        let (p0, p1, curve) = match lines[&i].clone().expect("instance has a line") {
            Loc::Line { p0, p1 } => (p0, p1, None),
            Loc::Arc { p0, p1, curve } => (p0, p1, Some(curve)),
        };
        let own = pack(i as i64);
        // the families the wall names, in one pass over its body (a project has hundreds of
        // families: a search per family took 11.5 of the 11.6 s of reading 2044 R24's 801 walls)
        let named: HashSet<i64> = (0..(l + 1).saturating_sub(w)).map(|o| rd(bd, o)).filter(|v| fam_ids.contains(v)).collect();
        let in_place = !named.is_empty() && fam.iter().any(|(f, fb)| named.contains(&as_stored(*f)) && find(fb, &own).is_some());
        let cat = cat_of.get(&i).copied();
        let mut wall = Wall {
            id: i,
            wall_type: typ,
            base_level: base,
            top_level: top,
            base_offset: bo,
            top_offset: tof,
            height,
            p0,
            p1,
            curve,
            in_place,
            stacked_parent: parent.get(&i).copied(),
            stacked: cat == Some(CAT_STACKED),
            curtain_panel: cat == Some(CAT_CURTAIN_PANELS),
            phase_created: None,
            phase_demolished: None,
        };
        // step 778: Phase Created @102 / Demolished @110 (−1 = none); 32-bit @78/@82 (steps 1335/1341)
        if l >= if bits32 { 90 } else { 118 } {
            let po = if bits32 { 78 } else { 102 };
            if po + 2 * w <= l {
                let (pc, pd) = (rd(bd, po), rd(bd, po + w));
                if phase_ids.contains(&pc) {
                    wall.phase_created = Some(pc);
                    wall.phase_demolished = Some(phase_ids.contains(&pd).then_some(pd));
                }
            }
        }
        out.push(wall);
    }
    out
}

/// Step 2237: wall attached to a roof, floor or ceiling ("attach top"): the wall body holds
/// `u32 1 | id target | u32 flag | ffffffff` (id i64, 32-bit i32), target of category −2000035,
/// −2000032 or −2000038; flag 1 = top (API WALL_TOP_IS_ATTACHED: rac_basic 16/16, dyn 1/1). First
/// hit per (wall, target).
pub(crate) fn wall_attachments(ch: &Chain) -> Vec<rvt_model::WallAttachment> {
    let targets: HashSet<i64> = ch.elements.iter().filter(|e| matches!(e.cat, -2000035 | -2000032 | -2000038)).map(|e| e.id as i64).collect();
    if targets.is_empty() {
        return vec![];
    }
    let bits32 = ch.bits == Some(32);
    let w = if bits32 { 4 } else { 8 };
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.cat == -2000011) {
        let b = ch.body(e);
        let mut seen = HashSet::new();
        for o in 4..b.len().saturating_sub(8 + w) {
            let v = if bits32 { i32_at(b, o) as i64 } else { i64_at(b, o) };
            if targets.contains(&v) && !seen.contains(&v) && u32_at(b, o - 4) == 1 && u32_at(b, o + w + 4) == 0xffff_ffff {
                seen.insert(v);
                out.push(rvt_model::WallAttachment { id: e.id, target: v, flag: u32_at(b, o + w) });
            }
        }
    }
    out
}

const TAG_WALL_SWEEP: u16 = 4520;

/// Step 719 (port, step 2271): wall sweeps and reveals (cat −2000181 / −2000182, tag 4520), bodies
/// of at least 400 B. Path = the first line/arc block in the last 200 B, else the first from @300
/// (blocks with header byte 04/24 and no denormal coordinates, step 1406). 64-bit: host = first
/// wall id, Distance f64 @136, type i64 @205 (@207 after `08 0d`, or when @205 is no id; after
/// `ff ff` @209/@213, step 1884) when it is an element id or −1. 32-bit (step 1535): host = first
/// i32 wall id, type = first i32 that is a sweep type (same category, other tag) or any element.
pub(crate) fn wall_sweeps(ch: &Chain, is_element: &dyn Fn(i64) -> bool) -> Vec<rvt_model::WallSweep> {
    let Some(tag) = vtag(ch.version, TAG_WALL_SWEEP) else { return vec![] };
    let walls: HashSet<i64> = ch.elements.iter().filter(|e| matches!(e.cat, -2000011 | -2003500)).map(|e| e.id as i64).collect();
    let ids: HashSet<i64> = ch.elements.iter().map(|e| e.id as i64).collect();
    let bits32 = ch.bits == Some(32);
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| matches!(e.cat, -2000181 | -2000182) && e.tag == Some(tag)) {
        let bd = ch.body(e);
        let l = bd.len();
        if l < 400 {
            continue;
        }
        let ok = |i: usize| i + 8 < l && matches!(bd[i + 8], 0x04 | 0x24);
        let line_or_arc = |i: usize| -> Option<Block> {
            if !ok(i) {
                return None;
            }
            nodenorm(block(bd, i)).filter(|b| matches!(b, Block::Line { .. } | Block::Arc { .. }))
        };
        let path = (l.saturating_sub(200)..l.saturating_sub(40)).find_map(line_or_arc).or_else(|| (300..l.saturating_sub(40)).find_map(line_or_arc)).map(|b| b.to_curve(0));
        let mut segments: Vec<rvt_model::Curve> = Vec::new();
        for i in 300..l.saturating_sub(40) {
            if let Some(c) = line_or_arc(i).map(|b| b.to_curve(0)) {
                if !segments.contains(&c) {
                    segments.push(c);
                }
            }
        }
        if bits32 {
            let tys: HashSet<i64> = ch.elements.iter().filter(|x| x.cat == e.cat && x.tag != e.tag).map(|x| x.id as i64).collect();
            let hosts: Vec<i64> = (0..l.saturating_sub(3)).map(|o| i32_at(bd, o) as i64).find(|q| walls.contains(q)).into_iter().collect();
            let ty = (0..l.saturating_sub(3)).map(|o| i32_at(bd, o) as i64).find(|q| tys.contains(q) || is_element(*q));
            out.push(rvt_model::WallSweep { id: e.id, cat: e.cat, sweep_type: ty, distance: None, hosts, path, segments });
            continue;
        }
        let valid = |t: i64| ids.contains(&t) || t == -1 || is_element(t);
        let hosts: Vec<i64> = (0..l.saturating_sub(7)).map(|o| i64_at(bd, o)).find(|q| walls.contains(q)).into_iter().collect();
        let mut t = i64_at(bd, if bd[205..207] == [0x08, 0x0d] { 207 } else { 205 });
        if !valid(t) && l >= 215 {
            let t207 = i64_at(bd, 207);
            if valid(t207) {
                t = t207;
            }
        }
        if !valid(t) && bd[205..207] == [0xff, 0xff] {
            if let Some(q) = [209usize, 213].iter().filter(|&&o| o + 8 <= l).map(|&o| i64_at(bd, o)).find(|&q| q != e.id as i64 && (ids.contains(&q) || is_element(q))) {
                t = q;
            }
        }
        out.push(rvt_model::WallSweep { id: e.id, cat: e.cat, sweep_type: valid(t).then_some(t), distance: Some(f64_at(bd, 136)), hosts, path, segments });
    }
    out
}
