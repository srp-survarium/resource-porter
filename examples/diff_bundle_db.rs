//! List every bundle file whose packed bytes differ from (or are absent in) a database.
//! Usage: diff_bundle_db DB BUNDLE_DIR
use sha2::{Digest, Sha256};
use std::env;
use vostok_resource_porter::ResourceDb;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut db = ResourceDb::open(&args[0])?;
    let manifest: serde_json::Value = serde_json::from_reader(std::fs::File::open(format!(
        "{}/port-manifest.json",
        args[1]
    ))?)?;
    let (mut same, mut differ, mut missing) = (0, 0, 0);
    for file in manifest["files"].as_array().unwrap() {
        let path = file["path"].as_str().unwrap();
        if !db.is_file(path) {
            println!("MISSING\t{path}");
            missing += 1;
            continue;
        }
        let got = format!("{:x}", Sha256::digest(db.read(path)?));
        if got == file["sha256"].as_str().unwrap() {
            same += 1;
        } else {
            println!("DIFFER\t{path}");
            differ += 1;
        }
    }
    eprintln!("same={same} differ={differ} missing={missing}");
    Ok(())
}
