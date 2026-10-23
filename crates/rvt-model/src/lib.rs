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

/// MEP connector of a family instance (§15.3, steps 220–221, 919).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Connector {
    #[serde(rename = "P")]
    pub position: [f64; 3],
    /// Flow direction (local Z).
    pub dir: [f64; 3],
    /// Local X (orientation of rectangular connectors).
    pub xdir: [f64; 3],
    pub index: u32,
    /// "round" or "rectangular".
    pub shape: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
}

/// Family instance (§13 placement, §16 type, §30 levels; steps 667, 1161–1162, 1327, 2001–2004,
/// 2038–2048, 2069, 2093, 2151).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Instance {
    pub id: u64,
    pub cat: i32,
    /// Rotation 3×3, row-major as stored (rows are the local X, Y, Z axes).
    #[serde(rename = "M")]
    pub rotation: Option<[f64; 9]>,
    /// Origin (insertion point).
    #[serde(rename = "O")]
    pub origin: Option<[f64; 3]>,
    /// FamilySymbol (type) id.
    #[serde(rename = "type")]
    pub symbol: Option<u64>,
    pub instance_type: Option<u64>,
    pub connectors: Vec<Connector>,
    /// View scale of annotation symbols placed in a view (step 1162).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_level: Option<i64>,
    /// "sketch_plane", "ff_anchor" or "work_plane_host".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_level_source: Option<&'static str>,
    /// Host of the work plane (ceiling, curtain panel, …) for work-plane based instances (step 2045).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_plane_host: Option<i64>,
    /// MEP system of duct and pipe terminals/accessories (present, possibly null, for those categories).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Option<u64>>,
}

/// Straight MEP curve segment: duct, pipe, cable tray or conduit (§15.1, steps 240, 1132,
/// 1330, 1467, 1469, 1818).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Segment {
    pub id: u64,
    pub p0: [f64; 3],
    pub p1: [f64; 3],
    /// Width (diameter for round segments).
    pub width: f64,
    pub height: f64,
    pub round: bool,
    /// MEPCurveType id.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub segment_type: Option<i64>,
    /// MEP system id, if exactly one is referenced.
    pub system: Option<u64>,
    /// Reference level (steps 2006/2011/2159).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
}

/// MEP system (§15.5): id and name.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MepSystem {
    pub id: u64,
    pub name: Option<String>,
}

/// Railing (§35, steps 433–445, 1373). Placement fields are anchored on the RailingType id in the
/// railing body; without a type they stay empty.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Railing {
    pub id: u64,
    /// Sketch (tag 4419) holding the path.
    pub sketch: u64,
    #[serde(rename = "type")]
    pub railing_type: Option<i64>,
    /// Host element, or −1 when the railing is free.
    pub host: Option<i64>,
    pub placement_offset: Option<f64>,
    /// Base level.
    pub level: Option<i64>,
    /// Offset of the base from the level (feet; step 2202), when the type anchor was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_offset: Option<f64>,
    /// Path curves (feet, model coordinates).
    pub path: Vec<Curve>,
    /// Values of the RailingType; present when the type was found.
    #[serde(flatten)]
    pub type_info: Option<RailingTypeInfo>,
}

/// RailingType values (§35.3–35.4, steps 440–444).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RailingTypeInfo {
    /// Railing Height (feet).
    pub height: Option<f64>,
    pub toprail_height: Option<f64>,
    pub toprail_type: Option<i64>,
}

/// Component stairs (§33, steps 401–405, 722, 1328, 1453–1454, 1489–1490). Lengths in feet.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stair {
    pub id: u64,
    pub riser: f64,
    pub tread: f64,
    pub base_level: i64,
    pub base_offset: f64,
    pub top_level: i64,
    pub top_offset: f64,
    /// Level difference (top level + offset − base level − offset), or the desired height when a
    /// level is unknown (step 722).
    pub height: f64,
    pub desired_height: f64,
    pub risers: i64,
    pub base_z: Option<f64>,
    /// StairsRun ids (tag 4039) whose body references the stairs.
    pub runs: Vec<u64>,
    /// Geometry per run id (as a string key, like the reference export).
    pub run_geometry: std::collections::BTreeMap<String, RunGeometry>,
}

/// Geometry of one stairs run (steps 446–456): sketch lines by kind ("path", "riser",
/// "boundary", "run", "landing_center") and, for component runs without a path sketch, the
/// cached walk path.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct RunGeometry {
    #[serde(flatten)]
    pub lines: std::collections::BTreeMap<&'static str, Vec<Curve>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_cached: Option<Vec<PathSegment>>,
}

/// Segment of a cached run path: a line, or a helix for spiral runs (§33.5, steps 451–456).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PathSegment {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(rename = "C", skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 3]>,
    #[serde(rename = "X", skip_serializing_if = "Option::is_none")]
    pub x: Option<[f64; 3]>,
    #[serde(rename = "Y", skip_serializing_if = "Option::is_none")]
    pub y: Option<[f64; 3]>,
    #[serde(rename = "Z", skip_serializing_if = "Option::is_none")]
    pub z: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 3]>>,
}

/// Stairs run (tag 4039, step 722, 723, 1361–1362): owning stairs, width and base/top elevation.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StairsRun {
    pub id: u64,
    pub stairs: i64,
    pub width: f64,
    pub base_elevation: f64,
    pub top_elevation: f64,
}
