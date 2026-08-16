//! The declaration of one Shadow of Chernobyl map port.
//!
//! Like the other porter manifests this file records *intent*, not procedure.
//! Everything the converter cannot derive from the source — which v0.100b
//! template stands in for an X-Ray shader family, where the player spawns,
//! which target project the generated pair is cloned from — is declared here
//! and validated, so a review has one place to look.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::convert::render_material::RenderFamily;

/// A v0.100b resource this port clones instead of synthesising.
///
/// Cloning is not laziness: the target schemas carry fields whose semantics are
/// not recovered, and a template preserves them exactly while the converter
/// rewrites only what it understands.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Templates {
    /// A rendered `.model/settings` resource.
    pub model_settings: String,
    /// A `.model/render/export_properties` resource.
    pub model_render_properties: String,
    /// One render surface's `export_properties`.
    pub surface_properties: String,
    /// A collision-only `.model/settings` resource.
    pub collision_settings: String,
    /// A `.options` resource for a block-compressed texture.
    pub texture_options: String,
    /// A client project to clone the generated one from.
    pub client_project: String,
    /// A server project to clone the generated one from.
    pub server_project: String,
}

/// Where inside a cloned project the converter writes its own data.
///
/// The paths are declared rather than assumed because the project schema is
/// build-specific; the converter checks that each path already exists in the
/// template and has the expected shape before writing.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProjectBindings {
    /// Dotted path of the field holding the project's own name.
    pub project_name_path: String,
    /// Dotted path of the indexed array of visual instances.
    pub visual_array_path: String,
    /// Dotted path of the indexed array of collision objects.
    pub collision_array_path: String,
    /// Dotted path, inside one visual/collision entry, of its model reference.
    pub instance_model_path: String,
    /// Dotted path of the server project's own name.
    pub server_project_name_path: String,
    /// Dotted path of the server respawn array.
    pub server_respawn_array_path: String,
    /// Dotted path, inside one respawn entry, of its position.
    pub respawn_position_path: String,

    /// Dotted path of the indexed array of sky ambient-occlusion volumes.
    ///
    /// Each entry is a box that says which patch of world its baked occlusion
    /// texture covers. Inherited from the template it describes the template's
    /// map, and anything outside it receives no sky light at all — which reads
    /// as a hard horizontal plane with everything below it black.
    pub sky_ao_array_path: Option<String>,
    /// Dotted path of the indexed array of ambient volumes: same box shape,
    /// same failure, scaling ambient light instead of supplying sky light.
    pub ambient_volume_array_path: Option<String>,
    /// Dotted path of the indexed array of environment probes.
    ///
    /// Unlike the two volume kinds a probe cannot be re-derived: it needs a
    /// cubemap captured in this map, and the port has none. Declaring the path
    /// clears the array so the scene falls back to the global environment,
    /// which is flatter than correct probes but never black.
    pub environment_probe_array_path: Option<String>,

    /// Dotted path, inside one volume entry, of its position.
    pub volume_position_path: Option<String>,
    /// Dotted paths, inside one volume entry, of its three side lengths.
    ///
    /// Order is `(x, y, z)` by *effect*, not by field name. v0.100b builds the
    /// box with `set_scale(float3(width, depth, height))`, so the field called
    /// `depth` drives the vertical axis and the one called `height` drives Z.
    /// Declaring them positionally keeps that trap out of the converter.
    pub volume_size_paths: Option<[String; 3]>,
}

/// One Shadow of Chernobyl map port.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SocMapManifest {
    pub format_version: u32,
    /// Stable identifier recorded in the build manifest.
    pub asset_id: String,
    /// Source level directory under `gamedata/levels`, e.g. `l01_escape`.
    pub source_level: String,
    /// Resource namespace for every generated resource, e.g.
    /// `ported/soc/cordon`.
    pub namespace: String,
    /// Base name of the generated model archives, e.g. `cordon` for
    /// `cordon_000.model`.
    pub model_prefix: String,
    /// Runtime project name, e.g. `soc_cordon`.
    pub project_name: String,
    pub templates: Templates,
    pub project_bindings: ProjectBindings,
    /// Surfaces per generated model archive.
    #[serde(default = "default_surfaces_per_model")]
    pub surfaces_per_model: usize,
    pub spawn_position: [f32; 3],
    #[serde(default)]
    pub spawn_orientation: f32,
    /// Reviewed additions to the default X-Ray shader classification.
    #[serde(default)]
    pub render_shader_families: BTreeMap<String, RenderFamily>,
    /// Reviewed additions to the default physical-material correspondence.
    #[serde(default)]
    pub collision_material_overrides: BTreeMap<String, String>,
    /// Reviewed v0.100b material template per render family.
    pub material_templates: BTreeMap<RenderFamily, String>,
    /// Name of the material property the diffuse texture binds to.
    #[serde(default = "default_diffuse_property")]
    pub diffuse_property: String,
    /// Source textures a surface may reference but the tree does not contain.
    /// Each entry names an audited replacement source texture.
    #[serde(default)]
    pub texture_aliases: BTreeMap<String, String>,
    /// Extra world units added to every side of a derived lighting volume.
    ///
    /// A volume sized exactly to the map leaves its outermost surfaces on the
    /// boundary, where a rounding difference is enough to fall outside.
    #[serde(default = "default_lighting_volume_margin")]
    pub lighting_volume_margin: f32,
    /// Accept source mip chains that continue past the last complete 4x4 block.
    /// v0.100b can substitute its visible `missing texture` fallback for those,
    /// so this must be a deliberate, recorded decision.
    #[serde(default)]
    pub allow_untrimmed_mip_chains: bool,
}

fn default_surfaces_per_model() -> usize {
    200
}

fn default_diffuse_property() -> String {
    "texture_diffuse".to_owned()
}

fn default_lighting_volume_margin() -> f32 {
    16.0
}

impl SocMapManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading SoC map manifest {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing SoC map manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!(
                "unsupported SoC map manifest format version {}",
                self.format_version
            );
        }
        for (label, value) in [
            ("asset_id", &self.asset_id),
            ("source_level", &self.source_level),
            ("namespace", &self.namespace),
            ("model_prefix", &self.model_prefix),
            ("project_name", &self.project_name),
        ] {
            if value.is_empty() {
                bail!("SoC map manifest {label} is empty");
            }
        }
        if self.namespace.starts_with('/')
            || self.namespace.ends_with('/')
            || self.namespace.contains("..")
            || self.namespace.contains('\\')
        {
            bail!("SoC map manifest namespace {:?} is not a safe resource prefix", self.namespace);
        }
        if self.source_level.contains('/') || self.source_level.contains('\\') {
            bail!(
                "SoC map manifest source_level {:?} must be one level directory name",
                self.source_level
            );
        }
        if self.surfaces_per_model == 0 {
            bail!("SoC map manifest surfaces_per_model must be positive");
        }
        if !self.spawn_position.iter().all(|value| value.is_finite())
            || !self.spawn_orientation.is_finite()
        {
            bail!("SoC map manifest spawn is not finite");
        }
        if !self.lighting_volume_margin.is_finite() || self.lighting_volume_margin < 0.0 {
            bail!("SoC map manifest lighting_volume_margin must be finite and non-negative");
        }
        // A volume array can only be resized if the converter also knows which
        // fields inside an entry carry its position and size.
        let resizes_volumes = self.project_bindings.sky_ao_array_path.is_some()
            || self.project_bindings.ambient_volume_array_path.is_some();
        if resizes_volumes
            && (self.project_bindings.volume_position_path.is_none()
                || self.project_bindings.volume_size_paths.is_none())
        {
            bail!(
                "SoC map manifest declares a lighting volume array but not \
                 volume_position_path and volume_size_paths"
            );
        }
        // Every family the classification can produce needs a template, because
        // the family is chosen from the source and cannot be declined later.
        for family in RenderFamily::ALL {
            if !self.material_templates.contains_key(family) {
                bail!(
                    "SoC map manifest has no material template for the {:?} render family",
                    family.as_str()
                );
            }
        }
        Ok(())
    }

    /// The reviewed name overrides with this manifest's additions.
    pub fn effective_name_overrides(&self) -> BTreeMap<String, RenderFamily> {
        let mut overrides = super::convert::render_material::default_name_overrides();
        overrides.extend(
            self.render_shader_families
                .iter()
                .map(|(shader, family)| (shader.clone(), *family)),
        );
        overrides
    }

    /// The reviewed default physical-material correspondence with this
    /// manifest's additions.
    pub fn effective_correspondence(&self) -> BTreeMap<String, String> {
        let mut correspondence = super::convert::game_material::default_correspondence();
        correspondence.extend(
            self.collision_material_overrides
                .iter()
                .map(|(source, target)| (source.clone(), target.clone())),
        );
        correspondence
    }

    /// Resource path of one generated model archive.
    pub fn model_path(&self, index: usize) -> String {
        format!(
            "models/{}/{}_{index:03}.model",
            self.namespace, self.model_prefix
        )
    }

    /// Resource path of the generated collision model archive.
    pub fn collision_model_path(&self) -> String {
        format!(
            "models/{}/{}_collision.model",
            self.namespace, self.model_prefix
        )
    }

    pub fn client_project_path(&self) -> String {
        format!("projects/{}/client_project", self.project_name)
    }

    pub fn server_project_path(&self) -> String {
        format!("projects/{}/server_project", self.project_name)
    }
}

#[cfg(test)]
pub(crate) fn sample_manifest() -> SocMapManifest {
    SocMapManifest {
        format_version: 1,
        asset_id: "soc-cordon-to-0100b".into(),
        source_level: "l01_escape".into(),
        namespace: "ported/soc/cordon".into(),
        model_prefix: "cordon".into(),
        project_name: "soc_cordon".into(),
        templates: Templates {
            model_settings: "models/template.model/settings".into(),
            model_render_properties: "models/template.model/render/export_properties".into(),
            surface_properties: "models/template.model/render/a/export_properties".into(),
            collision_settings: "models/collision_template.model/settings".into(),
            texture_options: "textures/template.options".into(),
            client_project: "projects/level_03/client_project".into(),
            server_project: "projects/level_03/server_project".into(),
        },
        project_bindings: ProjectBindings {
            project_name_path: "project_name".into(),
            visual_array_path: "objects.visuals".into(),
            collision_array_path: "objects.collision".into(),
            instance_model_path: "model".into(),
            server_project_name_path: "project_name".into(),
            server_respawn_array_path: "objects.respawns".into(),
            respawn_position_path: "position".into(),
            sky_ao_array_path: Some("objects.sky_ao".into()),
            ambient_volume_array_path: Some("objects.ambient".into()),
            environment_probe_array_path: Some("objects.probes".into()),
            volume_position_path: Some("position".into()),
            volume_size_paths: Some(["width".into(), "depth".into(), "height".into()]),
        },
        surfaces_per_model: 200,
        spawn_position: [0.0, 2.1951056, 0.0],
        spawn_orientation: 0.0,
        render_shader_families: BTreeMap::new(),
        collision_material_overrides: BTreeMap::new(),
        material_templates: RenderFamily::ALL
            .iter()
            .map(|family| (*family, format!("material_instances/stock/{}", family.as_str())))
            .collect(),
        lighting_volume_margin: 16.0,
        diffuse_property: "texture_diffuse".into(),
        texture_aliases: BTreeMap::new(),
        allow_untrimmed_mip_chains: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_complete_manifest() {
        sample_manifest().validate().unwrap();
    }

    #[test]
    fn names_generated_resources_from_the_namespace() {
        let manifest = sample_manifest();
        assert_eq!(
            manifest.model_path(2),
            "models/ported/soc/cordon/cordon_002.model"
        );
        assert_eq!(
            manifest.collision_model_path(),
            "models/ported/soc/cordon/cordon_collision.model"
        );
        assert_eq!(
            manifest.client_project_path(),
            "projects/soc_cordon/client_project"
        );
    }

    #[test]
    fn rejects_an_unsafe_namespace() {
        let mut manifest = sample_manifest();
        manifest.namespace = "../escape".into();
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn rejects_a_missing_material_template() {
        let mut manifest = sample_manifest();
        manifest.material_templates.remove(&RenderFamily::Foliage);
        let error = manifest.validate().unwrap_err().to_string();
        assert!(error.contains("foliage"), "{error}");
    }

    #[test]
    fn an_added_shader_family_must_also_have_a_template() {
        let mut manifest = sample_manifest();
        manifest
            .render_shader_families
            .insert("water\\water".into(), RenderFamily::Translucent);
        manifest.validate().unwrap();
        manifest.material_templates.remove(&RenderFamily::Translucent);
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn overrides_extend_the_reviewed_defaults() {
        let mut manifest = sample_manifest();
        manifest
            .collision_material_overrides
            .insert("materials\\concrete".into(), "stone".into());
        let correspondence = manifest.effective_correspondence();
        assert_eq!(correspondence["materials\\concrete"], "stone");
        assert_eq!(correspondence["materials\\asphalt"], "asphalt");
    }

    #[test]
    fn rejects_an_unknown_field() {
        let text = r#"{"format_version":1,"asset_id":"a","surprise":true}"#;
        let error = serde_json::from_str::<SocMapManifest>(text).unwrap_err().to_string();
        assert!(error.contains("surprise"), "{error}");
    }
}
