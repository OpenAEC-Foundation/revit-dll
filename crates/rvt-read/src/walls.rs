//! Walls (§29).

use std::collections::{BTreeMap, HashMap, HashSet};

use rvt_core::{vtag, Chain};
use rvt_model::{ArcCurve, Wall};

use crate::curves::{arc_point, block, nodenorm, Block};
use crate::util::{f64_at, find, i32_at, i64_at};

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
            Block::Line { p0, p1 } => Some(Loc::Line { p0, p1 }),
            Block::Arc { t0, t1, x, y, r, c } => Some(Loc::Arc {
                p0: arc_point(&c, r, &x, &y, t0),
                p1: arc_point(&c, r, &x, &y, t1),
                curve: ArcCurve { kind: "arc", center: c, r, x, y, t0, t1 },
            }),
            Block::Ellipse => None,
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
                if let Some(Block::Line { p0, p1 }) = block(bd, kk - 4) {
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
        let in_place = fam.iter().any(|(f, fb)| find(bd, &pack(*f as i64)).is_some() && find(fb, &own).is_some());
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
