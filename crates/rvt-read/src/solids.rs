//! Family geometry: extrusion solids of a family chain (§8, §8.11; steps 488–491, 502–504,
//! 546–550, 923, 1352–1353, 2165).

use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;

use rvt_core::{tags::mapped_tag, vtag, Chain, Element};
use rvt_model::{Curve, ExtrusionSolid};

use crate::curves::{block, curves_in, is_unit, Block};
use crate::util::{f64_at, find, i32_at, i64_at, u32_at, u64_at};

const TAG_EXTRUSION: u16 = 1680;
const TAG_SKETCH: u16 = 4419;
const TAG_CURVE: u16 = 954;
const BIP_EXTRUSION_END: i64 = -1001801;
const BIP_EXTRUSION_START: i64 = -1001800;

/// Step 392: sketch plane frame `u8 01 | O | X | Y` (X, Y unit and perpendicular) with all
/// profile points in the plane; returns (O, X, Y, N = X × Y).
pub(crate) fn sketch_frame(sb: &[u8], pts: &[[f64; 3]]) -> Option<[[f64; 3]; 4]> {
    if pts.is_empty() {
        return None;
    }
    for o in 1..sb.len().saturating_sub(72) {
        if sb[o - 1] != 1 {
            continue;
        }
        let v: Vec<f64> = (0..9).map(|i| f64_at(sb, o + 8 * i)).collect();
        let (org, x, y) = ([v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]);
        if !(v.iter().all(|t| t.is_finite()) && is_unit(&x) && is_unit(&y) && (x[0] * y[0] + x[1] * y[1] + x[2] * y[2]).abs() < 1e-9) {
            continue;
        }
        let n = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
        if pts.iter().all(|p| ((p[0] - org[0]) * n[0] + (p[1] - org[1]) * n[1] + (p[2] - org[2]) * n[2]).abs() < 1e-6) {
            return Some([org, x, y, n]);
        }
    }
    None
}

/// Extrusion shape record: void flag and start/end along the sketch normal (§8; 32-bit step 1353).
struct Shape {
    id: u64,
    void: Option<bool>,
    range: Option<(f64, f64)>, // (start, end)
    bbox: Option<[f64; 6]>,
}

fn extrusion_shapes(ch: &Chain) -> Vec<Shape> {
    // step 1352: the shape class is recognised by its tag in this version
    let Some(t) = mapped_tag(ch.version, TAG_EXTRUSION) else { return vec![] };
    let bits32 = ch.bits == Some(32);
    if !bits32 && ch.bits != Some(64) {
        return vec![];
    }
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag == Some(t)) {
        let bd = ch.body(e);
        let l = bd.len();
        let (void, range) = if bits32 {
            let void = (l > 108).then(|| bd[107] == 1);
            let range = (l >= 138
                && u32_at(bd, 110) == 2
                && i32_at(bd, 122) as i64 == BIP_EXTRUSION_END
                && i32_at(bd, 134) as i64 == BIP_EXTRUSION_START)
                .then(|| (f64_at(bd, 126), f64_at(bd, 114)));
            (void, range)
        } else {
            let void = (l > 152).then(|| bd[151] == 1);
            let range = (l >= 190
                && u32_at(bd, 154) == 2
                && i64_at(bd, 166) == BIP_EXTRUSION_END
                && i64_at(bd, 182) == BIP_EXTRUSION_START)
                .then(|| (f64_at(bd, 174), f64_at(bd, 158)));
            (void, range)
        };
        out.push(Shape { id: e.id, void, range, bbox: e.bbox });
    }
    out
}

/// Steps 502–504: curve ids from the first complete curve table in the sketch body; per id the
/// first strictly valid arc block (1e-4 < r < 1e3, t1 > t0) in its own curve body. Arcs only.
fn own_body_curves(ch: &Chain, byid: &HashMap<u64, &Element>, sk: &Element, cur: &HashSet<u64>) -> Vec<Curve> {
    let sb = ch.body(sk);
    let mut ids: Vec<u64> = Vec::new();
    for o in 0..sb.len().saturating_sub(16) {
        let n = u32_at(sb, o) as usize;
        if (1..=60).contains(&n) && o + 4 + 12 * n <= sb.len() {
            let rows: Vec<u64> = (0..n).map(|i| u64_at(sb, o + 4 + 12 * i)).collect();
            if rows.iter().all(|r| cur.contains(r)) {
                ids = rows;
                break;
            }
        }
    }
    let mut out = Vec::new();
    for i in ids {
        let cb: &[u8] = byid.get(&i).map(|e| ch.body(e)).unwrap_or(&[]);
        let hit = (0..cb.len().saturating_sub(107)).find_map(|j| match block(cb, j) {
            Some(b @ Block::Arc { t0, t1, r, .. }) if t1 > t0 && 1e-4 < r && r < 1e3 => Some(b),
            _ => None,
        });
        let Some(hit) = hit else { return vec![] };
        out.push(hit.to_curve(i));
    }
    out
}

/// §8.11a (steps 546–550): flip the extrusion direction only if the shape bbox fits
/// profile − t·N and not profile + t·N (±0.01); arcs/ellipses sampled with 64 steps.
fn bbox_wants_flip(cs: &[Curve], n: &[f64; 3], t0: f64, t1: f64, bb: Option<[f64; 6]>) -> bool {
    let Some(bb) = bb else { return false };
    let mut p: Vec<[f64; 3]> = Vec::new();
    for c in cs {
        if let (Some(a), Some(b)) = (c.p0, c.p1) {
            p.push(a);
            p.push(b);
        } else if let (Some(cc), Some(x), Some(y)) = (c.center, c.x, c.y) {
            let (rx, ry) = match (c.rx, c.ry) {
                (Some(rx), Some(ry)) => (rx, ry),
                _ => (c.r.unwrap_or(0.0), c.r.unwrap_or(0.0)),
            };
            let (a0, a1) = if c.kind != "circle" { (c.t0.unwrap_or(0.0), c.t1.unwrap_or(2.0 * PI)) } else { (0.0, 2.0 * PI) };
            for i in 0..=64 {
                let t = a0 + (a1 - a0) * i as f64 / 64.0;
                let mut q = [0.0; 3];
                for k in 0..3 {
                    q[k] = cc[k] + rx * t.cos() * x[k] + ry * t.sin() * y[k];
                }
                p.push(q);
            }
        }
    }
    if p.is_empty() {
        return false;
    }
    let fits = |sg: f64| {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for q in &p {
            for t in [t0, t1] {
                for k in 0..3 {
                    let v = q[k] + sg * t * n[k];
                    lo[k] = lo[k].min(v);
                    hi[k] = hi[k].max(v);
                }
            }
        }
        (0..3).all(|k| (lo[k] - bb[k]).abs() < 0.01 && (hi[k] - bb[k + 3]).abs() < 0.01)
    };
    fits(-1.0) && !fits(1.0)
}

/// §8.11 (steps 488–491, 504, 2165): extrusions of one chain as solids: profile = curves in the
/// sketch owned by the extrusion (@127, 32-bit @91), frame from the sketch body, start/end along N.
pub(crate) fn extrusion_solids(ch: &Chain) -> Vec<ExtrusionSolid> {
    let bits32 = ch.bits == Some(32);
    let ct = vtag(ch.version, TAG_CURVE);
    let cur: HashSet<u64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == ct).map(|e| e.id).collect();
    let byid: HashMap<u64, &Element> = ch.elements.iter().map(|e| (e.id, e)).collect();
    let st = vtag(ch.version, TAG_SKETCH);
    let mut own: HashMap<i64, &Element> = HashMap::new();
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == st) {
        let sb = ch.body(e);
        if bits32 && sb.len() >= 95 {
            own.entry(i32_at(sb, 91) as i64).or_insert(e);
        } else if !bits32 && sb.len() >= 135 {
            own.entry(i64_at(sb, 127)).or_insert(e);
        }
    }
    let sketch_ids: HashSet<u64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == st).map(|e| e.id).collect();
    let host_mark: [u8; 16] = [0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    let mut out = Vec::new();
    for s in extrusion_shapes(ch) {
        let Some((start, end)) = s.range else { continue };
        let Some(sk) = own.get(&(s.id as i64)) else { continue };
        let (lo, n) = sk.body;
        let mut cs = curves_in(bits32, &ch.data, &cur, lo, lo + n);
        if cs.is_empty() && !bits32 {
            cs = own_body_curves(ch, &byid, sk, &cur);
        }
        let pts: Vec<[f64; 3]> = cs
            .iter()
            .flat_map(|c| match (c.p0, c.p1, c.center) {
                (Some(a), Some(b), _) => vec![a, b],
                (_, _, Some(cc)) => vec![cc],
                _ => vec![],
            })
            .collect();
        let Some([o, x, mut y, mut nn]) = sketch_frame(ch.body(sk), &pts) else { continue };
        if bbox_wants_flip(&cs, &nn, start, end, s.bbox) {
            y = [-y[0], -y[1], -y[2]];
            nn = [-nn[0], -nn[1], -nn[2]];
        }
        // step 923: host block of face-based templates = code 0x019f3d after `ffffffff | 0 | 1 | 0`
        let sb = byid.get(&s.id).map(|e| ch.body(e)).unwrap_or(&[]);
        let host_block = find(sb, &host_mark).filter(|&k| k + 20 <= sb.len() && u32_at(sb, k + 16) == 0x19F3D).map(|_| true);
        // step 804: sketch ids (i64, also in 32-bit) in the shape body from offset 60, first occurrence order
        let mut sketches: Vec<u64> = Vec::new();
        for o2 in 60..sb.len().saturating_sub(7) {
            let q = i64_at(sb, o2);
            if q >= 0 && sketch_ids.contains(&(q as u64)) && !sketches.contains(&(q as u64)) {
                sketches.push(q as u64);
            }
        }
        let sketches = (!sketches.is_empty()).then_some(sketches);
        out.push(ExtrusionSolid { id: s.id, void: s.void, sketch: sk.id, start, end, o, x, y, n: nn, profile: cs, host_block, sketches });
    }
    out
}

const TAG_SWEEP: u16 = 4226;

/// §8.12 (steps 496–497, 529, 2165): sweeps of one chain whose profile and path are sketches.
/// Profile = curves of the profile sketch (in 3D, already placed on the path), path = curves of the
/// path sketch, (X, Y, N) = path sketch plane. Pick-path sweeps (path = −1, step 565) and profile
/// families (profile = −1, step 1012) are not ported yet.
pub(crate) fn sweep_solids(ch: &Chain, chains: &[Chain]) -> Vec<rvt_model::SweepSolid> {
    let Some(t) = mapped_tag(ch.version, TAG_SWEEP) else { return vec![] };
    let bits32 = ch.bits == Some(32);
    if !bits32 && ch.bits != Some(64) {
        return vec![];
    }
    let ct = vtag(ch.version, TAG_CURVE);
    let cur: HashSet<u64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == ct).map(|e| e.id).collect();
    let byid: HashMap<u64, &Element> = ch.elements.iter().map(|e| (e.id, e)).collect();
    let cv = |sid: u64| -> Vec<Curve> {
        let Some(e) = byid.get(&sid) else { return vec![] };
        let (lo, n) = e.body;
        let cs = curves_in(bits32, &ch.data, &cur, lo, lo + n);
        if cs.is_empty() && !bits32 {
            own_body_curves(ch, &byid, e, &cur)
        } else {
            cs
        }
    };
    let mut out = Vec::new();
    // step 2227: computed once per chain
    let mut planes_cache: Option<HashMap<i64, [[f64; 3]; 4]>> = None;
    let mut kid_curves: HashMap<usize, Vec<Curve>> = HashMap::new();
    let mut kid_of: HashMap<u64, Option<usize>> = HashMap::new();
    for e in ch.elements.iter().filter(|e| e.tag == Some(t)) {
        let bd = ch.body(e);
        let l = bd.len();
        let (void, sketches) = if bits32 {
            ((l > 108).then(|| bd[105] == 1), (l >= 163).then(|| (i32_at(bd, 130) as i64 as u64, i32_at(bd, 159) as i64 as u64)))
        } else {
            ((l > 152).then(|| bd[149] == 1), (l >= 231).then(|| (u64_at(bd, 174), u64_at(bd, 223))))
        };
        let Some((ps, pa)) = sketches else { continue };
        let (mut prof, mut path) = (cv(ps), cv(pa));
        // step 565: pick path (path sketch = −1): curve blocks in the path list element
        let mut frame: Option<([f64; 3], [f64; 3], [f64; 3])> = None;
        if path.is_empty() && pa == u64::MAX {
            path = pick_path(ch, &byid, e.id, bits32);
            // plane from two non-parallel lines (reference order: first pair found)
            let ds: Vec<[f64; 3]> = path
                .iter()
                .filter_map(|c| match (c.p0, c.p1) {
                    (Some(a), Some(b)) => Some([b[0] - a[0], b[1] - a[1], b[2] - a[2]]),
                    _ => None,
                })
                .collect();
            if path.len() >= 2 {
                'outer: for a in &ds {
                    for b in &ds {
                        let nv = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                        let l = (nv[0] * nv[0] + nv[1] * nv[1] + nv[2] * nv[2]).sqrt();
                        if l > 1e-6 {
                            let la = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
                            let x = [a[0] / la, a[1] / la, a[2] / la];
                            let n = [nv[0] / l, nv[1] / l, nv[2] / l];
                            let y = [n[1] * x[2] - n[2] * x[1], n[2] * x[0] - n[0] * x[2], n[0] * x[1] - n[1] * x[0]];
                            frame = Some((x, y, n));
                            break 'outer;
                        }
                    }
                }
            }
        }
        // steps 1012, 1121: profile family (profile sketch = −1), placed at the path start
        if prof.is_empty() && ps == u64::MAX && !path.is_empty() {
            let loc = profile_family_curves(ch, chains, &byid, e.id, bits32, &mut kid_curves, &mut kid_of);
            let c0 = &path[0];
            let start = match (c0.p0, c0.p1, c0.center, c0.x, c0.y) {
                (Some(a), Some(b), ..) => Some((a, [b[0] - a[0], b[1] - a[1], b[2] - a[2]])),
                (_, _, Some(c), Some(x), Some(y)) => {
                    let t0 = c0.t0.unwrap_or(0.0);
                    let (rx, ry) = (c0.r.or(c0.rx).unwrap_or(0.0), c0.r.or(c0.ry).unwrap_or(0.0));
                    let p0 = [0, 1, 2].map(|i| c[i] + rx * t0.cos() * x[i] + ry * t0.sin() * y[i]);
                    let tg = [0, 1, 2].map(|i| -rx * t0.sin() * x[i] + ry * t0.cos() * y[i]);
                    Some((p0, tg))
                }
                _ => None,
            };
            if let (false, Some((p0, tg))) = (loc.is_empty(), start) {
                let l = (tg[0] * tg[0] + tg[1] * tg[1] + tg[2] * tg[2]).sqrt();
                let l = if l == 0.0 { 1.0 } else { l };
                let tg = [tg[0] / l, tg[1] / l, tg[2] / l];
                let nn = match frame {
                    Some((_, _, n)) => n,
                    None => match (c0.x, c0.y) {
                        (Some(x), Some(y)) => [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]],
                        _ => [0.0, 0.0, 1.0],
                    },
                };
                let mut vv = [tg[1] * nn[2] - tg[2] * nn[1], tg[2] * nn[0] - tg[0] * nn[2], tg[0] * nn[1] - tg[1] * nn[0]];
                let mut uu = nn;
                // step 1121: profile plane (normal ∥ tangent) referenced by the sweep body
                let planes = planes_cache.get_or_insert_with(|| profile_planes(ch));
                let sb = ch.body(e);
                let w = if bits32 { 4 } else { 8 };
                let pl = (60..sb.len().saturating_sub(w - 1)).find_map(|o| {
                    let q = if bits32 { i32_at(sb, o) as i64 } else { i64_at(sb, o) };
                    planes.get(&q).filter(|pl| (pl[3][0] * tg[0] + pl[3][1] * tg[1] + pl[3][2] * tg[2]).abs() > 0.999)
                });
                if let Some(pl) = pl {
                    uu = pl[1];
                    vv = pl[2];
                }
                prof = place_curves(&loc, p0, uu, vv);
            }
        }
        if prof.is_empty() || path.is_empty() {
            continue;
        }
        let pts: Vec<[f64; 3]> = path
            .iter()
            .flat_map(|c| match (c.p0, c.p1, c.center) {
                (Some(a), Some(b), _) => vec![a, b],
                (_, _, Some(cc)) => vec![cc],
                _ => vec![],
            })
            .collect();
        let fr = frame.or_else(|| byid.get(&pa).and_then(|sk| sketch_frame(ch.body(sk), &pts)).map(|[_, x, y, n]| (x, y, n)));
        let (x, y, n) = fr.unwrap_or(([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]));
        out.push(rvt_model::SweepSolid { id: e.id, void, profile: prof, path, x, y, n });
    }
    out
}

/// §8.12b (steps 564–565, 1123, 2166): curves of a pick-path sweep. 64-bit: in the path list
/// element (sweep @231, tag 3204; otherwise every element naming the sweep) the §6 blocks after
/// `ff×8 | u32 | 00×4 | 04 00`; 32-bit: path element i32 @163, blocks at marker
/// `ffffffff 04 00 .. 01` − 4. The fallback through curve elements (steps 1110, 2166) is not ported.
fn pick_path(ch: &Chain, byid: &HashMap<u64, &Element>, sweep: u64, bits32: bool) -> Vec<Curve> {
    let mut out = Vec::new();
    let sb: &[u8] = byid.get(&sweep).map(|e| ch.body(e)).unwrap_or(&[]);
    if bits32 {
        let Some(pe) = (sb.len() >= 167).then(|| i32_at(sb, 163) as i64).filter(|v| *v >= 0).and_then(|v| byid.get(&(v as u64))) else {
            return out;
        };
        let pb = ch.body(pe);
        let mk = [0xffu8, 0xff, 0xff, 0xff, 0x04, 0x00];
        let mut k = 0;
        while let Some(f) = find(&pb[k..], &mk).map(|x| x + k) {
            if f >= 4 && f + 7 < pb.len() && pb[f + 7] == 1 {
                if let Some(b) = block(pb, f - 4) {
                    out.push(b.to_curve(0));
                }
            }
            k = f + 1;
        }
        return out;
    }
    let key = sweep.to_le_bytes();
    let pt = vtag(ch.version, 3204);
    let pe = (sb.len() >= 239)
        .then(|| i64_at(sb, 231))
        .filter(|v| *v >= 0)
        .and_then(|v| byid.get(&(v as u64)))
        .filter(|e| e.tag.is_some() && e.tag == pt);
    let cand: Vec<&Element> = match pe {
        Some(e) => vec![*e],
        None => ch.elements.iter().collect(),
    };
    for e in cand {
        if e.id == sweep {
            continue;
        }
        let b = ch.body(e);
        if find(b, &key).is_none() {
            continue;
        }
        for o in 0..b.len().saturating_sub(20) {
            if b[o..o + 8] == [0xff; 8] && b[o + 12..o + 16] == [0; 4] && b[o + 16] == 4 && b[o + 17] == 0 {
                if let Some(bl) = block(b, o + 8) {
                    out.push(bl.to_curve(0));
                }
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    out
}

/// Step 815/1117: plane of a sweep profile plane element (tag 3338): first unit, perpendicular
/// pair X, Y from @84 with a sane origin before it; (origin, x, y, normal).
fn profile_planes(ch: &Chain) -> HashMap<i64, [[f64; 3]; 4]> {
    let t = vtag(ch.version, 3338);
    let mut out = HashMap::new();
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == t) {
        let b = ch.body(e);
        let l = b.len();
        if l < 48 {
            continue;
        }
        for o in 84..l.saturating_sub(48) {
            let (x, y) = (v3(b, o), v3(b, o + 24));
            let sq = |v: &[f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] - 1.0).abs() < 1e-9;
            if x.iter().chain(y.iter()).all(|v| v.is_finite()) && sq(&x) && sq(&y) && (x[0] * y[0] + x[1] * y[1] + x[2] * y[2]).abs() < 1e-9 {
                let org = v3(b, o - 24);
                if !org.iter().all(|v| v.is_finite() && v.abs() < 1e6) {
                    continue;
                }
                let n = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
                out.insert(e.id as i64, [org, x, y, n]);
                break;
            }
        }
    }
    out
}

/// Steps 1011–1012, 2166: profile of a sweep with a profile family (FamilySymbol @182, 32-bit
/// @134): the family's own chain (its key in the family body) and its curve elements, in the
/// profile family's local XY plane.
fn profile_family_curves(
    ch: &Chain,
    chains: &[Chain],
    byid: &HashMap<u64, &Element>,
    sweep: u64,
    bits32: bool,
    kid_curves: &mut HashMap<usize, Vec<Curve>>,
    kid_of: &mut HashMap<u64, Option<usize>>,
) -> Vec<Curve> {
    let Some(e) = byid.get(&sweep) else { return vec![] };
    let b = ch.body(e);
    let off = if bits32 { 134 } else { 182 };
    if b.len() < off + 8 {
        return vec![];
    }
    let pack = |v: i64| if bits32 { (v as i32).to_le_bytes().to_vec() } else { v.to_le_bytes().to_vec() };
    let prof = if bits32 { i32_at(b, off) as i64 } else { i64_at(b, off) };
    let Some(pe) = (prof >= 0).then(|| byid.get(&(prof as u64))).flatten() else { return vec![] };
    let sb = ch.body(pe);
    let ft = vtag(ch.version, 1967);
    // step 2227: one scan of the symbol body for the first position of every family id
    let fam_by_id: HashMap<i64, &Element> = ch.elements.iter().filter(|f| f.tag.is_some() && f.tag == ft).map(|f| (f.id as i64, f)).collect();
    let w = if bits32 { 4 } else { 8 };
    let mut first: HashMap<i64, usize> = HashMap::new();
    for o in 0..(sb.len() + 1).saturating_sub(w) {
        let v = if bits32 { i32_at(sb, o) as i64 } else { i64_at(sb, o) };
        if fam_by_id.contains_key(&v) {
            first.entry(v).or_insert(o);
        }
    }
    let mut fams: Vec<(usize, &Element)> = first.into_iter().map(|(v, pos)| (pos, fam_by_id[&v])).collect();
    fams.sort_by_key(|(pos, _)| *pos);
    let _ = pack;
    for (_, f) in fams {
        let (lo, ln) = f.body;
        // step 2227: the family -> chain match is cached (many sweeps share a profile family)
        let kid = *kid_of
            .entry(f.id)
            .or_insert_with(|| chains.iter().position(|x| !std::ptr::eq(x, ch) && x.key.is_some_and(|k| find(&ch.data[lo..lo + ln], &k).is_some())));
        if let Some(ki) = kid {
            return kid_curves
                .entry(ki)
                .or_insert_with(|| crate::curve_elements::curve_elements(&chains[ki]).into_iter().map(|(_, c)| c).collect())
                .clone();
        }
    }
    vec![]
}

/// Step 1012: curves from a local XY plane to 3D: P + x·U + y·V (z ignored); spline points are
/// left as they are, like the reference.
fn place_curves(curves: &[Curve], p: [f64; 3], u: [f64; 3], v: [f64; 3]) -> Vec<Curve> {
    let tp = |q: [f64; 3]| [p[0] + q[0] * u[0] + q[1] * v[0], p[1] + q[0] * u[1] + q[1] * v[1], p[2] + q[0] * u[2] + q[1] * v[2]];
    let td = |d: [f64; 3]| [d[0] * u[0] + d[1] * v[0], d[0] * u[1] + d[1] * v[1], d[0] * u[2] + d[1] * v[2]];
    curves
        .iter()
        .map(|c| {
            let mut c = c.clone();
            c.p0 = c.p0.map(tp);
            c.p1 = c.p1.map(tp);
            c.center = c.center.map(tp);
            c.x = c.x.map(td);
            c.y = c.y.map(td);
            c
        })
        .collect()
}

fn v3(b: &[u8], o: usize) -> [f64; 3] {
    [f64_at(b, o), f64_at(b, o + 8), f64_at(b, o + 16)]
}

const TAG_REVOLUTION: u16 = 3760;
const TAG_BLEND: u16 = 626;

/// Value pair `f64 | id BIP a | f64 | id BIP b` (64-bit i64 BIPs, 32-bit i32) anywhere in the body:
/// returns (first f64, second f64) of the first match (§8).
fn bip_pair(bd: &[u8], bits32: bool, a: i64, b: i64) -> Option<(f64, f64)> {
    let l = bd.len();
    if bits32 {
        (0..l.saturating_sub(24)).find(|&o| i32_at(bd, o + 8) as i64 == a && i32_at(bd, o + 20) as i64 == b).map(|o| (f64_at(bd, o), f64_at(bd, o + 12)))
    } else {
        (0..l.saturating_sub(36)).find(|&o| i64_at(bd, o + 8) == a && i64_at(bd, o + 24) == b).map(|o| (f64_at(bd, o), f64_at(bd, o + 16)))
    }
}

fn curves_of(ch: &Chain, byid: &HashMap<u64, &Element>, cur: &HashSet<u64>, sid: u64, bits32: bool) -> Vec<Curve> {
    byid.get(&sid).map(|e| curves_in(bits32, &ch.data, cur, e.body.0, e.body.0 + e.body.1)).unwrap_or_default()
}

/// §8.13 (steps 499–500, 2165): blends; the first two sketch ids after the void byte (64-bit u64
/// from @152, 32-bit i32 from @108), offsets from the BIP pair −1001805/−1001804.
pub(crate) fn blend_solids(ch: &Chain) -> Vec<rvt_model::BlendSolid> {
    let Some(t) = mapped_tag(ch.version, TAG_BLEND) else { return vec![] };
    let bits32 = ch.bits == Some(32);
    if !bits32 && ch.bits != Some(64) {
        return vec![];
    }
    let ct = vtag(ch.version, TAG_CURVE);
    let cur: HashSet<u64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == ct).map(|e| e.id).collect();
    let byid: HashMap<u64, &Element> = ch.elements.iter().map(|e| (e.id, e)).collect();
    let st = vtag(ch.version, TAG_SKETCH);
    let is_sketch = |v: u64| byid.get(&v).is_some_and(|e| e.tag.is_some() && e.tag == st);
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag == Some(t)) {
        let bd = ch.body(e);
        let l = bd.len();
        let void = if bits32 { (l > 108).then(|| bd[107] == 1) } else { (l > 152).then(|| bd[151] == 1) };
        let sk: Vec<u64> = if bits32 {
            (108..l.saturating_sub(4).min(300)).map(|o| i32_at(bd, o) as i64 as u64).filter(|v| is_sketch(*v)).collect()
        } else {
            (152..l.saturating_sub(8).min(400)).map(|o| u64_at(bd, o)).filter(|v| is_sketch(*v)).collect()
        };
        if sk.len() < 2 {
            continue;
        }
        let (bs, ts) = (sk[0], sk[1..].iter().copied().find(|x| *x != sk[0]));
        let Some(ts) = ts else { continue };
        let (top_off, bot_off) = bip_pair(bd, bits32, -1001805, -1001804).unwrap_or((0.0, 0.0));
        let (bot, top) = (curves_of(ch, &byid, &cur, bs, bits32), curves_of(ch, &byid, &cur, ts, bits32));
        let pts: Vec<[f64; 3]> = bot
            .iter()
            .flat_map(|c| match (c.p0, c.p1, c.center) {
                (Some(a), Some(b), _) => vec![a, b],
                (_, _, Some(cc)) => vec![cc],
                _ => vec![],
            })
            .collect();
        if bot.is_empty() || top.is_empty() || pts.is_empty() {
            continue;
        }
        let Some([_, _, _, n]) = byid.get(&bs).and_then(|sk| sketch_frame(ch.body(sk), &pts)) else { continue };
        out.push(rvt_model::BlendSolid { id: e.id, void, bottom: bot, top, n, bottom_offset: bot_off, top_offset: top_off });
    }
    out
}

/// End points of a curve (lines, arcs, ellipses).
fn curve_ends(c: &Curve) -> Vec<[f64; 3]> {
    if let (Some(a), Some(b)) = (c.p0, c.p1) {
        return vec![a, b];
    }
    let (Some(cc), Some(x), Some(y), Some(t0), Some(t1)) = (c.center, c.x, c.y, c.t0, c.t1) else { return vec![] };
    let (rx, ry) = match c.kind {
        "arc" => (c.r.unwrap_or(0.0), c.r.unwrap_or(0.0)),
        "ellipse" => (c.rx.unwrap_or(0.0), c.ry.unwrap_or(0.0)),
        _ => return vec![],
    };
    [t0, t1].iter().map(|t| [0, 1, 2].map(|k| cc[k] + rx * t.cos() * x[k] + ry * t.sin() * y[k])).collect()
}

/// Python `tuple(round(v, 5) for v in p)` as a hashable key (−0.0 equals 0.0).
fn rkey(p: &[f64; 3]) -> [u64; 3] {
    p.map(|v| {
        let r: f64 = format!("{v:.5}").parse().unwrap_or(v);
        if r == 0.0 { 0u64 } else { r.to_bits() }
    })
}

/// §8.14 (steps 506–509, 602–603, 606, 2165): revolutions. Axis = the line whose two ends are free
/// (node degree 1); else the only line whose removal leaves only even degrees; else the one
/// doubled profile edge (the copy with the larger id). Axis direction so that (n × (c − A))·N < 0.
pub(crate) fn revolve_solids(ch: &Chain) -> Vec<rvt_model::RevolveSolid> {
    let Some(t) = mapped_tag(ch.version, TAG_REVOLUTION) else { return vec![] };
    let bits32 = ch.bits == Some(32);
    if !bits32 && ch.bits != Some(64) {
        return vec![];
    }
    let ct = vtag(ch.version, TAG_CURVE);
    let cur: HashSet<u64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == ct).map(|e| e.id).collect();
    let byid: HashMap<u64, &Element> = ch.elements.iter().map(|e| (e.id, e)).collect();
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag == Some(t)) {
        let bd = ch.body(e);
        let l = bd.len();
        let (void, sketch) = if bits32 {
            ((l > 108).then(|| bd[107] == 1), (l >= 112).then(|| i32_at(bd, 108) as i64 as u64))
        } else {
            ((l > 152).then(|| bd[151] == 1), (l >= 160).then(|| u64_at(bd, 152)))
        };
        let Some(sketch) = sketch.filter(|s| byid.contains_key(s)) else { continue };
        let (end_a, start_a) = bip_pair(bd, bits32, -1001803, -1001802).unwrap_or((2.0 * PI, 0.0));
        let cs = curves_of(ch, &byid, &cur, sketch, bits32);
        let mut deg: HashMap<[u64; 3], usize> = HashMap::new();
        for c in &cs {
            for p in curve_ends(c) {
                *deg.entry(rkey(&p)).or_default() += 1;
            }
        }
        let is_line = |c: &Curve| c.p0.is_some() && c.p1.is_some();
        let mut axes: Vec<usize> = (0..cs.len())
            .filter(|&i| is_line(&cs[i]) && [cs[i].p0.unwrap(), cs[i].p1.unwrap()].iter().all(|p| deg.get(&rkey(p)) == Some(&1)))
            .collect();
        if axes.len() != 1 {
            axes = (0..cs.len())
                .filter(|&i| {
                    if !is_line(&cs[i]) {
                        return false;
                    }
                    let mut d2: HashMap<[u64; 3], usize> = HashMap::new();
                    for (j, c) in cs.iter().enumerate() {
                        if j != i {
                            for p in curve_ends(c) {
                                *d2.entry(rkey(&p)).or_default() += 1;
                            }
                        }
                    }
                    d2.values().all(|n| n % 2 == 0)
                })
                .collect();
        }
        if axes.len() != 1 {
            let mut dup: Vec<([[u64; 3]; 2], Vec<usize>)> = Vec::new();
            for (i, c) in cs.iter().enumerate() {
                if let (Some(a), Some(b)) = (c.p0, c.p1) {
                    let mut k = [rkey(&a), rkey(&b)];
                    k.sort();
                    match dup.iter_mut().find(|(kk, _)| *kk == k) {
                        Some((_, v)) => v.push(i),
                        None => dup.push((k, vec![i])),
                    }
                }
            }
            let pairs: Vec<&Vec<usize>> = dup.iter().map(|(_, v)| v).filter(|v| v.len() == 2).collect();
            if pairs.len() == 1 {
                // max by id (first of equal ids, like Python max)
                let v = pairs[0];
                let best = if cs[v[1]].id > cs[v[0]].id { v[1] } else { v[0] };
                axes = vec![best];
            }
        }
        if axes.len() != 1 {
            continue;
        }
        let ai = axes[0];
        let (mut a, mut b) = (cs[ai].p0.unwrap(), cs[ai].p1.unwrap());
        let mut pts: Vec<[f64; 3]> = cs.iter().enumerate().filter(|(i, _)| *i != ai).flat_map(|(_, c)| curve_ends(c)).collect();
        for (i, c) in cs.iter().enumerate() {
            if i != ai && matches!(c.kind, "arc" | "ellipse") {
                if let (Some(t0), Some(t1)) = (c.t0, c.t1) {
                    let mut m = c.clone();
                    m.t0 = Some((t0 + t1) / 2.0);
                    if let Some(p) = curve_ends(&m).first() {
                        pts.push(*p);
                    }
                }
            }
        }
        if !pts.is_empty() {
            let mut all = pts.clone();
            all.push(a);
            all.push(b);
            if let Some([_, _, _, nn]) = sketch_frame(ch.body(byid[&sketch]), &all) {
                let n_ = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let cnt = pts.len() as f64;
                let c_ = [0, 1, 2].map(|k| pts.iter().map(|q| q[k]).sum::<f64>() / cnt - a[k]);
                let x_ = [n_[1] * c_[2] - n_[2] * c_[1], n_[2] * c_[0] - n_[0] * c_[2], n_[0] * c_[1] - n_[1] * c_[0]];
                if x_[0] * nn[0] + x_[1] * nn[1] + x_[2] * nn[2] > 0.0 {
                    std::mem::swap(&mut a, &mut b);
                }
            }
        }
        let profile: Vec<Curve> = cs.iter().enumerate().filter(|(i, _)| *i != ai).map(|(_, c)| c.clone()).collect();
        out.push(rvt_model::RevolveSolid { id: e.id, void, axis: [a, b], profile, start_angle: start_a, end_angle: end_a });
    }
    out
}

