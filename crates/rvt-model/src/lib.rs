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
    /// Curve element id; 0 for curves read from a block without an element (pick paths), which
    /// the reference writes without an id.
    #[serde(skip_serializing_if = "is_zero")]
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
    /// Spline control points (hermite: points on the curve; nurbs: control points; step 670).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 3]>>,
    /// Hermite tangents and parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tangents: Option<Vec<[f64; 3]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<f64>>,
    /// NURBS degree, weights and knots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degree: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weights: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knots: Option<Vec<f64>>,
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
    /// Hand and facing flip (step 2257): the two bytes after `ff×4 | 00×12 | 01` before the matrix.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hand_flipped: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub facing_flipped: bool,
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

/// Extrusion of a family document as a solid (§8.11): profile in the sketch plane (O, X, Y, N),
/// swept along N from `end` to `start` (feet, family coordinates).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExtrusionSolid {
    pub id: u64,
    /// Void (cutting) form; `None` when the body is too short to say.
    pub void: Option<bool>,
    pub sketch: u64,
    pub start: f64,
    pub end: f64,
    #[serde(rename = "O")]
    pub o: [f64; 3],
    #[serde(rename = "X")]
    pub x: [f64; 3],
    #[serde(rename = "Y")]
    pub y: [f64; 3],
    #[serde(rename = "N")]
    pub n: [f64; 3],
    pub profile: Vec<Curve>,
    /// Host block of a face-based template (step 923).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_block: Option<bool>,
    /// Sketch ids referenced by the shape body, in order (step 804).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sketches: Option<Vec<u64>>,
}

/// Hosted family instance (§30, steps 374–377): host element and position along it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Hosted {
    pub id: u64,
    pub level: i64,
    /// Host element (wall, floor, ...); for a work-plane host the linked model instance, or null.
    pub host: Option<i64>,
    /// Position along the host's location line (HostParameter).
    pub host_param: f64,
    /// Sill height (steps 476–477).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sill: Option<f64>,
    /// SketchPlane when the host field is a work plane (step 377).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_plane: Option<i64>,
}

/// Sweep of a family document (§8.12): profile curves (placed in 3D on the path) swept along the
/// path curves; (X, Y, N) is the path sketch plane.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SweepSolid {
    pub id: u64,
    pub void: Option<bool>,
    pub profile: Vec<Curve>,
    pub path: Vec<Curve>,
    #[serde(rename = "X")]
    pub x: [f64; 3],
    #[serde(rename = "Y")]
    pub y: [f64; 3],
    #[serde(rename = "N")]
    pub n: [f64; 3],
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// Blend of a family document (§8.13): loft between the bottom and top profile, both in the
/// bottom sketch plane, moved by bottom_offset·N and top_offset·N.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BlendSolid {
    pub id: u64,
    pub void: Option<bool>,
    pub bottom: Vec<Curve>,
    pub top: Vec<Curve>,
    #[serde(rename = "N")]
    pub n: [f64; 3],
    pub bottom_offset: f64,
    pub top_offset: f64,
}

/// Revolution of a family document (§8.14): profile rotated about the axis A→B from start to end
/// angle (radians).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RevolveSolid {
    pub id: u64,
    pub void: Option<bool>,
    pub axis: [[f64; 3]; 2],
    pub profile: Vec<Curve>,
    pub start_angle: f64,
    pub end_angle: f64,
}

/// Old sketch-based stairs (step 1455): levels, legacy StairsType and the sketch lines by kind.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LegacyStair {
    pub id: u64,
    pub base_level: Option<i64>,
    pub top_level: Option<i64>,
    #[serde(rename = "type")]
    pub r#type: Option<i64>,
    pub base_z: Option<f64>,
    pub top_z: Option<f64>,
    pub lines: std::collections::BTreeMap<&'static str, Vec<Curve>>,
    /// Multistory top level (step 2232: the word after the top level; -1 = none).
    pub multistory_top_level: Option<i64>,
    pub multistory_top_z: Option<f64>,
}

/// Topography surface (step 1532): terrain points (the largest point list) and the lengths of the
/// other point lists in its body.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Topography {
    pub id: u64,
    pub points: Vec<[f64; 3]>,
    pub other_lists: Vec<usize>,
}

/// Element that belongs to a group type's hidden definition copy (step 2233).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GroupDefinitionMember {
    pub id: u64,
    pub group_type: i64,
}

/// Building pad (step 1672): level, phase, type, the topography it cuts and its cut surface, the
/// height offset from the level and the outline from its own sketch.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct BuildingPad {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<i64>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topo: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cut_surface: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_offset: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sketch: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<Vec<Curve>>,
}

/// Wall attached to a roof, floor or ceiling (step 2237); flag 1 = top.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WallAttachment {
    pub id: u64,
    pub target: i64,
    pub flag: u32,
}

/// Element that belongs to the content of an in-place family (step 2238).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InplaceMember {
    pub id: u64,
    pub family: i64,
}

/// Column level block (§30.5): base/top level and offsets; height = level difference with offsets.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Column {
    pub id: u64,
    pub cat: i32,
    pub base_level: i64,
    pub base_offset: f64,
    pub top_level: i64,
    pub top_offset: f64,
    pub height: Option<f64>,
}

/// Flex duct (§15.6): centre line points, end tangents, type, system and level.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FlexCurve {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    pub points: Vec<[f64; 3]>,
    pub start_tangent: [f64; 3],
    pub end_tangent: [f64; 3],
    pub system: Option<u64>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub flex_type: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
}

/// Other MEP curve (step 702): pipe, cable tray or conduit with the segment layout, or a flex pipe
/// with points.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MepCurve {
    pub id: u64,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p0: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p1: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<bool>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub segment_type: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 3]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_tangent: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_tangent: Option<[f64; 3]>,
    pub system: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
}

/// One row of a value set (§9.2): parameter, number, element reference and text.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ValueRow {
    pub param: i64,
    pub num: f64,
    pub elem: i64,
    pub text: String,
    /// Integer value (yes/no, integer parameters), the i32 after the parameter id (step 2268).
    #[serde(skip_serializing)]
    pub int: i32,
}

/// Value set of a Family or FamilySymbol body (§9.2): the type values.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ValueSet {
    pub owner: u64,
    #[serde(rename = "type")]
    pub type_name: String,
    pub rows: Vec<ValueRow>,
}

/// Wall sweep or reveal (step 719; cat −2000181 / −2000182): type, Distance, host wall and the
/// path block (step 2271: Rust port of rvtread `wall_sweeps`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WallSweep {
    pub id: u64,
    pub cat: i32,
    #[serde(rename = "type")]
    pub sweep_type: Option<i64>,
    pub distance: Option<f64>,
    pub hosts: Vec<i64>,
    pub path: Option<Curve>,
    /// Step 2272: every distinct line/arc block of the body (a sweep along several wall faces
    /// stores each segment, twice); not part of the reference output.
    #[serde(skip_serializing)]
    pub segments: Vec<Curve>,
}

/// Slab edge (step 2284; cat −2001392, class 1387): a profile swept along floor edges. Each
/// path piece is [t0][t1] then a line (origin, unit direction) or an arc (X, Y, radius, centre),
/// given here as a `Curve` with its end points; the profile lies on the right of the direction
/// (outward), its top `vertical_offset` above the edge (API SWEEP_BASE_VERT_OFFSET −1012827) and
/// `horizontal_offset` out (SWEEP_BASE_OFFSET −1012825).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SlabEdge {
    pub id: u64,
    pub edge_type: Option<u64>,
    pub host: Option<u64>,
    pub vertical_offset: f64,
    pub horizontal_offset: f64,
    pub segments: Vec<Curve>,
}

/// Top rail or handrail (step 2285; cat −2000946 / −2000947): its type, host railing and path
/// (the pieces of the path element it references, class 3140; snowdon arch 2025 handrail 621419:
/// path 621421, 10 lines, the rail's top runs on it).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Rail {
    pub id: u64,
    pub cat: i32,
    pub rail_type: Option<u64>,
    pub host: Option<u64>,
    pub path: Vec<Curve>,
}

/// Stairs run type (step 2276; StairsRunType, cat −2000919): flags and sizes (API IsMonolithic,
/// HasTreads, HasRisers, StructuralDepth, TreadThickness, RiserThickness, NosingLength).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StairRunType {
    pub id: u64,
    pub monolithic: bool,
    pub treads: bool,
    pub risers: bool,
    pub structural_depth: f64,
    pub tread_thickness: f64,
    pub riser_thickness: f64,
    pub nosing: f64,
}
