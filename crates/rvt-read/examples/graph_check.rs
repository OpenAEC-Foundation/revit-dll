// Decode every record of a channel with the schema; report failures (branch open-research).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    let ch: u64 = a[2].parse().unwrap();
    let recs = rvt_core::records::file_channel(&cfb, &s, ch).expect("records");
    let (mut ok, mut bad) = (0, 0);
    let mut errs = std::collections::BTreeMap::<String, (usize, u64)>::new();
    for (id, r) in &recs {
        match rvt_core::graph::Graph::decode(&s, &r.body) {
            Ok(_) => ok += 1,
            Err(e) => {
                bad += 1;
                let m = e.to_string();
                // group by the first class::field of the trail (offsets stripped)
                let k = m.split(" in ").nth(1).map(|t| t.split(" > ").next().unwrap_or("").split('@').next().unwrap_or("").to_string()).unwrap_or_else(|| m.split(" at ").next().unwrap_or("").to_string());
                let ent = errs.entry(k).or_insert((0, *id));
                ent.0 += 1;
            }
        }
    }
    println!("channel {ch}: ok {ok} failed {bad}");
    for (k, (n, id)) in errs.iter().take(15) {
        println!("  {n} × {k} (e.g. {id})");
    }
    for id in &a[3..] {
        let id: u64 = id.parse().unwrap();
        if let Some(r) = recs.get(&id) {
            match rvt_core::graph::Graph::decode(&s, &r.body) {
                Ok(g) => {
                    let mut hist = std::collections::BTreeMap::<&str, usize>::new();
                    for i in 0..g.objects.len() {
                        *hist.entry(g.class_name(&s, i).unwrap_or("?")).or_default() += 1;
                    }
                    println!("{id}: {} objects {:?}", g.objects.len(), hist);
                }
                Err(e) => println!("{id}: {e}"),
            }
        }
    }
}
