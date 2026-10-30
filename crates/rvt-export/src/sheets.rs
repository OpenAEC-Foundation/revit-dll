//! Sheets for a viewer (branch open-research, beta; open-research step 2323): every sheet with
//! its number, name, title block and the views placed on it. Read through the generic object view
//! (par C), named schema fields only:
//! - a sheet is a `DBViewDrafting` with `m_sheetNumber` (view family 107); `m_viewName` is the
//!   sheet name, `m_sheetTitleBlockId` the title block (a `FamilyInstance` of category Title
//!   Blocks, −2000280, owned by the sheet), `m_dbDrawingId` its drawing;
//! - every view has a `DBDrawing` with `m_viewports`; each `Viewport` joins a view
//!   (`m_dbViewId`) to a drawing (`m_dbDrawingId`). The first viewport of a sheet's drawing is
//!   the sheet itself (position 0); the others are the placed views with `m_viewPosition.m_xyz`
//!   (sheet feet), `m_scale` (sheet / model, 0.05 = 1:20), `m_detailNumber` and
//!   `m_labelOrigin`;
//! - the position is where the origin of the view's crop frame (its local 0, 0) lands on the
//!   sheet, so the box is `position + crop min…max × scale`. Not the box centre: 3077 R26 'BP-21'
//!   places 'Plattegrond Begane Grond' (crop −14.9…80.3 × −12.4…63.0 ft at 1:50) at
//!   (−755, 203) mm; as a centre the box would hang 27 mm below the A0 paper, from the origin it
//!   lies inside (−846…−265 × 128…587 mm). Checked by eye only, not against the API;
//! - the box is the placed view's active crop box (`views::all_views`) times the viewport
//!   scale, `None` without an active crop box (an inactive one keeps Revit's default extents);
//! - the paper size is not stored as a built-in parameter. Firms put it in title block type
//!   parameters of their own (3077 R26: 'vrije_breedte' 3.901 ft and 'vrije_hoogte' 2.759 ft, an
//!   A0 sheet, beside the text 'tekening_formaat_txt' = "A0"). Taken from the type's length
//!   parameters whose caption names a width and a height; `None` otherwise (the viewer then
//!   frames the viewports). The title block's own lines (family geometry) are not read yet.
//!
//! Coordinates are sheet feet with the title block's insertion point at the origin (x right,
//! y up). 3077 R26: that point is the lower right corner of the paper (22 sheets, 17 with views;
//! the A0 sheets have their viewports at x −1259…−50 mm, y −35…776 mm).

use std::collections::HashMap;

use rvt_model::object::{FieldValue, ObjectView};
use rvt_read::Document;
use serde::Serialize;

use crate::views::{all_views, ViewInfo, ViewKind};

/// One sheet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SheetInfo {
    pub id: u64,
    pub number: String,
    pub name: String,
    pub title_block: Option<TitleBlock>,
    /// The placed views, in the drawing's order.
    pub viewports: Vec<ViewportInfo>,
}

/// The title block of a sheet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TitleBlock {
    pub id: u64,
    /// The family type (`FamilyInstance::m_masterSymbolId`).
    pub symbol: Option<u64>,
    /// Insertion point on the sheet (feet).
    pub origin: [f64; 2],
    /// Paper width and height (feet), from the type's width and height parameters when it has
    /// them.
    pub size: Option<[f64; 2]>,
}

/// A view placed on a sheet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewportInfo {
    pub id: u64,
    pub view: u64,
    pub view_name: String,
    pub view_kind: ViewKind,
    /// Where the origin of the view's crop frame lands on the sheet (feet).
    pub position: [f64; 2],
    /// Sheet length per model length (0.05 = 1:20).
    pub scale: f64,
    /// The box on the sheet (feet, min and max): the view's active crop box times `scale`,
    /// placed at `position`.
    pub rect: Option<[[f64; 2]; 2]>,
    pub detail_number: Option<String>,
    /// Start of the view title on the sheet (feet).
    pub label_origin: Option<[f64; 2]>,
}

/// The project's sheets, by sheet number (natural order). Empty when the file has no readable
/// object records.
pub fn sheets(doc: &Document) -> Vec<SheetInfo> {
    let views: HashMap<u64, ViewInfo> = all_views(doc).into_iter().map(|v| (v.id, v)).collect();
    let mut captions = HashMap::new();
    let mut out = Vec::new();
    for id in doc.object_ids_of_class("DBViewDrafting") {
        let Some(o) = doc.object_root(id).or_else(|| doc.object(id)) else { continue };
        let Some(number) = o.get("m_sheetNumber").and_then(|v| v.as_str()) else { continue };
        if element(o.get("m_famId")).is_some() || boolean(o.get("m_isTemplate")) == Some(true) {
            continue;
        }
        let name = o.get("m_viewName").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let title_block = element(o.get("m_sheetTitleBlockId")).and_then(|tb| title_block(doc, tb, &mut captions));
        let viewports = element(o.get("m_dbDrawingId"))
            .and_then(|d| doc.object_root(d).or_else(|| doc.object(d)))
            .map(|d| {
                d.get("m_viewports")
                    .and_then(|v| v.as_list())
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|vp| viewport(doc, vp.element()? as u64, id, &views))
                    .collect()
            })
            .unwrap_or_default();
        out.push(SheetInfo { id, number: number.to_string(), name, title_block, viewports });
    }
    out.sort_by(|a, b| natural(&a.number).cmp(&natural(&b.number)).then(a.id.cmp(&b.id)));
    out
}

fn viewport(doc: &Document, id: u64, sheet: u64, views: &HashMap<u64, ViewInfo>) -> Option<ViewportInfo> {
    let o = doc.object_root(id).or_else(|| doc.object(id))?;
    let view = element(o.get("m_dbViewId"))?;
    // the sheet's own viewport
    if view == sheet {
        return None;
    }
    let info = views.get(&view);
    let position = vec3(o.path(&["m_viewPosition", "m_xyz"]))?;
    let scale = o.get("m_scale").and_then(|v| v.as_f64()).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
    // an inactive crop box keeps Revit's default extents (3077 R26: 61 m squares), no box
    let rect = info
        .and_then(|v| v.crop_box.as_ref())
        .filter(|b| b.active && b.max[0] > b.min[0] && b.max[1] > b.min[1])
        .map(|b| {
            let at = |k: usize, v: f64| position[k] + v * scale;
            [[at(0, b.min[0]), at(1, b.min[1])], [at(0, b.max[0]), at(1, b.max[1])]]
        });
    let detail_number = o.get("m_detailNumber").and_then(|v| match v {
        FieldValue::Str(s) => Some(s.clone()),
        v => v.as_i64().map(|i| i.to_string()),
    });
    Some(ViewportInfo {
        id,
        view,
        view_name: info.map(|v| v.name.clone()).unwrap_or_default(),
        view_kind: info.map(|v| v.kind.clone()).unwrap_or(ViewKind::Other("unknown".into())),
        position: [position[0], position[1]],
        scale,
        rect,
        detail_number: detail_number.filter(|d| !d.is_empty()),
        label_origin: vec3(o.get("m_labelOrigin")).filter(|p| p[0] != 0.0 || p[1] != 0.0).map(|p| [p[0], p[1]]),
    })
}

fn title_block(doc: &Document, id: u64, captions: &mut HashMap<i64, Option<String>>) -> Option<TitleBlock> {
    let o = doc.object(id)?;
    let origin = vec3(o.get("m_instOrigin")).unwrap_or([0.0; 3]);
    let symbol = element(o.get("m_masterSymbolId"));
    let size = symbol.and_then(|s| doc.object(s)).and_then(|s| paper_size(doc, &s, captions));
    Some(TitleBlock { id, symbol, origin: [origin[0], origin[1]], size })
}

/// Width and height from the family type's length parameters named after them.
fn paper_size(doc: &Document, symbol: &ObjectView, captions: &mut HashMap<i64, Option<String>>) -> Option<[f64; 2]> {
    let params = symbol.get("m_pParams").and_then(|v| symbol.deref(v))?;
    let (mut width, mut height) = (None, None);
    for np in params.get("m_params").and_then(|v| v.as_list()).unwrap_or(&[]) {
        let Some(np) = np.as_object() else { continue };
        let Some(pid) = np.get("m_paramId").and_then(|v| v.element()) else { continue };
        let Some(value) = np.get("m_value").and_then(|v| v.as_f64()).filter(|v| v.is_finite() && (0.3..20.0).contains(v)) else { continue };
        let Some(caption) = caption(doc, captions, pid) else { continue };
        let c = caption.to_lowercase();
        if ["width", "breedte", "breite", "largeur"].iter().any(|w| c.contains(w)) {
            width.get_or_insert(value);
        } else if ["height", "hoogte", "höhe", "hauteur"].iter().any(|h| c.contains(h)) {
            height.get_or_insert(value);
        }
    }
    Some([width?, height?])
}

fn caption(doc: &Document, cache: &mut HashMap<i64, Option<String>>, id: i64) -> Option<String> {
    if id <= 0 {
        return None;
    }
    cache
        .entry(id)
        .or_insert_with(|| {
            let o = doc.object(id as u64)?;
            let spec = o.path(&["m_pParamDef", "m_specTypeId", "m_typeId"]).and_then(|v| v.as_str()).unwrap_or_default();
            if !spec.contains(":length-") {
                return None;
            }
            o.path(&["m_pParamDef", "m_caption"])?.as_str().map(str::to_string)
        })
        .clone()
}

/// Sort key: digits as numbers, so "A-2" comes before "A-10".
fn natural(s: &str) -> Vec<(u8, u64, String)> {
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        if digits > 0 {
            out.push((0, rest[..digits].parse().unwrap_or(u64::MAX), String::new()));
            rest = &rest[digits..];
        } else {
            let text = rest.find(|c: char| c.is_ascii_digit()).unwrap_or(rest.len());
            out.push((1, 0, rest[..text].to_lowercase()));
            rest = &rest[text..];
        }
    }
    out
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

fn vec3(v: Option<&FieldValue>) -> Option<[f64; 3]> {
    let l = v?.as_list()?;
    let f: Vec<f64> = l.iter().filter_map(|x| x.as_f64()).collect();
    (f.len() == 3 && f.iter().all(|x| x.is_finite())).then(|| [f[0], f[1], f[2]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["A-10", "A-2", "B-1", "a-3"];
        v.sort_by_key(|s| natural(s));
        assert_eq!(v, ["A-2", "a-3", "A-10", "B-1"]);
    }

    /// With `RVT_SAMPLE_3077=<3077_model R26.rvt>`: 'CP-21 Constructietekening' with its A0 title
    /// block and seven placed views.
    #[test]
    fn sample_3077() {
        let Some(p) = std::env::var_os("RVT_SAMPLE_3077") else { return };
        let doc = Document::open(p).unwrap();
        let s = sheets(&doc);
        let cp21 = s.iter().find(|s| s.id == 567144).unwrap();
        assert_eq!((cp21.number.as_str(), cp21.name.as_str()), ("CP-21", "Constructietekening"));
        assert_eq!(cp21.viewports.len(), 7);
        let size = cp21.title_block.as_ref().and_then(|t| t.size).unwrap();
        assert!((size[0] - 1189.0 / 304.8).abs() < 1e-6 && (size[1] - 841.0 / 304.8).abs() < 1e-6, "{size:?}");
        let vp = cp21.viewports.iter().find(|v| v.id == 1309873).unwrap();
        assert_eq!((vp.view, vp.scale, vp.detail_number.as_deref()), (1309798, 0.05, Some("1")));
        // BP-21: the cropped ground floor plan lies on the A0 paper (x −1189…0, y 0…841 mm)
        let bp21 = s.iter().find(|s| s.number == "BP-21").unwrap();
        let plan = bp21.viewports.iter().find(|v| v.view == 598468).unwrap();
        let [lo, hi] = plan.rect.unwrap().map(|p| p.map(|x| x * 304.8));
        assert!(lo[0] > -1189.0 && hi[0] < 0.0 && lo[1] > 0.0 && hi[1] < 841.0, "{lo:?} {hi:?}");
    }
}
