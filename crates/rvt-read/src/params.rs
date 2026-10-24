//! Parameters (§9.1) and value sets (§9.2): ParameterElements of a chain and the value sets in
//! Family (1967) and FamilySymbol (2022) bodies, i.e. the type values per symbol.

use std::collections::{HashMap, HashSet};

use rvt_core::{vtag, Chain};
use rvt_model::{ValueRow, ValueSet};

use crate::util::{f64_at, find, i32_at, i64_at, u32_at};

const TAG_FAMILY: u16 = 1967;
const TAG_SYMBOL: u16 = 2022;
const TAG_PARAM: u16 = 3152;
const TAG_SHARED_PARAM: u16 = 3151;

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// Python's `str.isprintable` (step 2263: the full table, including unassigned code points; snowdon
/// arch 2025: an IES file stored as bytes read as UTF-16 holds U+0A0D, unassigned).
fn printable(s: &str) -> bool {
    s.chars().all(|c| {
        let u = c as u32;
        let k = crate::printable_table::NOT_PRINTABLE.partition_point(|&(_, hi)| hi < u);
        !crate::printable_table::NOT_PRINTABLE.get(k).is_some_and(|&(lo, _)| lo <= u)
    })
}

/// `u32 n | n × UTF-16`; `None` when implausible (reference `_str`): longer than `maxlen`, past the
/// end, invalid UTF-16 or a control character.
fn read_str(b: &[u8], o: usize, maxlen: usize) -> Option<(String, usize)> {
    if o + 4 > b.len() {
        return None;
    }
    let n = u32_at(b, o) as usize;
    if n > maxlen || o + 4 + 2 * n > b.len() {
        return None;
    }
    let units: Vec<u16> = b[o + 4..o + 4 + 2 * n].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let s = String::from_utf16(&units).ok()?;
    if s.chars().any(|c| (c as u32) < 32) {
        return None;
    }
    Some((s, o + 4 + 2 * n))
}

/// ParameterElement ids of a chain (tag 3152 or 3151, or a short body with a 'revit.local.' /
/// 'autodesk.parameter.group' UniqueId string) with their name where it can be read (64-bit: the
/// string right before the 'revit.' id; else after the group string).
pub(crate) fn parameter_names(ch: &Chain) -> HashMap<u64, Option<String>> {
    let (pt, st, ft, yt) = (vtag(ch.version, TAG_PARAM), vtag(ch.version, TAG_SHARED_PARAM), vtag(ch.version, TAG_FAMILY), vtag(ch.version, TAG_SYMBOL));
    let keys = [utf16("revit.local."), utf16("autodesk.parameter.group")];
    let revit = utf16("revit.");
    let aec = utf16("autodesk.parameter.aec.");
    let group = utf16("autodesk.parameter.group");
    let mut out = HashMap::new();
    for e in ch.elements.iter() {
        if e.tag.is_some() && (e.tag == ft || e.tag == yt) {
            continue;
        }
        let bd = ch.body(e);
        let tagged = e.tag.is_some() && (e.tag == pt || e.tag == st) || e.tag == Some(TAG_SHARED_PARAM);
        if !tagged && !(bd.len() < 4096 && keys.iter().any(|k| find(bd, k).is_some())) {
            continue;
        }
        let mut t = find(bd, &revit);
        if let Some(t2) = find(bd, &aec) {
            if t.is_none_or(|t| t2 < t) {
                t = Some(t2);
            }
        }
        let name = match t {
            Some(t) => (1..150).find_map(|n| {
                let o = t.checked_sub(8 + 2 * n)?;
                (u32_at(bd, o) as usize == n).then(|| {
                    let units: Vec<u16> = bd[t - 4 - 2 * n..t - 4].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    String::from_utf16_lossy(&units)
                })
            }),
            None => find(bd, &group).and_then(|g| g.checked_sub(4)).and_then(|o| read_str(bd, o, 300)).and_then(|(_, o2)| read_str(bd, o2, 300)).map(|(s, _)| s),
        };
        if t.is_none() && find(bd, &group).is_none() && !tagged {
            continue;
        }
        out.insert(e.id, name);
    }
    out
}

/// §9.2: `str type name | u32 n | n × (str text | u32 X [+2 B when 0xffffffff] | f64 | id element |
/// id param | …)` (64-bit: i64 ids and 6 trailing bytes; 32-bit: i32 ids and 6 bytes).
fn value_set(b: &[u8], o: usize, valid: &dyn Fn(i64) -> bool, w32: bool) -> Option<(String, Vec<ValueRow>, usize)> {
    let (name, p) = read_str(b, o, 80)?;
    if !printable(&name) || p + 4 > b.len() {
        return None;
    }
    let cnt = u32_at(b, p) as usize;
    if !(1..=400).contains(&cnt) {
        return None;
    }
    let mut q = p + 4;
    let mut rows = Vec::with_capacity(cnt);
    for _ in 0..cnt {
        let (txt, q2) = read_str(b, q, 300)?;
        q = q2;
        if !printable(&txt) || q + if w32 { 26 } else { 34 } > b.len() {
            return None;
        }
        let x = u32_at(b, q);
        q += 4;
        if x == 0xffff_ffff {
            q += 2;
        } else if x != 0 {
            return None;
        }
        let (v, el, pid) = if w32 {
            let r = (f64_at(b, q), i32_at(b, q + 8) as i64, i32_at(b, q + 12) as i64);
            q += 22;
            r
        } else {
            let r = (f64_at(b, q), i64_at(b, q + 8), i64_at(b, q + 16));
            q += 30;
            r
        };
        if !valid(pid) {
            return None;
        }
        rows.push(ValueRow { param: pid, num: v, elem: el, text: txt });
    }
    Some((name, rows, q))
}

/// Value sets in the bodies of elements with the given (canonical) tags; type name '' = the
/// instance type (§16.1). A set needs at least two rows.
pub(crate) fn value_sets(ch: &Chain, params: &HashSet<u64>, tags: &[u16]) -> Vec<ValueSet> {
    let valid = |p: i64| (p > 0 && params.contains(&(p as u64))) || (-3_000_000 < p && p < -1_000_000);
    let vt: HashSet<Option<u16>> = tags.iter().map(|t| vtag(ch.version, *t)).collect();
    let w32 = ch.bits == Some(32);
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.tag.is_some() && vt.contains(&e.tag)) {
        let bd = ch.body(e);
        let l = bd.len();
        let mut o = 0usize;
        while o + 16 < l {
            let n0 = u32_at(bd, o) as usize;
            let p0 = o + 4 + 2 * n0;
            let s = if n0 <= 80 && p0 + 8 <= l && (1..=400).contains(&(u32_at(bd, p0) as usize)) && u32_at(bd, p0 + 4) <= 300 {
                value_set(bd, o, &valid, w32)
            } else {
                None
            };
            match s {
                Some((name, rows, end)) if rows.len() >= 2 => {
                    out.push(ValueSet { owner: e.id, type_name: name, rows });
                    o = end;
                }
                _ => o += 1,
            }
        }
    }
    out
}
