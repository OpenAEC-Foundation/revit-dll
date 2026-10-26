//! Rebar as tubes along its bar centerlines (branch open-research, step 2305).
//!
//! Rebar has no saved graphics (channel 103); its channel-102 object graph carries the bars.
//! Byte observation on rstbasicsampleproject_2026 (decoded with the file's schema):
//! - `Rebar` → `m_oInstance` (`RebarInstance`): `m_barPositionsRelativeToFirstBarArr` (one `Trf`
//!   per bar), `m_includedBars`, `m_oCacheData` (`RebarInstCache`) →
//!   `m_noSelfIntersectingCenterlineCurves[0]` or else `m_selfIntersectingCenterlineCurves[0]`
//!   (`OneBarCenterlineCurves`) → `m_centerlineCurves`:
//!   the first bar's `GLine`/`GArc` in model coordinates; `m_barTypeId` (on a sub-object) names the
//!   `RebarBarType`, whose `m_barModelDiameter` is the bar diameter (feet);
//! - `RebarInSystem` (bars of an area/path reinforcement): `m_systemId` and `m_key` (`m_layer`,
//!   `m_index`) → the system's `m_layers[m_layer]` (`RebarSystemLayer`): `m_uncutCurves` (one line
//!   per bar), `m_uncutRebarInstances` (each a `RebarInstance` covering the next run of lines),
//!   `m_barType`.
//! rst 2026 bar set 1381437: 17 bars × 58.89 ft × π × 0.0262 ft = 82.4 ft², Revit 82.3 ft².

use rvt_core::graph::{Graph, Object, Value};
use rvt_read::SavedGraphics;

type P3 = [f64; 3];

fn id_of(v: &Value) -> Option<i64> {
    v.as_obj()?.get("m_id")?.as_obj()?.get("m_id64")?.as_i64()
}

fn graph(sg: &SavedGraphics, id: u64) -> Option<Graph> {
    Graph::decode(&sg.schema, sg.objects.get(&id)?).ok()
}

/// Bar diameter of a `RebarBarType` (feet).
fn diameter(sg: &SavedGraphics, bar_type: i64) -> Option<f64> {
    graph(sg, bar_type as u64)?.objects.first()?.get("m_barModelDiameter")?.as_f64().filter(|d| *d > 0.0)
}

/// Points along a `GLine` or `GArc` (`m_endParams`).
fn curve_points(o: &Object) -> Option<Vec<P3>> {
    let e = o.get("m_endParams")?.floats()?;
    let (t0, t1) = (*e.first()?, *e.get(1)?);
    if let Some(d) = o.get("m_dirVec").and_then(|v| v.floats()) {
        let p = o.get("m_origin")?.floats()?;
        return Some([t0, t1].iter().map(|t| [p[0] + t * d[0], p[1] + t * d[1], p[2] + t * d[2]]).collect());
    }
    let c = o.get("m_center")?.floats()?;
    let r = o.get("m_radius")?.as_f64()?;
    let (x, y) = (o.get("m_xVec")?.floats()?, o.get("m_yVec")?.floats()?);
    let n = ((t1 - t0).abs() / 15f64.to_radians()).ceil().max(1.0) as usize;
    Some((0..=n).map(|k| t0 + (t1 - t0) * k as f64 / n as f64).map(|t| std::array::from_fn(|i| c[i] + r * (t.cos() * x[i] + t.sin() * y[i]))).collect())
}

/// Row-major 3×3 and `m_or` of a `Trf`.
fn trf(v: &Value) -> Option<([[f64; 3]; 3], P3)> {
    let t = v.as_obj()?;
    let rows = t.get("m_3x3")?.as_list()?;
    let mut m = [[0.0; 3]; 3];
    for (i, r) in rows.iter().take(3).enumerate() {
        let f = r.floats()?;
        m[i] = [*f.first()?, *f.get(1)?, *f.get(2)?];
    }
    let o = t.get("m_or")?.floats()?;
    Some((m, [o[0], o[1], o[2]]))
}

fn apply(t: &([[f64; 3]; 3], P3), p: P3) -> P3 {
    std::array::from_fn(|i| t.0[i][0] * p[0] + t.0[i][1] * p[1] + t.0[i][2] * p[2] + t.1[i])
}

/// An 8-sided tube of diameter `d` along each segment of a polyline.
fn tube(path: &[P3], d: f64, out: &mut Vec<[P3; 3]>) {
    // radius scaled so the octagon's perimeter equals the bar's circumference (the surface
    // matches Revit's: 8 sides at r alone gave 0.97)
    let r = d / 2.0 * (std::f64::consts::PI / 8.0) / (std::f64::consts::PI / 8.0).sin();
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        let t = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let l = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
        if l < 1e-9 {
            continue;
        }
        let t = [t[0] / l, t[1] / l, t[2] / l];
        let h = if t[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        let mut n = [t[1] * h[2] - t[2] * h[1], t[2] * h[0] - t[0] * h[2], t[0] * h[1] - t[1] * h[0]];
        let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        n = [n[0] / nl, n[1] / nl, n[2] / nl];
        let bv = [t[1] * n[2] - t[2] * n[1], t[2] * n[0] - t[0] * n[2], t[0] * n[1] - t[1] * n[0]];
        let ring = |c: P3| -> Vec<P3> {
            (0..8).map(|k| {
                let a = k as f64 * std::f64::consts::FRAC_PI_4;
                let (s, co) = a.sin_cos();
                std::array::from_fn(|i| c[i] + r * (co * n[i] + s * bv[i]))
            }).collect()
        };
        let (ra, rb) = (ring(a), ring(b));
        for k in 0..8 {
            let j = (k + 1) % 8;
            out.push([ra[k], ra[j], rb[j]]);
            out.push([ra[k], rb[j], rb[k]]);
        }
    }
}

/// Triangles of a `Rebar` or `RebarInSystem` element (feet); none for other elements.
pub fn element(sg: &SavedGraphics, id: u64) -> Option<Vec<[P3; 3]>> {
    let g = graph(sg, id)?;
    let root = g.objects.first()?;
    let name = sg.schema.class(root.class)?.name.as_str();
    let mut out = Vec::new();
    match name {
        "Rebar" => {
            let inst = g.objects.get(root.get("m_oInstance")?.as_ptr()?)?;
            let cache = g.objects.get(inst.get("m_oCacheData")?.as_ptr()?)?;
            // either list may be empty (rst 2026 bar 1372483: only the self-intersecting one)
            let first = |f: &str| cache.get(f).and_then(|v| v.as_list()).and_then(|l| l.first()).and_then(|v| v.as_ptr());
            let one = g.objects.get(first("m_noSelfIntersectingCenterlineCurves").or_else(|| first("m_selfIntersectingCenterlineCurves"))?)?;
            let mut path: Vec<P3> = Vec::new();
            for c in one.get("m_centerlineCurves")?.as_list()?.iter().filter_map(|v| v.as_ptr()).filter_map(|k| g.objects.get(k)) {
                let mut pts = curve_points(c)?;
                // the curves run either way; continue from the previous end
                if let (Some(&p), Some(&f), Some(&l)) = (path.last(), pts.first(), pts.last()) {
                    let d = |a: P3, b: P3| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>();
                    if d(p, l) < d(p, f) {
                        pts.reverse();
                    }
                }
                path.extend(pts);
            }
            let bar_type = g.objects.iter().find_map(|o| o.get("m_barTypeId").and_then(id_of))?;
            let d = diameter(sg, bar_type)?;
            let positions = inst.get("m_barPositionsRelativeToFirstBarArr")?.as_list()?;
            let included = inst.get("m_includedBars").and_then(|v| v.as_list()).unwrap_or(&[]);
            for (k, p) in positions.iter().enumerate() {
                if included.get(k).and_then(|v| v.as_i64()) == Some(0) {
                    continue;
                }
                let Some(t) = trf(p) else { continue };
                let moved: Vec<P3> = path.iter().map(|q| apply(&t, *q)).collect();
                tube(&moved, d, &mut out);
            }
        }
        "RebarInSystem" => {
            let sys_id = root.get("m_systemId").and_then(id_of)?;
            let key = root.get("m_key")?.as_obj()?;
            let (layer, index) = (key.get("m_layer")?.as_i64()? as usize, key.get("m_index")?.as_i64()? as usize);
            let sg2 = graph(sg, sys_id as u64)?;
            let layers = sg2.objects.first()?.get("m_layers")?.as_list()?;
            let l = layers.get(layer)?.as_obj()?;
            let d = diameter(sg, l.get("m_barType").and_then(id_of)?)?;
            let curves: Vec<usize> = l.get("m_uncutCurves")?.as_list()?.iter().filter_map(|v| v.as_ptr()).collect();
            let insts: Vec<&Object> = l.get("m_uncutRebarInstances")?.as_list()?.iter().filter_map(|v| v.as_ptr()).filter_map(|k| sg2.objects.get(k)).collect();
            let count = |o: &Object| o.get("m_barPositionsRelativeToFirstBarArr").and_then(|v| v.as_list()).map_or(0, |l| l.len());
            let first: usize = insts.iter().take(index).map(|o| count(o)).sum();
            let inst = insts.get(index)?;
            let included = inst.get("m_includedBars").and_then(|v| v.as_list()).unwrap_or(&[]);
            for k in 0..count(inst) {
                if included.get(k).and_then(|v| v.as_i64()) == Some(0) {
                    continue;
                }
                let Some(c) = curves.get(first + k).and_then(|&i| sg2.objects.get(i)) else { continue };
                if let Some(pts) = curve_points(c) {
                    tube(&pts, d, &mut out);
                }
            }
        }
        _ => return None,
    }
    (!out.is_empty()).then_some(out)
}
