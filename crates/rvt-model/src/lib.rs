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
