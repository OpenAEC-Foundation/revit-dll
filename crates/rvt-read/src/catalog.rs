//! Readers on top of the file's own schema: project information, views, families, element types
//! and parameters, plus generic access to any element as a decoded object.
//!
//! Field meanings were established by comparison with public-API output (`tools/parity_oracle.py`).

use std::collections::{BTreeMap, HashMap};

use rvt_core::{decode_element, decode_global, inflated_stream, Cfb, Chain, Decoded, Element, EsField, EsSchema, EsSchemas, Node, Result, Schema, Value};
use rvt_model::builtins::{builtin_category, builtin_parameter};
use rvt_model::{ElementType, Family, ParamData, ParamValue, ParameterBinding, ParameterDef, ProjectInfo, SheetInfo, View};
use serde_json::{json, Map, Value as Json};

use crate::Document;

const P_VIEW_DETAIL: i64 = -1011002;
const P_VIEW_DISCIPLINE: i64 = -1005163;
const P_VIEW_PHASE: i64 = -1012102;
const P_VIEW_PHASE_FILTER: i64 = -1012103;
const P_PROJECT_NUMBER: i64 = -1006316;
const P_PROJECT_NAME: i64 = -1006317;
const P_PROJECT_ADDRESS: i64 = -1006318;
const P_CLIENT_NAME: i64 = -1006319;
const P_PROJECT_STATUS: i64 = -1006320;
const P_PROJECT_ISSUE_DATE: i64 = -1006321;
const P_PROJECT_AUTHOR: i64 = -1019005;
const P_PROJECT_BUILDING_NAME: i64 = -1019006;
const P_PROJECT_ORGANIZATION_DESCRIPTION: i64 = -1019007;
const P_PROJECT_ORGANIZATION_NAME: i64 = -1019008;

/// What the file says about itself, apart from elements.
pub(crate) struct Formats {
    pub schema: Schema,
    pub es: EsSchemas,
    pub file: BTreeMap<String, String>,
    pub saved_by: Vec<String>,
}

fn guid_string(g: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g[3], g[2], g[1], g[0], g[5], g[4], g[7], g[6], g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
    )
}

/// `BasicFileInfo` ends in UTF-16 `key: value` lines, starting at "Worksharing:".
fn file_properties(raw: &[u8]) -> BTreeMap<String, String> {
    let needle: Vec<u8> = "Worksharing:".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let Some(start) = crate::util::find(raw, &needle) else { return BTreeMap::new() };
    let units: Vec<u16> = raw[start..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
        .lines()
        .filter_map(|l| l.split_once(':'))
        .take_while(|(k, _)| k.chars().all(|c| c.is_ascii_alphanumeric() || c == ' '))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_end_matches('\0').to_string()))
        .collect()
}

impl Formats {
    pub fn read(cfb: &Cfb) -> Option<Formats> {
        let schema = Schema::parse(&inflated_stream(cfb, "Formats/Latest").ok()?).ok()?;
        let file = cfb.stream("BasicFileInfo").map(|b| file_properties(&b)).unwrap_or_default();
        let mut formats = Formats { schema, es: EsSchemas::default(), file, saved_by: Vec::new() };
        if let Ok(raw) = inflated_stream(cfb, "Global/Latest") {
            if let Ok(d) = decode_global(&formats.schema, None, &raw) {
                let saved_by = d.root().items("m_storedByRevitBuild").iter().filter_map(|n| n.str("m_str").map(String::from)).collect();
                let es = es_schemas(&d);
                formats.saved_by = saved_by;
                formats.es = es;
            }
        }
        Some(formats)
    }
}

/// Extensible-storage schemas, kept in the document root's `ESSchemaStorage`.
fn es_schemas(d: &Decoded) -> EsSchemas {
    let mut list = Vec::new();
    for storage in d.of_class("ESSchemaStorage") {
        // up to 2023 `m_storedSchemas` pairs a GUID with the schema itself
        for entry in storage.items("m_schemaUsageMap").into_iter().chain(storage.items("m_storedSchemas")) {
            let guid = entry.child("first").and_then(|g| match g.get("m_guid") {
                Some(Value::Guid(v)) => Some(*v),
                _ => None,
            });
            let schema = entry.child("second").and_then(|u| u.child("m_schema").or(Some(u)));
            if let (Some(guid), Some(schema)) = (guid, schema) {
                let fields = schema
                    .items("m_fields")
                    .iter()
                    .map(|f| EsField {
                        name: f.str("m_fieldName").unwrap_or_default().to_string(),
                        type_name: f.str("m_fieldTypeName").unwrap_or_default().to_string(),
                        entry_index: f.int("m_entryIndex").unwrap_or(0),
                        container: f.int("m_containerType").unwrap_or(0),
                    })
                    .collect();
                list.push((guid, EsSchema { name: schema.str("m_schemaName").unwrap_or("ESBlob").to_string(), fields }));
            }
        }
    }
    EsSchemas::new(list)
}

/// A decoded graph as JSON: pointers resolved, `$class` on every object.
pub fn object_json(d: &Decoded) -> Json {
    obj_json(d, d.root().o)
}

fn obj_json(d: &Decoded, o: &rvt_core::Object) -> Json {
    let mut m = Map::new();
    m.insert("$class".into(), json!(o.class));
    for (k, v) in &o.fields {
        m.insert((*k).to_string(), val_json(d, v));
    }
    Json::Object(m)
}

fn val_json(d: &Decoded, v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => json!(b),
        Value::Int(i) | Value::Id(i) => json!(i),
        Value::Float(f) => json!(f),
        Value::Str(s) => json!(s),
        Value::Guid(g) => json!(guid_string(g)),
        Value::List(l) => Json::Array(l.iter().map(|x| val_json(d, x)).collect()),
        Value::Struct(o) => obj_json(d, o),
        Value::Ptr(i) => d.objects.get(*i).map_or(Json::Null, |o| obj_json(d, o)),
        Value::Back(n) => json!({"$back": n}),
    }
}

/// Values in `m_paramSet` of the four sets every element can have, then a family type's own table.
fn stored_parameters(n: &Node) -> Vec<(i64, &'static str, ParamData, bool)> {
    let mut out = Vec::new();
    for (field, storage) in [("m_pParamValueSetDouble", "Double"), ("m_pParamValueSetInt", "Integer"), ("m_pParamValueSetAString", "String"), ("m_pParamValueSetElementId", "ElementId")] {
        let Some(set) = n.ptr(field) else { continue };
        for item in set.items("m_paramSet") {
            let Some(id) = item.id("m_paramId") else { continue };
            let value = match item.get("m_value") {
                Some(Value::Float(f)) => ParamData::Float(*f),
                Some(Value::Int(i) | Value::Id(i)) => ParamData::Int(*i),
                Some(Value::Str(s)) => ParamData::Text(s.clone()),
                _ => ParamData::None(None),
            };
            out.push((id, storage, value, false));
        }
    }
    if let Some(table) = n.ptr("m_pParams") {
        for item in table.items("m_params") {
            let Some(id) = item.id("m_paramId") else { continue };
            let text = item.str("m_str").unwrap_or_default();
            let (real, whole, elem) = (item.f64("m_value").unwrap_or(0.0), item.int("m_int").unwrap_or(0), item.id("m_elemId").unwrap_or(-1));
            let value = if !text.is_empty() {
                ParamData::Text(text.to_string())
            } else if elem > 0 {
                ParamData::Int(elem)
            } else if real != 0.0 {
                ParamData::Float(real)
            } else if whole != 0 {
                ParamData::Int(whole)
            } else {
                ParamData::Int(0)
            };
            out.push((id, "Stored", value, item.ptr("m_oExpression").is_some()));
        }
    }
    out
}

fn view_kind(class: &str, n: &Node, family_of_type: &HashMap<i64, i64>) -> String {
    match class {
        "DBViewPlan" => match n.int("m_planViewType") {
            Some(2) => "CeilingPlan".into(),
            Some(1) => match n.id("m_dbViewTypeId").and_then(|t| family_of_type.get(&t)) {
                Some(&f) if f == STRUCTURAL_PLAN_FAMILY => "EngineeringPlan".into(),
                _ => "FloorPlan".into(),
            },
            v => format!("Plan{}", v.unwrap_or(-1)),
        },
        "DBViewSection" => match n.int("m_sectionViewType") {
            Some(0) => "Section".into(),
            Some(2) => "Elevation".into(),
            Some(3) => "Detail".into(),
            v => format!("Section{}", v.unwrap_or(-1)),
        },
        "DBView3d" => "ThreeD".into(),
        "DBViewDrafting" => "DrawingSheet".into(),
        "DBViewAnonDrafting" => "DraftingView".into(),
        "DBViewLegend" => "Legend".into(),
        "DBViewProject" => "ProjectBrowser".into(),
        "DBViewSchedule" => "Schedule".into(),
        "RbsDbViewSystemNavigator" => "SystemBrowser".into(),
        other => other.to_string(),
    }
}

/// `DBViewType.m_systemFamilyIdx` of structural plans.
const STRUCTURAL_PLAN_FAMILY: i64 = 120;

/// Definitions of a fixed kind have no spec field.
fn implied_spec(def_class: &str) -> Option<&'static str> {
    Some(match def_class {
        "ParamDefString" => "autodesk.spec:spec.string-2.0.0",
        "ParamDefYesNo" => "autodesk.spec:spec.bool-1.0.0",
        "ParamDefInt" => "autodesk.spec:spec.int64-2.0.0",
        "ParamDefMaterialBrowse" => "autodesk.spec.aec:material-1.0.0",
        "ParamDefURL" => "autodesk.spec.string:url-2.0.0",
        "ParamDefImageSymbolBrowseEdit" => "autodesk.spec.reference:image-1.0.0",
        _ => return None,
    })
}

/// Parameter definitions and bindings of the project.
pub struct Parameters {
    pub definitions: Vec<ParameterDef>,
    pub bindings: Vec<ParameterBinding>,
}

impl Document {
    pub fn schema(&self) -> Option<&Schema> {
        self.formats.as_ref().map(|f| &f.schema)
    }

    /// Chain 0 of every partition: the project document. Other chains are embedded family documents.
    fn host_elements(&self) -> impl Iterator<Item = (&Chain, &Element)> + '_ {
        self.partitions.iter().filter_map(|p| p.chains.first().filter(|c| c.bits.is_some())).flat_map(|c| c.elements.iter().map(move |e| (c, e)))
    }

    fn is_a(&self, e: &Element, base: u16) -> bool {
        e.tag.and_then(|t| self.schema()?.class(t)).is_some_and(|c| c.lineage.contains(&base))
    }

    fn decode(&self, ch: &Chain, e: &Element) -> Option<Result<Decoded<'_>>> {
        let f = self.formats.as_ref()?;
        Some(decode_element(&f.schema, Some(&f.es), ch, e))
    }

    /// Chain 0 elements of a schema class or a class derived from it, decoded. This includes the
    /// content of loaded families and of group types.
    fn host_all<'a>(&'a self, base: &str) -> Vec<(&'a Element, Decoded<'a>)> {
        let Some(id) = self.schema().and_then(|s| s.id_of(base)) else { return Vec::new() };
        self.host_elements().filter(|(_, e)| self.is_a(e, id)).filter_map(|(c, e)| Some((e, self.decode(c, e)?.ok()?))).collect()
    }

    /// As [`Self::host_all`], without what belongs to a family (`m_famId`) or to a group type
    /// (`m_unplacedOwnerId`): the elements the public API lists for the project.
    fn host_of<'a>(&'a self, base: &str) -> Vec<(&'a Element, Decoded<'a>)> {
        let mut all = self.host_all(base);
        all.retain(|(_, d)| {
            let n = d.root();
            n.id("m_famId").unwrap_or(-1) == -1 && n.id("m_unplacedOwnerId").unwrap_or(-1) == -1
        });
        all
    }

    /// Family types and other symbols; sub-types of a master type are internal.
    fn symbols_of<'a>(&'a self, base: &str) -> Vec<(&'a Element, Decoded<'a>)> {
        let mut all = self.host_of(base);
        all.retain(|(_, d)| d.root().id("m_masterId").unwrap_or(-1) == -1);
        all
    }

    /// Any element as a decoded object graph; `None` without schema or element.
    pub fn decoded(&self, id: u64) -> Option<Result<Decoded<'_>>> {
        let &(pi, ci, ei) = self.index.get(&id)?;
        let ch = &self.partitions[pi].chains[ci];
        self.decode(ch, &ch.elements[ei])
    }

    /// All project elements of a schema class, as JSON.
    pub fn class_objects(&self, class: &str) -> Result<Json> {
        Ok(Json::Array(self.host_of(class).iter().map(|(_, d)| object_json(d)).collect()))
    }

    /// Element count per schema class, over all chains.
    pub fn class_counts(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        let Some(s) = self.schema() else { return out };
        for p in &self.partitions {
            for c in &p.chains {
                for e in &c.elements {
                    if let Some(k) = e.tag.and_then(|t| s.class(t)) {
                        *out.entry(k.name.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
        out
    }

    /// Decodes every element of the file: elements read to their last byte, and the failures per
    /// class with one message each.
    pub fn decode_check(&self) -> Json {
        let (mut ok, mut failed) = (0usize, BTreeMap::<String, (usize, String)>::new());
        for p in &self.partitions {
            for c in &p.chains {
                for e in &c.elements {
                    match self.decode(c, e) {
                        Some(Ok(_)) => ok += 1,
                        Some(Err(err)) => {
                            let class = e.tag.and_then(|t| self.schema()?.class(t)).map_or("?".to_string(), |k| k.name.clone());
                            failed.entry(class).or_insert((0, err.to_string())).0 += 1;
                        }
                        None => {}
                    }
                }
            }
        }
        json!({"decoded": ok, "failed": failed.values().map(|f| f.0).sum::<usize>(), "by_class": failed})
    }

    /// The schema as JSON: classes with parent and field names.
    pub fn schema_json(&self) -> Json {
        let Some(s) = self.schema() else { return Json::Null };
        Json::Array(
            s.classes()
                .map(|c| {
                    json!({
                        "id": c.id, "name": c.name, "version": c.version,
                        "parent": s.class(c.parent).map(|p| p.name.clone()),
                        "fields": c.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
                    })
                })
                .collect(),
        )
    }

    /// Captions of all parameter elements in the project.
    fn parameter_names(&self) -> HashMap<i64, String> {
        self.host_all("ParamElem")
            .iter()
            .filter_map(|(e, d)| Some((e.id as i64, d.root().ptr("m_pParamDef")?.str("m_caption")?.to_string())))
            .collect()
    }

    fn values(&self, n: &Node, names: &HashMap<i64, String>) -> Vec<ParamValue> {
        stored_parameters(n)
            .into_iter()
            .map(|(id, storage, value, formula)| {
                let builtin = builtin_parameter(id);
                ParamValue {
                    id,
                    name: names.get(&id).cloned().or_else(|| builtin.and_then(|b| b.label.or(Some(b.name))).map(String::from)),
                    builtin: builtin.map(|b| b.name),
                    storage,
                    value,
                    formula,
                }
            })
            .collect()
    }

    /// Parameter values stored for one element; an instance also shows its type's values that it
    /// does not override.
    pub fn element_parameters(&self, id: u64) -> Vec<ParamValue> {
        let names = self.parameter_names();
        let Some(Ok(d)) = self.decoded(id) else { return Vec::new() };
        let root = d.root();
        let mut out = self.values(&root, &names);
        if let Some(symbol) = root.id("m_masterSymbolId").filter(|s| *s > 0) {
            if let Some(Ok(t)) = self.decoded(symbol as u64) {
                for v in self.values(&t.root(), &names) {
                    if !out.iter().any(|x| x.id == v.id) {
                        out.push(v);
                    }
                }
            }
        }
        out
    }

    pub fn project_info(&self) -> Option<ProjectInfo> {
        let all = self.host_of("ProjectInfo");
        let (e, d) = all.first()?;
        let parameters = self.values(&d.root(), &self.parameter_names());
        let text = |id: i64| {
            parameters.iter().find(|p| p.id == id).and_then(|p| match &p.value {
                ParamData::Text(s) => Some(s.clone()),
                _ => None,
            })
        };
        Some(ProjectInfo {
            id: e.id,
            name: text(P_PROJECT_NAME),
            number: text(P_PROJECT_NUMBER),
            client_name: text(P_CLIENT_NAME),
            address: text(P_PROJECT_ADDRESS),
            status: text(P_PROJECT_STATUS),
            issue_date: text(P_PROJECT_ISSUE_DATE),
            author: text(P_PROJECT_AUTHOR),
            building_name: text(P_PROJECT_BUILDING_NAME),
            organization_name: text(P_PROJECT_ORGANIZATION_NAME),
            organization_description: text(P_PROJECT_ORGANIZATION_DESCRIPTION),
            parameters,
        })
    }

    pub fn views(&self) -> Vec<View> {
        let names = self.parameter_names();
        let family_of_type: HashMap<i64, i64> = self.host_of("DBViewType").iter().filter_map(|(e, d)| Some((e.id as i64, d.root().int("m_systemFamilyIdx")?))).collect();
        self.host_of("DBView")
            .iter()
            .map(|(e, d)| {
                let n = d.root();
                let parameters = self.values(&n, &names);
                let stored = |id: i64| {
                    parameters.iter().find(|p| p.id == id).and_then(|p| match p.value {
                        ParamData::Int(v) => Some(v),
                        _ => None,
                    })
                };
                let is_plan = n.class() == "DBViewPlan";
                View {
                    id: e.id,
                    class: n.class().to_string(),
                    kind: view_kind(n.class(), &n, &family_of_type),
                    name: n.str("m_viewName").unwrap_or_default().to_string(),
                    description: n.str("m_viewDescription").unwrap_or_default().to_string(),
                    scale: n.f64("m_scale").filter(|s| *s > 0.0).map(|s| (1.0 / s).round() as i64),
                    is_template: n.bool("m_isTemplate").unwrap_or(false),
                    template: n.id("m_viewTemplateId").unwrap_or(-1),
                    view_type: n.id("m_dbViewTypeId").unwrap_or(-1),
                    level: n.id("m_genElemId").filter(|l| *l > 0),
                    origin: n.xyz("m_origin"),
                    right: n.xyz("m_horzDir"),
                    up: n.xyz("m_vertDir"),
                    direction: n.xyz("m_viewDir"),
                    primary: n.id("m_primaryViewId").unwrap_or(-1),
                    detail_level: stored(P_VIEW_DETAIL).and_then(|v| match v {
                        1 => Some("Coarse"),
                        2 => Some("Medium"),
                        3 => Some("Fine"),
                        _ => None,
                    }),
                    discipline: stored(P_VIEW_DISCIPLINE).and_then(|v| match v {
                        1 => Some("Architectural"),
                        2 => Some("Structural"),
                        4 => Some("Mechanical"),
                        8 => Some("Electrical"),
                        16 => Some("Plumbing"),
                        4095 => Some("Coordination"),
                        _ => None,
                    }),
                    phase: stored(P_VIEW_PHASE),
                    phase_filter: stored(P_VIEW_PHASE_FILTER),
                    cut_plane: n.f64("m_cutPlaneElev").filter(|_| is_plan),
                    top_clip: n.f64("m_topClipElev").filter(|_| is_plan),
                    bottom_clip: n.f64("m_bottomClipElev").filter(|_| is_plan),
                    sheet: (n.class() == "DBViewDrafting").then(|| {
                        let s = |f: &str| n.str(f).unwrap_or_default().to_string();
                        SheetInfo {
                            number: s("m_sheetNumber"),
                            date: s("m_date"),
                            drawn_by: s("m_drawnBy"),
                            checked_by: s("m_checkedBy"),
                            designed_by: s("m_designedBy"),
                            approved_by: s("m_approvedBy"),
                            issue_date: s("m_issueDate"),
                        }
                    }),
                    parameters,
                }
            })
            .collect()
    }

    pub fn families(&self) -> Vec<Family> {
        let mut types: HashMap<i64, Vec<u64>> = HashMap::new();
        for (e, d) in self.symbols_of("FamilySymbol") {
            if let Some(f) = d.root().id("m_familyId") {
                types.entry(f).or_default().push(e.id);
            }
        }
        self.host_of("Family")
            .iter()
            .map(|(e, d)| {
                let n = d.root();
                let flag = |f: &str| n.bool(f).unwrap_or(false);
                let category = n.id("m_categoryId").unwrap_or(-1);
                Family {
                    id: e.id,
                    name: n.str("m_name").unwrap_or_default().to_string(),
                    path: n.str("m_path").unwrap_or_default().to_string(),
                    category,
                    category_name: builtin_category(category).map(|c| c.label.unwrap_or(c.name)),
                    in_place: flag("m_inPlace"),
                    editable: flag("m_bIsSavable"),
                    parametric: flag("m_isParametric"),
                    shared: flag("m_bShared"),
                    host_based: flag("m_bIsHostBased"),
                    work_plane_based: flag("m_isWorkPlaneBased"),
                    curve_based: flag("m_curveBased"),
                    is_2d: flag("m_is2d"),
                    is_model: flag("m_isModel"),
                    types: types.remove(&(e.id as i64)).unwrap_or_default(),
                }
            })
            .collect()
    }

    pub fn element_types(&self) -> Vec<ElementType> {
        let names = self.parameter_names();
        let family_names: HashMap<i64, String> = self.host_of("Family").iter().filter_map(|(e, d)| Some((e.id as i64, d.root().str("m_name")?.to_string()))).collect();
        self.symbols_of("Symbol")
            .iter()
            .map(|(e, d)| {
                let n = d.root();
                let family = n.id("m_familyId").filter(|f| *f > 0);
                ElementType {
                    id: e.id,
                    class: n.class().to_string(),
                    category: e.cat,
                    name: n.ptr("m_symbolInfo").and_then(|s| s.str("m_name").map(String::from)).unwrap_or_default(),
                    family,
                    family_name: family.and_then(|f| family_names.get(&f).cloned()),
                    parameters: self.values(&n, &names),
                }
            })
            .collect()
    }

    pub fn parameters(&self) -> Parameters {
        let definitions: Vec<ParameterDef> = self
            .host_all("ParamElem")
            .iter()
            .map(|(e, d)| {
                let n = d.root();
                let def = n.ptr("m_pParamDef");
                let forge = |f: &str| def.and_then(|x| x.child(f)).and_then(|x| x.str("m_typeId")).filter(|s| !s.is_empty()).map(String::from);
                let spec = forge("m_specTypeId").or_else(|| def.and_then(|x| implied_spec(x.class())).map(String::from));
                let guid = n.child("m_externalParamKey").and_then(|k| k.child("m_guidValue")).and_then(|g| match g.get("m_guid") {
                    Some(Value::Guid(v)) => Some(guid_string(v)),
                    _ => None,
                });
                ParameterDef {
                    id: e.id,
                    class: n.class().to_string(),
                    kind: match n.class() {
                        "ParamElemExternal" => "shared",
                        "ParamElemProject" => "project",
                        "ParamElemGlobal" => "global",
                        "ParamElemFamily" => "family",
                        _ => "other",
                    },
                    name: def.and_then(|x| x.str("m_caption")).unwrap_or_default().to_string(),
                    description: n.str("m_description").unwrap_or_default().to_string(),
                    spec,
                    group: forge("m_groupTypeId"),
                    visible: def.and_then(|x| x.bool("m_userVisible")),
                    guid,
                    instance: n.bool("m_instanceParam"),
                    family: n.id("m_famId").filter(|f| *f != -1),
                }
            })
            .collect();
        let names: HashMap<i64, &str> = definitions.iter().map(|d| (d.id as i64, d.name.as_str())).collect();
        let mut bound: BTreeMap<i64, ParameterBinding> = BTreeMap::new();
        for (_, d) in self.host_of("ParamBinding") {
            let n = d.root();
            let Some(parameter) = n.id("m_paramId") else { continue };
            let entry = bound.entry(parameter).or_insert_with(|| ParameterBinding {
                parameter,
                name: names.get(&parameter).map(|s| s.to_string()),
                binding: if n.int("m_elemOrSymbol") == Some(2) { "TypeBinding" } else { "InstanceBinding" },
                categories: Vec::new(),
            });
            if let Some(c) = n.id("m_categoryId") {
                entry.categories.push(c);
            }
        }
        Parameters { definitions, bindings: bound.into_values().collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_is_written_in_the_usual_mixed_endian_form() {
        let bytes = [0xf7, 0x67, 0x03, 0x07, 0x38, 0x09, 0xb2, 0x47, 0xb1, 0xab, 0x51, 0xf4, 0xba, 0xfe, 0x69, 0x40];
        assert_eq!(guid_string(&bytes), "070367f7-0938-47b2-b1ab-51f4bafe6940");
    }

    #[test]
    fn file_properties_are_read_from_the_utf16_lines_up_to_the_first_odd_line() {
        let text = "Worksharing: Not enabled\r\nCentral Model Path: \r\nFormat: 2025\r\nnot a property\r\n";
        let mut raw = vec![1u8, 2, 3]; // an odd offset must not matter
        raw.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
        let p = file_properties(&raw);
        assert_eq!(p.get("Worksharing").map(String::as_str), Some("Not enabled"));
        assert_eq!(p.get("Central Model Path").map(String::as_str), Some(""));
        assert_eq!(p.get("Format").map(String::as_str), Some("2025"));
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn plan_kind_depends_on_plan_type_and_view_family() {
        let schema_bytes = |_: ()| {
            let mut b = vec![0u8, 0, 1, 0, b'V', 0, 0, 0, 0, 0, 0, 2, 0, 0, 0];
            b.extend_from_slice(&14u32.to_le_bytes());
            b.extend_from_slice(b"m_planViewType");
            b.extend_from_slice(&[4, 0, 0, 0]);
            b.extend_from_slice(&0u32.to_le_bytes());
            b
        };
        // one int field is enough for the ceiling plan branch
        let mut bytes = schema_bytes(());
        bytes[11] = 1;
        let schema = Schema::parse(&bytes).unwrap();
        let d = rvt_core::decode(&schema, None, &2i32.to_le_bytes(), 0, schema.id_of("V").unwrap()).unwrap();
        assert_eq!(view_kind("DBViewPlan", &d.root(), &HashMap::new()), "CeilingPlan");
        let d = rvt_core::decode(&schema, None, &1i32.to_le_bytes(), 0, schema.id_of("V").unwrap()).unwrap();
        assert_eq!(view_kind("DBViewPlan", &d.root(), &HashMap::new()), "FloorPlan");
    }
}
