//! Triangle meshes of a document for viewers: one batch per category, in metres, Y up
//! (three.js convention: view = (x, z, −y) × 0.3048).
//!
//! Walls, floors, ceilings, roofs and MEP segments are built from their reader data. Family
//! instances are shown as their stored bounding box until family geometry is ported.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use rvt_model::{Curve, Instance, Segment};
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
        let n = (((t1 - t0).abs() / (PI / 24.0)).ceil() as usize).clamp(2, 96);
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
    let mut pieces: Vec<Vec<P3>> = curves.iter().map(sample).filter(|p| p.len() >= 2).collect();
    let z = pieces.first().map(|p| p[0][2]).unwrap_or(0.0);
    let close = |a: P3, b: P3| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
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
    (groups, z)
}

fn instance_batch(cat: i32) -> (&'static str, [f32; 3]) {
    match cat {
        -2000023 => ("Doors", [0.71, 0.53, 0.35]),
        -2000014 => ("Windows", [0.45, 0.70, 0.92]),
        -2000080 | -2001100 => ("Furniture and casework", [0.80, 0.62, 0.40]),
        -2000100 | -2001330 => ("Columns", [0.62, 0.62, 0.66]),
        -2001320 => ("Structural framing", [0.55, 0.55, 0.60]),
        -2001160 | -2001040 => ("Plumbing fixtures", [0.85, 0.88, 0.90]),
        -2001140 | -2008013 | -2008010 | -2008016 | -2008055 => ("MEP equipment and terminals", [0.40, 0.65, 0.55]),
        -2001120 => ("Lighting fixtures", [0.96, 0.62, 0.04]),
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
    let mut batches: BTreeMap<&'static str, Batch> = BTreeMap::new();
    fn get<'a>(m: &'a mut BTreeMap<&'static str, Batch>, name: &'static str, color: [f32; 3]) -> &'a mut Batch {
        m.entry(name).or_insert_with(|| Batch::new(name, color))
    }
    macro_rules! batch {
        ($name:expr, $color:expr) => {
            get(&mut batches, $name, $color)
        };
    }

    for w in doc.walls() {
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
    for r in doc.roofs().into_iter().filter(|r| r.kind == "footprint") {
        let t = r.thickness.filter(|t| *t > 0.0).unwrap_or(0.75);
        let curves: Vec<Curve> = r.edges.iter().map(|e| e.curve.clone()).collect();
        slab(batch!("Roofs", [0.60, 0.30, 0.18]), &curves, r.base_z, t, false);
    }

    let segs: Vec<Segment> = doc.segments();
    for s in &segs {
        let cat = doc.element(s.id).map(|e| e.category);
        let (name, color) = segment_batch(cat);
        batch!(name, color).member(s.p0, s.p1, s.width, s.height, s.round);
    }

    let insts: Vec<Instance> = doc.instances();
    // Curtain panels and mullions are skipped: their stored boxes span the whole curtain wall, which
    // is already drawn as a wall.
    for i in insts.iter().filter(|i| i.scale.is_none() && i.origin.is_some() && !matches!(i.cat, -2000170 | -2000171)) {
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
        let (name, color) = instance_batch(i.cat);
        batch!(name, color).aabb(bb);
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
