//! Wall-clock time of the steps the viewer runs to open a file.
//!
//!   RVT_MESH_TIMING=1 cargo run --release -p rvt-export --example load_timing -- FILE.rvt

use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("usage: load_timing FILE.rvt");
    let t = Instant::now();
    let lap = |what: &str, t0: Instant| eprintln!("{what:<12} {:>8.2}s (total {:>8.2}s)", t0.elapsed().as_secs_f64(), t.elapsed().as_secs_f64());
    let t0 = Instant::now();
    let doc = rvt_read::Document::open(std::path::Path::new(&path)).expect("open");
    lap("open", t0);
    let t0 = Instant::now();
    let scene = rvt_export::mesh::scene(&doc);
    lap("scene", t0);
    let tris: usize = scene.batches.iter().map(|b| b.indices.len() / 3).sum();
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        rvt_export::mesh::encode(&scene).hash(&mut h);
        h.finish()
    };
    eprintln!("             {} batches, {tris} triangles, scene hash {hash:016x}", scene.batches.len());
    let t0 = Instant::now();
    let views = rvt_export::views::views(&doc);
    lap("views", t0);
    let t0 = Instant::now();
    let sheets = rvt_export::sheets::sheets(&doc);
    lap("sheets", t0);
    let t0 = Instant::now();
    let schedules = rvt_export::schedules::schedules(&doc);
    lap("schedules", t0);
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{schedules:?}").hash(&mut h);
        h.finish()
    };
    eprintln!("             {} views, {} sheets, {} schedules, schedules hash {hash:016x}", views.len(), sheets.len(), schedules.len());
}
