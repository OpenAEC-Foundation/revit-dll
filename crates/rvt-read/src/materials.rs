//! The cut patterns of a project's materials: how a cut through an element of a material is
//! hatched. A material names a fill pattern for its cut, with a colour, and maybe one behind it;
//! a fill pattern is grids of parallel lines (feet: on paper for a drafting pattern, in the model
//! for a model pattern), or solid when it has none.

use std::collections::HashMap;

use crate::Document;

/// Parallel rows of a hatch: lines at `angle` (radians, anticlockwise from x), the first through
/// `origin`, each next one `offset` across and `shift` along; `dashes` are the lengths drawn and
/// left open in turn from the row's start (none: a full line). Metres.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Grid {
    pub angle: f64,
    pub origin: [f64; 2],
    pub shift: f64,
    pub offset: f64,
    pub dashes: Vec<f64>,
}

/// How a cut through a material is hatched.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CutPattern {
    /// The colour behind the lines; none: the paper shows.
    pub fill: Option<[u8; 3]>,
    pub line: [u8; 3],
    /// Filled in the line colour, without lines.
    pub solid: bool,
    pub grids: Vec<Grid>,
    /// The grids are sized on paper (a drafting pattern), so they grow with the drawing's scale;
    /// else in the model.
    pub on_paper: bool,
}

/// Metres per foot.
const FOOT: f64 = 0.3048;
/// A fill pattern's target that sizes it in the model rather than on paper.
const MODEL_TARGET: i64 = 1;

/// Red, green and blue of a colour as the file stores it (blue in the high byte).
pub fn rgb(color: i64) -> [u8; 3] {
    [(color & 0xff) as u8, ((color >> 8) & 0xff) as u8, ((color >> 16) & 0xff) as u8]
}

/// A fill pattern: its grids in metres, and whether they are sized on paper. No grids: solid.
pub fn fill_pattern(doc: &Document, id: i64) -> Option<(Vec<Grid>, bool)> {
    let o = doc.object(u64::try_from(id).ok().filter(|id| *id > 0)?)?;
    let pattern = o.deref(o.get("m_pFillPattern")?)?;
    let pair = |v: &rvt_model::FieldValue| {
        let l = v.as_list()?;
        Some([l.first()?.as_f64()? * FOOT, l.get(1)?.as_f64()? * FOOT])
    };
    let grids = pattern
        .get("m_gridsArr")
        .and_then(|v| v.as_list())
        .unwrap_or_default()
        .iter()
        .filter_map(|g| {
            let g = o.deref(g)?;
            let [shift, offset] = pair(g.get("m_deltas")?)?;
            let dashes = g.get("m_segs").and_then(|v| v.as_list()).unwrap_or_default().iter().filter_map(|d| d.as_f64()).map(|d| d * FOOT).collect();
            Some(Grid { angle: g.get("m_angle")?.as_f64()?, origin: g.get("m_origin").and_then(pair).unwrap_or_default(), shift, offset, dashes })
        })
        .collect();
    let on_paper = pattern.get("m_fpTarget").and_then(|v| v.as_i64()) != Some(MODEL_TARGET);
    Some((grids, on_paper))
}

/// The name of a material (a `MaterialElem`), as [`cut_patterns`] names it.
pub fn name(doc: &Document, material: i64) -> Option<String> {
    let o = doc.object(u64::try_from(material).ok()?)?;
    let name = o.get("m_pMaterial").and_then(|v| o.deref(v))?.get("m_name")?.as_str()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// The cut patterns of the project's materials, by the material's name; a material without a
/// cut pattern is left out.
pub fn cut_patterns(doc: &Document) -> HashMap<String, CutPattern> {
    let mut fills: HashMap<i64, Option<(Vec<Grid>, bool)>> = HashMap::new();
    let mut out = HashMap::new();
    for id in doc.object_ids_of_class("MaterialElem") {
        let Some(o) = doc.object(id) else { continue };
        let Some(material) = o.get("m_pMaterial").and_then(|v| o.deref(v)) else { continue };
        let Some(name) = material.get("m_name").and_then(|v| v.as_str()).filter(|n| !n.is_empty()) else { continue };
        let element = |field: &str| material.get(field).and_then(|v| v.element()).unwrap_or(-1);
        let colour = |field: &str| material.get(field).and_then(|v| v.as_i64()).map(rgb);
        let cut = element("m_cutPatternId");
        let Some((grids, on_paper)) = fills.entry(cut).or_insert_with(|| fill_pattern(doc, cut)).clone() else { continue };
        // what lies behind the lines: the background pattern's colour where the material has one
        let fill = (element("m_cutBackgroundPatternId") > 0).then(|| colour("m_cutBackgroundPatternColor")).flatten();
        let solid = grids.is_empty();
        out.insert(name.to_string(), CutPattern { fill, line: colour("m_cutPatternColor").unwrap_or([0; 3]), solid, grids, on_paper });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_has_blue_in_its_high_byte() {
        assert_eq!(rgb(0x00c08040), [0x40, 0x80, 0xc0]);
    }

    /// With `RVT_DRAWINGS=<a project with materials>`: its materials bring their cut patterns,
    /// solid or as grids of lines sized on paper.
    #[test]
    fn materials_bring_their_cut_patterns() {
        let Some(path) = std::env::var_os("RVT_DRAWINGS").map(std::path::PathBuf::from).filter(|p| p.exists()) else { return };
        let doc = Document::open(&path).expect("rvt");
        let patterns = cut_patterns(&doc);
        eprintln!("{} materials with a cut pattern", patterns.len());
        assert!(!patterns.is_empty());
        assert!(patterns.values().any(|p| p.solid), "some are solid");
        let lined = patterns.values().find(|p| !p.solid).expect("some are lines");
        assert!(lined.grids.iter().all(|g| g.angle.is_finite() && g.offset.is_finite()));
        // drafting patterns: rows a fraction of a millimetre to centimetres apart on paper
        for (name, p) in patterns.iter().filter(|(_, p)| p.on_paper && !p.solid) {
            let apart = p.grids.iter().map(|g| g.offset.abs()).filter(|o| *o > 0.0).fold(f64::INFINITY, f64::min);
            assert!(apart > 1e-4 && apart < 0.05, "{name}: rows {apart} m apart on paper");
        }
    }
}
