//! Curve blocks (§6): line (84 bytes), arc (117 bytes), ellipse (124 bytes).

use crate::util::f64_at;

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Line { p0: [f64; 3], p1: [f64; 3] },
    Arc { t0: f64, t1: f64, x: [f64; 3], y: [f64; 3], r: f64, c: [f64; 3] },
    /// Recognised so that it is not misread as a line; walls ignore it.
    Ellipse,
}

fn v3(b: &[u8], o: usize) -> [f64; 3] {
    [f64_at(b, o), f64_at(b, o + 8), f64_at(b, o + 16)]
}

pub fn is_unit(v: &[f64; 3]) -> bool {
    ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() - 1.0).abs() < 1e-9
}

fn dot(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// §6 (v0.2 §5.2–5.5): arc or line at position `i`; `None` if there is no valid block.
pub fn block(b: &[u8], i: usize) -> Option<Block> {
    if i + 108 <= b.len() {
        // step 504: C ends @I+108
        let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
        let (x, y) = (v3(b, i + 28), v3(b, i + 52));
        let r = f64_at(b, i + 76);
        let c = v3(b, i + 84);
        if r.is_finite() && r > 0.0 && is_unit(&x) && is_unit(&y) && dot(&x, &y).abs() < 1e-9 {
            return Some(Block::Arc { t0, t1, x, y, r, c });
        }
    }
    if i + 124 <= b.len() && b[i + 8..i + 12] == [0x04, 0x00, 0x08, 0x01] {
        // step 352: ellipse
        let (c, x, y) = (v3(b, i + 28), v3(b, i + 52), v3(b, i + 76));
        let (rx, ry) = (f64_at(b, i + 100), f64_at(b, i + 108));
        if is_unit(&x) && is_unit(&y) && dot(&x, &y).abs() < 1e-9 && rx > 1e-6 && ry > 1e-6 && c.iter().all(|v| v.is_finite()) {
            return Some(Block::Ellipse);
        }
        if i + 116 <= b.len() {
            // step 1922: second ellipse layout
            let (rx, ry) = (f64_at(b, i + 28), f64_at(b, i + 36));
            let (c, x, y) = (v3(b, i + 44), v3(b, i + 68), v3(b, i + 92));
            if is_unit(&x)
                && is_unit(&y)
                && dot(&x, &y).abs() < 1e-9
                && 1e-6 < rx
                && rx < 1e5
                && 1e-6 < ry
                && ry < 1e5
                && c.iter().all(|v| v.is_finite() && v.abs() < 1e6)
            {
                return Some(Block::Ellipse);
            }
        }
    }
    if i + 84 <= b.len() {
        let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
        let (o, d) = (v3(b, i + 28), v3(b, i + 52));
        if o.iter().chain(d.iter()).chain([t0, t1].iter()).all(|v| v.is_finite()) && is_unit(&d) && t1 > t0 {
            let p = |t: f64| [o[0] + t * d[0], o[1] + t * d[1], o[2] + t * d[2]];
            return Some(Block::Line { p0: p(t0), p1: p(t1) });
        }
    }
    None
}

/// Step 1394: a line block with denormal coordinates is noise.
pub fn nodenorm(b: Option<Block>) -> Option<Block> {
    match &b {
        Some(Block::Line { p0, p1 }) if p0.iter().chain(p1.iter()).any(|v| v.abs() > 0.0 && v.abs() < 1e-300) => None,
        _ => b,
    }
}

/// Point on an arc at parameter t.
pub fn arc_point(c: &[f64; 3], r: f64, x: &[f64; 3], y: &[f64; 3], t: f64) -> [f64; 3] {
    let (ct, st) = (t.cos(), t.sin());
    [0, 1, 2].map(|k| c[k] + r * (ct * x[k] + st * y[k]))
}
