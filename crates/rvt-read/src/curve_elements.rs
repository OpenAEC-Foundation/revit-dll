//! Curve elements (step 669–673, 1357, 1394): the curve of every CurveElement (Model/Detail line,
//! arc, ellipse, spline; tag 954). Only the geometry is ported (no line style).

use std::collections::HashSet;

use rvt_core::{vtag, Chain};
use rvt_model::Curve;

use crate::curves::{block, is_unit, nodenorm};
use crate::util::{f64_at, i32_at, i64_at, u32_at};

const TAG_CURVE: u16 = 954;
const TAG_STYLE: u16 = 2252;

fn v3(b: &[u8], o: usize) -> [f64; 3] {
    [f64_at(b, o), f64_at(b, o + 8), f64_at(b, o + 16)]
}

fn unit_sq(v: &[f64; 3]) -> bool {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] - 1.0).abs() < 1e-9
}

/// Step 670: hermite or NURBS spline after the curve marker `04 00 08` (@I+8).
fn spline_block(b: &[u8], i: usize) -> Option<Curve> {
    if i + 41 > b.len() || b[i + 8..i + 11] != [0x04, 0x00, 0x08] {
        return None;
    }
    let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
    let n = u32_at(b, i + 29) as usize;
    if (2..=4096).contains(&n) && i + 41 + 56 * n - 8 <= b.len() && f64_at(b, i + 33) == t0 {
        let (mut p, mut t, mut q) = (Vec::new(), Vec::new(), vec![t0]);
        for k in 0..n {
            let o = i + 41 + 56 * k;
            p.push(v3(b, o));
            t.push(v3(b, o + 24));
            if k < n - 1 {
                q.push(f64_at(b, o + 48));
            }
        }
        let finite = p.iter().chain(t.iter()).flatten().chain(q.iter()).all(|v| v.is_finite());
        if finite && q.windows(2).all(|w| w[0] < w[1]) && (q[q.len() - 1] - t1).abs() < 1e-9 {
            return Some(Curve { kind: "hermite", t0: Some(t0), t1: Some(t1), points: Some(p), tangents: Some(t), params: Some(q), ..Default::default() });
        }
    }
    let (deg, n) = (u32_at(b, i + 28) as usize, u32_at(b, i + 34) as usize);
    let nk = n + deg + 1;
    let ke = b.len() as i64 - 4;
    let ks = ke - 8 * nk as i64;
    if (1..=15).contains(&deg) && (2..=4096).contains(&n) && (i + 38 + 32 * n) as i64 <= ks && ks >= 0 {
        let ks = ks as usize;
        let (mut w, mut p) = (Vec::new(), Vec::new());
        for k in 0..n {
            let o = i + 38 + 32 * k;
            w.push(f64_at(b, o));
            p.push(v3(b, o + 8));
        }
        let knots: Vec<f64> = (0..nk).map(|k| f64_at(b, ks + 8 * k)).collect();
        if w.iter().all(|x| *x > 0.0 && x.is_finite())
            && p.iter().flatten().all(|v| v.is_finite())
            && knots.windows(2).all(|x| x[0] <= x[1])
            && knots[0] == t0
            && knots[nk - 1] == t1
        {
            return Some(Curve { kind: "nurbs", t0: Some(t0), t1: Some(t1), degree: Some(deg as u32), points: Some(p), weights: Some(w), knots: Some(knots), ..Default::default() });
        }
    }
    None
}

/// (element id, curve) for every curve element of the chain, in element order.
pub(crate) fn curve_elements(ch: &Chain) -> Vec<(u64, Curve)> {
    let bits32 = ch.bits == Some(32);
    let t = vtag(ch.version, TAG_CURVE);
    let tg = vtag(ch.version, TAG_STYLE);
    let ids: HashSet<i64> = ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == tg).map(|e| e.id as i64).collect();
    // step 1357: 32-bit style = i32 at block+4; 64-bit i64 before the block
    let (qw, qoff, qs, qe): (usize, i64, usize, i64) = if bits32 { (4, -4, 4, 12) } else { (8, 8, 0, 0) };
    let q_at = |b: &[u8], o: usize| -> i64 { if bits32 { i32_at(b, o) as i64 } else { i64_at(b, o) } };
    let ok_prefix = |b: &[u8], o: usize| -> bool {
        let q = q_at(b, o);
        (q == -1 || ids.contains(&q)) && (!bits32 || b[o + 4] == 4)
    };
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == t) {
        let b = ch.body(e);
        let len = b.len() as i64;
        let range = |lo: i64, hi: i64| (lo.max(0)..hi.max(lo.max(0))).map(|v| v as usize);
        let mut cv: Option<Curve> = None;
        // step 670: splines and full ellipses first
        for o in range(qs as i64, len - 49) {
            if o + qw > b.len() || !ok_prefix(b, o) {
                continue;
            }
            let i = (o as i64 + qoff) as usize;
            cv = spline_block(b, i);
            if cv.is_none() && i + 124 <= b.len() && b[i + 8..i + 12] == [0x04, 0x00, 0x08, 0x00] {
                let (c, x, y) = (v3(b, i + 28), v3(b, i + 52), v3(b, i + 76));
                let (rx, ry) = (f64_at(b, i + 100), f64_at(b, i + 108));
                if unit_sq(&x) && unit_sq(&y) && (x[0] * y[0] + x[1] * y[1] + x[2] * y[2]).abs() < 1e-9 && rx > 1e-9 && ry > 1e-9 && c.iter().all(|v| v.is_finite()) {
                    cv = Some(Curve { kind: "ellipse", center: Some(c), x: Some(x), y: Some(y), rx: Some(rx), ry: Some(ry), bounded: Some(0), ..Default::default() });
                }
            }
            if cv.is_some() {
                break;
            }
        }
        if cv.is_none() {
            for o in range(qs as i64, len - 84 + qe) {
                if o + qw > b.len() || !ok_prefix(b, o) {
                    continue;
                }
                let i = (o as i64 + qoff) as usize;
                cv = nodenorm(block(b, i)).map(|bl| bl.to_curve(0));
                if cv.is_none() && i + 76 <= b.len() && b.len() < i + 84 {
                    let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
                    let (org, d) = (v3(b, i + 28), v3(b, i + 52));
                    if org.iter().chain(d.iter()).chain([t0, t1].iter()).all(|v| v.is_finite()) && is_unit(&d) && t1 > t0 {
                        let p = |t: f64| [org[0] + t * d[0], org[1] + t * d[1], org[2] + t * d[2]];
                        cv = Some(Curve { kind: "line", p0: Some(p(t0)), p1: Some(p(t1)), ..Default::default() });
                    }
                }
                if cv.is_some() {
                    break;
                }
            }
        }
        if cv.is_none() {
            // a line block up to the body end (80 B)
            for o in range(qs.max((len - 84 + qe).max(0) as usize) as i64, len - 84 + 8 + qe) {
                if o + qw > b.len() || !ok_prefix(b, o) {
                    continue;
                }
                let i = (o as i64 + qoff) as usize;
                if i + 76 <= b.len() {
                    let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
                    let (org, d) = (v3(b, i + 28), v3(b, i + 52));
                    if org.iter().chain(d.iter()).chain([t0, t1].iter()).all(|v| v.is_finite()) && is_unit(&d) && t1 > t0 {
                        let p = |t: f64| [org[0] + t * d[0], org[1] + t * d[1], org[2] + t * d[2]];
                        cv = Some(Curve { kind: "line", p0: Some(p(t0)), p1: Some(p(t1)), ..Default::default() });
                        break;
                    }
                }
            }
        }
        if cv.is_none() {
            // step 671: zero-length line
            for o in range(qs as i64, len - 83) {
                if o + qw > b.len() || !ok_prefix(b, o) {
                    continue;
                }
                let i = (o as i64 + qoff) as usize;
                if i + 76 > b.len() {
                    continue;
                }
                let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
                let (org, d) = (v3(b, i + 28), v3(b, i + 52));
                if b[i + 8..i + 12] == [0x04, 0x00, 0x08, 0x01] && t0 == t1 && is_unit(&d) && org.iter().chain(d.iter()).chain([t0].iter()).all(|v| v.is_finite()) {
                    let p = [org[0] + t0 * d[0], org[1] + t0 * d[1], org[2] + t0 * d[2]];
                    cv = Some(Curve { kind: "line", p0: Some(p), p1: Some(p), degenerate: Some(true), ..Default::default() });
                    break;
                }
            }
        }
        if let Some(c) = cv {
            out.push((e.id, c));
        }
    }
    out
}
