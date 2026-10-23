//! Curve blocks (§6) and curve tables (§7).

use std::collections::HashSet;
use std::f64::consts::PI;

use rvt_model::Curve;

use crate::util::{f64_at, u32_at, u64_at};

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Line { p0: [f64; 3], p1: [f64; 3], degenerate: bool },
    Arc { t0: f64, t1: f64, x: [f64; 3], y: [f64; 3], r: f64, c: [f64; 3], bounded: u8 },
    Ellipse { t0: f64, t1: f64, c: [f64; 3], x: [f64; 3], y: [f64; 3], rx: f64, ry: f64, layout2: bool },
}

impl Block {
    /// Bytes the block occupies (line 84, arc 117, ellipse 124).
    pub fn len(&self) -> usize {
        match self {
            Block::Line { .. } => 84,
            Block::Arc { .. } => 117,
            Block::Ellipse { .. } => 124,
        }
    }

    pub fn to_curve(&self, id: u64) -> Curve {
        match *self {
            Block::Line { p0, p1, degenerate } => {
                Curve { id, kind: "line", p0: Some(p0), p1: Some(p1), degenerate: degenerate.then_some(true), ..Default::default() }
            }
            Block::Arc { t0, t1, x, y, r, c, bounded } => Curve {
                id,
                kind: "arc",
                t0: Some(t0),
                t1: Some(t1),
                x: Some(x),
                y: Some(y),
                r: Some(r),
                center: Some(c),
                bounded: Some(bounded),
                ..Default::default()
            },
            Block::Ellipse { t0, t1, c, x, y, rx, ry, layout2 } => Curve {
                id,
                kind: "ellipse",
                t0: Some(t0),
                t1: Some(t1),
                center: Some(c),
                x: Some(x),
                y: Some(y),
                rx: Some(rx),
                ry: Some(ry),
                layout: layout2.then_some(2),
                ..Default::default()
            },
        }
    }
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

/// §6 (v0.2 §5.2–5.5): arc, ellipse or line at position `i`; `None` if there is no valid block.
pub fn block(b: &[u8], i: usize) -> Option<Block> {
    if i + 108 <= b.len() {
        // step 504: C ends @I+108
        let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
        let (x, y) = (v3(b, i + 28), v3(b, i + 52));
        let r = f64_at(b, i + 76);
        let c = v3(b, i + 84);
        if r.is_finite() && r > 0.0 && is_unit(&x) && is_unit(&y) && dot(&x, &y).abs() < 1e-9 {
            return Some(Block::Arc { t0, t1, x, y, r, c, bounded: b[i + 11] });
        }
    }
    if i + 124 <= b.len() && b[i + 8..i + 12] == [0x04, 0x00, 0x08, 0x01] {
        // step 352: ellipse
        let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
        let (c, x, y) = (v3(b, i + 28), v3(b, i + 52), v3(b, i + 76));
        let (rx, ry) = (f64_at(b, i + 100), f64_at(b, i + 108));
        if is_unit(&x) && is_unit(&y) && dot(&x, &y).abs() < 1e-9 && rx > 1e-6 && ry > 1e-6 && c.iter().all(|v| v.is_finite()) {
            return Some(Block::Ellipse { t0, t1, c, x, y, rx, ry, layout2: false });
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
                return Some(Block::Ellipse { t0, t1, c, x, y, rx, ry, layout2: true });
            }
        }
    }
    if i + 84 <= b.len() {
        let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
        let (o, d) = (v3(b, i + 28), v3(b, i + 52));
        if o.iter().chain(d.iter()).chain([t0, t1].iter()).all(|v| v.is_finite()) && is_unit(&d) && t1 > t0 {
            let p = |t: f64| [o[0] + t * d[0], o[1] + t * d[1], o[2] + t * d[2]];
            return Some(Block::Line { p0: p(t0), p1: p(t1), degenerate: false });
        }
    }
    None
}

/// Steps 466–467: zero-length line block (t0 = t1), only inside a curve table.
fn zero_line(b: &[u8], i: usize) -> Option<Block> {
    if i + 84 > b.len() {
        return None;
    }
    let (t0, t1) = (f64_at(b, i + 12), f64_at(b, i + 20));
    let (o, d) = (v3(b, i + 28), v3(b, i + 52));
    if o.iter().chain(d.iter()).chain([t0, t1].iter()).all(|v| v.is_finite()) && is_unit(&d) && t1 == t0 {
        let p = [o[0] + t0 * d[0], o[1] + t0 * d[1], o[2] + t0 * d[2]];
        return Some(Block::Line { p0: p, p1: p, degenerate: true });
    }
    None
}

/// Step 1394: a line block with denormal coordinates is noise.
pub fn nodenorm(b: Option<Block>) -> Option<Block> {
    match &b {
        Some(Block::Line { p0, p1, .. }) if p0.iter().chain(p1.iter()).any(|v| v.abs() > 0.0 && v.abs() < 1e-300) => None,
        _ => b,
    }
}

/// Point on an arc at parameter t.
pub fn arc_point(c: &[f64; 3], r: f64, x: &[f64; 3], y: &[f64; 3], t: f64) -> [f64; 3] {
    let (ct, st) = (t.cos(), t.sin());
    [0, 1, 2].map(|k| c[k] + r * (ct * x[k] + st * y[k]))
}

/// Two half arcs [0, π] + [π, 2π] of the same circle.
fn full_circle(a: &Block, c: &Block) -> bool {
    let (Block::Arc { t0: a0, t1: a1, x: ax, y: ay, r: ar, c: ac, .. }, Block::Arc { t0: c0, t1: c1, x: cx, y: cy, r: cr, c: cc, .. }) = (a, c)
    else {
        return false;
    };
    let va = ac.iter().chain(ax).chain(ay).chain([ar]);
    let vc = cc.iter().chain(cx).chain(cy).chain([cr]);
    va.zip(vc).all(|(p, q)| (p - q).abs() < 1e-9)
        && a0.abs() < 1e-9
        && (a1 - PI).abs() < 1e-9
        && (c0 - PI).abs() < 1e-9
        && (c1 - 2.0 * PI).abs() < 1e-9
}

/// Python `b.find(sub, start, end)`: the match must lie fully inside `[start, end)`.
pub fn find_in(b: &[u8], sub: &[u8], start: usize, end: usize) -> Option<usize> {
    let end = end.min(b.len());
    if start >= end || end - start < sub.len() {
        return None;
    }
    b[start..end].windows(sub.len()).position(|w| w == sub).map(|o| o + start)
}

/// Emits the curves of one table: a full circle or full ellipse for n = 1 split in two halves,
/// otherwise one curve per row.
fn emit(out: &mut Vec<Curve>, rows: &[(u64, u32)], blocks: &[(usize, Block)], may_merge: bool) {
    let n = rows.len();
    if may_merge && blocks.len() > 1 && full_circle(&blocks[0].1, &blocks[1].1) {
        if let Block::Arc { x, y, r, c, .. } = blocks[0].1 {
            out.push(Curve { id: rows[0].0, kind: "circle", center: Some(c), r: Some(r), x: Some(x), y: Some(y), ..Default::default() });
        }
        return;
    }
    if may_merge && blocks.len() > 1 {
        if let (Block::Ellipse { t0, c, x, y, rx, ry, .. }, Block::Ellipse { t1, .. }) = (&blocks[0].1, &blocks[1].1) {
            // step 352: full ellipse = two halves
            out.push(Curve {
                id: rows[0].0,
                kind: "ellipse",
                center: Some(*c),
                x: Some(*x),
                y: Some(*y),
                rx: Some(*rx),
                ry: Some(*ry),
                t0: Some(*t0),
                t1: Some(*t1),
                ..Default::default()
            });
            return;
        }
    }
    for ((id, _), (_, bl)) in rows.iter().zip(&blocks[..n]) {
        out.push(bl.to_curve(*id));
    }
}

const FRAME: [u8; 12] = [0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];

/// §7 (64-bit): table `u32 n | n × (u64 id, u32 k)` (k increasing), then the blocks after a frame.
pub fn curves_in64(b: &[u8], cur: &HashSet<u64>, lo: usize, hi: usize, seen: &mut HashSet<usize>) -> Vec<Curve> {
    let mut out = Vec::new();
    let mut valid_until = 0usize;
    let mut p = lo;
    let lim = hi.min(b.len().saturating_sub(16));
    while p < lim {
        let n = u32_at(b, p) as usize;
        if (1..=500).contains(&n) && p >= valid_until && p + 4 + 12 * n <= b.len() && cur.contains(&u64_at(b, p + 4)) {
            let rows: Vec<(u64, u32)> = (0..n).map(|i| (u64_at(b, p + 4 + 12 * i), u32_at(b, p + 12 + 12 * i))).collect();
            if rows.iter().all(|r| cur.contains(&r.0)) && rows.windows(2).all(|w| w[0].1 < w[1].1) {
                let end = p + 4 + 12 * n;
                let mut f = find_in(b, &FRAME, end, end + 4096);
                let run = |mut i: usize, f: usize| {
                    let mut blocks = Vec::new();
                    // step 437: a longer ff run before the first block
                    while i + 4 <= b.len() && b[i..i + 4] == [0xff; 4] && i < f + 64 {
                        i += 4;
                    }
                    for j in 0..=n {
                        let bl = block(b, i).or_else(|| if j < n { zero_line(b, i) } else { None });
                        let Some(bl) = bl else { break };
                        let l = bl.len();
                        blocks.push((i, bl));
                        i += l;
                    }
                    blocks
                };
                let mut blocks = f.map(|f| run(f + 12, f)).unwrap_or_default();
                if blocks.len() < n {
                    // steps 530/568: first ff×8 + block (u32 index, u32 0, 04 00)
                    let mut g = find_in(b, &[0xff; 8], end, end + 4096);
                    while let Some(gg) = g {
                        if f.is_some_and(|f| gg >= f) {
                            break;
                        }
                        if gg + 18 <= b.len() && b[gg + 12..gg + 16] == [0; 4] && b[gg + 16..gg + 18] == [0x04, 0x00] {
                            let alt = run(gg + 8, gg);
                            if alt.len() >= n {
                                blocks = alt;
                                f = Some(gg);
                                break;
                            }
                        }
                        g = find_in(b, &[0xff; 8], gg + 1, end + 4096);
                    }
                }
                if f.is_some() && blocks.len() >= n {
                    if seen.insert(blocks[0].0) {
                        emit(&mut out, &rows, &blocks, n == 1);
                    }
                    valid_until = end;
                    p = end;
                    continue;
                }
            }
        }
        p += 1;
    }
    out
}

/// Step 1324: §7 in 32-bit: table `u32 n | n × (u32 id, u32 k)`, then per block
/// `u32 idx | u32 0 | ff×4 | 04 00 xx 01 …` (block = marker − 4).
pub fn curves_in32(b: &[u8], cur: &HashSet<u64>, lo: usize, hi: usize, seen: &mut HashSet<usize>) -> Vec<Curve> {
    const MK: [u8; 6] = [0xff, 0xff, 0xff, 0xff, 0x04, 0x00];
    let mut out = Vec::new();
    let mut p = lo;
    let lim = hi.min(b.len().saturating_sub(12));
    while p < lim {
        let n = u32_at(b, p) as usize;
        if (1..=500).contains(&n) && p + 4 + 8 * n <= b.len() && cur.contains(&(u32_at(b, p + 4) as u64)) {
            let rows: Vec<(u64, u32)> = (0..n).map(|i| (u32_at(b, p + 4 + 8 * i) as u64, u32_at(b, p + 8 + 8 * i))).collect();
            if rows.iter().all(|r| cur.contains(&r.0)) && rows.windows(2).all(|w| w[0].1 < w[1].1) {
                let end = p + 4 + 8 * n;
                let nb = if n == 1 { 2 } else { n }; // step 2165: n = 1 may be a full circle in two halves
                let mut blocks: Vec<(usize, Block)> = Vec::new();
                let mut k = find_in(b, &MK, end, end + 4096);
                while let Some(kk) = k {
                    if blocks.len() >= nb {
                        break;
                    }
                    if kk + 7 < b.len() && b[kk + 7] == 1 && kk >= 4 {
                        if let Some(bl) = block(b, kk - 4) {
                            let next = kk - 4 + bl.len();
                            blocks.push((kk - 4, bl));
                            k = find_in(b, &MK, next, kk + 4096);
                            continue;
                        }
                    }
                    k = find_in(b, &MK, kk + 1, kk + 4096);
                }
                if blocks.len() >= n && !seen.contains(&blocks[0].0) {
                    seen.insert(blocks[0].0);
                    emit(&mut out, &rows, &blocks, nb > n);
                    p = end;
                    continue;
                }
            }
        }
        p += 1;
    }
    out
}

/// Curves of a table region, choosing the layout by bit width.
pub fn curves_in(bits32: bool, b: &[u8], cur: &HashSet<u64>, lo: usize, hi: usize) -> Vec<Curve> {
    let mut seen = HashSet::new();
    if bits32 {
        curves_in32(b, cur, lo, hi, &mut seen)
    } else {
        curves_in64(b, cur, lo, hi, &mut seen)
    }
}
