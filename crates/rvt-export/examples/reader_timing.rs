//! Wall-clock time of the document readers the scene and the schedules share.
//!
//!   cargo run --release -p rvt-export --example reader_timing -- FILE.rvt

use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("usage: reader_timing FILE.rvt");
    let doc = rvt_read::Document::open(std::path::Path::new(&path)).expect("open");
    macro_rules! time {
        ($what:expr, $e:expr) => {{
            let t = Instant::now();
            let n = $e;
            eprintln!("{:<32} {:>7.2}s", $what, t.elapsed().as_secs_f64());
            n
        }};
    }
    time!("object store", doc.object_store().map(|s| s.records.len()));
    time!("walls", doc.walls().len());
    time!("walls again", doc.walls().len());
    time!("element_phases", doc.element_phases().len());
    time!("group_definition_ids", doc.group_definition_ids().len());
    time!("secondary_option_elements", doc.secondary_option_elements().len());
    time!("instances", doc.instances().len());
    time!("inplace_ids", doc.inplace_ids().len());
    time!("saved_graphics", doc.saved_graphics().map(|s| s.bodies.len()));
    time!("symbol_family", doc.symbol_family().len());
    time!("family_chains", doc.family_chains().len());
    time!("family_solids_by_chain", doc.family_solids_by_chain().len());
    time!("family_sweeps_by_chain", doc.family_sweeps_by_chain().len());
    time!("family_blends_revolves", doc.family_blends_revolves_by_chain().len());
    time!("group_definition_members", doc.group_definition_members_by_chain().len());
    time!("type_values_by_chain", doc.type_values_by_chain().len());
    time!("family_param_associations", doc.family_param_associations().len());
    time!("floors", doc.floors().len());
    time!("roofs", doc.roofs().len());
}
