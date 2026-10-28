//! Element properties and material colours for the viewer (branch open-research, step 2311),
//! read through the generic object view (par C):
//! - material: `MaterialElem::m_pMaterial` → `Material`: `m_name`, `m_color` (Windows COLORREF,
//!   0x00BBGGRR), `m_transparency` (3077 R26 material 13690: "j7_beplating_MDF", 5534351 =
//!   RGB 143, 115, 84);
//! - level: `Element::m_assocLevelId` → `Level::m_text`;
//! - type: the referenced element whose class derives from `Symbol` (a wall's
//!   `VWall::m_WallAttributesId` → `BasicWallType`, a door's `FamilySymbol`), its name
//!   `Symbol::m_symbolInfo` → `SymbolInfo::m_name`; family: the type's `m_familyId` →
//!   `FamilyBase::m_name` (system types such as wall types have none: their class is shown).

use std::collections::HashMap;

use rvt_model::object::{FieldValue, ObjectView};
use rvt_read::Document;

/// A material as the viewer draws it.
#[derive(Debug, Clone)]
pub struct MaterialInfo {
    pub name: String,
    pub rgb: [u8; 3],
    pub transparency: f64,
}

/// Name and material lookups with caches (one per meshing thread).
pub struct Props<'a> {
    doc: &'a Document,
    materials: HashMap<i64, Option<MaterialInfo>>,
    names: HashMap<i64, Option<(String, String)>>,
}

impl<'a> Props<'a> {
    pub fn new(doc: &'a Document) -> Self {
        Props { doc, materials: HashMap::new(), names: HashMap::new() }
    }

    /// The material with this id, if it is one and has a colour.
    pub fn material(&mut self, id: i64) -> Option<&MaterialInfo> {
        if id < 0 {
            return None;
        }
        if !self.materials.contains_key(&id) {
            let m = self.doc.object(id as u64).and_then(|o| {
                let mat = o.deref(o.get("m_pMaterial")?)?;
                let color = mat.get("m_color")?.as_i64()?;
                Some(MaterialInfo {
                    name: mat.get("m_name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    rgb: [(color & 0xff) as u8, ((color >> 8) & 0xff) as u8, ((color >> 16) & 0xff) as u8],
                    transparency: mat.get("m_transparency").and_then(|v| v.as_f64()).unwrap_or(0.0),
                })
            });
            self.materials.insert(id, m);
        }
        self.materials.get(&id).and_then(|m| m.as_ref())
    }

    /// Class and display name of an element (level text, type name, family name).
    fn named(&mut self, id: i64) -> Option<(String, String)> {
        if id < 0 {
            return None;
        }
        if let Some(v) = self.names.get(&id) {
            return v.clone();
        }
        let v = self.doc.object(id as u64).map(|o| {
            let name = o
                .get("m_text")
                .or_else(|| o.get("m_name"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| o.path(&["m_symbolInfo", "m_name"]).and_then(|v| v.as_str()).map(str::to_string))
                .unwrap_or_default();
            (o.class.clone(), name)
        });
        self.names.insert(id, v.clone());
        v
    }

    /// The properties of a drawn element as a JSON object (keys: id, category, class, family,
    /// type, level, materials).
    pub fn element_json(&mut self, id: u64, category: &str, mats: &[i64]) -> String {
        let mut out: Vec<(&str, String)> = vec![("id", id.to_string()), ("category", category.to_string())];
        if let Some(o) = self.doc.object(id) {
            out.push(("class", o.class.clone()));
            if let Some(level) = o.get("m_assocLevelId").and_then(|v| v.element()).and_then(|l| self.named(l)) {
                out.push(("level", level.1));
            }
            if let Some(t) = self.type_of(&o) {
                if let Some((class, name)) = self.named(t) {
                    out.push(("type", name));
                    let family = self
                        .doc
                        .object(t as u64)
                        .and_then(|to| to.get("m_familyId").and_then(|v| v.element()))
                        .and_then(|f| self.named(f))
                        .map(|f| f.1)
                        .filter(|f| !f.is_empty());
                    out.push(("family", family.unwrap_or_else(|| system_family(&class))));
                }
            }
        }
        let mut names: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for &m in mats {
            if seen.insert(m) {
                if let Some(info) = self.material(m) {
                    if !info.name.is_empty() && !names.contains(&info.name) {
                        names.push(info.name.clone());
                    }
                }
            }
        }
        if !names.is_empty() {
            out.push(("materials", names.join(", ")));
        }
        let body: Vec<String> = out.iter().map(|(k, v)| format!("\"{k}\":{}", json_string(v))).collect();
        format!("{{{}}}", body.join(","))
    }

    /// The first referenced element (root fields, then nested objects) whose class derives from
    /// `Symbol`.
    fn type_of(&self, o: &ObjectView) -> Option<i64> {
        let mut ids: Vec<i64> = Vec::new();
        for f in &o.fields {
            collect_elements(&f.value, &mut ids);
        }
        for n in &o.objects {
            for f in &n.fields {
                collect_elements(&f.value, &mut ids);
            }
        }
        ids.into_iter().filter(|&i| i > 0 && i as u64 != o.id).find(|&i| {
            self.doc.object(i as u64).is_some_and(|t| t.is_a("Symbol"))
        })
    }
}

/// The name Revit shows for a system family, from its type class (others as the class itself).
fn system_family(class: &str) -> String {
    match class {
        "BasicWallType" => "Basic Wall",
        "CurtainWallType" | "NewCurtainWallType" => "Curtain Wall",
        "StackedWallType" => "Stacked Wall",
        "RoofAttributes" => "Basic Roof",
        "CurtainRoofAttributes" => "Sloped Glazing",
        "FloorAttributes" => "Floor",
        "CompoundCeilingType" | "CeilingAttributes" => "Compound Ceiling",
        "StairsType" => "Stair",
        "StairsRailingAttr" | "RailingType" => "Railing",
        "TopRailType" => "Top Rail",
        "HandRailType" => "Handrail",
        "BuildingPadType" => "Pad",
        "ToposolidType" => "Toposolid",
        "EdgeSlabAttr" => "Slab Edge",
        "CorniceAttr" => "Wall Sweep",
        "RampAttributes" => "Ramp",
        other => other,
    }
    .to_string()
}

fn collect_elements(v: &FieldValue, out: &mut Vec<i64>) {
    match v {
        FieldValue::Element { element } => out.push(*element),
        FieldValue::List(l) => l.iter().for_each(|x| collect_elements(x, out)),
        FieldValue::Object(o) => o.fields.iter().for_each(|f| collect_elements(&f.value, out)),
        _ => {}
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
