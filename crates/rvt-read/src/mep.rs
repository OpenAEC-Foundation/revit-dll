//! MEP: systems (§15.5) and straight curve segments (§15.1).

use std::collections::HashSet;

use rvt_core::{vtag, Chain, Element};
use rvt_model::{MepSystem, Segment, FlexCurve, MepCurve};

use crate::util::{f64_at, i32_at, i64_at, u32_at, u64_at};

pub(crate) const TAG_SEGMENT: u16 = 3442;
const TAG_SYSTEM: u16 = 3481;

/// Printable ASCII runs of at least 4 characters (Python `re.findall(r"[ -~]{4,}", text)`).
fn ascii_runs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if (' '..='~').contains(&c) {
            cur.push(c);
        } else {
            if cur.chars().count() >= 4 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
        }
    }
    if cur.chars().count() >= 4 {
        out.push(cur);
    }
    out
}

/// UTF-16LE decode with replacement characters (Python `decode("utf-16-le", "replace")`).
fn utf16_lossy(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let mut s = String::from_utf16_lossy(&units);
    if b.len() % 2 == 1 {
        s.push('\u{FFFD}');
    }
    s
}

/// §15.5: MechanicalSystem elements of a chain → name (first readable string that contains " Air "
/// or starts with "Mechanical", at even or odd byte alignment).
pub(crate) fn system_names(ch: &Chain) -> Vec<MepSystem> {
    let Some(t) = vtag(ch.version, TAG_SYSTEM) else { return vec![] };
    ch.elements
        .iter()
        .filter(|e| e.tag == Some(t))
        .map(|e| {
            let bd = ch.body(e);
            let mut runs = ascii_runs(&utf16_lossy(bd));
            if !bd.is_empty() {
                runs.extend(ascii_runs(&utf16_lossy(&bd[1..])));
            }
            MepSystem { id: e.id, name: runs.into_iter().find(|x| x.contains(" Air ") || x.starts_with("Mechanical")) }
        })
        .collect()
}

/// The single system id referenced in a body (step 1818: skip the 16-byte header); `None` if zero or
/// several are referenced.
pub(crate) fn system_of(ch: &Chain, e: &Element, systems: &HashSet<u64>) -> Option<u64> {
    let bd = ch.body(e);
    let bits32 = ch.bits == Some(32);
    let w = if bits32 { 4 } else { 8 };
    let mut found = HashSet::new();
    for o in 16..bd.len().saturating_sub(w) {
        let v = if bits32 { u32_at(bd, o) as u64 } else { u64_at(bd, o) };
        if systems.contains(&v) {
            found.insert(v);
        }
    }
    if found.len() == 1 {
        found.into_iter().next()
    } else {
        None
    }
}

/// Section marker `(u32 3 | ff×4) | u16 tag A | ff×4 | u16 tag A+1` with a plausible width and height
/// (steps 240, 1467, 1469); regex matches do not overlap.
fn section_marker(bd: &[u8]) -> Option<usize> {
    let mut p = 0usize;
    while p + 12 <= bd.len() {
        let pre = &bd[p..p + 4];
        let is_match = (pre == [3, 0, 0, 0] || pre == [0xff; 4]) && bd[p + 6..p + 10] == [0xff; 4];
        if !is_match {
            p += 1;
            continue;
        }
        let a = u16::from_le_bytes([bd[p + 4], bd[p + 5]]);
        let b = u16::from_le_bytes([bd[p + 10], bd[p + 11]]);
        if b as u32 == a as u32 + 1 && p + 28 <= bd.len() {
            let (w, h) = (f64_at(bd, p + 12), f64_at(bd, p + 20));
            if 0.0 < w && w < 100.0 && 0.0 < h && h < 100.0 {
                return Some(p);
            }
        }
        p += 12;
    }
    None
}

/// §15.1: location line (first bounded line block within bbox ± 2 ft) + cross section.
fn segment(ch: &Chain, e: &Element) -> Option<Segment> {
    let bd = ch.body(e);
    let bits32 = ch.bits == Some(32);
    // step 1330: 32-bit records have no section A bbox; no range filter
    let bb = match (e.bbox, bits32) {
        (Some(b), _) => b,
        (None, true) => [-1e12, -1e12, -1e12, 1e12, 1e12, 1e12],
        (None, false) => return None,
    };
    for k in 12..bd.len().saturating_sub(64) {
        if bd[k - 1] != 1 {
            continue;
        }
        let v: Vec<f64> = (0..8).map(|i| f64_at(bd, k + 8 * i)).collect();
        let (t0, t1, o, d) = (v[0], v[1], [v[2], v[3], v[4]], [v[5], v[6], v[7]]);
        if !([t0, t1, o[0], o[1], o[2]].iter().all(|x| x.is_finite()) && (d[0] * d[0] + d[1] * d[1] + d[2] * d[2] - 1.0).abs() < 1e-9 && t1 > t0) {
            continue;
        }
        let p0 = [o[0] + t0 * d[0], o[1] + t0 * d[1], o[2] + t0 * d[2]];
        let p1 = [o[0] + t1 * d[0], o[1] + t1 * d[1], o[2] + t1 * d[2]];
        if !(0..3).all(|i| [p0, p1].iter().all(|p| bb[i] - 2.0 <= p[i] && p[i] <= bb[i + 3] + 2.0)) {
            continue;
        }
        let m = section_marker(bd)?;
        let (w, h) = (f64_at(bd, m + 12), f64_at(bd, m + 20));
        let len = ((p1[0] - p0[0]).powi(2) + (p1[1] - p0[1]).powi(2) + (p1[2] - p0[2]).powi(2)).sqrt();
        let dir = [0, 1, 2].map(|i| (p1[i] - p0[i]) / len);
        let ext = [0, 1, 2].map(|i| bb[i + 3] - bb[i]);
        let cyl = [0, 1, 2].map(|i| (p1[i] - p0[i]).abs() + w * (1.0 - dir[i] * dir[i]).max(0.0).sqrt());
        let round = (w - h).abs() < 1e-9 && (0..3).all(|i| (cyl[i] - ext[i]).abs() <= 0.012 * w + 1e-6);
        let ty = if bits32 {
            // step 1330: 32-bit MEPCurve type = i32 @178
            if bd.len() >= 182 { i32_at(bd, 178) as i64 } else { 0 }
        } else {
            // step 1132: type @204 (sometimes @206) followed by ff ff ff ff
            [204usize, 206]
                .into_iter()
                .find(|&o| o + 12 <= bd.len() && bd[o + 8..o + 12] == [0xff; 4] && 0 < i64_at(bd, o) && i64_at(bd, o) < 1 << 40)
                .map(|o| i64_at(bd, o))
                .unwrap_or(0)
        };
        return Some(Segment { id: e.id, p0, p1, width: w, height: h, round, segment_type: (ty > 0).then_some(ty), system: None, level: None });
    }
    None
}

/// Steps 2006/2011: level of ducts/pipes/flex = first Level ref in [O−4, O+6] with O = 70 (64-bit),
/// 186 (32-bit) or 62 (Revit 2023, step 2159).
pub(crate) fn curve_level(ch: &Chain, bd: &[u8], levels: &HashSet<i64>) -> Option<i64> {
    let bits32 = ch.bits == Some(32);
    let (o0, w) = if bits32 { (if ch.version == Some(2023) { 62 } else { 186 }, 4) } else { (70, 8) };
    (o0 - 4..(o0 + 7).min((bd.len() + 1).saturating_sub(w)))
        .map(|o| if bits32 { i32_at(bd, o) as i64 } else { i64_at(bd, o) })
        .find(|v| levels.contains(v))
}

/// Segments (tag 3442) of a chain with their system and level.
pub(crate) fn segments(ch: &Chain, systems: &HashSet<u64>) -> Vec<Segment> {
    let Some(t) = vtag(ch.version, TAG_SEGMENT) else { return vec![] };
    let levels: HashSet<i64> = ch.elements.iter().filter(|e| e.cat == -2000240).map(|e| e.id as i64).collect();
    ch.elements
        .iter()
        .filter(|e| e.tag == Some(t))
        .filter_map(|e| {
            segment(ch, e).map(|s| Segment { system: system_of(ch, e, systems), level: curve_level(ch, ch.body(e), &levels), ..s })
        })
        .collect()
}

const TAG_FLEX_DUCT: u16 = 3470;
const TAG_FLEX_PIPE: u16 = 3473;
const TAG_PIPE: u16 = 3503;
const TAG_CABLE_TRAY: u16 = 694;
const TAG_CONDUIT: u16 = 3427;

fn d3(b: &[u8], o: usize) -> [f64; 3] {
    [f64_at(b, o), f64_at(b, o + 8), f64_at(b, o + 16)]
}

/// Flex ducts (tag 3470, §15.6; steps 250, 1130, 2032, 2033, 2034, 2159): points `u32 n @294 | n × 3
/// f64` (rme 2026 header variant +2 B: n @296), tangents @242/@266, FlexDuctType i64 @204. 32-bit:
/// n @252, points @256, tangents @200/@224, type i32 @178 (2023: geometry 4 B earlier; 2014: 1 B).
pub(crate) fn flex_ducts(ch: &Chain, systems: &HashSet<u64>) -> Vec<FlexCurve> {
    let Some(t) = vtag(ch.version, TAG_FLEX_DUCT) else { return vec![] };
    let levels: HashSet<i64> = ch.elements.iter().filter(|e| e.cat == -2000240).map(|e| e.id as i64).collect();
    let bits32 = ch.bits == Some(32);
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag == Some(t) && e.bbox.is_some()) {
        let bd = ch.body(e);
        let n_at = |o: usize| if bd.len() >= o + 4 { u32_at(bd, o) as usize } else { 0 };
        let r = if bits32 && e.cat == -2008020 {
            let mut d: isize = if ch.version == Some(2023) { -4 } else { 0 };
            let at = |o: usize, d: isize| (o as isize + d) as usize;
            let mut n = if bd.len() > 256 { n_at(at(252, d)) } else { 0 };
            if !((2..=64).contains(&n) && at(256, d) + 24 * n <= bd.len()) && bd.len() > 255 {
                n = n_at(251);
                d = -1;
            }
            ((2..=64).contains(&n) && at(256, d) + 24 * n <= bd.len()).then(|| FlexCurve {
                id: e.id,
                kind: None,
                points: (0..n).map(|k| d3(bd, at(256, d) + 24 * k)).collect(),
                start_tangent: d3(bd, at(200, d)),
                end_tangent: d3(bd, at(224, d)),
                system: system_of(ch, e, systems),
                flex_type: Some(i32_at(bd, 178) as i64).filter(|v| *v > 0),
                level: None,
            })
        } else {
            let mut n = n_at(294);
            let mut sh = 0;
            if !(2..=64).contains(&n) && bd.len() > 300 {
                let n2 = n_at(296);
                if (2..=64).contains(&n2) && 300 + 24 * n2 <= bd.len() {
                    n = n2;
                    sh = 2;
                }
            }
            ((2..=64).contains(&n) && 298 + sh + 24 * n <= bd.len()).then(|| FlexCurve {
                id: e.id,
                kind: None,
                points: (0..n).map(|k| d3(bd, 298 + sh + 24 * k)).collect(),
                start_tangent: d3(bd, 242 + sh),
                end_tangent: d3(bd, 266 + sh),
                system: system_of(ch, e, systems),
                flex_type: (bd.len() >= 212 + sh).then(|| i64_at(bd, 204 + sh)).filter(|v| *v > 0),
                level: None,
            })
        };
        if let Some(mut r) = r {
            r.level = curve_level(ch, bd, &levels);
            out.push(r);
        }
    }
    out
}

/// Step 702: the other MEP curves with the segment layout (§15.1): pipe 3503, cable tray 694,
/// conduit 3427 (step 1156: else by category, when the record has a box and is not a type), and
/// flex pipes (3473) with points like flex ducts.
pub(crate) fn mep_curves(ch: &Chain, systems: &HashSet<u64>) -> Vec<MepCurve> {
    let levels: HashSet<i64> = ch.elements.iter().filter(|e| e.cat == -2000240).map(|e| e.id as i64).collect();
    let kinds = [(vtag(ch.version, TAG_PIPE), "pipe"), (vtag(ch.version, TAG_CABLE_TRAY), "cable_tray"), (vtag(ch.version, TAG_CONDUIT), "conduit")];
    let types: HashSet<Option<u16>> = [3520u16, 3418, 3428].iter().map(|t| vtag(ch.version, *t)).collect();
    let flex_pipe = vtag(ch.version, TAG_FLEX_PIPE);
    let mut out = Vec::new();
    for e in ch.elements.iter() {
        let mut kind = kinds.iter().find(|(t, _)| t.is_some() && *t == e.tag).map(|(_, k)| *k);
        if kind.is_none() && e.bbox.is_some() && !types.contains(&e.tag) {
            kind = match e.cat {
                -2008044 => Some("pipe"),
                -2008130 => Some("cable_tray"),
                -2008132 => Some("conduit"),
                _ => None,
            };
        }
        if let Some(k) = kind {
            if let Some(s) = segment(ch, e) {
                out.push(MepCurve {
                    id: e.id,
                    kind: k,
                    p0: Some(s.p0),
                    p1: Some(s.p1),
                    width: Some(s.width),
                    height: Some(s.height),
                    round: Some(s.round),
                    segment_type: s.segment_type,
                    points: None,
                    start_tangent: None,
                    end_tangent: None,
                    system: system_of(ch, e, systems),
                    level: curve_level(ch, ch.body(e), &levels),
                });
                continue;
            }
        }
        if flex_pipe.is_some() && e.tag == flex_pipe && e.bbox.is_some() {
            let bd = ch.body(e);
            let n = if bd.len() > 298 { u32_at(bd, 294) as usize } else { 0 };
            if (2..=64).contains(&n) && 298 + 24 * n <= bd.len() {
                out.push(MepCurve {
                    id: e.id,
                    kind: "flex_pipe",
                    p0: None,
                    p1: None,
                    width: None,
                    height: None,
                    round: None,
                    segment_type: None,
                    points: Some((0..n).map(|k| d3(bd, 298 + 24 * k)).collect()),
                    start_tangent: Some(d3(bd, 242)),
                    end_tangent: Some(d3(bd, 266)),
                    system: system_of(ch, e, systems),
                    level: curve_level(ch, bd, &levels),
                });
            }
        }
    }
    out
}
