//! Topography (step 1532): TopographySurface (category −2001340, 2026 tag 3919) holds point lists
//! `u32 n | n × 3 f64` in its body; the largest list is the terrain points (rac_basic 607/515/146/162
//! points, identical 2014–2026), the smaller ones are probably boundary or helper geometry.

use std::collections::HashSet;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

use rvt_core::{vtag, Chain};
use rvt_model::Topography;

use crate::util::f64_at;

const TAG_TOPOGRAPHY: u16 = 3919;
const CAT_TOPOGRAPHY: i32 = -2001340;
/// The most points a list can hold.
const MAX_POINTS: usize = 200_000;
/// Bodies at least this long are scanned on several threads, in pieces of this many offsets.
const PARALLEL_SCAN: usize = 1 << 16;
const PIECE: usize = 1 << 15;

pub(crate) fn topography(ch: &Chain) -> Vec<Topography> {
    let Some(t) = vtag(ch.version, TAG_TOPOGRAPHY) else { return vec![] };
    let mut out = Vec::new();
    for e in ch.elements.iter().filter(|e| e.cat == CAT_TOPOGRAPHY && e.tag == Some(t)) {
        let bd = ch.body(e);
        let lists = point_lists(bd);
        // the first longest list
        let Some(best) = (0..lists.len()).fold(None, |b: Option<usize>, i| match b {
            Some(j) if lists[j].1 >= lists[i].1 => Some(j),
            _ => Some(i),
        }) else {
            continue;
        };
        let other_lists = lists.iter().enumerate().filter(|(i, _)| *i != best).map(|(_, x)| x.1).collect();
        let (q, n) = lists[best];
        let points = (0..n).map(|k| point(bd, q + 4 + 24 * k)).collect();
        out.push(Topography { id: e.id, points, other_lists });
    }
    out
}

/// Offset and length of the point lists in a body, in the order of their offsets. Every offset is
/// tried (a false early hit must not skip over the real list). Many offsets look like the start of
/// a long list, so how many sound points follow each byte is worked out once, and the distinct
/// points are counted only where the whole list is sound (a terrain of 40 000 points: 4 s before).
/// A large body is scanned in pieces on several threads, joined in order.
fn point_lists(bd: &[u8]) -> Vec<(usize, usize)> {
    let end = bd.len().saturating_sub(28);
    let run = sound_runs(bd);
    let lists = |from: usize, to: usize| (from..to).filter_map(|q| list_at(bd, &run, q)).collect::<Vec<_>>();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    if end < PARALLEL_SCAN || threads == 1 {
        return lists(0, end);
    }
    let pieces = end.div_ceil(PIECE);
    let next = AtomicUsize::new(0);
    let mut found: Vec<Vec<(usize, usize)>> = vec![Vec::new(); pieces];
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= pieces {
                            break mine;
                        }
                        mine.push((i, lists(i * PIECE, ((i + 1) * PIECE).min(end))));
                    }
                })
            })
            .collect();
        for w in workers {
            for (i, l) in w.join().expect("topography scan") {
                found[i] = l;
            }
        }
    });
    found.concat()
}

/// The point (3 f64) at byte `o`.
fn point(bd: &[u8], o: usize) -> [f64; 3] {
    [f64_at(bd, o), f64_at(bd, o + 8), f64_at(bd, o + 16)]
}

/// For every byte, how many sound points (finite, within 100 km) follow one another from it.
fn sound_runs(bd: &[u8]) -> Vec<u32> {
    let mut run = vec![0u32; bd.len()];
    for o in (0..bd.len().saturating_sub(23)).rev() {
        if point(bd, o).iter().all(|v| v.is_finite() && v.abs() < 1e5) {
            run[o] = 1 + run.get(o + 24).copied().unwrap_or(0);
        }
    }
    run
}

/// The list `u32 n | n × 3 f64` at offset `q`, when it holds 3 to 200 000 sound points and more
/// than 90 % of them are distinct (as the reference counts them, 0.0 == -0.0).
fn list_at(bd: &[u8], run: &[u32], q: usize) -> Option<(usize, usize)> {
    let n = u32::from_le_bytes(bd[q..q + 4].try_into().expect("4 bytes")) as usize;
    if !(3..=MAX_POINTS).contains(&n) || q + 4 + 24 * n > bd.len() || (run[q + 4] as usize) < n {
        return None;
    }
    let key = |v: f64| if v == 0.0 { 0u64 } else { v.to_bits() };
    let mut seen: HashSet<[u64; 3], BuildHasherDefault<Mix>> = HashSet::with_capacity_and_hasher(n, Default::default());
    let mut repeated = 0;
    for k in 0..n {
        let p = point(bd, q + 4 + 24 * k);
        // once too many points repeat, the list cannot reach 90 % distinct points any more
        if !seen.insert([key(p[0]), key(p[1]), key(p[2])]) {
            repeated += 1;
            if ((n - repeated) as f64) <= n as f64 * 0.9 {
                return None;
            }
        }
    }
    Some((q, n))
}

/// A quick hasher for counting distinct points (the keys are the bits of coordinates).
#[derive(Default)]
struct Mix(u64);

impl Hasher for Mix {
    fn write(&mut self, bytes: &[u8]) {
        for c in bytes.chunks(8) {
            let mut b = [0u8; 8];
            b[..c.len()].copy_from_slice(c);
            self.write_u64(u64::from_le_bytes(b));
        }
    }

    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0 ^ v).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    }

    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }

    fn finish(&self) -> u64 {
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(points: &[[f64; 3]]) -> Vec<u8> {
        let mut b = (points.len() as u32).to_le_bytes().to_vec();
        for p in points {
            for v in p {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn a_list_is_found_after_a_false_start_and_repeated_points_reject_one() {
        let terrain: Vec<[f64; 3]> = (0..50).map(|i| [i as f64, (i * 7 % 13) as f64, 0.5 * i as f64]).collect();
        let mut bd = vec![1, 2, 3];
        bd.extend(list(&terrain));
        // four points, two of them the same: 75 % distinct
        bd.extend(list(&[[1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [2.0, 2.0, 2.0], [3.0, 3.0, 3.0]]));
        // -0.0 and 0.0 count as one point
        bd.extend(list(&[[0.0, 0.0, 0.0], [-0.0, 0.0, 0.0], [5.0, 5.0, 5.0]]));
        bd.extend([0u8; 40]);
        let found = point_lists(&bd);
        assert!(found.contains(&(3, 50)), "{found:?}");
        let (repeated, signed_zero) = (3 + 4 + 50 * 24, 3 + 4 + 50 * 24 + 4 + 4 * 24);
        assert!(found.iter().all(|&(q, _)| q != repeated && q != signed_zero), "{found:?}");
        assert_eq!(found, slow_point_lists(&bd));
    }

    #[test]
    fn a_large_body_scanned_on_threads_finds_what_one_thread_finds() {
        let mut bd = Vec::new();
        for r in 0..40 {
            let pts: Vec<[f64; 3]> = (0..2000).map(|i| [(i + r) as f64 * 0.37, (i % 97) as f64 * 1.3, r as f64]).collect();
            bd.extend(list(&pts));
            bd.extend(std::iter::repeat(r as u8).take(1 + r));
        }
        assert!(bd.len() > PARALLEL_SCAN);
        assert_eq!(point_lists(&bd), slow_point_lists(&bd));
    }

    /// The scan as it was: every point read and hashed at every offset.
    fn slow_point_lists(bd: &[u8]) -> Vec<(usize, usize)> {
        (0..bd.len().saturating_sub(28))
            .filter(|&q| {
                let n = u32::from_le_bytes(bd[q..q + 4].try_into().unwrap()) as usize;
                if !(3..=MAX_POINTS).contains(&n) || q + 4 + 24 * n > bd.len() {
                    return false;
                }
                let pts: Vec<[f64; 3]> = (0..n).map(|k| point(bd, q + 4 + 24 * k)).collect();
                if !pts.iter().flatten().all(|v| v.is_finite() && v.abs() < 1e5) {
                    return false;
                }
                let key = |v: f64| if v == 0.0 { 0u64 } else { v.to_bits() };
                let uniq: HashSet<[u64; 3]> = pts.iter().map(|p| [key(p[0]), key(p[1]), key(p[2])]).collect();
                uniq.len() as f64 > n as f64 * 0.9
            })
            .map(|q| (q, u32::from_le_bytes(bd[q..q + 4].try_into().unwrap()) as usize))
            .collect()
    }
}
