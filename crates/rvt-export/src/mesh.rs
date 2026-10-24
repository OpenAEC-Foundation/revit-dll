//! Triangle meshes of a document for viewers: one batch per category, in metres, Y up
//! (three.js convention: view = (x, z, −y) × 0.3048).
//!
//! Walls, floors, ceilings, roofs and MEP segments are built from their reader data. Family
//! instances are shown as their stored bounding box until family geometry is ported.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use rvt_model::{BlendSolid, Curve, ExtrusionSolid, Instance, RevolveSolid, Segment, Stair, SweepSolid};
use rvt_read::Document;

const FT: f64 = 0.3048;

/// One mesh batch: flat-shaded triangles of one category.
#[derive(Debug, Clone, Default)]
pub struct Batch {
    pub name: String,
    pub color: [f32; 3],
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
}

/// All batches plus bounds (view coordinates).
#[derive(Debug, Clone, Default)]
pub struct Scene {
    pub batches: Vec<Batch>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

type P3 = [f64; 3];

fn view(p: P3) -> [f32; 3] {
    [(p[0] * FT) as f32, (p[2] * FT) as f32, (-p[1] * FT) as f32]
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul(a: P3, s: f64) -> P3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn cross(a: P3, b: P3) -> P3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: P3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn unit(a: P3) -> Option<P3> {
    let n = norm(a);
    (n > 1e-12 && n.is_finite()).then(|| mul(a, 1.0 / n))
}

impl Batch {
    fn new(name: &str, color: [f32; 3]) -> Self {
        Batch { name: name.to_string(), color, ..Default::default() }
    }

    /// Triangle in model coordinates (feet, Z up); the normal follows the winding.
    fn tri(&mut self, a: P3, b: P3, c: P3) {
        let (va, vb, vc) = (view(a), view(b), view(c));
        let u = [vb[0] - va[0], vb[1] - va[1], vb[2] - va[2]];
        let w = [vc[0] - va[0], vc[1] - va[1], vc[2] - va[2]];
        let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if !(l > 1e-12) {
            return;
        }
        let n = [n[0] / l, n[1] / l, n[2] / l];
        let base = (self.positions.len() / 3) as u32;
        for v in [va, vb, vc] {
            self.positions.extend_from_slice(&v);
            self.normals.extend_from_slice(&n);
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    fn quad(&mut self, a: P3, b: P3, c: P3, d: P3) {
        self.tri(a, b, c);
        self.tri(a, c, d);
    }

    /// Box from 8 corners: bottom a b c d (counter-clockwise seen from above), top e f g h.
    fn hexa(&mut self, bottom: [P3; 4], top: [P3; 4]) {
        let [a, b, c, d] = bottom;
        let [e, f, g, h] = top;
        self.quad(a, d, c, b);
        self.quad(e, f, g, h);
        self.quad(a, b, f, e);
        self.quad(b, c, g, f);
        self.quad(c, d, h, g);
        self.quad(d, a, e, h);
    }

    fn aabb(&mut self, bb: [f64; 6]) {
        let (x0, y0, z0, x1, y1, z1) = (bb[0], bb[1], bb[2], bb[3], bb[4], bb[5]);
        self.hexa(
            [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]],
            [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        );
    }

    /// Vertical prism over plan loops (first = outer, rest = holes) from z0 to z1.
    fn prism(&mut self, loops: &[Vec<[f64; 2]>], z0: f64, z1: f64) {
        if loops.is_empty() || loops[0].len() < 3 || !(z1 > z0) {
            return;
        }
        let mut flat = Vec::new();
        let mut holes = Vec::new();
        for (i, l) in loops.iter().enumerate() {
            if i > 0 {
                holes.push(flat.len() / 2);
            }
            for p in l {
                flat.extend_from_slice(p);
            }
        }
        let pts: Vec<[f64; 2]> = loops.iter().flatten().copied().collect();
        if let Ok(tris) = earcutr::earcut(&flat, &holes, 2) {
            for t in tris.chunks_exact(3) {
                let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
                let ccw = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) > 0.0;
                let (b, c) = if ccw { (b, c) } else { (c, b) };
                self.tri([a[0], a[1], z1], [b[0], b[1], z1], [c[0], c[1], z1]);
                self.tri([a[0], a[1], z0], [c[0], c[1], z0], [b[0], b[1], z0]);
            }
        }
        for l in loops {
            let ccw = signed_area(l) > 0.0;
            for i in 0..l.len() {
                let (p, q) = (l[i], l[(i + 1) % l.len()]);
                let (p, q) = if ccw { (p, q) } else { (q, p) };
                self.quad([p[0], p[1], z0], [q[0], q[1], z0], [q[0], q[1], z1], [p[0], p[1], z1]);
            }
        }
    }

    /// Straight member along p0→p1 with a rectangular (w × h) or round (diameter w) section.
    fn member(&mut self, p0: P3, p1: P3, w: f64, h: f64, round: bool) {
        let Some(d) = unit(sub(p1, p0)) else { return };
        let up0 = if d[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        let Some(side) = unit(cross(d, up0)) else { return };
        let up = cross(side, d);
        if round {
            let r = w / 2.0;
            let n = 12;
            let ring = |c: P3, k: usize| {
                let t = 2.0 * PI * k as f64 / n as f64;
                add(c, add(mul(side, r * t.cos()), mul(up, r * t.sin())))
            };
            for k in 0..n {
                let (a, b) = (ring(p0, k), ring(p0, k + 1));
                let (c, e) = (ring(p1, k + 1), ring(p1, k));
                self.quad(a, e, c, b);
            }
            return;
        }
        let (sw, sh) = (mul(side, w / 2.0), mul(up, h / 2.0));
        let corner = |c: P3, s: f64, u: f64| add(c, add(mul(sw, s), mul(sh, u)));
        self.hexa(
            [corner(p0, -1.0, -1.0), corner(p1, -1.0, -1.0), corner(p1, 1.0, -1.0), corner(p0, 1.0, -1.0)],
            [corner(p0, -1.0, 1.0), corner(p1, -1.0, 1.0), corner(p1, 1.0, 1.0), corner(p0, 1.0, 1.0)],
        );
    }
}

fn signed_area(l: &[[f64; 2]]) -> f64 {
    (0..l.len()).map(|i| {
        let (p, q) = (l[i], l[(i + 1) % l.len()]);
        p[0] * q[1] - q[0] * p[1]
    }).sum::<f64>() / 2.0
}

fn point_in(pt: [f64; 2], l: &[[f64; 2]]) -> bool {
    let mut c = false;
    let mut j = l.len() - 1;
    for i in 0..l.len() {
        let (a, b) = (l[i], l[j]);
        if (a[1] > pt[1]) != (b[1] > pt[1]) && pt[0] < (b[0] - a[0]) * (pt[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// Polyline of a curve (model coordinates).
fn sample(c: &Curve) -> Vec<P3> {
    let arc_pts = |center: P3, rx: f64, ry: f64, x: P3, y: P3, t0: f64, t1: f64| {
        // step 2230: step angle from a chord error of at most ~1 mm, never finer than 7.5° and
        // at least one step per 90° (small profile fillets no longer get 7.5° steps)
        let r = rx.abs().max(ry.abs());
        let tol = 0.003;
        let step = if r > tol { (2.0 * (1.0 - tol / r).acos()).clamp(PI / 24.0, PI / 2.0) } else { PI / 2.0 };
        let n = (((t1 - t0).abs() / step).ceil() as usize).clamp(1, 96);
        (0..=n)
            .map(|k| {
                let t = t0 + (t1 - t0) * k as f64 / n as f64;
                add(center, add(mul(x, rx * t.cos()), mul(y, ry * t.sin())))
            })
            .collect::<Vec<_>>()
    };
    match c.kind {
        "line" => match (c.p0, c.p1) {
            (Some(a), Some(b)) => vec![a, b],
            _ => vec![],
        },
        "arc" => match (c.center, c.r, c.x, c.y, c.t0, c.t1) {
            (Some(o), Some(r), Some(x), Some(y), Some(t0), Some(t1)) => arc_pts(o, r, r, x, y, t0, t1),
            _ => vec![],
        },
        "circle" => match (c.center, c.r, c.x, c.y) {
            (Some(o), Some(r), Some(x), Some(y)) => arc_pts(o, r, r, x, y, 0.0, 2.0 * PI),
            _ => vec![],
        },
        "ellipse" => match (c.center, c.rx, c.ry, c.x, c.y, c.t0, c.t1) {
            (Some(o), Some(rx), Some(ry), Some(x), Some(y), Some(t0), Some(t1)) => arc_pts(o, rx, ry, x, y, t0, t1),
            _ => vec![],
        },
        _ => vec![],
    }
}

/// Closed plan loops from a sketch's curves (endpoints chained within 1e-4 ft), sorted into
/// (outer, holes) groups by containment.
fn plan_loops(curves: &[Curve]) -> (Vec<Vec<Vec<[f64; 2]>>>, f64) {
    let pieces: Vec<Vec<P3>> = curves.iter().map(sample).filter(|p| p.len() >= 2).collect();
    let z = pieces.first().map(|p| p[0][2]).unwrap_or(0.0);
    (group_loops(pieces, false), z)
}

/// Chains polyline pieces (x, y used) into closed loops and groups them as (outer, holes...).
/// With `strict` (family profiles, step 2218) chains that do not close are dropped instead of being
/// closed by a straight edge (an open chain gave triangular wedges), with a 1e-3 ft tolerance.
fn group_loops(mut pieces: Vec<Vec<P3>>, strict: bool) -> Vec<Vec<Vec<[f64; 2]>>> {
    let tol = if strict { 1e-3 } else { 1e-4 };
    let close = |a: P3, b: P3| (a[0] - b[0]).abs() < tol && (a[1] - b[1]).abs() < tol;
    let mut loops: Vec<Vec<[f64; 2]>> = Vec::new();
    while let Some(first) = pieces.pop() {
        let mut path = first;
        loop {
            let end = *path.last().expect("non-empty");
            if path.len() > 2 && close(end, path[0]) {
                break;
            }
            let Some(k) = pieces.iter().position(|p| close(p[0], end) || close(*p.last().expect("non-empty"), end)) else { break };
            let mut next = pieces.swap_remove(k);
            if !close(next[0], end) {
                next.reverse();
            }
            path.extend(next.into_iter().skip(1));
        }
        if strict && !(path.len() > 2 && close(*path.last().expect("non-empty"), path[0])) {
            continue;
        }
        let mut l: Vec<[f64; 2]> = path.iter().map(|p| [p[0], p[1]]).collect();
        if l.len() > 1 && close([l[0][0], l[0][1], 0.0], [l[l.len() - 1][0], l[l.len() - 1][1], 0.0]) {
            l.pop();
        }
        if l.len() >= 3 && signed_area(&l).abs() > 1e-6 {
            loops.push(l);
        }
    }
    loops.sort_by(|a, b| signed_area(b).abs().total_cmp(&signed_area(a).abs()));
    let mut groups: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
    for l in loops {
        match groups.iter_mut().find(|g| point_in(l[0], &g[0])) {
            Some(g) => g.push(l),
            None => groups.push(vec![l]),
        }
    }
    groups
}

fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Instance placement (step 2214): world[r] = o[r] + Σk m[3r+k]·p[k].
fn place(o: P3, m: &[f64; 9], p: P3) -> P3 {
    [
        o[0] + m[0] * p[0] + m[1] * p[1] + m[2] * p[2],
        o[1] + m[3] * p[0] + m[4] * p[1] + m[5] * p[2],
        o[2] + m[6] * p[0] + m[7] * p[1] + m[8] * p[2],
    ]
}

/// Rotation of `v` about the unit axis `k` by `a` (Rodrigues).
fn rotate(v: P3, k: P3, a: f64) -> P3 {
    let (c, s) = (a.cos(), a.sin());
    let kv = cross(k, v);
    let kd = dot(k, v);
    [
        v[0] * c + kv[0] * s + k[0] * kd * (1.0 - c),
        v[1] * c + kv[1] * s + k[1] * kd * (1.0 - c),
        v[2] * c + kv[2] * s + k[2] * kd * (1.0 - c),
    ]
}

/// Chains curve polylines into one path (endpoints within 1e-3 ft), in order.
fn chain_path(curves: &[Curve]) -> Vec<P3> {
    let mut pieces: Vec<Vec<P3>> = curves.iter().map(sample).filter(|p| p.len() >= 2).collect();
    let close = |a: P3, b: P3| norm(sub(a, b)) < 1e-3;
    let Some(mut path) = pieces.pop() else { return vec![] };
    loop {
        let end = *path.last().expect("non-empty");
        let start = path[0];
        if let Some(k) = pieces.iter().position(|p| close(p[0], end) || close(*p.last().expect("non-empty"), end)) {
            let mut next = pieces.swap_remove(k);
            if !close(next[0], end) {
                next.reverse();
            }
            path.extend(next.into_iter().skip(1));
        } else if let Some(k) = pieces.iter().position(|p| close(p[0], start) || close(*p.last().expect("non-empty"), start)) {
            let mut next = pieces.swap_remove(k);
            if close(next[0], start) {
                next.reverse();
            }
            next.pop();
            next.extend(path);
            path = next;
        } else {
            break;
        }
    }
    path
}

/// §8.12 (step 2221): sweep triangles in family coordinates. The profile curves lie in 3D on the
/// path at P(s0) (the path point nearest the profile centroid); per path segment the profile is
/// placed as P + R_N(θ(segment) − θ(s0))·(q − P(s0)), θ = tangent angle in the path plane.
fn sweep_triangles(s: &SweepSolid) -> Vec<[P3; 3]> {
    let path = chain_path(&s.path);
    let prof: Vec<Vec<P3>> = s.profile.iter().map(sample).filter(|p| p.len() >= 2).collect();
    let all: Vec<P3> = prof.iter().flatten().copied().collect();
    if path.len() < 2 || all.len() < 3 {
        return vec![];
    }
    let n = s.n;
    let cen = mul(all.iter().fold([0.0; 3], |a, p| add(a, *p)), 1.0 / all.len() as f64);
    // nearest point on the path to the profile centroid, with its segment direction
    let (mut best, mut p0, mut d0) = (f64::INFINITY, path[0], sub(path[1], path[0]));
    for w in path.windows(2) {
        let d = sub(w[1], w[0]);
        let l2 = dot(d, d);
        if l2 < 1e-18 {
            continue;
        }
        let t = (dot(sub(cen, w[0]), d) / l2).clamp(0.0, 1.0);
        let q = add(w[0], mul(d, t));
        let dist = norm(sub(cen, q));
        if dist < best {
            (best, p0, d0) = (dist, q, d);
        }
    }
    let theta = |d: P3| dot(d, s.y).atan2(dot(d, s.x));
    let th0 = theta(d0);
    // profile loops in the profile plane (normal = path tangent at s0)
    let Some(t0) = unit(d0) else { return vec![] };
    let u = unit(cross(n, t0)).unwrap_or(s.x);
    let v = cross(t0, u);
    let pieces: Vec<Vec<P3>> = prof.iter().map(|pc| pc.iter().map(|q| { let r = sub(*q, p0); [dot(r, u), dot(r, v), 0.0] }).collect()).collect();
    let groups = group_loops(pieces, true);
    let mut out = Vec::new();
    for w in path.windows(2) {
        let d = sub(w[1], w[0]);
        if norm(d) < 1e-9 {
            continue;
        }
        let a = theta(d) - th0;
        // profile point (u, v) relative to P(s0), rotated about N and moved to the path point
        let place = |base: P3, q: [f64; 2]| -> P3 { add(base, rotate(add(mul(u, q[0]), mul(v, q[1])), n, a)) };
        for g in &groups {
            if g.is_empty() || g[0].len() < 3 {
                continue;
            }
            // project w[0] onto the sweep line through p0 rotated: the segment start relative to p0
            let (b0, b1) = (w[0], w[1]);
            for l in g {
                for i in 0..l.len() {
                    let (qa, qb) = (l[i], l[(i + 1) % l.len()]);
                    out.push([place(b0, qa), place(b0, qb), place(b1, qb)]);
                    out.push([place(b0, qa), place(b1, qb), place(b1, qa)]);
                }
            }
            let mut flat = Vec::new();
            let mut holes = Vec::new();
            for (i, l) in g.iter().enumerate() {
                if i > 0 {
                    holes.push(flat.len() / 2);
                }
                for q in l {
                    flat.extend_from_slice(q);
                }
            }
            let pts: Vec<[f64; 2]> = g.iter().flatten().copied().collect();
            if let Ok(tris) = earcutr::earcut(&flat, &holes, 2) {
                for t in tris.chunks_exact(3) {
                    let (qa, qb, qc) = (pts[t[0]], pts[t[1]], pts[t[2]]);
                    out.push([place(b0, qa), place(b0, qc), place(b0, qb)]);
                    out.push([place(b1, qa), place(b1, qb), place(b1, qc)]);
                }
            }
        }
    }
    out
}

/// Closed loop resampled to `m` points at equal arc length.
fn resample_loop(l: &[P3], m: usize) -> Vec<P3> {
    let n = l.len();
    let seg: Vec<f64> = (0..n).map(|i| norm(sub(l[(i + 1) % n], l[i]))).collect();
    let total: f64 = seg.iter().sum();
    if total <= 0.0 {
        return vec![l[0]; m];
    }
    let mut out = Vec::with_capacity(m);
    let (mut i, mut acc) = (0usize, 0.0);
    for k in 0..m {
        let target = total * k as f64 / m as f64;
        while i < n - 1 && acc + seg[i] < target {
            acc += seg[i];
            i += 1;
        }
        let f = if seg[i] > 0.0 { (target - acc) / seg[i] } else { 0.0 };
        out.push(add(l[i], mul(sub(l[(i + 1) % n], l[i]), f.clamp(0.0, 1.0))));
    }
    out
}

/// §8.13 (step 2226): blend as a loft between the bottom and top loop (each moved along N by its
/// offset), both resampled to the same count and aligned (start nearest, same orientation).
fn blend_triangles(b: &BlendSolid) -> Vec<[P3; 3]> {
    let n = b.n;
    let lp = |cs: &[Curve], off: f64| -> Vec<P3> {
        let mut path = chain_path(cs);
        if path.len() > 2 && norm(sub(path[0], *path.last().expect("non-empty"))) < 1e-3 {
            path.pop();
        }
        path.into_iter().map(|q| add(q, mul(n, off))).collect()
    };
    let (bot, top) = (lp(&b.bottom, b.bottom_offset), lp(&b.top, b.top_offset));
    if bot.len() < 3 || top.len() < 3 {
        return vec![];
    }
    let u = unit(cross(n, if n[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] })).unwrap_or([1.0, 0.0, 0.0]);
    let v = cross(n, u);
    let area = |l: &[P3]| -> f64 {
        (0..l.len()).map(|i| { let (a, c) = (l[i], l[(i + 1) % l.len()]); dot(a, u) * dot(c, v) - dot(c, u) * dot(a, v) }).sum::<f64>() / 2.0
    };
    let m = bot.len().max(top.len()).clamp(16, 96);
    let rb = resample_loop(&bot, m);
    let mut top_l = top.clone();
    if (area(&bot) > 0.0) != (area(&top_l) > 0.0) {
        top_l.reverse();
    }
    let mut rt = resample_loop(&top_l, m);
    let shift = (0..m).min_by(|&i, &j| {
        let di = norm(sub(rt[i], rb[0])) - dot(sub(rt[i], rb[0]), n).abs();
        let dj = norm(sub(rt[j], rb[0])) - dot(sub(rt[j], rb[0]), n).abs();
        di.total_cmp(&dj)
    }).unwrap_or(0);
    rt.rotate_left(shift);
    let mut out = Vec::new();
    for i in 0..m {
        let j = (i + 1) % m;
        out.push([rb[i], rb[j], rt[j]]);
        out.push([rb[i], rt[j], rt[i]]);
    }
    for (l, flip) in [(&rb, true), (&rt, false)] {
        let flat: Vec<f64> = l.iter().flat_map(|q| [dot(*q, u), dot(*q, v)]).collect();
        if let Ok(tris) = earcutr::earcut(&flat, &[], 2) {
            for t in tris.chunks_exact(3) {
                if flip {
                    out.push([l[t[0]], l[t[2]], l[t[1]]]);
                } else {
                    out.push([l[t[0]], l[t[1]], l[t[2]]]);
                }
            }
        }
    }
    out
}

/// §8.14 (step 2226): revolution: every profile segment rotated about the axis A→B from the start
/// to the end angle in steps of at most 15°.
fn revolve_triangles(r: &RevolveSolid) -> Vec<[P3; 3]> {
    let (a, b) = (r.axis[0], r.axis[1]);
    let Some(k) = unit(sub(b, a)) else { return vec![] };
    let span = r.end_angle - r.start_angle;
    if !(span.abs() > 1e-9) {
        return vec![];
    }
    let steps = ((span.abs() / (PI / 12.0)).ceil() as usize).clamp(1, 48);
    let rot = |q: P3, ang: f64| -> P3 { add(a, rotate(sub(q, a), k, ang)) };
    let mut out = Vec::new();
    for c in &r.profile {
        for sg in sample(c).windows(2) {
            let (p, q) = (sg[0], sg[1]);
            for i in 0..steps {
                let (a0, a1) = (r.start_angle + span * i as f64 / steps as f64, r.start_angle + span * (i + 1) as f64 / steps as f64);
                let (p0, q0, p1, q1) = (rot(p, a0), rot(q, a0), rot(p, a1), rot(q, a1));
                out.push([p0, q0, q1]);
                out.push([p0, q1, p1]);
            }
        }
    }
    out
}

/// Triangles of a family's extrusion solids in family (local) coordinates.
fn family_triangles(solids: &[&ExtrusionSolid]) -> Vec<[P3; 3]> {
    let xf = |p: P3| -> P3 { p };
    let mut out = Vec::new();
    for s in solids {
        let to2 = |p: P3| -> P3 {
            let d = sub(p, s.o);
            [dot(d, s.x), dot(d, s.y), 0.0]
        };
        let pieces: Vec<Vec<P3>> =
            s.profile.iter().map(sample).filter(|p| p.len() >= 2).map(|v| v.into_iter().map(to2).collect()).collect();
        let lift = |q: [f64; 2], t: f64| -> P3 { xf(add(add(s.o, add(mul(s.x, q[0]), mul(s.y, q[1]))), mul(s.n, t))) };
        let (t0, t1) = (s.end.min(s.start), s.end.max(s.start));
        if !(t1 > t0) {
            continue;
        }
        for g in group_loops(pieces, true) {
            if g.is_empty() || g[0].len() < 3 {
                continue;
            }
            let mut flat = Vec::new();
            let mut holes = Vec::new();
            for (i, l) in g.iter().enumerate() {
                if i > 0 {
                    holes.push(flat.len() / 2);
                }
                for q in l {
                    flat.extend_from_slice(q);
                }
            }
            let pts: Vec<[f64; 2]> = g.iter().flatten().copied().collect();
            if let Ok(tris) = earcutr::earcut(&flat, &holes, 2) {
                for t in tris.chunks_exact(3) {
                    let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
                    out.push([lift(a, t1), lift(b, t1), lift(c, t1)]);
                    out.push([lift(a, t0), lift(c, t0), lift(b, t0)]);
                }
            }
            for l in &g {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    out.push([lift(a, t0), lift(b, t0), lift(b, t1)]);
                    out.push([lift(a, t0), lift(b, t1), lift(a, t1)]);
                }
            }
        }
    }
    out
}

/// Walk path of each run of a stair (sketch "path" lines, else the cached path), in run order.
fn run_paths(st: &Stair) -> Vec<(u64, Vec<P3>)> {
    st.runs
        .iter()
        .filter_map(|r| st.run_geometry.get(&r.to_string()).map(|g| (*r, g)))
        .map(|(r, g)| {
            let mut pts: Vec<P3> = Vec::new();
            let mut push = |q: Vec<P3>| {
                for p in q {
                    if pts.last().map_or(true, |l: &P3| norm(sub(*l, p)) > 1e-9) {
                        pts.push(p);
                    }
                }
            };
            match g.lines.get("path").filter(|c| !c.is_empty()) {
                Some(curves) => curves.iter().for_each(|c| push(sample(c))),
                None => {
                    for seg in g.path_cached.iter().flatten() {
                        match (seg.p0, seg.p1, &seg.points) {
                            (Some(a), Some(b), _) => push(vec![a, b]),
                            (_, _, Some(ps)) => push(ps.clone()),
                            _ => {}
                        }
                    }
                }
            }
            (r, pts)
        })
        .filter(|(_, p)| p.len() >= 2)
        .collect()
}

fn polyline_len(p: &[P3]) -> f64 {
    p.windows(2).map(|s| norm(sub([s[1][0], s[1][1], 0.0], [s[0][0], s[0][1], 0.0]))).sum()
}

fn instance_batch(cat: i32) -> (&'static str, [f32; 3]) {
    match cat {
        -2000023 => ("Doors", [0.71, 0.53, 0.35]),
        -2000014 => ("Windows", [0.45, 0.70, 0.92]),
        -2000080 | -2001100 => ("Furniture and casework", [0.80, 0.62, 0.40]),
        -2000100 | -2001330 => ("Columns", [0.62, 0.62, 0.66]),
        -2001320 => ("Structural framing", [0.55, 0.55, 0.60]),
        -2001160 => ("Plumbing fixtures", [0.85, 0.88, 0.90]),
        -2001040 | -2001060 => ("Electrical equipment and fixtures", [0.36, 0.42, 0.55]),
        -2001360 => ("Planting", [0.35, 0.55, 0.30]),
        -2001260 => ("Site", [0.55, 0.52, 0.45]),
        -2001140 | -2008013 | -2008010 | -2008016 | -2008055 => ("MEP equipment and terminals", [0.40, 0.65, 0.55]),
        -2001120 => ("Lighting fixtures", [0.96, 0.62, 0.04]),
        -2000170 => ("Curtain panels", [0.62, 0.74, 0.82]),
        -2000171 => ("Curtain wall mullions", [0.55, 0.57, 0.60]),
        -2000151 => ("Generic models", [0.70, 0.68, 0.64]),
        _ => ("Other family instances", [0.66, 0.66, 0.68]),
    }
}

fn segment_batch(cat: Option<i32>) -> (&'static str, [f32; 3]) {
    match cat {
        Some(-2008000) => ("Ducts", [0.35, 0.60, 0.80]),
        Some(-2008044) => ("Pipes", [0.30, 0.70, 0.45]),
        Some(-2008130) | Some(-2008132) => ("Cable trays and conduits", [0.85, 0.55, 0.20]),
        _ => ("MEP segments", [0.45, 0.62, 0.70]),
    }
}

/// Builds the viewer scene of a document.
pub fn scene(doc: &Document) -> Scene {
    // step 2227: the expensive readers once
    let walls_all = {
        // step 2228: one record per id (a wall can appear in two project chains)
        let mut seen = std::collections::HashSet::new();
        doc.walls().into_iter().filter(|w| seen.insert(w.id)).collect::<Vec<_>>()
    };
    // step 2228: an element can appear in two project chains (Storax: 366 copies in a second chain,
    // mostly without a type); keep one record per id, the one with a type
    let insts_all: Vec<Instance> = {
        let mut by_id: Vec<Instance> = Vec::new();
        let mut at: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        for i in doc.instances() {
            match at.get(&i.id) {
                Some(&k) => {
                    if by_id[k].symbol.is_none() && i.symbol.is_some() {
                        by_id[k] = i;
                    }
                }
                None => {
                    at.insert(i.id, by_id.len());
                    by_id.push(i);
                }
            }
        }
        by_id
    };
    let mut batches: BTreeMap<&'static str, Batch> = BTreeMap::new();
    fn get<'a>(m: &'a mut BTreeMap<&'static str, Batch>, name: &'static str, color: [f32; 3]) -> &'a mut Batch {
        m.entry(name).or_insert_with(|| Batch::new(name, color))
    }
    macro_rules! batch {
        ($name:expr, $color:expr) => {
            get(&mut batches, $name, $color)
        };
    }

    // Curtain panels and mullions (step 2215): their stored boxes are the real panel/mullion extents
    // (Storax: mullion 28 × 64 × 2000 mm, louvre 60 × 100 × 2000 mm), except in some models where a
    // panel box spans a whole wall; those (two sides > 3 m) are skipped.
    let curtain_boxes: Vec<[f64; 6]> = insts_all
        .iter()
        .filter(|i| matches!(i.cat, -2000170 | -2000171))
        .filter_map(|i| doc.element(i.id).and_then(|e| e.bbox))
        .filter(|bb| {
            let ext = [bb[3] - bb[0], bb[4] - bb[1], bb[5] - bb[2]];
            ext.iter().all(|v| v.is_finite() && *v >= 0.0 && *v < 1e4) && ext.iter().filter(|v| **v * FT > 3.0).count() < 2
        })
        .collect();
    for w in walls_all.iter().cloned() {
        // A wall without a compound type (curtain wall, storefront) is drawn by its panels and
        // mullions when it has any; drawing it solid hides them (step 2215).
        if doc.type_thickness(w.wall_type).is_none() && !curtain_boxes.is_empty() {
            let (lo, hi) = (
                [w.p0[0].min(w.p1[0]) - 1.0, w.p0[1].min(w.p1[1]) - 1.0],
                [w.p0[0].max(w.p1[0]) + 1.0, w.p0[1].max(w.p1[1]) + 1.0],
            );
            let inside = |bb: &[f64; 6]| {
                let (cx, cy) = ((bb[0] + bb[3]) / 2.0, (bb[1] + bb[4]) / 2.0);
                cx >= lo[0] && cx <= hi[0] && cy >= lo[1] && cy <= hi[1]
            };
            if curtain_boxes.iter().any(inside) {
                continue;
            }
        }
        let base = doc.level_elevation(w.base_level).map(|z| z + w.base_offset).unwrap_or(w.p0[2]);
        let height = w.height.filter(|h| *h > 0.0 && h.is_finite()).unwrap_or(10.0);
        let t = doc.type_thickness(w.wall_type).filter(|t| *t > 0.0).unwrap_or(0.5);
        let line: Vec<P3> = match &w.curve {
            Some(c) => sample(&Curve { id: w.id, kind: "arc", center: Some(c.center), r: Some(c.r), x: Some(c.x), y: Some(c.y), t0: Some(c.t0), t1: Some(c.t1), ..Default::default() }),
            None => vec![w.p0, w.p1],
        };
        let b = batch!("Walls", [0.90, 0.88, 0.84]);
        for s in line.windows(2) {
            let (a, c) = ([s[0][0], s[0][1], base], [s[1][0], s[1][1], base]);
            let Some(d) = unit(sub(c, a)) else { continue };
            let n = mul([-d[1], d[0], 0.0], t / 2.0);
            let (a0, a1, c0, c1) = (sub(a, n), add(a, n), sub(c, n), add(c, n));
            let up = [0.0, 0.0, height];
            b.hexa([a0, c0, c1, a1], [add(a0, up), add(c0, up), add(c1, up), add(a1, up)]);
        }
    }

    let slab = |b: &mut Batch, curves: &[Curve], z0: Option<f64>, thickness: f64, down: bool| {
        let (groups, cz) = plan_loops(curves);
        let z = z0.unwrap_or(cz);
        let (lo, hi) = if down { (z - thickness, z) } else { (z, z + thickness) };
        for g in groups {
            b.prism(&g, lo, hi);
        }
    };
    for f in doc.floors() {
        let t = f.thickness.filter(|t| *t > 0.0).unwrap_or(0.5);
        slab(batch!("Floors", [0.62, 0.60, 0.56]), &f.curves, f.top_z, t, true);
    }
    for f in doc.ceilings() {
        let t = f.thickness.filter(|t| *t > 0.0).unwrap_or(0.08);
        slab(batch!("Ceilings", [0.93, 0.92, 0.90]), &f.curves, f.top_z, t, false);
    }
    for r in doc.roofs() {
        let t = r.thickness.filter(|t| *t > 0.0).unwrap_or(0.75);
        let curves: Vec<Curve> = r.edges.iter().map(|e| e.curve.clone()).collect();
        if r.kind == "footprint" {
            slab(batch!("Roofs", [0.60, 0.30, 0.18]), &curves, r.base_z, t, false);
            continue;
        }
        // Extrusion roof (§32, steps 1359–1360): the profile lies in a vertical sketch plane with
        // normal N; each profile segment becomes a plate of the roof thickness (hanging below the
        // profile) swept along N from `end` to `start`.
        let (Some(Some(n)), Some(s0), Some(s1)) = (r.normal, r.end, r.start) else { continue };
        let Some(n) = unit(n) else { continue };
        let b = batch!("Roofs", [0.60, 0.30, 0.18]);
        let (o0, o1) = (mul(n, s0), mul(n, s1));
        for c in &curves {
            for s in sample(c).windows(2) {
                let (p, q) = (s[0], s[1]);
                let Some(m) = unit(cross(n, sub(q, p))) else { continue };
                let m = if m[2] > 0.0 { mul(m, -t) } else { mul(m, t) };
                let (pb, qb) = (add(p, m), add(q, m));
                b.hexa(
                    [add(pb, o0), add(qb, o0), add(qb, o1), add(pb, o1)],
                    [add(p, o0), add(q, o0), add(q, o1), add(p, o1)],
                );
            }
        }
    }

    let segs: Vec<Segment> = doc.segments();
    for s in &segs {
        let cat = doc.element(s.id).map(|e| e.category);
        let (name, color) = segment_batch(cat);
        batch!(name, color).member(s.p0, s.p1, s.width, s.height, s.round);
    }

    // Railings (§35): top rail along the path at the railing (or top rail) height, posts at most
    // 3 ft apart. The base is the railing's level plus its base offset when it has a level
    // (steps 2200, 2202: the path z is sometimes 0 for a railing on an upper level); otherwise
    // the path z.
    // Stairs (§33): per run, one block per tread along the walk path, each a riser high and as wide
    // as the run (step 722; 3 ft if unknown), rising by one riser per tread from the run's base
    // elevation (absolute, step 2211), or from the stair base counted over the runs.
    let stairs = doc.stairs();
    let runs: std::collections::HashMap<u64, rvt_model::StairsRun> = doc.stairs_runs().into_iter().map(|r| (r.id, r)).collect();
    for st in &stairs {
        let Some(zb) = st.base_z else { continue };
        if !(st.riser > 0.0 && st.tread > 0.0) {
            continue;
        }
        let b = batch!("Stairs", [0.70, 0.66, 0.60]);
        let mut k = 0usize;
        for (rid, path) in run_paths(st) {
            let run = runs.get(&rid).filter(|r| r.width > 0.0 && r.width < 50.0 && r.base_elevation.is_finite());
            let half = run.map_or(1.5, |r| r.width / 2.0);
            if let Some(r) = run {
                k = ((r.base_elevation - zb) / st.riser).round().max(0.0) as usize;
            }
            let len = polyline_len(&path);
            let n = ((len / st.tread).round() as usize).max(1);
            // points at equal steps along the path
            let at = |t: f64| -> P3 {
                let mut rest = t * len;
                for s in path.windows(2) {
                    let l = norm(sub([s[1][0], s[1][1], 0.0], [s[0][0], s[0][1], 0.0]));
                    if rest <= l || l == 0.0 {
                        let f = if l > 0.0 { rest / l } else { 0.0 };
                        return add(s[0], mul(sub(s[1], s[0]), f));
                    }
                    rest -= l;
                }
                *path.last().expect("non-empty")
            };
            for i in 0..n {
                if k as i64 >= st.risers {
                    break;
                }
                let (a, c) = (at(i as f64 / n as f64), at((i + 1) as f64 / n as f64));
                let z0 = zb + st.riser * k as f64;
                let Some(dir) = unit(sub([c[0], c[1], 0.0], [a[0], a[1], 0.0])) else { continue };
                let w = mul([-dir[1], dir[0], 0.0], half);
                let (a, c) = ([a[0], a[1], z0], [c[0], c[1], z0]);
                let (a0, a1, c0, c1) = (sub(a, w), add(a, w), sub(c, w), add(c, w));
                let up = [0.0, 0.0, st.riser];
                b.hexa([a0, c0, c1, a1], [add(a0, up), add(c0, up), add(c1, up), add(a1, up)]);
                k += 1;
            }
        }
    }
    let stair_by_id: std::collections::HashMap<u64, &Stair> = stairs.iter().map(|s| (s.id, s)).collect();

    let bits32 = doc.info().bits == Some(32);
    for r in doc.railings() {
        let info = r.type_info.as_ref();
        // step 2204: 64-bit top rail height first (Railing Height is 0 for some types); 32-bit
        // Railing Height first (its top rail fields read as 4.0 / type 4 there)
        let ok = |v: Option<f64>| v.filter(|h| *h > 0.0 && *h < 20.0);
        let (th, rh) = (ok(info.and_then(|t| t.toprail_height)), ok(info.and_then(|t| t.height)));
        let h = if bits32 { rh.or(th) } else { th.or(rh) }.unwrap_or(3.5);
        let up = [0.0, 0.0, h];
        let path_z = r.path.iter().flat_map(sample).next().map(|p| p[2]);
        let dz = match (r.level.and_then(|l| doc.level_elevation(l)), path_z) {
            (Some(z), Some(pz)) => z + r.base_offset.unwrap_or(0.0) - pz,
            _ => 0.0,
        };
        let mut polys: Vec<Vec<P3>> = r.path.iter().map(|c| sample(c).into_iter().map(|p| [p[0], p[1], p[2] + dz]).collect()).collect();
        // step 2207: a stair railing rises from the stair base at the end nearest a run start (p0)
        // to the stair top at the other end, linearly along the path
        if let Some(st) = r.host.and_then(|h| stair_by_id.get(&(h as u64))) {
            let rp = run_paths(st);
            let p0s: Vec<P3> = rp.iter().map(|(_, p)| p[0]).collect();
            let all: Vec<P3> = polys.iter().flatten().copied().collect();
            if let (Some(zb0), Some(&first), Some(&last)) = (st.base_z, all.first(), all.last()) {
                if !p0s.is_empty() && st.height > 0.0 {
                    let flat = |a: P3, q: P3| norm(sub([a[0], a[1], 0.0], [q[0], q[1], 0.0]));
                    let d = |a: P3| p0s.iter().map(|q| flat(a, *q)).fold(f64::INFINITY, f64::min);
                    let low_first = d(first) <= d(last);
                    let (lo_pt, hi_pt) = if low_first { (first, last) } else { (last, first) };
                    // step 2211: a railing along one run of a multi-run stair spans that run only:
                    // base of the run starting nearest the low end, top of the run ending nearest
                    // the high end
                    let nearest = |pt: P3, start: bool| {
                        rp.iter()
                            .min_by(|a, b| {
                                let ka = if start { a.1[0] } else { *a.1.last().expect("non-empty") };
                                let kb = if start { b.1[0] } else { *b.1.last().expect("non-empty") };
                                flat(pt, ka).total_cmp(&flat(pt, kb))
                            })
                            .and_then(|(r, _)| runs.get(r))
                    };
                    let zb = nearest(lo_pt, true).map_or(zb0, |r| r.base_elevation);
                    let zt = nearest(hi_pt, false).map_or(zb0 + st.height, |r| r.top_elevation);
                    let rise = if zt > zb { zt - zb } else { st.height };
                    let total: f64 = polys.iter().map(|p| polyline_len(p)).sum();
                    let mut acc = 0.0;
                    for poly in polys.iter_mut() {
                        let mut prev: Option<P3> = None;
                        for p in poly.iter_mut() {
                            if let Some(q) = prev {
                                acc += norm(sub([p[0], p[1], 0.0], [q[0], q[1], 0.0]));
                            }
                            prev = Some(*p);
                            let f = if total > 0.0 { acc / total } else { 0.0 };
                            p[2] = zb + rise * if low_first { f } else { 1.0 - f };
                        }
                    }
                }
            }
        }
        let b = batch!("Railings", [0.45, 0.47, 0.50]);
        for pts in polys {
            let mut posts = pts.first().copied().into_iter().collect::<Vec<P3>>();
            let mut since = 0.0;
            for s in pts.windows(2) {
                b.member(add(s[0], up), add(s[1], up), 0.15, 0.15, false);
                let len = norm(sub(s[1], s[0]));
                let mut t = 3.0 - since;
                while t < len {
                    posts.push(add(s[0], mul(sub(s[1], s[0]), t / len)));
                    t += 3.0;
                }
                since = (since + len) % 3.0;
            }
            posts.extend(pts.last().copied());
            for p in posts {
                b.member(p, add(p, up), 0.08, 0.08, false);
            }
        }
    }

    let insts: &[Instance] = &insts_all;
    // Family geometry (§8.11, steps 2214–2216): extrusion solids of the instance's family, used when
    // they cover the element's stored box (±0.15 ft); otherwise the box itself.
    let symbol_family = doc.symbol_family();
    let family_chain = doc.family_chains();
    let chain_solids: std::collections::HashMap<(String, usize), Vec<ExtrusionSolid>> =
        doc.family_solids_by_chain().into_iter().map(|(st, ix, v)| ((st, ix), v)).collect();
    let chain_sweeps: std::collections::HashMap<(String, usize), Vec<SweepSolid>> =
        doc.family_sweeps_by_chain().into_iter().map(|(st, ix, v)| ((st, ix), v)).collect();
    let solids_of = |sym: u64| -> Vec<&ExtrusionSolid> {
        symbol_family
            .get(&sym)
            .and_then(|f| family_chain.get(f))
            .and_then(|k| chain_solids.get(k))
            .map(|v| v.iter().filter(|s| s.void != Some(true) && s.host_block.is_none()).collect())
            .unwrap_or_default()
    };
    let chain_blrev: std::collections::HashMap<(String, usize), (Vec<BlendSolid>, Vec<RevolveSolid>)> =
        doc.family_blends_revolves_by_chain().into_iter().map(|(st, ix, b, r)| ((st, ix), (b, r))).collect();
    let blrev_of = |sym: u64| -> Vec<[P3; 3]> {
        let mut t = Vec::new();
        if let Some((bl, rv)) = symbol_family.get(&sym).and_then(|f| family_chain.get(f)).and_then(|k| chain_blrev.get(k)) {
            for b in bl.iter().filter(|b| b.void != Some(true)) {
                t.extend(blend_triangles(b));
            }
            for r in rv.iter().filter(|r| r.void != Some(true)) {
                t.extend(revolve_triangles(r));
            }
        }
        t
    };
    let sweeps_of = |sym: u64| -> Vec<&SweepSolid> {
        symbol_family
            .get(&sym)
            .and_then(|f| family_chain.get(f))
            .and_then(|k| chain_sweeps.get(k))
            .map(|v| v.iter().filter(|s| s.void != Some(true)).collect())
            .unwrap_or_default()
    };
    let (mut n_family, mut n_box) = (0usize, 0usize);
    let mut box_by_cat: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
    // step 2227: family shapes per symbol are computed once (many instances share a type)
    let mut shape_cache: std::collections::HashMap<u64, std::rc::Rc<Vec<Vec<[P3; 3]>>>> = std::collections::HashMap::new();
    // step 2230: alternatives per symbol: (shape, local min, local max)
    let mut alt_cache: std::collections::HashMap<u64, std::rc::Rc<Vec<(Vec<[P3; 3]>, P3, P3)>>> = std::collections::HashMap::new();
    // Step 2219: door/window boxes include the swing; clip them to the host wall band instead.
    let wall_band: std::collections::HashMap<i64, (P3, P3, f64)> = walls_all
        .iter()
        .map(|w| (w.id as i64, (w.p0, w.p1, doc.type_thickness(w.wall_type).filter(|t| *t > 0.0).unwrap_or(0.5))))
        .collect();
    let host_of: std::collections::HashMap<u64, i64> = doc.hosted().into_iter().filter_map(|h| h.host.map(|x| (h.id, x))).collect();
    'inst: for i in insts.iter().filter(|i| i.scale.is_none() && i.origin.is_some()) {
        let Some(e) = doc.element(i.id) else { continue };
        if e.view.is_some_and(|v| v != -1) {
            continue; // view-specific (annotation)
        }
        let Some(bb) = e.bbox else { continue };
        let ext = [bb[3] - bb[0], bb[4] - bb[1], bb[5] - bb[2]];
        if ext.iter().any(|v| !(v.is_finite() && *v >= 0.0 && *v < 1e4)) || ext.iter().all(|v| *v < 1e-6) {
            continue;
        }
        // A stored box that is large in all three directions (> 3 m) covers more than the family
        // itself (site, massing or nested content); drawing it hides the building.
        if ext.iter().all(|v| *v * FT > 3.0) {
            continue;
        }
        if matches!(i.cat, -2000170 | -2000171) && ext.iter().filter(|v| **v * FT > 3.0).count() >= 2 {
            continue; // panel box spanning a whole wall (see curtain_boxes above)
        }
        let (name, color) = instance_batch(i.cat);
        if let (Some(sym), Some(o), Some(m)) = (i.symbol, i.origin, i.rotation) {
            let candidates = shape_cache
                .entry(sym)
                .or_insert_with(|| {
                    let sol = solids_of(sym);
                    let swp = sweeps_of(sym);
                    let br = blrev_of(sym);
                    // step 2224: all forms first; when a sweep spoils the fit, without sweeps; then
                    // extrusions alone
                    let ext_only = family_triangles(&sol);
                    let mut no_sweeps = ext_only.clone();
                    no_sweeps.extend(br.iter().copied());
                    let mut all = no_sweeps.clone();
                    for sw in &swp {
                        all.extend(sweep_triangles(sw));
                    }
                    let mut c: Vec<Vec<[P3; 3]>> = Vec::new();
                    for x in [all, no_sweeps, ext_only] {
                        if !x.is_empty() && !c.iter().any(|y| y.len() == x.len()) {
                            c.push(x);
                        }
                    }
                    std::rc::Rc::new(c)
                })
                .clone();
            // step 2230: a family with several sweeps along the same path (Storax louvres: 22
            // profile variants, the type switches one on) has alternatives; the one whose placed
            // sizes are closest to the element box goes first.
            let alts = alt_cache
                .entry(sym)
                .or_insert_with(|| {
                    let swp = sweeps_of(sym);
                    let key = |sw: &SweepSolid| -> Vec<i64> {
                        sw.path.iter().flat_map(|c| c.p0.into_iter().chain(c.p1)).flatten().map(|v| (v * 1e3).round() as i64).collect()
                    };
                    let mut groups: std::collections::BTreeMap<Vec<i64>, Vec<usize>> = std::collections::BTreeMap::new();
                    for (k, sw) in swp.iter().enumerate() {
                        groups.entry(key(sw)).or_default().push(k);
                    }
                    let Some(group) = groups.into_values().filter(|g| g.len() >= 3).max_by_key(|g| g.len()) else {
                        return std::rc::Rc::new(vec![]);
                    };
                    let mut base = family_triangles(&solids_of(sym));
                    base.extend(blrev_of(sym));
                    for (_, sw) in swp.iter().enumerate().filter(|(k, _)| !group.contains(k)) {
                        base.extend(sweep_triangles(sw));
                    }
                    let out = group
                        .iter()
                        .map(|&k| {
                            let mut t = base.clone();
                            t.extend(sweep_triangles(swp[k]));
                            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                            for tr in &t {
                                for q in tr {
                                    for j in 0..3 {
                                        lo[j] = lo[j].min(q[j]);
                                        hi[j] = hi[j].max(q[j]);
                                    }
                                }
                            }
                            (t, lo, hi)
                        })
                        .filter(|a| !a.0.is_empty())
                        .collect();
                    std::rc::Rc::new(out)
                })
                .clone();
            let best_alt = alts
                .iter()
                .map(|(t, lo, hi)| {
                    // placed bounds of the local box corners
                    let (mut wl, mut wh) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                    for c in 0..8 {
                        let q = place(o, &m, [if c & 1 == 0 { lo[0] } else { hi[0] }, if c & 2 == 0 { lo[1] } else { hi[1] }, if c & 4 == 0 { lo[2] } else { hi[2] }]);
                        for j in 0..3 {
                            wl[j] = wl[j].min(q[j]);
                            wh[j] = wh[j].max(q[j]);
                        }
                    }
                    // size ratio per axis (the fit below scales parametric lengths)
                    let err: f64 = (0..3).map(|j| ((wh[j] - wl[j] + 1e-3) / (ext[j] + 1e-3)).ln().abs()).sum();
                    (err, t)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, t)| t);
            let candidates: Vec<&Vec<[P3; 3]>> = best_alt.into_iter().chain(candidates.iter()).collect();
            if !candidates.is_empty() {
                for local in candidates.iter() {
                if std::env::var("RVT_MESH_DEBUG").ok().and_then(|v| v.parse::<u64>().ok()) == Some(i.id) {
                    eprintln!("debug {}: {} local triangles, bbox {:?}", i.id, local.len(), bb);
                }
                let bounds = |tris: &[[P3; 3]]| {
                    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                    for t in tris {
                        for p in t {
                            for k in 0..3 {
                                lo[k] = lo[k].min(p[k]);
                                hi[k] = hi[k].max(p[k]);
                            }
                        }
                    }
                    (lo, hi)
                };
                let world: Vec<[P3; 3]> = local.iter().map(|t| [place(o, &m, t[0]), place(o, &m, t[1]), place(o, &m, t[2])]).collect();
                let (lo, hi) = bounds(&world);
                let covers = !world.is_empty() && (0..3).all(|k| (lo[k] - bb[k]).abs() < 0.15 && (hi[k] - bb[k + 3]).abs() < 0.15);
                // Step 2217: family geometry is parametric (beam length, type sizes, height
                // parameters). When the rotation is axis-aligned, fit the family shape per local axis
                // to the element's stored box instead of falling back to the box.
                let axis_aligned = m.iter().all(|v| v.abs() < 1e-6 || (v.abs() - 1.0).abs() < 1e-6);
                let fitted: Option<Vec<[P3; 3]>> = (!covers && axis_aligned && !world.is_empty())
                    .then(|| {
                        // element box in local coordinates (m is a signed permutation: inverse = transpose)
                        let inv = |w: P3| -> P3 {
                            let d = sub(w, o);
                            [m[0] * d[0] + m[3] * d[1] + m[6] * d[2], m[1] * d[0] + m[4] * d[1] + m[7] * d[2], m[2] * d[0] + m[5] * d[1] + m[8] * d[2]]
                        };
                        let (mut elo, mut ehi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                        for c in 0..8 {
                            let w = [bb[if c & 1 == 0 { 0 } else { 3 }], bb[if c & 2 == 0 { 1 } else { 4 }], bb[if c & 4 == 0 { 2 } else { 5 }]];
                            let q = inv(w);
                            for k in 0..3 {
                                elo[k] = elo[k].min(q[k]);
                                ehi[k] = ehi[k].max(q[k]);
                            }
                        }
                        let (slo, shi) = bounds(&local);
                        // step 2218: the stored box of a door also covers its swing (8 ft deep for a
                        // 0.14 ft leaf); stretching up to 2× is plausible. Shrinking is safe down to 0.05×:
                        // a smaller box cannot contain a swing (step 2229: Storax louvres 0.295 → 0.067 ft)
                        let plausible = (0..3).all(|k| {
                            let (se, ee) = (shi[k] - slo[k], ehi[k] - elo[k]);
                            se < 1e-6 || (ee / se >= 0.05 && ee / se <= 2.0)
                        });
                        if !plausible {
                            return None;
                        }
                        let map = |p: P3| -> P3 {
                            let mut r = [0.0; 3];
                            for k in 0..3 {
                                let se = shi[k] - slo[k];
                                r[k] = if se > 1e-6 { elo[k] + (p[k] - slo[k]) * (ehi[k] - elo[k]) / se } else { (elo[k] + ehi[k]) / 2.0 };
                            }
                            place(o, &m, r)
                        };
                        Some(local.iter().map(|t| [map(t[0]), map(t[1]), map(t[2])]).collect())
                    })
                    .flatten()
                    // otherwise the unscaled family shape when it lies inside the stored box
                    .or_else(|| {
                        let inside = (0..3).all(|k| lo[k] >= bb[k] - 0.15 && hi[k] <= bb[k + 3] + 0.15);
                        // doors and windows: the stored box includes the swing, so it is no fallback
                        let opening = matches!(i.cat, -2000023 | -2000014);
                        (!covers && (inside || opening) && !world.is_empty()).then(|| world.clone())
                    });
                if covers || fitted.is_some() {
                    let b = batch!(name, color);
                    for t in fitted.unwrap_or(world) {
                        b.tri(t[0], t[1], t[2]);
                    }
                    n_family += 1;
                    continue 'inst;
                }
                }
            }
        }
        n_box += 1;
        *box_by_cat.entry(i.cat).or_default() += 1;
        // RVT_MESH_DEBUG_BOX=<category>: why an instance of that category stays a box
        if std::env::var("RVT_MESH_DEBUG_BOX").ok().and_then(|v| v.parse::<i32>().ok()) == Some(i.cat) && box_by_cat[&i.cat] <= 6 {
            let c = i.symbol.and_then(|s| shape_cache.get(&s)).map(|c| c.len()).unwrap_or(0);
            let ext = |t: &[[P3; 3]]| {
                let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for tr in t {
                    for p in tr {
                        for k in 0..3 {
                            lo[k] = lo[k].min(p[k]);
                            hi[k] = hi[k].max(p[k]);
                        }
                    }
                }
                [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
            };
            let local = i.symbol.and_then(|s| shape_cache.get(&s)).and_then(|c| c.first().map(|t| ext(t)));
            eprintln!("box {} sym {:?} candidates {} local ext {:?} element ext {:?} M {:?}", i.id, i.symbol, c, local, [bb[3] - bb[0], bb[4] - bb[1], bb[5] - bb[2]], i.rotation.map(|m| m.map(|v| (v * 100.0).round() / 100.0)));
        }
        if matches!(i.cat, -2000023 | -2000014) {
            if let Some(&(p0, p1, t)) = host_of.get(&i.id).and_then(|h| wall_band.get(h)) {
                if let Some(d) = unit(sub([p1[0], p1[1], 0.0], [p0[0], p0[1], 0.0])) {
                    let nrm = [-d[1], d[0], 0.0];
                    // extent of the stored box along the wall; across it the wall thickness
                    let corners = [[bb[0], bb[1]], [bb[3], bb[1]], [bb[0], bb[4]], [bb[3], bb[4]]];
                    let along: Vec<f64> = corners.iter().map(|c| (c[0] - p0[0]) * d[0] + (c[1] - p0[1]) * d[1]).collect();
                    let (a0, a1) = (along.iter().cloned().fold(f64::INFINITY, f64::min), along.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
                    let o = i.origin.unwrap_or(p0);
                    // the opening sits where its origin is along the wall; never longer than the box
                    let at = |a: f64, s: f64| -> P3 { add(add([p0[0], p0[1], 0.0], mul(d, a)), mul(nrm, s)) };
                    let off = (o[0] - p0[0]) * nrm[0] + (o[1] - p0[1]) * nrm[1];
                    let off = if off.abs() > t { 0.0 } else { off };
                    let (s0, s1) = (off - t / 2.0 - 0.02, off + t / 2.0 + 0.02);
                    let (z0, z1) = (bb[2], bb[5]);
                    let lift = |p: P3, z: f64| -> P3 { [p[0], p[1], z] };
                    let bottom = [at(a0, s0), at(a1, s0), at(a1, s1), at(a0, s1)];
                    batch!(name, color).hexa(
                        [lift(bottom[0], z0), lift(bottom[1], z0), lift(bottom[2], z0), lift(bottom[3], z0)],
                        [lift(bottom[0], z1), lift(bottom[1], z1), lift(bottom[2], z1), lift(bottom[3], z1)],
                    );
                    continue;
                }
            }
            // not wall-hosted (work plane): a thin slab through the origin along the local X axis
            if let (Some(o), Some(m)) = (i.origin, i.rotation) {
                let ax = [m[0], m[3], 0.0];
                if let Some(d) = unit(ax).filter(|_| m[6].abs() < 1e-6) {
                    let nrm = [-d[1], d[0], 0.0];
                    let corners = [[bb[0], bb[1]], [bb[3], bb[1]], [bb[0], bb[4]], [bb[3], bb[4]]];
                    let along: Vec<f64> = corners.iter().map(|c| (c[0] - o[0]) * d[0] + (c[1] - o[1]) * d[1]).collect();
                    let (a0, a1) = (along.iter().cloned().fold(f64::INFINITY, f64::min), along.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
                    let at = |a: f64, s: f64, z: f64| -> P3 { let q = add(add([o[0], o[1], 0.0], mul(d, a)), mul(nrm, s)); [q[0], q[1], z] };
                    let (z0, z1) = (bb[2], bb[5]);
                    batch!(name, color).hexa(
                        [at(a0, -0.125, z0), at(a1, -0.125, z0), at(a1, 0.125, z0), at(a0, 0.125, z0)],
                        [at(a0, -0.125, z1), at(a1, -0.125, z1), at(a1, 0.125, z1), at(a0, 0.125, z1)],
                    );
                    continue;
                }
            }
        }
        batch!(name, color).aabb(bb);
    }

    if std::env::var_os("RVT_MESH_STATS").is_some() {
        eprintln!("instances: {n_family} with family geometry, {n_box} as box");
        let mut v: Vec<_> = box_by_cat.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!("as box by category: {:?}", v.iter().take(8).map(|(c, n)| format!("{c}:{n}")).collect::<Vec<_>>());
    }
    let mut out = Scene { batches: batches.into_values().filter(|b| !b.indices.is_empty()).collect(), ..Default::default() };
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for b in &out.batches {
        for p in b.positions.chunks_exact(3) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    if lo[0].is_finite() {
        out.bounds_min = lo;
        out.bounds_max = hi;
    }
    out
}

/// Binary wire format (little endian): "RVTM", u32 version 1, u32 batch count; per batch
/// u32 name length, name (UTF-8, padded to 4), 3 × f32 colour, u32 vertex count, u32 index count,
/// positions (f32 × 3n), normals (f32 × 3n), indices (u32); then bounds min and max (6 × f32).
pub fn encode(scene: &Scene) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"RVTM");
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&(scene.batches.len() as u32).to_le_bytes());
    for x in &scene.batches {
        let name = x.name.as_bytes();
        b.extend_from_slice(&(name.len() as u32).to_le_bytes());
        b.extend_from_slice(name);
        b.resize(b.len() + (4 - name.len() % 4) % 4, 0);
        for c in x.color {
            b.extend_from_slice(&c.to_le_bytes());
        }
        b.extend_from_slice(&((x.positions.len() / 3) as u32).to_le_bytes());
        b.extend_from_slice(&(x.indices.len() as u32).to_le_bytes());
        for v in x.positions.iter().chain(&x.normals) {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for i in &x.indices {
            b.extend_from_slice(&i.to_le_bytes());
        }
    }
    for v in scene.bounds_min.iter().chain(&scene.bounds_max) {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}
