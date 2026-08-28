//! Versioned process boundary used by the recovered Vostok world editor.
//!
//! The legacy C++/CLI editor deliberately does not link Rust code.  It starts
//! `vostok-sdk-backend` for one operation and exchanges a JSON request plus
//! JSONL progress events.  Large and ABI-sensitive data remains in files.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::{ResourceDb, dependency};

pub const PROTOCOL_VERSION: u32 = 1;
pub const PROJECT_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceProfile {
    SurvariumV0100b802,
    SurvariumV011e,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetProfile {
    SurvariumV0100b802,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VariantInput {
    pub id: String,
    pub runtime_project: String,
    pub source_profile: SourceProfile,
    pub source_db: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SdkRequest {
    Handshake {
        protocol_version: u32,
    },
    DecompileMap {
        protocol_version: u32,
        project_name: String,
        target_profile: TargetProfile,
        variants: Vec<VariantInput>,
        output: PathBuf,
    },
    BuildPreview {
        protocol_version: u32,
        project_root: PathBuf,
        output: PathBuf,
    },
    RoundtripResources {
        protocol_version: u32,
        db: PathBuf,
        resources: Vec<String>,
    },
    ValidateProject {
        protocol_version: u32,
        project_root: PathBuf,
    },
}

impl SdkRequest {
    pub fn protocol_version(&self) -> u32 {
        match self {
            Self::Handshake { protocol_version }
            | Self::DecompileMap {
                protocol_version, ..
            }
            | Self::BuildPreview {
                protocol_version, ..
            }
            | Self::RoundtripResources {
                protocol_version, ..
            }
            | Self::ValidateProject {
                protocol_version, ..
            } => *protocol_version,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SdkEvent {
    Handshake {
        protocol_version: u32,
        backend_version: String,
        capabilities: Vec<String>,
    },
    Progress {
        code: String,
        message: String,
        current: usize,
        total: usize,
    },
    Diagnostic {
        severity: DiagnosticSeverity,
        code: String,
        message: String,
        resource: Option<String>,
    },
    Artifact {
        role: String,
        path: PathBuf,
    },
    Complete {
        success: bool,
        summary: serde_json::Value,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SdkProjectManifest {
    pub format_version: u32,
    pub project_name: String,
    pub target_profile: TargetProfile,
    pub variants: Vec<ProjectVariant>,
    pub shared: SharedVariantSummary,
    pub resources: Vec<ResourceRecord>,
    pub unresolved_references: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectVariant {
    pub id: String,
    pub runtime_project: String,
    pub source_profile: SourceProfile,
    pub source_db: PathBuf,
    pub client_project_path: String,
    pub server_project_path: Option<String>,
    pub client_sha256: String,
    pub server_sha256: Option<String>,
    pub render_visuals: usize,
    pub collision_objects: usize,
    pub client_objects: usize,
    pub server_objects: usize,
    pub client_object_types: BTreeMap<String, usize>,
    pub roundtrip: Vec<RoundtripEntry>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedVariantSummary {
    pub render_visuals_in_every_variant: usize,
    pub collision_objects_in_every_variant: usize,
    pub client_objects_in_every_variant: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRecord {
    pub source_db: PathBuf,
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
    pub disposition: RecoveryDisposition,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryDisposition {
    TypedProject,
    PreservedDependency,
    OpaqueProjectObject,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoundtripEntry {
    pub resource: String,
    pub original_bytes: usize,
    pub rebuilt_bytes: usize,
    pub original_sha256: String,
    pub rebuilt_sha256: String,
    pub byte_identical: bool,
    pub first_difference: Option<usize>,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildReport {
    pub project_name: String,
    pub target_profile: TargetProfile,
    pub files: Vec<BuildEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildEntry {
    pub variant: String,
    pub resource: String,
    pub bytes: usize,
    pub sha256: String,
    pub exact_original: bool,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CanonicalConfigValue {
    Boolean(bool),
    Integer(i32),
    FloatBits(u32),
    String(String),
    Float2Bits([u32; 2]),
    Float3Bits([u32; 3]),
    Float4Bits([u32; 4]),
    Named(Vec<CanonicalNamedValue>),
    Indexed(Vec<CanonicalConfigValue>),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalNamedValue {
    pub name: String,
    pub value: CanonicalConfigValue,
}

impl From<&ConfigValue> for CanonicalConfigValue {
    fn from(value: &ConfigValue) -> Self {
        match value {
            ConfigValue::Boolean(value) => Self::Boolean(*value),
            ConfigValue::Integer(value) => Self::Integer(*value),
            ConfigValue::Float(value) => Self::FloatBits(value.to_bits()),
            ConfigValue::String(value) => Self::String(value.clone()),
            ConfigValue::Float2(value) => Self::Float2Bits(value.map(f32::to_bits)),
            ConfigValue::Float3(value) => Self::Float3Bits(value.map(f32::to_bits)),
            ConfigValue::Float4(value) => Self::Float4Bits(value.map(f32::to_bits)),
            ConfigValue::Named(values) => Self::Named(
                values
                    .iter()
                    .map(|entry| CanonicalNamedValue {
                        name: entry.name.clone(),
                        value: Self::from(&entry.value),
                    })
                    .collect(),
            ),
            ConfigValue::Indexed(values) => Self::Indexed(values.iter().map(Self::from).collect()),
        }
    }
}

impl From<&CanonicalConfigValue> for ConfigValue {
    fn from(value: &CanonicalConfigValue) -> Self {
        match value {
            CanonicalConfigValue::Boolean(value) => Self::Boolean(*value),
            CanonicalConfigValue::Integer(value) => Self::Integer(*value),
            CanonicalConfigValue::FloatBits(value) => Self::Float(f32::from_bits(*value)),
            CanonicalConfigValue::String(value) => Self::String(value.clone()),
            CanonicalConfigValue::Float2Bits(value) => Self::Float2(value.map(f32::from_bits)),
            CanonicalConfigValue::Float3Bits(value) => Self::Float3(value.map(f32::from_bits)),
            CanonicalConfigValue::Float4Bits(value) => Self::Float4(value.map(f32::from_bits)),
            CanonicalConfigValue::Named(values) => Self::Named(
                values
                    .iter()
                    .map(|entry| NamedValue {
                        name: entry.name.clone(),
                        value: Self::from(&entry.value),
                    })
                    .collect(),
            ),
            CanonicalConfigValue::Indexed(values) => {
                Self::Indexed(values.iter().map(Self::from).collect())
            }
        }
    }
}

pub fn execute(request: SdkRequest, mut emit: impl FnMut(SdkEvent)) -> Result<()> {
    if request.protocol_version() != PROTOCOL_VERSION {
        bail!(
            "unsupported SDK protocol version {}, expected {}",
            request.protocol_version(),
            PROTOCOL_VERSION
        );
    }
    match request {
        SdkRequest::Handshake { .. } => {
            emit(SdkEvent::Handshake {
                protocol_version: PROTOCOL_VERSION,
                backend_version: env!("CARGO_PKG_VERSION").into(),
                capabilities: vec![
                    "decompile-map".into(),
                    "build-preview".into(),
                    "roundtrip-resources".into(),
                    "validate-project".into(),
                    "build-source:survarium-v0100b802".into(),
                    "inspect-source:survarium-v011e".into(),
                    "target:survarium-v0100b802".into(),
                ],
            });
            emit(SdkEvent::Complete {
                success: true,
                summary: serde_json::json!({"protocol_version": PROTOCOL_VERSION}),
            });
        }
        SdkRequest::RoundtripResources { db, resources, .. } => {
            let report = roundtrip_resources(&db, &resources, &mut emit)?;
            let exact = report.iter().filter(|entry| entry.byte_identical).count();
            emit(SdkEvent::Complete {
                success: exact == report.len(),
                summary: serde_json::json!({
                    "resources": report.len(),
                    "byte_identical": exact,
                    "report": report,
                }),
            });
        }
        SdkRequest::DecompileMap {
            project_name,
            target_profile,
            variants,
            output,
            ..
        } => {
            let manifest =
                decompile_map(&project_name, target_profile, &variants, &output, &mut emit)?;
            emit(SdkEvent::Artifact {
                role: "sdk-project".into(),
                path: output.join("project.sdk.json"),
            });
            emit(SdkEvent::Complete {
                success: true,
                summary: serde_json::json!({
                    "project": manifest.project_name,
                    "variants": manifest.variants.len(),
                    "resources": manifest.resources.len(),
                    "unresolved_references": manifest.unresolved_references.len(),
                }),
            });
        }
        SdkRequest::BuildPreview {
            project_root,
            output,
            ..
        } => {
            let report = build_preview(&project_root, &output, &mut emit)?;
            emit(SdkEvent::Artifact {
                role: "preview-overlay".into(),
                path: output.clone(),
            });
            emit(SdkEvent::Complete {
                success: true,
                summary: serde_json::json!({
                    "project": report.project_name,
                    "files": report.files.len(),
                    "exact_original": report.files.iter().filter(|file| file.exact_original).count(),
                }),
            });
        }
        SdkRequest::ValidateProject { project_root, .. } => {
            let manifest = validate_project(&project_root)?;
            emit(SdkEvent::Complete {
                success: true,
                summary: serde_json::json!({
                    "project": manifest.project_name,
                    "variants": manifest.variants.len(),
                    "resources": manifest.resources.len(),
                }),
            });
        }
    }
    Ok(())
}

fn build_preview(
    project_root: &Path,
    output: &Path,
    emit: &mut impl FnMut(SdkEvent),
) -> Result<BuildReport> {
    let manifest = validate_project(project_root)?;
    ensure_buildable_profiles(&manifest)?;
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .context("preview output has no UTF-8 file name")?;
    let staging = parent.join(format!(".{name}.sdk-build-{}", std::process::id()));
    let backup = parent.join(format!(".{name}.sdk-previous-{}", std::process::id()));
    if staging.exists() || backup.exists() {
        bail!(
            "stale SDK preview staging path exists beside {}",
            output.display()
        );
    }
    fs::create_dir_all(&staging)?;

    let result = build_preview_into(project_root, &staging, &manifest, emit);
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if output.exists() {
        fs::rename(output, &backup)
            .with_context(|| format!("moving previous preview overlay {}", output.display()))?;
    }
    if let Err(error) = fs::rename(&staging, output) {
        if backup.exists() {
            let _ = fs::rename(&backup, output);
        }
        return Err(error)
            .with_context(|| format!("promoting preview overlay {}", output.display()));
    }
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .with_context(|| format!("removing replaced preview overlay {}", backup.display()))?;
    }
    Ok(report)
}

fn ensure_buildable_profiles(manifest: &SdkProjectManifest) -> Result<()> {
    let unsupported = manifest
        .variants
        .iter()
        .filter(|variant| matches!(variant.source_profile, SourceProfile::SurvariumV011e))
        .map(|variant| variant.id.as_str())
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        bail!(
            "cannot build v0.11e variants ({}) as v0.100b: the per-resource ABI adapter is not integrated into the SDK backend; use the proven build-map port manifest until that adapter is wired here",
            unsupported.join(", ")
        );
    }
    Ok(())
}

fn build_preview_into(
    project_root: &Path,
    output: &Path,
    manifest: &SdkProjectManifest,
    emit: &mut impl FnMut(SdkEvent),
) -> Result<BuildReport> {
    let mut files = Vec::new();
    for (index, variant) in manifest.variants.iter().enumerate() {
        emit(SdkEvent::Progress {
            code: "sdk.build.variant".into(),
            message: format!("building {}", variant.runtime_project),
            current: index,
            total: manifest.variants.len(),
        });
        let variant_root = project_root.join(".sdk/variants").join(&variant.id);
        files.push(build_config_resource(
            &variant.id,
            &variant.client_project_path,
            &variant_root.join("client_project.original"),
            &variant_root.join("client_project.json"),
            output,
        )?);
        if let Some(path) = &variant.server_project_path {
            files.push(build_config_resource(
                &variant.id,
                path,
                &variant_root.join("server_project.original"),
                &variant_root.join("server_project.json"),
                output,
            )?);
        }
        for role in ["client_project.lua", "server_project.lua"] {
            let source = variant_root.join(role);
            if source.is_file() {
                let path = format!("projects/{}/{role}", variant.runtime_project);
                let destination = crate::archive::safe_join(output, &path)?;
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                let bytes = fs::read(&source)?;
                fs::write(&destination, &bytes)?;
                files.push(BuildEntry {
                    variant: variant.id.clone(),
                    resource: path,
                    bytes: bytes.len(),
                    sha256: sha256(&bytes),
                    exact_original: true,
                    reason: "preserved debug/source companion".into(),
                });
            }
        }
    }
    let report = BuildReport {
        project_name: manifest.project_name.clone(),
        target_profile: manifest.target_profile,
        files,
    };
    write_json(&output.join("sdk-build-report.json"), &report)?;
    Ok(report)
}

fn build_config_resource(
    variant: &str,
    resource: &str,
    original_path: &Path,
    canonical_path: &Path,
    output: &Path,
) -> Result<BuildEntry> {
    let original =
        fs::read(original_path).with_context(|| format!("reading {}", original_path.display()))?;
    let original_config =
        BinaryConfig::parse(&original).with_context(|| format!("parsing preserved {resource}"))?;
    let canonical: CanonicalConfigValue = serde_json::from_slice(
        &fs::read(canonical_path)
            .with_context(|| format!("reading {}", canonical_path.display()))?,
    )
    .with_context(|| format!("parsing {}", canonical_path.display()))?;
    let original_canonical = CanonicalConfigValue::from(&original_config.root);
    let (bytes, exact_original, reason) = if canonical == original_canonical {
        (
            original,
            true,
            "unchanged canonical tree; preserved original binary layout".to_owned(),
        )
    } else {
        let config = BinaryConfig {
            root: ConfigValue::from(&canonical),
        };
        let bytes = config.to_bytes()?;
        let reparsed = BinaryConfig::parse(&bytes)?;
        if CanonicalConfigValue::from(&reparsed.root) != canonical {
            bail!("rebuilt config does not preserve edited canonical tree: {resource}");
        }
        (
            bytes,
            false,
            "edited canonical tree; deterministically re-encoded with target semantic types"
                .to_owned(),
        )
    };
    let destination = crate::archive::safe_join(output, resource)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&destination, &bytes)?;
    Ok(BuildEntry {
        variant: variant.into(),
        resource: resource.into(),
        bytes: bytes.len(),
        sha256: sha256(&bytes),
        exact_original,
        reason,
    })
}

fn decompile_map(
    project_name: &str,
    target_profile: TargetProfile,
    variants: &[VariantInput],
    output: &Path,
    emit: &mut impl FnMut(SdkEvent),
) -> Result<SdkProjectManifest> {
    validate_identifier("project name", project_name)?;
    if variants.is_empty() {
        bail!("a decompiled map must contain at least one variant");
    }
    let mut ids = BTreeSet::new();
    for variant in variants {
        validate_identifier("variant id", &variant.id)?;
        validate_runtime_project(&variant.runtime_project)?;
        if !ids.insert(&variant.id) {
            bail!("duplicate variant id: {}", variant.id);
        }
    }
    if output.exists() {
        bail!("SDK project output already exists: {}", output.display());
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating output parent {}", parent.display()))?;
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .context("SDK project output has no UTF-8 file name")?;
    let staging = parent.join(format!(".{file_name}.sdk-staging-{}", std::process::id()));
    if staging.exists() {
        bail!(
            "SDK staging directory already exists: {}",
            staging.display()
        );
    }
    fs::create_dir_all(&staging)?;

    let result = decompile_map_into(project_name, target_profile, variants, &staging, emit);
    match result {
        Ok(manifest) => {
            fs::rename(&staging, output).with_context(|| {
                format!(
                    "promoting SDK project {} to {}",
                    staging.display(),
                    output.display()
                )
            })?;
            Ok(manifest)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

fn decompile_map_into(
    project_name: &str,
    target_profile: TargetProfile,
    variants: &[VariantInput],
    output: &Path,
    emit: &mut impl FnMut(SdkEvent),
) -> Result<SdkProjectManifest> {
    let mut project_variants = Vec::new();
    let mut all_resources = BTreeMap::<(PathBuf, String), ResourceRecord>::new();
    let mut unresolved = BTreeSet::new();
    let mut variant_sets = Vec::new();
    let mut dependency_roots = BTreeMap::<PathBuf, BTreeSet<String>>::new();
    let mut typed_projects = BTreeSet::<(PathBuf, String)>::new();

    for (index, variant) in variants.iter().enumerate() {
        emit(SdkEvent::Progress {
            code: "sdk.decompile.variant".into(),
            message: format!("decompiling {}", variant.runtime_project),
            current: index,
            total: variants.len(),
        });
        let mut db = ResourceDb::open(&variant.source_db)?;
        let client_path = format!("projects/{}/client_project", variant.runtime_project);
        let server_candidate = format!("projects/{}/server_project", variant.runtime_project);
        let server_path = db
            .is_file(&server_candidate)
            .then_some(server_candidate.clone());
        let client_bytes = db.read(&client_path)?;
        let client = BinaryConfig::parse(&client_bytes)
            .with_context(|| format!("decoding {client_path}"))?;
        let server_bytes = server_path.as_ref().map(|path| db.read(path)).transpose()?;
        let server = server_bytes
            .as_ref()
            .map(|bytes| BinaryConfig::parse(bytes))
            .transpose()
            .with_context(|| format!("decoding {server_candidate}"))?;

        let variant_root = output.join(".sdk").join("variants").join(&variant.id);
        fs::create_dir_all(&variant_root)?;
        fs::write(variant_root.join("client_project.original"), &client_bytes)?;
        write_json(
            &variant_root.join("client_project.json"),
            &CanonicalConfigValue::from(&client.root),
        )?;
        if let (Some(bytes), Some(config)) = (&server_bytes, &server) {
            fs::write(variant_root.join("server_project.original"), bytes)?;
            write_json(
                &variant_root.join("server_project.json"),
                &CanonicalConfigValue::from(&config.root),
            )?;
        }
        for role in ["client_project.lua", "server_project.lua"] {
            let path = format!("projects/{}/{role}", variant.runtime_project);
            if db.is_file(&path) {
                fs::write(variant_root.join(role), db.read(&path)?)?;
            }
        }

        let mut roundtrip = vec![roundtrip_entry(&client_path, &client_bytes, &client)?];
        if let (Some(path), Some(bytes), Some(config)) = (&server_path, &server_bytes, &server) {
            roundtrip.push(roundtrip_entry(path, bytes, config)?);
        }
        for entry in &roundtrip {
            if !entry.byte_identical {
                emit(SdkEvent::Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    code: "sdk.roundtrip.not-byte-identical".into(),
                    message: entry.reason.clone(),
                    resource: Some(entry.resource.clone()),
                });
            }
        }

        dependency_roots
            .entry(variant.source_db.clone())
            .or_default()
            .insert(client_path.clone());
        typed_projects.insert((variant.source_db.clone(), client_path.clone()));
        if let Some(path) = &server_path {
            dependency_roots
                .entry(variant.source_db.clone())
                .or_default()
                .insert(path.clone());
            typed_projects.insert((variant.source_db.clone(), path.clone()));
        }

        let client_sets = project_value_sets(&client.root)?;
        let server_object_count = server
            .as_ref()
            .map(|config| count_server_objects(&config.root))
            .unwrap_or(0);
        variant_sets.push(client_sets.fingerprints.clone());
        project_variants.push(ProjectVariant {
            id: variant.id.clone(),
            runtime_project: variant.runtime_project.clone(),
            source_profile: variant.source_profile,
            source_db: variant.source_db.clone(),
            client_project_path: client_path,
            server_project_path: server_path,
            client_sha256: sha256(&client_bytes),
            server_sha256: server_bytes.as_ref().map(|bytes| sha256(bytes)),
            render_visuals: client_sets.render_visuals,
            collision_objects: client_sets.collision_objects,
            client_objects: client_sets.client_objects,
            server_objects: server_object_count,
            client_object_types: client_sets.object_types,
            roundtrip,
        });
    }

    for (database_index, (source_db, roots)) in dependency_roots.iter().enumerate() {
        emit(SdkEvent::Progress {
            code: "sdk.decompile.dependencies".into(),
            message: format!("scanning {}", source_db.display()),
            current: database_index,
            total: dependency_roots.len(),
        });
        let mut donor = ResourceDb::open(source_db)?;
        let mut same_build = ResourceDb::open(source_db)?;
        let dependencies = dependency::scan(&mut donor, &mut same_build, roots.iter().cloned())?;
        unresolved.extend(dependencies.unresolved_references);
        let dependency_paths = dependencies
            .donor_files
            .into_iter()
            .chain(dependencies.target_files)
            .collect::<BTreeSet<_>>();
        drop(donor);
        drop(same_build);
        let mut db = ResourceDb::open(source_db)?;
        for path in dependency_paths {
            let bytes = db.read(&path)?;
            let key = (source_db.clone(), path.clone());
            let disposition = if typed_projects.contains(&key) {
                RecoveryDisposition::TypedProject
            } else {
                RecoveryDisposition::PreservedDependency
            };
            all_resources.insert(
                key,
                ResourceRecord {
                    source_db: source_db.clone(),
                    path,
                    bytes: bytes.len(),
                    sha256: sha256(&bytes),
                    disposition,
                },
            );
        }
    }

    let shared = intersect_variant_sets(&variant_sets);
    let manifest = SdkProjectManifest {
        format_version: PROJECT_FORMAT_VERSION,
        project_name: project_name.into(),
        target_profile,
        variants: project_variants,
        shared,
        resources: all_resources.into_values().collect(),
        unresolved_references: unresolved.into_iter().collect(),
    };
    write_json(&output.join("project.sdk.json"), &manifest)?;
    write_json(
        &output.join("roundtrip-report.json"),
        &manifest
            .variants
            .iter()
            .flat_map(|variant| variant.roundtrip.iter())
            .collect::<Vec<_>>(),
    )?;
    fs::write(output.join("project.xprj"), project_xprj(&manifest))?;
    fs::create_dir_all(output.join("sources/terrain"))?;
    fs::create_dir_all(output.join("sources/sounds"))?;
    Ok(manifest)
}

fn roundtrip_resources(
    db_path: &Path,
    resources: &[String],
    emit: &mut impl FnMut(SdkEvent),
) -> Result<Vec<RoundtripEntry>> {
    if resources.is_empty() {
        bail!("roundtrip-resources requires at least one resource");
    }
    let mut db = ResourceDb::open(db_path)?;
    let mut report = Vec::with_capacity(resources.len());
    for (index, path) in resources.iter().enumerate() {
        emit(SdkEvent::Progress {
            code: "sdk.roundtrip.resource".into(),
            message: path.clone(),
            current: index,
            total: resources.len(),
        });
        let bytes = db.read(path)?;
        let config = BinaryConfig::parse(&bytes)
            .with_context(|| format!("{path} is not a supported binary config"))?;
        report.push(roundtrip_entry(path, &bytes, &config)?);
    }
    Ok(report)
}

fn roundtrip_entry(path: &str, original: &[u8], config: &BinaryConfig) -> Result<RoundtripEntry> {
    let rebuilt = config.to_bytes()?;
    let first_difference = first_difference(original, &rebuilt);
    let byte_identical = first_difference.is_none();
    Ok(RoundtripEntry {
        resource: path.into(),
        original_bytes: original.len(),
        rebuilt_bytes: rebuilt.len(),
        original_sha256: sha256(original),
        rebuilt_sha256: sha256(&rebuilt),
        byte_identical,
        first_difference,
        reason: if byte_identical {
            "decoded and re-encoded bytes are identical".into()
        } else {
            "decoded values are preserved, but original allocation/name-interning layout is not yet represented"
                .into()
        },
    })
}

fn validate_project(project_root: &Path) -> Result<SdkProjectManifest> {
    let path = project_root.join("project.sdk.json");
    let manifest: SdkProjectManifest = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    if manifest.format_version != PROJECT_FORMAT_VERSION {
        bail!(
            "unsupported SDK project format {}, expected {}",
            manifest.format_version,
            PROJECT_FORMAT_VERSION
        );
    }
    if manifest.variants.is_empty() {
        bail!("SDK project contains no variants");
    }
    validate_identifier("project name", &manifest.project_name)?;
    let mut variant_ids = BTreeSet::new();
    for variant in &manifest.variants {
        validate_identifier("variant id", &variant.id)?;
        validate_runtime_project(&variant.runtime_project)?;
        if !variant_ids.insert(variant.id.as_str()) {
            bail!("duplicate variant id: {}", variant.id);
        }
        let expected_client = format!("projects/{}/client_project", variant.runtime_project);
        if variant.client_project_path != expected_client {
            bail!(
                "client project path for {} must be {expected_client}",
                variant.id
            );
        }
        if let Some(server_path) = &variant.server_project_path {
            let expected_server = format!("projects/{}/server_project", variant.runtime_project);
            if server_path != &expected_server {
                bail!(
                    "server project path for {} must be {expected_server}",
                    variant.id
                );
            }
        }
        let root = project_root.join(".sdk/variants").join(&variant.id);
        let original = fs::read(root.join("client_project.original"))?;
        if sha256(&original) != variant.client_sha256 {
            bail!("preserved client project hash changed for {}", variant.id);
        }
        let canonical: CanonicalConfigValue =
            serde_json::from_slice(&fs::read(root.join("client_project.json"))?)?;
        validate_edited_config(&canonical, &variant.client_project_path)?;

        if let (Some(resource), Some(expected_hash)) =
            (&variant.server_project_path, &variant.server_sha256)
        {
            let original = fs::read(root.join("server_project.original"))?;
            if sha256(&original) != *expected_hash {
                bail!("preserved server project hash changed for {}", variant.id);
            }
            let canonical: CanonicalConfigValue =
                serde_json::from_slice(&fs::read(root.join("server_project.json"))?)?;
            validate_edited_config(&canonical, resource)?;
        } else if variant.server_project_path.is_some() || variant.server_sha256.is_some() {
            bail!(
                "server project path/hash pair is incomplete for {}",
                variant.id
            );
        }
    }
    Ok(manifest)
}

fn validate_edited_config(value: &CanonicalConfigValue, resource: &str) -> Result<()> {
    let config = BinaryConfig {
        root: ConfigValue::from(value),
    };
    let bytes = config
        .to_bytes()
        .with_context(|| format!("encoding edited canonical tree for {resource}"))?;
    let reparsed = BinaryConfig::parse(&bytes)
        .with_context(|| format!("reparsing edited canonical tree for {resource}"))?;
    if CanonicalConfigValue::from(&reparsed.root) != *value {
        bail!("edited canonical tree is not representable for {resource}");
    }
    Ok(())
}

struct ProjectValueSets {
    render_visuals: usize,
    collision_objects: usize,
    client_objects: usize,
    object_types: BTreeMap<String, usize>,
    fingerprints: [BTreeSet<String>; 3],
}

fn project_value_sets(root: &ConfigValue) -> Result<ProjectValueSets> {
    let render = indexed_field(root, "render_visuals")?;
    let collision = indexed_field(root, "collision_objects")?;
    let objects = indexed_field(root, "objects")?;
    let mut object_types = BTreeMap::new();
    for object in objects {
        let kind = object
            .named("game_object_type")
            .and_then(|value| match value {
                ConfigValue::String(value) => Some(value.as_str()),
                _ => None,
            })
            .unwrap_or("<missing>");
        *object_types.entry(kind.to_owned()).or_insert(0) += 1;
    }
    Ok(ProjectValueSets {
        render_visuals: render.len(),
        collision_objects: collision.len(),
        client_objects: objects.len(),
        object_types,
        fingerprints: [
            fingerprints(render)?,
            fingerprints(collision)?,
            fingerprints(objects)?,
        ],
    })
}

fn indexed_field<'a>(root: &'a ConfigValue, name: &str) -> Result<&'a [ConfigValue]> {
    match root.named(name) {
        Some(ConfigValue::Indexed(values)) => Ok(values),
        Some(_) => bail!("project field {name} is not an indexed table"),
        None => Ok(&[]),
    }
}

fn count_server_objects(root: &ConfigValue) -> usize {
    match root.named("objects") {
        Some(ConfigValue::Indexed(values)) => values.len(),
        Some(ConfigValue::Named(values)) => values.len(),
        _ => 0,
    }
}

fn fingerprints(values: &[ConfigValue]) -> Result<BTreeSet<String>> {
    values
        .iter()
        .map(|value| {
            let bytes = serde_json::to_vec(&CanonicalConfigValue::from(value))?;
            Ok(sha256(&bytes))
        })
        .collect()
}

fn intersect_variant_sets(sets: &[[BTreeSet<String>; 3]]) -> SharedVariantSummary {
    let common = |index: usize| {
        let Some(first) = sets.first() else {
            return 0;
        };
        first[index]
            .iter()
            .filter(|fingerprint| {
                sets[1..]
                    .iter()
                    .all(|set| set[index].contains(*fingerprint))
            })
            .count()
    };
    SharedVariantSummary {
        render_visuals_in_every_variant: common(0),
        collision_objects_in_every_variant: common(1),
        client_objects_in_every_variant: common(2),
    }
}

fn project_xprj(manifest: &SdkProjectManifest) -> String {
    let variants = manifest
        .variants
        .iter()
        .map(|variant| {
            format!(
                "\t\t{{ id = {}, runtime_project = {} }},\n",
                lua_string(&variant.id),
                lua_string(&variant.runtime_project)
            )
        })
        .collect::<String>();
    format!(
        "return {{\n\tsdk = {{\n\t\tformat_version = {},\n\t\tproject_name = {},\n\t\ttarget_profile = \"survarium-v0100b802\",\n\t\tvariants = {{\n{}\t\t}},\n\t}},\n\tmap_backgrounds = {{}},\n}}\n",
        PROJECT_FORMAT_VERSION,
        lua_string(&manifest.project_name),
        variants
    )
}

fn lua_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn validate_identifier(role: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        bail!("invalid {role}: {value:?}");
    }
    Ok(())
}

fn validate_runtime_project(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('/')
        || value.ends_with('/')
        || value.split('/').any(|part| {
            part.is_empty()
                || matches!(part, "." | "..")
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
    {
        bail!("invalid runtime project path: {value:?}");
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn first_difference(left: &[u8], right: &[u8]) -> Option<usize> {
    left.iter()
        .zip(right)
        .position(|(left, right)| left != right)
        .or_else(|| (left.len() != right.len()).then_some(left.len().min(right.len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_config_preserves_float_bits_and_named_order() {
        let source = ConfigValue::Named(vec![
            NamedValue {
                name: "z".into(),
                value: ConfigValue::Float(f32::from_bits(0x7fc0_1234)),
            },
            NamedValue {
                name: "a".into(),
                value: ConfigValue::Float3([
                    f32::from_bits(0x8000_0000),
                    1.0,
                    f32::from_bits(0x7f80_0000),
                ]),
            },
        ]);
        let canonical = CanonicalConfigValue::from(&source);
        let recovered = ConfigValue::from(&canonical);
        assert_eq!(CanonicalConfigValue::from(&recovered), canonical);
    }

    #[test]
    fn variant_intersection_counts_exact_values() {
        let sets = vec![
            [
                BTreeSet::from(["a".into(), "b".into()]),
                BTreeSet::from(["c".into()]),
                BTreeSet::from(["d".into(), "e".into()]),
            ],
            [
                BTreeSet::from(["b".into(), "x".into()]),
                BTreeSet::from(["c".into()]),
                BTreeSet::from(["d".into()]),
            ],
        ];
        let summary = intersect_variant_sets(&sets);
        assert_eq!(summary.render_visuals_in_every_variant, 1);
        assert_eq!(summary.collision_objects_in_every_variant, 1);
        assert_eq!(summary.client_objects_in_every_variant, 1);
    }

    #[test]
    fn request_rejects_unknown_fields() {
        let request = serde_json::json!({
            "operation": "handshake",
            "protocol_version": 1,
            "surprise": true,
        });
        assert!(serde_json::from_value::<SdkRequest>(request).is_err());
    }

    #[test]
    fn validates_safe_names() {
        assert!(validate_identifier("project", "level_03-sdk").is_ok());
        assert!(validate_identifier("project", "../level_03").is_err());
        assert!(validate_runtime_project("level_03/day").is_ok());
        assert!(validate_runtime_project("level_03/../bad").is_err());
    }

    #[test]
    fn v011e_projects_fail_closed_before_preview_build() {
        let manifest = SdkProjectManifest {
            format_version: PROJECT_FORMAT_VERSION,
            project_name: "level04".into(),
            target_profile: TargetProfile::SurvariumV0100b802,
            variants: vec![ProjectVariant {
                id: "day".into(),
                runtime_project: "level04/day".into(),
                source_profile: SourceProfile::SurvariumV011e,
                source_db: "level04.db".into(),
                client_project_path: "projects/level04/day/client_project".into(),
                server_project_path: None,
                client_sha256: String::new(),
                server_sha256: None,
                render_visuals: 0,
                collision_objects: 0,
                client_objects: 0,
                server_objects: 0,
                client_object_types: BTreeMap::new(),
                roundtrip: Vec::new(),
            }],
            shared: SharedVariantSummary::default(),
            resources: Vec::new(),
            unresolved_references: Vec::new(),
        };

        let error = ensure_buildable_profiles(&manifest)
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot build v0.11e variants (day) as v0.100b"));
    }

    #[test]
    fn edited_config_builds_semantically_instead_of_copying_original() {
        let temporary = tempfile::tempdir().unwrap();
        let original_path = temporary.path().join("original");
        let canonical_path = temporary.path().join("canonical.json");
        let output = temporary.path().join("output");
        let original = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "value".into(),
                value: ConfigValue::Integer(7),
            }]),
        };
        fs::write(&original_path, original.to_bytes().unwrap()).unwrap();
        let edited = CanonicalConfigValue::Named(vec![CanonicalNamedValue {
            name: "value".into(),
            value: CanonicalConfigValue::Integer(8),
        }]);
        write_json(&canonical_path, &edited).unwrap();

        let entry = build_config_resource(
            "day",
            "projects/test/day/client_project",
            &original_path,
            &canonical_path,
            &output,
        )
        .unwrap();

        assert!(!entry.exact_original);
        let rebuilt = BinaryConfig::parse(
            &fs::read(output.join("projects/test/day/client_project")).unwrap(),
        )
        .unwrap();
        assert_eq!(CanonicalConfigValue::from(&rebuilt.root), edited);
    }
}
