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

/// Step 2282: adaptive placement points of an instance record, in the instance frame: a u32
/// count n, then n × (u32 tag, 3×3 rotation, origin) = 100 bytes each, every rotation orthonormal.
fn adaptive_points(b: &[u8]) -> Vec<P3> {
    let f = |o: usize| f64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let frame_ok = |o: usize| -> bool {
        let m: Vec<f64> = (0..9).map(|k| f(o + 8 * k)).collect();
        let row = |r: usize| (m[3 * r] * m[3 * r] + m[3 * r + 1] * m[3 * r + 1] + m[3 * r + 2] * m[3 * r + 2] - 1.0).abs() < 1e-4;
        let mm: [f64; 9] = m.clone().try_into().unwrap();
        m.iter().all(|v| v.is_finite()) && (0..3).all(row) && (det3(&mm) - 1.0).abs() < 1e-4 && (0..3).all(|k| f(o + 72 + 8 * k).abs() < 1e5)
    };
    for o in 200..b.len().saturating_sub(108) {
        let n = u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
        if !(2..=32).contains(&n) || o + 4 + n * 100 > b.len() {
            continue;
        }
        if (0..n).all(|k| frame_ok(o + 8 + k * 100)) {
            return (0..n).map(|k| {
                let q = o + 8 + k * 100 + 72;
                [f(q), f(q + 8), f(q + 16)]
            }).collect();
        }
    }
    Vec::new()
}

/// Step 2282: a sheet under a Catmull-Rom curve through `pts` (it passes through each point), its
/// lower edge `drop[k]` below point k (negative = down). A surface form: Revit's strut has one face
/// (all GT normals to one side, 0.76 m²), a plate of 'Thickness' drew twice that.
fn adaptive_rib(b: &mut Batch, pts: &[P3], drop: &[f64]) {
    let n = pts.len();
    let at = |k: isize| pts[k.clamp(0, n as isize - 1) as usize];
    let dz = |k: isize| drop[k.clamp(0, n as isize - 1) as usize];
    let cr = |a: f64, b: f64, c: f64, d: f64, t: f64| 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t + (3.0 * b - a - 3.0 * c + d) * t * t * t);
    let mut samples: Vec<(P3, f64)> = Vec::new();
    for k in 0..n as isize - 1 {
        let steps = if n > 2 { 8 } else { 1 };
        for s in 0..steps {
            let t = s as f64 / steps as f64;
            let (p0, p1, p2, p3) = (at(k - 1), at(k), at(k + 1), at(k + 2));
            let p = [0, 1, 2].map(|j| cr(p0[j], p1[j], p2[j], p3[j], t));
            samples.push((p, dz(k) + (dz(k + 1) - dz(k)) * t));
        }
    }
    samples.push((pts[n - 1], drop[n - 1]));
    for w in samples.windows(2) {
        let ((a, da), (c, dc)) = (w[0], w[1]);
        let low = |p: P3, d: f64| [p[0], p[1], p[2] + d];
        b.quad(a, c, low(c, dc), low(a, da));
    }
}

/// Steps 2284/2285: the profile of a sweep type (slab edge, rail): the sketch of the profile family
/// the type references first (an i64, else an i32 in 2019 and older) as one loop (arcs in 6 steps
/// from centre, axes, radius and parameters), sized per axis by the type: the current value equal to
/// the sketch extent takes the symbol's value of the same parameter name.
fn type_profile(
    doc: &Document,
    t: u64,
    fam_of: &std::collections::HashMap<u64, u64>,
    fam_chain: &std::collections::BTreeMap<u64, (String, usize)>,
    gnames: &std::collections::HashMap<u64, Option<String>>,
    rows: &std::collections::HashMap<u64, rvt_model::ValueSet>,
) -> Option<Vec<[f64; 2]>> {
    let b = doc.element_body(t)?;
    let sym_of = |v: i64| (v > 0).then(|| fam_of.get(&(v as u64)).and_then(|f| fam_chain.get(f)).map(|k| (v as u64, k.clone()))).flatten();
    // an i64 reference, else (2019 and older) an i32
    let (sym, key) = (0..b.len().saturating_sub(8))
        .find_map(|o| sym_of(i64_at_le(&b, o)))
        .or_else(|| (0..b.len().saturating_sub(4)).find_map(|o| sym_of(i32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as i64)))?;
    // the sketch as one loop: each curve a polyline (an arc in 6 steps from its centre,
    // axes, radius and parameters; snowdon type 909838: 4 fillet arcs without end points)
    let mut segs: Vec<Vec<[f64; 2]>> = doc
        .chain_curves(&key.0, key.1)
        .into_iter()
        .filter_map(|(_, c)| match (c.kind, c.center, c.x, c.y, c.r, c.t0, c.t1) {
            ("arc", Some(cc), Some(x), Some(y), Some(r), Some(t0), Some(t1)) => Some(
                (0..=6)
                    .map(|j| {
                        let t = t0 + (t1 - t0) * j as f64 / 6.0;
                        [cc[0] + r * (t.cos() * x[0] + t.sin() * y[0]), cc[1] + r * (t.cos() * x[1] + t.sin() * y[1])]
                    })
                    .collect(),
            ),
            // step 2285: a closed ellipse or circle (snowdon rail type 1607960: ellipse 0.125 × 0.094
            // round the path) in 24 steps
            ("ellipse" | "circle", Some(cc), Some(x), Some(y), ..) => {
                let (rx, ry) = (c.rx.or(c.r)?, c.ry.or(c.r)?);
                Some(
                    (0..=24)
                        .map(|j| {
                            let t = std::f64::consts::TAU * j as f64 / 24.0;
                            [cc[0] + rx * t.cos() * x[0] + ry * t.sin() * y[0], cc[1] + rx * t.cos() * x[1] + ry * t.sin() * y[1]]
                        })
                        .collect(),
                )
            }
            _ => Some(vec![[c.p0?[0], c.p0?[1]], [c.p1?[0], c.p1?[1]]]),
        })
        .collect();
    let mut poly = segs.pop()?;
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5;
    while !segs.is_empty() {
        let last = *poly.last().unwrap();
        let k = segs.iter().position(|v| near(v[0], last) || near(*v.last().unwrap(), last))?;
        let mut v = segs.remove(k);
        if !near(v[0], last) {
            v.reverse();
        }
        poly.extend(v.into_iter().skip(1));
    }
    if near(poly[0], *poly.last().unwrap()) {
        poly.pop();
    }
    if poly.len() < 3 {
        return None;
    }
    // size by the type: per axis the current value equal to the sketch extent
    let (lo, hi) = poly.iter().fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(l, h), q| ([l[0].min(q[0]), l[1].min(q[1])], [h[0].max(q[0]), h[1].max(q[1])]));
    let cur = doc.family_current_values(&key.0, key.1);
    let names = doc.parameter_names_of(&key.0, key.1);
    let scale = |k: usize| -> f64 {
        let ext = hi[k] - lo[k];
        let name = cur.as_ref().and_then(|c| c.rows.iter().find(|r| r.param > 0 && (r.num - ext).abs() < 1e-6)).and_then(|r| names.get(&(r.param as u64)).cloned().flatten());
        let v = name.and_then(|n| rows.get(&sym).and_then(|vs| vs.rows.iter().find(|r| r.param > 0 && gnames.get(&(r.param as u64)).cloned().flatten().as_deref() == Some(n.as_str()))).map(|r| r.num));
        v.filter(|v| *v > 1e-6 && ext > 1e-9).map_or(1.0, |v| v / ext)
    };
    let (sx, sy) = (scale(0), scale(1));
    Some(poly.into_iter().map(|q| [q[0] * sx, q[1] * sy]).collect())
}

/// Step 2285: a closed 2-D profile swept along path pieces (lines; arcs sampled at ≤ 0.5 ft), the
/// profile point placed by `frame(point, tangent, (x, y))`; sides as quads, both ends capped by a fan.
fn sweep_profile(b: &mut Batch, path: &[rvt_model::Curve], prof: &[[f64; 2]], frame: impl Fn(P3, P3, [f64; 2]) -> Option<P3>) {
    let k = prof.len();
    if k < 3 {
        return;
    }
    let c = prof.iter().fold([0.0, 0.0], |a, q| [a[0] + q[0] / k as f64, a[1] + q[1] / k as f64]);
    for cv in path {
        let samples: Vec<(P3, P3)> = match (cv.kind, cv.p0, cv.p1, cv.center, cv.x, cv.y, cv.r, cv.t0, cv.t1) {
            ("arc", _, _, Some(cc), Some(x), Some(y), Some(r), Some(t0), Some(t1)) => {
                let n = (((t1 - t0) * r / 0.5).ceil() as usize).clamp(2, 128);
                (0..=n)
                    .map(|j| {
                        let t = t0 + (t1 - t0) * j as f64 / n as f64;
                        (add(cc, add(mul(x, r * t.cos()), mul(y, r * t.sin()))), add(mul(x, -t.sin()), mul(y, t.cos())))
                    })
                    .collect()
            }
            (_, Some(p0), Some(p1), ..) => match unit(sub(p1, p0)) {
                Some(d) => vec![(p0, d), (p1, d)],
                None => continue,
            },
            _ => continue,
        };
        for w in samples.windows(2) {
            for j in 0..k {
                let (p, q) = (prof[j], prof[(j + 1) % k]);
                if let (Some(a0), Some(b0), Some(b1), Some(a1)) = (frame(w[0].0, w[0].1, p), frame(w[0].0, w[0].1, q), frame(w[1].0, w[1].1, q), frame(w[1].0, w[1].1, p)) {
                    b.quad(a0, b0, b1, a1);
                }
            }
        }
        for (end, flip) in [(samples[0], false), (samples[samples.len() - 1], true)] {
            for j in 0..k {
                let (p, q) = (prof[j], prof[(j + 1) % k]);
                if let (Some(cp), Some(pp), Some(qp)) = (frame(end.0, end.1, c), frame(end.0, end.1, p), frame(end.0, end.1, q)) {
                    if flip {
                        b.tri(cp, pp, qp);
                    } else {
                        b.tri(cp, qp, pp);
                    }
                }
            }
        }
    }
}

fn i64_at_le(b: &[u8], o: usize) -> i64 {
    i64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"))
}

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

    /// Box from 8 corners: bottom a b c d, top e f g h (top[i] above bottom[i]). Step 2274: the
    /// corners may run either way round; the faces are made to point outward (seen from the top the
    /// bottom must run counter-clockwise; callers laid boxes along a direction with a normal of
    /// either sign: snowdon wall sweeps and some treads were inside out).
    fn hexa(&mut self, bottom: [P3; 4], top: [P3; 4]) {
        let (mut bottom, mut top) = (bottom, top);
        let nb = cross(sub(bottom[1], bottom[0]), sub(bottom[2], bottom[0]));
        let up = sub(mul(add(add(top[0], top[1]), add(top[2], top[3])), 0.25), mul(add(add(bottom[0], bottom[1]), add(bottom[2], bottom[3])), 0.25));
        if dot(nb, up) < 0.0 {
            bottom.reverse();
            top.reverse();
        }
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

/// z where the vertical line through (x, y) meets triangle `t`, if it does.
fn ray_z(t: &[P3; 3], x: f64, y: f64) -> Option<f64> {
    let (a, b, c) = (t[0], t[1], t[2]);
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if d.abs() < 1e-12 {
        return None;
    }
    let l1 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / d;
    let l2 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / d;
    let l3 = 1.0 - l1 - l2;
    (l1 > -1e-9 && l2 > -1e-9 && l3 > -1e-9).then(|| l1 * a[2] + l2 * b[2] + l3 * c[2])
}

/// Triangles of a batch back in model coordinates (feet, Z up).
fn batch_tris(b: &Batch) -> Vec<[P3; 3]> {
    let p = |i: u32| -> P3 {
        let k = i as usize * 3;
        let v = &b.positions[k..k + 3];
        [v[0] as f64 / FT, -(v[2] as f64) / FT, v[1] as f64 / FT]
    };
    b.indices.chunks_exact(3).map(|t| [p(t[0]), p(t[1]), p(t[2])]).collect()
}

/// Slab of closed sketch loops with its top (or bottom) at `z0`.
fn slab_into(b: &mut Batch, curves: &[Curve], z0: Option<f64>, thickness: f64, down: bool) {
    let (groups, cz) = plan_loops(curves);
    let z = z0.unwrap_or(cz);
    let (lo, hi) = if down { (z - thickness, z) } else { (z, z + thickness) };
    for g in groups {
        b.prism(&g, lo, hi);
    }
}

/// Roof: a footprint roof as a slab on its base; an extrusion roof (§32, steps 1359–1360): the
/// profile lies in a vertical sketch plane with normal N; each profile segment becomes a plate of
/// the roof thickness (hanging below the profile) swept along N from `end` to `start`.
fn roof_into(b: &mut Batch, r: &rvt_model::Roof) {
    let t = r.thickness.filter(|t| *t > 0.0).unwrap_or(0.75);
    let curves: Vec<Curve> = r.edges.iter().map(|e| e.curve.clone()).collect();
    if r.kind == "footprint" {
        slab_into(b, &curves, r.base_z, t, false);
        return;
    }
    let (Some(Some(n)), Some(s0), Some(s1)) = (r.normal, r.end, r.start) else { return };
    let Some(n) = unit(n) else { return };
    let (o0, o1) = (mul(n, s0), mul(n, s1));
    for c in &curves {
        for s in sample(c).windows(2) {
            let (p, q) = (s[0], s[1]);
            let Some(m) = unit(cross(n, sub(q, p))) else { continue };
            let m = if m[2] > 0.0 { mul(m, -t) } else { mul(m, t) };
            let (pb, qb) = (add(p, m), add(q, m));
            b.hexa([add(pb, o0), add(qb, o0), add(qb, o1), add(pb, o1)], [add(p, o0), add(q, o0), add(q, o1), add(p, o1)]);
        }
    }
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
/// Step 2257: placement of a flipped instance. The stored matrix is a rotation; a hand or facing
/// flip (not both: that is a half turn, already in the stored rotation) mirrors the family's local
/// Y axis (rme/rac_basic 2026 doors: the leaf on Revit's side for 124/124).
fn flipped_rotation(i: &Instance) -> Option<[f64; 9]> {
    let mut m = i.rotation?;
    if i.hand_flipped != i.facing_flipped {
        for k in [1, 4, 7] {
            m[k] = -m[k];
        }
    }
    Some(m)
}

/// Step 2261: the oriented box behind a stored (world, axis-aligned) box. A box of local half sizes
/// e turned by m has world half sizes h = |m|·e; e ≥ 0 is the non-negative least-squares solution
/// over the subsets of free axes. None when m is a signed permutation (the stored box is exact).
/// snowdon arch 2025: adaptive canopy struts (0.7 m² in Revit) drew as 90 m² world boxes.
fn oriented_box(bb: &[f64; 6], m: &[f64; 9]) -> Option<([P3; 4], [P3; 4])> {
    if m.iter().all(|v| v.abs() < 1e-6 || (v.abs() - 1.0).abs() < 1e-6) {
        return None;
    }
    let a: [[f64; 3]; 3] = std::array::from_fn(|j| std::array::from_fn(|k| m[3 * j + k].abs()));
    let h = [(bb[3] - bb[0]) / 2.0, (bb[4] - bb[1]) / 2.0, (bb[5] - bb[2]) / 2.0];
    let mut best: Option<(f64, [f64; 3])> = None;
    for mask in 1u8..8 {
        let free: Vec<usize> = (0..3).filter(|k| mask & (1 << k) != 0).collect();
        // normal equations on the free columns
        let n = free.len();
        let mut ata = vec![vec![0.0; n]; n];
        let mut atb = vec![0.0; n];
        for (r, &kr) in free.iter().enumerate() {
            for (c, &kc) in free.iter().enumerate() {
                ata[r][c] = (0..3).map(|j| a[j][kr] * a[j][kc]).sum();
            }
            atb[r] = (0..3).map(|j| a[j][kr] * h[j]).sum();
        }
        // Gaussian elimination
        let mut ok = true;
        for c in 0..n {
            let piv = (c..n).max_by(|x, y| ata[*x][c].abs().total_cmp(&ata[*y][c].abs())).unwrap();
            if ata[piv][c].abs() < 1e-12 {
                ok = false;
                break;
            }
            ata.swap(c, piv);
            atb.swap(c, piv);
            for r in 0..n {
                if r != c {
                    let f = ata[r][c] / ata[c][c];
                    for k in c..n {
                        ata[r][k] -= f * ata[c][k];
                    }
                    atb[r] -= f * atb[c];
                }
            }
        }
        if !ok {
            continue;
        }
        let mut e = [0.0; 3];
        for (r, &k) in free.iter().enumerate() {
            e[k] = atb[r] / ata[r][r];
        }
        if e.iter().any(|v| *v < -1e-9) {
            continue;
        }
        let res: f64 = (0..3).map(|j| ((0..3).map(|k| a[j][k] * e[k]).sum::<f64>() - h[j]).powi(2)).sum();
        if best.map_or(true, |(r, _)| res < r - 1e-12) {
            best = Some((res, e));
        }
    }
    let (_, e) = best?;
    // a flat solution still gets a visible thickness
    let e = e.map(|v| v.max(0.02));
    let c = [(bb[0] + bb[3]) / 2.0, (bb[1] + bb[4]) / 2.0, (bb[2] + bb[5]) / 2.0];
    // a mirrored placement walks the corners the other way round (faces outward)
    let f = if det3(m) < 0.0 { -1.0 } else { 1.0 };
    let at = |sx: f64, sy: f64, sz: f64| place(c, m, [sx * e[0], f * sy * e[1], sz * e[2]]);
    Some((
        [at(-1.0, -1.0, -1.0), at(1.0, -1.0, -1.0), at(1.0, 1.0, -1.0), at(-1.0, 1.0, -1.0)],
        [at(-1.0, -1.0, 1.0), at(1.0, -1.0, 1.0), at(1.0, 1.0, 1.0), at(-1.0, 1.0, 1.0)],
    ))
}

/// Determinant of a 3×3 (row-major): negative for a mirrored placement.
fn det3(m: &[f64; 9]) -> f64 {
    m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6])
}

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
    outward(out)
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
    outward(out)
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
    outward(out)
}

/// Step 2254: detail-level variants. Extrusions with the same plane, origin, range and profile
/// bounds are one form drawn per detail level (rst steel columns: an I-profile of 12 and one of 16
/// curves); keep the finest (most profile curves), as Revit shows at Fine.
fn finest_variants<'a>(solids: Vec<&'a ExtrusionSolid>) -> Vec<&'a ExtrusionSolid> {
    let key = |s: &ExtrusionSolid| -> Vec<i64> {
        let pts: Vec<P3> = s.profile.iter().flat_map(|c| c.p0.into_iter().chain(c.p1).chain(c.center)).collect();
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for q in &pts {
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        [s.start, s.end].iter().chain(s.n.iter()).chain(s.o.iter()).chain(lo.iter()).chain(hi.iter()).map(|v| (v * 1e4).round() as i64).collect()
    };
    let mut best: Vec<(Vec<i64>, &'a ExtrusionSolid)> = Vec::new();
    for sld in solids {
        let k = key(sld);
        match best.iter_mut().find(|(bk, _)| *bk == k) {
            Some(entry) => {
                if sld.profile.len() > entry.1.profile.len() {
                    entry.1 = sld;
                }
            }
            None => best.push((k, sld)),
        }
    }
    best.into_iter().map(|(_, sld)| sld).collect()
}

/// Step 2279: detail-level variants of sweeps, like `finest_variants`: sweeps along the same path
/// with the same plane and profile bounds are one form per detail level (rst steel beams: an
/// I-profile of 16 and one of 12 curves along one path, both drawn = twice the area); keep the
/// finest (most profile curves).
fn finest_sweeps<'a>(sweeps: Vec<&'a SweepSolid>) -> Vec<&'a SweepSolid> {
    // exact: path and plane; within 0.1 ft: the profile bounds (rst channels: a C of 10 curves
    // y −0.16..0.08 and one of 8 curves y −0.19..0.10 along the same path)
    let key = |s: &SweepSolid| -> Vec<i64> {
        let path = s.path.iter().flat_map(|c| c.p0.into_iter().chain(c.p1).chain(c.center)).flatten();
        path.chain(s.x.iter().copied()).chain(s.y.iter().copied()).chain(s.n.iter().copied()).map(|v| (v * 1e4).round() as i64).collect()
    };
    let bounds = |s: &SweepSolid| -> [f64; 6] {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for q in s.profile.iter().flat_map(|c| c.p0.into_iter().chain(c.p1).chain(c.center)) {
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
    };
    let mut best: Vec<(Vec<i64>, [f64; 6], &'a SweepSolid)> = Vec::new();
    for sw in sweeps {
        let (k, b) = (key(sw), bounds(sw));
        match best.iter_mut().find(|(bk, bb, _)| *bk == k && (0..6).all(|i| (bb[i] - b[i]).abs() <= 0.1)) {
            Some(entry) => {
                if sw.profile.len() > entry.2.profile.len() {
                    entry.2 = sw;
                }
            }
            None => best.push((k, b, sw)),
        }
    }
    best.into_iter().map(|(_, _, sw)| sw).collect()
}

/// Triangles of a family's extrusion solids in family (local) coordinates.
fn family_triangles(solids: &[&ExtrusionSolid]) -> Vec<[P3; 3]> {
    let xf = |p: P3| -> P3 { p };
    let mut out = Vec::new();
    for s in solids {
        let mut out_all = std::mem::take(&mut out);
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
        // step 2274: each solid outward (a clockwise profile or a negative depth turned it inside out)
        out_all.extend(outward(std::mem::take(&mut out)));
        out = out_all;
    }
    out
}

/// Step 2274: the triangles of one closed solid with outward faces: reversed when their signed
/// volume about their centroid is negative (snowdon columns, fixtures were inside out).
fn outward(mut t: Vec<[P3; 3]>) -> Vec<[P3; 3]> {
    if t.is_empty() {
        return t;
    }
    let n = (t.len() * 3) as f64;
    let c = t.iter().flatten().fold([0.0; 3], |a, p| add(a, *p));
    let c = mul(c, 1.0 / n);
    let vol: f64 = t.iter().map(|tr| dot(sub(tr[0], c), cross(sub(tr[1], c), sub(tr[2], c)))).sum();
    if vol < 0.0 {
        for tr in t.iter_mut() {
            tr.swap(1, 2);
        }
    }
    t
}

/// Walking surface of an old sketch stair (step 2232): a run's nosing line or a landing strip from
/// `a` to `c` (plan), `half` wide to each side, at z0 (at `a`) rising to z1 (at `c`).
#[derive(Clone, Copy)]
struct WalkPiece {
    a: P3,
    c: P3,
    half: f64,
    z0: f64,
    z1: f64,
}

impl WalkPiece {
    /// (lateral excess, z) when the plan point lies on or beside this piece.
    fn z_at(&self, p: P3) -> Option<(f64, f64)> {
        let d = sub([self.c[0], self.c[1], 0.0], [self.a[0], self.a[1], 0.0]);
        let len = norm(d);
        let dir = unit(d)?;
        let v = sub([p[0], p[1], 0.0], [self.a[0], self.a[1], 0.0]);
        let t = dot(v, dir);
        let lat = norm(sub(v, mul(dir, t)));
        let ex = (lat - self.half).max(0.0);
        (t > -0.3 && t < len + 0.3 && ex < 0.75).then(|| (ex, self.z0 + (self.z1 - self.z0) * (t / len).clamp(0.0, 1.0)))
    }
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
        -2001300 => ("Structural foundations", [0.58, 0.56, 0.54]),
        -2001160 => ("Plumbing fixtures", [0.85, 0.88, 0.90]),
        -2001040 | -2001060 => ("Electrical equipment and fixtures", [0.36, 0.42, 0.55]),
        -2001360 => ("Planting", [0.35, 0.55, 0.30]),
        -2001260 => ("Site", [0.55, 0.52, 0.45]),
        -2001140 | -2008013 | -2008010 | -2008016 | -2008055 => ("MEP equipment and terminals", [0.40, 0.65, 0.55]),
        -2001120 => ("Lighting fixtures", [0.96, 0.62, 0.04]),
        -2000170 => ("Curtain panels", [0.62, 0.74, 0.82]),
        -2000171 => ("Curtain wall mullions", [0.55, 0.57, 0.60]),
        -2000151 => ("Generic models", [0.70, 0.68, 0.64]),
        // step 2285: railing supports (handrail brackets) belong with the railings
        -2000948 => ("Railings", [0.45, 0.47, 0.50]),
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
    // Step 2259: hidden group definition copies (S2233) are not drawn, whatever their kind. Their
    // group is none of the type's placed groups (Revit API: snowdon arch 2025 282/282 members of
    // groups outside GroupType.Groups and outside the Group collector; rac_basic fixture 800336, rme
    // 6 pipe insulations: absent from Revit's {3D} geometry).
    let mut group_defs = doc.group_definition_ids();
    // Step 2260: demolished elements (Phase Demolished set) are not shown in a {3D} view of the last
    // phase with 'Show Complete' (snowdon arch 2025: 6 legend floors, created = demolished = 'Legends';
    // rac/rme/rst 2026: no element demolished, every element of Revit's geometry has a phase).
    group_defs.extend(doc.element_phases().into_iter().filter(|(_, (_, d))| d.is_some()).map(|(id, _)| id));
    // Step 2284: elements of a secondary design option are not in the {3D} view (snowdon arch 2025
    // 'Bleacher Seating' option 'Continuous Bleachers no Stairs': 9 floors and their slab edges).
    group_defs.extend(doc.secondary_option_elements());
    // step 2227: the expensive readers once
    let walls_all = {
        // step 2228: one record per id (a wall can appear in two project chains)
        let mut seen = std::collections::HashSet::new();
        // step 2238: walls inside an in-place family are not separate walls
        let inplace = doc.inplace_ids();
        doc.walls().into_iter().filter(|w| seen.insert(w.id) && !inplace.contains(&w.id) && !group_defs.contains(&w.id)).collect::<Vec<_>>()
    };
    // step 2228: an element can appear in two project chains (Storax: 366 copies in a second chain,
    // mostly without a type); keep one record per id, the one with a type
    let insts_all: Vec<Instance> = {
        let mut by_id: Vec<Instance> = Vec::new();
        let mut at: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        for i in doc.instances().into_iter().filter(|i| !group_defs.contains(&i.id)) {
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
        for i in by_id.iter_mut() {
            i.rotation = flipped_rotation(i);
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
    // step 2237: top-attached walls and the triangles of their roofs/floors/ceilings
    let mut attach_top: std::collections::HashMap<u64, Vec<i64>> = std::collections::HashMap::new();
    for a in doc.wall_attachments().into_iter().filter(|a| a.flag == 1) {
        attach_top.entry(a.id).or_default().push(a.target);
    }
    let target_tris: std::collections::HashMap<i64, Vec<[P3; 3]>> = {
        let want: std::collections::HashSet<i64> = attach_top.values().flatten().copied().collect();
        let mut m = std::collections::HashMap::new();
        if !want.is_empty() {
            for r in doc.roofs().iter().filter(|r| want.contains(&(r.id as i64))) {
                let mut tb = Batch::new("tmp", [0.0; 3]);
                roof_into(&mut tb, r);
                m.insert(r.id as i64, batch_tris(&tb));
            }
            let floors = doc.floors();
            let ceilings = doc.ceilings();
            for (f, down, dt) in floors.iter().map(|f| (f, true, 0.5)).chain(ceilings.iter().map(|f| (f, false, 0.08))) {
                if want.contains(&(f.id as i64)) {
                    let mut tb = Batch::new("tmp", [0.0; 3]);
                    slab_into(&mut tb, &f.curves, f.top_z, f.thickness.filter(|t| *t > 0.0).unwrap_or(dt), down);
                    m.insert(f.id as i64, batch_tris(&tb));
                }
            }
        }
        m
    };
    // step 2236: door/window openings per host wall: (u0, u1, z0, z1) along the wall line
    let openings: std::collections::HashMap<u64, Vec<(f64, f64, f64, f64)>> = {
        let wall_line: std::collections::HashMap<u64, (P3, P3)> = walls_all.iter().map(|w| (w.id, (w.p0, w.p1))).collect();
        let cat: std::collections::HashMap<u64, i32> = insts_all.iter().map(|i| (i.id, i.cat)).collect();
        let mut m: std::collections::HashMap<u64, Vec<(f64, f64, f64, f64)>> = std::collections::HashMap::new();
        for h in doc.hosted() {
            if !matches!(cat.get(&h.id), Some(-2000023 | -2000014)) {
                continue;
            }
            let Some(host) = h.host.map(|x| x as u64) else { continue };
            let (Some(&(p0, p1)), Some(bb)) = (wall_line.get(&host), doc.element(h.id).and_then(|e| e.bbox)) else { continue };
            let a = [p0[0], p0[1], 0.0];
            let Some(d) = unit(sub([p1[0], p1[1], 0.0], a)) else { continue };
            let us: Vec<f64> = [(bb[0], bb[1]), (bb[3], bb[1]), (bb[0], bb[4]), (bb[3], bb[4])].iter().map(|&(x, y)| dot(sub([x, y, 0.0], a), d)).collect();
            let (u0, u1) = (us.iter().copied().fold(f64::INFINITY, f64::min), us.iter().copied().fold(f64::NEG_INFINITY, f64::max));
            m.entry(host).or_default().push((u0, u1, bb[2], bb[5]));
        }
        // step 2286: an embedded curtain wall (a straight wall without a compound type on the line
        // of a straight basic wall, within its length) cuts its extent out of the host (snowdon
        // 735573, 40.8 × 24.25 ft: curtain wall 1700768 40.2 × 17.5 inside, GT 685 ft² = both faces
        // less the opening; 1047581 and 1184003 alike)
        let zrange = |w: &rvt_model::Wall| -> (f64, f64) {
            let base = doc.level_elevation(w.base_level).map(|z| z + w.base_offset).unwrap_or(w.p0[2]);
            (base, base + w.height.filter(|h| *h > 0.0 && h.is_finite()).unwrap_or(10.0))
        };
        let curtains: Vec<&rvt_model::Wall> = walls_all.iter().filter(|w| w.curve.is_none() && doc.type_thickness(w.wall_type).is_none()).collect();
        if !curtains.is_empty() {
            let mut cuts: Vec<(u64, u64, f64, f64)> = Vec::new();
            for h in walls_all.iter().filter(|w| w.curve.is_none()) {
                let Some(t) = doc.type_thickness(h.wall_type).filter(|t| *t > 0.0) else { continue };
                let a = [h.p0[0], h.p0[1], 0.0];
                let Some(d) = unit(sub([h.p1[0], h.p1[1], 0.0], a)) else { continue };
                let len = norm(sub([h.p1[0], h.p1[1], 0.0], a));
                let (hz0, hz1) = zrange(h);
                for c in &curtains {
                    let rel = |p: P3| sub([p[0], p[1], 0.0], a);
                    let off = |p: P3| (rel(p)[0] * d[1] - rel(p)[1] * d[0]).abs();
                    if off(c.p0) > t / 2.0 + 0.05 || off(c.p1) > t / 2.0 + 0.05 {
                        continue;
                    }
                    let (u0, u1) = (dot(rel(c.p0), d).min(dot(rel(c.p1), d)), dot(rel(c.p0), d).max(dot(rel(c.p1), d)));
                    let (cz0, cz1) = zrange(c);
                    // clipped to the host's height (snowdon 1184003: curtain walls 0.74 ft above its top)
                    let (cz0, cz1) = (cz0.max(hz0), cz1.min(hz1));
                    if u0 < -0.1 || u1 > len + 0.1 || u1 - u0 < 0.1 || cz1 - cz0 < 0.1 {
                        continue;
                    }
                    m.entry(h.id).or_default().push((u0, u1, cz0, cz1));
                    cuts.push((h.id, c.id, cz0, cz1));
                }
            }
            // a straight wall lying face to face with a host (parallel, bands touching within
            // 0.05 ft) is cut by the same opening (snowdon 1474830, 0.45 ft against host 619404's
            // face, curtain wall 946544 on 619404's line: GT 1265 ft² = both faces less the opening)
            let by_id: std::collections::HashMap<u64, &rvt_model::Wall> = walls_all.iter().map(|w| (w.id, w)).collect();
            for (hid, cid, cz0, cz1) in cuts {
                let (Some(h), Some(c)) = (by_id.get(&hid), curtains.iter().find(|c| c.id == cid)) else { continue };
                let (Some(ht), Some(d)) = (doc.type_thickness(h.wall_type), unit(sub([h.p1[0], h.p1[1], 0.0], [h.p0[0], h.p0[1], 0.0]))) else { continue };
                for w in walls_all.iter().filter(|w| w.id != hid && w.curve.is_none()) {
                    let Some(wt) = doc.type_thickness(w.wall_type).filter(|t| *t > 0.0) else { continue };
                    let a = [w.p0[0], w.p0[1], 0.0];
                    let Some(wd) = unit(sub([w.p1[0], w.p1[1], 0.0], a)) else { continue };
                    if dot(wd, d).abs() < 1.0 - 1e-6 {
                        continue;
                    }
                    let rel = |p: P3| sub([p[0], p[1], 0.0], a);
                    let off = (rel(h.p0)[0] * wd[1] - rel(h.p0)[1] * wd[0]).abs();
                    if (off - (ht + wt) / 2.0).abs() > 0.05 {
                        continue;
                    }
                    let len = norm(sub([w.p1[0], w.p1[1], 0.0], a));
                    let (u0, u1) = (dot(rel(c.p0), wd).min(dot(rel(c.p1), wd)).max(0.0), dot(rel(c.p0), wd).max(dot(rel(c.p1), wd)).min(len));
                    let (wz0, wz1) = zrange(w);
                    let (z0, z1) = (cz0.max(wz0), cz1.min(wz1));
                    if u1 - u0 > 0.1 && z1 - z0 > 0.1 {
                        m.entry(w.id).or_default().push((u0, u1, z0, z1));
                    }
                }
            }
        }
        m
    };
    // RVT_MESH_DEBUG_WALLS=1: surface (m²) drawn per wall, for comparison with Revit
    let dbg_walls = std::env::var("RVT_MESH_DEBUG_WALLS").is_ok();
    let wall_area = |b: Option<&Batch>, n0: usize| -> f64 {
        let Some(b) = b else { return 0.0 };
        let p = |i: u32| -> P3 {
            let k = i as usize * 3;
            [b.positions[k] as f64, b.positions[k + 1] as f64, b.positions[k + 2] as f64]
        };
        b.indices[n0..].chunks_exact(3).map(|t| norm(cross(sub(p(t[1]), p(t[0])), sub(p(t[2]), p(t[0])))) / 2.0).sum()
    };
    let mut prev_wall: Option<(u64, usize)> = None;
    for w in walls_all.iter().cloned() {
        if dbg_walls {
            if let Some((id, n0)) = prev_wall.take() {
                eprintln!("wall {id} {:.2}", wall_area(batches.get("Walls"), n0));
            }
            prev_wall = Some((w.id, batches.get("Walls").map_or(0, |b| b.indices.len())));
        }
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
        // step 2236: door/window openings (straight walls): their stored box projected on the wall
        // line gives the interval along the wall, its z range the height
        let holes: Vec<(f64, f64, f64, f64)> = if line.len() == 2 { openings.get(&w.id).cloned().unwrap_or_default() } else { vec![] };
        // step 2237: a wall attached at its top follows the underside of its roof/floor
        let tops: Vec<&Vec<[P3; 3]>> = attach_top.get(&w.id).map(|ts| ts.iter().filter_map(|t| target_tris.get(t)).collect()).unwrap_or_default();
        if line.len() == 2 && (!holes.is_empty() || !tops.is_empty()) {
            let (a, c) = ([line[0][0], line[0][1], 0.0], [line[1][0], line[1][1], 0.0]);
            if let Some(d) = unit(sub(c, a)) {
                let len = norm(sub(c, a));
                let z0 = base;
                let zmax = z0 + height;
                // top along the wall: the lowest target face above the base (a vertical ray), else
                // the wall height
                let top_at = |u: f64| -> f64 {
                    let p = add(a, mul(d, u));
                    let hit = tops
                        .iter()
                        .flat_map(|ts| ts.iter())
                        .filter_map(|t| ray_z(t, p[0], p[1]))
                        .filter(|z| *z > z0 + 0.1)
                        .fold(f64::INFINITY, f64::min);
                    if hit.is_finite() { hit } else { zmax }
                };
                let n = mul([-d[1], d[0], 0.0], t / 2.0);
                let at = |u: f64, z: f64, s: f64| -> P3 {
                    let p = add(a, add(mul(d, u), mul(n, s)));
                    [p[0], p[1], z]
                };
                // u breakpoints: ends, opening edges, and every ≤ 1 ft for an attached top
                let mut us: Vec<f64> = vec![0.0, len];
                for h in &holes {
                    us.extend([h.0.clamp(0.0, len), h.1.clamp(0.0, len)]);
                }
                if !tops.is_empty() {
                    let k = (len.ceil() as usize).max(1);
                    us.extend((1..k).map(|i| len * i as f64 / k as f64));
                }
                us.sort_by(|x, y| x.total_cmp(y));
                us.dedup_by(|x, y| (*x - *y).abs() < 1e-6);
                let tz: Vec<f64> = us.iter().map(|&u| if tops.is_empty() { zmax } else { top_at(u) }).collect();
                let hs: Vec<(f64, f64, f64, f64)> = holes
                    .iter()
                    .map(|&(u0, u1, h0, h1)| (u0.max(0.0), u1.min(len), h0.max(z0), h1))
                    .filter(|h| h.1 - h.0 > 1e-3 && h.3 - h.2 > 1e-3)
                    .collect();
                for (j, win) in us.windows(2).enumerate() {
                    let (ua, ub) = (win[0], win[1]);
                    let (ta, tb) = (tz[j], tz[j + 1]);
                    let um = (ua + ub) / 2.0;
                    let tmin = ta.min(tb);
                    let mut cuts: Vec<(f64, f64)> =
                        hs.iter().filter(|h| h.0 <= um && um <= h.1).map(|h| (h.2, h.3.min(tmin))).filter(|c| c.1 > c.0).collect();
                    cuts.sort_by(|x, y| x.0.total_cmp(&y.0));
                    // solid z ranges; the last one ends on the (sloped) top
                    let mut solid: Vec<(f64, f64, bool)> = Vec::new();
                    let mut zc = z0;
                    for (c0, c1) in cuts {
                        if c0 > zc + 1e-6 {
                            solid.push((zc, c0, false));
                        }
                        zc = zc.max(c1);
                    }
                    if ta.max(tb) > zc + 1e-6 {
                        solid.push((zc, tmin, true));
                    }
                    for (za, zb, last) in solid {
                        let (zta, ztb) = if last { (ta, tb) } else { (zb, zb) };
                        b.quad(at(ua, za, -1.0), at(ub, za, -1.0), at(ub, ztb, -1.0), at(ua, zta, -1.0));
                        b.quad(at(ub, za, 1.0), at(ua, za, 1.0), at(ua, zta, 1.0), at(ub, ztb, 1.0));
                        b.quad(at(ua, zta, -1.0), at(ub, ztb, -1.0), at(ub, ztb, 1.0), at(ua, zta, 1.0));
                        b.quad(at(ua, za, 1.0), at(ub, za, 1.0), at(ub, za, -1.0), at(ua, za, -1.0));
                    }
                }
                // wall ends and the jambs of each opening
                for (u, tt) in [(0.0, tz[0]), (len, tz[tz.len() - 1])] {
                    b.quad(at(u, z0, 1.0), at(u, z0, -1.0), at(u, tt, -1.0), at(u, tt, 1.0));
                }
                for h in &hs {
                    for u in [h.0, h.1] {
                        if u > 1e-6 && u < len - 1e-6 {
                            b.quad(at(u, h.2, -1.0), at(u, h.2, 1.0), at(u, h.3, 1.0), at(u, h.3, -1.0));
                        }
                    }
                }
                continue;
            }
        }
        for s in line.windows(2) {
            let (a, c) = ([s[0][0], s[0][1], base], [s[1][0], s[1][1], base]);
            let Some(d) = unit(sub(c, a)) else { continue };
            let n = mul([-d[1], d[0], 0.0], t / 2.0);
            let (a0, a1, c0, c1) = (sub(a, n), add(a, n), sub(c, n), add(c, n));
            let up = [0.0, 0.0, height];
            b.hexa([a0, c0, c1, a1], [add(a0, up), add(c0, up), add(c1, up), add(a1, up)]);
        }
    }

    if let Some((id, n0)) = prev_wall.take() {
        eprintln!("wall {id} {:.2}", wall_area(batches.get("Walls"), n0));
    }
    let slab = slab_into;
    for f in doc.floors().into_iter().filter(|f| !group_defs.contains(&f.id)) {
        let t = f.thickness.filter(|t| *t > 0.0).unwrap_or(0.5);
        // step 2239: a foundation slab is a Floor of category Structural Foundations
        let b = if doc.element(f.id).map(|e| e.category) == Some(-2001300) {
            batch!("Structural foundations", [0.58, 0.56, 0.54])
        } else {
            batch!("Floors", [0.62, 0.60, 0.56])
        };
        slab(b, &f.curves, f.top_z, t, true);
    }
    for f in doc.ceilings().into_iter().filter(|f| !group_defs.contains(&f.id)) {
        let t = f.thickness.filter(|t| *t > 0.0).unwrap_or(0.08);
        slab(batch!("Ceilings", [0.93, 0.92, 0.90]), &f.curves, f.top_z, t, false);
    }
    for r in doc.roofs().into_iter().filter(|r| !group_defs.contains(&r.id)) {
        roof_into(batch!("Roofs", [0.60, 0.30, 0.18]), &r);
    }

    let segs: Vec<Segment> = doc.segments().into_iter().filter(|s| !group_defs.contains(&s.id)).collect();
    for s in &segs {
        let cat = doc.element(s.id).map(|e| e.category);
        let (name, color) = segment_batch(cat);
        batch!(name, color).member(s.p0, s.p1, s.width, s.height, s.round);
    }
    // step 2248: pipes, cable trays and conduits like segments; flex ducts and pipes along their
    // points (diameter from the type not read yet: 0.5 ft resp. 0.1 ft)
    for c in doc.mep_curves_by_chain().into_iter().flat_map(|(_, _, v)| v) {
        let (name, color) = match c.kind {
            "pipe" => ("Pipes", [0.30, 0.70, 0.45]),
            "flex_pipe" => ("Pipes", [0.30, 0.70, 0.45]),
            _ => ("Cable trays and conduits", [0.85, 0.55, 0.20]),
        };
        if let (Some(p0), Some(p1), Some(w), Some(h), Some(r)) = (c.p0, c.p1, c.width, c.height, c.round) {
            batch!(name, color).member(p0, p1, w, h, r);
        } else if let Some(pts) = &c.points {
            for s in pts.windows(2) {
                batch!(name, color).member(s[0], s[1], 0.1, 0.1, true);
            }
        }
    }
    for f in doc.flex_by_chain().into_iter().flat_map(|(_, _, v)| v) {
        for s in f.points.windows(2) {
            batch!("Ducts", [0.35, 0.60, 0.80]).member(s[0], s[1], 0.5, 0.5, true);
        }
    }

    // Railings (§35): top rail along the path at the railing (or top rail) height, posts at most
    // 3 ft apart. The base is the railing's level plus its base offset when it has a level
    // (steps 2200, 2202: the path z is sometimes 0 for a railing on an upper level); otherwise
    // the path z.
    // Stairs (§33): per run, one block per tread along the walk path, each a riser high and as wide
    // as the run (step 722; 3 ft if unknown), rising by one riser per tread from the run's base
    // elevation (absolute, step 2211), or from the stair base counted over the runs.
    let stairs: Vec<_> = doc.stairs().into_iter().filter(|s| !group_defs.contains(&s.id)).collect();
    let runs: std::collections::HashMap<u64, rvt_model::StairsRun> = doc.stairs_runs().into_iter().map(|r| (r.id, r)).collect();
    for st in &stairs {
        let Some(zb) = st.base_z else { continue };
        if !(st.riser > 0.0 && st.tread > 0.0) {
            continue;
        }
        // step 2274: a single run without path geometry (snowdon 2120094: run 2234674 has neither
        // path lines nor a cached path) walks along the long side of the stair's stored box when
        // that box is the run (long side = risers × tread or one tread less, short side = width)
        let mut paths = run_paths(st);
        if paths.is_empty() && st.runs.len() == 1 {
            let width = runs.get(&st.runs[0]).map(|r| r.width).filter(|w| *w > 0.0);
            if let (Some(bb), Some(w)) = (doc.element(st.id).and_then(|e| e.bbox), width) {
                let (ex, ey) = (bb[3] - bb[0], bb[4] - bb[1]);
                let (long, short, along_x) = if ex >= ey { (ex, ey, true) } else { (ey, ex, false) };
                let near = |a: f64, b: f64| (a - b).abs() <= 0.15 * b.max(1e-6);
                let n = st.risers.max(1) as f64;
                if (near(long, n * st.tread) || near(long, (n - 1.0) * st.tread)) && (short - w).abs() < 0.5 {
                    let (cx, cy) = ((bb[0] + bb[3]) / 2.0, (bb[1] + bb[4]) / 2.0);
                    let path = if along_x { vec![[bb[0], cy, zb], [bb[3], cy, zb]] } else { vec![[cx, bb[1], zb], [cx, bb[4], zb]] };
                    paths.push((st.runs[0], path));
                }
            }
        }
        // step 2276: a non-monolithic run (run type: treads, tread thickness) draws its treads as
        // plates of that thickness and risers only when it has them; a monolithic run keeps blocks
        let rt = doc.stair_run_type(st.id).filter(|r| !r.monolithic && r.treads && r.tread_thickness > 1e-3 && r.tread_thickness < st.riser);
        let tread_lo = |z0: f64| rt.as_ref().map_or(z0, |r| z0 + st.riser - r.tread_thickness);
        let riser_plate = rt.as_ref().filter(|r| r.risers && r.riser_thickness > 1e-3).map(|r| r.riser_thickness);
        let b = batch!("Stairs", [0.70, 0.66, 0.60]);
        let mut k = 0usize;
        for (rid, path) in paths {
            let run = runs.get(&rid).filter(|r| r.width > 0.0 && r.width < 50.0 && r.base_elevation.is_finite());
            let half = run.map_or(1.5, |r| r.width / 2.0);
            if let Some(r) = run {
                k = ((r.base_elevation - zb) / st.riser).round().max(0.0) as usize;
            }
            // step 2242: a sketched run with riser lines (winders included): a tread between each
            // pair of consecutive riser lines, ordered along the walk path, left/right by the walking
            // direction (rac_basic U-stair 665557: 17 lines, 16 risers)
            let rlines: Vec<(P3, P3)> = st
                .run_geometry
                .get(&rid.to_string())
                .and_then(|g| g.lines.get("riser"))
                .map(|cs| cs.iter().filter_map(|c| Some(([c.p0?[0], c.p0?[1], 0.0], [c.p1?[0], c.p1?[1], 0.0]))).collect())
                .unwrap_or_default();
            if rlines.len() >= 2 && path.len() >= 2 {
                let flat: Vec<P3> = path.iter().map(|q| [q[0], q[1], 0.0]).collect();
                // (arc length, tangent) of the path point nearest to m
                let locate = |m: P3| -> (f64, P3) {
                    let (mut best, mut acc) = ((f64::INFINITY, 0.0, [1.0, 0.0, 0.0]), 0.0);
                    for sg in flat.windows(2) {
                        let d = sub(sg[1], sg[0]);
                        let l = norm(d);
                        if l < 1e-9 {
                            continue;
                        }
                        let u = mul(d, 1.0 / l);
                        let t = dot(sub(m, sg[0]), u).clamp(0.0, l);
                        let dist = norm(sub(m, add(sg[0], mul(u, t))));
                        if dist < best.0 {
                            best = (dist, acc + t, u);
                        }
                        acc += l;
                    }
                    (best.1, best.2)
                };
                let mut lines: Vec<(f64, P3, P3)> = rlines
                    .iter()
                    .map(|&(a, c)| {
                        let (s, u) = locate(mul(add(a, c), 0.5));
                        // left endpoint first
                        let left = cross(u, sub(a, c))[2] > 0.0;
                        if left { (s, a, c) } else { (s, c, a) }
                    })
                    .collect();
                lines.sort_by(|x, y| x.0.total_cmp(&y.0));
                for w in lines.windows(2) {
                    if k as i64 >= st.risers {
                        break;
                    }
                    let z0 = zb + st.riser * k as f64;
                    let mut q = [w[0].1, w[0].2, w[1].2, w[1].1];
                    // counter-clockwise from above
                    let area2: f64 = (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum();
                    if area2 < 0.0 {
                        q.reverse();
                    }
                    let bot = q.map(|v| [v[0], v[1], tread_lo(z0)]);
                    b.hexa(bot, bot.map(|v| [v[0], v[1], z0 + st.riser]));
                    if let (Some(rth), true) = (riser_plate, rt.is_some()) {
                        // riser under the front edge (q[0] → q[3] is the first riser line)
                        if let Some(dir) = unit(sub(mul(add(q[1], q[2]), 0.5), mul(add(q[0], q[3]), 0.5))) {
                            let f = |v: P3, z: f64| [v[0], v[1], z];
                            let (p0, p3) = (q[0], q[3]);
                            let (p1, p2) = (add(p0, mul(dir, rth)), add(p3, mul(dir, rth)));
                            b.hexa([f(p0, z0), f(p1, z0), f(p2, z0), f(p3, z0)], [f(p0, tread_lo(z0)), f(p1, tread_lo(z0)), f(p2, tread_lo(z0)), f(p3, tread_lo(z0))]);
                        }
                    }
                    k += 1;
                }
                continue;
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
                let lo = [0.0, 0.0, tread_lo(z0) - z0];
                b.hexa([add(a0, lo), add(c0, lo), add(c1, lo), add(a1, lo)], [add(a0, up), add(c0, up), add(c1, up), add(a1, up)]);
                if let (Some(rth), true) = (riser_plate, rt.is_some()) {
                    let fwd = mul(dir, rth);
                    b.hexa([a0, add(a0, fwd), add(a1, fwd), a1], [add(a0, lo), add(add(a0, fwd), lo), add(add(a1, fwd), lo), add(a1, lo)]);
                }
                k += 1;
            }
        }
    }
    // Stair supports (step 2276; cat −2000123: stringers, carriages): the elements the stair record
    // references, each with its stored box, the run it carries (first cat −2000919 reference that is
    // one of the stair's runs) and its type (first cat −2000123 reference without a box: Structural
    // Depth On Run f64 @143, API snowdon 'Carriage - 24" Width' 0.333). A beam in the plan box along
    // its long side, rising from the run's base to its top elevation, the depth measured vertically.
    for st in &stairs {
        let Some(sb) = doc.element_body(st.id) else { continue };
        let refs: Vec<u64> = (150..sb.len().saturating_sub(8)).map(|o| i64_at_le(&sb, o)).filter(|&q| q > 0).map(|q| q as u64).collect();
        let mut seen = std::collections::HashSet::new();
        for sup in refs.into_iter().filter(|q| seen.insert(*q)) {
            let Some(se) = doc.element(sup).filter(|e| e.category == -2000123) else { continue };
            let Some(bb) = se.bbox.filter(|b| b.iter().all(|v| v.is_finite() && v.abs() < 1e6) && b[3] > b[0] && b[4] > b[1]) else { continue };
            let Some(body) = doc.element_body(sup) else { continue };
            let ids: Vec<u64> = (150..body.len().min(400).saturating_sub(8)).map(|o| i64_at_le(&body, o)).filter(|&q| q > 0).map(|q| q as u64).collect();
            let Some(run) = ids.iter().copied().find(|q| st.runs.contains(q)).and_then(|q| runs.get(&q)) else { continue };
            let depth = ids
                .iter()
                .copied()
                .filter(|q| doc.element(*q).is_some_and(|e| e.category == -2000123 && e.bbox.is_none()))
                .find_map(|q| doc.element_body(q).filter(|b| b.len() >= 151).map(|b| f64::from_le_bytes(b[143..151].try_into().unwrap())))
                .filter(|d| d.is_finite() && *d > 0.01 && *d < 5.0)
                .unwrap_or(0.5);
            let (ex, ey) = (bb[3] - bb[0], bb[4] - bb[1]);
            let along_x = ex >= ey;
            // low end: the end nearer to the run's start
            let start = run_paths(st).into_iter().find(|(r, _)| *r == run.id).and_then(|(_, p)| p.first().copied());
            let (lo_end, hi_end) = if along_x { (bb[0], bb[3]) } else { (bb[1], bb[4]) };
            let flip = start.is_some_and(|p0| {
                let c = if along_x { p0[0] } else { p0[1] };
                (c - hi_end).abs() < (c - lo_end).abs()
            });
            // under the treads: from the run's base to one riser below its top (the last riser
            // lands on the landing or floor)
            let (z0, z1) = (run.base_elevation, run.top_elevation - st.riser.max(0.0));
            if !(z1 > z0) {
                continue;
            }
            let (w0, w1) = if along_x { (bb[1], bb[4]) } else { (bb[0], bb[3]) };
            let pt = |t: f64, w: f64, z: f64| -> P3 { if along_x { [t, w, z] } else { [w, t, z] } };
            let (ta, tb) = if flip { (hi_end, lo_end) } else { (lo_end, hi_end) };
            // top line from (ta, z0) to (tb, z1); bottom `depth` lower
            let bot = [pt(ta, w0, z0 - depth), pt(tb, w0, z1 - depth), pt(tb, w1, z1 - depth), pt(ta, w1, z0 - depth)];
            let top = [pt(ta, w0, z0), pt(tb, w0, z1), pt(tb, w1, z1), pt(ta, w1, z0)];
            batch!("Stairs", [0.70, 0.66, 0.60]).hexa(bot, top);
        }
    }
    // Slab edges (step 2284; cat −2001392): the type's profile family sketch (the first symbol the
    // type references), sized by the type: an axis whose sketch extent equals a current value of
    // the profile family takes the symbol's value of the same parameter name (snowdon 'X' 0.0667 →
    // 0.0208, 'Y' 0.0833 → 1.0; GT plate 0.021 × 1.0 ft). Profile u runs out from the floor edge
    // (right of the segment direction), v down from the edge top + vertical offset (GT −0.75..+0.25
    // with offset 0.25). Lines are swept straight, arcs in steps of ≤ 0.5 ft; corners are not mitred.
    {
        let edges: Vec<rvt_model::SlabEdge> = doc.slab_edges().into_iter().filter(|e| !group_defs.contains(&e.id)).collect();
        if !edges.is_empty() {
            let fam_of = doc.symbol_family();
            let fam_chain = doc.family_chains();
            let gnames = doc.parameter_names_global();
            let rows: std::collections::HashMap<u64, rvt_model::ValueSet> = doc.type_values_by_chain().into_iter().flat_map(|(_, _, m)| m.into_iter()).collect();
            let mut profiles: std::collections::HashMap<u64, Option<Vec<[f64; 2]>>> = std::collections::HashMap::new();
            for e in &edges {
                let Some(t) = e.edge_type else { continue };
                let prof = profiles
                    .entry(t)
                    .or_insert_with(|| type_profile(doc, t, &fam_of, &fam_chain, &gnames, &rows))
                    .clone();
                let Some(prof) = prof else { continue };
                let b = batch!("Slab edges", [0.62, 0.60, 0.56]);
                let k = prof.len();
                let c = prof.iter().fold([0.0, 0.0], |a, q| [a[0] + q[0] / k as f64, a[1] + q[1] / k as f64]);
                // v down (snowdon: profile 0..1 hangs below the edge top + offset) or up (rac_advanced
                // 2019 217842: profile 0..−0.98, stored box from top − 0.98): the stored box's bottom
                // is the profile's bottom; the sign that puts it there (down without a box)
                let (vlo, vhi) = prof.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), q| (l.min(q[1]), h.max(q[1])));
                let top = e.segments.first().and_then(|c| c.p0).map_or(0.0, |p| p[2]) + e.vertical_offset;
                let vs = match doc.element(e.id).and_then(|x| x.bbox) {
                    Some(bb) if ((top + vlo) - bb[2]).abs() + 1e-9 < ((top - vhi) - bb[2]).abs() => 1.0,
                    _ => -1.0,
                };
                for cv in &e.segments {
                    // samples along the piece: (point, horizontal tangent)
                    let samples: Vec<(P3, P3)> = match (cv.kind, cv.p0, cv.p1, cv.center, cv.x, cv.y, cv.r, cv.t0, cv.t1) {
                        ("arc", _, _, Some(cc), Some(x), Some(y), Some(r), Some(t0), Some(t1)) => {
                            let n = (((t1 - t0) * r / 0.5).ceil() as usize).clamp(2, 128);
                            (0..=n)
                                .map(|j| {
                                    let t = t0 + (t1 - t0) * j as f64 / n as f64;
                                    let pt = add(cc, add(mul(x, r * t.cos()), mul(y, r * t.sin())));
                                    (pt, add(mul(x, -t.sin()), mul(y, t.cos())))
                                })
                                .collect()
                        }
                        (_, Some(p0), Some(p1), ..) => match unit(sub(p1, p0)) {
                            Some(d) => vec![(p0, d), (p1, d)],
                            None => continue,
                        },
                        _ => continue,
                    };
                    let frame = |(pt, d): (P3, P3), q: [f64; 2]| -> Option<P3> {
                        let n = unit([d[1], -d[0], 0.0])?;
                        Some(add(add(pt, mul(n, e.horizontal_offset + q[0])), [0.0, 0.0, e.vertical_offset + vs * q[1]]))
                    };
                    for w in samples.windows(2) {
                        for j in 0..k {
                            let (p, q) = (prof[j], prof[(j + 1) % k]);
                            if let (Some(a0), Some(b0), Some(b1), Some(a1)) = (frame(w[0], p), frame(w[0], q), frame(w[1], q), frame(w[1], p)) {
                                b.quad(a0, b0, b1, a1);
                            }
                        }
                    }
                    for (end, flip) in [(samples[0], false), (samples[samples.len() - 1], true)] {
                        for j in 0..k {
                            let (p, q) = (prof[j], prof[(j + 1) % k]);
                            if let (Some(cp), Some(pp), Some(qp)) = (frame(end, c), frame(end, p), frame(end, q)) {
                                if flip {
                                    b.tri(cp, pp, qp);
                                } else {
                                    b.tri(cp, qp, pp);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Wall sweeps (step 2272; cat −2000181, S719 + segments): per straight segment a rectangular
    // section, as high as the stored box (sweeps run level) and as wide as the type's profile (its
    // profile family's sketch, the symbol @155 of the sweep type), else the stored box across a
    // single segment; laid from the path towards the host wall. Segments outside the stored box are
    // other blocks of the record. An approximation: the profile's outline is not followed yet.
    {
        let fam_of = doc.symbol_family();
        let fam_chain = doc.family_chains();
        let wall_line: std::collections::HashMap<i64, (P3, P3)> = walls_all.iter().map(|w| (w.id as i64, (w.p0, w.p1))).collect();
        let mut prof_width: std::collections::HashMap<i64, Option<f64>> = std::collections::HashMap::new();
        for sw in doc.wall_sweeps().into_iter().filter(|s| s.cat == -2000181 && !group_defs.contains(&s.id)) {
            let Some(bb) = doc.element(sw.id).and_then(|e| e.bbox) else { continue };
            let width = sw.sweep_type.filter(|t| *t > 0).and_then(|t| {
                *prof_width.entry(t).or_insert_with(|| {
                    let b = doc.element_body(t as u64)?;
                    if b.len() < 163 {
                        return None;
                    }
                    let sym = i64::from_le_bytes(b[155..163].try_into().ok()?);
                    let key = fam_of.get(&(sym as u64)).and_then(|f| fam_chain.get(f))?;
                    let xs: Vec<f64> = doc.chain_curves(&key.0, key.1).iter().flat_map(|(_, c)| c.p0.into_iter().chain(c.p1).chain(c.center)).map(|p| p[0]).collect();
                    let (lo, hi) = (xs.iter().copied().fold(f64::INFINITY, f64::min), xs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
                    (hi - lo > 1e-3 && hi - lo < 10.0).then_some(hi - lo)
                })
            });
            let inside = |p: P3| (0..3).all(|k| p[k] >= bb[k] - 0.1 && p[k] <= bb[k + 3] + 0.1);
            let segs: Vec<(P3, P3)> = sw
                .segments
                .iter()
                .filter(|c| c.kind == "line")
                .filter_map(|c| Some((c.p0?, c.p1?)))
                .filter(|(a, b)| inside(*a) && inside(*b) && (a[2] - b[2]).abs() < 1e-6 && norm(sub(*b, *a)) > 0.05)
                .collect();
            if segs.is_empty() {
                continue;
            }
            let host = sw.hosts.first().and_then(|h| wall_line.get(h)).copied();
            for (a, b) in &segs {
                let Some(d) = unit(sub([b[0], b[1], 0.0], [a[0], a[1], 0.0])) else { continue };
                let mut n = [-d[1], d[0], 0.0];
                let mid = mul(add(*a, *b), 0.5);
                if let Some((w0, w1)) = host {
                    // towards the host wall's line
                    let wm = mul(add(w0, w1), 0.5);
                    let to_wall = dot(sub([wm[0], wm[1], 0.0], [mid[0], mid[1], 0.0]), n);
                    let to_line = {
                        let wd = unit(sub([w1[0], w1[1], 0.0], [w0[0], w0[1], 0.0])).unwrap_or(d);
                        let rel = sub([mid[0], mid[1], 0.0], [w0[0], w0[1], 0.0]);
                        let foot = add([w0[0], w0[1], 0.0], mul(wd, dot(rel, wd)));
                        dot(sub(foot, [mid[0], mid[1], 0.0]), n)
                    };
                    let s = if to_line.abs() > 1e-6 { to_line } else { to_wall };
                    if s < 0.0 {
                        n = mul(n, -1.0);
                    }
                }
                // width: the profile's, else the stored box across this (only) segment
                let w = width.or_else(|| {
                    (segs.len() == 1).then(|| {
                        let k = if d[0].abs() > d[1].abs() { 1 } else { 0 };
                        bb[k + 3] - bb[k]
                    })
                });
                let Some(w) = w.filter(|w| *w > 1e-3) else { continue };
                let (z0, z1) = (bb[2], bb[5]);
                let at = |p: P3, s: f64, z: f64| [p[0] + n[0] * s, p[1] + n[1] * s, z];
                batch!("Wall sweeps", [0.80, 0.78, 0.74]).hexa([at(*a, 0.0, z0), at(*b, 0.0, z0), at(*b, w, z0), at(*a, w, z0)], [at(*a, 0.0, z1), at(*b, 0.0, z1), at(*b, w, z1), at(*a, w, z1)]);
            }
        }
    }
    // Topography (step 2233): Delaunay triangulation of the terrain points in plan, like Revit
    // regenerates the surface from its points.
    // step 2233: hidden group type definition copies (rac_basic: a second terrain over the same area)
    let inplace = doc.inplace_ids();
    let hidden: std::collections::HashSet<u64> = doc.group_definition_ids().union(&inplace).copied().collect();
    // Building pads (step 2234): a slab of the type thickness under level + offset; the terrain it
    // cuts gets its outline (points every ≤ 1 ft, z from the terrain) and loses the triangles inside;
    // vertical pit walls run from the slab bottom up to the terrain. The pad's own cut surface is
    // not drawn (Revit: pit bottom = slab bottom, sides vertical).
    let pads: Vec<(rvt_model::BuildingPad, Vec<Vec<[f64; 2]>>, f64, f64)> = doc
        .building_pads()
        .into_iter()
        .filter_map(|p| {
            let (groups, _) = plan_loops(p.outline.as_deref()?);
            let outer = groups.into_iter().next()?.into_iter().next()?;
            let top = p.level.and_then(|l| doc.level_elevation(l))? + p.height_offset.unwrap_or(0.0);
            let th = p.r#type.and_then(|t| doc.type_thickness(t)).filter(|t| *t > 0.0).unwrap_or(1.0);
            Some((p, vec![outer], top, th))
        })
        .collect();
    let cut_surfaces: std::collections::HashSet<i64> = pads.iter().filter_map(|(p, ..)| p.cut_surface).collect();
    for t in doc.topography().into_iter().filter(|t| !hidden.contains(&t.id) && !cut_surfaces.contains(&(t.id as i64))) {
        let base = delaunator::triangulate(&t.points.iter().map(|p| delaunator::Point { x: p[0], y: p[1] }).collect::<Vec<_>>());
        if base.triangles.is_empty() {
            continue;
        }
        // terrain height in plan from the plain triangulation
        let z_at = |x: f64, y: f64| -> Option<f64> {
            base.triangles.chunks_exact(3).find_map(|c| {
                let (a, b, c) = (t.points[c[0]], t.points[c[1]], t.points[c[2]]);
                let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
                if d.abs() < 1e-12 {
                    return None;
                }
                let l1 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / d;
                let l2 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / d;
                let l3 = 1.0 - l1 - l2;
                (l1 > -1e-9 && l2 > -1e-9 && l3 > -1e-9).then(|| l1 * a[2] + l2 * b[2] + l3 * c[2])
            })
        };
        let mine: Vec<&(rvt_model::BuildingPad, Vec<Vec<[f64; 2]>>, f64, f64)> =
            pads.iter().filter(|(p, ..)| p.topo == Some(t.id as i64)).collect();
        let mut pts = t.points.clone();
        let mut rims: Vec<Vec<P3>> = Vec::new();
        for (_, loops, ..) in &mine {
            let l = &loops[0];
            let mut rim = Vec::new();
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                let n = ((((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()).ceil() as usize).max(1);
                for k in 0..n {
                    let f = k as f64 / n as f64;
                    let (x, y) = (a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f);
                    if let Some(z) = z_at(x, y) {
                        rim.push([x, y, z]);
                    }
                }
            }
            pts.extend(rim.iter().copied());
            rims.push(rim);
        }
        let tri = if mine.is_empty() { base } else { delaunator::triangulate(&pts.iter().map(|p| delaunator::Point { x: p[0], y: p[1] }).collect::<Vec<_>>()) };
        let b = batch!("Topography", [0.56, 0.64, 0.44]);
        for c in tri.triangles.chunks_exact(3) {
            let (p, q, r) = (pts[c[0]], pts[c[1]], pts[c[2]]);
            let m = [(p[0] + q[0] + r[0]) / 3.0, (p[1] + q[1] + r[1]) / 3.0];
            if mine.iter().any(|(_, loops, ..)| point_in(m, &loops[0])) {
                continue;
            }
            b.tri(p, r, q);
        }
        // pit walls from the slab bottom up to the terrain along the outline
        for ((_, _, top, th), rim) in mine.iter().copied().zip(&rims) {
            let zb = top - th;
            for i in 0..rim.len() {
                let (a, c) = (rim[i], rim[(i + 1) % rim.len()]);
                if a[2] > zb || c[2] > zb {
                    b.quad([a[0], a[1], zb], [c[0], c[1], zb], [c[0], c[1], c[2].max(zb)], [a[0], a[1], a[2].max(zb)]);
                }
            }
        }
    }
    for (_, loops, top, th) in &pads {
        let b = batch!("Pads", [0.66, 0.60, 0.52]);
        b.prism(loops, top - th, *top);
    }
    // Wall foundations (step 2256; class 918, category Structural Foundations): a footing under its
    // wall. The wall is the one whose line lies in the footing's stored box and whose base meets its
    // top; the footing runs along that line over the box, as wide as the box across an axis-aligned
    // wall (else the width of the axis-aligned footings), as thick as the box.
    {
        let fnds: Vec<[f64; 6]> = doc
            .elements_in_category(-2001300)
            .filter(|e| e.class_tag.is_some() && e.class_tag == doc.class_tag(918))
            .filter_map(|e| e.bbox)
            .collect();
        let mut widths = Vec::new();
        let mut placed: Vec<(P3, P3, f64, f64, f64, Option<f64>)> = Vec::new(); // a, b, z0, z1, axis-aligned width
        for bb in &fnds {
            let inside = |q: P3| q[0] >= bb[0] - 2.0 && q[0] <= bb[3] + 2.0 && q[1] >= bb[1] - 2.0 && q[1] <= bb[4] + 2.0;
            let Some(w) = walls_all.iter().find(|w| {
                let zb = doc.level_elevation(w.base_level).map(|z| z + w.base_offset).unwrap_or(w.p0[2]);
                // the wall line spans the footing box in both directions (an oblique footing's box is
                // its diagonal; a straight one's width is at most a few feet)
                let spans = w.p0[0].min(w.p1[0]) <= bb[0] + 4.5 && w.p0[0].max(w.p1[0]) >= bb[3] - 4.5 && w.p0[1].min(w.p1[1]) <= bb[1] + 4.5 && w.p0[1].max(w.p1[1]) >= bb[4] - 4.5;
                inside(w.p0) && inside(w.p1) && spans && (zb - bb[5]).abs() < 0.6
            }) else {
                continue;
            };
            let a = [w.p0[0], w.p0[1], 0.0];
            let Some(d) = unit(sub([w.p1[0], w.p1[1], 0.0], a)) else { continue };
            let corners = [[bb[0], bb[1]], [bb[3], bb[1]], [bb[0], bb[4]], [bb[3], bb[4]]];
            let us: Vec<f64> = corners.iter().map(|c| dot(sub([c[0], c[1], 0.0], a), d)).collect();
            let (u0, u1) = (us.iter().copied().fold(f64::INFINITY, f64::min), us.iter().copied().fold(f64::NEG_INFINITY, f64::max));
            let width = if d[0].abs() > 0.999 {
                Some(bb[4] - bb[1])
            } else if d[1].abs() > 0.999 {
                Some(bb[3] - bb[0])
            } else {
                None
            };
            if let Some(wd) = width {
                widths.push(wd);
            }
            // the footing is centred on the wall line; for an oblique wall the box gives the length only
            let (pa, pb) = (add(a, mul(d, u0)), add(a, mul(d, u1)));
            let half = width.map(|wd| wd / 2.0);
            placed.push((pa, pb, bb[2], bb[5], 0.0, half));
        }
        widths.sort_by(|x, y| x.total_cmp(y));
        let typical = widths.get(widths.len() / 2).copied().unwrap_or(2.0);
        for (pa, pb, z0, z1, _, half) in placed {
            let half = half.unwrap_or(typical / 2.0);
            let Some(d) = unit(sub(pb, pa)) else { continue };
            // an oblique footing's box also covers its width at both ends; trim that
            let trim = if d[0].abs() > 0.999 || d[1].abs() > 0.999 { 0.0 } else { half * (d[0].abs().min(d[1].abs())) / d[0].abs().max(d[1].abs()) };
            let (pa, pb) = (add(pa, mul(d, trim)), sub(pb, mul(d, trim)));
            let n = [-d[1] * half, d[0] * half, 0.0];
            let at = |q: P3, s: f64, z: f64| [q[0] + n[0] * s, q[1] + n[1] * s, z];
            let bot = [at(pa, -1.0, z0), at(pb, -1.0, z0), at(pb, 1.0, z0), at(pa, 1.0, z0)];
            let top = [at(pa, -1.0, z1), at(pb, -1.0, z1), at(pb, 1.0, z1), at(pa, 1.0, z1)];
            if std::env::var("RVT_MESH_DEBUG_WF").is_ok() {
                eprintln!("wf {:.2?} -> {:.2?} half {:.2} z {:.2}..{:.2}", pa, pb, half, z0, z1);
            }
            batch!("Structural foundations", [0.58, 0.56, 0.54]).hexa(bot, top);
        }
    }
    // Stair landings (step 2241): a component stair's landing has no geometry of its own in the
    // file, but its stored box matches Revit's landing (rac_basic 725396); a slab under the box top,
    // at most 1 ft thick (Revit 0.98 ft)
    for e in doc.elements_in_category(-2000920) {
        let Some(bb) = e.bbox else { continue };
        let ext = [bb[3] - bb[0], bb[4] - bb[1], bb[5] - bb[2]];
        if ext.iter().any(|v| !(v.is_finite() && *v > 1e-3 && *v < 500.0)) {
            continue;
        }
        let (z1, z0) = (bb[5], bb[5] - ext[2].min(1.0));
        let bot = [[bb[0], bb[1], z0], [bb[3], bb[1], z0], [bb[3], bb[4], z0], [bb[0], bb[4], z0]];
        batch!("Stairs", [0.70, 0.66, 0.60]).hexa(bot, bot.map(|v| [v[0], v[1], z1]));
    }
    // Old sketch stairs (step 2231): run lines in walking order; the riser lines crossing a run,
    // sorted along it, each start a tread one riser high (rise = level difference / riser lines;
    // the last line of a run is the step onto the landing or floor). Landings: a slab along each
    // landing centre line, as wide as the run, extended by half that width at joints.
    // Step 2232: a multistory stair repeats per storey up to its multistory top level; the walking
    // surfaces (story base, piece) are kept for the railings below.
    // (storey base, piece, storey index, storeys, storey height)
    let mut walk: Vec<(f64, WalkPiece, usize, usize, f64)> = Vec::new();
    for st in doc.legacy_stairs() {
        let (Some(zb), Some(zt)) = (st.base_z, st.top_z) else { continue };
        let empty = Vec::new();
        let get = |k: &str| st.lines.get(k).unwrap_or(&empty);
        let lines = |cs: &Vec<rvt_model::Curve>| -> Vec<(P3, P3)> {
            cs.iter().filter_map(|c| Some(([c.p0?[0], c.p0?[1], 0.0], [c.p1?[0], c.p1?[1], 0.0]))).collect()
        };
        let (runs, risers, landings) = (lines(get("run")), lines(get("riser")), lines(get("landing_center")));
        if risers.is_empty() || !(zt > zb) {
            continue;
        }
        let rise = (zt - zb) / risers.len() as f64;
        let storey = zt - zb;
        let storeys = st.multistory_top_z.map_or(1, |zm| (((zm - zb) / storey).round() as usize).clamp(1, 50));
        // pieces relative to the stair base (z measured from zb)
        let mut pieces: Vec<WalkPiece> = Vec::new();
        let mut boxes: Vec<[P3; 4]> = Vec::new(); // bottom faces, z relative
        let mut k = 0usize;
        let mut half = 1.5;
        let mut first_landing: Option<f64> = None;
        for &(r0, r1) in &runs {
            let Some(dir) = unit(sub(r1, r0)) else { continue };
            let len = norm(sub(r1, r0));
            // risers crossing this run: (position along the run, riser)
            let mut on: Vec<(f64, (P3, P3))> = risers
                .iter()
                .filter_map(|&(a, c)| {
                    let m = mul(add(a, c), 0.5);
                    let t = dot(sub(m, r0), dir);
                    let lat = norm(sub(sub(m, r0), mul(dir, t)));
                    (t > -1e-3 && t < len + 1e-3 && lat < norm(sub(c, a)) / 2.0 + 1e-3).then_some((t, (a, c)))
                })
                .collect();
            on.sort_by(|x, y| x.0.total_cmp(&y.0));
            let k0 = k;
            for (j, &(t, (a, c))) in on.iter().enumerate() {
                half = norm(sub(c, a)) / 2.0;
                if let Some(&(t1, _)) = on.get(j + 1) {
                    let z0 = rise * k as f64;
                    let w = mul([-dir[1], dir[0], 0.0], half);
                    let (p, q) = (add(r0, mul(dir, t)), add(r0, mul(dir, t1)));
                    let (p, q) = ([p[0], p[1], z0], [q[0], q[1], z0]);
                    boxes.push([sub(p, w), sub(q, w), add(q, w), add(p, w)]);
                }
                k += 1;
            }
            if let (Some(&(t0, _)), Some(&(t1, _))) = (on.first(), on.last()) {
                // nosing line: top of the first tread up to the top of the landing or floor
                pieces.push(WalkPiece { a: add(r0, mul(dir, t0)), c: add(r0, mul(dir, t1)), half, z0: rise * (k0 + 1) as f64, z1: rise * k as f64 });
            }
            first_landing.get_or_insert(rise * (k as f64 - 1.0));
        }
        // the landing after the first run (the slab top meets the step onto it)
        if let Some(z0) = first_landing {
            for (i, &(a, c)) in landings.iter().enumerate() {
                let Some(dir) = unit(sub(c, a)) else { continue };
                let joined = |p: P3| landings.iter().enumerate().any(|(j, &(x, y))| j != i && (norm(sub(x, p)) < 1e-3 || norm(sub(y, p)) < 1e-3));
                let a = if joined(a) { sub(a, mul(dir, half)) } else { a };
                let c = if joined(c) { add(c, mul(dir, half)) } else { c };
                let w = mul([-dir[1], dir[0], 0.0], half);
                let (a, c) = ([a[0], a[1], z0], [c[0], c[1], z0]);
                boxes.push([sub(a, w), sub(c, w), add(c, w), add(a, w)]);
                pieces.push(WalkPiece { a, c, half, z0: z0 + rise, z1: z0 + rise });
            }
        }
        let b = batch!("Stairs", [0.70, 0.66, 0.60]);
        for s in 0..storeys {
            let base = zb + storey * s as f64;
            let lift = |v: P3| [v[0], v[1], v[2] + base];
            for bot in &boxes {
                let bot = bot.map(lift);
                b.hexa(bot, bot.map(|v| add(v, [0.0, 0.0, rise])));
            }
            walk.extend(pieces.iter().map(|p| (base, WalkPiece { z0: p.z0 + base, z1: p.z1 + base, ..*p }, s, storeys, storey)));
        }
    }
    let stair_by_id: std::collections::HashMap<u64, &Stair> = stairs.iter().map(|s| (s.id, s)).collect();

    let bits32 = doc.info().bits == Some(32);
    // Step 2285: top rails and handrails are their own elements (path + type profile, drawn
    // below); a railing with a real top rail keeps only its posts here
    let rails: Vec<rvt_model::Rail> = doc.rails().into_iter().filter(|r| !group_defs.contains(&r.id)).collect();
    let with_top_rail: std::collections::HashSet<u64> = rails.iter().filter(|r| r.cat == -2000946).filter_map(|r| r.host).collect();
    for r in doc.railings().into_iter().filter(|r| !group_defs.contains(&r.id)) {
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
        // step 2232: railings over an old sketch stair (their host is a helper element, not the
        // stair): a point above a run or landing of the same storey gets that walking surface; a
        // railing on the first storey of a multistory stair repeats with it
        if !r.host.is_some_and(|h| stair_by_id.contains_key(&(h as u64))) && !walk.is_empty() {
            let mut repeat: Option<(usize, f64)> = None;
            for poly in polys.iter_mut() {
                for p in poly.iter_mut() {
                    let best = walk
                        .iter()
                        .filter(|(base, ..)| (p[2] - base).abs() < 0.5)
                        .filter_map(|(_, w, s, n, h)| w.z_at(*p).map(|(ex, z)| (ex, z, *s, *n, *h)))
                        .min_by(|x, y| x.0.total_cmp(&y.0));
                    if let Some((_, z, s, n, h)) = best {
                        p[2] = z;
                        if s == 0 && n > 1 {
                            repeat = Some((n, h));
                        }
                    }
                }
            }
            if let Some((n, h)) = repeat {
                let first = polys.clone();
                for i in 1..n {
                    polys.extend(first.iter().map(|poly| poly.iter().map(|p| [p[0], p[1], p[2] + h * i as f64]).collect::<Vec<P3>>()));
                }
            }
        }
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
                if !with_top_rail.contains(&r.id) {
                    b.member(add(s[0], up), add(s[1], up), 0.15, 0.15, false);
                }
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

    // Rails (step 2285; top rails −2000946, handrails −2000947): the type's profile swept along
    // the path, the profile's x across (right of the path, horizontal), its y up (perpendicular to
    // the path in its vertical plane): snowdon handrail 621419, circle r 0.0625 centred 0.0625 below
    // the path, Revit's rail top on the path. Pieces are swept one by one, joints not mitred.
    if !rails.is_empty() {
        let fam_of = doc.symbol_family();
        let fam_chain = doc.family_chains();
        let gnames = doc.parameter_names_global();
        let rows: std::collections::HashMap<u64, rvt_model::ValueSet> = doc.type_values_by_chain().into_iter().flat_map(|(_, _, m)| m.into_iter()).collect();
        let mut profiles: std::collections::HashMap<u64, Option<Vec<[f64; 2]>>> = std::collections::HashMap::new();
        let b = batch!("Railings", [0.45, 0.47, 0.50]);
        for r in &rails {
            let Some(t) = r.rail_type else { continue };
            // a type without a profile family (snowdon 168367: no symbol, no sizes) gets a 1.5" round
            // rail under the path (assumption; the file's most used rail profile, GT 0.332 ft²/ft
            // against 0.393 for this circle)
            let prof = profiles
                .entry(t)
                .or_insert_with(|| {
                    type_profile(doc, t, &fam_of, &fam_chain, &gnames, &rows)
                        .or_else(|| Some((0..24).map(|j| { let a = std::f64::consts::TAU * j as f64 / 24.0; [0.0625 * a.cos(), -0.0625 + 0.0625 * a.sin()] }).collect()))
                })
                .clone()
                .unwrap_or_default();
            sweep_profile(b, &r.path, &prof, |pt, d, q| {
                let n = unit([d[1], -d[0], 0.0])?;
                let up = unit(cross(n, d))?;
                Some(add(add(pt, mul(n, q[0])), mul(up, q[1])))
            });
        }
    }

    let insts: &[Instance] = &insts_all;
    // Family geometry (§8.11, steps 2214–2216): extrusion solids of the instance's family, used when
    // they cover the element's stored box (±0.15 ft); otherwise the box itself.
    let symbol_family = doc.symbol_family();
    let family_chain = doc.family_chains();
    let adaptive_chains: std::collections::HashSet<(String, usize)> =
        doc.elements().filter(|e| e.category == -2000710).map(|e| (e.partition, e.chain)).collect();
    // Step 2258: a family document's group type keeps hidden definition copies of its members (header
    // slot = the GroupType, as in projects, step 2233); rac_basic 2026 solar panel: an arrayed cell's
    // copy 4 ft beside the panel. Those forms are not drawn.
    // (the form lists hold family chains only, so the ids of every chain can be used)
    let fam_group_defs: std::collections::HashSet<u64> = doc
        .group_definition_members_by_chain()
        .into_iter()
        .flat_map(|(_, _, v)| v.into_iter().map(|m| m.id))
        .collect();
    let chain_solids: std::collections::HashMap<(String, usize), Vec<ExtrusionSolid>> = doc
        .family_solids_by_chain()
        .into_iter()
        .map(|(st, ix, mut v)| {
            v.retain(|s| !fam_group_defs.contains(&s.id));
            ((st, ix), v)
        })
        .collect();
    let chain_sweeps: std::collections::HashMap<(String, usize), Vec<SweepSolid>> = doc
        .family_sweeps_by_chain()
        .into_iter()
        .map(|(st, ix, mut v)| {
            v.retain(|s| !fam_group_defs.contains(&s.id));
            ((st, ix), v)
        })
        .collect();
    let chain_blrev: std::collections::HashMap<(String, usize), (Vec<BlendSolid>, Vec<RevolveSolid>)> = doc
        .family_blends_revolves_by_chain()
        .into_iter()
        .map(|(st, ix, mut b, mut r)| {
            b.retain(|x| !fam_group_defs.contains(&x.id));
            r.retain(|x| !fam_group_defs.contains(&x.id));
            ((st, ix), (b, r))
        })
        .collect();
    // Step 2268: a form whose Visible is bound to a yes/no family parameter (S2268) is hidden when
    // that parameter is 0 for the type: the symbol's value (matched by parameter name), else the
    // family's current value (snowdon door family 439: 'Masonry Frame' 1 / 'Dry Wall Frame' 0,
    // two frame sweeps; Revit draws one).
    let vis_assoc: std::collections::HashMap<u64, u64> = doc
        .family_param_associations()
        .into_iter()
        .filter_map(|(id, l)| l.iter().find(|(b, _)| *b == -1006205).map(|(_, f)| (id, *f)))
        .collect();
    let sym_rows: std::collections::HashMap<u64, rvt_model::ValueSet> = if vis_assoc.is_empty() {
        Default::default()
    } else {
        doc.type_values_by_chain().into_iter().flat_map(|(_, _, m)| m.into_iter()).collect()
    };
    let vis_names = if vis_assoc.is_empty() { Default::default() } else { doc.parameter_names_global() };
    let hidden_cache: std::cell::RefCell<std::collections::HashMap<((String, usize), Option<u64>, Option<u64>), std::rc::Rc<std::collections::HashSet<u64>>>> = Default::default();
    // step 2283: `host` = the type of the family a nested instance sits in; its value counts when
    // the nested type has none (snowdon canopy type 'Include Dome' 0, linked through the nested
    // instance to the support family's 'Include Dome', whose own current value is 1)
    let hidden_forms = |key: &(String, usize), sym: Option<u64>, host: Option<u64>| -> std::rc::Rc<std::collections::HashSet<u64>> {
        if let Some(h) = hidden_cache.borrow().get(&(key.clone(), sym, host)) {
            return h.clone();
        }
        let ids: Vec<(u64, u64)> = chain_solids
            .get(key)
            .into_iter()
            .flatten()
            .map(|s| s.id)
            .chain(chain_sweeps.get(key).into_iter().flatten().map(|s| s.id))
            // step 2283: blends and revolves too (snowdon canopy support family: a dome revolve,
            // Visible bound to a parameter that is off; Revit draws no dome)
            .chain(chain_blrev.get(key).into_iter().flat_map(|(b, r)| b.iter().map(|x| x.id).chain(r.iter().map(|x| x.id))))
            .filter_map(|id| vis_assoc.get(&id).map(|f| (id, *f)))
            .collect();
        let mut hidden = std::collections::HashSet::new();
        if !ids.is_empty() {
            let fnames = doc.parameter_names_of(&key.0, key.1);
            let current = doc.family_current_values(&key.0, key.1);
            for (id, fp) in ids {
                let name = fnames.get(&fp).cloned().flatten();
                let by_name = |s: u64| {
                    sym_rows.get(&s).and_then(|vs| {
                        vs.rows.iter().find(|r| r.param > 0 && name.is_some() && vis_names.get(&(r.param as u64)).cloned().flatten() == name).map(|r| r.int)
                    })
                };
                let from_type = sym.and_then(by_name).or_else(|| host.and_then(by_name));
                let value = from_type.or_else(|| current.as_ref().and_then(|c| c.rows.iter().find(|r| r.param == fp as i64).map(|r| r.int)));
                if value == Some(0) {
                    hidden.insert(id);
                }
            }
        }
        let h = std::rc::Rc::new(hidden);
        hidden_cache.borrow_mut().insert((key.clone(), sym, host), h.clone());
        h
    };
    let solids_of = |sym: u64| -> Vec<&ExtrusionSolid> {
        let Some(k) = symbol_family.get(&sym).and_then(|f| family_chain.get(f)) else { return vec![] };
        let hid = hidden_forms(k, Some(sym), None);
        chain_solids
            .get(k)
            .map(|v| finest_variants(v.iter().filter(|s| s.void != Some(true) && s.host_block.is_none() && !hid.contains(&s.id)).collect()))
            .unwrap_or_default()
    };
    // step 2249: columns are never face-based; a column family whose only extrusion carries the
    // host block mark (rme 2026 structural columns) shows that extrusion
    let column_solids_of = |sym: u64| -> Vec<&ExtrusionSolid> {
        let shown = solids_of(sym);
        if !shown.is_empty() {
            return shown;
        }
        symbol_family
            .get(&sym)
            .and_then(|f| family_chain.get(f))
            .and_then(|k| chain_solids.get(k))
            .map(|v| finest_variants(v.iter().filter(|s| s.void != Some(true)).collect()))
            .unwrap_or_default()
    };
    let blrev_of = |sym: u64| -> Vec<[P3; 3]> {
        let mut t = Vec::new();
        let Some(k) = symbol_family.get(&sym).and_then(|f| family_chain.get(f)) else { return t };
        let hid = hidden_forms(k, Some(sym), None);
        if let Some((bl, rv)) = chain_blrev.get(k) {
            for b in bl.iter().filter(|b| b.void != Some(true) && !hid.contains(&b.id)) {
                t.extend(blend_triangles(b));
            }
            for r in rv.iter().filter(|r| r.void != Some(true) && !hid.contains(&r.id)) {
                t.extend(revolve_triangles(r));
            }
        }
        t
    };
    let sweeps_of = |sym: u64| -> Vec<&SweepSolid> {
        let Some(k) = symbol_family.get(&sym).and_then(|f| family_chain.get(f)) else { return vec![] };
        let hid = hidden_forms(k, Some(sym), None);
        chain_sweeps.get(k).map(|v| finest_sweeps(v.iter().filter(|s| s.void != Some(true) && !hid.contains(&s.id)).collect())).unwrap_or_default()
    };
    let (mut n_family, mut n_box) = (0usize, 0usize);
    let mut box_by_cat: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
    // step 2227: family shapes per symbol are computed once (many instances share a type)
    // Step 2246 (§10): nested families. Instances inside a family document (placed in the host
    // family's coordinates) add their own family's forms, recursively (depth ≤ 3).
    let nested_symbol_family = doc.nested_symbol_family();
    let nested_family_chain = doc.nested_family_chains();
    let chain_insts: std::collections::HashMap<(String, usize), Vec<Instance>> =
        doc.family_chain_instances()
            .into_iter()
            .map(|(st, ix, mut v)| {
                for i in v.iter_mut() {
                    i.rotation = flipped_rotation(i);
                }
                ((st, ix), v)
            })
            .collect();
    let chain_of_symbol = |sym: u64| -> Option<(String, usize)> {
        let f = symbol_family.get(&sym).or_else(|| nested_symbol_family.get(&sym))?;
        family_chain.get(f).or_else(|| nested_family_chain.get(f)).cloned()
    };
    let chain_forms = |key: &(String, usize), sym: Option<u64>, host: Option<u64>| -> Vec<[P3; 3]> {
        let mut t = Vec::new();
        // step 2268: nested families by their current values; step 2283: the nested instance's
        // type first (snowdon canopy: support type 'Include Dome' = 0 hides the dome revolve)
        let hid = hidden_forms(key, sym, host);
        if let Some(v) = chain_solids.get(key) {
            let sol: Vec<&ExtrusionSolid> = finest_variants(v.iter().filter(|s| s.void != Some(true) && s.host_block.is_none() && !hid.contains(&s.id)).collect());
            t.extend(family_triangles(&sol));
        }
        if let Some(v) = chain_sweeps.get(key) {
            for sw in finest_sweeps(v.iter().filter(|s| s.void != Some(true) && !hid.contains(&s.id)).collect()) {
                t.extend(sweep_triangles(sw));
            }
        }
        if let Some((bl, rv)) = chain_blrev.get(key) {
            for b in bl.iter().filter(|b| b.void != Some(true) && !hid.contains(&b.id)) {
                t.extend(blend_triangles(b));
            }
            for r in rv.iter().filter(|r| r.void != Some(true) && !hid.contains(&r.id)) {
                t.extend(revolve_triangles(r));
            }
        }
        t
    };
    fn nested_tris(
        key: &(String, usize),
        depth: usize,
        chain_insts: &std::collections::HashMap<(String, usize), Vec<Instance>>,
        chain_of_symbol: &dyn Fn(u64) -> Option<(String, usize)>,
        chain_forms: &dyn Fn(&(String, usize), Option<u64>, Option<u64>) -> Vec<[P3; 3]>,
        host: Option<u64>,
    ) -> Vec<[P3; 3]> {
        let mut out = Vec::new();
        if depth > 3 {
            return out;
        }
        for i in chain_insts.get(key).into_iter().flatten() {
            let (Some(sym), Some(o), Some(m)) = (i.symbol, i.origin, i.rotation) else { continue };
            let Some(k2) = chain_of_symbol(sym).filter(|k2| k2 != key) else { continue };
            let mut t = chain_forms(&k2, Some(sym), host);
            t.extend(nested_tris(&k2, depth + 1, chain_insts, chain_of_symbol, chain_forms, Some(sym)));
            // a mirrored placement keeps the faces outward by reversing the winding
            let rev = det3(&m) < 0.0;
            out.extend(t.into_iter().map(|tr| {
                let (a, b) = if rev { (tr[2], tr[0]) } else { (tr[0], tr[2]) };
                [place(o, &m, a), place(o, &m, tr[1]), place(o, &m, b)]
            }));
        }
        out
    }
    let mut shape_cache: std::collections::HashMap<u64, std::rc::Rc<Vec<Vec<[P3; 3]>>>> = std::collections::HashMap::new();
    let mut plain_count: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
    let linked = doc.sub_component_links();
    // Step 2262: hosts of shared nested components: a linked instance whose stored box contains
    // another linked instance's box. Their stored box is the union with the components (snowdon
    // kitchen unit 908758: 8.5 × 4.9 × 5.3 ft, its own forms 4.5 × 2.1 × 2.8 ft = Revit's), so it
    // is no size to fit to.
    let linked_hosts: std::collections::HashSet<u64> = {
        let boxes: Vec<(u64, [f64; 6])> = linked.iter().filter_map(|&id| doc.element(id).and_then(|e| e.bbox).map(|b| (id, b))).collect();
        boxes
            .iter()
            .filter(|(i, a)| {
                boxes.iter().any(|(j, b)| {
                    j != i && (0..3).all(|k| b[k] >= a[k] - 0.05 && b[k + 3] <= a[k + 3] + 0.05) && (0..3).any(|k| b[k + 3] - b[k] < a[k + 3] - a[k] - 0.1)
                })
            })
            .map(|(i, _)| *i)
            .collect()
    };
    let columns: std::collections::HashMap<u64, rvt_model::Column> = doc.columns().into_iter().map(|c| (c.id, c)).collect();
    // Step 2253: column type sizes. Type values per symbol (with the project chain for parameter
    // names) and, per family chain, its value sets (the family's types).
    let mut sym_values: std::collections::HashMap<u64, (rvt_model::ValueSet, (String, usize))> = std::collections::HashMap::new();
    for (st, ix, tv) in doc.type_values_by_chain() {
        for (k, v) in tv {
            sym_values.insert(k, (v, (st.clone(), ix)));
        }
    }
    let chain_sets: std::cell::RefCell<std::collections::HashMap<(String, usize), Vec<rvt_model::ValueSet>>> = Default::default();
    let chain_names: std::cell::RefCell<std::collections::HashMap<(String, usize), std::collections::HashMap<u64, Option<String>>>> = Default::default();
    // plan scale (x, y) of a column family's own forms to its type: the structural section width
    // and depth (-1005502/-1005503) against the family type whose sizes match the forms, else a
    // single named size ('b', 'Diameter', 'Width') against a square/round form
    let column_scale = |sym: u64, own: &[[P3; 3]]| -> (f64, f64) {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for t in own {
            for q in t {
                for k in 0..2 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
        }
        let (ex, ey) = (hi[0] - lo[0], hi[1] - lo[1]);
        let Some((vs, pkey)) = sym_values.get(&sym) else { return (1.0, 1.0) };
        if !(ex > 1e-6 && ey > 1e-6) {
            return (1.0, 1.0);
        }
        let row = |set: &rvt_model::ValueSet, p: i64| set.rows.iter().find(|r| r.param == p).map(|r| r.num).filter(|v| *v > 1e-6);
        let close = |a: f64, b: f64| (a - b).abs() <= 0.05 * a.max(b);
        if let (Some(wt), Some(ht)) = (row(vs, -1005502), row(vs, -1005503)) {
            if let Some(key) = symbol_family.get(&sym).and_then(|f| family_chain.get(f)) {
                let mut cache = chain_sets.borrow_mut();
                let sets = cache.entry(key.clone()).or_insert_with(|| doc.value_sets_of(&key.0, key.1));
                for set in sets.iter() {
                    if let (Some(wf), Some(hf)) = (row(set, -1005502), row(set, -1005503)) {
                        if close(ex, wf) && close(ey, hf) {
                            return (wt / wf, ht / hf);
                        }
                        if close(ex, hf) && close(ey, wf) {
                            return (ht / hf, wt / wf);
                        }
                    }
                }
            }
        }
        if close(ex, ey) {
            let mut cache = chain_names.borrow_mut();
            let names = cache.entry(pkey.clone()).or_insert_with(|| doc.parameter_names_of(&pkey.0, pkey.1));
            let named = vs.rows.iter().find(|r| {
                r.num > 1e-6 && names.get(&(r.param as u64)).cloned().flatten().is_some_and(|n| matches!(n.as_str(), "b" | "Diameter" | "Width"))
            });
            if let Some(r) = named {
                let sc = r.num / ex.max(ey);
                if (0.05..=20.0).contains(&sc) {
                    return (sc, sc);
                }
            }
        }
        (1.0, 1.0)
    };
    // Step 2264: type sizes of any family. Width (−1001301 or 'Width'/'Length') along local x,
    // Depth (−1010003 or 'Depth') along y, Height (−1001300 or 'Height') along z: the symbol's own
    // value set against the family type whose sizes match the unscaled forms (snowdon casework
    // 906914: type 30" = 2.5 ft wide, the forms are the family's 36" = 3.0 ft). Scaled about the
    // family origin. Tried per instance before the other routes, and kept only when the scaled
    // shape lies in the stored box (±0.15 ft; rme face-based air terminals: no 'Height' along z).
    let global_names = doc.parameter_names_global();
    let typed: std::cell::RefCell<std::collections::HashMap<u64, [f64; 3]>> = Default::default();
    let current_cache: std::cell::RefCell<std::collections::HashMap<(String, usize), Option<rvt_model::ValueSet>>> = Default::default();
    let sizes = |set: &rvt_model::ValueSet, names: &std::collections::HashMap<u64, Option<String>>| -> [Option<f64>; 3] {
        let named = |p: i64, want: &[&str]| p > 0 && names.get(&(p as u64)).cloned().flatten().is_some_and(|n| want.contains(&n.as_str()));
        let pick = |bi: i64, want: &[&str]| {
            set.rows.iter().find(|r| r.param == bi).or_else(|| set.rows.iter().find(|r| named(r.param, want))).map(|r| r.num).filter(|v| *v > 1e-6)
        };
        [pick(-1001301, &["Width", "Length", "Door Width"]), pick(-1010003, &["Depth"]), pick(-1001300, &["Height", "Door Height"])]
    };
    let type_scale = |sym: u64, own: &[[P3; 3]]| -> Option<[f64; 3]> {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for t in own {
            for q in t {
                for k in 0..3 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
        }
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let (vs, _) = sym_values.get(&sym)?;
        let target = sizes(vs, &global_names);
        if target.iter().all(|v| v.is_none()) {
            return None;
        }
        let key = symbol_family.get(&sym).and_then(|f| family_chain.get(f))?;
        let fnames = {
            let mut cache = chain_names.borrow_mut();
            cache.entry(key.clone()).or_insert_with(|| doc.parameter_names_of(&key.0, key.1)).clone()
        };
        // the family type the forms are drawn at: the type whose known sizes all match the forms
        // (most sizes first); step 2266: else the family document's current values (elevator door:
        // 'Door Width' 3.0 + frame = 4.08 ft forms, matching no type)
        let matched = {
            let mut cache = chain_sets.borrow_mut();
            let sets = cache.entry(key.clone()).or_insert_with(|| doc.value_sets_of(&key.0, key.1));
            let close = |a: f64, b: f64| (a - b).abs() <= 0.05 * a.max(b);
            sets.iter()
                .map(|set| sizes(set, &fnames))
                .filter(|d| d.iter().any(|v| v.is_some()) && (0..3).all(|k| d[k].is_none_or(|v| close(v, ext[k]))))
                .max_by_key(|d| d.iter().filter(|v| v.is_some()).count())
        };
        let default = match matched {
            Some(d) => d,
            None => {
                let current = current_cache.borrow_mut().entry(key.clone()).or_insert_with(|| doc.family_current_values(&key.0, key.1)).clone();
                current.map(|c| sizes(&c, &fnames)).filter(|d| d.iter().any(|v| v.is_some()))?
            }
        };
        // additive: a size parameter need not span the whole form (elevator door: Door Width + frame)
        let sc: [f64; 3] = std::array::from_fn(|k| match (target[k], default[k]) {
            // a size counts for the axis only when it spans most of the form there, ≥ 70% (a shower's
            // pipe 'Length' 0.12 ft and door 1.875 of 2.83 ft are no overall size; elevator door 3.0 of 4.08)
            (Some(t), Some(d)) if ext[k] > 1e-6 && d >= 0.7 * ext[k] && t >= 0.7 * ext[k] => (ext[k] + t - d) / ext[k],
            _ => 1.0,
        });
        (sc.iter().any(|v| (v - 1.0).abs() > 0.01) && sc.iter().all(|v| (0.2..=5.0).contains(v))).then_some(sc)
    };
    let scale3 = |t: &[[P3; 3]], sc: [f64; 3]| -> Vec<[P3; 3]> { t.iter().map(|tr| tr.map(|q| [q[0] * sc[0], q[1] * sc[1], q[2] * sc[2]])).collect() };
    let scale_plan = |t: &[[P3; 3]], (sx, sy): (f64, f64)| -> Vec<[P3; 3]> {
        t.iter().map(|tr| tr.map(|q| [q[0] * sx, q[1] * sy, q[2]])).collect()
    };
    // step 2230: alternatives per symbol: (shape, local min, local max)
    let mut alt_cache: std::collections::HashMap<u64, std::rc::Rc<Vec<(Vec<[P3; 3]>, P3, P3)>>> = std::collections::HashMap::new();
    // Step 2219: door/window boxes include the swing; clip them to the host wall band instead.
    let wall_band: std::collections::HashMap<i64, (P3, P3, f64)> = walls_all
        .iter()
        .map(|w| (w.id as i64, (w.p0, w.p1, doc.type_thickness(w.wall_type).filter(|t| *t > 0.0).unwrap_or(0.5))))
        .collect();
    let host_of: std::collections::HashMap<u64, i64> = doc.hosted().into_iter().filter_map(|h| h.host.map(|x| (h.id, x))).collect();
    // Step 2257: a door or window family keeps its template's host wall (rme: 300 mm); its frame is
    // drawn against that wall's faces. Per family chain: the wall's normal (plan), centre offset and
    // width, when the family has exactly one host wall.
    let fam_host: std::collections::HashMap<(String, usize), (P3, f64, f64)> = doc
        .family_host_walls()
        .into_iter()
        .filter_map(|(k, v)| {
            let [(w, Some(t))] = v.as_slice() else { return None };
            let d = unit(sub([w.p1[0], w.p1[1], 0.0], [w.p0[0], w.p0[1], 0.0]))?;
            let n = [-d[1], d[0], 0.0];
            (*t > 1e-3 && w.curve.is_none()).then(|| (k, (n, dot([w.p0[0], w.p0[1], 0.0], n), *t)))
        })
        .collect();
    // RVT_MESH_DEBUG_INST=1: surface (m²) drawn per instance and batch, for comparison with Revit
    let dbg_inst = std::env::var("RVT_MESH_DEBUG_INST").is_ok();
    let mut prev_inst: Option<(u64, i32, Vec<(&'static str, usize)>)> = None;
    let report = |batches: &BTreeMap<&'static str, Batch>, prev: &(u64, i32, Vec<(&'static str, usize)>)| {
        for (name, n0) in &prev.2 {
            let Some(b) = batches.get(name) else { continue };
            if b.indices.len() > *n0 {
                let p = |i: u32| -> P3 {
                    let k = i as usize * 3;
                    [b.positions[k] as f64, b.positions[k + 1] as f64, b.positions[k + 2] as f64]
                };
                let a: f64 = b.indices[*n0..].chunks_exact(3).map(|t| norm(cross(sub(p(t[1]), p(t[0])), sub(p(t[2]), p(t[0])))) / 2.0).sum();
                eprintln!("inst {} {} {} {:.3}", prev.0, prev.1, name, a);
            }
        }
    };
    'inst: for i in insts.iter().filter(|i| i.scale.is_none() && i.origin.is_some()) {
        if dbg_inst {
            if let Some(pv) = prev_inst.take() {
                report(&batches, &pv);
            }
            prev_inst = Some((i.id, i.cat, batches.iter().map(|(k, b)| (*k, b.indices.len())).collect()));
        }
        let Some(e) = doc.element(i.id) else { continue };
        if e.view.is_some_and(|v| v != -1) {
            continue; // view-specific (annotation)
        }
        if i.symbol.and_then(|s| symbol_family.get(&s)).and_then(|f| family_chain.get(f)).is_some_and(|k| adaptive_chains.contains(k)) {
            // step 2282: an adaptive component: the placement points are in the instance record
            // (u32 count, then per point u32 tag + 3×3 + origin in the instance frame); Revit's rib
            // runs through them (snowdon strut 2220524: 4 points, GT vertices at points 1 and 4
            // exactly, its lower edge 'Z Point k' below point k, else 'Thickness'). A sheet under a
            // curve through the points; the frame without the flip of step 2257.
            let raw = i.rotation.map(|mut m| {
                if i.hand_flipped != i.facing_flipped {
                    for k in [1, 4, 7] {
                        m[k] = -m[k];
                    }
                }
                m
            });
            if let (Some(o), Some(m), Some(body)) = (i.origin, raw, doc.element_body(i.id)) {
                let pts: Vec<P3> = adaptive_points(&body).into_iter().map(|q| place(o, &m, q)).collect();
                if pts.len() >= 2 {
                    let named = |want: &str| -> Option<f64> {
                        (16..body.len().saturating_sub(8)).find_map(|q| {
                            let pid = i64_at_le(&body, q);
                            (pid > 0 && global_names.get(&(pid as u64)).cloned().flatten().is_some_and(|n| n == want))
                                .then(|| f64::from_le_bytes(body[q - 16..q - 8].try_into().unwrap()))
                                .filter(|v| v.is_finite() && v.abs() < 100.0)
                        })
                    };
                    let th = named("Thickness").filter(|v| *v > 1e-3).unwrap_or(0.125);
                    let drop: Vec<f64> = (1..=pts.len()).map(|k| named(&format!("Z Point {k}")).unwrap_or(-th)).collect();
                    let (name, color) = instance_batch(i.cat);
                    adaptive_rib(batch!(name, color), &pts, &drop);
                    n_family += 1;
                    continue;
                }
            }
        }
        // step 2285: a railing support (handrail bracket, cat −2000948) whose header flag after
        // `ff ff fc ff ff ff ff ff ff ff` is 0 is not in the {3D} view (snowdon arch 2025: 980/980
        // shown have 1, 1572 of 1628 not shown have 0; doors and fixtures shown have 0 too, so
        // only supports)
        if i.cat == -2000948 {
            if let Some(b) = doc.element_body(i.id) {
                let pat = [0xff, 0xff, 0xfc, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
                if let Some(k) = b[..b.len().min(300)].windows(pat.len()).position(|w| w == pat) {
                    if b.get(k + 10) == Some(&0) {
                        continue;
                    }
                }
            }
        }
        // step 2243: masses are hidden in views unless 'Show Mass' is on (rac_basic: in-place mass
        // 'water', 180 m2 not in Revit's view)
        if i.cat == -2003400 {
            continue;
        }
        // step 2240: columns have no stored box; build one from the family shape in plan and the
        // level block (§30.5) in z, so the fit below stretches the shape to the column height
        let col_bb = || -> Option<[f64; 6]> {
            let c = columns.get(&i.id)?;
            let (sym, o, m) = (i.symbol?, i.origin?, i.rotation?);
            let zb = doc.level_elevation(c.base_level)? + c.base_offset;
            let zt = doc.level_elevation(c.top_level)? + c.top_offset;
            let mut own = family_triangles(&column_solids_of(sym));
            for sw in sweeps_of(sym) {
                own.extend(sweep_triangles(sw));
            }
            own.extend(blrev_of(sym));
            let mut local = scale_plan(&own, column_scale(sym, &own));
            if let Some(key) = chain_of_symbol(sym) {
                local.extend(nested_tris(&key, 1, &chain_insts, &chain_of_symbol, &chain_forms, Some(sym)));
            }
            let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for t in &local {
                for q in t {
                    let w = place(o, &m, *q);
                    for k in 0..2 {
                        lo[k] = lo[k].min(w[k]);
                        hi[k] = hi[k].max(w[k]);
                    }
                }
            }
            (lo[0].is_finite() && zt > zb).then_some([lo[0], lo[1], zb, hi[0], hi[1], zt])
        };
        let Some(bb) = e.bbox.or_else(col_bb) else { continue };
        let ext = [bb[3] - bb[0], bb[4] - bb[1], bb[5] - bb[2]];
        if ext.iter().any(|v| !(v.is_finite() && *v >= 0.0 && *v < 1e4)) || ext.iter().all(|v| *v < 1e-6) {
            continue;
        }
        // Step 2235: RPC planting (no solids in its family) as Revit shows it: two crossed vertical
        // silhouettes (trunk and crown) at the placement, height = the stored box height, width =
        // the larger plan extent / √2 (Revit GT: 13.8/19.3/25 ft against 13.8/19/25.4).
        if i.cat == -2001360 {
            if let (Some(sym), Some(o), Some(m)) = (i.symbol, i.origin, i.rotation) {
                if solids_of(sym).is_empty() && sweeps_of(sym).is_empty() && blrev_of(sym).is_empty() {
                    let h = ext[2];
                    let half = ext[0].max(ext[1]) / std::f64::consts::SQRT_2 / 2.0;
                    // outline (u across, v up), crown as an octagon-like shape on a trunk
                    let outline: [(f64, f64); 12] = [
                        (-0.05, 0.0), (0.05, 0.0), (0.05, 0.28), (0.55, 0.36), (0.95, 0.55), (0.85, 0.80),
                        (0.45, 1.0), (-0.45, 1.0), (-0.85, 0.80), (-0.95, 0.55), (-0.55, 0.36), (-0.05, 0.28),
                    ];
                    let b = batch!("Planting", [0.35, 0.55, 0.30]);
                    let z0 = o[2].max(bb[2]) - o[2];
                    for axis in [[1.0, 0.0], [0.0, 1.0]] {
                        let pt = |u: f64, v: f64| place(o, &m, [axis[0] * u * half, axis[1] * u * half, z0 + v * h]);
                        let pts: Vec<P3> = outline.iter().map(|&(u, v)| pt(u, v)).collect();
                        // fan from the trunk top centre, both faces
                        let c = pt(0.0, 0.6);
                        for k in 0..pts.len() {
                            let (p, q) = (pts[k], pts[(k + 1) % pts.len()]);
                            b.tri(c, p, q);
                            b.tri(c, q, p);
                        }
                    }
                    n_family += 1;
                    continue;
                }
            }
        }
        // Step 2244: RPC entourage (people, cars; no solids) as Revit shows it: crossed vertical
        // silhouettes for people; a long one is a car, drawn as a low-poly body and cabin (Revit shows
        // a car mesh). Local half sizes from the stored box through the rotation.
        if i.cat == -2001370 {
            if let (Some(sym), Some(o), Some(m)) = (i.symbol, i.origin, i.rotation) {
                if solids_of(sym).is_empty() && sweeps_of(sym).is_empty() && blrev_of(sym).is_empty() {
                    let (c, s) = (m[0].abs(), m[3].abs());
                    let det = c * c - s * s;
                    let (hx, hy) = if det.abs() > 0.2 {
                        (((c * ext[0] - s * ext[1]) / det / 2.0).abs(), ((c * ext[1] - s * ext[0]) / det / 2.0).abs())
                    } else {
                        let h = ext[0].max(ext[1]) / (c + s).max(1.0) / 2.0;
                        (h, h)
                    };
                    let car = hx.max(hy) > 1.8 * hx.min(hy);
                    const PERSON: [(f64, f64); 12] = [
                        (-0.25, 0.0), (0.25, 0.0), (0.3, 0.5), (0.45, 0.55), (0.4, 0.8), (0.15, 0.85),
                        (0.15, 1.0), (-0.15, 1.0), (-0.15, 0.85), (-0.4, 0.8), (-0.45, 0.55), (-0.3, 0.5),
                    ];
                    let h = ext[2];
                    let z0 = bb[2] - o[2];
                    let b = batch!("Entourage", [0.42, 0.45, 0.55]);
                    if car {
                        // a low-poly car: body over the full footprint, a narrower cabin on top
                        let lb = |x0: f64, x1: f64, y0: f64, y1: f64, v0: f64, v1: f64| {
                            let c4 = |v: f64| [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|q| place(o, &m, [q[0], q[1], z0 + v * h]));
                            (c4(v0), c4(v1))
                        };
                        let (lx, ly) = (hx, hy);
                        let (bot, top) = lb(-lx, lx, -ly, ly, 0.15, 0.5);
                        b.hexa(bot, top);
                        let (cx0, cx1, cy0, cy1) = if hx >= hy { (-0.45 * lx, 0.3 * lx, -0.85 * ly, 0.85 * ly) } else { (-0.85 * lx, 0.85 * lx, -0.45 * ly, 0.3 * ly) };
                        let (bot, top) = lb(cx0, cx1, cy0, cy1, 0.5, 0.9);
                        b.hexa(bot, top);
                        n_family += 1;
                        continue;
                    }
                    for (axis, half) in [([1.0, 0.0], hx), ([0.0, 1.0], hy)] {
                        let outline: &[(f64, f64)] = &PERSON;
                        let pt = |u: f64, v: f64| place(o, &m, [axis[0] * u * half, axis[1] * u * half, z0 + v * h]);
                        let pts: Vec<P3> = outline.iter().map(|&(u, v)| pt(u, v)).collect();
                        let cen = pt(0.0, 0.5);
                        for k in 0..pts.len() {
                            let (p, q) = (pts[k], pts[(k + 1) % pts.len()]);
                            b.tri(cen, p, q);
                            b.tri(cen, q, p);
                        }
                    }
                    n_family += 1;
                    continue;
                }
            }
        }
        // A stored box that is large in all three directions (> 3 m) covers more than the family
        // itself (site, massing or nested content); drawing it hides the building.
        // step 2256: structural foundations (pile caps with piles) may be large in every direction
        // step 2278: planting and entourage may be large too (snowdon trees 15 x 16 x 18 ft)
        // step 2283: nor a family holding adaptive content (snowdon canopy 2220523, box 7.4 × 5.0 ×
        // 6.6 m: its box spans the shared struts; its own forms are placed unfitted further on)
        let adaptive_host = i.symbol.and_then(|s| symbol_family.get(&s)).and_then(|f| family_chain.get(f)).is_some_and(|k| adaptive_chains.contains(k));
        if ext.iter().all(|v| *v * FT > 3.0) && !matches!(i.cat, -2001300 | -2001360 | -2001370) && !adaptive_host {
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
                    let sol = if matches!(i.cat, -2000100 | -2001330) { column_solids_of(sym) } else { solids_of(sym) };
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
                    // step 2253: a column's own forms take its type size in plan
                    let (ext_only, no_sweeps, all) = if matches!(i.cat, -2000100 | -2001330) {
                        let sc = column_scale(sym, &all);
                        (scale_plan(&ext_only, sc), scale_plan(&no_sweeps, sc), scale_plan(&all, sc))
                    } else {
                        // step 2264: the type's own sizes, tried per instance below
                        if let Some(sc) = type_scale(sym, &all) {
                            typed.borrow_mut().insert(sym, sc);
                        }
                        (ext_only, no_sweeps, all)
                    };
                    // step 2246: nested families; candidates with them first (a shared nested component
                    // is a separate instance: linked instances skip these, see `linked`)
                    let nested = chain_of_symbol(sym).map(|key| nested_tris(&key, 1, &chain_insts, &chain_of_symbol, &chain_forms, Some(sym))).unwrap_or_default();
                    let with = |x: &Vec<[P3; 3]>| -> Vec<[P3; 3]> { x.iter().chain(nested.iter()).copied().collect() };
                    let withs = if nested.is_empty() { vec![] } else { vec![with(&all), with(&no_sweeps), with(&ext_only)] };
                    // nested candidates first; `plain_count` = how many come first (dropped for
                    // instances linked to their shared components)
                    let mut c: Vec<Vec<[P3; 3]>> = Vec::new();
                    for x in withs {
                        if !x.is_empty() && !c.iter().any(|y| y.len() == x.len()) {
                            c.push(x);
                        }
                    }
                    plain_count.insert(sym, c.len());
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
            // step 2246: shared nested components are separate instances: no nested shapes then
            let skip = if linked.contains(&i.id) { plain_count.get(&sym).copied().unwrap_or(0) } else { 0 };
            let mut candidates: Vec<&Vec<[P3; 3]>> = best_alt.into_iter().chain(candidates.iter().skip(skip)).collect();

            // step 2246: the first candidate that covers the stored box (±0.15 ft) goes first
            if let Some(k) = candidates.iter().position(|loc| {
                let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for t in loc.iter() {
                    for q in t {
                        let w = place(o, &m, *q);
                        for j in 0..3 {
                            lo[j] = lo[j].min(w[j]);
                            hi[j] = hi[j].max(w[j]);
                        }
                    }
                }
                !loc.is_empty() && (0..3).all(|j| (lo[j] - bb[j]).abs() < 0.15 && (hi[j] - bb[j + 3]).abs() < 0.15)
            }) {
                let c = candidates.remove(k);
                candidates.insert(0, c);
            }
            // step 2257: an opening's depth follows its host wall: the family wall's faces move onto
            // the project wall's faces, what lies beyond a face (casing, handles) moves with it, and
            // what lies between scales with the thickness
            let depth_map: Option<(P3, [f64; 4])> = (matches!(i.cat, -2000023 | -2000014))
                .then(|| {
                    let (fnrm, fc, ft) = *symbol_family.get(&sym).and_then(|f| family_chain.get(f)).and_then(|k| fam_host.get(k))?;
                    let &(wp0, wp1, wt) = host_of.get(&i.id).and_then(|h| wall_band.get(h))?;
                    let wd = unit(sub([wp1[0], wp1[1], 0.0], [wp0[0], wp0[1], 0.0]))?;
                    let wn = [-wd[1], wd[0], 0.0];
                    let dir = sub(place(o, &m, fnrm), o);
                    if dot(dir, wn).abs() < 0.99 {
                        return None;
                    }
                    // the project wall's centre line in the family's depth coordinate
                    let wc = dot(sub([wp0[0], wp0[1], 0.0], [o[0], o[1], 0.0]), dir);
                    Some((fnrm, [fc - ft / 2.0, fc + ft / 2.0, wc - wt / 2.0, wc + wt / 2.0]))
                })
                .flatten();
            let remapped: Vec<Vec<[P3; 3]>> = match depth_map {
                Some((nrm, [f0, f1, w0, w1])) => {
                    // step 2267: a door in a wall thicker than the family's (snowdon: 1.05 against 0.67
                    // ft) keeps its depth, its face on the +normal side on the wall's (Revit's depth:
                    // 111 of 127 doors within 0.25 ft, before 49); windows still follow both faces
                    let thicker = w1 - w0 > f1 - f0 && i.cat == -2000023;
                    let mv = |q: P3| -> P3 {
                        let sd = dot(q, nrm);
                        let s2 = if thicker {
                            sd + w1 - f1
                        } else if sd < f0 {
                            sd + w0 - f0
                        } else if sd > f1 {
                            sd + w1 - f1
                        } else {
                            w0 + (sd - f0) * (w1 - w0) / (f1 - f0)
                        };
                        add(q, mul(nrm, s2 - sd))
                    };
                    candidates.iter().map(|c| c.iter().map(|t| t.map(mv)).collect()).collect()
                }
                None => Vec::new(),
            };
            let candidates: Vec<&Vec<[P3; 3]>> = if depth_map.is_some() { remapped.iter().collect() } else { candidates };
            if !candidates.is_empty() {
                for local in candidates.iter() {
                if std::env::var("RVT_MESH_DEBUG").ok().and_then(|v| v.parse::<u64>().ok()) == Some(i.id) {
                    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                    for t in local.iter() {
                        for q in t {
                            let w = place(o, &m, *q);
                            for j in 0..3 {
                                lo[j] = lo[j].min(w[j]);
                                hi[j] = hi[j].max(w[j]);
                            }
                        }
                    }
                    eprintln!("debug {}: {} local triangles, bbox {:.2?}, placed {:.2?} {:.2?}", i.id, local.len(), bb, lo, hi);
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
                // step 2264: the shape at its type size, when that lies in the stored box and the unscaled
                // shape does not already cover it (rme panelboards: 'Height' is not the form height)
                if std::env::var("RVT_MESH_DEBUG").ok().and_then(|v| v.parse::<u64>().ok()) == Some(i.id) {
                    eprintln!("debug {}: type scale {:?}, covers {covers}", i.id, typed.borrow().get(&sym));
                }
                if let Some(sc) = typed.borrow().get(&sym).copied().filter(|_| !covers) {
                    let ws: Vec<[P3; 3]> = scale3(local, sc).iter().map(|t| [place(o, &m, t[0]), place(o, &m, t[1]), place(o, &m, t[2])]).collect();
                    let (sl, sh) = bounds(&ws);
                    if !ws.is_empty() && (0..3).all(|k| sl[k] >= bb[k] - 0.15 && sh[k] <= bb[k + 3] + 0.15) {
                        let b = batch!(name, color);
                        let rev = det3(&m) < 0.0;
                        for t in ws {
                            if rev {
                                b.tri(t[2], t[1], t[0]);
                            } else {
                                b.tri(t[0], t[1], t[2]);
                            }
                        }
                        n_family += 1;
                        continue 'inst;
                    }
                }
                // Step 2217: family geometry is parametric (beam length, type sizes, height
                // parameters). When the rotation is axis-aligned, fit the family shape per local axis
                // to the element's stored box instead of falling back to the box.
                let axis_aligned = m.iter().all(|v| v.abs() < 1e-6 || (v.abs() - 1.0).abs() < 1e-6);
                // step 2279: a local axis whose world image is a world axis (rst braces rolled 45°
                // about their length: the length is along x, the section is turned); framing is
                // fitted along such axes, the others keep the family size
                let aligned: [bool; 3] = std::array::from_fn(|k| [m[k], m[3 + k], m[6 + k]].iter().filter(|v| (v.abs() - 1.0).abs() < 1e-6).count() == 1);
                // (only when the length axis, local x, is one of them: else the brace rule below)
                let partly = !axis_aligned && i.cat == -2001320 && aligned[0];
                let fitted: Option<Vec<[P3; 3]>> = (!covers && (axis_aligned || partly) && !world.is_empty() && !linked_hosts.contains(&i.id))
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
                        // step 2236: doors and windows: an axis whose box includes the swing (factor
                        // > 2, a casement window 4.9x deep) keeps the family size; the other axes
                        // follow the type size, up to 4x (rac_basic: 1.0 x 1.0 m family, 1.5 x 2.7 m
                        // windows)
                        let opening = matches!(i.cat, -2000023 | -2000014);
                        let factor = |k: usize| (ehi[k] - elo[k]) / (shi[k] - slo[k]);
                        // step 2250: for every category, an axis whose stored box is far larger than
                        // the shape (rme troffers: box 2 ft high, Revit's shape 0.49 ft) keeps the
                        // family size; doors and windows as before
                        let keep: [bool; 3] = std::array::from_fn(|k| {
                            // step 2255: structural framing stretches freely (its length follows the
                            // beam, its profile the stored box)
                            (partly && !aligned[k] && shi[k] - slo[k] > 1e-6)
                                || shi[k] - slo[k] > 1e-6
                                && i.cat != -2001320
                                && if opening { factor(k) > 2.0 && (k != 2 || factor(k) > 4.0) } else { factor(k) > 2.0 && !(matches!(i.cat, -2000100 | -2001330) && k == 2) }
                        });
                        let plausible = (0..3).all(|k| {
                            let se = shi[k] - slo[k];
                            // step 2240: a column's length is fully parametric (vertical axis free)
                            let column = (matches!(i.cat, -2000100 | -2001330) && k == 2) || i.cat == -2001320;
                            se < 1e-6 || keep[k] || (factor(k) >= 0.05 && (column || factor(k) <= if opening { 4.0 } else { 2.0 }))
                        });
                        if !plausible || keep.iter().all(|x| *x) {
                            return None;
                        }
                        let map = |p: P3| -> P3 {
                            let mut r = [0.0; 3];
                            for k in 0..3 {
                                let se = shi[k] - slo[k];
                                r[k] = if keep[k] {
                                    p[k]
                                } else if se > 1e-6 {
                                    elo[k] + (p[k] - slo[k]) * (ehi[k] - elo[k]) / se
                                } else {
                                    (elo[k] + ehi[k]) / 2.0
                                };
                            }
                            place(o, &m, r)
                        };
                        Some(local.iter().map(|t| [map(t[0]), map(t[1]), map(t[2])]).collect())
                    })
                    .flatten()
                    // step 2280: a brace whose length axis (local x) runs diagonally (rst braces in a
                    // vertical plane, box 0.44 × 9.82 × 10.09 ft): its length is the box extent over the
                    // direction component, e_k / |d_k|, the smallest over the significant components;
                    // stretched along local x about the middle of the family's extent (a placed
                    // brace's origin is its midpoint: rst 1114997 origin = centre of Revit's box)
                    .or_else(|| {
                        if covers || i.cat != -2001320 || axis_aligned || aligned[0] {
                            return None;
                        }
                        let d = [m[0], m[3], m[6]];
                        let ext = [bb[3] - bb[0], bb[4] - bb[1], bb[5] - bb[2]];
                        let (mut llo, mut lhi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                        for q in local.iter().flatten() {
                            for k in 0..3 {
                                llo[k] = llo[k].min(q[k]);
                                lhi[k] = lhi[k].max(q[k]);
                            }
                        }
                        // the section (local y, z) adds |M[k][j]|·size_j to the box along world axis k
                        let sec = |k: usize| (1..3).map(|j| m[3 * k + j].abs() * (lhi[j] - llo[j])).sum::<f64>();
                        let len = (0..3).filter(|k| d[*k].abs() > 0.3).map(|k| (ext[k] - sec(k)).max(0.0) / d[k].abs()).fold(f64::INFINITY, f64::min);
                        let (sl, sh) = (llo[0], lhi[0]);
                        let se = sh - sl;
                        (len.is_finite() && se > 1e-3 && (0.1..=20.0).contains(&(len / se))).then(|| {
                            let mid = (sl + sh) / 2.0;
                            local.iter().map(|t| t.map(|q| place(o, &m, [mid + (q[0] - mid) * len / se, q[1], q[2]]))).collect()
                        })
                    })
                    // otherwise the unscaled family shape when it lies inside the stored box
                    .or_else(|| {
                        // step 2245: 0.5 ft margin (rac_basic parking items on the sloped terrain: the
                        // unscaled shape matches Revit in plan but stands 0.42 ft above the stored box)
                        let inside = (0..3).all(|k| lo[k] >= bb[k] - 0.5 && hi[k] <= bb[k + 3] + 0.5);
                        // doors and windows: the stored box includes the swing, so it is no fallback
                        let opening = matches!(i.cat, -2000023 | -2000014);
                        (!covers && (inside || opening) && !world.is_empty()).then(|| world.clone())
                    })
                    // step 2258: turned about the vertical only (rac_basic solar panels at 37°): the
                    // box's height is exact, its plan is the turned shape's; when the plan fits, the
                    // height follows the box (a tilt parameter: Revit 1.27 ft, the family 1.82 ft)
                    .or_else(|| {
                        let upright = (m[8] - 1.0).abs() < 1e-6;
                        let plan = (0..2).all(|k| lo[k] >= bb[k] - 0.5 && hi[k] <= bb[k + 3] + 0.5);
                        let (hz, bz) = (hi[2] - lo[2], bb[5] - bb[2]);
                        (!covers && upright && plan && hz > 1e-3 && (0.5..=2.0).contains(&(bz / hz)))
                            .then(|| world.iter().map(|t| t.map(|q| [q[0], q[1], bb[2] + (q[2] - lo[2]) * bz / hz])).collect())
                    });
                if covers || fitted.is_some() {
                    let b = batch!(name, color);
                    let rev = det3(&m) < 0.0;
                    for t in fitted.unwrap_or(world) {
                        if rev {
                            b.tri(t[2], t[1], t[0]);
                        } else {
                            b.tri(t[0], t[1], t[2]);
                        }
                    }
                    n_family += 1;
                    continue 'inst;
                }
                }
            }
        }
        // step 2281: an adaptive component (its family has adaptive points, cat −2000710) without
        // readable forms: its stored box also holds the reference points and says neither the
        // direction nor the size of the form (snowdon canopy struts: box 16.5 × 9.1 × 13.2 ft,
        // Revit's rod 8.8 × 2.9 × 7 ft, 0.7 m²); no box is less wrong than a slab.
        // Step 2283: a family that holds adaptive content beside readable forms (snowdon canopy
        // 2220523: nested columns, plus rings and shared struts we cannot fit) misses the box fit
        // because its box also spans that content; its forms are placed as they are, unfitted.
        if i.symbol.and_then(|s| symbol_family.get(&s)).and_then(|f| family_chain.get(f)).is_some_and(|k| adaptive_chains.contains(k)) {
            if let (Some(c), Some(o), Some(m)) = (i.symbol.and_then(|s| shape_cache.get(&s)).and_then(|c| c.first().cloned()), i.origin, i.rotation) {
                let rev = det3(&m) < 0.0;
                let b = batch!(name, color);
                for t in &c {
                    let (p, q) = if rev { (t[2], t[0]) } else { (t[0], t[2]) };
                    b.tri(place(o, &m, p), place(o, &m, t[1]), place(o, &m, q));
                }
                n_family += 1;
            }
            continue;
        }
        n_box += 1;
        *box_by_cat.entry(i.cat).or_default() += 1;
        if dbg_inst {
            eprintln!("asbox {} {} {:.3}", i.id, i.cat, i.rotation.map(|m| m[8]).unwrap_or(1.0));
        }
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
        // step 2261: a turned instance: the oriented box behind its stored box
        if let Some((bot, top)) = i.rotation.and_then(|m| oriented_box(&bb, &m)) {
            batch!(name, color).hexa(bot, top);
            continue;
        }
        batch!(name, color).aabb(bb);
    }

    // Step 2273: a member of a multistory stair serves every identical storey above it up to the next
    // member (snowdon: 621690 18.8 -> 32.2 ft, the next member at 45.5; Revit draws it twice, its
    // stored box spans 18.8..45.8). When the stored box spans n ≥ 2 times the stair's height, its
    // first storey (stairs, landings, railings in its plan box) is repeated n − 1 times.
    for st in stairs.iter() {
        let (Some(zb), Some(h)) = (st.base_z, Some(st.height).filter(|h| *h > 1.0)) else { continue };
        let Some(bb) = doc.element(st.id).and_then(|e| e.bbox) else { continue };
        let n = ((bb[5] - zb) / h).round();
        if !(2.0..=20.0).contains(&n) {
            continue;
        }
        let (lo, hi) = ([bb[0] - 0.1, bb[1] - 0.1, zb - 0.1], [bb[3] + 0.1, bb[4] + 0.1, zb + h + 3.6]);
        for name in ["Stairs", "Railings"] {
            let Some(b) = batches.get_mut(name) else { continue };
            let p = |b: &Batch, i: u32| -> P3 {
                let k = i as usize * 3;
                // view (x, z, −y) in metres -> model feet
                [b.positions[k] as f64 / FT, -(b.positions[k + 2] as f64) / FT, b.positions[k + 1] as f64 / FT]
            };
            let picked: Vec<[u32; 3]> = b
                .indices
                .chunks_exact(3)
                .map(|t| [t[0], t[1], t[2]])
                .filter(|t| {
                    let c = mul(add(add(p(b, t[0]), p(b, t[1])), p(b, t[2])), 1.0 / 3.0);
                    (0..3).all(|k| c[k] >= lo[k] && c[k] <= hi[k])
                })
                .collect();
            for j in 1..n as usize {
                let dz = (h * j as f64 * FT) as f32;
                for t in &picked {
                    let base = (b.positions.len() / 3) as u32;
                    for &i in t {
                        let k = i as usize * 3;
                        let (x, y, z) = (b.positions[k], b.positions[k + 1] + dz, b.positions[k + 2]);
                        let (nx, ny, nz) = (b.normals[k], b.normals[k + 1], b.normals[k + 2]);
                        b.positions.extend_from_slice(&[x, y, z]);
                        b.normals.extend_from_slice(&[nx, ny, nz]);
                    }
                    b.indices.extend_from_slice(&[base, base + 1, base + 2]);
                }
            }
        }
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
