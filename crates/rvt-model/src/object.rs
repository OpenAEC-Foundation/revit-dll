//! Generic object view (branch open-research, par C): any element as its schema class with named,
//! typed fields, decoded from its object-graph record (channel 102) with the file's own class
//! schema (`Formats/Latest`). Every element is reachable this way, also those without a typed
//! reader; typed readers remain for geometry and interpretation.
//!
//! JSON shape (field names are stable):
//! ```json
//! {"id": 1, "class": "SWall", "parents": ["VWall", "HostObj", "Element"], "category": -2000011,
//!  "complete": true,
//!  "fields": [{"name": "m_id", "type": "ElementId", "owner": "Element", "value": {"element": 1}}],
//!  "objects": [{"class": "Plane", "fields": [...]}]}
//! ```
//! A pointer field holds `{"ref": n}`, the index of its target in `objects`; a null pointer is
//! `null`. `ElementId` values are collapsed to `{"element": id}`.

use serde::{Deserialize, Serialize};

/// One element as an object of its schema class.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObjectView {
    pub id: u64,
    /// The schema class of the record's root object (e.g. `SWall`, `Level`, `MaterialElem`).
    pub class: String,
    /// Ancestor classes, nearest first (e.g. `["VWall", "HostObj", "Element"]`).
    pub parents: Vec<String>,
    /// Category id from the element header (channel 101) or the element table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<i64>,
    /// The whole record body was consumed without error.
    pub complete: bool,
    /// Why decoding stopped (then `fields` holds the root object only, when that decoded, and
    /// `objects` is empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Fields of the root object along its class chain, base class first.
    pub fields: Vec<FieldView>,
    /// Pointer targets of the record, in decoding order; `{"ref": n}` names `objects[n]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<NestedObject>,
    /// The element header record (channel 101, class `ElementHeader`), when decodable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<NestedObject>,
}

/// An object without an element id of its own (an inline field value or a pointer target).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NestedObject {
    pub class: String,
    pub fields: Vec<FieldView>,
}

/// One field: its schema name, a type description and its value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldView {
    pub name: String,
    /// `bool`, `u8`, `i16`, `i32`, `u32`, `f32`, `f64`, `string`, `guid`, `class`, `i64`, an
    /// inline object's class name, `ptr` or `ext_ptr`; `T[n]` for a fixed array, `vec<T>` for a
    /// vector.
    #[serde(rename = "type")]
    pub ty: String,
    /// The class that declares the field.
    pub owner: String,
    pub value: FieldValue,
}

/// A field value. Serialised untagged: plain JSON for scalars, small objects for references.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FieldValue {
    /// A null pointer.
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// A string; a GUID as `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    Str(String),
    /// An `ElementId`: a reference to another element (−1 = none).
    Element { element: i64 },
    /// A pointer to `ObjectView::objects[ref]`.
    Ref {
        #[serde(rename = "ref")]
        target: usize,
    },
    /// A schema class reference (descriptor base 10), by name.
    ClassRef { class_ref: String },
    List(Vec<FieldValue>),
    Object(NestedObject),
}

impl FieldValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            FieldValue::Float(v) => Some(*v),
            FieldValue::Int(v) => Some(*v as f64),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            FieldValue::Int(v) => Some(*v),
            FieldValue::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FieldValue::Str(s) => Some(s),
            _ => None,
        }
    }
    /// The id of an `ElementId` value.
    pub fn element(&self) -> Option<i64> {
        match self {
            FieldValue::Element { element } => Some(*element),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&NestedObject> {
        match self {
            FieldValue::Object(o) => Some(o),
            _ => None,
        }
    }
    pub fn as_ref_index(&self) -> Option<usize> {
        match self {
            FieldValue::Ref { target } => Some(*target),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[FieldValue]> {
        match self {
            FieldValue::List(l) => Some(l),
            _ => None,
        }
    }
}

fn find<'a>(fields: &'a [FieldView], name: &str) -> Option<&'a FieldValue> {
    fields.iter().find(|f| f.name == name).map(|f| &f.value)
}

impl NestedObject {
    pub fn get(&self, name: &str) -> Option<&FieldValue> {
        find(&self.fields, name)
    }
}

impl ObjectView {
    /// A field of the root object by name.
    pub fn get(&self, name: &str) -> Option<&FieldValue> {
        find(&self.fields, name)
    }
    /// Whether the object's class is `name` or derives from it.
    pub fn is_a(&self, name: &str) -> bool {
        self.class == name || self.parents.iter().any(|p| p == name)
    }
    /// The target object of a pointer value.
    pub fn deref(&self, v: &FieldValue) -> Option<&NestedObject> {
        self.objects.get(v.as_ref_index()?)
    }
    /// Follows a path of field names from the root through inline objects and pointers
    /// (e.g. `["m_pSurface", "m_origin"]`).
    pub fn path(&self, names: &[&str]) -> Option<&FieldValue> {
        let (first, rest) = names.split_first()?;
        let mut v = self.get(first)?;
        for n in rest {
            let o = match v {
                FieldValue::Object(o) => o,
                FieldValue::Ref { target } => self.objects.get(*target)?,
                _ => return None,
            };
            v = o.get(n)?;
        }
        Some(v)
    }
}

/// Decoding coverage of one file's partition records (par C).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Coverage {
    pub file: String,
    pub version: Option<u16>,
    pub bits: Option<u8>,
    pub channels: Vec<ChannelCoverage>,
}

/// Coverage of one channel (101 element headers, 102 object graphs, 103 saved graphics).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ChannelCoverage {
    pub channel: u64,
    pub records: usize,
    /// Records with an empty body (the special record of id −1), not counted below.
    pub empty: usize,
    /// Body consumed exactly, no error.
    pub complete: usize,
    /// Root classes with the most failing records: (class, failures, records of that class,
    /// most frequent reason, an example id).
    pub failing: Vec<FailingClass>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FailingClass {
    pub class: String,
    pub failed: usize,
    pub records: usize,
    pub reason: String,
    pub example: u64,
}
