//! The class schema a file carries in `Formats/Latest` (branch open-research, step 2300).
//!
//! Layout (byte observation on Revit 2014, 2019, 2023, 2025 and 2026; the functional description in
//! rvt-rs, Apache-2.0, `src/schema_registry.rs`, was the guide): after the page checksums are
//! stripped the stream is one gzip member holding a list of class definitions, ended by 8 zero bytes.
//! A definition is `u16 0`, `u16 n`, `n` bytes UTF-8 name, a class reference (parent), `u32` (a
//! version-like word), `u32` field count, the fields, `u32 m` and `m` × 16 bytes (GUID-like entries).
//! A field is `u32 n`, `n` bytes name, a `u32` descriptor: byte 0 the base type, byte 1 the modifier.
//! - modifier 0x10..=0x13: a fixed array, `u32` count follows;
//! - base 14 (object) with modifier low nibble 0 (inline object, or 0x50 a vector of them): a class
//!   reference follows; pointers (low nibble 1..=3) carry none;
//! - base 13 (nested): `u32 k` (1 seen), `u8` (0x20 seen), then `k` inner descriptors; an inline
//!   object inside repeats its class reference once per outer array element.
//! A class reference is a `u16`; bit 0x8000 means the referenced class is defined right here
//! (inline). A class's tag is 12 + its position in definition order, inline ones included; a
//! reference's low 15 bits are that tag.

use std::collections::HashMap;
use std::io::Read;

use crate::{Cfb, Error, Result};

/// One field of a class.
#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    /// The raw descriptor: byte 0 base type, byte 1 modifier, high 16 bits flags.
    pub desc: u32,
    /// Fixed array length (modifier 0x10..=0x13), else 0.
    pub count: u32,
    /// Class tag of an inline object (base 14, modifier low nibble 0) or of a nested one.
    pub class: Option<u16>,
    /// The inner descriptor of a nested field (base 13).
    pub inner: Option<Box<Field>>,
}

impl Field {
    pub fn base(&self) -> u8 {
        (self.desc & 0xff) as u8
    }
    pub fn modifier(&self) -> u8 {
        ((self.desc >> 8) & 0xff) as u8
    }
    pub fn flags(&self) -> u16 {
        (self.desc >> 16) as u16
    }
}

/// One class definition.
#[derive(Debug, Clone)]
pub struct Class {
    pub tag: u16,
    pub name: String,
    pub parent: Option<u16>,
    pub version: u32,
    pub fields: Vec<Field>,
}

/// All classes of a file's schema, by tag and by name.
#[derive(Debug, Clone, Default)]
pub struct Schema {
    pub classes: Vec<Class>,
    by_name: HashMap<String, u16>,
    /// per class (index tag − 12): its fields along the class chain as (class tag, field index),
    /// built on first use (record decoding asks for them per object)
    flat: std::sync::OnceLock<Vec<Vec<(u16, u16)>>>,
}

impl Schema {
    /// Reads `Formats/Latest` of an open file.
    pub fn read(cfb: &Cfb) -> Result<Self> {
        let raw = crate::chain::strip_inserted(&cfb.stream("Formats/Latest")?);
        let mut data = Vec::new();
        flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut data)?;
        Self::parse(&data)
    }

    /// Parses the inflated stream.
    pub fn parse(b: &[u8]) -> Result<Self> {
        let mut p = Parser { b, classes: Vec::new() };
        let mut o = 0usize;
        // the list ends with 8 zero bytes
        while o + 8 < b.len() {
            o = p.class(o)?;
        }
        let mut classes: Vec<Class> = p.classes.into_iter().map(|c| c.expect("every reserved slot is filled")).collect();
        classes.sort_by_key(|c| c.tag);
        let by_name = classes.iter().map(|c| (c.name.clone(), c.tag)).collect();
        Ok(Schema { classes, by_name, flat: Default::default() })
    }

    /// The class with this tag.
    pub fn class(&self, tag: u16) -> Option<&Class> {
        tag.checked_sub(12).and_then(|i| self.classes.get(i as usize)).filter(|c| c.tag == tag)
    }

    /// The tag of the class with this name.
    pub fn tag(&self, name: &str) -> Option<u16> {
        self.by_name.get(name).copied()
    }

    /// Whether class `tag` is the class named `name` or derives from it.
    pub fn is_a(&self, tag: u16, name: &str) -> bool {
        let mut t = Some(tag);
        let mut n = 0;
        while let Some(c) = t.and_then(|t| self.class(t)) {
            if c.name == name {
                return true;
            }
            t = c.parent;
            n += 1;
            if n > 64 {
                break;
            }
        }
        false
    }

    /// The fields of a class and its ancestors, base class first, as (class tag, field index);
    /// cached.
    pub fn flat_fields(&self, tag: u16) -> &[(u16, u16)] {
        let flat = self.flat.get_or_init(|| {
            self.classes
                .iter()
                .map(|c| {
                    self.all_fields(c.tag)
                        .into_iter()
                        .map(|(k, f)| (k.tag, k.fields.iter().position(|g| std::ptr::eq(g, f)).unwrap_or(0) as u16))
                        .collect()
                })
                .collect()
        });
        tag.checked_sub(12).and_then(|i| flat.get(i as usize)).map_or(&[], |v| v.as_slice())
    }

    /// The fields of a class and its ancestors, base class first.
    pub fn all_fields(&self, tag: u16) -> Vec<(&Class, &Field)> {
        let mut chain = Vec::new();
        let mut t = Some(tag);
        while let Some(c) = t.and_then(|t| self.class(t)) {
            chain.push(c);
            t = c.parent;
            if chain.len() > 64 {
                break;
            }
        }
        chain.iter().rev().flat_map(|c| c.fields.iter().map(move |f| (*c, f))).collect()
    }
}

struct Parser<'a> {
    b: &'a [u8],
    classes: Vec<Option<Class>>,
}

impl Parser<'_> {
    fn u16(&self, o: usize) -> Result<u16> {
        crate::le_u16(self.b, o)
    }
    fn u32(&self, o: usize) -> Result<u32> {
        crate::le_u32(self.b, o)
    }
    fn text(&self, o: usize, n: usize) -> Result<String> {
        let s = self.b.get(o..o + n).ok_or_else(|| Error::Format(format!("schema name at {o} out of range")))?;
        String::from_utf8(s.to_vec()).map_err(|_| Error::Format(format!("schema name at {o} is not UTF-8")))
    }

    /// A class reference at `o` (an inline definition when bit 0x8000 is set); returns the tag.
    fn reference(&mut self, o: usize) -> Result<(Option<u16>, usize)> {
        let r = self.u16(o)?;
        let mut o = o + 2;
        if r & 0x8000 != 0 {
            o = self.class(o)?;
        }
        let tag = r & 0x7fff;
        Ok(((tag != 0).then_some(tag), o))
    }

    fn descriptor(&mut self, name: String, o: usize, repeat: u32) -> Result<(Field, usize)> {
        let desc = self.u32(o)?;
        let mut o = o + 4;
        let (base, modifier) = (desc & 0xff, (desc >> 8) & 0xff);
        let mut count = 0;
        if (0x10..=0x13).contains(&modifier) {
            count = self.u32(o)?;
            o += 4;
        }
        let mut class = None;
        let mut inner = None;
        if base == 14 && modifier & 0x0f == 0 {
            for _ in 0..repeat.max(1) {
                let (t, n) = self.reference(o)?;
                class = class.or(t);
                o = n;
            }
        } else if base == 13 {
            let k = self.u32(o)?;
            o += 5;
            for _ in 0..k {
                let (f, n) = self.descriptor(String::new(), o, count.max(1))?;
                inner = Some(Box::new(f));
                o = n;
            }
        }
        Ok((Field { name, desc, count, class, inner }, o))
    }

    fn class(&mut self, o: usize) -> Result<usize> {
        if self.u16(o)? != 0 {
            return Err(Error::Format(format!("schema class at {o} has no 0 header")));
        }
        let n = self.u16(o + 2)? as usize;
        let name = self.text(o + 4, n)?;
        let mut o = o + 4 + n;
        // reserve the slot first: inline definitions inside take the following tags
        let idx = self.classes.len();
        self.classes.push(None);
        let (parent, n) = self.reference(o)?;
        o = n;
        let version = self.u32(o)?;
        let nf = self.u32(o + 4)? as usize;
        o += 8;
        let mut fields = Vec::with_capacity(nf.min(1024));
        for _ in 0..nf {
            let ln = self.u32(o)? as usize;
            let fname = self.text(o + 4, ln)?;
            let (f, n) = self.descriptor(fname, o + 4 + ln, 1)?;
            fields.push(f);
            o = n;
        }
        let ne = self.u32(o)? as usize;
        o += 4 + 16 * ne;
        self.classes[idx] = Some(Class { tag: 12 + idx as u16, name, parent, version, fields });
        Ok(o)
    }
}
