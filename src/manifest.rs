use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortManifest {
    pub format_version: u32,
    pub asset: AssetSpec,
    pub weapon: WeaponSpec,
    pub ammo: AmmoSpec,
    pub dictionary: DictionarySpec,
    pub localization: LocalizationSpec,
    #[serde(default)]
    pub generated_configs: Vec<GeneratedConfigSpec>,
    #[serde(default)]
    pub copy: CopySpec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmorPortManifest {
    pub format_version: u32,
    pub asset: AssetSpec,
    pub armors: Vec<ArmorSpec>,
    pub model: SkinnedModelSpec,
    /// Target-schema configs patched as part of a character/armor port.
    #[serde(default)]
    pub generated_configs: Vec<ArmorGeneratedConfigSpec>,
    #[serde(default)]
    pub materials: Vec<ArmorMaterialSpec>,
    #[serde(default)]
    pub textures: Vec<ArmorTextureSpec>,
    pub dictionary: DictionarySpec,
    pub localization: LocalizationSpec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stk2ContentManifest {
    pub format_version: u32,
    pub asset: AssetSpec,
    pub weapons: Vec<VisualWeaponSpec>,
    pub weapon_model: WeaponModelTemplateSpec,
    pub map_models: Vec<StaticModelPortSpec>,
    pub static_model: StaticModelTemplateSpec,
    #[serde(default)]
    pub materials: Vec<ArmorMaterialSpec>,
    #[serde(default)]
    pub textures: Vec<ArmorTextureSpec>,
    pub lobby: LobbyReplacementSpec,
    pub dictionary: DictionarySpec,
    pub localization: LocalizationSpec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapPortManifest {
    pub format_version: u32,
    pub asset: AssetSpec,
    pub map_name: String,
    pub roots: Vec<String>,
    /// Additional donor-owned files or resource-directory roots. Map roots are
    /// always donor-owned; all other shared dependencies prefer the target DB.
    #[serde(default)]
    pub forced_donor_paths: BTreeSet<String>,
    /// Models (as the project names them, `level/03/blocks`) emitted from the
    /// donor under `relocated_model_root` rather than resolved to the target's
    /// copy of the same path, with every client-project reference following.
    /// For a level's own architecture, which a later build edits together
    /// with its terrain; `list-relocatable-models` derives the list.
    #[serde(default)]
    pub relocated_donor_models: BTreeSet<String>,
    #[serde(default)]
    pub relocated_model_root: Option<String>,
    #[serde(default)]
    pub path_replacements: BTreeMap<String, String>,
    /// Resources cloned from the target DB into versioned output paths.
    #[serde(default)]
    pub target_copies: BTreeMap<String, String>,
    #[serde(default)]
    pub upgrade_stripped_static_vertex_declarations: bool,
    #[serde(default)]
    pub allowed_unresolved_references: BTreeSet<String>,
    /// Rebuild donor material instances from compatible target-build material
    /// configurations. This keeps target shader/effect switches while copying
    /// safe donor texture and constant values.
    #[serde(default)]
    pub convert_materials_to_target_schema: bool,
    /// Explicit target-build templates for materials whose stage/effect layout
    /// cannot be matched automatically.
    #[serde(default)]
    pub material_template_overrides: BTreeMap<String, String>,
    /// Exact string substitutions applied inside donor binary configs after
    /// schema conversion. This is used for known donor typos and for derived
    /// map variants that intentionally select a target-build resource.
    #[serde(default)]
    pub config_string_replacements: BTreeMap<String, String>,
    /// Per-model surface material overrides. Keys are donor model settings
    /// paths, inner keys are render-surface names, and values are target-build
    /// material names without the material_instances/ prefix or .material
    /// suffix.
    #[serde(default)]
    pub model_material_overrides: BTreeMap<String, BTreeMap<String, String>>,
    /// Convert the post-0.1.1 client-project layout, which split scene objects
    /// into server_objects/client_objects, back to the v0.100b layout with
    /// top-level respawn_points and objects.
    #[serde(default)]
    pub convert_split_client_objects_to_legacy: bool,
    /// Client scene object classes to remove while converting a newer project
    /// to the legacy layout. Every declared type must be present, preventing
    /// stale compatibility exceptions from silently accumulating.
    #[serde(default)]
    pub drop_client_object_types: BTreeSet<String>,
    /// Target-build post-process material assigned to newer `environment`
    /// objects. Newer projects serialize a `properties` table instead, while
    /// v0.100b unconditionally requests the legacy `post_effect` string.
    #[serde(default)]
    pub legacy_environment_post_effect: Option<String>,
    /// Factor applied to every converted `environment_probe`'s
    /// `diffuse_multiplier` and `specular_multiplier`. The 0.20f and 0.23h
    /// `environment_probe_lighting.ps` multiply each cubemap sample by a
    /// literal 0.25 that v0.100b's does not, so their probes were baked four
    /// times brighter than v0.100b draws at full strength.
    #[serde(default)]
    pub environment_probe_light_scale: Option<f32>,
    /// Largest radius a converted sphere `environment_probe` keeps. v0.100b
    /// draws a probe the camera is inside through its back faces, and its
    /// camera clips them at 500 m (`s_camera_far_plane`): a donor global
    /// probe of 640+ m is then drawn only towards the screen edges, where
    /// view depth is shorter than distance, which shows as an unshadowed,
    /// washed-out ring around a correctly lit centre. Retail level_03's
    /// global probe is 300 m.
    #[serde(default)]
    pub environment_probe_max_radius: Option<f32>,
    /// Optional target-build client project used as the outer legacy schema.
    /// Only sections named by legacy_client_project_sections are copied from
    /// the converted donor project; all other sections remain target-native.
    #[serde(default)]
    pub legacy_client_project_template: Option<String>,
    #[serde(default)]
    pub legacy_client_project_sections: BTreeSet<String>,
    /// Named scene objects to remove from remapped client projects. This is
    /// intentionally explicit: silently retaining an object whose runtime
    /// resource does not exist can crash old resource cooks.
    #[serde(default)]
    pub drop_client_objects: BTreeSet<String>,
    pub spawn_position: [f32; 3],
    pub spawn_orientation: f32,
    /// Set by `--camera-at-spawn`, never by a manifest: the offline client
    /// inserts the player at the client project's `camera.position`.
    #[serde(skip)]
    pub camera_at_spawn: bool,
}

/// Fail-closed reconstruction of a newer map into target-native v0.100b
/// projects and versioned resource namespaces.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetNativeMapManifest {
    pub format_version: u32,
    pub asset: AssetSpec,
    pub map_name: String,
    pub donor_client_project: String,
    pub target_client_template: String,
    pub target_client_project: String,
    pub target_server_template: String,
    pub target_server_project: String,
    /// Resource namespace below models/, material_instances/, and textures/.
    pub namespace: String,
    #[serde(default)]
    pub material_template_overrides: BTreeMap<String, String>,
    /// Optional older-build material parameter baselines. Keys are donor
    /// material paths and values are material paths in the separately
    /// declared compatibility database. The baseline is still converted
    /// through a v0.100b template and emitted below the map namespace.
    #[serde(default)]
    pub material_compatibility_templates: BTreeMap<String, String>,
    /// Explicit texture bindings applied after target-material schema
    /// conversion. Keys are donor material paths, then binary-config field
    /// paths; values are texture bases below textures/ without an extension.
    #[serde(default)]
    pub material_texture_overrides: BTreeMap<String, BTreeMap<String, String>>,
    /// Explicit donor model/surface material names used only when the donor
    /// setting contains an empty material reference.
    #[serde(default)]
    pub empty_render_material_overrides: BTreeMap<String, BTreeMap<String, String>>,
    /// Explicit donor model/surfaces whose empty material reference is
    /// preserved because the target build demonstrably uses the same sentinel.
    #[serde(default)]
    pub preserve_empty_render_materials: BTreeMap<String, BTreeSet<String>>,
    /// Render-referenced models whose complete render material set is empty
    /// in both donor and target builds. Their collision families remain in
    /// the project, but their unsafe zero-surface render instance is omitted.
    #[serde(default)]
    pub drop_empty_render_models: BTreeSet<String>,
    /// Render-referenced models deliberately omitted because the donor model
    /// unit cannot be reconstructed without inventing geometry or a target
    /// shader contract. Values are required audit reasons. Any collision
    /// families remain; pure visual models are removed from the project.
    #[serde(default)]
    pub drop_unsupported_render_models: BTreeMap<String, String>,
    /// Explicit donor model/surfaces represented by an empty archive folder
    /// with no converted mesh or export properties. The surface is removed
    /// from both material settings and the reconstructed LOD hierarchy.
    #[serde(default)]
    pub drop_empty_render_surfaces: BTreeMap<String, BTreeSet<String>>,
    /// Exact donor DDS paths that must reuse the same logical texture from
    /// the target database instead of importing an incompatible encoding.
    #[serde(default)]
    pub target_texture_fallbacks: BTreeSet<String>,
    /// Explicit aliases for target-runtime texture holes. Keys are missing
    /// target DDS paths and values are proven source DDS paths in the stock
    /// target database. The matching .options payload is copied with the DDS.
    #[serde(default)]
    pub target_texture_aliases: BTreeMap<String, String>,
    /// Explicit semantic aliases from donor game-material names to names in
    /// the stock v0.100b game-material dictionary.
    #[serde(default)]
    pub game_material_name_overrides: BTreeMap<String, String>,
    /// Explicit model/surface target bindings used to correct or add
    /// game-material assignments for collision dictionaries.
    #[serde(default)]
    pub game_material_binding_overrides: BTreeMap<String, BTreeMap<String, String>>,
    /// Declared collision resource folders that are present in the donor
    /// project but contain no payload and must therefore be omitted.
    #[serde(default)]
    pub drop_empty_collision_families: BTreeSet<String>,
    /// Stock archive hard links that must become independent file nodes for
    /// target-runtime compatibility. Payload bytes are copied unchanged.
    #[serde(default)]
    pub materialize_target_hard_links: BTreeSet<String>,
    pub spawn_position: [f32; 3],
    pub spawn_orientation: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayableLobbyManifest {
    pub format_version: u32,
    pub asset: AssetSpec,
    pub map_name: String,
    pub source_client_project: String,
    pub target_client_project: String,
    pub server_template_project: String,
    pub target_server_project: String,
    pub team_spawns: [[f32; 3]; 2],
    pub spawn_orientation: f32,
    pub victory_item_spawners: Vec<[f32; 3]>,
    pub victory_item_containers: [[f32; 3]; 2],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stk2MapSliceManifest {
    pub format_version: u32,
    pub asset: AssetSpec,
    pub map_name: String,
    #[serde(default)]
    pub models: Vec<StaticModelPortSpec>,
    pub static_model: StaticModelTemplateSpec,
    pub client_template_project: String,
    pub target_client_project: String,
    pub server_template_project: String,
    pub target_server_project: String,
    #[serde(default)]
    pub visuals: Vec<LobbyVisualSpec>,
    /// Optional Lua-style STK2 editor project. Direct solid visuals are
    /// discovered and converted automatically from the exported-model tree.
    #[serde(default)]
    pub source_project: Option<String>,
    #[serde(default)]
    pub source_origin: [f32; 3],
    #[serde(default)]
    pub auto_material: Option<String>,
    #[serde(default)]
    pub material_template: Option<String>,
    /// Reviewed v0.100b material-instance templates used to translate the
    /// source STK2 material contract. When absent, `material_template` is used
    /// for every converted diffuse (the legacy, low-fidelity behavior).
    #[serde(default)]
    pub material_templates: Option<Stk2MaterialTemplates>,
    #[serde(default)]
    pub texture_options_template: Option<String>,
    /// Source texture names which are known to be absent or malformed in the
    /// STK2 source tree and may therefore use `auto_material`.
    #[serde(default)]
    pub fallback_source_textures: BTreeSet<String>,
    /// Fully-qualified `library#render/surface` bindings whose STK2
    /// export_properties intentionally contain an empty texture name.
    #[serde(default)]
    pub fallback_empty_surfaces: BTreeSet<String>,
    /// Fully-qualified `library#render/surface` to exact STK2 source material
    /// instance. This disambiguates cases where one diffuse image is used by
    /// source materials with different shader families or auxiliary maps.
    #[serde(default)]
    pub surface_material_overrides: BTreeMap<String, String>,
    /// Exact STK2 source-material names to reviewed v0.100b material-instance
    /// templates. This refines the broad opaque/metal/foliage family choice
    /// when shader semantics such as concrete detail normals matter.
    #[serde(default)]
    pub source_material_template_overrides: BTreeMap<String, String>,
    /// Explicit source-project library to compiled archive overrides. This is
    /// required when the editor project retains an old library path or only a
    /// `.model.orig` cooked archive survived.
    #[serde(default)]
    pub model_archive_overrides: BTreeMap<String, String>,
    /// Active editor model libraries deliberately omitted because no compiled
    /// archive survived. Values are required audit reasons; stale entries fail
    /// the build.
    #[serde(default)]
    pub omitted_model_libraries: BTreeMap<String, String>,
    /// Missing terrain texture name to a reviewed source texture name. Target
    /// resources retain the original layer name so project bindings remain
    /// stable.
    #[serde(default)]
    pub terrain_texture_aliases: BTreeMap<String, String>,
    /// Replace spatial objects inherited from the target client-project
    /// template with STK2 light objects projected through matching v0.100b
    /// light schemas. Global sky/environment objects remain target-native.
    #[serde(default)]
    pub convert_source_lights: bool,
    /// Optional STK2 light types to convert. Target-template lights of other
    /// types are retained, allowing each old-engine light contract to be
    /// introduced and validated independently. When absent, all source light
    /// types are converted.
    #[serde(default)]
    pub source_light_types: Option<BTreeSet<i32>>,
    /// Remove environment probes inherited from the unrelated target map
    /// template. Their finite bounds and cubemaps are map-specific; retaining
    /// them creates hard spatial lighting boundaries in reconstructed maps.
    #[serde(default)]
    pub drop_target_environment_probes: bool,
    /// Project the STK2 environment material's understood fog and ambient
    /// fields into a cloned v0.100b post-process material.
    #[serde(default)]
    pub convert_source_environment: bool,
    #[serde(default)]
    pub environment_material_template: Option<String>,
    #[serde(default)]
    pub target_environment_material: Option<String>,
    /// Stock v0.100b 16x16x16 volume-LUT DDS whose header and mip layout are
    /// used to cook the STK2 256x16 strip LUT.
    #[serde(default)]
    pub environment_color_grading_template: Option<String>,
    /// Logical target texture name without textures/ or an extension.
    #[serde(default)]
    pub target_environment_color_grading_texture: Option<String>,
    #[serde(default)]
    pub flat_collision_scale: Option<[f32; 3]>,
    #[serde(default)]
    pub convert_model_collision: bool,
    /// Target v0.100b game-material name used for STK2 model collisions.
    /// STK2 exported low-collision archives do not retain per-face material
    /// dictionaries, so this is an explicit, audited model-level fallback.
    #[serde(default)]
    pub static_collision_game_material: Option<String>,
    #[serde(default)]
    pub terrain_directory: Option<String>,
    /// Target-owned texture bindings applied only to generated terrain
    /// materials. Keys are old-schema material properties such as
    /// `texture_normal`; values omit the `textures/` prefix and extension.
    #[serde(default)]
    pub terrain_texture_binding_overrides: BTreeMap<String, String>,
    /// Target v0.100b game-material name used for generated terrain collision.
    #[serde(default)]
    pub terrain_collision_game_material: Option<String>,
    /// Looping ambience imported from a Survarium donor database. Only the
    /// encoded OGG payload crosses builds; object and option contracts are
    /// cloned from explicitly named v0.100b templates.
    #[serde(default)]
    pub ambient_sounds: Vec<Stk2AmbientSoundSpec>,
    pub team_spawns: [[f32; 3]; 2],
    pub spawn_orientation: f32,
    pub victory_item_spawners: Vec<[f32; 3]>,
    pub victory_item_containers: [[f32; 3]; 2],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stk2AmbientSoundSpec {
    /// Logical name below `sounds/single/` in the donor database.
    pub donor_sound: String,
    /// Logical name below `sounds/single/` emitted into the target bundle.
    pub target_sound: String,
    /// v0.100b project containing the `object_sound` instance to clone.
    pub object_template_project: String,
    /// Logical sound name of the `object_sound` instance to clone.
    pub object_template_sound: String,
    /// v0.100b `.single_sound_options` resource to clone.
    pub options_template: String,
    pub object_name: String,
    pub position: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stk2MaterialTemplates {
    pub opaque: String,
    pub alpha_test: String,
    pub alpha_test_two_sided: String,
    pub two_sided: String,
    pub decal: String,
    pub metal: String,
    pub metal_alpha_test: String,
    pub metal_alpha_test_two_sided: String,
    pub metal_two_sided: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualWeaponSpec {
    pub dict_id: u32,
    pub item_category: i32,
    pub donor_model_archive: String,
    pub target_model: String,
    pub template_config: String,
    pub target_config: String,
    #[serde(default)]
    pub patch: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub replace_strings: BTreeMap<String, String>,
    pub surfaces: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponModelTemplateSpec {
    pub bind_pose_template: String,
    pub settings_template: String,
    pub skeleton_template: String,
    pub render_properties_template: String,
    pub surface_properties_template: String,
    pub rigid_bone_index: u8,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticModelPortSpec {
    pub donor_model_archive: String,
    pub target_model: String,
    pub surfaces: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticModelTemplateSpec {
    pub settings_template: String,
    pub render_properties_template: String,
    pub surface_properties_template: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LobbyReplacementSpec {
    pub template_project: String,
    pub target_project: String,
    pub target_project_lua: String,
    pub visuals: Vec<LobbyVisualSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LobbyVisualSpec {
    pub library_name: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmorGeneratedConfigSpec {
    pub template_config: String,
    pub target_config: String,
    #[serde(default)]
    pub patch: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub replace_strings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmorMaterialSpec {
    pub template: String,
    pub target: String,
    pub replace_strings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmorTextureSpec {
    pub source_file: String,
    pub template_options: String,
    /// Target resource root without `.dds` or `.options`.
    pub target: String,
    pub compression: TextureCompression,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextureCompression {
    Dxt1,
    Dxt5Normal,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSpec {
    pub id: String,
    pub donor_build: String,
    pub target_build: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponSpec {
    pub dict_id: u32,
    pub item_category: i32,
    pub donor_config: String,
    pub template_config: String,
    pub target_config: String,
    #[serde(default)]
    pub patch: BTreeMap<String, serde_json::Value>,
    /// Exact string substitutions applied recursively after structural patches.
    #[serde(default)]
    pub replace_strings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmmoSpec {
    pub dict_id: u32,
    pub item_category: i32,
    pub donor_config: String,
    pub template_config: String,
    pub target_config: String,
    /// Reuse an ammunition item already present in the target database.
    #[serde(default)]
    pub reuse_existing: bool,
    #[serde(default)]
    pub patch: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmorSpec {
    pub dict_id: u32,
    pub item_category: i32,
    pub template_config: String,
    pub target_config: String,
    #[serde(default)]
    pub patch: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub replace_strings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinnedModelSpec {
    pub animation_skeleton: String,
    pub bind_pose_template: String,
    pub settings_template: String,
    pub skeleton_template: String,
    pub render_properties_template: String,
    pub surface_properties_template: String,
    /// Keep character-only settings such as hit parameters and leg IK while
    /// replacing the rendered surface/material lists.
    #[serde(default)]
    pub preserve_settings_template: bool,
    /// Rotate donor geometry and tangent bases by 180 degrees around Y.
    #[serde(default)]
    pub rotate_y_180: bool,
    /// Rebase Maya geometry from the donor arm bind frames into the target
    /// skeleton's arm bind frames. Leave disabled to preserve donor proportions.
    #[serde(default)]
    pub retarget_maya_bind_pose: bool,
    /// Replaces selected target model bind frames with converted Maya donor
    /// frames. Keys are target bone names and values are Maya influence names.
    #[serde(default)]
    pub maya_bind_pose_map: BTreeMap<String, String>,
    /// Optional target-schema skinned surface whose geometry gradients and
    /// weights are used as a correspondence reference for selected Maya bones.
    pub weight_transfer_template: Option<String>,
    #[serde(default)]
    pub weight_transfer_influences: BTreeSet<String>,
    #[serde(default = "default_weight_transfer_threshold")]
    pub weight_transfer_threshold: f64,
    /// Remove triangles containing vertices strongly controlled by any listed
    /// Maya influence. This is a deliberately lossy containment option for
    /// animation-time spikes when a donor helper has no compatible target.
    #[serde(default)]
    pub trim_maya_influences: BTreeSet<String>,
    #[serde(default = "default_trim_maya_weight_threshold")]
    pub trim_maya_weight_threshold: f64,
    /// Maps bone names embedded in the donor bind pose to target bind-pose names.
    pub bone_map: BTreeMap<String, String>,
    /// Optional linear target-bone mixtures for donor influences whose source
    /// joint has no geometrically compatible target counterpart.
    #[serde(default)]
    pub bone_weight_blends: BTreeMap<String, BTreeMap<String, f64>>,
    pub surfaces: BTreeMap<String, String>,
    /// Target model roots and the donor geometry contained in each armor part.
    pub target_models: BTreeMap<String, TargetModelSpec>,
}

fn default_weight_transfer_threshold() -> f64 {
    0.05
}

fn default_trim_maya_weight_threshold() -> f64 {
    0.5
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetModelSpec {
    pub surfaces: BTreeMap<String, SurfaceOutputSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceOutputSpec {
    pub source: String,
    #[serde(default)]
    pub selection: SurfaceSelection,
    /// For Maya-skinned static donors, retain only triangles whose average
    /// weight belongs to these donor influences.
    #[serde(default)]
    pub include_maya_influences: BTreeSet<String>,
    #[serde(default = "default_include_maya_weight_threshold")]
    pub include_maya_weight_threshold: f64,
    /// Exact zero-based source triangle IDs to omit.
    #[serde(default)]
    pub trim_triangles: BTreeSet<usize>,
}

fn default_include_maya_weight_threshold() -> f64 {
    0.5
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceSelection {
    #[default]
    All,
    Hands,
    ExcludeHands,
    Feet,
    ExcludeFeet,
    /// Emit one degenerate triangle to suppress a stock character surface.
    Hidden,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DictionarySpec {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizationSpec {
    pub path: String,
    pub strings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedConfigSpec {
    pub donor_config: String,
    pub template_config: String,
    pub target_config: String,
    #[serde(default)]
    pub patch: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub replace_strings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopySpec {
    #[serde(default)]
    pub prefixes: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BuildRecord {
    pub format_version: u32,
    pub asset_id: String,
    pub donor_build: String,
    pub target_build: String,
    pub weapon_dict_id: u32,
    pub ammo_dict_id: u32,
    pub files: Vec<BuiltFile>,
    pub shared_dependencies: Vec<String>,
    pub unresolved_references: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArmorBuildRecord {
    pub format_version: u32,
    pub asset_id: String,
    pub donor_build: String,
    pub target_build: String,
    pub armor_dict_ids: Vec<u32>,
    pub files: Vec<BuiltFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Stk2ContentBuildRecord {
    pub format_version: u32,
    pub asset_id: String,
    pub donor_build: String,
    pub target_build: String,
    pub weapon_dict_ids: Vec<u32>,
    pub replaced_project: String,
    pub files: Vec<BuiltFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MapBuildRecord {
    pub format_version: u32,
    pub asset_id: String,
    pub donor_build: String,
    pub target_build: String,
    pub map_name: String,
    pub roots: Vec<String>,
    pub spawn_position: [f32; 3],
    pub spawn_orientation: f32,
    pub files: Vec<BuiltFile>,
    pub shared_dependencies: Vec<String>,
    pub converted_static_models: Vec<String>,
    pub converted_materials: BTreeMap<String, String>,
    pub dropped_client_objects: Vec<String>,
    pub allowed_unresolved_references: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ComposedBundleRecord {
    pub format_version: u32,
    pub asset_id: String,
    /// Ordered from lowest to highest precedence.
    pub bundles: Vec<String>,
    pub overwritten_paths: Vec<String>,
    #[serde(default)]
    pub materialize_target_hard_links: BTreeSet<String>,
    pub files: Vec<BuiltFile>,
}

/// Runtime catalog additions consumed by SRP.
///
/// This deliberately contains only additive IDs. Existing demo content stays
/// owned by SRP, so importing a weapon cannot silently replace an old one.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SrpContentManifest {
    pub format_version: u32,
    pub extra_weapon_dict_ids: Vec<u16>,
    pub extra_ammo_dict_ids: Vec<u16>,
    pub primary_weapon_dict_id: Option<u16>,
    pub primary_ammo_dict_id: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BuiltFile {
    pub path: String,
    pub source: String,
    pub size: u64,
    pub sha256: String,
}

impl PortManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading port manifest {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing port manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!(
                "unsupported manifest format {}, expected 1",
                self.format_version
            );
        }
        if self.asset.id.is_empty() {
            bail!("asset.id must not be empty");
        }
        if self.weapon.dict_id == self.ammo.dict_id {
            bail!("weapon and ammunition must have distinct dictionary IDs");
        }
        if self.weapon.target_config == self.ammo.target_config {
            bail!("weapon and ammunition target paths must differ");
        }
        Ok(())
    }
}

impl ArmorPortManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading armor port manifest {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing armor port manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!(
                "unsupported manifest format {}, expected 1",
                self.format_version
            );
        }
        if self.asset.id.is_empty() {
            bail!("asset.id must not be empty");
        }
        let armor_ids: BTreeSet<u32> = self.armors.iter().map(|armor| armor.dict_id).collect();
        if armor_ids.len() != self.armors.len() {
            bail!("armor dictionary IDs must be unique");
        }
        let armor_paths: BTreeSet<&str> = self
            .armors
            .iter()
            .map(|armor| armor.target_config.as_str())
            .collect();
        if armor_paths.len() != self.armors.len() {
            bail!("armor target config paths must be unique");
        }
        if self.model.surfaces.is_empty() {
            bail!("model.surfaces must not be empty");
        }
        if self.model.bone_map.is_empty() {
            bail!("model.bone_map must not be empty");
        }
        for (source, blend) in &self.model.bone_weight_blends {
            if !self.model.bone_map.contains_key(source) {
                bail!("model.bone_weight_blends source {source} is absent from bone_map");
            }
            let total: f64 = blend.values().sum();
            if blend.is_empty()
                || !blend
                    .values()
                    .all(|weight| weight.is_finite() && *weight > 0.0)
                || (total - 1.0).abs() > 1.0e-6
            {
                bail!(
                    "model.bone_weight_blends.{source} must contain positive finite weights summing to one"
                );
            }
        }
        if self.model.weight_transfer_template.is_some()
            != !self.model.weight_transfer_influences.is_empty()
        {
            bail!(
                "model.weight_transfer_template and model.weight_transfer_influences must be set together"
            );
        }
        if !self.model.weight_transfer_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.model.weight_transfer_threshold)
        {
            bail!("model.weight_transfer_threshold must be between zero and one");
        }
        for influence in &self.model.trim_maya_influences {
            if !self.model.bone_map.contains_key(influence) {
                bail!("model.trim_maya_influences entry {influence} is absent from bone_map");
            }
        }
        if !self.model.trim_maya_weight_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.model.trim_maya_weight_threshold)
        {
            bail!("model.trim_maya_weight_threshold must be between zero and one");
        }
        if self.model.target_models.is_empty() {
            bail!("model.target_models must not be empty");
        }
        for (target_model, model) in &self.model.target_models {
            for (surface, output) in &model.surfaces {
                for influence in &output.include_maya_influences {
                    if !self.model.bone_map.contains_key(influence) {
                        bail!(
                            "model.target_models.{target_model}.surfaces.{surface}.include_maya_influences entry {influence} is absent from bone_map"
                        );
                    }
                }
                if !output.include_maya_weight_threshold.is_finite()
                    || !(0.0..=1.0).contains(&output.include_maya_weight_threshold)
                {
                    bail!(
                        "model.target_models.{target_model}.surfaces.{surface}.include_maya_weight_threshold must be between zero and one"
                    );
                }
            }
        }
        let material_targets: BTreeSet<&str> = self
            .materials
            .iter()
            .map(|item| item.target.as_str())
            .collect();
        if material_targets.len() != self.materials.len() {
            bail!("material target paths must be unique");
        }
        let texture_targets: BTreeSet<&str> = self
            .textures
            .iter()
            .map(|item| item.target.as_str())
            .collect();
        if texture_targets.len() != self.textures.len() {
            bail!("texture target roots must be unique");
        }
        let mut used_sources = BTreeSet::new();
        for (target, model) in &self.model.target_models {
            if target.is_empty() || model.surfaces.is_empty() {
                bail!("model target paths and surface lists must not be empty");
            }
            for (output_surface, output) in &model.surfaces {
                if output_surface.is_empty() {
                    bail!("target model {target} has an empty output surface name");
                }
                if !self.model.surfaces.contains_key(&output.source) {
                    bail!(
                        "target model {target} output {output_surface} references undeclared source surface {}",
                        output.source
                    );
                }
                used_sources.insert(output.source.as_str());
            }
        }
        let declared_surfaces: BTreeSet<&str> =
            self.model.surfaces.keys().map(String::as_str).collect();
        if used_sources != declared_surfaces {
            bail!("every declared source surface must be assigned to a target model");
        }
        Ok(())
    }
}
impl Stk2ContentManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading STK2 content manifest {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing STK2 content manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!(
                "unsupported manifest format {}, expected 1",
                self.format_version
            );
        }
        if self.asset.id.is_empty() {
            bail!("asset.id must not be empty");
        }
        if self.weapons.is_empty() {
            bail!("weapons must not be empty");
        }
        let weapon_ids: BTreeSet<u32> = self.weapons.iter().map(|weapon| weapon.dict_id).collect();
        if weapon_ids.len() != self.weapons.len() {
            bail!("weapon dictionary IDs must be unique");
        }
        let weapon_configs: BTreeSet<&str> = self
            .weapons
            .iter()
            .map(|weapon| weapon.target_config.as_str())
            .collect();
        if weapon_configs.len() != self.weapons.len() {
            bail!("weapon target config paths must be unique");
        }
        let model_targets: BTreeSet<&str> = self
            .weapons
            .iter()
            .map(|weapon| weapon.target_model.as_str())
            .chain(
                self.map_models
                    .iter()
                    .map(|model| model.target_model.as_str()),
            )
            .collect();
        if model_targets.len() != self.weapons.len() + self.map_models.len() {
            bail!("target model paths must be unique");
        }
        for weapon in &self.weapons {
            if weapon.surfaces.is_empty() {
                bail!("weapon {} has no surfaces", weapon.target_config);
            }
        }
        for model in &self.map_models {
            if model.surfaces.is_empty() {
                bail!("map model {} has no surfaces", model.target_model);
            }
        }
        if self.lobby.visuals.is_empty() {
            bail!("lobby.visuals must not be empty");
        }
        Ok(())
    }
}

impl MapPortManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading map port manifest {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing map port manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!(
                "unsupported map manifest format {}, expected 1",
                self.format_version
            );
        }
        if self.asset.id.is_empty() || self.map_name.is_empty() {
            bail!("map asset.id and map_name must not be empty");
        }
        if self.roots.is_empty() {
            bail!("map roots must not be empty");
        }
        if self.roots.iter().any(|root| root.is_empty()) {
            bail!("map roots must not contain empty paths");
        }
        if self
            .path_replacements
            .iter()
            .any(|(source, target)| source.is_empty() || target.is_empty())
        {
            bail!("map path replacement sources and targets must not be empty");
        }
        let replacement_targets: BTreeSet<&str> = self
            .path_replacements
            .values()
            .chain(self.target_copies.values())
            .map(String::as_str)
            .collect();
        if replacement_targets.len() != self.path_replacements.len() + self.target_copies.len() {
            bail!("map replacement and target-copy destinations must be unique");
        }
        if self
            .target_copies
            .iter()
            .any(|(source, target)| source.is_empty() || target.is_empty())
        {
            bail!("map target-copy sources and destinations must not be empty");
        }
        if self
            .allowed_unresolved_references
            .iter()
            .any(|path| path.is_empty())
        {
            bail!("allowed unresolved map references must not contain empty paths");
        }
        if self
            .material_template_overrides
            .iter()
            .any(|(source, template)| {
                source.is_empty()
                    || template.is_empty()
                    || !source.ends_with(".material")
                    || !template.ends_with(".material")
            })
        {
            bail!("map material template overrides must contain non-empty .material paths");
        }
        if !self.convert_materials_to_target_schema && !self.material_template_overrides.is_empty()
        {
            bail!("map material_template_overrides require convert_materials_to_target_schema");
        }
        if self
            .config_string_replacements
            .iter()
            .any(|(source, replacement)| source.is_empty() || replacement.is_empty())
        {
            bail!("map config string replacements must not contain empty strings");
        }
        if self
            .model_material_overrides
            .iter()
            .any(|(settings, surfaces)| {
                !settings.ends_with("/settings")
                    || surfaces.is_empty()
                    || surfaces
                        .iter()
                        .any(|(surface, material)| surface.is_empty() || material.is_empty())
            })
        {
            bail!(
                "map model material overrides require a /settings path and non-empty surface/material names"
            );
        }
        if self.drop_client_objects.iter().any(|name| name.is_empty()) {
            bail!("map drop_client_objects must not contain empty names");
        }
        if self
            .drop_client_object_types
            .iter()
            .any(|name| name.is_empty())
        {
            bail!("map drop_client_object_types must not contain empty names");
        }
        if !self.drop_client_object_types.is_empty() && !self.convert_split_client_objects_to_legacy
        {
            bail!("map drop_client_object_types requires convert_split_client_objects_to_legacy");
        }
        if self
            .legacy_environment_post_effect
            .as_ref()
            .is_some_and(|name| name.is_empty() || name.ends_with(".material"))
        {
            bail!(
                "map legacy_environment_post_effect must be a non-empty material name without .material"
            );
        }
        if self.legacy_environment_post_effect.is_some()
            && !self.convert_split_client_objects_to_legacy
        {
            bail!(
                "map legacy_environment_post_effect requires convert_split_client_objects_to_legacy"
            );
        }
        if self.legacy_client_project_template.is_none()
            && !self.legacy_client_project_sections.is_empty()
        {
            bail!("map legacy_client_project_sections requires legacy_client_project_template");
        }
        if self
            .legacy_client_project_sections
            .iter()
            .any(|name| name.is_empty() || name == "project_name")
        {
            bail!(
                "map legacy client-project section names must be non-empty and must not replace project_name"
            );
        }
        if !self
            .spawn_position
            .into_iter()
            .chain([self.spawn_orientation])
            .all(f32::is_finite)
        {
            bail!("map spawn transform must contain only finite values");
        }
        Ok(())
    }
}

impl TargetNativeMapManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading target-native map manifest {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing target-native map manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!(
                "unsupported target-native map manifest format {}, expected 1",
                self.format_version
            );
        }
        if self.asset.id.is_empty() || self.map_name.is_empty() {
            bail!("target-native map asset.id and map_name must not be empty");
        }
        for path in [
            &self.donor_client_project,
            &self.target_client_template,
            &self.target_client_project,
            &self.target_server_template,
            &self.target_server_project,
        ] {
            if path.is_empty() {
                bail!("target-native map project paths must not be empty");
            }
        }
        if self.target_client_project == self.target_client_template
            || self.target_server_project == self.target_server_template
        {
            bail!("target-native map outputs must not overwrite their target templates");
        }
        if !self.namespace.starts_with("ported/")
            || self.namespace.starts_with('/')
            || self
                .namespace
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | ".."))
        {
            bail!("target-native map namespace must be a safe path below ported/");
        }
        if self
            .material_template_overrides
            .iter()
            .any(|(donor, target)| {
                !donor.starts_with("material_instances/")
                    || !donor.ends_with(".material")
                    || !target.starts_with("material_instances/")
                    || !target.ends_with(".material")
            })
        {
            bail!("target-native material overrides require complete material paths");
        }
        if self
            .material_compatibility_templates
            .iter()
            .any(|(donor, compatibility)| {
                !donor.starts_with("material_instances/")
                    || !donor.ends_with(".material")
                    || !compatibility.starts_with("material_instances/")
                    || !compatibility.ends_with(".material")
            })
        {
            bail!("target-native compatibility material templates require complete material paths");
        }
        if self
            .material_texture_overrides
            .iter()
            .any(|(material, bindings)| {
                !material.starts_with("material_instances/")
                    || !material.ends_with(".material")
                    || bindings.is_empty()
                    || bindings.iter().any(|(field, texture)| {
                        field.is_empty()
                            || texture.is_empty()
                            || texture.starts_with('/')
                            || texture.ends_with(".dds")
                            || texture.split('/').any(|component| {
                                component.is_empty() || matches!(component, "." | "..")
                            })
                    })
            })
        {
            bail!("target-native material texture overrides are malformed");
        }
        if self
            .empty_render_material_overrides
            .iter()
            .any(|(model, bindings)| {
                !model.starts_with("models/")
                    || !model.ends_with(".model")
                    || bindings.is_empty()
                    || bindings.iter().any(|(surface, material)| {
                        surface.is_empty()
                            || material.is_empty()
                            || material.starts_with('/')
                            || material.ends_with(".material")
                            || material.split('/').any(|component| {
                                component.is_empty() || matches!(component, "." | "..")
                            })
                    })
            })
        {
            bail!("target-native empty render-material overrides are malformed");
        }
        if self
            .preserve_empty_render_materials
            .iter()
            .any(|(model, surfaces)| {
                !model.starts_with("models/")
                    || !model.ends_with(".model")
                    || surfaces.is_empty()
                    || surfaces.iter().any(String::is_empty)
            })
        {
            bail!("target-native preserved empty render-material entries are malformed");
        }
        if self.drop_empty_render_models.iter().any(|model| {
            !model.starts_with("models/")
                || !model.ends_with(".model")
                || model
                    .split('/')
                    .any(|component| component.is_empty() || matches!(component, "." | ".."))
        }) {
            bail!("target-native dropped empty render models are malformed");
        }
        for (model, overrides) in &self.empty_render_material_overrides {
            if let Some(preserved) = self.preserve_empty_render_materials.get(model) {
                let conflicts: Vec<_> = overrides
                    .keys()
                    .filter(|surface| preserved.contains(*surface))
                    .collect();
                if !conflicts.is_empty() {
                    bail!(
                        "target-native empty render-material surfaces cannot be both overridden and preserved: {model} {conflicts:?}"
                    );
                }
            }
        }
        if self
            .target_texture_fallbacks
            .iter()
            .any(|path| !path.starts_with("textures/") || !path.ends_with(".dds"))
        {
            bail!("target-native texture fallbacks require complete textures/*.dds paths");
        }
        if self.target_texture_aliases.iter().any(|(target, source)| {
            target == source
                || !target.starts_with("textures/")
                || !target.ends_with(".dds")
                || !source.starts_with("textures/")
                || !source.ends_with(".dds")
                || [target, source].into_iter().any(|path| {
                    path.split('/')
                        .any(|component| component.is_empty() || matches!(component, "." | ".."))
                })
        }) {
            bail!(
                "target-native texture aliases require distinct safe textures/*.dds target/source paths"
            );
        }
        if self
            .game_material_name_overrides
            .iter()
            .any(|(donor, target)| donor.is_empty() || target.is_empty())
        {
            bail!("target-native game-material aliases must not be empty");
        }
        if self
            .game_material_binding_overrides
            .iter()
            .any(|(model, bindings)| {
                !model.starts_with("models/")
                    || !model.ends_with(".model")
                    || bindings.is_empty()
                    || bindings
                        .iter()
                        .any(|(surface, material)| surface.is_empty() || material.is_empty())
            })
        {
            bail!("target-native game-material binding overrides are malformed");
        }
        if self.drop_empty_collision_families.iter().any(|path| {
            !path.starts_with("models/")
                || !(path.ends_with("/collision") || path.ends_with("/walker_collision"))
        }) {
            bail!(
                "target-native empty collision drops require complete models/.../(walker_)collision paths"
            );
        }
        if self.materialize_target_hard_links.iter().any(|path| {
            path.starts_with('/')
                || path
                    .split('/')
                    .any(|component| component.is_empty() || matches!(component, "." | ".."))
        }) {
            bail!("target-native materialized hard-link paths must be safe virtual paths");
        }
        if !self
            .spawn_position
            .into_iter()
            .chain([self.spawn_orientation])
            .all(f32::is_finite)
        {
            bail!("target-native map spawn transform must be finite");
        }
        Ok(())
    }
}

impl PlayableLobbyManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading playable lobby manifest {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing playable lobby manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!(
                "unsupported playable lobby manifest format {}, expected 1",
                self.format_version
            );
        }
        if self.asset.id.is_empty() || self.map_name.is_empty() {
            bail!("playable lobby asset.id and map_name must not be empty");
        }
        for path in [
            &self.source_client_project,
            &self.target_client_project,
            &self.server_template_project,
            &self.target_server_project,
        ] {
            if path.is_empty() {
                bail!("playable lobby project paths must not be empty");
            }
        }
        if self.source_client_project == self.target_client_project {
            bail!("playable lobby must not overwrite its source menu project");
        }
        if self.victory_item_spawners.is_empty() {
            bail!("playable lobby needs at least one victory item spawner");
        }
        if !self
            .team_spawns
            .into_iter()
            .chain(self.victory_item_containers)
            .chain(self.victory_item_spawners.iter().copied())
            .flatten()
            .chain([self.spawn_orientation])
            .all(f32::is_finite)
        {
            bail!("playable lobby transforms must contain only finite values");
        }
        Ok(())
    }
}

impl Stk2MapSliceManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading STK2 map-slice manifest {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing STK2 map-slice manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!(
                "unsupported STK2 map-slice manifest format {}, expected 1",
                self.format_version
            );
        }
        if self.asset.id.is_empty() || self.map_name.is_empty() {
            bail!("STK2 map-slice asset.id and map_name must not be empty");
        }
        if self.map_name.len() >= 30 {
            bail!(
                "STK2 map-slice map_name {:?} is {} bytes; build 802 match options require fewer than 30",
                self.map_name,
                self.map_name.len()
            );
        }
        if self.source_project.is_none() && (self.models.is_empty() || self.visuals.is_empty()) {
            bail!("STK2 map slices need at least one model and one visual");
        }
        if self.source_project.is_some() && self.auto_material.as_deref().unwrap_or("").is_empty() {
            bail!("an STK2 source project requires auto_material");
        }
        if self.source_project.is_some()
            && ((self.material_template.as_deref().unwrap_or("").is_empty()
                && self.material_templates.is_none())
                || self
                    .texture_options_template
                    .as_deref()
                    .unwrap_or("")
                    .is_empty())
        {
            bail!(
                "an STK2 source project requires material_template or material_templates, and texture_options_template"
            );
        }
        if let Some(templates) = &self.material_templates
            && [
                &templates.opaque,
                &templates.alpha_test,
                &templates.alpha_test_two_sided,
                &templates.two_sided,
                &templates.decal,
                &templates.metal,
                &templates.metal_alpha_test,
                &templates.metal_alpha_test_two_sided,
                &templates.metal_two_sided,
            ]
            .into_iter()
            .any(|template| template.is_empty())
        {
            bail!("STK2 material_templates entries must not be empty");
        }
        if self
            .fallback_source_textures
            .iter()
            .chain(self.fallback_empty_surfaces.iter())
            .any(|entry| entry.is_empty())
        {
            bail!("STK2 fallback texture and surface allowlists must not contain empty entries");
        }
        if self.source_project.is_none()
            && (!self.fallback_source_textures.is_empty()
                || !self.fallback_empty_surfaces.is_empty()
                || !self.surface_material_overrides.is_empty()
                || !self.model_archive_overrides.is_empty()
                || !self.omitted_model_libraries.is_empty()
                || !self.terrain_texture_aliases.is_empty())
        {
            bail!("STK2 fallback allowlists and model overrides require a source_project");
        }
        if self
            .surface_material_overrides
            .iter()
            .any(|(surface, material)| surface.is_empty() || material.is_empty())
        {
            bail!("STK2 surface material overrides require non-empty paths");
        }
        if self
            .source_material_template_overrides
            .iter()
            .any(|(source, template)| source.is_empty() || template.is_empty())
        {
            bail!("STK2 source material template overrides require non-empty paths");
        }
        if self.convert_model_collision
            && self
                .static_collision_game_material
                .as_deref()
                .unwrap_or("")
                .is_empty()
        {
            bail!("STK2 model collision conversion requires static_collision_game_material");
        }
        if self.terrain_directory.is_some()
            && self
                .terrain_collision_game_material
                .as_deref()
                .unwrap_or("")
                .is_empty()
        {
            bail!("STK2 terrain conversion requires terrain_collision_game_material");
        }
        if self
            .model_archive_overrides
            .iter()
            .any(|(library, archive)| {
                library.is_empty()
                    || archive.is_empty()
                    || archive.starts_with('/')
                    || archive.split('/').any(|component| {
                        component.is_empty() || component == "." || component == ".."
                    })
            })
        {
            bail!("STK2 model archive overrides must use non-empty safe relative paths");
        }
        if self
            .omitted_model_libraries
            .iter()
            .any(|(library, reason)| {
                library.is_empty()
                    || reason.trim().is_empty()
                    || library.starts_with('/')
                    || library.split('/').any(|component| {
                        component.is_empty() || component == "." || component == ".."
                    })
            })
        {
            bail!("STK2 omitted model libraries require safe names and non-empty audit reasons");
        }
        if self.terrain_texture_aliases.iter().any(|(target, source)| {
            [target, source].into_iter().any(|name| {
                name.is_empty()
                    || name.starts_with('/')
                    || name.split('/').any(|component| {
                        component.is_empty() || component == "." || component == ".."
                    })
            })
        }) {
            bail!("STK2 terrain texture aliases must use non-empty safe relative names");
        }
        let model_targets: BTreeSet<&str> = self
            .models
            .iter()
            .map(|model| model.target_model.as_str())
            .collect();
        if model_targets.len() != self.models.len() {
            bail!("STK2 map-slice target model paths must be unique");
        }
        for model in &self.models {
            if model.donor_model_archive.is_empty()
                || model.target_model.is_empty()
                || model.surfaces.is_empty()
            {
                bail!("STK2 map-slice models and their surface maps must not be empty");
            }
        }
        for path in [
            &self.static_model.settings_template,
            &self.static_model.render_properties_template,
            &self.static_model.surface_properties_template,
            &self.client_template_project,
            &self.target_client_project,
            &self.server_template_project,
            &self.target_server_project,
        ] {
            if path.is_empty() {
                bail!("STK2 map-slice template and project paths must not be empty");
            }
        }
        if self.client_template_project == self.target_client_project {
            bail!("STK2 map slice must not overwrite its client template");
        }
        if self.convert_source_lights && self.source_project.is_none() {
            bail!("STK2 source-light conversion requires source_project");
        }
        if self.source_light_types.is_some() && !self.convert_source_lights {
            bail!("STK2 source_light_types requires convert_source_lights");
        }
        if self
            .source_light_types
            .as_ref()
            .is_some_and(BTreeSet::is_empty)
        {
            bail!("STK2 source_light_types must not be empty");
        }
        if self.convert_source_environment {
            if self.source_project.is_none()
                || self
                    .environment_material_template
                    .as_deref()
                    .unwrap_or("")
                    .is_empty()
                || self
                    .target_environment_material
                    .as_deref()
                    .unwrap_or("")
                    .is_empty()
                || self
                    .environment_color_grading_template
                    .as_deref()
                    .unwrap_or("")
                    .is_empty()
                || self
                    .target_environment_color_grading_texture
                    .as_deref()
                    .unwrap_or("")
                    .is_empty()
            {
                bail!(
                    "STK2 source environment conversion requires source_project, environment_material_template, target_environment_material, environment_color_grading_template, and target_environment_color_grading_texture"
                );
            }
        } else if self.environment_material_template.is_some()
            || self.target_environment_material.is_some()
            || self.environment_color_grading_template.is_some()
            || self.target_environment_color_grading_texture.is_some()
        {
            bail!("STK2 environment conversion settings require convert_source_environment");
        }
        if self
            .terrain_texture_binding_overrides
            .iter()
            .any(|(property, texture)| property.is_empty() || texture.is_empty())
        {
            bail!("STK2 terrain_texture_binding_overrides entries must not be empty");
        }
        let mut ambient_targets = BTreeSet::new();
        for ambient in &self.ambient_sounds {
            if [
                ambient.donor_sound.as_str(),
                ambient.target_sound.as_str(),
                ambient.object_template_project.as_str(),
                ambient.object_template_sound.as_str(),
                ambient.options_template.as_str(),
                ambient.object_name.as_str(),
            ]
            .into_iter()
            .any(str::is_empty)
            {
                bail!("STK2 ambient sound entries must not contain empty strings");
            }
            for logical_name in [&ambient.donor_sound, &ambient.target_sound] {
                if logical_name.starts_with('/')
                    || logical_name.split('/').any(|component| {
                        component.is_empty() || component == "." || component == ".."
                    })
                {
                    bail!("STK2 ambient sound names must be safe relative names");
                }
            }
            if !ambient.options_template.ends_with(".single_sound_options") {
                bail!("STK2 ambient options templates must be .single_sound_options resources");
            }
            if !ambient_targets.insert(&ambient.target_sound) {
                bail!(
                    "STK2 ambient target sound {:?} is declared more than once",
                    ambient.target_sound
                );
            }
            if !ambient.position.into_iter().all(f32::is_finite) {
                bail!("STK2 ambient sound positions must contain only finite values");
            }
        }
        if self.victory_item_spawners.is_empty() {
            bail!("STK2 map slice needs at least one victory item spawner");
        }
        if !self
            .team_spawns
            .into_iter()
            .chain(self.victory_item_containers)
            .chain(self.victory_item_spawners.iter().copied())
            .chain(
                self.visuals
                    .iter()
                    .flat_map(|visual| [visual.position, visual.rotation, visual.scale]),
            )
            .chain([self.source_origin])
            .chain(self.flat_collision_scale)
            .flatten()
            .chain([self.spawn_orientation])
            .all(f32::is_finite)
        {
            bail!("STK2 map-slice transforms must contain only finite values");
        }
        Ok(())
    }
}
