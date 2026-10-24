//! Stairs (§33, steps 401–405, 722, 1328, 1453–1454, 1489–1490): riser, tread, levels, offsets,
//! height and runs of component stairs (tag 4012), with run geometry (steps 446–456).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::f64::consts::PI;

use rvt_core::{vtag, Chain, Element};
use rvt_model::{Curve, LegacyStair, PathSegment, RunGeometry, Stair};

use crate::curves::{block, curves_in, is_unit, Block};
use crate::util::{f64_at, find, i32_at, i64_at};

const TAG_STAIRS: u16 = 4012;
const TAG_RUN: u16 = 4039;
const TAG_MARKER: u16 = 1065;
const TAG_SKETCH: u16 = 4419;
const TAG_CURVE: u16 = 954;
const TAG_RUN_CACHE: u16 = 1396;
const CAT_LEVELS: i32 = -2000240;
const RUN_TAIL: [u8; 12] = [0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0x04, 0x00, 0x08, 0x00];
const BLOCK_MARK: [u8; 4] = [0x04, 0x00, 0x08, 0x01];

/// Sketch line kind by category (steps 446–447, API enum names).
fn line_kind(cat: i32) -> Option<&'static str> {
    match cat {
        -2000945 => Some("path"),
        -2000068 => Some("riser"),
        -2000067 => Some("boundary"),
        -2000069 => Some("run"),
        -2000070 => Some("landing_center"),
        _ => None,
    }
}

/// Steps 446–448: owner id -> {kind: curves} for sketches (tag 4419, owner @127, 32-bit @91)
/// with stairs/ramp sketch lines; the curve set is every curve element (tag 954).
fn sketch_lines(ch: &Chain) -> HashMap<i64, BTreeMap<&'static str, Vec<Curve>>> {
    let bits32 = ch.bits == Some(32);
    let byid: HashMap<u64, &Element> = ch.elements.iter().map(|e| (e.id, e)).collect();
    let ct = vtag(ch.version, TAG_CURVE);
    let cur: HashSet<u64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == ct).map(|e| e.id).collect();
    let st = vtag(ch.version, TAG_SKETCH);
    let mut out = HashMap::new();
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == st) {
        let sb = ch.body(e);
        if sb.len() < if bits32 { 95 } else { 135 } {
            continue;
        }
        let own = if bits32 { i32_at(sb, 91) as i64 } else { i64_at(sb, 127) };
        if own < 0 || !byid.contains_key(&(own as u64)) {
            continue;
        }
        let mut d: BTreeMap<&'static str, Vec<Curve>> = BTreeMap::new();
        let (lo, n) = e.body;
        for c in curves_in(bits32, &ch.data, &cur, lo, lo + n) {
            if let Some(k) = byid.get(&c.id).and_then(|x| line_kind(x.cat)) {
                d.entry(k).or_default().push(c);
            }
        }
        if !d.is_empty() {
            out.insert(own, d);
        }
    }
    out
}

fn v3(b: &[u8], o: usize) -> [f64; 3] {
    [f64_at(b, o), f64_at(b, o + 8), f64_at(b, o + 16)]
}

fn line_segment(p0: [f64; 3], p1: [f64; 3]) -> PathSegment {
    PathSegment {
        kind: "line",
        p0: Some(p0),
        p1: Some(p1),
        t0: None,
        t1: None,
        r: None,
        pitch: None,
        center: None,
        x: None,
        y: None,
        z: None,
        points: None,
    }
}

/// §33.5 (step 455): helix block (spiral run path) at `j`, 148 bytes: t0, t1 @+12, r @+28,
/// pitch @+36 (ft/rad), C @+44, X @+68, Y @+92, Z @+116; P(t) = C + r(cos t·X + sin t·Y) + s·t·Z.
fn helix_block(bd: &[u8], j: usize) -> Option<PathSegment> {
    if j + 148 > bd.len() || bd[j + 8..j + 12] != BLOCK_MARK {
        return None;
    }
    let (t0, t1, r, s) = (f64_at(bd, j + 12), f64_at(bd, j + 20), f64_at(bd, j + 28), f64_at(bd, j + 36));
    let (c, x, y, z) = (v3(bd, j + 44), v3(bd, j + 68), v3(bd, j + 92), v3(bd, j + 116));
    let finite = [t0, t1, r, s, c[0], c[1], c[2]].iter().all(|v| v.is_finite());
    if !(finite && r > 0.0 && t1 > t0 && is_unit(&x) && is_unit(&y) && is_unit(&z)) {
        return None;
    }
    let n = (((t1 - t0) / (PI / 24.0)) as i64).max(8);
    let points = (0..=n)
        .map(|i| {
            let t = t0 + (t1 - t0) * i as f64 / n as f64;
            let mut p = [0.0; 3];
            for k in 0..3 {
                p[k] = c[k] + r * (t.cos() * x[k] + t.sin() * y[k]) + s * t * z[k];
            }
            p
        })
        .collect();
    Some(PathSegment {
        kind: "helix",
        p0: None,
        p1: None,
        t0: Some(t0),
        t1: Some(t1),
        r: Some(r),
        pitch: Some(s),
        center: Some(c),
        x: Some(x),
        y: Some(y),
        z: Some(z),
        points: Some(points),
    })
}

/// §33.5 (steps 451–456): walk path from the cached run geometry: 84-byte line blocks right after
/// `u16 tag 1396 | 00×4 | ff×8` (32-bit ff×4), followed by the tail `ffffffff 00000000 04000800`
/// (not checked in 32-bit); the last matching list wins.
fn run_cached_path(ch: &Chain, bd: &[u8]) -> Vec<PathSegment> {
    let mut best = Vec::new();
    let Some(m) = vtag(ch.version, TAG_RUN_CACHE) else { return best };
    let bits32 = ch.bits == Some(32);
    let mut pre = m.to_le_bytes().to_vec();
    pre.extend_from_slice(&[0; 4]);
    pre.extend(std::iter::repeat(0xff).take(if bits32 { 4 } else { 8 }));
    let jo = if bits32 { 10 } else { 14 };
    let mut i = 0;
    while i <= bd.len() {
        let Some(k) = find(&bd[i..], &pre).map(|x| x + i) else { break };
        let mut j = k + jo;
        let mut segs = Vec::new();
        while j + 84 <= bd.len() && bd[j + 8..j + 12] == BLOCK_MARK {
            match block(bd, j) {
                Some(Block::Line { p0, p1, .. }) => {
                    segs.push(line_segment(p0, p1));
                    j += 84;
                }
                _ => {
                    // step 456: spiral run -> helix
                    if let Some(h) = helix_block(bd, j) {
                        j += 148;
                        segs.push(h);
                    }
                    break;
                }
            }
        }
        if !segs.is_empty() && (bits32 || bd.get(j..j + 12) == Some(&RUN_TAIL[..])) {
            best = segs;
        }
        i = k + 1;
    }
    best
}

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
    let run_body: HashMap<u64, &[u8]> = runs.iter().copied().collect();
    let skl = sketch_lines(ch);
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
    // step 448: path/treads/edges per run; step 453: component run -> path from the cache
    // (not in the 2014–2017 layout)
    let geometry = |ids: &[u64], cached: bool| -> BTreeMap<String, RunGeometry> {
        ids.iter()
            .map(|x| {
                let mut g = skl.get(&(*x as i64)).map(|l| RunGeometry { lines: l.clone(), path_cached: None }).unwrap_or_default();
                if cached && g.lines.get("path").map_or(true, |p| p.is_empty()) {
                    let cp = run_body.get(x).map(|b| run_cached_path(ch, b)).unwrap_or_default();
                    if !cp.is_empty() {
                        g.path_cached = Some(cp);
                    }
                }
                (x.to_string(), g)
            })
            .collect()
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
            let ids = run_ids(e.id);
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
                run_geometry: geometry(&ids, false),
                runs: ids,
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
        let ids = run_ids(e.id);
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
            run_geometry: geometry(&ids, true),
            runs: ids,
        });
    }
    out
}

/// Step 722: stairs runs (tag 4039): after the first stairs id S, the marker
/// `ffffffff k ffffffff k` (k = tag 1065; step 723) with `u32 n | n × (u32, u32)`, then f64 base,
/// f64 top, 16 zero bytes, f64 width (step 1362); fallback base/top @S+77/85, width @S+109 or
/// @marker+64.
pub(crate) fn stairs_runs(ch: &Chain) -> Vec<rvt_model::StairsRun> {
    let bits32 = ch.bits == Some(32);
    let st_tag = vtag(ch.version, TAG_STAIRS);
    let st: HashSet<i64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == st_tag).map(|e| e.id as i64).collect();
    let run_tag = vtag(ch.version, TAG_RUN);
    let marker: Option<Vec<u8>> = vtag(ch.version, TAG_MARKER).map(|k| {
        let mut s = vec![0xff; 4];
        s.extend_from_slice(&k.to_le_bytes());
        s.extend_from_slice(&[0xff; 4]);
        s.extend_from_slice(&k.to_le_bytes());
        s
    });
    let q = |b: &[u8], o: usize| if bits32 { i32_at(b, o) as i64 } else { i64_at(b, o) };
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == run_tag) {
        let bd = ch.body(e);
        let lim = bd.len().saturating_sub(8).min(20000);
        let Some(s) = (0..lim).find(|&o| st.contains(&q(bd, o))) else { continue };
        if s + 117 > bd.len() {
            continue;
        }
        let (mut b, mut t, mut w) = (f64_at(bd, s + 77), f64_at(bd, s + 85), f64_at(bd, s + 109));
        if let Some(m) = marker.as_ref().and_then(|mk| find(&bd[s..], mk).map(|x| x + s)) {
            if m + 72 <= bd.len() {
                w = f64_at(bd, m + 64);
                let n = crate::util::u32_at(bd, m + 12) as usize;
                if n < 1000 && m + 56 + 8 * n <= bd.len() && bd[m + 32 + 8 * n..m + 48 + 8 * n].iter().all(|&x| x == 0) {
                    w = f64_at(bd, m + 48 + 8 * n);
                    b = f64_at(bd, m + 16 + 8 * n);
                    t = f64_at(bd, m + 24 + 8 * n);
                }
            }
        }
        out.push(rvt_model::StairsRun { id: e.id, stairs: q(bd, s), width: w, base_elevation: b, top_elevation: t });
    }
    out
}

const TAG_LEGACY_STAIRS: u16 = 3398;
const TAG_LEGACY_STAIRS_TYPE: u16 = 4000;

/// Step 1455: old sketch-based stairs (2026 tag 3398, 2014 tag 2614; category −2000120). Base and
/// top level = the first two consecutive level references (@219/@227 in 64-bit), type = a reference
/// to the legacy StairsType (4000, step 1431), geometry = the sketch lines of its own Sketch.
/// `level_z` = the chain's own levels (like the reference).
pub(crate) fn legacy_stairs(ch: &Chain, level_z: &HashMap<i64, f64>) -> Vec<LegacyStair> {
    let Some(t) = vtag(ch.version, TAG_LEGACY_STAIRS) else { return vec![] };
    let xs: Vec<&Element> = ch.elements.iter().filter(|e| e.tag == Some(t) && e.cat == -2000120).collect();
    if xs.is_empty() {
        return vec![];
    }
    let bits32 = ch.bits == Some(32);
    let w = if bits32 { 4 } else { 8 };
    let lv: HashSet<i64> = ch.elements.iter().filter(|e| e.cat == CAT_LEVELS).map(|e| e.id as i64).collect();
    let tt = vtag(ch.version, TAG_LEGACY_STAIRS_TYPE);
    let types: HashSet<i64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == tt).map(|e| e.id as i64).collect();
    let skl = sketch_lines(ch);
    xs.into_iter()
        .map(|e| {
            let b = ch.body(e);
            let q = |o: usize| if bits32 { i32_at(b, o) as i64 } else { i64_at(b, o) };
            let k = (40..b.len().saturating_sub(2 * w).min(800)).find(|&o| lv.contains(&q(o)) && lv.contains(&q(o + w)));
            let ty = (40..b.len().saturating_sub(w).min(800)).map(q).find(|v| types.contains(v));
            let (bl, tl) = k.map_or((None, None), |k| (Some(q(k)), Some(q(k + w))));
            LegacyStair {
                id: e.id,
                base_level: bl,
                top_level: tl,
                r#type: ty,
                base_z: bl.and_then(|l| level_z.get(&l).copied()),
                top_z: tl.and_then(|l| level_z.get(&l).copied()),
                lines: skl.get(&(e.id as i64)).cloned().unwrap_or_default(),
            }
        })
        .collect()
}
