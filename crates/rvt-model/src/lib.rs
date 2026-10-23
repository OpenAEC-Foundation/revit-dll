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

/// One layer of a compound structure (§25).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Layer {
    /// None, Structure, Substrate, Insulation, Finish1, Finish2, Membrane, StructuralDeck.
    pub function: &'static str,
    /// Material element id.
    pub material: Option<i64>,
    pub width: f64,
    /// Layer participates in wrapping (LayerCapFlag).
    pub cap: bool,
    /// Deck profile family symbol, for structural deck layers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deck_profile: Option<i64>,
}

/// Type with a compound structure: wall, floor, roof or ceiling type (§25, steps 293–296, 307,
/// 319, 427, 789, 1349).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CompoundType {
    pub id: u64,
    pub cat: i32,
    pub name: Option<String>,
    pub layers: Vec<Layer>,
    /// Total thickness.
    pub width: f64,
    pub structural_index: Option<i32>,
    /// First and last core layer.
    pub core: Option<[i32; 2]>,
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

/// One curve of a sketch or curve table (§6/§7): line, arc, ellipse or full circle. Only the
/// fields of its kind are set, as in the reference export.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Curve {
    pub id: u64,
    /// "line", "arc", "ellipse" or "circle".
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p0: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p1: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t0: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t1: Option<f64>,
    #[serde(rename = "C", skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 3]>,
    #[serde(rename = "X", skip_serializing_if = "Option::is_none")]
    pub x: Option<[f64; 3]>,
    #[serde(rename = "Y", skip_serializing_if = "Option::is_none")]
    pub y: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ry: Option<f64>,
    /// Arc flag byte @I+11.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounded: Option<u8>,
    /// Ellipse layout 2 (step 1922).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<u8>,
    /// Zero-length line inside a curve table (step 467).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degenerate: Option<bool>,
}

/// Floor or ceiling (§31, steps 383–386, 393, 1323–1324).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Floor {
    pub id: u64,
    pub sketch: u64,
    pub level: i64,
    #[serde(rename = "type")]
    pub floor_type: Option<i64>,
    /// Height offset from the level.
    pub offset: f64,
    pub thickness: Option<f64>,
    pub top_z: Option<f64>,
    /// Boundary loops' curves, in table order.
    pub curves: Vec<Curve>,
}

/// Roof edge: a sketch curve with its slope definition (§32).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RoofEdge {
    #[serde(flatten)]
    pub curve: Curve,
    pub slope: Option<bool>,
    /// Slope angle in radians.
    pub angle: Option<f64>,
}

/// Footprint or extrusion roof (§32, steps 387–392, 1325, 1359–1360).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Roof {
    pub id: u64,
    /// "footprint" or "extrusion".
    pub kind: &'static str,
    pub sketch: u64,
    pub level: i64,
    #[serde(rename = "type")]
    pub roof_type: Option<i64>,
    pub offset: f64,
    pub thickness: Option<f64>,
    pub base_z: Option<f64>,
    pub edges: Vec<RoofEdge>,
    /// Extrusion roofs: extrusion start and end along the sketch plane normal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    /// Extrusion roofs: sketch plane normal (null when the frame is not found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal: Option<Option<[f64; 3]>>,
}
