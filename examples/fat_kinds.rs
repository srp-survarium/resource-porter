// Print node-kind statistics of resource databases.
use resources_db::{Archive, NodeKind};
use std::collections::BTreeMap;
fn main() {
    for path in std::env::args().skip(1) {
        let a = Archive::open(&path).unwrap();
        let mut stats: BTreeMap<String, usize> = BTreeMap::new();
        let mut examples: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for e in a.list() {
            let k = format!("{:?}", e.kind);
            *stats.entry(k.clone()).or_default() += 1;
            let ex = examples.entry(k).or_default();
            if ex.len() < 3 { ex.push(String::from_utf8_lossy(&e.path).into_owned()); }
        }
        println!("{path}: nodes {} {:?}", a.num_nodes, stats);
        for (k, v) in examples { println!("  {k}: {v:?}"); }
    }
}
