//! Decodes serialised objects (element bodies, `Global/*`) with the file's [`Schema`].
//!
//! Fields are written in schema order, root class first. An owned pointer is `u32 0` (null) or
//! `u32 marker | u16 class`, and its target's fields follow after the whole object: targets come
//! first in, first out, including those found on the way. No header precedes a target. Vectors are
//! `u32 n` + items, strings `u32 n` + n × UTF-16. An element body has a 18-byte header (14 with
//! 32-bit ids) and a 4-byte trailer that repeats its size.

use std::collections::{HashMap, VecDeque};

use crate::schema::*;
use crate::{le_f64, le_i32, le_i64, le_u16, le_u32, Chain, Element, Error, Result};

const MAX_ITEMS: u32 = 5_000_000;
const MAX_DEPTH: usize = 256;

/// Field of an extensible-storage schema; `type_name` is a primitive or a class in the [`Schema`].
#[derive(Debug, Clone)]
pub struct EsField {
    pub name: String,
    pub type_name: String,
    pub entry_index: i64,
    /// 0 single value; 1 array and 2 map are `u32 n` + items.
    pub container: i64,
}

#[derive(Debug, Clone)]
pub struct EsSchema {
    pub name: String,
    pub fields: Vec<EsField>,
}

/// The extensible-storage schemas of a document, by GUID.
#[derive(Debug, Clone, Default)]
pub struct EsSchemas(HashMap<[u8; 16], EsSchema>);

impl EsSchemas {
    pub fn new(list: Vec<([u8; 16], EsSchema)>) -> Self {
        let mut map = HashMap::new();
        for (g, mut s) in list {
            s.fields.sort_by_key(|f| f.entry_index);
            map.insert(g, s);
        }
        EsSchemas(map)
    }
    pub fn get(&self, guid: &[u8; 16]) -> Option<&EsSchema> {
        self.0.get(guid)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value<'s> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Guid([u8; 16]),
    /// An ElementId.
    Id(i64),
    List(Vec<Value<'s>>),
    /// A class stored by value.
    Struct(Box<Object<'s>>),
    /// Owned pointer: index in [`Decoded::objects`].
    Ptr(usize),
    /// Back reference to a numbered object.
    Back(u32),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Object<'s> {
    pub class: &'s str,
    pub fields: Vec<(&'s str, Value<'s>)>,
}

/// A decoded object graph; `objects[0]` is the root.
#[derive(Debug, Clone)]
pub struct Decoded<'s> {
    pub objects: Vec<Object<'s>>,
    /// Offset after the last byte read.
    pub end: usize,
}

impl<'s> Decoded<'s> {
    pub fn root(&self) -> Node<'_, 's> {
        Node { d: self, o: &self.objects[0] }
    }
    pub fn node(&self, i: usize) -> Option<Node<'_, 's>> {
        self.objects.get(i).map(|o| Node { d: self, o })
    }
    /// All objects of a class in the graph, in decoding order.
    pub fn of_class<'a>(&'a self, class: &'a str) -> impl Iterator<Item = Node<'a, 's>> + 'a {
        self.objects.iter().filter(move |o| o.class == class).map(move |o| Node { d: self, o })
    }
}

/// An object with typed access; on a repeated field name the most derived class wins.
#[derive(Clone, Copy)]
pub struct Node<'a, 's> {
    pub d: &'a Decoded<'s>,
    pub o: &'a Object<'s>,
}

impl<'a, 's> Node<'a, 's> {
    pub fn class(&self) -> &'s str {
        self.o.class
    }
    pub fn get(&self, name: &str) -> Option<&'a Value<'s>> {
        self.o.fields.iter().rev().find(|(n, _)| *n == name).map(|(_, v)| v)
    }
    pub fn str(&self, name: &str) -> Option<&'a str> {
        match self.get(name) {
            Some(Value::Str(s)) => Some(s),
            _ => None,
        }
    }
    pub fn bool(&self, name: &str) -> Option<bool> {
        match self.get(name) {
            Some(Value::Bool(v)) => Some(*v),
            _ => None,
        }
    }
    pub fn int(&self, name: &str) -> Option<i64> {
        match self.get(name) {
            Some(Value::Int(v)) => Some(*v),
            _ => None,
        }
    }
    pub fn id(&self, name: &str) -> Option<i64> {
        match self.get(name) {
            Some(Value::Id(v)) => Some(*v),
            _ => None,
        }
    }
    pub fn f64(&self, name: &str) -> Option<f64> {
        match self.get(name) {
            Some(Value::Float(v)) => Some(*v),
            _ => None,
        }
    }
    pub fn f64s(&self, name: &str) -> Option<Vec<f64>> {
        match self.get(name) {
            Some(Value::List(v)) => v.iter().map(|x| if let Value::Float(f) = x { Some(*f) } else { None }).collect(),
            _ => None,
        }
    }
    pub fn xyz(&self, name: &str) -> Option<[f64; 3]> {
        self.f64s(name)?.try_into().ok()
    }
    pub fn ids(&self, name: &str) -> Option<Vec<i64>> {
        match self.get(name) {
            Some(Value::List(v)) => v.iter().map(|x| if let Value::Id(f) = x { Some(*f) } else { None }).collect(),
            _ => None,
        }
    }
    /// Target of an owned pointer.
    pub fn ptr(&self, name: &str) -> Option<Node<'a, 's>> {
        match self.get(name) {
            Some(Value::Ptr(i)) => self.d.node(*i),
            _ => None,
        }
    }
    /// A class stored by value.
    pub fn child(&self, name: &str) -> Option<Node<'a, 's>> {
        match self.get(name) {
            Some(v) => self.of_value(v),
            _ => None,
        }
    }
    /// The object behind a by-value or pointer item, e.g. of a list.
    pub fn of_value(&self, v: &'a Value<'s>) -> Option<Node<'a, 's>> {
        match v {
            Value::Struct(o) => Some(Node { d: self.d, o }),
            Value::Ptr(i) => self.d.node(*i),
            _ => None,
        }
    }
    /// Objects of a list field (by value or pointers).
    pub fn items(&self, name: &str) -> Vec<Node<'a, 's>> {
        match self.get(name) {
            Some(Value::List(v)) => v.iter().filter_map(|x| self.of_value(x)).collect(),
            _ => Vec::new(),
        }
    }
}

enum Target {
    Class(u16),
    Es([u8; 16]),
}

struct Dec<'a, 's> {
    s: &'s Schema,
    es: Option<&'s EsSchemas>,
    b: &'a [u8],
    o: usize,
    objects: Vec<Object<'s>>,
    queue: VecDeque<(usize, Target)>,
    depth: usize,
}

impl<'s> Dec<'_, 's> {
    fn u32(&mut self) -> Result<u32> {
        let v = le_u32(self.b, self.o)?;
        self.o += 4;
        Ok(v)
    }

    fn prim(&mut self, code: u8) -> Result<Value<'s>> {
        let (b, o) = (self.b, self.o);
        let one = |o: usize| b.get(o).copied().ok_or_else(|| Error::Format(format!("byte at {o} past end")));
        Ok(match code {
            CODE_BOOL => {
                self.o += 1;
                Value::Bool(one(o)? != 0)
            }
            CODE_U8 => {
                self.o += 1;
                Value::Int(one(o)? as i64)
            }
            CODE_I16 => {
                self.o += 2;
                Value::Int(le_u16(b, o)? as i16 as i64)
            }
            CODE_I32 => {
                self.o += 4;
                Value::Int(le_i32(b, o)? as i64)
            }
            CODE_U32 => {
                self.o += 4;
                Value::Int(le_u32(b, o)? as i64)
            }
            CODE_F32 => {
                self.o += 4;
                Value::Float(f32::from_bits(le_u32(b, o)?) as f64)
            }
            CODE_F64 => {
                self.o += 8;
                Value::Float(le_f64(b, o)?)
            }
            CODE_I64 => {
                self.o += 8;
                Value::Int(le_i64(b, o)?)
            }
            CODE_STR => {
                let n = self.u32()? as usize;
                let t = n.checked_mul(2).and_then(|l| b.get(self.o..self.o.checked_add(l)?)).ok_or_else(|| Error::Format(format!("string of {n} at {o} past end")))?;
                self.o += 2 * n;
                let units: Vec<u16> = t.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                Value::Str(String::from_utf16_lossy(&units))
            }
            CODE_GUID => {
                let t = b.get(o..o + 16).ok_or_else(|| Error::Format(format!("guid at {o} past end")))?;
                self.o += 16;
                Value::Guid(t.try_into().expect("16 bytes"))
            }
            c => return Err(Error::Format(format!("field type {c} at {o} not known"))),
        })
    }

    fn pointer(&mut self, flavour: u8) -> Result<Value<'s>> {
        let m = self.u32()?;
        if flavour == 3 {
            return Ok(Value::Back(m));
        }
        if m == 0 {
            return Ok(Value::Null);
        }
        let c = le_u16(self.b, self.o)?;
        self.o += 2;
        let name = self.s.class(c).ok_or_else(|| Error::Format(format!("pointer to unknown class {c} at {}", self.o)))?.name.as_str();
        Ok(self.queued(name, Target::Class(c)))
    }

    fn queued(&mut self, class: &'s str, target: Target) -> Value<'s> {
        self.objects.push(Object { class, fields: Vec::new() });
        let i = self.objects.len() - 1;
        self.queue.push_back((i, target));
        Value::Ptr(i)
    }

    fn item(&mut self, kind: &'s Kind) -> Result<Value<'s>> {
        match kind {
            Kind::Prim(c) => self.prim(*c),
            Kind::Ptr(f) => self.pointer(*f),
            Kind::Class(c) => self.by_value(*c),
            Kind::Nested(inner) => {
                if let [one] = inner.as_slice() {
                    return self.value(one);
                }
                inner.iter().map(|t| self.value(t)).collect::<Result<Vec<_>>>().map(Value::List)
            }
        }
    }

    fn value(&mut self, t: &'s Type) -> Result<Value<'s>> {
        let n = match t.shape {
            Shape::One => return self.item(&t.kind),
            Shape::Array(n) => n,
            Shape::Vector => {
                let at = self.o;
                let n = self.u32()?;
                if n > MAX_ITEMS {
                    return Err(Error::Format(format!("vector of {n} at {at}")));
                }
                n
            }
        };
        let mut out = Vec::new();
        for _ in 0..n {
            out.push(self.item(&t.kind)?);
        }
        Ok(Value::List(out))
    }

    fn fields_of(&mut self, class: u16) -> Result<Vec<(&'s str, Value<'s>)>> {
        let schema = self.s;
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::Format("objects nested too deep".into()));
        }
        let c = schema.class(class).ok_or_else(|| Error::Format(format!("unknown class {class}")))?;
        let mut out = Vec::new();
        for &id in &c.lineage {
            let level = schema.class(id).expect("lineage is checked at parse");
            for f in &level.fields {
                out.push((f.name.as_str(), self.value(&f.ty)?));
            }
        }
        self.depth -= 1;
        Ok(out)
    }

    fn by_value(&mut self, class: u16) -> Result<Value<'s>> {
        if Some(class) == self.s.element_id {
            match self.s.id_width {
                Some(8) => {
                    let v = le_i64(self.b, self.o)?;
                    self.o += 8;
                    return Ok(Value::Id(v));
                }
                Some(_) => {
                    let v = le_i32(self.b, self.o)?;
                    self.o += 4;
                    return Ok(Value::Id(v as i64));
                }
                None => {}
            }
        }
        if Some(class) == self.s.es_entity {
            return self.entity(class);
        }
        let fields = self.fields_of(class)?;
        let name = self.s.class(class).expect("checked in fields_of").name.as_str();
        Ok(Value::Struct(Box::new(Object { class: name, fields })))
    }

    /// `ESEntity`: `u32 0`, or `u32 marker | guid` with a blob of that storage schema in the queue.
    fn entity(&mut self, class: u16) -> Result<Value<'s>> {
        let name = self.s.class(class).expect("known id").name.as_str();
        let at = self.o;
        let m = self.u32()?;
        let blob = if m == 0 {
            Value::Null
        } else {
            let g: [u8; 16] = self.b.get(self.o..self.o + 16).ok_or_else(|| Error::Format(format!("entity at {at} past end")))?.try_into().expect("16 bytes");
            self.o += 16;
            let s = self.es.and_then(|e| e.get(&g)).ok_or_else(|| Error::Format(format!("entity at {at}: storage schema not known")))?;
            self.queued(s.name.as_str(), Target::Es(g))
        };
        Ok(Value::Struct(Box::new(Object { class: name, fields: vec![("m_blob", blob)] })))
    }

    fn es_item(&mut self, type_name: &'s str) -> Result<Value<'s>> {
        match type_name {
            "bool" => self.prim(CODE_BOOL),
            "char" => self.prim(CODE_U8),
            "short" => self.prim(CODE_I16),
            "int" => self.prim(CODE_I32),
            "float" => self.prim(CODE_F32),
            "double" => self.prim(CODE_F64),
            "TCHAR" => self.prim(CODE_STR),
            name => {
                let c = self.s.id_of(name).ok_or_else(|| Error::Format(format!("storage field type {name} not known")))?;
                self.by_value(c)
            }
        }
    }

    fn es_blob(&mut self, s: &'s EsSchema) -> Result<Vec<(&'s str, Value<'s>)>> {
        let mut out = Vec::new();
        for f in &s.fields {
            let v = if f.container == 0 {
                self.es_item(&f.type_name)?
            } else {
                let n = self.u32()?;
                if n > MAX_ITEMS {
                    return Err(Error::Format(format!("storage list of {n}")));
                }
                let mut l = Vec::new();
                for _ in 0..n {
                    l.push(self.es_item(&f.type_name)?);
                }
                Value::List(l)
            };
            out.push((f.name.as_str(), v));
        }
        Ok(out)
    }
}

/// Decodes one object of `class` at `start`, with everything its pointers own.
pub fn decode<'s>(s: &'s Schema, es: Option<&'s EsSchemas>, b: &[u8], start: usize, class: u16) -> Result<Decoded<'s>> {
    let name = s.class(class).ok_or_else(|| Error::Format(format!("unknown class {class}")))?.name.as_str();
    let mut d = Dec { s, es, b, o: start, objects: vec![Object { class: name, fields: Vec::new() }], queue: VecDeque::new(), depth: 0 };
    d.objects[0].fields = d.fields_of(class)?;
    while let Some((slot, target)) = d.queue.pop_front() {
        d.depth = 0;
        let fields = match target {
            Target::Class(c) => d.fields_of(c)?,
            Target::Es(g) => {
                let schema = es.and_then(|e| e.get(&g)).expect("checked when queued");
                d.es_blob(schema)?
            }
        };
        d.objects[slot].fields = fields;
    }
    Ok(Decoded { objects: d.objects, end: d.o })
}

/// Decodes an element's body; the whole body must be consumed.
pub fn decode_element<'s>(s: &'s Schema, es: Option<&'s EsSchemas>, ch: &Chain, e: &Element) -> Result<Decoded<'s>> {
    let tag = e.tag.ok_or_else(|| Error::Format(format!("element {} has no class tag", e.id)))?;
    let start = if ch.bits == Some(32) { 14 } else { 18 };
    let body = ch.body(e);
    let d = decode(s, es, body, start, tag)?;
    if d.end + 4 != body.len() {
        return Err(Error::Format(format!("element {}: decoded {} of {} bytes", e.id, d.end, body.len() - 4)));
    }
    Ok(d)
}
