// Generic object layer against the typed readers (branch open-research, par C): for levels and
// walls the values read from the object view (class fields, no byte offsets) must equal the
// typed readers' values; materials are compared with the reference export when one is given.
//
// Usage: object_parity <file.rvt> [reference export .json]
// Prints one JSON line: per list the records compared, per field the matches, and examples of
// mismatches.
//
// Field mapping (object view, rvt-model ObjectView):
// - Level: name = DatumPlane::m_text; origin = m_pSurface → Plane::m_origin, elevation = its z;
//   uv_box = Plane::m_Envelope.m_corners; type = Level::m_attrId.
// - Wall: type = VWall::m_WallAttributesId; base_level = Element::m_assocLevelId; top_level =
//   VWall::m_upToLevelId; base/top offset = ParamValueSetDouble values of WALL_BASE_OFFSET
//   (−1001108) / WALL_TOP_OFFSET (−1001109); p0/p1 = VWall::m_pCurveDriver → VWallDriver::m_pCrv
//   → GLine m_origin + t·m_dirVec at m_endParams (straight walls; within 1e-9 ft).
// - Material: Material (MaterialElem::m_pMaterial) m_name, m_color (0xBBGGRR), m_transparency,
//   m_smoothness, m_shininess, m_appearanceAssetId, m_cutPatternId, m_surfacePatternId,
//   m_sMaterialClass, m_sCategory.
use std::collections::BTreeMap;

use rvt_model::{FieldValue, ObjectView};
use serde_json::{json, Value};

#[derive(Default)]
struct Tally {
    records: usize,
    missing: usize,
    fields: BTreeMap<&'static str, (usize, usize)>,
    examples: Vec<String>,
}

impl Tally {
    fn check(&mut self, id: u64, field: &'static str, ok: bool, detail: impl FnOnce() -> String) {
        let e = self.fields.entry(field).or_default();
        e.1 += 1;
        if ok {
            e.0 += 1;
        } else if self.examples.len() < 8 {
            self.examples.push(format!("{id} {field}: {}", detail()));
        }
    }
    fn json(&self) -> Value {
        let all = self.fields.values().all(|(m, n)| m == n);
        json!({
            "records": self.records,
            "missing_object": self.missing,
            "all_fields_equal": all && self.missing == 0,
            "fields": self.fields.iter().map(|(k, (m, n))| (k.to_string(), json!(format!("{m}/{n}")))).collect::<serde_json::Map<_, _>>(),
            "examples": self.examples,
        })
    }
}

fn floats(v: Option<&FieldValue>) -> Option<Vec<f64>> {
    v?.as_list()?.iter().map(|x| x.as_f64()).collect()
}

fn target<'a>(o: &'a ObjectView, name: &str) -> Option<&'a rvt_model::NestedObject> {
    o.deref(o.get(name)?)
}

fn param<'a>(o: &'a ObjectView, set: &str, pid: i64) -> Option<&'a FieldValue> {
    let set = target(o, set)?;
    set.get("m_paramSet")?.as_list()?.iter().filter_map(|p| p.as_object()).find(|p| p.get("m_paramId").and_then(|v| v.element()) == Some(pid))?.get("m_value")
}

fn param_double(o: &ObjectView, pid: i64) -> Option<f64> {
    param(o, "m_pParamValueSetDouble", pid)?.as_f64()
}

fn levels(doc: &rvt_read::Document) -> Tally {
    let mut t = Tally::default();
    for l in doc.levels() {
        t.records += 1;
        let Some(o) = doc.object(l.id) else {
            t.missing += 1;
            continue;
        };
        let plane = target(&o, "m_pSurface");
        let origin = plane.and_then(|p| floats(p.get("m_origin")));
        let uv: Option<Vec<f64>> = plane.and_then(|p| p.get("m_Envelope")?.as_object()?.get("m_corners")?.as_list()?.iter().map(|c| floats(Some(c))).collect::<Option<Vec<_>>>()).map(|c| c.concat());
        let name = o.get("m_text").and_then(|v| v.as_str()).map(str::to_string);
        let ty = o.get("m_attrId").and_then(|v| v.element());
        t.check(l.id, "class", o.is_a("Level"), || o.class.clone());
        t.check(l.id, "name", name.as_deref() == Some(l.name.as_str()), || format!("{name:?} vs {:?}", l.name));
        t.check(l.id, "elevation", origin.as_ref().and_then(|v| v.get(2)) == Some(&l.elevation), || format!("{origin:?} vs {}", l.elevation));
        t.check(l.id, "origin", origin.as_deref() == Some(&l.origin[..]), || format!("{origin:?} vs {:?}", l.origin));
        t.check(l.id, "uv_box", uv.as_deref() == Some(&l.uv_box[..]), || format!("{uv:?} vs {:?}", l.uv_box));
        t.check(l.id, "type", ty.map(|v| v as u64) == l.level_type || (ty == Some(-1) && l.level_type.is_none()), || format!("{ty:?} vs {:?}", l.level_type));
    }
    t
}

fn walls(doc: &rvt_read::Document) -> Tally {
    let mut t = Tally::default();
    for w in doc.walls() {
        t.records += 1;
        let Some(o) = doc.object(w.id) else {
            t.missing += 1;
            continue;
        };
        let el = |n: &str| o.get(n).and_then(|v| v.element());
        t.check(w.id, "class", o.is_a("VWall"), || o.class.clone());
        t.check(w.id, "type", el("m_WallAttributesId") == Some(w.wall_type), || format!("{:?} vs {}", el("m_WallAttributesId"), w.wall_type));
        t.check(w.id, "base_level", el("m_assocLevelId") == Some(w.base_level), || format!("{:?} vs {}", el("m_assocLevelId"), w.base_level));
        t.check(w.id, "top_level", el("m_upToLevelId") == Some(w.top_level), || format!("{:?} vs {}", el("m_upToLevelId"), w.top_level));
        let bo = param_double(&o, -1001108);
        t.check(w.id, "base_offset", bo == Some(w.base_offset), || format!("{bo:?} vs {}", w.base_offset));
        let to = param_double(&o, -1001109);
        t.check(w.id, "top_offset", to == Some(w.top_offset), || format!("{to:?} vs {}", w.top_offset));
        if w.curve.is_some() {
            continue; // arc walls: not compared
        }
        let line = target(&o, "m_pCurveDriver").and_then(|d| o.deref(d.get("m_pCrv")?)).filter(|c| c.class == "GLine");
        let ends = line.and_then(|c| {
            let (org, dir, tp) = (floats(c.get("m_origin"))?, floats(c.get("m_dirVec"))?, floats(c.get("m_endParams"))?);
            Some([0, 1].map(|k| [0, 1, 2].map(|i| org[i] + tp[k] * dir[i])))
        });
        let close = |a: &[f64; 3], b: &[f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-9);
        t.check(w.id, "p0_p1", ends.is_some_and(|e| close(&e[0], &w.p0) && close(&e[1], &w.p1)), || format!("{ends:?} vs {:?} {:?}", w.p0, w.p1));
    }
    t
}

/// Materials of the reference export (project chains), by id.
fn reference_materials(path: &str) -> BTreeMap<u64, Value> {
    // the reference writes Python floats: NaN / Infinity are not JSON
    let text = std::fs::read_to_string(path).expect("reference").replace("-Infinity", "null").replace("Infinity", "null").replace("NaN", "null");
    let v: Value = serde_json::from_str(&text).expect("json");
    let mut out = BTreeMap::new();
    for p in v["partitions"].as_array().into_iter().flatten() {
        for c in p["chains"].as_array().into_iter().flatten() {
            for m in c["project"]["materials"].as_array().into_iter().flatten() {
                if let Some(id) = m["id"].as_u64() {
                    out.insert(id, m.clone());
                }
            }
        }
    }
    out
}

fn materials(doc: &rvt_read::Document, reference: Option<&str>) -> Tally {
    let mut t = Tally::default();
    let Some(r) = reference else {
        // without a reference: the materials the typed compound-type reader names must be
        // MaterialElem objects
        let ids: std::collections::BTreeSet<i64> = doc.compound_types().iter().flat_map(|c| c.layers.iter().filter_map(|l| l.material)).collect();
        for id in ids {
            t.records += 1;
            let o = doc.object(id as u64);
            t.check(id as u64, "layer_material_is_MaterialElem", o.as_ref().is_some_and(|o| o.is_a("MaterialElem")), || format!("{:?}", o.map(|o| o.class)));
        }
        return t;
    };
    for (id, m) in reference_materials(r) {
        t.records += 1;
        let Some(o) = doc.object(id) else {
            t.missing += 1;
            continue;
        };
        let Some(mat) = target(&o, "m_pMaterial") else {
            t.check(id, "material", false, || "no m_pMaterial".into());
            continue;
        };
        let s = |n: &str| mat.get(n).and_then(|v| v.as_str()).map(|x| json!(x)).unwrap_or(Value::Null);
        let f = |n: &str| mat.get(n).and_then(|v| v.as_f64());
        let idref = |n: &str| mat.get(n).and_then(|v| v.element()).filter(|&x| x != -1).map_or(Value::Null, |x| json!(x));
        let color = mat.get("m_color").and_then(|v| v.as_i64());
        let rgb = color.map(|c| json!([c & 0xff, (c >> 8) & 0xff, (c >> 16) & 0xff])).unwrap_or(Value::Null);
        let num = |a: Option<f64>, b: &Value| a.is_some() && a == b.as_f64();
        t.check(id, "name", s("m_name") == m["name"], || format!("{} vs {}", s("m_name"), m["name"]));
        t.check(id, "rgb", rgb == m["rgb"], || format!("{rgb} vs {}", m["rgb"]));
        t.check(id, "shininess", num(f("m_shininess"), &m["shininess"]), || format!("{:?} vs {}", f("m_shininess"), m["shininess"]));
        t.check(id, "smoothness", num(f("m_smoothness"), &m["smoothness"]), || format!("{:?} vs {}", f("m_smoothness"), m["smoothness"]));
        t.check(id, "transparency", num(f("m_transparency"), &m["transparency"]) || num(f("m_transparency").map(|x| x / 100.0), &m["transparency"]), || format!("{:?} vs {}", f("m_transparency"), m["transparency"]));
        t.check(id, "appearance_asset", idref("m_appearanceAssetId") == m["appearance_asset"], || format!("{} vs {}", idref("m_appearanceAssetId"), m["appearance_asset"]));
        t.check(id, "cut_pattern", idref("m_cutPatternId") == m["cut_pattern"], || format!("{} vs {}", idref("m_cutPatternId"), m["cut_pattern"]));
        t.check(id, "surface_pattern", idref("m_surfacePatternId") == m["surface_pattern"], || format!("{} vs {}", idref("m_surfacePatternId"), m["surface_pattern"]));
        t.check(id, "mat_class", s("m_sMaterialClass") == m["mat_class"] || (m["mat_class"].is_null() && s("m_sMaterialClass") == json!("")), || format!("class {} category {} vs ref class {} category {}", s("m_sMaterialClass"), s("m_sCategory"), m["mat_class"], m["mat_category"]));
        t.check(id, "mat_category", s("m_sCategory") == m["mat_category"] || (m["mat_category"].is_null() && s("m_sCategory") == json!("")), || format!("{} vs {}", s("m_sCategory"), m["mat_category"]));
        // where the class differs: is the reference's value the element's Description parameter
        // (ALL_MODEL_DESCRIPTION −1010103, a ParamValueSetAString value)?
        if s("m_sMaterialClass") != m["mat_class"] && !m["mat_class"].is_null() {
            let desc = param(&o, "m_pParamValueSetAString", -1010103).and_then(|v| v.as_str()).map(|x| json!(x));
            t.check(id, "mat_class_mismatch_ref_is_description", desc.as_ref() == Some(&m["mat_class"]), || format!("{desc:?} vs {}", m["mat_class"]));
        }
    }
    t
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = rvt_read::Document::open(&a[1]).expect("open");
    let info = doc.info();
    let out = json!({
        "file": a[1],
        "version": info.version,
        "bits": info.bits,
        "object_records": doc.object_store().is_some(),
        "levels": levels(&doc).json(),
        "walls": walls(&doc).json(),
        "materials": materials(&doc, a.get(2).map(String::as_str)).json(),
    });
    println!("{out}");
}
