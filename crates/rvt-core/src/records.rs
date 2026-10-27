//! Partition records per channel (branch open-research, step 2301).
//!
//! Byte observation on racbasicsampleproject_2026 `Partitions/15` (functional description: rvt-rs,
//! Apache-2.0, `src/native_segments.rs` / `src/native_document.rs`): after the page checksums are
//! stripped a partition is an 8-byte prefix and a run of markers, each a `u16` class tag from the
//! file's own schema:
//! - `SegmentMarker`: `u32` continuation bits, object count, segment size, uncompressed size; with a
//!   non-zero size a `u64` channel and one gzip member of `size − 8` bytes follow; a zero size (all
//!   four zero) closes a content block and carries nothing else;
//! - `SegmentCheckback`: `u32` segment size;
//! - `ContentMarker`: `u32` content-key token (non-zero: `u16` class tag + 16-byte key), `u32` count;
//!   a keyless one with count −1 ends the partition (other data follows);
//! - `SignatureMarker`: `u32 n` + `n` bytes, `u32 m` + `m` UTF-16 code units.
//! Continuation bit 1: the segment continues the previous one, bit 2: it continues into the next;
//! continued segments form one group. A group is a run of records: `u64` element id, a header
//! (`u32` on channel 101, `u64` on 102 and 103) whose last 4 bytes are the body length, the body,
//! the length again. Channel 101 holds the element records the rest of this crate reads, 103 the
//! element's saved graphics (root class `GElement`), 102 its object graph. Revit 2024+ (64-bit ids);
//! step 2307: files with 32-bit ids (Revit 2023, Storax NAS models) store the id as a `u32`, the
//! rest alike (storax_icarus: 1104 / 13 / 1 records in the first group of 101 / 102 / 103, every
//! trailing length matching; the first id 32899 is the first element of the file).

use std::collections::HashMap;
use std::io::Read;

use crate::{le_u16, le_u32, le_u64, Cfb, Error, Result, Schema};

/// The tags of the partition markers in a file's schema.
#[derive(Debug, Clone, Copy)]
pub struct Markers {
    pub segment: u16,
    pub checkback: u16,
    pub content: u16,
    pub signature: Option<u16>,
}

impl Markers {
    pub fn from_schema(s: &Schema) -> Option<Self> {
        Some(Markers { segment: s.tag("SegmentMarker")?, checkback: s.tag("SegmentCheckback")?, content: s.tag("ContentMarker")?, signature: s.tag("SignatureMarker") })
    }
}

/// One record: the content block it is in (1 = the document itself) and its body.
#[derive(Debug, Clone)]
pub struct Record {
    pub content: usize,
    pub body: Vec<u8>,
}

/// The records of `channel` in one stripped partition stream, by element id (a later copy of an
/// id replaces an earlier one). Only content block 1 when `main_only`.
pub fn channel_records(raw: &[u8], m: Markers, channel: u64, main_only: bool) -> Result<HashMap<u64, Record>> {
    channel_records_ids(raw, m, channel, main_only, 8)
}

/// As `channel_records`, with element ids of `id_bytes` (8: Revit 2024+, 4: 32-bit files).
pub fn channel_records_ids(raw: &[u8], m: Markers, channel: u64, main_only: bool, id_bytes: usize) -> Result<HashMap<u64, Record>> {
    let mut out = HashMap::new();
    let mut p = 8usize;
    let mut content = 0usize;
    let mut group: Vec<u8> = Vec::new();
    let mut gch = 0u64;
    while p + 2 <= raw.len() {
        let t = le_u16(raw, p)?;
        p += 2;
        if t == m.segment {
            let flags = le_u32(raw, p)?;
            let size = le_u32(raw, p + 8)? as usize;
            if size == 0 {
                p += 16;
                continue;
            }
            let ch = le_u64(raw, p + 16)?;
            p += 24;
            let member = raw.get(p..p + size - 8).ok_or_else(|| Error::Format(format!("segment at {p} past end")))?;
            p += size - 8;
            if flags & 1 == 0 {
                group.clear();
                gch = ch;
            }
            if gch == channel && (!main_only || content == 1) {
                flate2::read::GzDecoder::new(member).read_to_end(&mut group)?;
            }
            if flags & 2 == 0 && gch == channel && (!main_only || content == 1) {
                split(&group, channel, content, id_bytes, &mut out)?;
                group.clear();
            }
        } else if t == m.checkback {
            p += 4;
        } else if t == m.content {
            let tok = le_u32(raw, p)?;
            p += 4 + if tok != 0 { 18 } else { 0 };
            // a count of −1 closes the partition (rst 2026 Partitions/21: ~58 bytes of other data follow)
            if le_u32(raw, p)? == u32::MAX {
                break;
            }
            p += 4;
            content += 1;
        } else if Some(t) == m.signature {
            let nb = le_u32(raw, p)? as usize;
            p += 4 + nb;
            let ns = le_u32(raw, p)? as usize;
            p += 4 + 2 * ns;
        } else if t == 0 {
            break; // zero padding at the end of the stream
        } else {
            return Err(Error::Format(format!("unknown partition marker {t} at {}", p - 2)));
        }
    }
    Ok(out)
}

fn split(g: &[u8], channel: u64, content: usize, id_bytes: usize, out: &mut HashMap<u64, Record>) -> Result<()> {
    let hdr = if channel == 101 { 4 } else { 8 };
    let mut q = 0usize;
    while q + id_bytes + hdr <= g.len() {
        let id = if id_bytes == 4 { le_u32(g, q)? as u64 } else { le_u64(g, q)? };
        let len = le_u32(g, q + id_bytes + hdr - 4)? as usize;
        let start = q + id_bytes + hdr;
        let body = g.get(start..start + len).ok_or_else(|| Error::Format(format!("record {id} past group end")))?;
        if le_u32(g, start + len)? as usize != len {
            return Err(Error::Format(format!("record {id}: trailing length differs")));
        }
        out.insert(id, Record { content, body: body.to_vec() });
        q = start + len + 4;
    }
    Ok(())
}

/// The records of `channel` of every partition of a file, by element id. The id width (8 or 4
/// bytes) is the one whose records split exactly.
pub fn file_channel(cfb: &Cfb, schema: &Schema, channel: u64) -> Result<HashMap<u64, Record>> {
    let m = Markers::from_schema(schema).ok_or_else(|| Error::Format("schema has no partition markers".into()))?;
    let mut out = HashMap::new();
    let mut width: Option<usize> = None;
    for (path, _) in cfb.paths().into_iter().filter(|(p, _)| p.starts_with("Partitions/")) {
        let raw = crate::chain::strip_inserted(&cfb.stream(&path)?);
        let recs = match width {
            Some(w) => channel_records_ids(&raw, m, channel, true, w)?,
            None => match channel_records_ids(&raw, m, channel, true, 8) {
                Ok(r) => {
                    if !r.is_empty() {
                        width = Some(8);
                    }
                    r
                }
                Err(_) => {
                    width = Some(4);
                    channel_records_ids(&raw, m, channel, true, 4)?
                }
            },
        };
        out.extend(recs);
    }
    Ok(out)
}
