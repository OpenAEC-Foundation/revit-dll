//! Version-specific class tags (appendix A). Readers are written against canonical 2026 tags;
//! `vtag` translates to the tag used in a chain's version.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::tags_gen::{TAG_MAP, VERSIONS32, VERSIONS64};

struct VersionTags {
    map: HashMap<u16, u16>,
    values: HashSet<u16>,
}

fn tables() -> &'static HashMap<u16, VersionTags> {
    static T: OnceLock<HashMap<u16, VersionTags>> = OnceLock::new();
    T.get_or_init(|| {
        TAG_MAP
            .iter()
            .map(|(ver, pairs)| {
                let map: HashMap<u16, u16> = pairs.iter().copied().collect();
                let values = map.values().copied().collect();
                (*ver, VersionTags { map, values })
            })
            .collect()
    })
}

/// Canonical 2026 tag `t` -> tag in `version`. `None` = unknown in this version: there is no pair
/// and the number is already another tag's value in this version (step 1277, rule N.19b).
pub fn vtag(version: Option<u16>, t: u16) -> Option<u16> {
    let Some(vt) = version.and_then(|v| tables().get(&v)) else { return Some(t) };
    if let Some(&m) = vt.map.get(&t) {
        return Some(m);
    }
    if vt.values.contains(&t) {
        return None;
    }
    Some(t)
}

pub fn version64(code: u16) -> Option<u16> {
    VERSIONS64.iter().find(|(c, _)| *c == code).map(|(_, v)| *v)
}

pub fn version32(code: u16) -> Option<u16> {
    VERSIONS32.iter().find(|(c, _)| *c == code).map(|(_, v)| *v)
}
