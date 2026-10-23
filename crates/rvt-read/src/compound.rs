//! Compound structures of wall, floor, roof and ceiling types (§25).

use rvt_core::Chain;
use rvt_model::{CompoundType, Layer};

use crate::util::{f64_at, i32_at, i64_at, last_lstr_before, py_sum, u32_at};

/// Layer function codes (step 789: 200 = structural deck).
fn layer_function(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "None",
        1 => "Structure",
        2 => "Substrate",
        3 => "Insulation",
        4 => "Finish1",
        5 => "Finish2",
        100 => "Membrane",
        200 => "StructuralDeck",
        _ => return None,
    })
}

fn is_fn_byte(b: u8) -> bool {
    matches!(b, 1..=5 | 100)
}

/// Step 319: candidate starts = (start of `ff×8 | u32 function (1..5, 100)`) − 20, as non-overlapping
/// regex matches would give them.
fn candidates64(bd: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 12 <= bd.len() {
        if bd[p..p + 8] == [0xff; 8] && is_fn_byte(bd[p + 8]) && bd[p + 9..p + 12] == [0, 0, 0] {
            if p >= 20 {
                out.push(p - 20);
            }
            p += 12;
        } else {
            p += 1;
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// §25 (steps 293–294): list = `u32 n` + n × 41 bytes (2025: 37 bytes, without function2):
/// f64 width | i64 material | i64 −1 | u32 function | [u32 function2] | i32 −1 | u32 index | u8 cap.
fn types64(ch: &Chain) -> Vec<CompoundType> {
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.cat < -2000000) {
        let bd = ch.body(e);
        let l = bd.len();
        let mut hit = None;
        'cand: for o in candidates64(bd) {
            if o + 41 > l {
                continue;
            }
            let n = u32_at(bd, o) as usize;
            if !(1..=50).contains(&n) {
                continue;
            }
            for r_len in [41usize, 37] {
                if o + 4 + r_len * n > l {
                    continue;
                }
                let ok = (0..n).all(|i| {
                    let r = o + 4 + r_len * i;
                    let f = u32_at(bd, r + 24);
                    let w = f64_at(bd, r);
                    let idx = u32_at(bd, r + r_len - 5);
                    layer_function(f).is_some()
                        && (idx == i as u32 || idx == 0) // step 788
                        && bd[r + r_len - 1] <= 1
                        && (bd[r + 16..r + 24] == [0xff; 8] || f == 200)
                        && (bd[r + r_len - 9..r + r_len - 5] == [0xff; 4] || (f == 200 && bd[r + r_len - 9..r + r_len - 5] == [0; 4]))
                        && (0.0..50.0).contains(&w)
                        && !(n == 1 && w <= 0.0) // step 307
                });
                if ok {
                    hit = Some((o, n, r_len));
                    break 'cand;
                }
            }
        }
        let Some((o, n, r_len)) = hit else { continue };
        let layers: Vec<Layer> = (0..n)
            .map(|i| {
                let r = o + 4 + r_len * i;
                let f = u32_at(bd, r + 24);
                let m = i64_at(bd, r + 8);
                Layer {
                    function: layer_function(f).expect("checked"),
                    material: (m > 0).then_some(m),
                    width: f64_at(bd, r),
                    cap: bd[r + r_len - 1] != 0, // steps 427–428
                    deck_profile: (f == 200).then(|| i64_at(bd, r + 16)),
                }
            })
            .collect();
        let end = o + 4 + r_len * n;
        let tail = end + 40 <= l;
        out.push(CompoundType {
            id: e.id,
            cat: e.cat,
            name: last_lstr_before(bd, o), // step 295
            width: py_sum(layers.iter().map(|x| x.width)),
            layers,
            structural_index: tail.then(|| i32_at(bd, end + 20)), // step 296
            core: tail.then(|| [i32_at(bd, end + 32), i32_at(bd, end + 36)]),
        });
    }
    out
}

/// Step 1349: 32-bit list = `u32 n` + n × 29 bytes: f64 width | u32 function | i32 −1 |
/// i32 material | i32 −1 | u32 index | u8 cap. After the list (E): structural @E+16, core @E+28/@E+32.
fn types32(ch: &Chain) -> Vec<CompoundType> {
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.cat < -2000000) {
        let bd = ch.body(e);
        let l = bd.len();
        let mut hit = None;
        for p in 0..l.saturating_sub(7) {
            if !(is_fn_byte(bd[p]) && bd[p + 1..p + 4] == [0, 0, 0] && bd[p + 4..p + 8] == [0xff; 4]) {
                continue;
            }
            let Some(o) = p.checked_sub(12) else { continue };
            if o + 33 > l {
                continue;
            }
            let n = u32_at(bd, o) as usize;
            if !(1..=50).contains(&n) || o + 4 + 29 * n > l {
                continue;
            }
            let ok = (0..n).all(|i| {
                let r = o + 4 + 29 * i;
                let w = f64_at(bd, r);
                let idx = u32_at(bd, r + 24);
                layer_function(u32_at(bd, r + 8)).is_some()
                    && bd[r + 12..r + 16] == [0xff; 4]
                    && bd[r + 20..r + 24] == [0xff; 4]
                    && (idx == i as u32 || idx == 0)
                    && bd[r + 28] <= 1
                    && (w == 0.0 || (1e-6 < w && w < 50.0))
            });
            let end = o + 4 + 29 * n;
            if ok && end + 36 <= l && [i32_at(bd, end + 28), i32_at(bd, end + 32)].iter().all(|&c| -1 <= c && c <= n as i32 + 1) {
                hit = Some((o, n));
                break;
            }
        }
        let Some((o, n)) = hit else { continue };
        let layers: Vec<Layer> = (0..n)
            .map(|i| {
                let r = o + 4 + 29 * i;
                let mt = i32_at(bd, r + 16) as i64;
                Layer {
                    function: layer_function(u32_at(bd, r + 8)).expect("checked"),
                    material: (mt > 0).then_some(mt),
                    width: f64_at(bd, r),
                    cap: bd[r + 28] != 0,
                    deck_profile: None,
                }
            })
            .collect();
        let end = o + 4 + 29 * n;
        out.push(CompoundType {
            id: e.id,
            cat: e.cat,
            name: last_lstr_before(bd, o),
            width: py_sum(layers.iter().map(|x| x.width)),
            layers,
            structural_index: Some(i32_at(bd, end + 16)),
            core: Some([i32_at(bd, end + 28), i32_at(bd, end + 32)]),
        });
    }
    out
}

pub(crate) fn compound_types(ch: &Chain) -> Vec<CompoundType> {
    if ch.bits == Some(32) {
        types32(ch)
    } else {
        types64(ch)
    }
}
