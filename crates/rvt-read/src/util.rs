//! Byte helpers shared by the readers.

/// First occurrence of `needle` in `hay`.
pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Last occurrence of `needle` that starts before `end`.
pub fn rfind_before(hay: &[u8], needle: &[u8], end: usize) -> Option<usize> {
    let end = end.min(hay.len());
    if needle.len() > end {
        return None;
    }
    (0..=end - needle.len()).rev().find(|&o| &hay[o..o + needle.len()] == needle)
}

pub fn f64_at(b: &[u8], o: usize) -> f64 {
    f64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"))
}

pub fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"))
}

pub fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"))
}

pub fn i64_at(b: &[u8], o: usize) -> i64 {
    i64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"))
}

/// Python `str.isprintable` approximation: no control, format, separator (other than space) or
/// surrogate/private-use characters.
pub fn is_printable(s: &str) -> bool {
    s.chars().all(|c| {
        c == ' '
            || !(c.is_control()
                || c.is_whitespace()
                || matches!(c as u32, 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB | 0xE000..=0xF8FF))
    })
}

/// `u32 n | n × UTF-16` at `o` if it fits before `end`; `None` on invalid UTF-16 (strict, like Python).
pub fn lstr_at(b: &[u8], o: usize, end: usize, max_n: usize) -> Option<(usize, String)> {
    if o + 4 > end.min(b.len()) {
        return None;
    }
    let n = u32_at(b, o) as usize;
    if !(1..=max_n).contains(&n) || o + 4 + 2 * n > end {
        return None;
    }
    let units: Vec<u16> = b[o + 4..o + 4 + 2 * n].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16(&units).ok().map(|s| (n, s))
}

/// Last readable `u32 n | n×UTF-16` (printable, with a letter or digit) that ends before `end`
/// (reference `_last_lstr_before`, window 600).
pub fn last_lstr_before(b: &[u8], end: usize) -> Option<String> {
    let mut best = None;
    let mut o = end.saturating_sub(600);
    while o + 4 < end {
        if let Some((n, v)) = lstr_at(b, o, end, 200) {
            if is_printable(&v) && v.chars().any(char::is_alphanumeric) {
                best = Some(v);
                o += 4 + 2 * n;
                continue;
            }
        }
        o += 1;
    }
    best
}

/// Sum as CPython 3.12+ `sum()` computes it for floats (Neumaier compensated summation), so that
/// totals match the reference export bit for bit.
pub fn py_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for x in values {
        let t = s + x;
        if s.abs() >= x.abs() {
            c += (s - t) + x;
        } else {
            c += (x - t) + s;
        }
        s = t;
    }
    if c != 0.0 && c.is_finite() {
        s += c;
    }
    s
}

pub fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"))
}
