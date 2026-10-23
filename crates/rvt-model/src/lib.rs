//! Neutral, queryable building model produced by the RVT reader.
//!
//! Every object in a file is reachable as an [`ElementInfo`] (id, category, class, owner view)
//! even before a typed reader exists for it. Typed structs (`Level`, …) are added per port step.
//! Lengths are in Revit internal units (feet), as in the reference export.

use serde::Serialize;

/// Generic view of one element: what the element table says about it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ElementInfo {
    pub id: u64,
    /// Built-in category id (negative) or a project subcategory id; −1 = none.
    pub category: i32,
    /// Class tag as stored in the file's version.
    pub class_tag: Option<u16>,
    /// Owner view for view-specific elements.
    pub view: Option<i64>,
    /// Axis-aligned bounding box (xmin, ymin, zmin, xmax, ymax, zmax), if stored.
    pub bbox: Option<[f64; 6]>,
    /// Partition stream and chain index the element lives in.
    pub partition: String,
    pub chain: usize,
}

/// Document-level facts.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DocumentInfo {
    /// Revit version of the project chains (e.g. 2014, 2026).
    pub version: Option<u16>,
    /// 32 or 64 (element-id width in the file).
    pub bits: Option<u8>,
    pub element_count: usize,
}

/// Circular arc: P(t) = C + r·(cos t·X + sin t·Y), t ∈ [t0, t1] (§6, step 460).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArcCurve {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "C")]
    pub center: [f64; 3],
    pub r: f64,
    #[serde(rename = "X")]
    pub x: [f64; 3],
    #[serde(rename = "Y")]
    pub y: [f64; 3],
    pub t0: f64,
    pub t1: f64,
}

/// Wall instance (§29, steps 365–370, 460, 718, 778, 1321–1341, 1422, 1423, 2163).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Wall {
    pub id: u64,
    /// WallType id.
    #[serde(rename = "type")]
    pub wall_type: i64,
    pub base_level: i64,
    /// Top constraint level, −1 = unconnected.
    pub top_level: i64,
    pub base_offset: f64,
    pub top_offset: f64,
    pub height: Option<f64>,
    /// Location line start and end.
    pub p0: [f64; 3],
    pub p1: [f64; 3],
    /// Set for curved walls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curve: Option<ArcCurve>,
    /// Member of an in-place family.
    pub in_place: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stacked_parent: Option<u64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stacked: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub curtain_panel: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_created: Option<i64>,
    /// Present (possibly null) whenever `phase_created` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_demolished: Option<Option<i64>>,
}

/// Level (§18, steps 261/368, 1001, 1180, 1344, 1456).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Level {
    pub id: u64,
    pub name: String,
    /// Elevation = z of the datum plane origin.
    pub elevation: f64,
    pub origin: [f64; 3],
    /// Extent of the datum plane in its own (u, v) coordinates: umin, vmin, umax, vmax.
    pub uv_box: [f64; 4],
    /// LevelType element id.
    #[serde(rename = "type")]
    pub level_type: Option<u64>,
}
