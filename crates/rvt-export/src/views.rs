//! Project views for a viewer (branch open-research, beta): 3D views with their camera and
//! section box, plans with their level and view range, sections and elevations with their crop
//! box. Read through the generic object view (par C), named schema fields only:
//! - every view derives from `DBView`: `m_viewName`, `m_isTemplate`, `m_scale` (= 1 / scale),
//!   `m_origin` / `m_viewDir` (towards the viewer) / `m_vertDir`, `m_dbViewTypeId` (the view family
//!   type), `m_viewerId` (the element holding the view's frame and crop);
//! - the view family: `DBViewType::m_systemFamilyIdx` of the view type (observed: 102 3D,
//!   103 walkthrough, 105 schedule, 107 sheet, 108 drafting, 109 floor plan, 110 area plan,
//!   111 ceiling plan, 112 section, 113 detail, 114 elevation, 117 legend, 120 structural plan);
//!   views without a type (view templates, some elevations) fall back to the class (`DBView3d`,
//!   `DBViewPlan` with `m_planViewType` 2 = ceiling, `DBViewSection::m_sectionViewType` 0 =
//!   section, 1/2 = elevation, 3 = detail; Snowdon HVAC 2026 templates 8/8 against the API);
//!   checked: Snowdon Structural 2026 117/117 and Snowdon HVAC 2026 86/86 views (id, name,
//!   ViewType, IsTemplate; gt_s657_views.json, gt_s297_views.json), HVAC levels 21/21 and scales
//!   43/43;
//! - camera (3D views): `Viewer3d::m_boundedSpace` = `BoundedSpace { m_orig, m_basis,
//!   m_boundOffset, m_boundActive, m_isOn }`: origin = eye, basis rows = right, up, forward (from
//!   the eye into the model); `Viewer::m_projMethodType` 2 = perspective, 1 = orthographic.
//!   rvt100 043 (Revit 2026): 'RE Iso' and 'RE Camera' exact against `View3D.GetOrientation()` and
//!   `IsPerspective` (gt_s709_views.json); Snowdon Structural 2026 against gt_s709_struct3d.json;
//! - section box: `Viewer3d::m_modelClipBoxId` → `ModelClipBox::m_boundedSpace`; `m_isOn` = the
//!   box is active (043: on for 'RE Iso', off for 'RE Camera', the notebook's byte o+126);
//! - bounds of a `BoundedSpace`: `m_boundOffset` flattened = max.x, min.x, max.y, min.y, max.z,
//!   min.z in the box's own frame (notebook steps 771-772: the crop box layout);
//! - plans: `DBViewPlan::m_genElemId` = the associated level; view range as absolute elevations
//!   `m_topClipElev`, `m_cutPlaneElev`, `m_bottomClipElev`, `m_levelBelowElev` (view depth); ±1e9
//!   = unlimited (notebook step 770: absolute heights, Revit's metric defaults 0/1200/0/2300 mm).
//!
//! Views of in-place families (`m_famId` ≠ −1) and of group types (`m_unplacedOwnerId` ≠ −1)
//! are not project views and are left out; `all_views` keeps them (`owned` = true).
//!
//! Coordinates are raw Revit model coordinates in feet, Z up (the mesh export's viewer
//! coordinates are metres, Y up: (x, z, −y) × 0.3048; the viewer converts).

use std::collections::HashMap;

use rvt_model::object::{FieldValue, ObjectView};
use rvt_read::Document;
use serde::Serialize;

/// The view family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ViewKind {
    ThreeD,
    FloorPlan,
    CeilingPlan,
    AreaPlan,
    StructuralPlan,
    Section,
    Elevation,
    /// A detail (callout) view of a section or plan.
    Detail,
    /// Anything else: "Sheet", "Schedule", "Drafting", "Legend", "Walkthrough", "Rendering",
    /// "ProjectBrowser", "SystemBrowser", or "family <n>" for an unknown view family.
    Other(String),
}

impl ViewKind {
    /// A plan seen from above or below at a level (floor, ceiling, area or structural plan).
    pub fn is_plan(&self) -> bool {
        matches!(self, ViewKind::FloorPlan | ViewKind::CeilingPlan | ViewKind::AreaPlan | ViewKind::StructuralPlan)
    }
    /// A view of the model (3D, plan, section, elevation or detail), as opposed to sheets,
    /// schedules, drafting views and legends.
    pub fn is_model_view(&self) -> bool {
        !matches!(self, ViewKind::Other(_))
    }
    /// Short name ("3D", "FloorPlan", …; the text of `Other`).
    pub fn name(&self) -> &str {
        match self {
            ViewKind::ThreeD => "3D",
            ViewKind::FloorPlan => "FloorPlan",
            ViewKind::CeilingPlan => "CeilingPlan",
            ViewKind::AreaPlan => "AreaPlan",
            ViewKind::StructuralPlan => "StructuralPlan",
            ViewKind::Section => "Section",
            ViewKind::Elevation => "Elevation",
            ViewKind::Detail => "Detail",
            ViewKind::Other(s) => s,
        }
    }
}

/// The camera of a 3D view (feet, model coordinates, unit vectors).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewCamera {
    pub eye: [f64; 3],
    /// View direction, from the eye into the model (`View3D.GetOrientation().ForwardDirection`).
    pub forward: [f64; 3],
    pub up: [f64; 3],
    pub perspective: bool,
    /// Perspective views: full horizontal and vertical field of view in radians, from the image
    /// rectangle on the near plane (043 'RE Camera': 50.2° × 38.7°, 4:3). Derived, not checked
    /// against the API.
    pub fov: Option<[f64; 2]>,
}

/// A box in its own frame: a point (x, y, z) of the box is `origin + x·basis[0] + y·basis[1] +
/// z·basis[2]` in model coordinates (feet). For a viewer's crop box basis[0] = right,
/// basis[1] = up, basis[2] = the view direction into the model, min.z = near (0) and max.z = far.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrientedBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub origin: [f64; 3],
    pub basis: [[f64; 3]; 3],
    /// Section box: the box is on (`BoundedSpace::m_isOn`). Crop box: bit 0 of
    /// `Viewer::m_viewerFlags` (hypothesis for CropBoxActive, not checked against the API).
    pub active: bool,
}

impl OrientedBox {
    /// The eight corners in model coordinates.
    pub fn corners(&self) -> [[f64; 3]; 8] {
        let mut out = [[0.0; 3]; 8];
        for (i, c) in out.iter_mut().enumerate() {
            let l = [
                if i & 1 == 0 { self.min[0] } else { self.max[0] },
                if i & 2 == 0 { self.min[1] } else { self.max[1] },
                if i & 4 == 0 { self.min[2] } else { self.max[2] },
            ];
            for k in 0..3 {
                c[k] = self.origin[k] + l[0] * self.basis[0][k] + l[1] * self.basis[1][k] + l[2] * self.basis[2][k];
            }
        }
        out
    }

    /// Axis-aligned bounds of the box in model coordinates (min, max).
    pub fn world_bounds(&self) -> ([f64; 3], [f64; 3]) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for c in self.corners() {
            for k in 0..3 {
                lo[k] = lo[k].min(c[k]);
                hi[k] = hi[k].max(c[k]);
            }
        }
        (lo, hi)
    }
}

/// The view range of a plan as absolute elevations (feet); `None` = unlimited.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewRange {
    pub top: Option<f64>,
    pub cut: Option<f64>,
    pub bottom: Option<f64>,
    /// View depth (the lowest plane drawn; for a ceiling plan the highest).
    pub depth: Option<f64>,
}

/// One view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewInfo {
    pub id: u64,
    pub name: String,
    pub kind: ViewKind,
    /// Schema class of the view (`DBView3d`, `DBViewPlan`, `DBViewSection`, …).
    pub class: String,
    /// 3D views: eye, forward, up in model feet as stored; perspective flag.
    pub camera: Option<ViewCamera>,
    /// 3D views with an active section box (`None` when the box is off).
    pub section_box: Option<OrientedBox>,
    /// The viewer's crop box (plans, sections, elevations, 3D views), whether active or not.
    pub crop_box: Option<OrientedBox>,
    /// Plans: the associated level id and its elevation (feet).
    pub level: Option<(u64, f64)>,
    pub level_name: Option<String>,
    /// Plans: cut plane height above the level (feet).
    pub cut_offset: Option<f64>,
    /// Plans: the view range as absolute elevations (feet).
    pub view_range: Option<ViewRange>,
    /// The scale denominator (100 = 1:100); `None` for perspective views.
    pub scale: Option<f64>,
    pub is_template: bool,
    /// The view belongs to an in-place family or a group type, not to the project browser.
    pub owned: bool,
}

/// The project's model views a viewer can show: 3D views, plans, sections, elevations and
/// details, without view templates, sheets, schedules, drafting views, legends and the views of
/// in-place families and group types. Sorted by kind, then name. Empty when the file has no
/// readable object records.
pub fn views(doc: &Document) -> Vec<ViewInfo> {
    let mut v: Vec<ViewInfo> = collect(doc, true).into_iter().filter(|v| v.kind.is_model_view() && !v.is_template && !v.owned).collect();
    v.sort_by(|a, b| kind_order(&a.kind).cmp(&kind_order(&b.kind)).then_with(|| a.name.cmp(&b.name)).then(a.id.cmp(&b.id)));
    v
}

/// Every view element (`DBView` and subclasses), templates, sheets, schedules, legends and
/// browsers included, by ascending id.
pub fn all_views(doc: &Document) -> Vec<ViewInfo> {
    collect(doc, false)
}

/// The categories and subcategories the project's default 3D view ("{3D}") hides (Visibility/
/// Graphics), from its draw filter `IckyExcludedCategoriesSetPtrWrapper::m_categoryIds`
/// (open-research step 2315). 3077 R26: 68 ids (rebar, planting, entourage, parking, light
/// source, MEP insulation and lining, ...); rac basic 2026 70, snowdon arch 2025 51 (none of them
/// in Revit's {3D} geometry, every category of that geometry absent from the list). Empty when
/// the file has no such view or no object records.
pub fn default_3d_hidden_categories(doc: &Document) -> std::collections::HashSet<i64> {
    let mut out = std::collections::HashSet::new();
    for id in doc.object_ids_of_class("DBView3d") {
        let Some(o) = doc.object(id) else { continue };
        if o.get("m_viewName").and_then(|v| v.as_str()) != Some("{3D}") || boolean(o.get("m_isTemplate")) == Some(true) {
            continue;
        }
        for f in o.get("m_oaDrawFilters").and_then(|v| v.as_list()).unwrap_or(&[]) {
            let Some(df) = o.deref(f) else { continue };
            if !df.class.starts_with("IckyExcludedCategories") {
                continue;
            }
            out.extend(df.get("m_categoryIds").and_then(|v| v.as_list()).unwrap_or(&[]).iter().filter_map(|c| c.element()));
        }
        break;
    }
    out
}

/// Classes that never hold a model view (skipped without decoding when `model_only`).
const NON_MODEL_CLASSES: &[&str] =
    &["DBViewDrafting", "DBViewAnonDrafting", "DBViewLegend", "DBViewSchedule", "DBViewProject", "RbsDbViewSystemNavigator", "DBViewRendering"];

fn collect(doc: &Document, model_only: bool) -> Vec<ViewInfo> {
    let Some(store) = doc.object_store() else { return Vec::new() };
    let mut out = Vec::new();
    let mut families = HashMap::new();
    for id in doc.object_ids_of_class("DBView") {
        if model_only {
            let c = store.class_of(id).unwrap_or_default();
            if NON_MODEL_CLASSES.contains(&c) || c.starts_with("DBViewGraphSched") {
                continue;
            }
        }
        if let Some(o) = root(doc, id) {
            out.push(view_info(doc, &o, &mut families));
        }
    }
    out
}

fn kind_order(k: &ViewKind) -> u8 {
    match k {
        ViewKind::ThreeD => 0,
        ViewKind::FloorPlan => 1,
        ViewKind::StructuralPlan => 2,
        ViewKind::AreaPlan => 3,
        ViewKind::CeilingPlan => 4,
        ViewKind::Section => 5,
        ViewKind::Elevation => 6,
        ViewKind::Detail => 7,
        ViewKind::Other(_) => 8,
    }
}

/// The root object of an element (fast; all fields read here are root fields or inline
/// objects), else the full object.
fn root(doc: &Document, id: u64) -> Option<ObjectView> {
    doc.object_root(id).or_else(|| doc.object(id))
}

fn element(v: Option<&FieldValue>) -> Option<u64> {
    v?.element().filter(|&e| e >= 0).map(|e| e as u64)
}

fn boolean(v: Option<&FieldValue>) -> Option<bool> {
    match v? {
        FieldValue::Bool(b) => Some(*b),
        FieldValue::Int(i) => Some(*i != 0),
        _ => None,
    }
}

/// All numbers of a (nested) list, in order.
fn floats(v: &FieldValue, out: &mut Vec<f64>) {
    match v {
        FieldValue::List(l) => l.iter().for_each(|x| floats(x, out)),
        x => {
            if let Some(f) = x.as_f64() {
                out.push(f)
            }
        }
    }
}

fn vec3(v: Option<&FieldValue>) -> Option<[f64; 3]> {
    let mut f = Vec::new();
    floats(v?, &mut f);
    (f.len() == 3 && f.iter().all(|x| x.is_finite())).then(|| [f[0], f[1], f[2]])
}

fn unit(v: [f64; 3]) -> bool {
    ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() - 1.0).abs() < 1e-6
}

/// A `BoundedSpace` (inline object or pointer target) as a box; `active` from `m_isOn`.
fn bounded_space(o: &ObjectView, v: &FieldValue) -> Option<OrientedBox> {
    let bs = match v {
        FieldValue::Object(n) => n,
        FieldValue::Ref { .. } => o.deref(v)?,
        _ => return None,
    };
    let origin = vec3(bs.get("m_orig"))?;
    let mut b = Vec::new();
    floats(bs.get("m_basis")?, &mut b);
    let mut off = Vec::new();
    floats(bs.get("m_boundOffset")?, &mut off);
    if b.len() != 9 || off.len() != 6 || !b.iter().chain(off.iter()).all(|x| x.is_finite()) {
        return None;
    }
    Some(OrientedBox {
        min: [off[1], off[3], off[5]],
        max: [off[0], off[2], off[4]],
        origin,
        basis: [[b[0], b[1], b[2]], [b[3], b[4], b[5]], [b[6], b[7], b[8]]],
        active: boolean(bs.get("m_isOn")).unwrap_or(false),
    })
}

/// The view family from the view type's `m_systemFamilyIdx`.
fn family_kind(n: i64) -> Option<ViewKind> {
    Some(match n {
        102 => ViewKind::ThreeD,
        103 => ViewKind::Other("Walkthrough".into()),
        105 => ViewKind::Other("Schedule".into()),
        107 => ViewKind::Other("Sheet".into()),
        108 => ViewKind::Other("Drafting".into()),
        109 => ViewKind::FloorPlan,
        110 => ViewKind::AreaPlan,
        111 => ViewKind::CeilingPlan,
        112 => ViewKind::Section,
        113 => ViewKind::Detail,
        114 => ViewKind::Elevation,
        117 => ViewKind::Other("Legend".into()),
        120 => ViewKind::StructuralPlan,
        _ => return None,
    })
}

fn view_kind(doc: &Document, o: &ObjectView, viewer: Option<&ObjectView>, families: &mut HashMap<u64, Option<i64>>) -> ViewKind {
    let family = element(o.get("m_dbViewTypeId")).and_then(|t| {
        *families.entry(t).or_insert_with(|| root(doc, t).and_then(|t| t.get("m_systemFamilyIdx").and_then(|v| v.as_i64())))
    });
    if let Some(k) = family.and_then(family_kind) {
        // a view type of another family than the class can hold is not trusted
        let fits = match o.class.as_str() {
            "DBView3d" => matches!(k, ViewKind::ThreeD) || k == ViewKind::Other("Walkthrough".into()),
            "DBViewPlan" => k.is_plan() || k == ViewKind::Detail,
            "DBViewSection" => matches!(k, ViewKind::Section | ViewKind::Elevation | ViewKind::Detail),
            _ => true,
        };
        if fits {
            return k;
        }
    }
    match o.class.as_str() {
        "DBView3d" => ViewKind::ThreeD,
        "DBViewPlan" => match o.get("m_planViewType").and_then(|v| v.as_i64()) {
            Some(2) => ViewKind::CeilingPlan,
            _ => ViewKind::FloorPlan,
        },
        "DBViewSection" => match o.get("m_sectionViewType").and_then(|v| v.as_i64()) {
            Some(0) => ViewKind::Section,
            Some(1 | 2) => ViewKind::Elevation,
            Some(3) => ViewKind::Detail,
            _ if viewer.is_some_and(|v| v.class == "InteriorElevArrow") => ViewKind::Elevation,
            _ => ViewKind::Section,
        },
        "DBViewDrafting" => ViewKind::Other("Sheet".into()),
        "DBViewAnonDrafting" => ViewKind::Other("Drafting".into()),
        "DBViewLegend" => ViewKind::Other("Legend".into()),
        "DBViewSchedule" => ViewKind::Other("Schedule".into()),
        "DBViewProject" => ViewKind::Other("ProjectBrowser".into()),
        "RbsDbViewSystemNavigator" => ViewKind::Other("SystemBrowser".into()),
        "DBViewRendering" => ViewKind::Other("Rendering".into()),
        c if c.starts_with("DBViewGraphSched") => ViewKind::Other("Schedule".into()),
        c => match family {
            Some(n) => ViewKind::Other(format!("family {n}")),
            None => ViewKind::Other(c.to_string()),
        },
    }
}

/// An elevation, `None` for the ±1e9 "unlimited" marker.
fn limited(v: Option<&FieldValue>) -> Option<f64> {
    v?.as_f64().filter(|x| x.is_finite() && x.abs() < 1e8)
}

fn view_info(doc: &Document, o: &ObjectView, families: &mut HashMap<u64, Option<i64>>) -> ViewInfo {
    let viewer = element(o.get("m_viewerId")).and_then(|v| root(doc, v));
    let kind = view_kind(doc, o, viewer.as_ref(), families);
    let is_template = boolean(o.get("m_isTemplate")).unwrap_or(false);
    let owned = element(o.get("m_famId")).is_some() || element(o.get("m_unplacedOwnerId")).is_some();
    let name = o.get("m_viewName").and_then(|v| v.as_str()).unwrap_or_default().to_string();

    let frame = viewer.as_ref().and_then(|v| bounded_space(v, v.get("m_boundedSpace")?));
    let perspective = viewer.as_ref().and_then(|v| v.get("m_projMethodType")?.as_i64()) == Some(2);
    let crop_box = frame.clone().map(|mut b| {
        b.active = viewer.as_ref().and_then(|v| v.get("m_viewerFlags")?.as_i64()).is_some_and(|f| f & 1 != 0);
        b
    });

    let mut camera = None;
    let mut section_box = None;
    if kind == ViewKind::ThreeD || o.class == "DBView3d" {
        camera = match &frame {
            Some(f) if unit(f.basis[1]) && unit(f.basis[2]) => {
                let fov = (perspective && f.min[2] > 1e-9).then(|| {
                    let (w, h) = ((f.max[0] - f.min[0]) / 2.0, (f.max[1] - f.min[1]) / 2.0);
                    [2.0 * (w / f.min[2]).atan(), 2.0 * (h / f.min[2]).atan()]
                });
                Some(ViewCamera { eye: f.origin, forward: f.basis[2], up: f.basis[1], perspective, fov })
            }
            // no viewer (a template): the view's own frame, `m_viewDir` points to the viewer
            _ => match (vec3(o.get("m_origin")), vec3(o.get("m_viewDir")), vec3(o.get("m_vertDir"))) {
                (Some(eye), Some(d), Some(up)) if unit(d) && unit(up) => {
                    Some(ViewCamera { eye, forward: [-d[0], -d[1], -d[2]], up, perspective, fov: None })
                }
                _ => None,
            },
        };
        section_box = viewer
            .as_ref()
            .and_then(|v| element(v.get("m_modelClipBoxId")))
            .and_then(|b| root(doc, b))
            .and_then(|b| bounded_space(&b, b.get("m_boundedSpace")?))
            .filter(|b| b.active);
    }

    let (mut level, mut level_name, mut cut_offset, mut view_range) = (None, None, None, None);
    if o.class == "DBViewPlan" {
        if let Some(l) = element(o.get("m_genElemId")) {
            let z = doc
                .level_elevation(l as i64)
                .or_else(|| doc.object(l).and_then(|lo| vec3(lo.path(&["m_pSurface", "m_origin"]))).map(|p| p[2]));
            level_name = root(doc, l).and_then(|lo| lo.get("m_text")?.as_str().map(str::to_string));
            if let Some(z) = z {
                level = Some((l, z));
            }
        }
        let range = ViewRange {
            top: limited(o.get("m_topClipElev")),
            cut: limited(o.get("m_cutPlaneElev")),
            bottom: limited(o.get("m_bottomClipElev")),
            depth: limited(o.get("m_levelBelowElev")),
        };
        if let (Some((_, z)), Some(c)) = (level, range.cut) {
            cut_offset = Some(c - z);
        }
        if range.top.is_some() || range.cut.is_some() || range.bottom.is_some() || range.depth.is_some() {
            view_range = Some(range);
        }
    }

    let scale = o.get("m_scale").and_then(|v| v.as_f64()).filter(|s| !perspective && *s > 0.0 && s.is_finite()).map(|s| (1.0 / s * 1e6).round() / 1e6);

    ViewInfo {
        id: o.id,
        name,
        kind,
        class: o.class.clone(),
        camera,
        section_box,
        crop_box,
        level,
        level_name,
        cut_offset,
        view_range,
        scale,
        is_template,
        owned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvt_model::object::{FieldView, NestedObject};

    fn field(name: &str, value: FieldValue) -> FieldView {
        FieldView { name: name.into(), ty: String::new(), owner: String::new(), value }
    }

    fn list(v: &[f64]) -> FieldValue {
        FieldValue::List(v.iter().map(|&x| FieldValue::Float(x)).collect())
    }

    /// 043 'RE Iso' section box (ModelClipBox 2799): local = world, min (−1000, −1000, −500) mm,
    /// max (4000, 1000, 2500) mm, on.
    #[test]
    fn bounded_space_bounds() {
        let ft = |mm: f64| mm / 304.8;
        let bs = NestedObject {
            class: "BoundedSpace".into(),
            fields: vec![
                field("m_orig", list(&[0.0, 0.0, 0.0])),
                field("m_basis", FieldValue::List(vec![list(&[1.0, 0.0, 0.0]), list(&[0.0, 1.0, 0.0]), list(&[0.0, 0.0, 1.0])])),
                field(
                    "m_boundOffset",
                    FieldValue::List(vec![list(&[ft(4000.0), ft(-1000.0)]), list(&[ft(1000.0), ft(-1000.0)]), list(&[ft(2500.0), ft(-500.0)])]),
                ),
                field("m_isOn", FieldValue::Bool(true)),
            ],
        };
        let o = ObjectView {
            id: 2799,
            class: "ModelClipBox".into(),
            parents: vec![],
            category: None,
            complete: true,
            error: None,
            fields: vec![field("m_boundedSpace", FieldValue::Object(bs))],
            objects: vec![],
            header: None,
        };
        let b = bounded_space(&o, o.get("m_boundedSpace").unwrap()).unwrap();
        assert!(b.active);
        assert_eq!(b.min, [ft(-1000.0), ft(-1000.0), ft(-500.0)]);
        assert_eq!(b.max, [ft(4000.0), ft(1000.0), ft(2500.0)]);
        assert_eq!(b.world_bounds(), (b.min, b.max));
    }

    #[test]
    fn rotated_box_corners() {
        // a box turned 90° about Z: local x runs along world y
        let b = OrientedBox {
            min: [0.0, 0.0, 0.0],
            max: [2.0, 1.0, 1.0],
            origin: [10.0, 0.0, 0.0],
            basis: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            active: true,
        };
        assert_eq!(b.world_bounds(), ([9.0, 0.0, 0.0], [10.0, 2.0, 1.0]));
    }

    #[test]
    fn view_families() {
        assert_eq!(family_kind(102), Some(ViewKind::ThreeD));
        assert_eq!(family_kind(120), Some(ViewKind::StructuralPlan));
        assert!(family_kind(107).is_some_and(|k| !k.is_model_view()));
        assert!(family_kind(999).is_none());
        assert!(ViewKind::AreaPlan.is_plan() && !ViewKind::Section.is_plan());
    }

    /// With `RVT_SAMPLE_043=<rvt100/043_3d_aanzichten.rvt>`: both 3D views exact against the API
    /// (gt_s709_views.json, `View3D.GetOrientation()`, `IsPerspective`), the section box of
    /// 'RE Iso' on and that of 'RE Camera' off.
    #[test]
    fn sample_043() {
        let Some(p) = std::env::var_os("RVT_SAMPLE_043") else { return };
        let doc = Document::open(p).unwrap();
        let v = views(&doc);
        let get = |id: u64| v.iter().find(|x| x.id == id).unwrap();
        let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        let iso = get(2800);
        let c = iso.camera.as_ref().unwrap();
        assert_eq!((iso.name.as_str(), &iso.kind, c.perspective), ("RE Iso", &ViewKind::ThreeD, false));
        assert!(close(c.eye, [29.707330761544483, -14.707330761544483, 14.707330761544483]));
        assert!(close(c.forward, [-0.5773502691896258, 0.5773502691896258, -0.5773502691896258]));
        assert!(close(c.up, [-0.408248290463863, 0.408248290463863, 0.816496580927726]));
        assert!(iso.section_box.is_some());
        let cam = get(2811);
        let c = cam.camera.as_ref().unwrap();
        assert!(c.perspective && cam.section_box.is_none());
        assert!(close(c.eye, [-26.246719160104988, -26.246719160104988, 16.404199475065617]));
        assert!(close(c.forward, [0.6337242505244779, 0.6337242505244779, -0.4436069753671345]));
        let plan = get(1979);
        assert_eq!((&plan.kind, plan.level.map(|l| l.0)), (&ViewKind::FloorPlan, Some(1959)));
        assert!((plan.cut_offset.unwrap() - 1200.0 / 304.8).abs() < 1e-9);
    }
}
