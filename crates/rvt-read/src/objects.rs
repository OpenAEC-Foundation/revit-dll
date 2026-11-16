//! Generic object query layer (branch open-research, par C): every element as an object of its
//! schema class, with named, typed fields.
//!
//! An element's object-graph record (partition channel 102, step 2301) starts with the class tag
//! of the element's own class (`SWall`, `Level`, `MaterialElem`, …) and decodes completely with
//! the file's class schema (`Formats/Latest`, step 2300) and `Graph::decode` (step 2302). Its
//! element header (channel 101, class `ElementHeader`) holds the category. This module turns
//! such graphs into the serialisable [`ObjectView`] of `rvt-model`; typed readers stay
//! responsible for geometry and interpretation.

use std::collections::{BTreeMap, HashMap, HashSet};

use rvt_core::graph::{EsCatalog, EsField, EsSchema, Graph, Object, Value};
use rvt_core::records::{file_channel, stream_channel};
use rvt_core::schema::{Field, Schema};
use rvt_core::{Cfb, Result};
use rvt_model::{ChannelCoverage, FailingClass, FieldValue, FieldView, NestedObject, ObjectView};

/// The object records of one file: its schema, the channel-102 object graphs and the channel-101
/// element headers, by element id (bodies are decoded on demand).
pub struct ObjectStore {
    pub schema: Schema,
    pub records: HashMap<u64, Vec<u8>>,
    pub headers: HashMap<u64, Vec<u8>>,
    /// Extensible-storage schemas from the document graph (empty when unreadable).
    pub es: EsCatalog,
}

/// The extensible-storage schemas of a file: the `ESSchema` objects of its document graph
/// (`Global/Latest`: after the page checksums an 8-byte prefix and one gzip member, decoding as an
/// `ADocument` graph; rst 2026: 1 586 246 bytes, 4 left over).
pub fn es_catalog(cfb: &Cfb, schema: &Schema) -> EsCatalog {
    use std::io::Read;
    let mut cat = EsCatalog::default();
    let Ok(raw) = cfb.stream("Global/Latest") else { return cat };
    let raw = rvt_core::strip_inserted(&raw);
    let Some(st) = raw.windows(3).take(64).position(|w| w == [0x1f, 0x8b, 0x08]) else { return cat };
    let mut data = Vec::new();
    if flate2::read::GzDecoder::new(&raw[st..]).read_to_end(&mut data).is_err() {
        return cat;
    }
    let Ok((g, _)) = Graph::decode_prefix(schema, &data) else { return cat };
    let Some(tag) = schema.tag("ESSchema") else { return cat };
    let mut found = Vec::new();
    for o in &g.objects {
        collect(o, tag, &mut found);
    }
    for o in found {
        let text = |n: &str| match o.get(n) {
            Some(Value::Str(t)) => t.clone(),
            _ => String::new(),
        };
        let Some(Value::Guid(guid)) = o.get("m_guid").and_then(|v| v.as_obj()).and_then(|g| g.get("m_guid")) else { continue };
        let mut fields: Vec<(i64, EsField)> = o
            .get("m_fields")
            .and_then(|v| v.as_list())
            .unwrap_or(&[])
            .iter()
            .filter_map(|f| {
                let f = f.as_obj()?;
                let t = |n: &str| match f.get(n) {
                    Some(Value::Str(t)) => t.clone(),
                    _ => String::new(),
                };
                let ix = f.get("m_entryIndex")?.as_i64()?;
                Some((ix, EsField { name: t("m_fieldName"), type_name: t("m_fieldTypeName"), container: f.get("m_containerType")?.as_i64()? as i32 }))
            })
            .collect();
        fields.sort_by_key(|f| f.0);
        cat.schemas.insert(*guid, EsSchema { name: text("m_schemaName"), fields: fields.into_iter().map(|f| f.1).collect() });
    }
    cat
}

/// Objects of class `tag` in an object and its inline values.
fn collect<'a>(o: &'a Object, tag: u16, out: &mut Vec<&'a Object>) {
    if o.class == tag {
        out.push(o);
    }
    for (_, v) in &o.fields {
        collect_value(v, tag, out);
    }
}

fn collect_value<'a>(v: &'a Value, tag: u16, out: &mut Vec<&'a Object>) {
    match v {
        Value::Obj(o) => collect(o, tag, out),
        Value::List(l) => l.iter().for_each(|x| collect_value(x, tag, out)),
        _ => {}
    }
}

/// The root class tag of a record body.
pub fn root_tag(body: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes([*body.first()?, *body.get(1)?]))
}

impl ObjectStore {
    pub fn read(cfb: &Cfb) -> Result<Self> {
        let schema = Schema::read(cfb)?;
        // the three beside each other (the channels inflate on all cores)
        let (headers, (es, records)) =
            rvt_core::par::both(|| file_channel(cfb, &schema, 101), || rvt_core::par::both(|| es_catalog(cfb, &schema), || file_channel(cfb, &schema, 102)));
        let records = records?.into_iter().map(|(k, r)| (k, r.body)).collect();
        let headers = headers?.into_iter().map(|(k, r)| (k, r.body)).collect();
        Ok(ObjectStore { schema, records, headers, es })
    }

    /// The object records of one family document: content block `chain + 1` of partition stream
    /// `stream` (the chain `Document::family_chains` names for a family), read with this file's
    /// schema. Element ids are the family document's own.
    pub fn family(&self, cfb: &Cfb, stream: &str, chain: usize) -> Result<Self> {
        let records = stream_channel(cfb, &self.schema, stream, chain + 1, 102)?.into_iter().map(|(k, r)| (k, r.body)).collect();
        let headers = stream_channel(cfb, &self.schema, stream, chain + 1, 101)?.into_iter().map(|(k, r)| (k, r.body)).collect();
        Ok(ObjectStore { schema: self.schema.clone(), records, headers, es: self.es.clone() })
    }

    /// Decodes a record body with this file's schema and ES schemas.
    pub fn decode(&self, body: &[u8]) -> Result<Graph> {
        Graph::decode_es(&self.schema, body, &self.es)
    }

    /// Class name of an element's object record.
    pub fn class_of(&self, id: u64) -> Option<&str> {
        let tag = root_tag(self.records.get(&id)?)?;
        self.schema.class(tag).map(|c| c.name.as_str())
    }

    /// Whether an element's object record has class `name` or one deriving from it.
    pub fn is_a(&self, id: u64, name: &str) -> bool {
        self.records.get(&id).and_then(|b| root_tag(b)).is_some_and(|t| self.schema.is_a(t, name))
    }

    /// Root class → number of object records.
    pub fn classes(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for b in self.records.values() {
            if let Some(c) = root_tag(b).and_then(|t| self.schema.class(t)) {
                *out.entry(c.name.clone()).or_insert(0) += 1;
            }
        }
        out
    }

    /// Ids (ascending) of the elements whose class is `name` or derives from it.
    pub fn ids_of_class(&self, name: &str) -> Vec<u64> {
        let tags: HashSet<u16> = self.schema.classes.iter().filter(|c| self.schema.is_a(c.tag, name)).map(|c| c.tag).collect();
        let mut ids: Vec<u64> = self.records.iter().filter(|(_, b)| root_tag(b).is_some_and(|t| tags.contains(&t))).map(|(&k, _)| k).collect();
        ids.sort_unstable();
        ids
    }

    /// The ancestor class names of a class, nearest first.
    pub fn parents(&self, tag: u16) -> Vec<String> {
        let mut out = Vec::new();
        let mut t = self.schema.class(tag).and_then(|c| c.parent);
        while let Some(c) = t.and_then(|t| self.schema.class(t)) {
            out.push(c.name.clone());
            t = c.parent;
            if out.len() > 64 {
                break;
            }
        }
        out
    }

    /// The element header (channel 101) as an object, pointer targets inlined.
    pub fn header(&self, id: u64) -> Option<NestedObject> {
        let g = self.decode(self.headers.get(&id)?).ok()?;
        let conv = Conv { s: &self.schema, n: g.objects.len(), graph: Some(&g) };
        Some(conv.object(&g.objects[0], &mut Vec::new()))
    }

    /// One element as an object view; none when it has no object record. `category` is the
    /// fallback when the header has none.
    pub fn view(&self, id: u64, category: Option<i64>) -> Option<ObjectView> {
        let body = self.records.get(&id)?;
        let s = &self.schema;
        let tag = root_tag(body)?;
        let header = self.header(id);
        let category = header.as_ref().and_then(|h| h.get("m_categroryId").or_else(|| h.get("m_categoryId"))).and_then(|v| v.element()).or(category);
        let class = s.class(tag).map_or_else(|| format!("#{tag}"), |c| c.name.clone());
        let parents = self.parents(tag);
        let (complete, error, fields, objects) = match self.decode(body) {
            Ok(g) => {
                let conv = Conv { s, n: g.objects.len(), graph: None };
                let objects = g.objects[1..].iter().map(|o| conv.object(o, &mut Vec::new())).collect();
                (true, None, conv.object(&g.objects[0], &mut Vec::new()).fields, objects)
            }
            Err(e) => {
                // the root alone, when it decodes; its pointers have no decoded targets
                let root = Graph::decode_root(s, body).ok();
                let conv = Conv { s, n: 0, graph: None };
                (false, Some(e.to_string()), root.map(|o| conv.object(&o, &mut Vec::new()).fields).unwrap_or_default(), Vec::new())
            }
        };
        Some(ObjectView { id, class, parents, category, complete, error, fields, objects, header })
    }
}

impl ObjectStore {
    /// The root object of an element alone: its fields, no pointer targets (`objects` empty,
    /// pointers as `{"ref"}` without target), no header and category. Much faster than `view`
    /// for large records of which only scalar fields and inline objects are needed (views: a
    /// `DBView` graph carries dozens of override objects). `complete` is false.
    pub fn root_view(&self, id: u64) -> Option<ObjectView> {
        let body = self.records.get(&id)?;
        let tag = root_tag(body)?;
        let root = Graph::decode_root(&self.schema, body).ok()?;
        let conv = Conv { s: &self.schema, n: usize::MAX, graph: None };
        Some(ObjectView {
            id,
            class: self.schema.class(tag).map_or_else(|| format!("#{tag}"), |c| c.name.clone()),
            parents: self.parents(tag),
            category: None,
            complete: false,
            error: None,
            fields: conv.object(&root, &mut Vec::new()).fields,
            objects: Vec::new(),
            header: None,
        })
    }
}

/// Graph values to `FieldValue`s.
struct Conv<'a> {
    s: &'a Schema,
    /// number of decoded objects of the graph (pointers beyond it become null)
    n: usize,
    /// inline pointer targets from this graph instead of `{"ref"}` (headers)
    graph: Option<&'a Graph>,
}

impl Conv<'_> {
    fn class_name(&self, tag: u16) -> String {
        self.s.class(tag).map_or_else(|| format!("#{tag}"), |c| c.name.clone())
    }

    /// The fields of an object with their declarations, in the decoder's order (transient fields
    /// absent, a name inherited twice kept once).
    fn object(&self, o: &Object, path: &mut Vec<usize>) -> NestedObject {
        let mut decl: Vec<(u16, &Field)> = Vec::new();
        for &(ct, fi) in self.s.flat_fields(o.class) {
            let Some(f) = self.s.class(ct).and_then(|c| c.fields.get(fi as usize)) else { continue };
            if f.flags() & 2 != 0 || decl.iter().any(|(_, g)| g.name == f.name) {
                continue;
            }
            decl.push((ct, f));
        }
        let fields = o
            .fields
            .iter()
            .map(|(name, v)| {
                let d = decl.iter().find(|(_, f)| f.name == *name);
                FieldView {
                    name: name.clone(),
                    ty: d.map_or_else(|| "?".into(), |(_, f)| type_name(self.s, f)),
                    owner: d.map_or_else(String::new, |(ct, _)| self.class_name(*ct)),
                    value: self.value(v, d.is_some_and(|(_, f)| is_class_ref(f)), path),
                }
            })
            .collect();
        NestedObject { class: self.class_name(o.class), fields }
    }

    fn value(&self, v: &Value, class_ref: bool, path: &mut Vec<usize>) -> FieldValue {
        match v {
            Value::Bool(b) => FieldValue::Bool(*b),
            Value::Int(i) if class_ref => FieldValue::ClassRef { class_ref: self.class_name(*i as u16) },
            Value::Int(i) => FieldValue::Int(*i),
            Value::Float(f) => FieldValue::Float(*f),
            Value::Str(t) => FieldValue::Str(t.clone()),
            Value::Guid(g) => FieldValue::Str(guid(g)),
            Value::Obj(o) => match v.element_id() {
                Some(id) if self.s.class(o.class).is_some_and(|c| c.name == "ElementId") => FieldValue::Element { element: id },
                _ => FieldValue::Object(self.object(o, path)),
            },
            Value::Ptr(Some(i)) if *i >= 1 && *i < self.n => match self.graph {
                // inline (header graphs are small); a cycle stays a reference
                Some(g) if !path.contains(i) => {
                    path.push(*i);
                    let o = self.object(&g.objects[*i], path);
                    path.pop();
                    FieldValue::Object(o)
                }
                _ => FieldValue::Ref { target: i - 1 },
            },
            Value::Ptr(_) | Value::Ext(_) => FieldValue::Null,
            Value::List(l) => FieldValue::List(l.iter().map(|x| self.value(x, class_ref, path)).collect()),
        }
    }
}

fn is_class_ref(f: &Field) -> bool {
    f.base() == 10 || (f.base() == 13 && f.inner.as_deref().is_some_and(is_class_ref))
}

/// A GUID in its usual text form (first three groups little-endian, as Windows stores them).
fn guid(g: &[u8; 16]) -> String {
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    let tail: String = g[10..].iter().map(|b| format!("{b:02x}")).collect();
    format!("{d1:08x}-{d2:04x}-{d3:04x}-{:02x}{:02x}-{tail}", g[8], g[9])
}

/// A field's type in the stable notation of [`FieldView::ty`].
pub fn type_name(s: &Schema, f: &Field) -> String {
    let m = f.modifier();
    let t = match f.base() {
        1 => "bool".into(),
        2 => "u8".into(),
        3 => "i16".into(),
        4 => "i32".into(),
        5 => "u32".into(),
        6 => "f32".into(),
        7 => "f64".into(),
        8 => "string".into(),
        9 => "guid".into(),
        10 => "class".into(),
        11 => "i64".into(),
        13 => f.inner.as_deref().map_or_else(|| "nested".into(), |i| type_name(s, i)),
        14 if m & 0x0f == 0 => f.class.and_then(|t| s.class(t)).map_or_else(|| "object".into(), |c| c.name.clone()),
        14 if m & 0x0f == 3 => "ext_ptr".into(),
        14 => "ptr".into(),
        b => format!("base{b}"),
    };
    if (0x10..=0x13).contains(&m) {
        format!("{t}[{}]", f.count)
    } else if (0x50..=0x53).contains(&m) {
        format!("vec<{t}>")
    } else {
        t
    }
}

/// A failure reason without offsets: the message head and the innermost `Class::field`.
pub fn reason(msg: &str) -> String {
    let head = msg.trim_start_matches("format error: ").trim_start_matches("graph: ");
    let head = head.split(" in ").next().unwrap_or(head);
    // digit runs → N
    let mut h = String::new();
    for c in head.chars() {
        if !c.is_ascii_digit() {
            h.push(c);
        } else if !h.ends_with('N') {
            h.push('N');
        }
    }
    let head = h;
    let field = msg.rsplit_once(" (").map(|(_, f)| f.trim_end_matches(')')).filter(|f| f.contains("::"));
    match field {
        Some(f) => format!("{} [{f}]", head.trim()),
        None => head.trim().to_string(),
    }
}

/// Decoding coverage of the records of one channel: complete count and the ten root classes
/// with the most failures.
pub fn channel_coverage(schema: &Schema, es: &EsCatalog, channel: u64, bodies: &HashMap<u64, Vec<u8>>) -> ChannelCoverage {
    let mut complete = 0;
    let mut empty = 0;
    // class → (failed, records, reason → count, example)
    let mut per: HashMap<String, (usize, usize, HashMap<String, usize>, u64)> = HashMap::new();
    for (&id, b) in bodies {
        if b.is_empty() {
            empty += 1;
            continue;
        }
        let class = root_tag(b).and_then(|t| schema.class(t)).map_or_else(|| "?".to_string(), |c| c.name.clone());
        let e = per.entry(class).or_insert_with(|| (0, 0, HashMap::new(), id));
        e.1 += 1;
        match Graph::decode_es(schema, b, es) {
            Ok(_) => complete += 1,
            Err(err) => {
                if e.0 == 0 || id < e.3 {
                    e.3 = id;
                }
                e.0 += 1;
                *e.2.entry(reason(&err.to_string())).or_insert(0) += 1;
            }
        }
    }
    let mut failing: Vec<FailingClass> = per
        .into_iter()
        .filter(|(_, v)| v.0 > 0)
        .map(|(class, (failed, records, reasons, example))| {
            let reason = reasons.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|r| r.0).unwrap_or_default();
            FailingClass { class, failed, records, reason, example }
        })
        .collect();
    failing.sort_by(|a, b| b.failed.cmp(&a.failed).then(a.class.cmp(&b.class)));
    failing.truncate(10);
    ChannelCoverage { channel, records: bodies.len(), empty, complete, failing }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_strips_offsets() {
        let m = "format error: graph: read of 4 at 120 past end 300 in SWall@0 > Foo@12 (Foo::m_bar)";
        assert_eq!(reason(m), "read of N at N past end N [Foo::m_bar]");
        assert_eq!(reason("format error: graph: 10 of 12 bytes consumed"), "N of N bytes consumed");
    }

    /// A store with one element 42 of class Thing (derives from Element).
    fn store() -> ObjectStore {
        let schema = Schema::parse(&rvt_core::schema::test_schema_bytes(&[
            ("Identifier", None, &[("m_id64", 0xb, None)]),
            ("ElementId", None, &[("m_id", 0xe, Some(12))]),
            ("Element", None, &[("m_id", 0xe, Some(13))]),
            ("Thing", Some(14), &[("m_name", 0x6008, None), ("m_p", 0x10e, None), ("m_tag", 0xa, None), ("m_ids", 0x500e, Some(13))]),
            ("Leaf", None, &[("v", 0x7, None)]),
        ]))
        .unwrap();
        let mut b = 15u16.to_le_bytes().to_vec();
        b.extend_from_slice(&42i64.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&[b'a', 0, b'b', 0]);
        b.extend_from_slice(&5u32.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&(-1i64).to_le_bytes());
        b.extend_from_slice(&1.5f64.to_le_bytes());
        ObjectStore { schema, records: HashMap::from([(42, b), (43, vec![15, 0, 1])]), headers: HashMap::new(), es: EsCatalog::default() }
    }

    #[test]
    fn view_of_an_element() {
        let s = store();
        assert_eq!(s.classes(), BTreeMap::from([("Thing".to_string(), 2)]));
        assert_eq!(s.ids_of_class("Element"), vec![42, 43]);
        assert!(s.ids_of_class("Leaf").is_empty());
        let v = s.view(42, Some(-7)).unwrap();
        assert!(v.complete && v.error.is_none());
        assert_eq!((v.class.as_str(), v.parents.clone(), v.category), ("Thing", vec!["Element".to_string()], Some(-7)));
        let types: Vec<(&str, &str, &str)> = v.fields.iter().map(|f| (f.name.as_str(), f.ty.as_str(), f.owner.as_str())).collect();
        assert_eq!(types, vec![("m_id", "ElementId", "Element"), ("m_name", "string", "Thing"), ("m_p", "ptr", "Thing"), ("m_tag", "class", "Thing"), ("m_ids", "vec<ElementId>", "Thing")]);
        assert_eq!(v.get("m_id").and_then(|x| x.element()), Some(42));
        assert_eq!(v.get("m_name").and_then(|x| x.as_str()), Some("ab"));
        assert_eq!(v.get("m_tag"), Some(&FieldValue::ClassRef { class_ref: "Leaf".into() }));
        assert_eq!(v.path(&["m_p", "v"]).and_then(|x| x.as_f64()), Some(1.5));
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("\"element\":42") && json.contains("\"ref\":0") && json.contains("\"class_ref\":\"Leaf\""), "{json}");
        // a record that does not decode: error, no fields beyond what the root gives
        let bad = s.view(43, None).unwrap();
        assert!(!bad.complete && bad.error.is_some());
        assert!(s.view(44, None).is_none());
    }

    #[test]
    fn root_view_has_the_root_fields_only() {
        let s = store();
        let full = s.view(42, None).unwrap();
        let root = s.root_view(42).unwrap();
        assert_eq!(root.fields, full.fields);
        assert_eq!((root.class.as_str(), root.complete, root.objects.len()), ("Thing", false, 0));
        // the pointer is kept as a reference without a target
        assert!(root.path(&["m_p", "v"]).is_none());
        assert!(s.root_view(44).is_none());
    }

    #[test]
    fn guid_text() {
        let g = [0x78, 0x56, 0x34, 0x12, 0x34, 0x12, 0x78, 0x56, 1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(guid(&g), "12345678-1234-5678-0102-030405060708");
    }
}
