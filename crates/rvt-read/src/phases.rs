//! The phasing of a project: the order of its phases, the phase every element is made and
//! demolished in, the override the project gives each status (halftone or not), and for every
//! view its phase and what its phase filter does with each status.

use std::collections::HashMap;

use rvt_model::FieldValue;

use crate::Document;

/// The phase of a view (its place in the project's order of phases) and its filter: per status
/// (by its number) how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewPhase {
    pub phase: u16,
    pub show: [u8; 7],
}

/// The phasing of a project.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Phasing {
    /// Per element: the phase it is made in and the one it is demolished in, as places in the
    /// project's order of phases.
    pub elements: HashMap<u64, (u16, Option<u16>)>,
    /// Per status (by its number): whether the project's override for it is halftone.
    pub halftone: [bool; 7],
    /// Per view (its element): its phase and filter.
    pub views: HashMap<u64, ViewPhase>,
}

/// The parameters of a view that name its phase filter and its phase.
const PHASE_FILTER: i64 = -1012103;
const PHASE: i64 = -1012102;

/// An element a view names in one of its parameters.
fn named(o: &rvt_model::ObjectView, parameter: i64) -> Option<i64> {
    let set = o.deref(o.get("m_pParamValueSetElementId")?)?;
    set.get("m_paramSet")?.as_list()?.iter().filter_map(FieldValue::as_object).find_map(|p| {
        (p.get("m_paramId")?.element()? == parameter).then(|| p.get("m_value")?.element()).flatten()
    })
}

/// What a phase filter does with each status, by the status's number.
fn filter(doc: &Document, id: i64) -> Option<[u8; 7]> {
    let o = doc.object(u64::try_from(id).ok()?)?;
    let list = o.get("m_phaseStatusPresentation")?.as_list()?;
    let mut show = [0u8; 7];
    for (k, v) in list.iter().take(7).enumerate() {
        show[k] = v.as_i64()? as u8;
    }
    Some(show)
}

/// The phasing of the project and of `views`; empty for a project without phases.
pub fn phasing(doc: &Document, views: &[u64]) -> Phasing {
    let mut out = Phasing::default();
    let Some(all) = doc.object_ids_of_class("AllProjectPhases").first().and_then(|id| doc.object(*id)) else { return out };
    // the phases in the project's order
    let order: HashMap<i64, u16> = all
        .get("m_phaseIds")
        .and_then(FieldValue::as_list)
        .unwrap_or_default()
        .iter()
        .filter_map(FieldValue::element)
        .enumerate()
        .map(|(k, id)| (id, k as u16))
        .collect();
    if order.is_empty() {
        return out;
    }
    // the override of each status: halftone or not
    for o in all.get("m_arrPhasingOverrides").and_then(FieldValue::as_list).unwrap_or_default().iter().filter_map(|v| all.deref(v)) {
        let status = o.get("m_elementOnPhaseStatus").and_then(FieldValue::as_i64).unwrap_or(-1);
        let halftone = o.get("m_oOverrideGraphicSettings").and_then(|v| all.deref(v)).and_then(|s| s.get("m_bHalftone")).and_then(|v| match v {
            FieldValue::Bool(b) => Some(*b),
            _ => None,
        });
        if let (Ok(status @ 0..=6), Some(halftone)) = (usize::try_from(status), halftone) {
            out.halftone[status] = halftone;
        }
    }
    for (element, (created, demolished)) in doc.element_phases() {
        if let Some(&created) = order.get(&created) {
            out.elements.insert(element, (created, demolished.and_then(|d| order.get(&d).copied())));
        }
    }
    let mut filters: HashMap<i64, Option<[u8; 7]>> = HashMap::new();
    for &view in views {
        let Some(o) = doc.object(view) else { continue };
        let (Some(phase), Some(id)) = (named(&o, PHASE).and_then(|p| order.get(&p).copied()), named(&o, PHASE_FILTER)) else { continue };
        if let Some(show) = *filters.entry(id).or_insert_with(|| filter(doc, id)) {
            out.views.insert(view, ViewPhase { phase, show });
        }
    }
    out
}
