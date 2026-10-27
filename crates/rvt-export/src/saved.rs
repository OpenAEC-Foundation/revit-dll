//! Triangles from Revit's own saved graphics (branch open-research, step 2303).
//!
//! The channel-103 record of an element is a graph rooted in a `GElement` (decoded with the
//! file's schema, `rvt_core::graph`). Traversal and tessellation follow the functional
//! description of rvt-rs (Apache-2.0, `src/native_graphics_traversal.rs`,
//! `src/native_saved_mesh.rs`; the saved-graphics work there is by rosejn, PR #255), checked
//! against our own decoded records:
//! - `GElement`, `GGroup`, `GGTag`, `GFilter` hold children in `m_subNodes`; a node whose `GInfo`
//!   flags give visibility 1 or 3 is skipped; a `GFilter` is entered when all its conditions hold
//!   in a default 3D view (view direction (−1, −1, 1)/√3, view type 4, detail level 3);
//! - `GInstance`: `InstanceInfo.m_Trf` (row-major 3×3 + `m_or`) applied to its embedded graph or
//!   to the saved graphics of `InstanceInfo.m_symbolId`;
//! - `Geometry.m_pFaces` → `Face`: `m_pSurf`, loops from `m_pFirstLoop` along `m_nextLoop`; a loop
//!   walks its edges from `EdgeLoop.m_next` along `Edge.m_next[side]` (side: where
//!   `Edge.m_pFace[side]` is the face), each edge's `EdgePnt.uv[side]` samples (first, interior,
//!   last) in the face's parameter space; the UV polygon is triangulated and mapped by the surface
//!   (`Plane`: origin + u·x + v·y; `CylSurf`: centre + r(cos u·x + sin u·y) + v·z).
//! - `GPolyMesh` → `FacetedTopology0` points and facets.
//!
//! Step 2304 (checked against Revit's {3D} geometry of six models): a loop's first edge is
//! oriented towards the second; loops inside an odd number of others are holes, the rest outer
//! loops (a face can have several); `ConeSurf` (apex + v(sin α(cos u·x + sin u·y) + cos α·z)),
//! `SurfRev` (profile turned about z) and `RuledSurf` (lines between two profiles) with `GLine`,
//! `GArc`, `GEllipse` and `GHermiteSpline` profiles; curved faces are cut at a face-wide 5° grid;
//! nodes whose `GInfo` category is a graphics style of a subcategory a model view does not draw
//! on the instance (light source, MEP insulation/lining copies) are skipped.

use std::collections::HashMap;

use rvt_core::graph::{Graph, Object, Value};
use rvt_core::Schema;
use rvt_read::SavedGraphics;

type P3 = [f64; 3];
/// Row-major 3×3 and translation.
#[derive(Clone, Copy)]
struct Xf {
    m: [[f64; 3]; 3],
    t: P3,
}

impl Xf {
    const ID: Xf = Xf { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0; 3] };
    fn apply(&self, p: P3) -> P3 {
        std::array::from_fn(|i| self.m[i][0] * p[0] + self.m[i][1] * p[1] + self.m[i][2] * p[2] + self.t[i])
    }
    fn dir(&self, p: P3) -> P3 {
        std::array::from_fn(|i| self.m[i][0] * p[0] + self.m[i][1] * p[1] + self.m[i][2] * p[2])
    }
    /// self ∘ other (other applied first)
    fn then(&self, o: &Xf) -> Xf {
        let m = std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| self.m[i][k] * o.m[k][j]).sum()));
        Xf { m, t: self.apply(o.t) }
    }
    fn det(&self) -> f64 {
        let m = &self.m;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }
}

/// Saved-graphics triangles per element, with a cache of symbols in their own coordinates.
pub struct SavedMesher<'a> {
    sg: &'a SavedGraphics,
    symbols: HashMap<u64, Option<std::rc::Rc<Vec<[P3; 3]>>>>,
    /// statistics: faces drawn / faces skipped (unsupported surface or loop)
    pub faces: usize,
    pub skipped: usize,
    /// statistics: time spent decoding records
    pub decode_time: std::time::Duration,
}

impl<'a> SavedMesher<'a> {
    pub fn new(sg: &'a SavedGraphics) -> Self {
        SavedMesher { sg, symbols: HashMap::new(), faces: 0, skipped: 0, decode_time: Default::default() }
    }

    /// World triangles of an element (feet), none when it has no saved graphics.
    pub fn element(&mut self, id: u64) -> Option<Vec<[P3; 3]>> {
        let g = self.graph(id)?;
        let mut out = Vec::new();
        self.visit(&g, 0, Xf::ID, &mut out, 0);
        (!out.is_empty()).then_some(out)
    }

    fn graph(&mut self, id: u64) -> Option<Graph> {
        let body = self.sg.bodies.get(&id)?;
        let t = std::time::Instant::now();
        let g = Graph::decode(&self.sg.schema, body).ok();
        self.decode_time += t.elapsed();
        let g = g?;
        (g.class_name(&self.sg.schema, 0) == Some("GElement")).then_some(g)
    }

    fn symbol(&mut self, id: u64) -> Option<std::rc::Rc<Vec<[P3; 3]>>> {
        if let Some(c) = self.symbols.get(&id) {
            return c.clone();
        }
        self.symbols.insert(id, None); // guards against cycles
        let r = self.graph(id).map(|g| {
            let mut out = Vec::new();
            self.visit(&g, 0, Xf::ID, &mut out, 1);
            std::rc::Rc::new(out)
        });
        self.symbols.insert(id, r.clone());
        r
    }

    fn name(&self, o: &Object) -> &str {
        self.sg.schema.class(o.class).map_or("", |c| c.name.as_str())
    }

    fn visit(&mut self, g: &Graph, i: usize, xf: Xf, out: &mut Vec<[P3; 3]>, depth: usize) {
        if depth > 32 {
            return;
        }
        let Some(o) = g.objects.get(i) else { return };
        if let Some(gi) = o.get("m_GInfo").and_then(|v| v.as_obj()) {
            if let Some(flags) = gi.get("m_flags").and_then(|v| v.as_i64()) {
                if matches!(visibility(flags as u64), 1 | 3) {
                    return;
                }
            }
            // subcategories a default 3D view hides
            let cat = gi.get("m_categoryId").and_then(element_id).and_then(|st| self.sg.styles.get(&(st as u64)));
            if cat.is_some_and(|c| HIDDEN_CATEGORIES.contains(c)) {
                return;
            }
        }
        let s = &self.sg.schema;
        let name = self.name(o).to_string();
        let tree = env_flag(1);
        let before = out.len();
        if tree {
            let flags = o.get("m_GInfo").and_then(|v| v.as_obj()).and_then(|gi| gi.get("m_flags")).and_then(|v| v.as_i64()).unwrap_or(-1);
            let conds: Vec<String> = o.get("m_oConditions").and_then(|v| v.as_list()).unwrap_or(&[]).iter().filter_map(|v| v.as_ptr()).filter_map(|k| g.objects.get(k)).map(|c| {
                let f = |n: &str| c.get(n).map(|v| format!("{v:?}").chars().take(60).collect::<String>()).unwrap_or_default();
                format!("{}(param {} value {} comp {} dir {} neg {})", self.name(c), f("m_param"), f("m_value"), f("m_comp"), f("m_dir"), f("m_negateDirCondition"))
            }).collect();
            let held = if name == "GFilter" { format!(" holds {}", self.filter_holds(g, o, &xf)) } else { String::new() };
            let gi: Vec<String> = o.get("m_GInfo").and_then(|v| v.as_obj()).map(|gi| gi.fields.iter().filter(|(n, _)| n != "m_flags").map(|(n, v)| format!("{n}={}", element_id(v).map_or_else(|| format!("{v:?}").chars().take(40).collect::<String>(), |x| x.to_string()))).collect()).unwrap_or_default();
            eprintln!("{:indent$}{name} #{i} flags {flags:#x}{held} [{}] {}", "", gi.join(" "), conds.join(" "), indent = 2 * depth);
            if env_flag(2) {
                let rest: Vec<String> = o.fields.iter().filter(|(n, _)| n != "m_subNodes" && n != "m_oConditions" && n != "m_pFaces").map(|(n, v)| format!("{n}={}", format!("{v:?}").chars().take(160).collect::<String>())).collect();
                eprintln!("{:indent$}  {}", "", rest.join(" "), indent = 2 * depth);
            }
        }
        if name == "GFilter" && !self.filter_holds(g, o, &xf) {
            return;
        }
        if let Some(children) = o.get("m_subNodes").and_then(|v| v.as_list()) {
            for c in children.iter().filter_map(|v| v.as_ptr()) {
                self.visit(g, c, xf, out, depth + 1);
            }
        }
        match name.as_str() {
            "GInstance" => {
                let Some(info) = o.get("m_instanceInfo").and_then(|v| v.as_ptr()).and_then(|k| g.objects.get(k)) else { return };
                let Some(trf) = info.get("m_Trf").and_then(|v| v.as_obj()) else { return };
                let Some(local) = read_trf(trf) else { return };
                let xf2 = xf.then(&local);
                if let Some(e) = o.get("m_oEmbeddedSymbolGRep").and_then(|v| v.as_ptr()) {
                    self.visit(g, e, xf2, out, depth + 1);
                } else if let Some(sym) = info.get("m_symbolId").and_then(element_id) {
                    if let Some(tris) = self.symbol(sym as u64) {
                        out.extend(tris.iter().map(|t| t.map(|p| xf2.apply(p))));
                    }
                }
            }
            _ if s.is_a(o.class, "GBRep") => {
                if let Some(faces) = o.get("m_pFaces").and_then(|v| v.as_list()) {
                    for f in faces.iter().filter_map(|v| v.as_ptr()) {
                        let tris = face_triangles(s, g, f);
                        if env_flag(0) && tris.is_none() {
                            let fo = &g.objects[f];
                            let surf = fo.get("m_pSurf").and_then(|v| v.as_ptr()).and_then(|k| g.objects.get(k));
                            let lp = fo.get("m_pFirstLoop").and_then(|v| v.as_ptr());
                            let l = lp.and_then(|k| g.objects.get(k));
                            let uv = l.map(|l| loop_uv(g, l, f));
                            eprintln!("face {f}: surf {:?} loop {:?} uv {:?}", surf.map(|o| s.class(o.class).map(|c| c.name.clone())), lp, uv.map(|u| u.map(|p| p.len())));
                        }
                        match tris {
                            Some(tris) => {
                                self.faces += 1;
                                let flip = xf.det() < 0.0;
                                out.extend(tris.into_iter().map(|t| {
                                    let w = t.map(|p| xf.apply(p));
                                    if flip { [w[0], w[2], w[1]] } else { w }
                                }));
                            }
                            None => self.skipped += 1,
                        }
                    }
                }
            }
            "GPolyMesh" => {
                if let Some(tris) = polymesh(g, o) {
                    out.extend(tris.into_iter().map(|t| t.map(|p| xf.apply(p))));
                }
            }
            _ => {}
        }
        if tree && out.len() > before {
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for p in out[before..].iter().flatten() {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            eprintln!("{:indent$}= {name} #{i}: {} triangles box {lo:.2?} {hi:.2?}", "", out.len() - before, indent = 2 * depth);
        }
    }

    fn filter_holds(&self, g: &Graph, o: &Object, xf: &Xf) -> bool {
        let Some(conds) = o.get("m_oConditions").and_then(|v| v.as_list()) else { return true };
        conds.iter().filter_map(|v| v.as_ptr()).filter_map(|k| g.objects.get(k)).all(|c| {
            let n = self.name(c);
            match n {
                "GConditionDir" | "GConditionCut" => direction_condition(c, xf).unwrap_or(false),
                "GConditionInt" | "GConditionViewType" | "GConditionViewDetailLevel" | "GConditionVModeType" => int_condition(c).unwrap_or(false),
                _ => false,
            }
        })
    }
}

/// Debug switches, read once (the mesher is hot): 0 RVT_SAVED_DEBUG (faces), 1 RVT_SAVED_TREE
/// (nodes), 2 RVT_SAVED_TREE=2 (node fields), 3 RVT_SAVED_SURF (unsupported surfaces).
fn env_flag(k: usize) -> bool {
    static F: std::sync::OnceLock<[bool; 4]> = std::sync::OnceLock::new();
    F.get_or_init(|| {
        let tree = std::env::var("RVT_SAVED_TREE").ok();
        [std::env::var_os("RVT_SAVED_DEBUG").is_some(), tree.is_some(), tree.as_deref() == Some("2"), std::env::var_os("RVT_SAVED_SURF").is_some()]
    })[k]
}

/// Family subcategories a model view does not draw on the family instance itself: light source
/// (−2001121, off by default), and the insulation/lining copies inside MEP fittings and curves
/// (duct fitting −2008067/−2008068, pipe fitting −2008073, duct −2008063/−2008064, flex duct
/// −2008065, pipe −2008070, flex pipe −2008071; separate insulation elements show them; rme 2026
/// duct fittings were three times the Revit area).
const HIDDEN_CATEGORIES: &[i64] = &[-2001121, -2008067, -2008068, -2008073, -2008063, -2008064, -2008065, -2008070, -2008071];

/// `GInfo.m_flags` visibility class (1 and 3: not drawn).
fn visibility(flags: u64) -> u8 {
    let bits = flags & 0x1e0;
    if bits == 0 {
        0
    } else if bits == 0xe0 {
        3
    } else if bits < 0xe0 {
        if flags & 0x160 == 0x20 { 1 } else { 2 }
    } else if flags & 0x1a0 == 0x100 {
        1
    } else {
        2
    }
}

fn element_id(v: &Value) -> Option<i64> {
    v.element_id()
}

fn read_trf(t: &Object) -> Option<Xf> {
    let rows = t.get("m_3x3")?.as_list()?;
    let mut m = [[0.0; 3]; 3];
    for (i, r) in rows.iter().take(3).enumerate() {
        let f = r.floats()?;
        for j in 0..3 {
            m[i][j] = *f.get(j)?;
        }
    }
    let or = t.get("m_or")?.floats()?;
    Some(Xf { m, t: [or[0], or[1], or[2]] })
}

/// Direction conditions in a default 3D view (codes 6–8: false; 10/11: angle with the view
/// direction below/above 90°; 0/3 and −1: unequal/equal).
fn direction_condition(c: &Object, xf: &Xf) -> Option<bool> {
    let negate = c.get("m_negateDirCondition").and_then(|v| v.as_i64()).unwrap_or(0) != 0;
    let comp = c.get("m_comp")?.as_i64()?;
    if (6..=8).contains(&comp) {
        return Some(negate);
    }
    let d = c.get("m_dir")?.floats()?;
    let w = xf.dir([d[0], d[1], d[2]]);
    let len = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
    if len < 1e-12 {
        return None;
    }
    let view = [-0.577350269189625, -0.577350269189627, 0.577350269189626];
    let ans = match comp {
        10 | 11 => {
            let ang = ((w[0] * view[0] + w[1] * view[1] + w[2] * view[2]) / len).clamp(-1.0, 1.0).acos();
            if comp == 10 { ang < std::f64::consts::FRAC_PI_2 } else { ang > std::f64::consts::FRAC_PI_2 }
        }
        -1 | 0 | 3 => {
            let dist = (0..3).map(|i| (w[i] / len - view[i]).powi(2)).sum::<f64>().sqrt();
            let equal = dist < 1e-9;
            if comp == 0 { !equal } else { equal }
        }
        _ => return None,
    };
    Some(ans != negate)
}

/// Integer conditions: parameter 1 (view type) = 4, 3 (detail level) = 3 (Fine), 6 = 0.
fn int_condition(c: &Object) -> Option<bool> {
    let kind = c.get("m_param")?.as_i64()?;
    let saved = c.get("m_value")?.as_i64()?;
    let comp = c.get("m_comp")?.as_i64()?;
    if kind == 7 && saved == 2 && comp == 0 {
        return Some(true);
    }
    let actual = match kind {
        1 => 4,
        3 => 3,
        6 => 0,
        _ => return None,
    };
    Some(match comp {
        -1 | 3 => saved == actual,
        0 => saved != actual,
        1 => saved > actual,
        2 => saved >= actual,
        4 => saved <= actual,
        5 => saved < actual,
        _ => return None,
    })
}

/// A face's triangles in the coordinates of its graph.
fn face_triangles(s: &Schema, g: &Graph, fi: usize) -> Option<Vec<[P3; 3]>> {
    let f = g.objects.get(fi)?;
    let surf = g.objects.get(f.get("m_pSurf")?.as_ptr()?)?;
    let sname = s.class(surf.class)?.name.as_str();
    // loops in UV
    let mut loops: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut lp = f.get("m_pFirstLoop")?.as_ptr();
    let mut guard = 0;
    while let Some(li) = lp {
        guard += 1;
        if guard > 1000 {
            return None;
        }
        let l = g.objects.get(li)?;
        loops.push(loop_uv(g, l, fi)?);
        lp = l.get("m_nextLoop").and_then(|v| v.as_ptr());
    }
    if loops.is_empty() || loops[0].len() < 3 {
        return None;
    }
    // RVT_SAVED_FACE=<index>: the UV loops of that face object
    if std::env::var("RVT_SAVED_FACE").ok().and_then(|v| v.parse::<usize>().ok()) == Some(fi) {
        eprintln!("face {fi} loops {:?}", loops.iter().map(|l| l.iter().map(|p| [(p[0] * 1000.0).round() / 1000.0, (p[1] * 1000.0).round() / 1000.0]).collect::<Vec<_>>()).collect::<Vec<_>>());
    }
    // the largest loop first (the first loop need not be the outer one: door 505974's glass
    // openings); holes and further outer loops are told apart below
    let area = |l: &Vec<[f64; 2]>| -> f64 { (0..l.len()).map(|k| { let (a, b) = (l[k], l[(k + 1) % l.len()]); a[0] * b[1] - b[0] * a[1] }).sum::<f64>() / 2.0 };
    if let Some(big) = (0..loops.len()).max_by(|&a, &b| area(&loops[a]).abs().total_cmp(&area(&loops[b]).abs())) {
        loops.swap(0, big);
    }
    // the surface: UV → point, and the parameter step per direction that keeps chords close to
    // the surface (none: the surface is linear that way)
    let (map, ustep, vstep): (Box<dyn Fn([f64; 2]) -> P3>, Option<f64>, Option<f64>) = match sname {
        "Plane" => {
            let o = surf.get("m_origin")?.floats()?;
            let x = surf.get("m_xVec")?.floats()?;
            let y = surf.get("m_yVec")?.floats()?;
            (Box::new(move |uv: [f64; 2]| std::array::from_fn(|k| o[k] + uv[0] * x[k] + uv[1] * y[k])), None, None)
        }
        "CylSurf" => {
            let c = surf.get("m_center")?.floats()?;
            let r = surf.get("m_radius")?.as_f64()?;
            let (x, y, z) = frame(surf)?;
            (Box::new(move |uv: [f64; 2]| std::array::from_fn(|k| c[k] + r * (uv[0].cos() * x[k] + uv[0].sin() * y[k]) + uv[1] * z[k])), Some(arc_step(r)), None)
        }
        "ConeSurf" => {
            // (u, v): angle about z and distance along the slant from the apex `m_center`
            let c = surf.get("m_center")?.floats()?;
            let a = surf.get("m_halfAngle")?.as_f64()?;
            let (x, y, z) = frame(surf)?;
            let (sa, ca) = a.sin_cos();
            let r = loops.iter().flatten().map(|p| p[1].abs() * sa.abs()).fold(0.0, f64::max);
            (Box::new(move |uv: [f64; 2]| std::array::from_fn(|k| c[k] + uv[1] * (sa * (uv[0].cos() * x[k] + uv[0].sin() * y[k]) + ca * z[k]))), Some(arc_step(r)), None)
        }
        "SurfRev" => {
            // the profile curve (in the surface frame) at v, turned by u about z (functional
            // description: rvt-rs `native_parametric_surface.rs`)
            let c = surf.get("m_center")?.floats()?;
            let (x, y, z) = frame(surf)?;
            let prof = curve(g, surf.get("m_pProfileCurve")?.as_ptr()?)?;
            let vstep = prof.step;
            // the largest distance of the profile from the axis
            let r = (0..=8).map(|k| (prof.at)(prof.range[0] + (prof.range[1] - prof.range[0]) * k as f64 / 8.0)).map(|q| q[0].hypot(q[1])).fold(0.0, f64::max);
            (
                Box::new(move |uv: [f64; 2]| {
                    let q = (prof.at)(uv[1]);
                    let (su, cu) = uv[0].sin_cos();
                    let l = [q[0] * cu - q[1] * su, q[0] * su + q[1] * cu, q[2]];
                    std::array::from_fn(|k| c[k] + l[0] * x[k] + l[1] * y[k] + l[2] * z[k])
                }),
                Some(arc_step(r)),
                vstep,
            )
        }
        "RuledSurf" => {
            // straight lines between two profiles; u ∈ [0, 1] runs over each profile's range
            let p1 = curve(g, surf.get("m_pProfileCurve1")?.as_ptr()?)?;
            let p2 = curve(g, surf.get("m_pProfileCurve2")?.as_ptr()?)?;
            let ustep = [&p1, &p2].iter().filter_map(|c| c.step.map(|s| s / (c.range[1] - c.range[0]).abs().max(1e-9))).reduce(f64::min);
            (
                Box::new(move |uv: [f64; 2]| {
                    let a = (p1.at)(p1.range[0] + uv[0] * (p1.range[1] - p1.range[0]));
                    let b = (p2.at)(p2.range[0] + uv[0] * (p2.range[1] - p2.range[0]));
                    std::array::from_fn(|k| a[k] * (1.0 - uv[1]) + b[k] * uv[1])
                }),
                ustep,
                None,
            )
        }
        _ => {
            if env_flag(3) {
                eprintln!("surface {sname}: {:?}", surf.fields.iter().map(|(n, v)| format!("{n}={}", format!("{v:?}").chars().take(120).collect::<String>())).collect::<Vec<_>>());
            }
            return None;
        }
    };
    // refine the loops along curved directions
    for (axis, step) in [(0, ustep), (1, vstep)] {
        let Some(step) = step else { continue };
        for l in loops.iter_mut() {
            let mut fine = Vec::with_capacity(l.len());
            for k in 0..l.len() {
                let (a, b) = (l[k], l[(k + 1) % l.len()]);
                let n = ((b[axis] - a[axis]).abs() / step).ceil().max(1.0) as usize;
                for j in 0..n {
                    let t = j as f64 / n as f64;
                    fine.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
                }
            }
            *l = fine;
        }
    }
    let pts: Vec<[f64; 2]> = loops.iter().flatten().copied().collect();
    let (mut u0, mut u1) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut v0, mut v1) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in &pts {
        (u0, u1, v0, v1) = (u0.min(p[0]), u1.max(p[0]), v0.min(p[1]), v1.max(p[1]));
    }
    // grid lines of the face: the same for every triangle, so shared edges get the same points
    let grid = |lo: f64, hi: f64, step: Option<f64>| -> Option<(f64, f64)> { step.map(|s| (lo, (hi - lo) / ((hi - lo) / s).ceil().max(1.0))) };
    let (gu, gv) = (grid(u0, u1, ustep), grid(v0, v1, vstep));
    // a curved face that fills its UV box (cylinder sides): a grid of quads
    let boxed = (u1 - u0) * (v1 - v0);
    if (gu.is_some() || gv.is_some()) && loops.len() == 1 && (area(&loops[0]).abs() - boxed).abs() < 1e-6 * boxed.max(1e-12) {
        let cuts = |lo: f64, hi: f64, g: Option<(f64, f64)>| -> Vec<f64> {
            match g {
                Some((_, s)) => {
                    let n = ((hi - lo) / s).round().max(1.0) as usize;
                    (0..=n).map(|k| if k == n { hi } else { lo + s * k as f64 }).collect()
                }
                None => vec![lo, hi],
            }
        };
        let (us, vs) = (cuts(u0, u1, gu), cuts(v0, v1, gv));
        let flip = area(&loops[0]) < 0.0;
        let mut out = Vec::with_capacity(2 * us.len() * vs.len());
        for a in us.windows(2) {
            for b in vs.windows(2) {
                let q = [map([a[0], b[0]]), map([a[1], b[0]]), map([a[1], b[1]]), map([a[0], b[1]])];
                if flip {
                    out.push([q[0], q[2], q[1]]);
                    out.push([q[0], q[3], q[2]]);
                } else {
                    out.push([q[0], q[1], q[2]]);
                    out.push([q[0], q[2], q[3]]);
                }
            }
        }
        return Some(out);
    }
    // loops inside an odd number of others are holes of the smallest loop around them; the rest
    // are outer loops (a face can have several: racbasic 2026 table 981963)
    let inside = |p: [f64; 2], l: &[[f64; 2]]| -> bool {
        let mut c = false;
        for k in 0..l.len() {
            let (a, b) = (l[k], l[(k + 1) % l.len()]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                c = !c;
            }
        }
        c
    };
    // a test point of a loop: the middle of its first segment (vertices may touch another loop)
    let probe = |l: &[[f64; 2]]| [(l[0][0] + l[1 % l.len()][0]) / 2.0, (l[0][1] + l[1 % l.len()][1]) / 2.0];
    let around: Vec<Vec<usize>> = (0..loops.len()).map(|a| (0..loops.len()).filter(|&b| b != a && area(&loops[b]).abs() > area(&loops[a]).abs() && inside(probe(&loops[a]), &loops[b])).collect()).collect();
    let mut groups: Vec<(usize, Vec<usize>)> = (0..loops.len()).filter(|&a| around[a].len() % 2 == 0).map(|a| (a, Vec::new())).collect();
    for a in (0..loops.len()).filter(|&a| around[a].len() % 2 == 1) {
        let owner = around[a].iter().copied().filter(|&b| around[b].len() % 2 == 0).min_by(|&x, &y| area(&loops[x]).abs().total_cmp(&area(&loops[y]).abs()));
        if let Some(g) = owner.and_then(|o| groups.iter_mut().find(|g| g.0 == o)) {
            g.1.push(a);
        }
    }
    let mut out = Vec::new();
    for (outer, holes) in &groups {
        let mut flat: Vec<f64> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        let mut gp: Vec<[f64; 2]> = Vec::new();
        for (k, &l) in std::iter::once(outer).chain(holes.iter()).enumerate() {
            if k > 0 {
                starts.push(gp.len());
            }
            gp.extend_from_slice(&loops[l]);
            flat.extend(loops[l].iter().flatten());
        }
        let Ok(idx) = earcutr::earcut(&flat, &starts, 2) else { continue };
        if env_flag(0) {
            let tri_area: f64 = idx.chunks_exact(3).map(|t| { let (a, b, c) = (gp[t[0]], gp[t[1]], gp[t[2]]); ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0 }).sum();
            let poly = area(&loops[*outer]).abs() - holes.iter().map(|&l| area(&loops[l]).abs()).sum::<f64>();
            if (tri_area - poly).abs() > 1e-6 * poly.abs().max(1.0) {
                eprintln!("face {fi}: uv area {poly:.4} triangles {tri_area:.4} outer {outer} holes {holes:?} of {}", loops.len());
            }
        }
        for t in idx.chunks_exact(3) {
            // cut UV triangles at the grid lines so the surface is followed
            for part in cells([gp[t[0]], gp[t[1]], gp[t[2]]], gu, gv) {
                out.push(part.map(|p| map(p)));
            }
        }
    }
    if env_flag(0) && out.len() > 1000 {
        eprintln!("face {fi}: {sname} {} triangles, loops {:?}, uv [{u0:.3}, {v0:.3}] [{u1:.3}, {v1:.3}]", out.len(), loops.iter().map(|l| l.len()).collect::<Vec<_>>());
    }
    Some(out)
}

/// The angle step (radians) about an axis at distance `r` (feet) whose chords stay within 1.5 mm
/// of the arc, between 5° and 45° (step 2304b: 5° everywhere gave 11 million triangles for the
/// pipe fittings of five models).
fn arc_step(r: f64) -> f64 {
    let tol = 0.005;
    let a = if r > tol { 2.0 * (1.0 - tol / r).acos() } else { f64::INFINITY };
    a.clamp(5f64.to_radians(), 45f64.to_radians())
}

/// The orthonormal frame of a surface (`m_xVec`, `m_yVec`, `m_zVec`).
fn frame(surf: &Object) -> Option<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    Some((surf.get("m_xVec")?.floats()?, surf.get("m_yVec")?.floats()?, surf.get("m_zVec")?.floats()?))
}

/// A profile curve of a surface: its point at a parameter, its parameter range, and the
/// parameter step that follows its bends (none: straight).
struct Curve {
    at: Box<dyn Fn(f64) -> P3>,
    range: [f64; 2],
    step: Option<f64>,
}

/// `GLine` (`m_origin` + t·`m_dirVec`) or `GArc` (`m_center` + r(cos t·x + sin t·y)), with
/// `m_endParams`.
fn curve(g: &Graph, i: usize) -> Option<Curve> {
    let o = g.objects.get(i)?;
    let ends = o.get("m_endParams")?.floats()?;
    let range = [*ends.first()?, *ends.get(1)?];
    if o.get("m_dirVec").is_some() {
        let p = o.get("m_origin")?.floats()?;
        let d = o.get("m_dirVec")?.floats()?;
        return Some(Curve { at: Box::new(move |t| std::array::from_fn(|k| p[k] + t * d[k])), range, step: None });
    }
    if let Some(nodes) = o.get("m_NodeArray").and_then(|v| v.as_list()) {
        // GHermiteSpline: cubic Hermite pieces between nodes (parameter, point, tangent)
        let nodes: Vec<(f64, P3, P3)> = nodes
            .iter()
            .filter_map(|n| {
                let n = n.as_obj()?;
                let (p, t) = (n.get("m_iPoint")?.floats()?, n.get("m_iTangent")?.floats()?);
                Some((n.get("m_iParametr")?.as_f64()?, [p[0], p[1], p[2]], [t[0], t[1], t[2]]))
            })
            .collect();
        if nodes.len() < 2 {
            return None;
        }
        let step = (range[1] - range[0]).abs() / (8 * (nodes.len() - 1)) as f64;
        return Some(Curve {
            at: Box::new(move |t| {
                let k = nodes.windows(2).position(|w| t <= w[1].0).unwrap_or(nodes.len() - 2);
                let ((t0, p0, d0), (t1, p1, d1)) = (nodes[k], nodes[k + 1]);
                let h = (t1 - t0).max(1e-12);
                let s = (t - t0) / h;
                let (s2, s3) = (s * s, s * s * s);
                let (h00, h10, h01, h11) = (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2);
                std::array::from_fn(|i| h00 * p0[i] + h10 * h * d0[i] + h01 * p1[i] + h11 * h * d1[i])
            }),
            range,
            step: Some(step),
        });
    }
    let c = o.get("m_center")?.floats()?;
    let x = o.get("m_xVec")?.floats()?;
    let y = o.get("m_yVec")?.floats()?;
    // GArc (one radius) or GEllipse (two half-axes)
    let (rx, ry) = match o.get("m_radius").and_then(|v| v.as_f64()) {
        Some(r) => (r, r),
        None => (o.get("m_xLen")?.as_f64()?, o.get("m_yLen")?.as_f64()?),
    };
    Some(Curve { at: Box::new(move |t| std::array::from_fn(|k| c[k] + rx * t.cos() * x[k] + ry * t.sin() * y[k])), range, step: Some(arc_step(rx.abs().max(ry.abs()))) })
}

/// A UV triangle cut by the grid lines u = u0 + k·du and v = v0 + k·dv into convex pieces,
/// fanned into triangles (orientation kept).
fn cells(t: [[f64; 2]; 3], gu: Option<(f64, f64)>, gv: Option<(f64, f64)>) -> Vec<[[f64; 2]; 3]> {
    // the part of a convex polygon on one side of the line p[axis] = c
    fn clip(poly: &[[f64; 2]], axis: usize, c: f64, keep_above: bool) -> Vec<[f64; 2]> {
        let inside = |p: &[f64; 2]| if keep_above { p[axis] >= c } else { p[axis] <= c };
        let mut r = Vec::with_capacity(poly.len() + 2);
        for k in 0..poly.len() {
            let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
            if inside(&a) {
                r.push(a);
            }
            if inside(&a) != inside(&b) {
                let s = (c - a[axis]) / (b[axis] - a[axis]);
                let mut q = [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])];
                q[axis] = c;
                r.push(q);
            }
        }
        r
    }
    let mut pieces: Vec<Vec<[f64; 2]>> = vec![t.to_vec()];
    for (axis, g) in [(0, gu), (1, gv)] {
        let Some((o, step)) = g else { continue };
        let mut next = Vec::new();
        for poly in pieces {
            let lo = poly.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
            let hi = poly.iter().map(|p| p[axis]).fold(f64::NEG_INFINITY, f64::max);
            let (k0, k1) = (((lo - o) / step + 1e-9).floor() as i64, ((hi - o) / step - 1e-9).ceil() as i64);
            if k1 - k0 <= 1 {
                next.push(poly);
                continue;
            }
            for k in k0..k1 {
                let piece = clip(&clip(&poly, axis, o + step * k as f64, true), axis, o + step * (k + 1) as f64, false);
                if piece.len() >= 3 {
                    next.push(piece);
                }
            }
        }
        pieces = next;
    }
    let mut out = Vec::new();
    for p in pieces {
        for j in 1..p.len() - 1 {
            out.push([p[0], p[j], p[j + 1]]);
        }
    }
    out
}

/// The UV polygon of a loop of face `fi`.
fn loop_uv(g: &Graph, l: &Object, fi: usize) -> Option<Vec<[f64; 2]>> {
    let first = l.get("m_next")?.as_ptr()?;
    let mut edges: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut e = first;
    for _ in 0..100_000 {
        let edge = g.objects.get(e)?;
        if env_flag(0) && edges.is_empty() {
            eprintln!("  edge {e}: fields {:?}", edge.fields.iter().map(|(n, v)| (n.clone(), format!("{v:?}").chars().take(80).collect::<String>())).collect::<Vec<_>>());
        }
        let faces = edge.get("m_pFace")?.as_list()?;
        let side = faces.iter().position(|v| v.as_ptr() == Some(fi)).unwrap_or(0);
        let ends = edge.get("m_firstAndLastEdgePnts")?.as_list()?;
        let interior = edge.get("m_interiorEdgePnts").and_then(|v| v.as_list()).unwrap_or(&[]);
        let uv = |p: &Value| -> Option<[f64; 2]> {
            let l = p.as_obj()?.get("uv")?.as_list()?;
            let q = l.get(side)?.floats()?;
            Some([q[0], q[1]])
        };
        let mut seq: Vec<[f64; 2]> = Vec::with_capacity(interior.len() + 2);
        seq.push(uv(ends.first()?)?);
        for p in interior {
            seq.push(uv(p)?);
        }
        seq.push(uv(ends.get(1)?)?);
        edges.push(seq);
        // the last edge's next is the loop itself (as the first edge's prev is)
        match edge.get("m_next").and_then(|v| v.as_list()).and_then(|l| l.get(side)).and_then(|v| v.as_ptr()) {
            Some(n) if n != first && g.objects.get(n).is_some_and(|o| o.get("m_pFace").is_some_and(|v| v.as_list().is_some())) => e = n,
            _ => break,
        }
    }
    // chain the edges: the first one is oriented towards the second (its stored direction says
    // nothing: racbasic door 423107 came out with the rest of the loop reversed), every later one
    // continues from the previous end
    let d = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    if edges.len() > 1 {
        let (a, b) = (&edges[0], &edges[1]);
        let near = |p: [f64; 2]| d(p, b[0]).min(d(p, b[b.len() - 1]));
        if near(a[0]) < near(a[a.len() - 1]) {
            edges[0].reverse();
        }
    }
    let mut pts: Vec<[f64; 2]> = Vec::new();
    for mut seq in edges {
        if let Some(&prev) = pts.last() {
            if d(prev, seq[seq.len() - 1]) < d(prev, seq[0]) {
                seq.reverse();
            }
            pts.extend(seq.into_iter().skip(1)); // the start is the previous end
        } else {
            pts.extend(seq);
        }
    }
    // the last point closes the loop
    if pts.len() > 1 && (pts[0][0] - pts[pts.len() - 1][0]).hypot(pts[0][1] - pts[pts.len() - 1][1]) < 1e-9 {
        pts.pop();
    }
    Some(pts)
}

fn polymesh(g: &Graph, o: &Object) -> Option<Vec<[P3; 3]>> {
    let topo = g.objects.get(o.get("m_pFacetedTopology")?.as_ptr()?)?;
    let pts: Vec<P3> = topo
        .get("m_pointsArr")?
        .as_list()?
        .iter()
        .filter_map(|p| {
            let f = p.floats().or_else(|| p.as_obj().and_then(|o| o.fields.first()).and_then(|(_, v)| v.floats()))?;
            Some([f[0], f[1], f[2]])
        })
        .collect();
    let facets = topo.get("m_facetsArr")?.as_list()?;
    let mut out = Vec::new();
    for f in facets {
        let idx: Vec<i64> = f.as_list()?.iter().filter_map(|v| v.as_i64()).collect();
        if idx.len() >= 3 {
            let p = |i: i64| pts.get(i as usize).copied();
            out.push([p(idx[0])?, p(idx[1])?, p(idx[2])?]);
        }
    }
    Some(out)
}
