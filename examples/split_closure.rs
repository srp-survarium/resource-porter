// Walk the config closure of roots in a donor (possibly split) set and report
// which volumes must be extracted. Payloads that are not configs are leaves.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use vostok_resource_porter::{BinaryConfig, ResourceDb, dependency::resolve_candidates};
fn main() {
    let mut args = std::env::args().skip(1);
    let donor_path = args.next().unwrap();
    let target_path = args.next().unwrap();
    let roots: Vec<String> = args.collect();
    let mut donor = ResourceDb::open(&donor_path).unwrap();
    let target = ResourceDb::open(&target_path).unwrap();
    let mut queue: VecDeque<String> = roots.into_iter().collect();
    let mut seen = BTreeSet::new();
    let mut need: BTreeMap<usize, (usize, u64)> = BTreeMap::new();
    let mut files = BTreeMap::<usize, (usize, u64)>::new();
    let mut unresolved = BTreeSet::new();
    while let Some(path) = queue.pop_front() {
        if !seen.insert(path.clone()) { continue; }
        let paths: Vec<String> = if donor.is_file(&path) { vec![path.clone()] } else {
            let prefix = format!("{}/", path.trim_end_matches('/'));
            let c: Vec<String> = donor.paths_with_prefix(&prefix).filter(|p| donor.is_file(p)).map(str::to_owned).collect();
            if c.is_empty() { unresolved.insert(path.clone()); }
            c
        };
        for p in paths {
            if p != path && !seen.insert(p.clone()) { continue; }
            let v = donor.volume_of(&p).unwrap();
            let size = donor.entries()[&p].size_in_db as u64;
            let f = files.entry(v).or_default(); f.0 += 1; f.1 += size;
            let leaf = p.ends_with(".dds") || p.ends_with(".ogg") || p.ends_with("converted_model") || p.ends_with("/vertices") || p.ends_with("/indices") || p.ends_with("/face_data") || p.ends_with(".veg") || p.ends_with(".rms");
            if leaf { continue; }
            let bytes = match donor.read(&p) { Ok(b) => b, Err(_) => { let n = need.entry(v).or_default(); n.0 += 1; n.1 += size; continue; } };
            let Ok(c) = BinaryConfig::parse(&bytes) else { continue };
            let mut strings = Vec::new();
            c.root.collect_strings(&mut strings);
            for s in strings {
                for cand in resolve_candidates(s, &donor, &target) {
                    if donor.contains(&cand) || donor.paths_with_prefix(&format!("{}/", cand)).next().is_some() { queue.push_back(cand); }
                }
            }
        }
    }
    for (v, (n, b)) in &files { println!("files\t{:?}\t{n}\t{b}", donor.volumes()[*v]); }
    for (v, (n, b)) in &need { println!("NEED\t{:?}\t{n} unread configs\t{b}", donor.volumes()[*v]); }
    println!("unresolved roots/paths {}", unresolved.len());
}
