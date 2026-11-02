//! The file's own type schema, stream `Formats/Latest`.
//!
//! Every file lists its classes and their fields. Class tags in the element tables are ids in this
//! list, so tag -> class holds per file and per version without a table of our own. Layout:
//!
//! ```text
//! file  := { u16 0 | class } …  (a name length of 0 ends the list)
//! class := u16 n | name | classref parent | u32 version | u32 nf | field × nf | u32 ng | guid × ng
//! field := u32 n | name | type
//! type  := u8 code | u8 flags | u16 extra | [u32 count if flags high nibble = 1] | kind
//! classref := u16 id, and if bit 15 is set: u16 0 | class  (first use defines it inline)
//! ```
//!
//! Ids are serial from 12 in order of first appearance: a class gets its id when its name is read,
//! before an inline parent or field class.

use std::collections::HashMap;

use crate::{le_u16, le_u32, Error, Result};

const FIRST_ID: u16 = 12;
const MAX_INLINE_DEPTH: usize = 1024;

pub const CODE_BOOL: u8 = 1;
pub const CODE_U8: u8 = 2;
pub const CODE_I16: u8 = 3;
pub const CODE_I32: u8 = 4;
pub const CODE_U32: u8 = 5;
pub const CODE_F32: u8 = 6;
pub const CODE_F64: u8 = 7;
pub const CODE_STR: u8 = 8;
pub const CODE_GUID: u8 = 9;
pub const CODE_I64: u8 = 11;
const CODE_NESTED: u8 = 13;
const CODE_CLASS: u8 = 14;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Prim(u8),
    /// A class stored by value.
    Class(u16),
    /// Pointer; the low nibble of the flags. 3 is a back reference (u32 only).
    Ptr(u8),
    /// Group of inner types, e.g. `(f64[3])[]`.
    Nested(Vec<Type>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    One,
    Array(u32),
    Vector,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: Kind,
    pub shape: Shape,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct Class {
    pub id: u16,
    pub name: String,
    /// Parent class id, 0 = none.
    pub parent: u16,
    pub version: u32,
    pub fields: Vec<Field>,
    /// Ids from the root class down to this one.
    pub lineage: Vec<u16>,
}

#[derive(Debug, Default)]
pub struct Schema {
    classes: Vec<Option<Class>>,
    by_name: HashMap<String, u16>,
    pub(crate) element_id: Option<u16>,
    pub(crate) es_entity: Option<u16>,
    /// Bytes of an ElementId: 8 from Revit 2024, 4 before.
    pub(crate) id_width: Option<u8>,
}

struct Parser<'a> {
    b: &'a [u8],
    o: usize,
    next_id: u16,
    depth: usize,
    s: Schema,
}

impl Parser<'_> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self.b.get(self.o).ok_or_else(|| Error::Format("schema: past end".into()))?;
        self.o += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16> {
        let v = le_u16(self.b, self.o)?;
        self.o += 2;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32> {
        let v = le_u32(self.b, self.o)?;
        self.o += 4;
        Ok(v)
    }
    fn name(&mut self, n: usize) -> Result<String> {
        let t = self.b.get(self.o..self.o + n).ok_or_else(|| Error::Format(format!("schema: name of {n} bytes past end at {}", self.o)))?;
        if !t.iter().all(|c| (32..127).contains(c)) {
            return Err(Error::Format(format!("schema: bad name at {}", self.o)));
        }
        self.o += n;
        Ok(String::from_utf8_lossy(t).into_owned())
    }

    fn class_ref(&mut self) -> Result<u16> {
        let r = self.u16()?;
        if r & 0x8000 == 0 {
            return Ok(r);
        }
        if self.u16()? != 0 {
            return Err(Error::Format(format!("schema: inline class prefix at {}", self.o)));
        }
        let n = self.u16()? as usize;
        self.class_def(n, Some(r & 0x7fff))?;
        Ok(r & 0x7fff)
    }

    fn type_(&mut self) -> Result<Type> {
        let code = self.u8()?;
        let flags = self.u8()?;
        self.u16()?;
        let shape = match flags & 0xf0 {
            0x10 => Shape::Array(self.u32()?),
            0x50 => Shape::Vector,
            _ => Shape::One,
        };
        let kind = match code {
            CODE_CLASS if flags & 0x0f == 0 => Kind::Class(self.class_ref()?),
            CODE_CLASS => Kind::Ptr(flags & 0x0f),
            CODE_NESTED => {
                let n = self.u32()?;
                let mut inner = Vec::new();
                for _ in 0..n {
                    self.u8()?;
                    let t = self.type_()?;
                    if matches!(t.kind, Kind::Class(_)) {
                        self.class_ref()?; // written a second time after a by-value class
                    }
                    inner.push(t);
                }
                Kind::Nested(inner)
            }
            c => Kind::Prim(c),
        };
        Ok(Type { kind, shape })
    }

    fn class_def(&mut self, name_len: usize, id: Option<u16>) -> Result<()> {
        if !(1..400).contains(&name_len) {
            return Err(Error::Format(format!("schema: class name length {name_len} at {}", self.o)));
        }
        self.depth += 1;
        if self.depth > MAX_INLINE_DEPTH {
            return Err(Error::Format("schema: inline classes nested too deep".into()));
        }
        let name = self.name(name_len)?;
        let id = id.unwrap_or(self.next_id);
        self.next_id = self.next_id.max(id) + 1;
        let parent = self.class_ref()?;
        let version = self.u32()?;
        let nf = self.u32()?;
        let mut fields = Vec::new();
        for _ in 0..nf {
            let n = self.u32()? as usize;
            if !(1..400).contains(&n) {
                return Err(Error::Format(format!("schema: field name length {n} at {} in {name}", self.o)));
            }
            let fname = self.name(n)?;
            fields.push(Field { name: fname, ty: self.type_()? });
        }
        let guids = self.u32()? as usize;
        self.o = self.o.checked_add(guids.saturating_mul(16)).filter(|e| *e <= self.b.len()).ok_or_else(|| Error::Format(format!("schema: {guids} guids past end in {name}")))?;
        let i = id as usize;
        if self.s.classes.len() <= i {
            self.s.classes.resize_with(i + 1, || None);
        }
        self.s.by_name.entry(name.clone()).or_insert(id);
        self.s.classes[i] = Some(Class { id, name, parent, version, fields, lineage: Vec::new() });
        self.depth -= 1;
        Ok(())
    }
}

impl Schema {
    /// Parses inflated `Formats/Latest` bytes.
    pub fn parse(b: &[u8]) -> Result<Schema> {
        let mut p = Parser { b, o: 0, next_id: FIRST_ID, depth: 0, s: Schema::default() };
        while p.o + 4 <= b.len() {
            if p.u16()? != 0 {
                return Err(Error::Format(format!("schema: record prefix at {}", p.o - 2)));
            }
            let n = p.u16()? as usize;
            if n == 0 {
                break;
            }
            p.class_def(n, None)?;
        }
        let mut s = p.s;
        s.finish()?;
        Ok(s)
    }

    fn finish(&mut self) -> Result<()> {
        for i in 0..self.classes.len() {
            let Some(c) = &self.classes[i] else { continue };
            let mut lineage = vec![c.id];
            let mut parent = c.parent;
            while parent != 0 {
                if lineage.len() > self.classes.len() {
                    return Err(Error::Format(format!("schema: parent cycle at {}", c.name)));
                }
                lineage.push(parent);
                parent = self.class(parent).ok_or_else(|| Error::Format(format!("schema: unknown parent {parent}")))?.parent;
            }
            lineage.reverse();
            self.classes[i].as_mut().expect("present").lineage = lineage;
        }
        self.element_id = self.id_of("ElementId");
        self.es_entity = self.id_of("ESEntity");
        self.id_width = self.id_of("Identifier").and_then(|i| self.class(i)).and_then(|c| match c.fields.as_slice() {
            [Field { ty: Type { kind: Kind::Prim(CODE_I64), shape: Shape::One }, .. }] => Some(8),
            [Field { ty: Type { kind: Kind::Prim(CODE_I32), shape: Shape::One }, .. }] => Some(4),
            _ => None,
        });
        Ok(())
    }

    pub fn class(&self, id: u16) -> Option<&Class> {
        self.classes.get(id as usize)?.as_ref()
    }

    pub fn id_of(&self, name: &str) -> Option<u16> {
        self.by_name.get(name).copied()
    }

    pub fn classes(&self) -> impl Iterator<Item = &Class> {
        self.classes.iter().flatten()
    }

    pub fn len(&self) -> usize {
        self.classes().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
