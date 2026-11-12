//! Partition streams: inserted-byte removal (§2.1), gzip members (§2.5), chains (§2.2–2.4)
//! and the element tables, section A and section B (§3, §4).

use std::io::Read;

use crate::tags::{version32, version64};
use crate::{le_f64, le_i32, le_i64, le_u16, le_u32, le_u64, Error, Result};

const UNIT: usize = 65249;
const EXTRA: usize = 353;

/// One chain of a partition stream: the inflated data and its element table.
#[derive(Debug, Clone)]
pub struct Chain {
    pub index: usize,
    pub key: Option<[u8; 16]>,
    pub count: u32,
    pub data: Vec<u8>,
    pub bits: Option<u8>,
    pub version: Option<u16>,
    pub elements: Vec<Element>,
    /// u16 @8 of the partition header; separates 2015 from 2016 (step 1510).
    pub hdr: u16,
}

/// A section A entry plus the location of its section B body.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub id: u64,
    pub cat: i32,
    pub tag: Option<u16>,
    pub bbox: Option<[f64; 6]>,
    pub refs: Option<Vec<u64>>,
    /// OwnerViewId (step 673, 64-bit; step 1372, 32-bit).
    pub view: Option<i64>,
    /// 32-bit only: owning family (i32 @18) or −1 (step 2103).
    pub owner32: Option<i32>,
    /// (offset, length) of the body in `Chain::data` (§4).
    pub body: (usize, usize),
}

impl Chain {
    pub fn body(&self, e: &Element) -> &[u8] {
        &self.data[e.body.0..e.body.0 + e.body.1]
    }
}

/// §2.1: remove [n·65249 − 353, n·65249) for n = 1, 2, …
pub fn strip_inserted(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let (mut start, mut n) = (0usize, 1usize);
    while n * UNIT - EXTRA < raw.len() {
        out.extend_from_slice(&raw[start..n * UNIT - EXTRA]);
        start = (n * UNIT).min(raw.len());
        n += 1;
    }
    out.extend_from_slice(&raw[start..]);
    out
}

/// §2.5: a gzip member; raw deflate from member + 10, CRC32/ISIZE checked. Inflated into `out`,
/// which has the length its ISIZE gives.
fn inflate(raw: &[u8], pos: usize, length: usize, out: &mut [u8]) -> Result<()> {
    let src = raw
        .get(pos + 10..pos + length)
        .ok_or_else(|| Error::Format(format!("gzip member at {pos} out of range")))?;
    let mismatch = || Error::Format(format!("CRC/ISIZE mismatch in member at {pos}"));
    let mut decoder = flate2::read::DeflateDecoder::new(src);
    decoder.read_exact(out).map_err(|_| mismatch())?;
    // nothing may follow what its ISIZE announced
    if decoder.read(&mut [0u8; 1])? != 0 {
        return Err(mismatch());
    }
    let crc = le_u32(raw, pos + length - 8)?;
    let mut c = flate2::Crc::new();
    c.update(out);
    if c.sum() != crc {
        return Err(mismatch());
    }
    Ok(())
}

/// The length a gzip member inflates to: its ISIZE, the last four bytes.
fn inflated_len(raw: &[u8], pos: usize, length: usize) -> Result<usize> {
    if length < 18 {
        return Err(Error::Format(format!("gzip member at {pos} out of range")));
    }
    Ok(le_u32(raw, pos + length - 4)? as usize)
}

/// §2.2–2.4: chains follow the header lengths.
pub fn chains(raw: &[u8]) -> Result<Vec<Chain>> {
    let head = le_u32(raw, 0)?;
    if head == 5 {
        // §1.3 Revit 2008: 8 bytes + one gzip member
        let mut data = Vec::new();
        flate2::read::GzDecoder::new(&raw[8..]).read_to_end(&mut data)?;
        return Ok(vec![Chain { index: 0, key: None, count: u32::MAX, data, bits: None, version: None, elements: vec![], hdr: 0 }]);
    }
    // step 1338: 2014 has header 7; step 1476: 2018 header 8 (2016/2017 7, 2019 9)
    if !matches!(head, 7..=9) {
        return Err(Error::Format(format!("unknown partition header {head}")));
    }
    let hdr = le_u16(raw, 8)?;
    let mut out = Vec::new();
    let mut idx = 0usize;
    let mut count = le_u32(raw, 14)?;
    let mut pos = 44usize;
    let mut len = le_u32(raw, 28)? as isize - 8;
    let mut key = None;
    // the gzip members of every chain first (their places follow from the lengths alone), then
    // inflated on all cores: a large project is one partition of some thousand chains
    let mut members: Vec<(usize, usize, usize, usize)> = Vec::new();
    loop {
        while len > 0 {
            members.push((idx, pos, len as usize, inflated_len(raw, pos, len as usize)?));
            let h = pos + len as usize;
            len = le_u32(raw, h + 16)? as isize - 8;
            pos = h + 32;
        }
        out.push(Chain { index: idx, key, count, data: Vec::new(), bits: None, version: None, elements: vec![], hdr });
        // marker = u16 (t+1) | ff×4 | u16 t (step 254); 220 bytes, or 78 in 2016/2017 (step 1481)
        let mut next = None;
        for gap in [220usize, 78] {
            let ch = (pos + gap).wrapping_sub(32 + 54);
            if ch + 54 <= raw.len() && raw[ch + 2..ch + 6] == [0xff; 4] && le_u16(raw, ch)? == le_u16(raw, ch + 6)?.wrapping_add(1) {
                next = Some(ch);
                break;
            }
        }
        let Some(ch) = next else { break };
        idx += 1;
        count = le_u32(raw, ch + 8)?;
        key = Some(raw[ch + 12..ch + 28].try_into().expect("16 bytes"));
        len = le_u32(raw, ch + 38)? as isize - 8;
        pos = ch + 54;
    }
    // each chain at its full length, each member inflated into its own part of it
    let mut lengths = vec![0usize; out.len()];
    for &(chain, .., n) in &members {
        lengths[chain] += n;
    }
    for (ch, n) in out.iter_mut().zip(lengths) {
        ch.data = vec![0; n];
    }
    let mut parts = Vec::with_capacity(members.len());
    let mut of = members.iter().peekable();
    for ch in out.iter_mut() {
        let mut rest = ch.data.as_mut_slice();
        while let Some(&(_, pos, len, n)) = of.next_if(|m| m.0 == ch.index) {
            let (part, after) = rest.split_at_mut(n);
            parts.push(std::sync::Mutex::new((pos, len, part)));
            rest = after;
        }
    }
    for done in crate::par::map(&parts, |part| {
        let mut part = part.lock().expect("one thread per member");
        let (pos, len) = (part.0, part.1);
        inflate(raw, pos, len, part.2)
    }) {
        done?;
    }
    Ok(out)
}

/// §3.4 presence rule (64-bit), shifted by s = 22·w.
fn bbox_refs(b: &[u8], o: usize, ln: usize, s: usize) -> Result<(Option<[f64; 6]>, Option<Vec<u64>>)> {
    if ln < 140 + s {
        return Ok((None, None));
    }
    let mut bb = [0f64; 6];
    for (i, v) in bb.iter_mut().enumerate() {
        *v = le_f64(b, o + 88 + s + 8 * i)?;
    }
    let n = le_u32(b, o + 136 + s)? as usize;
    if !(bb.iter().all(|v| v.is_finite()) && (0..3).all(|i| bb[i] <= bb[i + 3])) || 140 + s + 8 * n > ln {
        return Ok((None, None));
    }
    let refs: Vec<u64> = (0..n).map(|i| le_u64(b, o + 140 + s + 8 * i)).collect::<Result<_>>()?;
    if refs.windows(2).any(|w| w[0] >= w[1]) {
        return Ok((None, None));
    }
    Ok((Some(bb), Some(refs)))
}

/// §3 / §3.6 section A and §4 / §4.3 section B; fills `ch.elements`, `ch.bits` and `ch.version`.
pub fn section_a_b(ch: &mut Chain) -> Result<()> {
    let b = &ch.data;
    let mut elements = Vec::new();
    let (p0, idlen, ylen, step);
    if let Some(v) = version32(le_u16(b, 8)?) {
        // 32-bit (2008, 2014–2023)
        ch.bits = Some(32);
        ch.version = Some(if v == 2016 && ch.hdr == 0x02DE { 2015 } else { v }); // step 1510
        let mut o = 0usize;
        while b.get(o..o + 4).ok_or_else(|| Error::Format("section A overrun".into()))? != [0xff; 4] {
            let eid = le_u32(b, o)? as u64;
            let x = le_u32(b, o + 4)? as usize;
            let mut bb32 = None;
            if x + 12 >= 108 {
                // step 1330: 32-bit section A bbox = 6 × f64 @60
                let mut v = [0f64; 6];
                for (i, t) in v.iter_mut().enumerate() {
                    *t = le_f64(b, o + 60 + 8 * i)?;
                }
                if v.iter().all(|t| t.is_finite()) && (0..3).all(|i| v[i] <= v[i + 3]) && v.iter().all(|t| t.abs() < 1e9) {
                    bb32 = Some(v);
                }
            }
            elements.push(Element {
                id: eid,
                cat: le_i32(b, o + 14)?,
                tag: if x + 12 >= 48 { Some(le_u16(b, o + 46)?) } else { None },
                bbox: bb32,
                refs: None,
                view: if x + 12 >= 26 { Some(le_i32(b, o + 22)? as i64) } else { None },
                owner32: if x + 12 >= 22 { Some(le_i32(b, o + 18)?) } else { None },
                body: (0, 0),
            });
            o += x + 12;
        }
        (p0, idlen, ylen, step) = (o + 12, 4usize, 8usize, 16usize);
    } else {
        // 64-bit (2024–2026)
        ch.bits = Some(64);
        ch.version = version64(le_u16(b, 12)?);
        let mut o = 0usize;
        while b.get(o..o + 8).ok_or_else(|| Error::Format("section A overrun".into()))? != [0xff; 8] {
            let eid = le_u64(b, o)?;
            let x = le_u32(b, o + 8)? as usize;
            let w = le_u16(b, o + 14)? as usize;
            let s = 22 * w;
            let ln = x + 16;
            let (bb, refs) = bbox_refs(b, o, ln, s)?;
            elements.push(Element {
                id: eid,
                cat: le_i32(b, o + 18 + s)?,
                tag: if ln >= 76 + s { Some(le_u16(b, o + 74 + s)?) } else { None },
                bbox: bb,
                refs,
                view: if ln >= 42 + s { Some(le_i64(b, o + 34 + s)?) } else { None }, // step 673
                owner32: None,
                body: (0, 0),
            });
            o += ln;
        }
        (p0, idlen, ylen, step) = (o + 16, 8usize, 12usize, 20usize);
    }
    let mut p = p0;
    for e in elements.iter_mut() {
        // §4.1: id check
        let id = if idlen == 8 { le_u64(b, p)? } else { le_u32(b, p)? as u64 };
        if id != e.id {
            return Err(Error::Format(format!("section B: id {} not at {p}", e.id)));
        }
        let y = le_u32(b, p + ylen)? as usize;
        e.body = (p, y + step);
        p += y + step;
    }
    ch.elements = elements;
    Ok(())
}
