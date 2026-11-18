//! Which walls and floors of a project carry load: the ones ticked Structural. A wall keeps
//! that as `m_isStructuralSignificant` (its structural usage is no guide: a wall that is not
//! structural keeps the usage it was made with); a floor has the parameter among its stored
//! values.

use rvt_model::FieldValue;

use crate::Document;

/// The parameter Structural of a floor.
const FLOOR_IS_STRUCTURAL: i64 = -1001954;

/// Whether wall or floor `id` carries load; `None` for anything else, and where the file does
/// not say.
pub fn bearing(doc: &Document, id: u64) -> Option<bool> {
    let o = doc.object(id)?;
    if let Some(FieldValue::Bool(structural)) = o.get("m_isStructuralSignificant") {
        return Some(*structural);
    }
    if o.class != "Floor" && !o.parents.iter().any(|p| p == "Floor") {
        return None;
    }
    let set = o.deref(o.get("m_pParamValueSetInt")?)?;
    set.get("m_paramSet")?.as_list()?.iter().filter_map(FieldValue::as_object).find_map(|p| {
        (p.get("m_paramId")?.element()? == FLOOR_IS_STRUCTURAL).then(|| p.get("m_value")?.as_i64()).flatten().map(|v| v != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With `RVT_DRAWINGS=<a project with walls and floors>`: its walls say how they are used
    /// and its floors whether they are structural.
    #[test]
    fn walls_and_floors_say_whether_they_carry() {
        let Some(path) = std::env::var_os("RVT_DRAWINGS").map(std::path::PathBuf::from).filter(|p| p.exists()) else { return };
        let doc = Document::open(&path).expect("rvt");
        let walls: Vec<Option<bool>> = doc.object_ids_of_class("SWall").into_iter().map(|id| bearing(&doc, id)).collect();
        let floors: Vec<Option<bool>> = doc.object_ids_of_class("Floor").into_iter().map(|id| bearing(&doc, id)).collect();
        eprintln!("walls {walls:?}\nfloors {floors:?}");
        assert!(!walls.is_empty() && walls.iter().all(Option::is_some), "every wall says whether it is structural");
        assert!(walls.contains(&Some(true)) && walls.contains(&Some(false)), "structural walls and one that is not");
        assert!(floors.iter().any(|f| *f == Some(true)) && floors.iter().any(|f| *f == Some(false)), "a structural floor and a finish floor");
        let level = doc.object_ids_of_class("Level").first().copied().unwrap();
        assert_eq!(bearing(&doc, level), None, "a level is neither");
    }
}
