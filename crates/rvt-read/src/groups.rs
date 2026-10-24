//! Group type definition copies (step 2233): right after the header's ff block some elements hold
//! the id of a GroupType (2026 tag 1502) of the same chain (64-bit @80/86/88, 32-bit @64/70/72 by
//! class; searched @76–96 resp. @60–80). These copies are shown in no view (API, rac_basic 2026:
//! level 800333, topography 800334 and fixture 800336 of 'Array Group 1').

use std::collections::HashSet;

use rvt_core::{vtag, Chain};
use rvt_model::GroupDefinitionMember;

use crate::util::{i32_at, i64_at};

const TAG_GROUP: u16 = 1491;
const TAG_GROUP_TYPE: u16 = 1502;

pub(crate) fn group_definition_members(ch: &Chain) -> Vec<GroupDefinitionMember> {
    let gt_tag = vtag(ch.version, TAG_GROUP_TYPE);
    if gt_tag.is_none() {
        return vec![];
    }
    let g_tag = vtag(ch.version, TAG_GROUP);
    let gts: HashSet<i64> = ch.elements.iter().filter(|e| e.tag == gt_tag).map(|e| e.id as i64).collect();
    if gts.is_empty() {
        return vec![];
    }
    let bits32 = ch.bits == Some(32);
    let (w, lo, hi) = if bits32 { (4, 60, 80) } else { (8, 76, 96) };
    ch.elements
        .iter()
        .filter(|e| e.tag != gt_tag && !(g_tag.is_some() && e.tag == g_tag))
        .filter_map(|e| {
            let b = ch.body(e);
            let end = hi.min((b.len() + 1).saturating_sub(w));
            (lo..end)
                .map(|o| if bits32 { i32_at(b, o) as i64 } else { i64_at(b, o) })
                .find(|v| gts.contains(v))
                .map(|g| GroupDefinitionMember { id: e.id, group_type: g })
        })
        .collect()
}

const TAG_FAMILY: u16 = 1967;

/// Step 2238: content of an in-place family: the same header slot holds the id of a Family (tag
/// 1967) of the same chain (rac_basic 2026: 4 walls of the in-place mass 'water', shown in no view).
pub(crate) fn inplace_members(ch: &Chain) -> Vec<rvt_model::InplaceMember> {
    let f_tag = vtag(ch.version, TAG_FAMILY);
    if f_tag.is_none() {
        return vec![];
    }
    let fams: HashSet<i64> = ch.elements.iter().filter(|e| e.tag == f_tag).map(|e| e.id as i64).collect();
    if fams.is_empty() {
        return vec![];
    }
    let bits32 = ch.bits == Some(32);
    let (w, lo, hi) = if bits32 { (4, 60, 80) } else { (8, 76, 96) };
    ch.elements
        .iter()
        .filter(|e| e.tag != f_tag)
        .filter_map(|e| {
            let b = ch.body(e);
            let end = hi.min((b.len() + 1).saturating_sub(w));
            (lo..end)
                .map(|o| if bits32 { i32_at(b, o) as i64 } else { i64_at(b, o) })
                .find(|v| fams.contains(v))
                .map(|f| rvt_model::InplaceMember { id: e.id, family: f })
        })
        .collect()
}
