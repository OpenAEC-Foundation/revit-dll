// Dump a decoded channel record (classes and fields, depth-limited) (branch open-research).
use rvt_core::graph::{Graph, Value};
fn show(s: &rvt_core::Schema, v: &Value, depth: usize) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => format!("{f:.4}"),
        Value::Str(t) => format!("{t:?}"),
        Value::Guid(_) => "guid".into(),
        Value::Ptr(p) => format!("->{}", p.map_or("null".into(), |i| i.to_string())),
        Value::Ext(t) => format!("ext{t}"),
        Value::List(l) => {
            let items: Vec<String> = l.iter().take(6).map(|x| show(s, x, depth)).collect();
            format!("[{}{}]", items.join(", "), if l.len() > 6 { format!(", …{}", l.len()) } else { String::new() })
        }
        Value::Obj(o) => {
            if depth > 2 { return format!("{}{{…}}", s.class(o.class).map_or("?", |c| &c.name)); }
            let f: Vec<String> = o.fields.iter().map(|(n, v)| format!("{n}={}", show(s, v, depth + 1))).collect();
            format!("{}{{{}}}", s.class(o.class).map_or("?", |c| &c.name), f.join(" "))
        }
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cfb = rvt_core::Cfb::open(&a[1]).expect("open");
    let s = rvt_core::Schema::read(&cfb).expect("schema");
    let recs = rvt_core::records::file_channel(&cfb, &s, a[2].parse().unwrap()).expect("records");
    let max: usize = a.get(4).and_then(|x| x.parse().ok()).unwrap_or(30);
    let r = &recs[&a[3].parse::<u64>().unwrap()];
    let g = Graph::decode(&s, &r.body).expect("decode");
    for (i, o) in g.objects.iter().enumerate().take(max) {
        let f: Vec<String> = o.fields.iter().map(|(n, v)| format!("{n}={}", show(&s, v, 1))).collect();
        println!("#{i} {} {}", s.class(o.class).map_or("?", |c| &c.name), f.join(" "));
    }
    println!("{} objects", g.objects.len());
}
