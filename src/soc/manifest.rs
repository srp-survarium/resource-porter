// SPDX-License-Identifier: GPL-3.0-or-later
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

use super::convert::collision::FaceDictionary;
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
    /// A post-process material to clone the generated one from.
    ///
    /// This is where `skylight_lower` and `skylight_upper` live: the world
    /// heights between which sky light ramps from nothing to full. They are
    /// authored for the template's map, so they have to be re-anchored.
    #[serde(default = "default_post_process_material")]
    pub post_process_material: String,
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

    /// Dotted path of the template's heterogeneous scene-object array.
    ///
    /// In v0.100b this is one flat array holding lights, decals, environment
    /// probes, collision geometry, sounds, vegetation, sky and fog — all
    /// discriminated by [`Self::object_type_path`]. Cloning a template project
    /// drags every one of them along, and almost all are positioned for the
    /// template's map: `projects/level_03/client_project` carries 248 objects,
    /// of which 36 are environment probes, 110 decals and 47 collision
    /// geometries, none of which belong anywhere near a converted map.
    pub objects_array_path: Option<String>,
    /// Dotted path, inside one scene object, of its type discriminator.
    pub object_type_path: Option<String>,
    /// Object types kept when cloning the template.
    ///
    /// Only types whose meaning is *global* rather than positional can be
    /// retained — sky, the environment settings, volumetric fog. Everything
    /// else is dropped, because its coordinates describe the template's map.
    /// This is the rule the STK2 port arrived at the hard way: "spatial
    /// lights, probes, and decals inherited from a convenient client-project
    /// template must be removed because their coordinates belong to the
    /// template map".
    #[serde(default)]
    pub retained_object_types: Vec<String>,
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
    /// Set by `--camera-at-spawn`, never by a manifest: the offline client
    /// inserts the player at the client project's `camera.position`, which
    /// the template inherits from another map.
    #[serde(skip)]
    pub camera_at_spawn: bool,
    /// Reviewed additions to the default X-Ray shader classification.
    #[serde(default)]
    pub render_shader_families: BTreeMap<String, RenderFamily>,
    /// Reviewed additions to the default physical-material correspondence.
    #[serde(default)]
    pub collision_material_overrides: BTreeMap<String, String>,
    /// Reviewed v0.100b material template per render family.
    pub material_templates: BTreeMap<RenderFamily, String>,
    /// Dotted path of the material field the diffuse texture binds to.
    ///
    /// v0.100b materials nest every binding under `material.g_stage.effect`,
    /// and each leaf is a `{ value, is_model_parameter }` pair, so this is a
    /// path rather than a field name.
    #[serde(default = "default_diffuse_path")]
    pub diffuse_path: String,
    /// Texture-coordinate multiplier per render family.
    ///
    /// For shaders whose detail came from the shader rather than the base
    /// texture -- water above all -- the source tiling is unusable once that
    /// shader is gone. Declared per family and recorded, because it is an
    /// approximation of the original look, not a faithful conversion.
    #[serde(default)]
    pub family_uv_scale: BTreeMap<RenderFamily, f32>,
    /// Families whose template binds its colour somewhere else.
    ///
    /// Decal and additive materials are forward-rendered rather than written
    /// into the G-buffer, so their colour lives at
    /// `material.forward.effect.texture_emissive` and the `g_stage` path does
    /// not exist on them at all.
    #[serde(default)]
    pub family_diffuse_paths: BTreeMap<RenderFamily, String>,
    /// Preserve foliage cutout/two-sided rendering but disable the target
    /// template's procedural wind when source deformation inputs are absent.
    #[serde(default)]
    pub disable_foliage_wind: bool,
    /// Dotted path of the field a converted normal map binds to, and of the
    /// flag that enables normal mapping.
    ///
    /// Shadow of Chernobyl ships a `<name>_bump` companion for 587 of its
    /// 3,743 base textures. Binding them is the largest fidelity gain
    /// available at no risk: the payload is a DDS the target already reads,
    /// and the binding is one field.
    #[serde(default = "default_normal_path")]
    pub normal_path: String,
    #[serde(default = "default_normal_enable_path")]
    pub normal_enable_path: String,
    /// Convert and bind `_bump` companions when the source has them.
    #[serde(default = "default_true")]
    pub bind_normal_maps: bool,
    /// Source textures a surface may reference but the tree does not contain.
    /// Each entry names an audited replacement source texture.
    #[serde(default)]
    pub texture_aliases: BTreeMap<String, String>,
    /// Target-build normal-map override keyed by source diffuse texture.
    ///
    /// Values omit the `textures/` prefix and extension. This keeps a source
    /// diffuse while replacing a bump map whose tangent convention is not
    /// compatible with the target material shader.
    #[serde(default)]
    pub normal_texture_overrides: BTreeMap<String, String>,
    /// Source *materials* a world references but no archive carries, each
    /// naming an audited substitute material. Used by the lilith path:
    /// F.E.A.R. retail ships a handful of dangling `.Mat00` references where
    /// its own engine shows a fallback. Keys and values are archive paths as
    /// the world names them, compared case-insensitively.
    #[serde(default)]
    pub material_aliases: BTreeMap<String, String>,
    /// Reflection probe a converted water material samples.
    ///
    /// `water/water_green` names a probe from a project this database does not
    /// ship, and a material sampling a missing texture renders the engine.s
    /// visible `missing texture` fallback.
    #[serde(default = "default_water_reflection_probe")]
    pub water_reflection_probe: String,
    /// Tiled detail texture blended over the level's own terrain shaders.
    ///
    /// X-Ray terrain is a unique colour map stretched across the whole level
    /// plus a tiled detail texture; without the second the ground is flat.
    /// `None` leaves the target material as the template had it.
    #[serde(default = "default_terrain_detail_texture")]
    pub terrain_detail_texture: Option<String>,
    /// How many times that detail texture repeats.
    #[serde(default = "default_terrain_detail_tile")]
    pub terrain_detail_tile: i32,
    /// How the generated terrain mask weights the four layers, as one texel.
    ///
    /// Build 802's `terrain_gbuffer_pass.ps` samples the mask at the *untiled*
    /// mesh UV and divides it by the sum of its channels before weighting:
    ///
    /// ```hlsl
    /// float4 mask = texture_mask.Sample(s_base, parameters.tc);
    /// mask        /= mask.x + mask.y + mask.z + mask.w;
    /// float3 diffuse = mask.x * diffuse_0.xyz + mask.y * diffuse_1.xyz + ...
    /// ```
    ///
    /// These are therefore ordinary relative weights. Converted X-Ray terrain
    /// currently binds the same tiled material to all four layers, making the
    /// result robust to both DXT mask quantisation and custom weight ratios.
    #[serde(default = "default_terrain_layer_weights")]
    pub terrain_layer_weights: [u8; 4],
    /// How many times terrain layer 0 repeats across the level.
    ///
    /// X-Ray stretches its colour map once over the whole map, so this is 1 --
    /// against the template's 60, which tiles a soil swatch. It is a shader
    /// *constant*, not one of the fourteen macros in `shaders/masks`, so it
    /// changes without asking for an uncompiled permutation.
    #[serde(default = "default_terrain_base_tile")]
    pub terrain_base_tile: i32,
    /// How many world units one repeat of a tiled terrain layer covers.
    ///
    /// The tile *count* is derived from this and the shader's own UV density,
    /// because the UV range a terrain shader covers differs per shader and per
    /// map: the template's 60 is a repeat every 3m on level_03 and every 24m on
    /// Cordon.
    #[serde(default = "default_terrain_layer_metres")]
    pub terrain_layer_metres: f32,
    /// Tiled ground texture per terrain shader, where the name misleads.
    ///
    /// Keyed by the X-Ray shader name, valued with a source texture.
    /// [`crate::soc::convert::render_material::terrain_detail_texture`] derives
    /// this from the shader's name for all 51 terrain shaders in the game; an
    /// entry here overrides it after review.
    #[serde(default)]
    pub terrain_layer_textures: BTreeMap<String, String>,
    /// Target-build normal map per terrain shader.
    ///
    /// Normally the converter binds X-Ray's `<detail>_bump` companion. This
    /// override is useful when the source normal-map tangent convention does
    /// not agree with the target terrain shader: values are existing v0.100b
    /// texture paths, without the `textures/` prefix or extension.
    #[serde(default)]
    pub terrain_layer_normal_textures: BTreeMap<String, String>,
    /// Legacy terrain brightness knob. Retained for manifest compatibility;
    /// build 802 exposes no scalar brightness control in this material, and a
    /// flat detail texture changes the hard-light blend rather than exposure.
    #[serde(default = "default_terrain_exposure")]
    pub terrain_exposure: f32,
    /// World units a decal surface is lifted along its normal.
    ///
    /// X-Ray lays posters, litter and grime exactly on their host surface and
    /// biased them at draw time; v0.100b has no material-level depth bias, so
    /// coplanar surfaces flicker. Millimetres are enough.
    #[serde(default = "default_decal_lift")]
    pub decal_lift: f32,
    /// Extra world units added to every side of a derived lighting volume.
    ///
    /// A volume sized exactly to the map leaves its outermost surfaces on the
    /// boundary, where a rounding difference is enough to fall outside.
    #[serde(default = "default_lighting_volume_margin")]
    pub lighting_volume_margin: f32,
    /// Emit the collision model and bind it into the client project.
    ///
    /// v0.100b ships no collision-only model -- all 236 of its collision models
    /// also carry render surfaces -- so a collision-only archive is a shape the
    /// engine has never been asked to load. Turning this off leaves the
    /// template's own collision in place, which is wrong for the map but
    /// isolates the archive when a converted map fails to load.
    #[serde(default = "default_true")]
    pub emit_collision: bool,
    /// How many entries the collision face-material dictionary carries.
    ///
    /// `per_material` is what the source supports and what the port wants;
    /// `single` is the only shape this engine has been observed to load.
    #[serde(default = "default_face_dictionary")]
    pub collision_face_dictionary: FaceDictionary,
    /// Largest collision mesh emitted into one model archive.
    ///
    /// See [`crate::soc::convert::collision::MAX_TRIANGLES_PER_MODEL`]: a
    /// level-sized mesh fails to cook, and the engine crashes on the result it
    /// never checks.
    #[serde(default = "default_max_collision_triangles")]
    pub max_collision_triangles: usize,
    /// Largest render surface (vertex count) a shader part may have.
    ///
    /// The default is one `u16` index space. A map that continues into 0.26
    /// sets 35,000, the native exporter's bound: 0.26 copies each static
    /// surface's vertices through a 20-byte-per-vertex stack buffer while
    /// loading it (`_alloca` at 0x16ebe4 under a 1 MiB thread stack), and a
    /// 65,238-vertex part (mp_rembasa) overflows it on load.
    #[serde(default = "default_max_surface_vertices")]
    pub max_surface_vertices: usize,
    /// Accept source mip chains that continue past the last complete 4x4 block.
    /// v0.100b can substitute its visible `missing texture` fallback for those,
    /// so this must be a deliberate, recorded decision.
    #[serde(default)]
    pub allow_untrimmed_mip_chains: bool,
}

fn default_surfaces_per_model() -> usize {
    200
}

fn default_diffuse_path() -> String {
    "material.g_stage.effect.texture_diffuse.value".to_owned()
}

fn default_normal_path() -> String {
    "material.g_stage.effect.texture_normal.value".to_owned()
}

fn default_normal_enable_path() -> String {
    "material.g_stage.effect.use_nmap.value".to_owned()
}

fn default_terrain_detail_texture() -> Option<String> {
    // Off. The feature is real -- `effect_gstage_default_materials.cpp` reads
    // `use_tditail_diffuse` and binds `t_detail` -- but build 802 ships no
    // compiled shader for it. Of the flag positions in `gbuffer_pass.ps`, 32
    // vary across the 1,136 shipped permutations and 5 are stuck at zero; this
    // is one of the five, so enabling it faults with
    // `data->new_resource->shader_source == 0`. Set it on a build whose shader
    // set includes the permutation.
    None
}

fn default_terrain_detail_tile() -> i32 {
    20
}

fn default_terrain_layer_weights() -> [u8; 4] {
    // Each X-Ray terrain surface already identifies one ground material, so a
    // single live layer is sufficient. The writer binds the same texture to
    // all four as a guard against compressed-mask leakage.
    [255, 0, 0, 0]
}

fn default_terrain_base_tile() -> i32 {
    1
}

fn default_terrain_layer_metres() -> f32 {
    // A repeat every four metres: close enough that the texture reads as ground
    // underfoot, far enough that the pattern does not announce itself.
    4.0
}

fn default_terrain_exposure() -> f32 {
    1.0
}

fn default_decal_lift() -> f32 {
    // 1cm reduced the flicker but did not settle it; 3cm is still under the
    // thickness of the litter and posters themselves.
    0.03
}

fn default_water_reflection_probe() -> String {
    "environment_probes/environment_probe".to_owned()
}

fn default_post_process_material() -> String {
    "material_instances/post_process/level_03_day.material".to_owned()
}

fn default_max_collision_triangles() -> usize {
    super::convert::collision::MAX_TRIANGLES_PER_MODEL
}

fn default_max_surface_vertices() -> usize {
    super::convert::surface::MAX_SURFACE_VERTICES
}

fn default_face_dictionary() -> FaceDictionary {
    FaceDictionary::PerMaterial
}

fn default_true() -> bool {
    true
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
            bail!(
                "SoC map manifest namespace {:?} is not a safe resource prefix",
                self.namespace
            );
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
        // Filtering the template's scene objects needs both the array and the
        // field that says what each object is.
        if self.project_bindings.objects_array_path.is_some()
            != self.project_bindings.object_type_path.is_some()
        {
            bail!("SoC map manifest must declare objects_array_path and object_type_path together");
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

    /// The archive carrying collision.
    ///
    /// Collision rides inside the first render archive rather than in one of
    /// its own: v0.100b ships no collision-only model, and a standalone one
    /// faults the loader.
    pub fn collision_model_path(&self) -> String {
        self.model_path(0)
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
            post_process_material: default_post_process_material(),
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
            objects_array_path: Some("objects".into()),
            object_type_path: Some("game_object_type".into()),
            retained_object_types: vec!["sky".into(), "environment".into(), "volume_fog".into()],
        },
        surfaces_per_model: 200,
        spawn_position: [0.0, 2.1951056, 0.0],
        spawn_orientation: 0.0,
        camera_at_spawn: false,
        render_shader_families: BTreeMap::new(),
        collision_material_overrides: BTreeMap::new(),
        material_templates: RenderFamily::ALL
            .iter()
            .map(|family| {
                (
                    *family,
                    format!("material_instances/stock/{}", family.as_str()),
                )
            })
            .collect(),
        lighting_volume_margin: 16.0,
        decal_lift: default_decal_lift(),
        terrain_detail_texture: default_terrain_detail_texture(),
        terrain_detail_tile: default_terrain_detail_tile(),
        terrain_layer_weights: default_terrain_layer_weights(),
        terrain_base_tile: default_terrain_base_tile(),
        terrain_layer_metres: default_terrain_layer_metres(),
        terrain_layer_textures: BTreeMap::new(),
        terrain_layer_normal_textures: BTreeMap::new(),
        terrain_exposure: default_terrain_exposure(),
        water_reflection_probe: default_water_reflection_probe(),
        diffuse_path: default_diffuse_path(),
        family_uv_scale: BTreeMap::new(),
        family_diffuse_paths: BTreeMap::new(),
        disable_foliage_wind: false,
        normal_path: default_normal_path(),
        normal_enable_path: default_normal_enable_path(),
        bind_normal_maps: true,
        emit_collision: true,
        collision_face_dictionary: default_face_dictionary(),
        max_collision_triangles: default_max_collision_triangles(),
        max_surface_vertices: default_max_surface_vertices(),
        texture_aliases: BTreeMap::new(),
        normal_texture_overrides: BTreeMap::new(),
        material_aliases: BTreeMap::new(),
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
        // Collision rides inside the first render archive, not beside it.
        assert_eq!(manifest.collision_model_path(), manifest.model_path(0));
        assert_eq!(
            manifest.collision_model_path(),
            "models/ported/soc/cordon/cordon_000.model"
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
        manifest
            .material_templates
            .remove(&RenderFamily::Translucent);
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
        let error = serde_json::from_str::<SocMapManifest>(text)
            .unwrap_err()
            .to_string();
        assert!(error.contains("surprise"), "{error}");
    }
}
