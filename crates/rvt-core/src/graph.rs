//! Schema-driven decoding of a record body into an object graph (branch open-research, step 2302).
//!
//! Functional description: rvt-rs (Apache-2.0), `src/native_parameters.rs` (`Cursor`); checked by
//! byte observation (a body must be consumed exactly). A body starts with the root object's `u16`
//! class tag. An object's fields are decoded along its class chain, base class first, in schema
//! order; fields whose flags have bit 2 set are transient and absent. Field types by descriptor
//! base: 1 bool (1 B), 2 u8, 3 i16, 4 i32, 5 u32, 6 f32, 7 f64, 8 string (u32 count + UTF-16), 9
//! GUID (16 B), 10 a `u16` class tag, 11 i64, 13 nested (the inner descriptor per element), 14 object: modifier low
//! nibble 0 inline (the field's class), 1..=3 a pointer: `u32` token, then (non-zero token, not
//! external = 3) the target's `u16` class tag; targets are decoded after the current object, in
//! order of appearance, once per token (0xffffffff never shared). Modifier 0x10..=0x13: fixed array of the field's count,
//! 0x50..=0x53: vector with a `u32` count.

use std::collections::HashMap;

use crate::schema::{Field, Schema};
use crate::{Error, Result};

/// A decoded value.
#[derive(Debug, Clone)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Guid([u8; 16]),
    /// An inline object.
    Obj(Box<Object>),
    /// A pointer: the index of the target in `Graph::objects`, none when null or unresolved.
    /// External pointers (modifier 3) carry only a token; it is resolved after decoding to the
    /// object that a non-external pointer with that token introduced (step 2303: Edge::m_pFace,
    /// Edge::m_next, EdgeLoop::m_next).
    Ptr(Option<usize>),
    /// An external pointer before resolution (token).
    Ext(u32),
    List(Vec<Value>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(v) => Some(*v),
            Value::Int(v) => Some(*v as f64),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }
    pub fn as_obj(&self) -> Option<&Object> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }
    pub fn as_ptr(&self) -> Option<usize> {
        match self {
            Value::Ptr(p) => *p,
            _ => None,
        }
    }

    fn resolve(&mut self, tokens: &HashMap<u32, usize>) {
        match self {
            Value::Ext(t) => *self = Value::Ptr(tokens.get(t).copied()),
            Value::List(l) => l.iter_mut().for_each(|v| v.resolve(tokens)),
            Value::Obj(o) => o.fields.iter_mut().for_each(|(_, v)| v.resolve(tokens)),
            _ => {}
        }
    }
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(v) => Some(v),
            _ => None,
        }
    }
    /// The id of an `ElementId` value: `m_id` → `Identifier`, whose `m_id64` (Revit 2024+) or
    /// `m_id` (32-bit files, step 2307) holds it.
    pub fn element_id(&self) -> Option<i64> {
        let ident = self.as_obj()?.get("m_id")?.as_obj()?;
        ident.get("m_id64").or_else(|| ident.get("m_id"))?.as_i64()
    }
    /// A list of numbers (a fixed f64 array, a point).
    pub fn floats(&self) -> Option<Vec<f64>> {
        self.as_list()?.iter().map(|v| v.as_f64()).collect()
    }
}

/// A decoded object: its class tag and fields (by name; a name inherited twice keeps the first).
#[derive(Debug, Clone)]
pub struct Object {
    pub class: u16,
    pub fields: Vec<(String, Value)>,
}

impl Object {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

/// All objects of one record: the root first, pointer targets after it.
#[derive(Debug, Clone)]
pub struct Graph {
    pub objects: Vec<Object>,
}

impl Graph {
    /// Decodes a record body; the whole body must be consumed.
    pub fn decode(schema: &Schema, body: &[u8]) -> Result<Graph> {
        let mut d = Decoder { s: schema, b: body, p: 0, queue: Vec::new(), tokens: HashMap::new(), trail: Vec::new() };
        let root = d.u16()?;
        let mut objects = vec![d.object(root)?];
        let mut i = 0;
        while i < d.queue.len() {
            let (tag, _) = d.queue[i];
            objects.push(d.object(tag)?);
            i += 1;
        }
        if d.p != body.len() {
            return Err(Error::Format(format!("graph: {} of {} bytes consumed", d.p, body.len())));
        }
        // external pointers: the object whose token they name (the root has no token)
        let tokens = d.tokens.clone();
        for o in objects.iter_mut() {
            o.fields.iter_mut().for_each(|(_, v)| v.resolve(&tokens));
        }
        Ok(Graph { objects })
    }

    /// Only the root object of a record body (its pointer targets are not decoded; the rest of
    /// the body is not checked). Cheap: for header fields such as the phases (step 2308).
    pub fn decode_root(schema: &Schema, body: &[u8]) -> Result<Object> {
        let mut d = Decoder { s: schema, b: body, p: 0, queue: Vec::new(), tokens: HashMap::new(), trail: Vec::new() };
        let root = d.u16()?;
        d.object(root)
    }

    pub fn class_name<'a>(&self, schema: &'a Schema, i: usize) -> Option<&'a str> {
        self.objects.get(i).and_then(|o| schema.class(o.class)).map(|c| c.name.as_str())
    }
}

/// RVT_GRAPH_TRACE=1: objects, pointers and GInfo flags on stderr (read once: decoding is hot).
fn trace() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| std::env::var_os("RVT_GRAPH_TRACE").is_some())
}

struct Decoder<'a> {
    s: &'a Schema,
    b: &'a [u8],
    p: usize,
    /// pointer targets to decode, in order: (class tag, token)
    queue: Vec<(u16, u32)>,
    /// token → index in `Graph::objects` (root = 0, queue[i] = i + 1)
    tokens: HashMap<u32, usize>,
    /// (class tag, field index, offset) of the values being read (error messages)
    trail: Vec<(u16, u16, usize)>,
}

impl Decoder<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let s = self.b.get(self.p..self.p + n).ok_or_else(|| Error::Format(format!("graph: read of {n} at {} past end {}", self.p, self.b.len())))?;
        self.p += n;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn object(&mut self, tag: u16) -> Result<Object> {
        let s = self.s;
        if s.class(tag).is_none() {
            return Err(Error::Format(format!("graph: unknown class tag {tag} at {}", self.p)));
        }
        if trace() {
            eprintln!("obj {} at {}", s.class(tag).map_or("?", |c| c.name.as_str()), self.p);
        }
        let mut fields: Vec<(String, Value)> = Vec::new();
        for &(ct, fi) in s.flat_fields(tag) {
            let Some(f) = s.class(ct).and_then(|c| c.fields.get(fi as usize)) else { continue };
            let flags = f.flags();
            if flags & 2 != 0 {
                continue; // transient
            }
            self.trail.push((tag, fi, self.p));
            let v = self.field(f).map_err(|e| match e {
                Error::Format(m) if !m.contains(" in ") => {
                    let path: Vec<String> = self
                        .trail
                        .iter()
                        .map(|&(t, _, at)| format!("{}@{at}", s.class(t).map_or("?", |c| c.name.as_str())))
                        .collect();
                    Error::Format(format!("{m} in {} ({}::{})", path.join(" > "), s.class(ct).map_or("?", |c| c.name.as_str()), f.name))
                }
                e => e,
            })?;
            self.trail.pop();
            if trace() && f.name == "m_flags" && s.class(tag).is_some_and(|c| c.name == "GInfo") {
                eprintln!("   GInfo.m_flags {:?} ends {}", v.as_i64().map(|x| format!("{x:#x}")), self.p);
            }
            if !fields.iter().any(|(n, _)| *n == f.name) {
                fields.push((f.name.clone(), v));
            }
        }
        Ok(Object { class: tag, fields })
    }

    fn field(&mut self, f: &Field) -> Result<Value> {
        let m = f.modifier();
        if (0x10..=0x13).contains(&m) {
            (0..f.count).map(|_| self.scalar(f)).collect::<Result<Vec<_>>>().map(Value::List)
        } else if (0x50..=0x53).contains(&m) {
            let n = self.u32()? as usize;
            if n > self.b.len() {
                return Err(Error::Format(format!("graph: vector of {n} at {}", self.p)));
            }
            (0..n).map(|_| self.scalar(f)).collect::<Result<Vec<_>>>().map(Value::List)
        } else {
            self.scalar(f)
        }
    }

    /// One element of field `f` (ignoring its array modifier).
    fn scalar(&mut self, f: &Field) -> Result<Value> {
        let m = f.modifier();
        Ok(match f.base() {
            1 => Value::Bool(self.take(1)?[0] != 0),
            2 => Value::Int(self.take(1)?[0] as i64),
            3 => Value::Int(i16::from_le_bytes(self.take(2)?.try_into().unwrap()) as i64),
            4 => Value::Int(i32::from_le_bytes(self.take(4)?.try_into().unwrap()) as i64),
            5 => Value::Int(self.u32()? as i64),
            6 => Value::Float(f32::from_le_bytes(self.take(4)?.try_into().unwrap()) as f64),
            7 => Value::Float(f64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            8 => {
                let n = self.u32()? as usize;
                let units: Vec<u16> = self.take(2 * n)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                Value::Str(String::from_utf16_lossy(&units))
            }
            9 => Value::Guid(self.take(16)?.try_into().unwrap()),
            // a schema class tag (ClassDefinitionRef::m_ref)
            10 => Value::Int(self.u16()? as i64),
            11 => Value::Int(i64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            13 => {
                let inner = f.inner.as_ref().ok_or_else(|| Error::Format("graph: nested field without inner descriptor".into()))?;
                self.field(inner)?
            }
            14 if m & 0x0f == 0 => {
                let tag = f.class.ok_or_else(|| Error::Format(format!("graph: inline field {} without class", f.name)))?;
                Value::Obj(Box::new(self.object(tag)?))
            }
            14 => {
                let token = self.u32()?;
                if token == 0 {
                    Value::Ptr(None)
                } else if m & 0x0f == 3 {
                    Value::Ext(token)
                } else {
                    let tag = self.u16()?;
                    if trace() {
                        eprintln!("   ptr token {token} -> {} {}", self.s.class(tag).map_or("?", |c| c.name.as_str()), if self.tokens.contains_key(&token) { "(seen)" } else { "" });
                    }
                    // token 0xffffffff is never shared: every such pointer has its own target
                    // (racbasic 2026 symbol 978910: five GConditionDir; rvt-rs does the same)
                    let idx = match self.tokens.get(&token).filter(|_| token != u32::MAX) {
                        Some(&i) => i,
                        None => {
                            self.queue.push((tag, token));
                            let i = self.queue.len();
                            self.tokens.insert(token, i);
                            i
                        }
                    };
                    Value::Ptr(Some(idx))
                }
            }
            b => return Err(Error::Format(format!("graph: field {} has unknown base {b} at {}", f.name, self.p))),
        })
    }
}
