use anyhow::Result;
use serde_json::json;
use vostok_resource_porter::{ResourceDb, material_conversion::MaterialCatalog};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let mut donor = ResourceDb::open(&args[1])?;
    let mut target = ResourceDb::open(&args[2])?;
    let catalog = MaterialCatalog::load(&mut target)?;
    let paths: Vec<_> = donor
        .paths_with_prefix("material_instances/")
        .filter(|p| p.ends_with(".material"))
        .map(str::to_owned)
        .collect();
    let mut rows = Vec::new();
    for path in paths {
        let bytes = donor.read(&path)?;
        let Ok(ranked) = catalog.rank_templates(&path, &bytes) else {
            continue;
        };
        let eligible = ranked
            .iter()
            .find(|r| r.stage_contract_match && r.semantic_domain_rank.is_some());
        let converted = catalog.convert(&path, &bytes, None, &donor, &target);
        rows.push(json!({"path":path, "same_name_exists":target.is_file(&path), "first_eligible":eligible, "closest":ranked.first(), "converted_template":converted.as_ref().ok().map(|r|&r.template_path), "error":converted.err().map(|e|e.to_string())}));
    }
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}
