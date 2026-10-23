//! Floors and ceilings (§31) and roofs (§32): sketch-based elements.

use std::collections::{BTreeMap, HashMap, HashSet};

use rvt_core::{vtag, Chain, Element};
use rvt_model::{Curve, Floor, Roof, RoofEdge};

use crate::curves::{curves_in, is_unit};
use crate::util::{f64_at, find, i32_at, i64_at, u32_at};

pub(crate) const CURVE_CAT: i32 = -2000045;
const TAG_SKETCH: u16 = 4419;
const TAG_EXTRUSION_ROOF: u16 = 3764;
const TAG_ROOF_EDGE_ANCHOR: u16 = 0x0F5C;
pub(crate) const CAT_FLOORS: i32 = -2000032;
pub(crate) const CAT_STRUCTURAL_FOUNDATION: i32 = -2001300;
pub(crate) const CAT_CEILINGS: i32 = -2000038;
pub(crate) const CAT_ROOFS: i32 = -2000035;

/// Inputs gathered over the whole file.
pub(crate) struct SlabContext<'a> {
    pub level_z: &'a HashMap<i64, f64>,
    /// Compound type id -> total thickness (§25).
    pub type_width: &'a HashMap<i64, f64>,
    /// Ids per category over chain 0 of all partitions (types may live in another stream, step 389).
    pub cat_ids: &'a HashMap<i32, HashSet<i64>>,
}

struct Common<'a> {
    bits32: bool,
    w: usize,
    byid: HashMap<u64, &'a Element>,
    cur: HashSet<u64>,
    hdr: Vec<u8>,
}

impl<'a> Common<'a> {
    fn new(ch: &'a Chain) -> Option<Self> {
        let bits32 = ch.bits == Some(32);
        Some(Common {
            bits32,
            w: if bits32 { 4 } else { 8 },
            byid: ch.elements.iter().map(|e| (e.id, e)).collect(),
            cur: ch.elements.iter().filter(|e| e.cat == CURVE_CAT).map(|e| e.id).collect(),
            hdr: crate::hdr_marker(ch)?,
        })
    }

    fn rd(&self, b: &[u8], o: usize) -> i64 {
        if self.bits32 {
            i32_at(b, o) as i64
        } else {
            i64_at(b, o)
        }
    }

    fn pack(&self, v: i64) -> Vec<u8> {
        if self.bits32 {
            (v as i32).to_le_bytes().to_vec()
        } else {
            v.to_le_bytes().to_vec()
        }
    }

    /// Sketches (tag 4419) whose owner @127 (32-bit @91, step 1323) is an element of `cats`.
    fn owners(&self, ch: &'a Chain, cats: &[i32]) -> BTreeMap<u64, &'a Element> {
        let ow = if self.bits32 { 91 } else { 127 };
        let st = vtag(ch.version, TAG_SKETCH);
        let mut owner = BTreeMap::new();
        for e in ch.elements.iter().filter(|e| e.tag.is_some() && e.tag == st) {
            let bd = ch.body(e);
            if bd.len() >= ow + self.w {
                let f = self.rd(bd, ow);
                if f >= 0 && self.byid.get(&(f as u64)).is_some_and(|x| cats.contains(&x.cat)) {
                    owner.insert(f as u64, e);
                }
            }
        }
        owner
    }

    /// Header `ffffffff fd02 01000000 | id own | id level` within the first 200 bytes; (marker, level).
    fn head(&self, bd: &[u8], id: u64) -> Option<(usize, i64)> {
        let m = find(bd, &self.hdr)?;
        if m > 200 || m + 10 + 2 * self.w > bd.len() {
            return None;
        }
        (self.rd(bd, m + 10) == id as i64).then(|| (m, self.rd(bd, m + 10 + self.w)))
    }

    fn type_after(&self, bd: &[u8], m: usize, types: &HashSet<i64>) -> Option<usize> {
        (m + 10 + 2 * self.w..bd.len().saturating_sub(self.w)).find(|&o| types.contains(&self.rd(bd, o)))
    }

    fn sketch_curves(&self, ch: &Chain, sk: &Element) -> Vec<Curve> {
        let (lo, n) = sk.body;
        curves_in(self.bits32, &ch.data, &self.cur, lo, lo + n)
    }
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..).and_then(|h| find(h, needle)).map(|o| o + from)
}

/// §31 (steps 383–386): floors (categories −2000032/−2001300, offset BIP −1001951) and, with other
/// arguments, ceilings (step 393: −2000038, BIP −1002300).
pub(crate) fn floors(ch: &Chain, ctx: &SlabContext, cats: &[i32], off_bip: i64) -> Vec<Floor> {
    let Some(c) = Common::new(ch) else { return vec![] };
    let owner = c.owners(ch, cats);
    let mut ftypes: HashSet<i64> = ch.elements.iter().filter(|e| cats.contains(&e.cat)).map(|e| e.id as i64).collect();
    for cat in cats {
        ftypes.extend(ctx.cat_ids.get(cat).into_iter().flatten());
    }
    for f in owner.keys() {
        ftypes.remove(&(*f as i64));
    }
    let mut out = Vec::new();
    for (&fid, sk) in &owner {
        let bd = ch.body(c.byid[&fid]);
        let Some((m, lvl)) = c.head(bd, fid) else { continue };
        let t = c.type_after(bd, m, &ftypes);
        let typ = t.map(|t| c.rd(bd, t));
        let off = match find_from(bd, &c.pack(off_bip), t.unwrap_or(0)) {
            Some(k) if k >= 8 => f64_at(bd, k - 8),
            _ => 0.0,
        };
        let top = ctx.level_z.get(&lvl).copied();
        out.push(Floor {
            id: fid,
            sketch: sk.id,
            level: lvl,
            floor_type: typ,
            offset: off,
            thickness: typ.and_then(|t| ctx.type_width.get(&t).copied()),
            top_z: top.map(|z| z + off),
            curves: c.sketch_curves(ch, sk),
        });
    }
    out
}

/// Step 392: sketch plane frame `u8 01 | O | X | Y` with all profile points in the plane; returns N = X × Y.
fn sketch_normal(sb: &[u8], pts: &[[f64; 3]]) -> Option<[f64; 3]> {
    if pts.is_empty() {
        return None;
    }
    for o in 1..sb.len().saturating_sub(72) {
        if sb[o - 1] != 1 {
            continue;
        }
        let v: Vec<f64> = (0..9).map(|i| f64_at(sb, o + 8 * i)).collect();
        let (org, x, y) = ([v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]);
        if !(v.iter().all(|t| t.is_finite()) && is_unit(&x) && is_unit(&y) && (x[0] * y[0] + x[1] * y[1] + x[2] * y[2]).abs() < 1e-9) {
            continue;
        }
        let n = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
        if pts.iter().all(|p| ((p[0] - org[0]) * n[0] + (p[1] - org[1]) * n[1] + (p[2] - org[2]) * n[2]).abs() < 1e-6) {
            return Some(n);
        }
    }
    None
}

/// §32 (steps 387–389): footprint roofs; extrusion roofs (steps 391–392, 1359).
pub(crate) fn roofs(ch: &Chain, ctx: &SlabContext) -> Vec<Roof> {
    let Some(c) = Common::new(ch) else { return vec![] };
    let owner = c.owners(ch, &[CAT_ROOFS]);
    let mut rtypes: HashSet<i64> = ch.elements.iter().filter(|e| e.cat == CAT_ROOFS).map(|e| e.id as i64).collect();
    rtypes.extend(ctx.cat_ids.get(&CAT_ROOFS).into_iter().flatten());
    for f in owner.keys() {
        rtypes.remove(&(*f as i64));
    }
    // angle / 'defines slope' relative to the edge anchor (step 1360: 32-bit flag @169)
    let (aa, as_) = if c.bits32 { (117usize, 169usize) } else { (133, 205) };
    let amk: Option<Vec<u8>> = vtag(ch.version, TAG_ROOF_EDGE_ANCHOR).map(|t| {
        let mut m = vec![0xff; 4];
        m.extend_from_slice(&t.to_le_bytes());
        m
    });
    let ext_tag = vtag(ch.version, TAG_EXTRUSION_ROOF);
    let mut out = Vec::new();
    for (&rid, sk) in &owner {
        let re = c.byid[&rid];
        let bd = ch.body(re);
        let Some((m, lvl)) = c.head(bd, rid) else { continue };
        let typ = c.type_after(bd, m, &rtypes).map(|t| c.rd(bd, t));
        let off = match find(bd, &c.pack(-1001701)) {
            Some(k) if k >= 8 => f64_at(bd, k - 8),
            _ => 0.0,
        };
        let edges: Vec<RoofEdge> = c
            .sketch_curves(ch, sk)
            .into_iter()
            .map(|cv| {
                let cb: &[u8] = c.byid.get(&cv.id).map(|e| ch.body(e)).unwrap_or(&[]);
                let a = amk.as_ref().and_then(|mk| find(cb, mk));
                let ok = a.filter(|&a| a + as_ + 1 <= cb.len());
                RoofEdge { slope: ok.map(|a| cb[a + as_] != 0), angle: ok.map(|a| f64_at(cb, a + aa)), curve: cv }
            })
            .collect();
        let z = ctx.level_z.get(&lvl).copied();
        let mut r = Roof {
            id: rid,
            kind: "footprint",
            sketch: sk.id,
            level: lvl,
            roof_type: typ,
            offset: off,
            thickness: typ.and_then(|t| ctx.type_width.get(&t).copied()),
            base_z: z.map(|z| z + off),
            edges,
            start: None,
            end: None,
            normal: None,
        };
        if re.tag.is_some() && re.tag == ext_tag {
            let pts: Vec<[f64; 3]> = r.edges.iter().filter_map(|e| Some([e.curve.p0?, e.curve.p1?])).flatten().collect();
            let normal = sketch_normal(ch.body(sk), &pts);
            let mut so = m + 114;
            // step 1359: before start/end a list `u32 n | n × 6 bytes` + 12 zero bytes
            let (no, base) = if c.bits32 { (m + 62, m + 78) } else { (m + 98, m + 114) };
            let n = if no + 4 <= bd.len() { u32_at(bd, no) as usize } else { 0 };
            if n < 1000 && base + 6 * n + 16 <= bd.len() {
                so = base + 6 * n;
            }
            r.kind = "extrusion";
            r.start = Some(f64_at(bd, so));
            r.end = Some(f64_at(bd, so + 8));
            r.normal = Some(normal);
            r.base_z = None;
        }
        out.push(r);
    }
    out
}
