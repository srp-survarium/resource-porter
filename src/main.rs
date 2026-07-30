use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use resources_db::NodeKind;
use vostok_resource_porter::armor;
use vostok_resource_porter::dependency;
use vostok_resource_porter::map_compare;
use vostok_resource_porter::map_port;
use vostok_resource_porter::material_conversion::MaterialCatalog;
use vostok_resource_porter::maya_binary;
use vostok_resource_porter::stk2_content;
use vostok_resource_porter::target_native_map;
use vostok_resource_porter::weapon;
use vostok_resource_porter::{
    ArmorPortManifest, BinaryConfig, MapPortManifest, PlayableLobbyManifest, PortManifest,
    ResourceDb, Stk2ContentManifest, Stk2MapSliceManifest, TargetNativeMapManifest,
};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print a stable summary of a resources.db archive.
    Inventory {
        #[arg(long)]
        db: PathBuf,
    },
    /// List virtual resource paths, optionally restricted to a prefix.
    List {
        #[arg(long)]
        db: PathBuf,
        #[arg(long, default_value = "")]
        prefix: String,
    },
    /// Print archive metadata for one virtual resource path.
    Stat {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        path: String,
    },
    /// Decode a self-describing binary config for investigation.
    DumpConfig {
        #[arg(long)]
        file: PathBuf,
    },
    /// Decode a self-describing binary config directly from a resources.db.
    DumpDbConfig {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        path: String,
    },
    /// Compare decoded project trees and independently resolved resource graphs.
    CompareMapStructures {
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        target_project: String,
        #[arg(long)]
        donor_db: PathBuf,
        #[arg(long)]
        donor_project: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Rank target-build material templates for one donor material.
    RankMaterialTemplates {
        #[arg(long)]
        donor_db: PathBuf,
        #[arg(long)]
        donor_path: String,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Inspect a Maya Binary scene's skin-cluster influences and weights.
    InspectMayaSkin {
        #[arg(long)]
        scene: PathBuf,
        /// Compiled static model archive used to measure vertex correspondence.
        #[arg(long)]
        static_model: Option<PathBuf>,
    },
    /// Extract one virtual file from a resource archive.
    Extract {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        path: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Produce a recursive best-effort dependency report for resource roots.
    Scan {
        #[arg(long)]
        donor_db: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long = "root", required = true)]
        roots: Vec<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Build a new-weapon overlay bundle from a declarative manifest.
    BuildWeapon {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        donor_db: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Build an old-schema armor bundle around a converted STK2 skinned model.
    BuildArmor {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        donor_model_archive: PathBuf,
        /// Maya Binary scene supplying skin weights for a static donor model.
        #[arg(long)]
        donor_maya_scene: Option<PathBuf>,
        /// Directory containing source TGA files declared by the armor manifest.
        #[arg(long)]
        donor_texture_dir: Option<PathBuf>,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Build three rigid STK2 weapon items and optionally replace the v0.100b lobby scene.
    BuildStk2Content {
        #[arg(long)]
        manifest: PathBuf,
        /// Root containing the extracted STK2 `exported/` and `sources/` trees.
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// Build only the weapon content and leave the target lobby project unchanged.
        #[arg(long)]
        preserve_lobby: bool,
    },
    /// Build a complete cross-version map overlay from a dependency closure.
    BuildMap {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        donor_db: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Reconstruct a newer map through explicit v0.100b ABI adapters.
    BuildTargetNativeMap {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        donor_db: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        compatibility_db: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Clone the presentation lobby into a separate multiplayer map and generate server logic.
    BuildPlayableLobby {
        #[arg(long)]
        manifest: PathBuf,
        /// Database containing the already ported lobby visuals.
        #[arg(long)]
        source_db: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Build a playable, explicitly derived map slice around converted STK2 static models.
    BuildStk2MapSlice {
        #[arg(long)]
        manifest: PathBuf,
        /// Root containing the extracted STK2 `exported/` tree.
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Compose overlays in order; later bundles supersede shared paths.
    Compose {
        #[arg(long = "bundle", required = true)]
        bundles: Vec<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Merge a bundle into a target archive and verify every emitted resource.
    Apply {
        #[arg(long)]
        base_db: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        work_dir: PathBuf,
        #[arg(long)]
        output_db: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Verify a previously packed database against a bundle manifest.
    Verify {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Validate every rendered model surface against its settings contract.
    ValidateModels {
        #[arg(long)]
        db: PathBuf,
    },
    /// Validate an additive overlay against the exact effective target view.
    ValidateOverlay {
        #[arg(long)]
        base_db: PathBuf,
        #[arg(long)]
        overlay: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Inventory { db } => {
            let archive = ResourceDb::open(db)?;
            let mut files = 0_u64;
            let mut stored_bytes = 0_u64;
            for entry in archive.entries().values() {
                if matches!(entry.kind, NodeKind::File { .. }) {
                    files += 1;
                    stored_bytes += u64::from(entry.size_in_db);
                }
            }
            println!("files={files}");
            println!("unique_paths={}", archive.entries().len());
            println!("stored_bytes={stored_bytes}");
        }
        Command::List { db, prefix } => {
            let archive = ResourceDb::open(db)?;
            for path in archive.paths_with_prefix(&prefix) {
                println!("{path}");
            }
        }
        Command::Stat { db, path } => {
            let archive = ResourceDb::open(db)?;
            let entry = archive
                .entries()
                .get(&path)
                .ok_or_else(|| anyhow::anyhow!("resource does not exist: {path}"))?;
            println!("path={path}");
            println!("kind={:?}", entry.kind);
            println!("size_in_db={}", entry.size_in_db);
            println!("uncompressed_size={}", entry.uncompressed_size);
            println!("pos_in_db={}", entry.pos_in_db);
            println!("inline_buffer_offset={:?}", entry.inline_buffer_offset);
        }
        Command::DumpConfig { file } => {
            let config = BinaryConfig::parse(&std::fs::read(file)?)?;
            println!("{:#?}", config.root);
        }
        Command::DumpDbConfig { db, path } => {
            let mut archive = ResourceDb::open(db)?;
            let config = BinaryConfig::parse(&archive.read(&path)?)?;
            println!("{:#?}", config.root);
        }
        Command::CompareMapStructures {
            target_db,
            target_project,
            donor_db,
            donor_project,
            output,
        } => {
            let report =
                map_compare::compare(&target_db, &target_project, &donor_db, &donor_project)?;
            map_compare::write_report(&report, &output)?;
            println!(
                "project_shape_differences={}",
                report.counts.project_shape_differences
            );
            println!("target_graph_nodes={}", report.counts.target_graph_nodes);
            println!("donor_graph_nodes={}", report.counts.donor_graph_nodes);
            println!("identical_resources={}", report.counts.identical_resources);
            println!(
                "changed_same_schema_resources={}",
                report.counts.changed_same_schema_resources
            );
            println!(
                "changed_schema_resources={}",
                report.counts.changed_schema_resources
            );
            println!(
                "changed_opaque_resources={}",
                report.counts.changed_opaque_resources
            );
            println!(
                "target_only_resources={}",
                report.counts.target_only_resources
            );
            println!(
                "donor_only_resources={}",
                report.counts.donor_only_resources
            );
            println!(
                "changed_resource_families={}",
                report.counts.changed_resource_families
            );
            println!("report={}", output.display());
        }
        Command::RankMaterialTemplates {
            donor_db,
            donor_path,
            target_db,
            limit,
        } => {
            let mut donor = ResourceDb::open(donor_db)?;
            let mut target = ResourceDb::open(target_db)?;
            let catalog = MaterialCatalog::load(&mut target)?;
            let donor_bytes = donor.read(&donor_path)?;
            for candidate in catalog
                .rank_templates(&donor_path, &donor_bytes)?
                .into_iter()
                .take(limit)
            {
                println!(
                    "{}\t{}\tstage_contract={}\tdomain_rank={:?}\tmismatches={}\tshared_path_tokens={}\teffects={}\tdonor_stages={}\ttarget_stages={}",
                    candidate.path,
                    candidate.effect_match,
                    candidate.stage_contract_match,
                    candidate.semantic_domain_rank,
                    candidate.feature_mismatches,
                    candidate.shared_path_tokens,
                    serde_json::to_string(&candidate.effect_contract)?,
                    serde_json::to_string(&candidate.donor_stage_selectors)?,
                    serde_json::to_string(&candidate.target_stage_selectors)?
                );
            }
        }
        Command::InspectMayaSkin {
            scene,
            static_model,
        } => {
            maya_binary::inspect_skin(&scene, static_model.as_deref())?;
        }
        Command::Extract { db, path, output } => {
            let mut archive = ResourceDb::open(db)?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&output, archive.read(&path)?)?;
            println!("extracted={}", output.display());
        }
        Command::Scan {
            donor_db,
            target_db,
            roots,
            output,
        } => {
            let mut donor = ResourceDb::open(donor_db)?;
            let mut target = ResourceDb::open(target_db)?;
            let plan = dependency::scan(&mut donor, &mut target, roots)?;
            let json = serde_json::to_string_pretty(&plan)?;
            if let Some(output) = output {
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(output, json)?;
            } else {
                println!("{json}");
            }
        }
        Command::BuildWeapon {
            manifest,
            donor_db,
            target_db,
            output,
        } => {
            let manifest = PortManifest::load(manifest)?;
            let record = weapon::build_bundle(&manifest, &target_db, &donor_db, &output)?;
            println!("asset={}", record.asset_id);
            println!("files={}", record.files.len());
            println!("weapon_dict_id={}", record.weapon_dict_id);
            println!("ammo_dict_id={}", record.ammo_dict_id);
            println!("bundle={}", output.display());
        }
        Command::BuildArmor {
            manifest,
            donor_model_archive,
            donor_maya_scene,
            donor_texture_dir,
            target_db,
            output,
        } => {
            let manifest = ArmorPortManifest::load(manifest)?;
            let record = armor::build_bundle(
                &manifest,
                &target_db,
                &donor_model_archive,
                donor_maya_scene.as_deref(),
                donor_texture_dir.as_deref(),
                &output,
            )?;
            println!("asset={}", record.asset_id);
            println!("files={}", record.files.len());
            println!("armor_dict_ids={:?}", record.armor_dict_ids);
            println!("bundle={}", output.display());
        }
        Command::BuildStk2Content {
            manifest,
            source_root,
            target_db,
            output,
            preserve_lobby,
        } => {
            let manifest = Stk2ContentManifest::load(manifest)?;
            let record = stk2_content::build_bundle(
                &manifest,
                &source_root,
                &target_db,
                &output,
                preserve_lobby,
            )?;
            println!("asset={}", record.asset_id);
            println!("files={}", record.files.len());
            println!("weapon_dict_ids={:?}", record.weapon_dict_ids);
            println!("replaced_project={}", record.replaced_project);
            println!("bundle={}", output.display());
        }
        Command::BuildMap {
            manifest,
            donor_db,
            target_db,
            output,
        } => {
            let manifest = MapPortManifest::load(manifest)?;
            let record = map_port::build_bundle(&manifest, &donor_db, &target_db, &output)?;
            println!("asset={}", record.asset_id);
            println!("map_name={}", record.map_name);
            println!("files={}", record.files.len());
            println!("shared_dependencies={}", record.shared_dependencies.len());
            println!("bundle={}", output.display());
        }
        Command::BuildTargetNativeMap {
            manifest,
            donor_db,
            target_db,
            compatibility_db,
            output,
        } => {
            let manifest = TargetNativeMapManifest::load(manifest)?;
            let record = target_native_map::build_bundle(
                &manifest,
                &donor_db,
                &target_db,
                compatibility_db.as_deref(),
                &output,
            )?;
            println!("asset={}", record.asset_id);
            println!("map_name={}", record.map_name);
            println!("models={}", record.converted_models.len());
            println!("files={}", record.files.len());
            println!("bundle={}", output.display());
        }
        Command::BuildPlayableLobby {
            manifest,
            source_db,
            target_db,
            output,
        } => {
            let manifest = PlayableLobbyManifest::load(manifest)?;
            let record =
                map_port::build_playable_lobby_bundle(&manifest, &source_db, &target_db, &output)?;
            println!("asset={}", record.asset_id);
            println!("map_name={}", record.map_name);
            println!("files={}", record.files.len());
            println!("shared_dependencies={}", record.shared_dependencies.len());
            println!("bundle={}", output.display());
        }
        Command::BuildStk2MapSlice {
            manifest,
            source_root,
            target_db,
            output,
        } => {
            let manifest = Stk2MapSliceManifest::load(manifest)?;
            let record = map_port::build_stk2_map_slice_bundle(
                &manifest,
                &source_root,
                &target_db,
                &output,
            )?;
            println!("asset={}", record.asset_id);
            println!("map_name={}", record.map_name);
            println!("files={}", record.files.len());
            println!("bundle={}", output.display());
        }
        Command::Compose { bundles, output } => {
            let record = weapon::compose_bundles(&bundles, &output)?;
            println!("asset={}", record.asset_id);
            println!("bundles={}", record.bundles.len());
            println!("files={}", record.files.len());
            println!("overwritten_paths={}", record.overwritten_paths.len());
            println!("bundle={}", output.display());
        }
        Command::Apply {
            base_db,
            bundle,
            work_dir,
            output_db,
            force,
        } => {
            let sha = weapon::apply_bundle(&base_db, &bundle, &work_dir, &output_db, force)?;
            println!("output={}", output_db.display());
            println!("sha256={sha}");
        }
        Command::Verify { db, bundle } => {
            weapon::verify_bundle(&db, &bundle)?;
            println!("verified={}", db.display());
        }
        Command::ValidateModels { db } => {
            let report = vostok_resource_porter::model_validation::validate_database(&db)?;
            println!("validated_models={}", report.models);
        }
        Command::ValidateOverlay { base_db, overlay } => {
            let report = vostok_resource_porter::model_validation::validate_effective_overlay(
                &base_db, &overlay,
            )?;
            println!("validated_models={}", report.models);
            println!("validated_overlay_configs={}", report.overlay_configs);
        }
    }
    Ok(())
}
