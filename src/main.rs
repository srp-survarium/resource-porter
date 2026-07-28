use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use resources_db::NodeKind;
use vostok_resource_porter::dependency;
use vostok_resource_porter::weapon;
use vostok_resource_porter::{BinaryConfig, PortManifest, ResourceDb};

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
        Command::DumpConfig { file } => {
            let config = BinaryConfig::parse(&std::fs::read(file)?)?;
            println!("{:#?}", config.root);
        }
        Command::DumpDbConfig { db, path } => {
            let mut archive = ResourceDb::open(db)?;
            let config = BinaryConfig::parse(&archive.read(&path)?)?;
            println!("{:#?}", config.root);
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
    }
    Ok(())
}
