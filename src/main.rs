use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use resources_db::NodeKind;
use vostok_resource_porter::armor;
use vostok_resource_porter::config_patch;
use vostok_resource_porter::dependency;
use vostok_resource_porter::map_compare;
use vostok_resource_porter::map_port;
use vostok_resource_porter::material_conversion::MaterialCatalog;
use vostok_resource_porter::material_patch;
use vostok_resource_porter::maya_binary;
use vostok_resource_porter::resource_copy_patch;
use vostok_resource_porter::soc;
use vostok_resource_porter::soc_addon;
use vostok_resource_porter::static_basis_patch;
use vostok_resource_porter::stk2_content;
use vostok_resource_porter::target_native_map;
use vostok_resource_porter::terrain_surface_patch;
use vostok_resource_porter::weapon;
use vostok_resource_porter::{
    ArmorPortManifest, BinaryConfig, ConfigValue, MapPortManifest, PlayableLobbyManifest,
    PortManifest, ResourceDb, Stk2ContentManifest, Stk2MapSliceManifest, TargetNativeMapManifest,
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
        /// Print one stable path/value pair per scalar leaf.
        #[arg(long)]
        leaves: bool,
    },
    /// Decode a self-describing binary config directly from a resources.db.
    DumpDbConfig {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        path: String,
        /// Print one stable path/value pair per scalar leaf.
        #[arg(long)]
        leaves: bool,
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
    /// Rebuild selected materials through explicit target-build templates.
    BuildMaterialPatch {
        /// Database containing the effective materials to convert.
        #[arg(long)]
        source_db: PathBuf,
        /// Target-build database containing the shader-compatible templates.
        #[arg(long)]
        target_db: PathBuf,
        /// Material mapping in RESOURCE_PATH=TEMPLATE_PATH form.
        #[arg(long = "material", required = true)]
        materials: Vec<String>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Patch existing boolean leaves in one self-describing binary config.
    BuildConfigPatch {
        #[arg(long)]
        source_db: PathBuf,
        #[arg(long)]
        resource: String,
        /// Boolean patch in CONFIG_PATH=true|false form.
        #[arg(long = "set-bool")]
        boolean_patches: Vec<String>,
        /// Retain only the last COUNT entries of an indexed config table.
        #[arg(long = "retain-indexed-suffix")]
        retained_indexed_suffixes: Vec<String>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Replace packed static-model vertex bases below a prefix with a flat diagnostic basis.
    BuildFlatStaticBasisPatch {
        #[arg(long)]
        source_db: PathBuf,
        #[arg(long)]
        prefix: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Flatten static-model vertex heights below a prefix for depth diagnostics.
    BuildFlatStaticHeightPatch {
        #[arg(long)]
        source_db: PathBuf,
        #[arg(long)]
        prefix: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Restore the native v0.100b alpha contract in packed static vertex bases.
    BuildCanonicalStaticBasisAlphaPatch {
        #[arg(long)]
        source_db: PathBuf,
        #[arg(long)]
        prefix: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Scale packed static-model UVs below a prefix for sampling diagnostics.
    BuildStaticUvScalePatch {
        #[arg(long)]
        source_db: PathBuf,
        #[arg(long)]
        prefix: String,
        #[arg(long)]
        scale: f32,
        #[arg(long)]
        output: PathBuf,
    },
    /// Merge every terrain cell into one surface and apply the native model contract.
    BuildSingleSurfaceTerrainPatch {
        #[arg(long)]
        source_db: PathBuf,
        #[arg(long)]
        source_prefix: String,
        #[arg(long)]
        target_prefix: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        material: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Copy exact resources from one database to new virtual paths.
    BuildResourceCopyPatch {
        #[arg(long)]
        source_db: PathBuf,
        /// Exact resource copy in SOURCE=DESTINATION form.
        #[arg(long = "copy", required = true)]
        copies: Vec<String>,
        #[arg(long)]
        output: PathBuf,
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
        /// Move the client project's camera onto the spawn, where the
        /// offline client inserts the player.
        #[arg(long)]
        camera_at_spawn: bool,
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
        /// Optional Survarium database supplying ambient OGG payloads declared
        /// by the map manifest.
        #[arg(long)]
        sound_donor_db: Option<PathBuf>,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Summarise a packed retail X-Ray installation.
    SocInventory {
        /// Installation holding root `gamedata.db*` or split archive directories.
        #[arg(long)]
        install_root: PathBuf,
        /// Restrict the listing to names starting with this prefix.
        #[arg(long, default_value = "")]
        prefix: String,
        /// Print every matching name instead of a count.
        #[arg(long)]
        list: bool,
        /// List gamemtl.xr materials with their behaviour flags.
        #[arg(long)]
        list_game_materials: bool,
        /// Write each archive's decoded file table here for inspection.
        #[arg(long)]
        dump_tables: Option<PathBuf>,
    },
    /// Decode one X-Ray level and report what a port would contain.
    ///
    /// Needs no target database, so it can run against a bare installation.
    SocProbeLevel {
        /// A retail installation, or an unpacked `gamedata` tree.
        #[arg(long)]
        source_root: PathBuf,
        /// Level directory name, or `all` to probe every level.
        #[arg(long)]
        level: String,
        /// Write the full report here as JSON.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Report places on a level's collision mesh where a player can stand.
    ///
    /// A converted map inherits no spawn points, and the centre of its
    /// bounding box is not ground -- for `mp_pool` it is the bottom of the
    /// pool. Ranked widest-ground first.
    SocSpawnCandidates {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        level: String,
        /// Grid resolution in world units.
        #[arg(long, default_value_t = 2.0)]
        cell: f32,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// List the collision under one point, in the order a falling player meets it.
    ///
    /// The question every "I fell through the map" report actually asks: is
    /// there a floor here, what is it made of, and does the port keep it?
    /// Each triangle whose footprint covers the point is reported with the
    /// height it sits at, its source material, and whether that material is
    /// passable -- because a passable triangle is dropped from the converted
    /// collision, and a spawn standing on one has nothing to stand on.
    SocProbeGround {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        level: String,
        /// World x of the point to look under.
        #[arg(long, allow_hyphen_values = true)]
        x: f32,
        /// World z of the point to look under.
        #[arg(long, allow_hyphen_values = true)]
        z: f32,
    },
    /// Measure one level's merged surfaces: extent, UV range, facing.
    ///
    /// Turns "the texture tiles wrong" into numbers: the UV range says how
    /// many times a texture repeats, and the facing histogram says whether a
    /// surface is a flat plane or the walls of a volume.
    SocSurfaceStats {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        level: String,
        /// Only report surfaces whose shader name contains this.
        #[arg(long)]
        shader: Option<String>,
        /// Only report surfaces whose triangles cover this world X coordinate.
        #[arg(long, requires = "z", allow_hyphen_values = true)]
        x: Option<f32>,
        /// Only report surfaces whose triangles cover this world Z coordinate.
        #[arg(long, requires = "x", allow_hyphen_values = true)]
        z: Option<f32>,
    },
    /// Report each shared vertex buffer.s declaration.
    SocAuditBuffers {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        level: String,
    },
    /// Report which OGF chunks each visual kind carries.
    ///
    /// A visual the converter declines may be a grouping node that loses
    /// nothing, or may hold its geometry in embedded children. This says which.
    SocAuditVisuals {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        level: String,
    },
    /// Extract one file from a Shadow of Chernobyl installation or gamedata tree.
    SocExtract {
        #[arg(long)]
        source_root: PathBuf,
        /// Archive-relative path, e.g. `levels/l01_escape/level.spawn`.
        #[arg(long)]
        path: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Decode a motion bank and report the motions and bones it carries.
    ///
    /// The bank names its own bones, so this needs no actor model: it answers
    /// "does this animation exist, on which bones, for how long" directly.
    SocProbeMotions {
        #[arg(long)]
        source_root: PathBuf,
        /// Archive-relative bank path, e.g. `meshes/actors/mp_stalker.omf`.
        #[arg(long)]
        bank: String,
        /// Restrict the motion listing to names starting with this prefix.
        #[arg(long, default_value = "")]
        prefix: String,
        /// Report this motion's per-bone tracks instead of the listing.
        #[arg(long)]
        motion: Option<String>,
    },
    /// Retarget one Shadow of Chernobyl motion onto the v0.100b character rig.
    ///
    /// Writes `<output>.b-spline` and `<output>.bones_names`, the pair the
    /// engine loads as one `animation_class` resource.
    SocRetargetMotion {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        /// Motion bank, e.g. `meshes/actors/mp_stalker.omf`.
        #[arg(long)]
        bank: String,
        /// An actor model bound to that bank, for its rest pose.
        #[arg(long)]
        model: String,
        /// Motion name, e.g. `cl_run_fwd_0`.
        #[arg(long)]
        motion: String,
        /// Target rest pose.
        #[arg(
            long,
            default_value = "models/character/human/base/base_character_boot.skinned_model/render/bind_pose"
        )]
        bind_pose: String,
        /// Target bone hierarchy.
        #[arg(long, default_value = "animations/skeletons/scavengers_01.skeleton")]
        skeleton: String,
        /// Output path without an extension.
        #[arg(long)]
        output: PathBuf,
        /// Also report where this frame's pose lands in both rigs.
        #[arg(long)]
        compare_frame: Option<u32>,
    },
    /// Report an actor model's bone hierarchy and rest pose.
    ///
    /// The rest pose is what a retarget measures against: a motion key
    /// replaces a bone's local frame, so the difference between two rigs is
    /// the difference between the poses they were bound in.
    SocProbeSkeleton {
        #[arg(long)]
        source_root: PathBuf,
        /// Archive-relative model path, e.g. `meshes/actors/mp_stalker.ogf`.
        #[arg(long)]
        model: String,
        /// Also report which of this bank's bones the model carries.
        #[arg(long)]
        bank: Option<String>,
    },
    /// Report which blender class every shader a level binds resolves to.
    ///
    /// Cross-checks the converter's reviewed render-family table against what
    /// `shaders.xr` actually records.
    SocAuditShaders {
        #[arg(long)]
        source_root: PathBuf,
        /// Level directory name, or `all`.
        #[arg(long, default_value = "all")]
        level: String,
    },
    /// Convert a compiled Shadow of Chernobyl level into a v0.100b bundle.
    BuildSocMap {
        #[arg(long)]
        manifest: PathBuf,
        /// Root of an unpacked Shadow of Chernobyl `gamedata` tree.
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// Move the client project's camera onto the spawn, where the
        /// offline client inserts the player.
        #[arg(long)]
        camera_at_spawn: bool,
    },
    /// Build an optional meadow, flower-island, lighting and victory-item layer for an X-Ray map.
    BuildSocMapAddon {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        level: String,
        /// Effective converted map database to patch.
        #[arg(long)]
        base_db: PathBuf,
        #[arg(long)]
        flower_png: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 16500)]
        grass_limit: usize,
        #[arg(long, default_value_t = 32400)]
        conifer_limit: usize,
        /// Centre of the wildflower island. The retail Steam Training Camp
        /// has only four standable cells around the earlier [-40, 189]; the
        /// nearest island with room for a full meadow is [-12, 138].
        #[arg(long, default_value_t = -12.0, allow_hyphen_values = true)]
        flower_center_x: f32,
        #[arg(long, default_value_t = 138.0, allow_hyphen_values = true)]
        flower_center_z: f32,
        #[arg(long, default_value_t = 30.0)]
        flower_radius: f32,
        #[arg(long, default_value_t = 900)]
        flower_limit: usize,
        #[arg(long, default_value_t = 10)]
        victory_item_count: usize,
        #[arg(long, default_value_t = 0.8)]
        skylight_multiplier: f32,
        #[arg(long, default_value_t = 0.18)]
        skylight_power: f32,
    },
    /// List installed Dirty Bomb cooked maps and their streamed packages.
    DirtyBombInventory {
        /// Dirty Bomb install root, or its `ShooterGame/CookedPC` directory.
        #[arg(long)]
        install_root: PathBuf,
        /// Restrict the inventory to one map (`Overground` and
        /// `EXE_Overground` are equivalent).
        #[arg(long)]
        map: Option<String>,
        /// Print every package rather than only the per-map totals.
        #[arg(long)]
        list_packages: bool,
    },
    /// Decode one complete Dirty Bomb streamed map graph and report port inputs.
    ///
    /// This validates the retail UE3 868/9 ABI, every LZO block and the object
    /// tables before counting geometry, collision, materials and spawn actors.
    DirtyBombProbeMap {
        /// Dirty Bomb install root, or its `ShooterGame/CookedPC` directory.
        #[arg(long)]
        source_root: PathBuf,
        /// Map directory name, with optional `EXE_` prefix.
        #[arg(long)]
        map: String,
        /// Write the complete stable report as JSON.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Convert Dirty Bomb mesh placements in memory and report target geometry.
    ///
    /// This is the no-database dry run for the Survarium build path: it applies
    /// visibility, material overrides, actor/component transforms, handedness
    /// conversion and per-section collision selection.
    DirtyBombProbeConversion {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        map: String,
        /// Write the stable conversion counters and material mapping as JSON.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Convert a cooked Dirty Bomb map graph into a Survarium v0.100b bundle.
    BuildDirtyBombMap {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Summarise an installed F.E.A.R. archive set.
    LilithInventory {
        /// The install root holding `Default.archcfg`; expansions in `FEARXP`
        /// and `FEARXP2` are mounted over it when present.
        #[arg(long)]
        install_root: PathBuf,
        /// Restrict the listing to normalized paths starting with this prefix.
        #[arg(long, default_value = "")]
        prefix: String,
        /// Print every matching path instead of a count.
        #[arg(long)]
        list: bool,
        /// List the multiplayer worlds with the archive each resolves to.
        #[arg(long)]
        list_worlds: bool,
    },
    /// Decode one F.E.A.R. world and report what a port would contain.
    ///
    /// Needs no target database, so it can run against a bare installation.
    LilithProbeWorld {
        #[arg(long)]
        source_root: PathBuf,
        /// A world path, a bare multiplayer world name, or `all`.
        #[arg(long)]
        world: String,
    },
    /// Extract one file from an installed F.E.A.R. archive set.
    LilithExtract {
        #[arg(long)]
        source_root: PathBuf,
        /// Archive-relative path, case-insensitive, either separator.
        #[arg(long)]
        path: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// List the collision under one point of a F.E.A.R. world, in metres.
    ///
    /// Reports both candidate sources -- the converted physics shapes and the
    /// brush BSP polygons -- so "I fell through the map" answers which source
    /// actually carries the floor.
    LilithProbeGround {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        world: String,
        #[arg(long, allow_hyphen_values = true)]
        x: f32,
        #[arg(long, allow_hyphen_values = true)]
        z: f32,
        /// Also list raw brush polygons over the column with their surface
        /// flags, for reverse-engineering what a flag bit means.
        #[arg(long)]
        bsp_flags: bool,
    },
    /// Judge every authored start point of a F.E.A.R. world for spawn use.
    ///
    /// The engine resolves a spawn onto the topmost collision over its
    /// column, so only a point whose column is open to the sky spawns where
    /// it was authored. Usable points print first.
    LilithSpawnCandidates {
        #[arg(long)]
        source_root: PathBuf,
        /// A world name, or `all` for every multiplayer world.
        #[arg(long, default_value = "all")]
        world: String,
    },
    /// Report every shader the multiplayer worlds' materials render with.
    ///
    /// The FEAR analogue of `soc-audit-shaders`: the `.fx` path is what the
    /// port's family classification keys on.
    LilithAuditShaders {
        #[arg(long)]
        source_root: PathBuf,
        /// A world name, or `all` for every multiplayer world.
        #[arg(long, default_value = "all")]
        world: String,
    },
    /// Dump one world's render geometry as Wavefront OBJ, for inspection.
    ///
    /// Positions are scaled to metres; surfaces become OBJ groups named by
    /// their material, so a viewer answers "which material is that wall".
    LilithDumpObj {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        world: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Convert a compiled F.E.A.R. multiplayer world into a v0.100b bundle.
    BuildLilithMap {
        #[arg(long)]
        manifest: PathBuf,
        /// The F.E.A.R. install root holding `Default.archcfg`.
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        target_db: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// Move the client project's camera onto the spawn, where the
        /// offline client inserts the player.
        #[arg(long)]
        camera_at_spawn: bool,
    },
    /// Compose overlays in priority order; earlier bundles supersede shared paths.
    Compose {
        #[arg(long = "bundle", required = true)]
        bundles: Vec<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Drop project collision objects whose collision cannot cook against the
    /// base database; rewrites the bundle in place and writes collision-report.json.
    PruneCollision {
        #[arg(long)]
        base_db: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Copy every file under the given prefixes out of a packed database as a bundle.
    ExtractBundle {
        #[arg(long)]
        db: PathBuf,
        #[arg(long = "prefix", required = true)]
        prefixes: Vec<String>,
        #[arg(long)]
        output: PathBuf,
    },
    /// List project collision objects in a packed database that cannot cook.
    AuditCollision {
        #[arg(long)]
        db: PathBuf,
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

fn print_config(root: &ConfigValue, leaves: bool) -> Result<()> {
    if leaves {
        print_config_leaves(root, "")?;
    } else {
        println!("{root:#?}");
    }
    Ok(())
}

fn print_config_leaves(value: &ConfigValue, path: &str) -> Result<()> {
    match value {
        ConfigValue::Named(values) => {
            for entry in values {
                let child_path = if path.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{path}.{}", entry.name)
                };
                print_config_leaves(&entry.value, &child_path)?;
            }
        }
        ConfigValue::Indexed(values) => {
            for (index, entry) in values.iter().enumerate() {
                print_config_leaves(entry, &format!("{path}[{index}]"))?;
            }
        }
        ConfigValue::Boolean(value) => println!("{path}\tbool:{value}"),
        ConfigValue::Integer(value) => println!("{path}\ti32:{value}"),
        ConfigValue::Float(value) => println!("{path}\tf32:{value:?}"),
        ConfigValue::String(value) => {
            println!("{path}\tstring:{}", serde_json::to_string(value)?)
        }
        ConfigValue::Float2(value) => {
            println!("{path}\tfloat2:{}", serde_json::to_string(value)?)
        }
        ConfigValue::Float3(value) => {
            println!("{path}\tfloat3:{}", serde_json::to_string(value)?)
        }
        ConfigValue::Float4(value) => {
            println!("{path}\tfloat4:{}", serde_json::to_string(value)?)
        }
    }
    Ok(())
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
        Command::DumpConfig { file, leaves } => {
            let config = BinaryConfig::parse(&std::fs::read(file)?)?;
            print_config(&config.root, leaves)?;
        }
        Command::DumpDbConfig { db, path, leaves } => {
            let mut archive = ResourceDb::open(db)?;
            let config = BinaryConfig::parse(&archive.read(&path)?)?;
            print_config(&config.root, leaves)?;
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
                    "{}\t{}\tstage_contract={}\tdomain_rank={:?}\tmismatches={}\tshared_path_tokens={}\teffects={}\tdonor_stages={}\ttarget_stages={}\ttarget_features={}",
                    candidate.path,
                    candidate.effect_match,
                    candidate.stage_contract_match,
                    candidate.semantic_domain_rank,
                    candidate.feature_mismatches,
                    candidate.shared_path_tokens,
                    serde_json::to_string(&candidate.effect_contract)?,
                    serde_json::to_string(&candidate.donor_stage_selectors)?,
                    serde_json::to_string(&candidate.target_stage_selectors)?,
                    serde_json::to_string(&candidate.target_features)?
                );
            }
        }
        Command::BuildMaterialPatch {
            source_db,
            target_db,
            materials,
            output,
        } => {
            material_patch::build_bundle(&source_db, &target_db, &materials, &output)?;
            println!("materials={}", materials.len());
            println!("bundle={}", output.display());
        }
        Command::BuildConfigPatch {
            source_db,
            resource,
            boolean_patches,
            retained_indexed_suffixes,
            output,
        } => {
            config_patch::build_bundle(
                &source_db,
                &resource,
                &boolean_patches,
                &retained_indexed_suffixes,
                &output,
            )?;
            println!("resource={resource}");
            println!("boolean_patches={}", boolean_patches.len());
            println!(
                "retained_indexed_suffixes={}",
                retained_indexed_suffixes.len()
            );
            println!("bundle={}", output.display());
        }
        Command::BuildFlatStaticBasisPatch {
            source_db,
            prefix,
            output,
        } => {
            static_basis_patch::build_flat_basis_bundle(&source_db, &prefix, &output)?;
            println!("prefix={prefix}");
            println!("bundle={}", output.display());
        }
        Command::BuildFlatStaticHeightPatch {
            source_db,
            prefix,
            output,
        } => {
            static_basis_patch::build_flat_height_bundle(&source_db, &prefix, &output)?;
            println!("prefix={prefix}");
            println!("bundle={}", output.display());
        }
        Command::BuildCanonicalStaticBasisAlphaPatch {
            source_db,
            prefix,
            output,
        } => {
            static_basis_patch::build_canonical_alpha_bundle(&source_db, &prefix, &output)?;
            println!("prefix={prefix}");
            println!("bundle={}", output.display());
        }
        Command::BuildStaticUvScalePatch {
            source_db,
            prefix,
            scale,
            output,
        } => {
            static_basis_patch::build_uv_scale_bundle(&source_db, &prefix, scale, &output)?;
            println!("prefix={prefix}");
            println!("scale={scale}");
            println!("bundle={}", output.display());
        }
        Command::BuildSingleSurfaceTerrainPatch {
            source_db,
            source_prefix,
            target_prefix,
            project,
            material,
            output,
        } => {
            terrain_surface_patch::build_single_surface_bundle(
                &source_db,
                &source_prefix,
                &target_prefix,
                &project,
                &material,
                &output,
            )?;
            println!("source_prefix={source_prefix}");
            println!("target_prefix={target_prefix}");
            println!("project={project}");
            println!("bundle={}", output.display());
        }
        Command::BuildResourceCopyPatch {
            source_db,
            copies,
            output,
        } => {
            resource_copy_patch::build_bundle(&source_db, &copies, &output)?;
            println!("copies={}", copies.len());
            println!("bundle={}", output.display());
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
            camera_at_spawn,
        } => {
            let mut manifest = MapPortManifest::load(manifest)?;
            manifest.camera_at_spawn = camera_at_spawn;
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
            sound_donor_db,
            target_db,
            output,
        } => {
            let manifest = Stk2MapSliceManifest::load(manifest)?;
            let record = map_port::build_stk2_map_slice_bundle(
                &manifest,
                &source_root,
                sound_donor_db.as_deref(),
                &target_db,
                &output,
            )?;
            println!("asset={}", record.asset_id);
            println!("map_name={}", record.map_name);
            println!("files={}", record.files.len());
            println!("bundle={}", output.display());
        }
        Command::SocInventory {
            install_root,
            prefix,
            list,
            dump_tables,
            list_game_materials,
        } => {
            if let Some(directory) = &dump_tables {
                std::fs::create_dir_all(directory)?;
                let mut archives: Vec<PathBuf> = std::fs::read_dir(&install_root)?
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                name.to_ascii_lowercase().starts_with("gamedata.db")
                            })
                    })
                    .collect();
                archives.sort();
                for archive in archives {
                    let table = soc::codec::archive::decoded_file_table(&archive)?;
                    let name = archive.file_name().unwrap().to_string_lossy().into_owned();
                    let output = directory.join(format!("{name}.table"));
                    std::fs::write(&output, &table)?;
                    println!("table={} bytes={}", output.display(), table.len());
                }
                return Ok(());
            }
            let set = soc::codec::archive::SocArchiveSet::open_installation(&install_root)?;
            for archive in set.archives() {
                println!(
                    "archive={} files={} key={:?} header={}",
                    archive.path().display(),
                    archive.entries().len(),
                    archive.key,
                    archive.header.is_some()
                );
            }
            println!("unique_paths={}", set.len());
            if list_game_materials {
                let mut source = soc::source::SocSource::open(&install_root)?;
                let library = vostok_resource_porter::soc::codec::gamemtl::parse_library(
                    &source.read_game_materials()?,
                )?;
                println!("game_materials={}", library.len());
                for (id, material) in &library {
                    println!("  {id:3} {:32} {:?}", material.name, material.flag_names());
                }
                return Ok(());
            }
            let prefix = prefix.replace('\\', "/").to_ascii_lowercase();
            let matching: Vec<&str> = set
                .names()
                .filter(|name| name.starts_with(&prefix))
                .collect();
            println!("matching={}", matching.len());
            if list {
                for name in matching {
                    println!("{name}");
                }
            }
        }
        Command::SocSpawnCandidates {
            source_root,
            level,
            cell,
            limit,
        } => {
            let mut source = soc::source::SocSource::open(&source_root)?;
            // The level's own multiplayer points come first: they are authored,
            // and the collision heuristic can land outside the arena.
            match source.read_level_file(&level, "level.spawn") {
                Ok(bytes) => {
                    for base in soc::codec::spawn::team_bases(&bytes) {
                        println!(
                            "team_base=[{:.2}, {:.2}, {:.2}] name={}",
                            base.position[0], base.position[1], base.position[2], base.name
                        );
                    }
                }
                Err(error) => println!("team_bases_unavailable: {error:#}"),
            }
            match source.read_level_file(&level, "level.game") {
                Ok(bytes) => match soc::codec::spawn::actor_respawns(&bytes) {
                    Ok(respawns) => {
                        for point in respawns {
                            println!(
                                "actor_respawn=[{:.2}, {:.2}, {:.2}] team={} game_type={:#06x}",
                                point.position[0],
                                point.position[1],
                                point.position[2],
                                point.team,
                                point.game_type
                            );
                        }
                    }
                    Err(error) => println!("actor_respawns_unavailable: {error:#}"),
                },
                Err(error) => println!("actor_respawns_unavailable: {error:#}"),
            }
            let cform = soc::build::decode_level(&mut source, &level)?.cform;
            let (min, max) = cform.header.bounding_box;
            println!(
                "extent x={:.2}..{:.2} y={:.2}..{:.2} z={:.2}..{:.2}",
                min[0], max[0], min[1], max[1], min[2], max[2]
            );
            for candidate in soc::convert::spawn::standable(&cform, cell, limit) {
                println!(
                    "spawn=[{:.2}, {:.2}, {:.2}] area={:.1} headroom={:.2}",
                    candidate.position[0],
                    candidate.position[1],
                    candidate.position[2],
                    candidate.area,
                    candidate.headroom
                );
            }
        }
        Command::SocProbeGround {
            source_root,
            level,
            x,
            z,
        } => {
            let mut source = soc::source::SocSource::open(&source_root)?;
            let library = soc::codec::gamemtl::parse_library(&source.read_game_materials()?)?;
            let cform = soc::build::decode_level(&mut source, &level)?.cform;

            // A triangle is under the point when the point is inside its
            // horizontal projection. Barycentric signs, so the winding does not
            // matter -- the source is not consistent about it.
            let mut hits = Vec::new();
            for face in &cform.faces {
                let [a, b, c] = face.vertices.map(|index| cform.vertices[index as usize]);
                let side = |p: [f32; 3], q: [f32; 3]| {
                    (q[0] - p[0]) * (z - p[2]) - (q[2] - p[2]) * (x - p[0])
                };
                let (ab, bc, ca) = (side(a, b), side(b, c), side(c, a));
                let inside =
                    (ab >= 0.0 && bc >= 0.0 && ca >= 0.0) || (ab <= 0.0 && bc <= 0.0 && ca <= 0.0);
                if !inside {
                    continue;
                }
                // Interpolate the height rather than averaging the corners: on
                // a ramp the difference is the whole answer.
                let area = (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2]);
                let height = if area.abs() < f32::EPSILON {
                    (a[1] + b[1] + c[1]) / 3.0
                } else {
                    (bc * a[1] + ca * b[1] + ab * c[1]) / (ab + bc + ca)
                };
                // Upward-facing means standable; the same triangle seen from
                // below is a ceiling.
                let normal_y = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
                hits.push((height, face.material, normal_y));
            }
            hits.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            println!("probe=[{x:.2}, ?, {z:.2}] triangles={}", hits.len());
            for (height, material_id, normal_y) in &hits {
                let material = library.get(material_id);
                let name = material.map_or("<unknown>", |material| material.name.as_str());
                let kept = material.is_none_or(soc::codec::gamemtl::GameMaterial::blocks_movement);
                println!(
                    "  y={height:8.2} material={name:<28} facing={:<7} converted={}",
                    if *normal_y >= 0.0 { "up" } else { "down" },
                    if kept { "yes" } else { "no (passable)" },
                );
            }
            if hits.is_empty() {
                println!("  nothing: the level has no collision over or under this point");
            }
        }
        Command::SocSurfaceStats {
            source_root,
            level,
            shader,
            x,
            z,
        } => {
            let mut source = soc::source::SocSource::open(&source_root)?;
            let decoded = soc::build::decode_level(&mut source, &level)?;
            let (surfaces, _) =
                soc::convert::surface::merge_visuals(&decoded.visuals, &decoded.buffers)?;
            for merged in &surfaces {
                let name = decoded
                    .shader_table
                    .get(merged.shader_id)
                    .map(|entry| entry.shader.as_str())
                    .unwrap_or("<unknown>");
                if shader
                    .as_ref()
                    .is_some_and(|want| !name.contains(want.as_str()))
                {
                    continue;
                }

                let mut point_hits = Vec::new();
                if let (Some(x), Some(z)) = (x, z) {
                    for triangle in merged.indices.chunks_exact(3) {
                        let at = |index: u16| merged.vertices[usize::from(index)].position;
                        let (a, b, c) = (at(triangle[0]), at(triangle[1]), at(triangle[2]));
                        let side = |p: [f32; 3], q: [f32; 3]| {
                            (q[0] - p[0]) * (z - p[2]) - (q[2] - p[2]) * (x - p[0])
                        };
                        let (ab, bc, ca) = (side(a, b), side(b, c), side(c, a));
                        if !((ab >= 0.0 && bc >= 0.0 && ca >= 0.0)
                            || (ab <= 0.0 && bc <= 0.0 && ca <= 0.0))
                        {
                            continue;
                        }
                        let denominator = ab + bc + ca;
                        let height = if denominator.abs() < f32::EPSILON {
                            (a[1] + b[1] + c[1]) / 3.0
                        } else {
                            (bc * a[1] + ca * b[1] + ab * c[1]) / denominator
                        };
                        point_hits.push(height);
                    }
                    if point_hits.is_empty() {
                        continue;
                    }
                }

                let mut min = [f32::INFINITY; 3];
                let mut max = [f32::NEG_INFINITY; 3];
                let mut uv_min = [f32::INFINITY; 2];
                let mut uv_max = [f32::NEG_INFINITY; 2];
                for vertex in &merged.vertices {
                    for axis in 0..3 {
                        min[axis] = min[axis].min(vertex.position[axis]);
                        max[axis] = max[axis].max(vertex.position[axis]);
                    }
                    for axis in 0..2 {
                        uv_min[axis] = uv_min[axis].min(vertex.uv[axis]);
                        uv_max[axis] = uv_max[axis].max(vertex.uv[axis]);
                    }
                }

                // Facing is taken from the winding, not from the stored normal:
                // a flat plane is all `up`, the walls of a volume are `side`.
                let (mut up, mut down, mut side) = (0_usize, 0_usize, 0_usize);
                for triangle in merged.indices.chunks_exact(3) {
                    let at = |index: u16| merged.vertices[usize::from(index)].position;
                    let (a, b, c) = (at(triangle[0]), at(triangle[1]), at(triangle[2]));
                    let edge0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let edge1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    let normal = [
                        edge0[1] * edge1[2] - edge0[2] * edge1[1],
                        edge0[2] * edge1[0] - edge0[0] * edge1[2],
                        edge0[0] * edge1[1] - edge0[1] * edge1[0],
                    ];
                    let length =
                        (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
                            .sqrt();
                    if !length.is_normal() {
                        continue;
                    }
                    match normal[1] / length {
                        cosine if cosine > 0.7 => up += 1,
                        cosine if cosine < -0.7 => down += 1,
                        _ => side += 1,
                    }
                }

                println!(
                    "{} shader={name} visuals={}/{} tris={} x={:.1}..{:.1} y={:.2}..{:.2} z={:.1}..{:.1} u={:.1}..{:.1} v={:.1}..{:.1} \
                     up={up} down={down} side={side} hits={point_hits:?}",
                    merged.name(),
                    merged.tree_visual_count,
                    merged.visual_count,
                    merged.indices.len() / 3,
                    min[0],
                    max[0],
                    min[1],
                    max[1],
                    min[2],
                    max[2],
                    uv_min[0],
                    uv_max[0],
                    uv_min[1],
                    uv_max[1],
                );
            }
        }
        Command::SocAuditBuffers { source_root, level } => {
            let mut source = soc::source::SocSource::open(&source_root)?;
            let geom = source.read_level_file(&level, "level.geom")?;
            let buffers = soc::codec::level::parse_geometry(&geom)?;
            println!("sliding_window_records={}", buffers.sliding_windows.len());
            for (index, vb) in buffers.vertex_buffers.iter().enumerate() {
                println!(
                    "vertex_buffer[{index}] vertices={} stride={} texcoord={:?}",
                    vb.count,
                    vb.layout.stride,
                    vb.layout.texcoord_kind()
                );
            }
        }
        Command::SocAuditVisuals { source_root, level } => {
            let mut source = soc::source::SocSource::open(&source_root)?;
            let bytes = source.read_level_file(&level, "level")?;
            let visuals = soc::codec::level::visual_payloads(&bytes)?;
            let mut by_kind: std::collections::BTreeMap<
                String,
                (usize, std::collections::BTreeSet<u32>),
            > = std::collections::BTreeMap::new();
            for payload in &visuals {
                let kind = match soc::codec::ogf::parse_visual(payload, 0) {
                    Ok(visual) => visual.kind.label(),
                    Err(_) => "<unparsed>".to_owned(),
                };
                let entry = by_kind.entry(kind).or_default();
                entry.0 += 1;
                entry.1.extend(soc::codec::ogf::visual_chunk_ids(payload));
            }
            println!("visuals={}", visuals.len());
            for (kind, (count, chunks)) in by_kind {
                let named: Vec<String> = chunks
                    .iter()
                    .map(|id| match id {
                        1 => "HEADER(1)".to_owned(),
                        9 => "CHILDREN(9)".to_owned(),
                        10 => "CHILDREN_L(10)".to_owned(),
                        11 => "LODDEF2(11)".to_owned(),
                        12 => "TREEDEF2(12)".to_owned(),
                        21 => "GCONTAINER(21)".to_owned(),
                        other => format!("chunk_{other}"),
                    })
                    .collect();
                println!("{kind}: count={count} chunks=[{}]", named.join(", "));
            }
        }
        Command::SocProbeLevel {
            source_root,
            level,
            output,
        } => {
            let mut source = soc::source::SocSource::open(&source_root)?;
            let levels = if level == "all" {
                source.levels()
            } else {
                vec![level]
            };
            let mut reports = Vec::with_capacity(levels.len());
            for name in &levels {
                match soc::build::probe_level(&mut source, name) {
                    Ok(report) => {
                        println!(
                            "{name}: visuals={} rendered={} trees={} surfaces={} triangles={} \
                             (tree_tris={} static_tris={} distinct_tris={} covered_tris={} ranges={}) shaders={} textures={} \
                             missing_textures={} collision={}v/{}t \
                             collision_materials={} size={:.0}x{:.0}x{:.0} y=[{:.1}..{:.1}] \
                             unclassified={:?} dropped={:?}",
                            report.visuals,
                            report.rendered_surfaces,
                            report.tree_surfaces,
                            report.converted_surfaces,
                            report.rendered_triangles,
                            report.tree_triangles,
                            report.rendered_triangles - report.tree_triangles,
                            report.distinct_source_triangles,
                            report.covered_source_triangles,
                            report.distinct_source_ranges,
                            report.bound_shaders,
                            report.source_textures,
                            report.missing_textures.len(),
                            report.collision_vertices,
                            report.collision_triangles,
                            report.collision_materials.len(),
                            report.world_size[0],
                            report.world_size[1],
                            report.world_size[2],
                            report.world_bounds.0[1],
                            report.world_bounds.1[1],
                            report.unclassified_shaders,
                            report.dropped_by_kind,
                        );
                        reports.push(report);
                    }
                    Err(error) => println!("{name}: FAILED {error:#}"),
                }
            }
            if let Some(output) = output {
                std::fs::write(&output, serde_json::to_vec_pretty(&reports)?)?;
                println!("report={}", output.display());
            }
        }
        Command::SocExtract {
            source_root,
            path,
            output,
        } => {
            let mut source = soc::source::SocSource::open(&source_root)?;
            let bytes = source.read(&path)?;
            std::fs::write(&output, &bytes)?;
            println!("path={path}");
            println!("bytes={}", bytes.len());
            println!("output={}", output.display());
        }
        Command::SocProbeMotions {
            source_root,
            bank,
            prefix,
            motion,
        } => {
            use vostok_resource_porter::soc::codec::omf;
            let mut source = soc::source::SocSource::open(&source_root)?;
            let bytes = source.read(&bank)?;
            let decoded = omf::parse(&bytes, &bank)?;
            println!("bank={bank}");
            println!("version={}", decoded.version);
            println!("bones={}", decoded.bones.len());
            for partition in &decoded.partitions {
                println!(
                    "partition={} bones={}",
                    partition.name,
                    partition.bones.len()
                );
            }
            println!("motions={}", decoded.motions.len());

            match motion {
                Some(name) => {
                    let motion = decoded
                        .motion(&name)
                        .with_context(|| format!("{bank}: no motion named {name}"))?;
                    println!(
                        "motion={} frames={} seconds={:.3} speed={:.3}",
                        motion.name,
                        motion.frames,
                        motion.duration(),
                        motion.speed
                    );
                    let animated = motion
                        .tracks
                        .iter()
                        .filter(|track| !track.is_static())
                        .count();
                    println!(
                        "animated_bones={animated} static_bones={}",
                        motion.tracks.len() - animated
                    );
                    for (index, track) in motion.tracks.iter().enumerate() {
                        println!(
                            "  {:<24} rotations={:<4} translations={:<4}{}",
                            decoded.bones[index],
                            track.rotations.len(),
                            track.translations.len(),
                            if track.is_static() { " static" } else { "" }
                        );
                    }
                }
                None => {
                    for motion in &decoded.motions {
                        if !motion.name.starts_with(&prefix) {
                            continue;
                        }
                        println!(
                            "motion={} frames={} seconds={:.3}",
                            motion.name,
                            motion.frames,
                            motion.duration()
                        );
                    }
                }
            }
        }
        Command::SocRetargetMotion {
            source_root,
            target_db,
            bank,
            model,
            motion,
            bind_pose,
            skeleton,
            output,
            compare_frame,
        } => {
            use vostok_resource_porter::skeleton_animation::Rig;
            use vostok_resource_porter::soc::codec::{omf, skeleton as soc_skeleton};
            use vostok_resource_porter::soc::convert::animation;

            let mut source = soc::source::SocSource::open(&source_root)?;
            let bank_bytes = source.read(&bank)?;
            let decoded = omf::parse(&bank_bytes, &bank)?;
            let model_bytes = source.read(&model)?;
            let source_skeleton = soc_skeleton::parse(&model_bytes, &model)?;

            let mut target = ResourceDb::open(&target_db)?;
            let rig = Rig::parse(&target.read(&bind_pose)?, &target.read(&skeleton)?)?;

            let motion = decoded
                .motion(&motion)
                .with_context(|| format!("{bank}: no motion named {motion}"))?;
            let (clip, report) = animation::retarget(motion, &decoded, &source_skeleton, &rig)?;

            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let spline = output.with_extension("b-spline");
            let names = output.with_extension("bones_names");
            std::fs::write(&spline, clip.to_bspline()?)?;
            std::fs::write(&names, clip.to_bones_names()?)?;

            println!("motion={}", report.motion);
            println!("samples={}", report.samples);
            println!("driven_bones={} held_bones={}", report.driven, report.held);
            println!("max_residual={:.6}", report.max_residual);
            println!(
                "alignment_yaw_degrees={:.2} alignment_residual={:.4}",
                report.alignment.yaw.to_degrees(),
                report.alignment.residual
            );
            println!("bones={}", clip.bones.len());
            println!(
                "b_spline={} bytes={}",
                spline.display(),
                std::fs::metadata(&spline)?.len()
            );
            println!(
                "bones_names={} bytes={}",
                names.display(),
                std::fs::metadata(&names)?.len()
            );

            if let Some(frame) = compare_frame {
                let samples = animation::compare_pose(
                    &clip,
                    &rig,
                    motion,
                    &decoded,
                    &source_skeleton,
                    frame,
                )?;
                println!("compare_frame={frame}");
                for sample in samples {
                    println!(
                        "  {:<16} -> {:<14} soc=[{:7.3},{:7.3},{:7.3}] target=[{:7.3},{:7.3},{:7.3}]",
                        sample.source_bone,
                        sample.target_bone,
                        sample.source[0],
                        sample.source[1],
                        sample.source[2],
                        sample.target[0],
                        sample.target[1],
                        sample.target[2],
                    );
                }
            }
        }
        Command::SocProbeSkeleton {
            source_root,
            model,
            bank,
        } => {
            use vostok_resource_porter::soc::codec::{omf, skeleton};
            let mut source = soc::source::SocSource::open(&source_root)?;
            let bytes = source.read(&model)?;
            let decoded = skeleton::parse(&bytes, &model)?;
            println!("model={model}");
            println!("bones={}", decoded.bones.len());
            let model_rest = decoded.model_rest();
            for (index, bone) in decoded.bones.iter().enumerate() {
                let parent = bone
                    .parent
                    .map(|parent| decoded.bones[parent].name.as_str())
                    .unwrap_or("-");
                let position = model_rest[index].translation();
                println!(
                    "  {:<20} parent={:<20} rest=[{:.4}, {:.4}, {:.4}] model=[{:.4}, {:.4}, {:.4}]",
                    bone.name,
                    parent,
                    bone.rest_translation[0],
                    bone.rest_translation[1],
                    bone.rest_translation[2],
                    position[0],
                    position[1],
                    position[2],
                );
            }
            if let Some(bank) = bank {
                let bytes = source.read(&bank)?;
                let bank_data = omf::parse(&bytes, &bank)?;
                let missing: Vec<&str> = bank_data
                    .bones
                    .iter()
                    .filter(|bone| decoded.index(bone).is_none())
                    .map(String::as_str)
                    .collect();
                println!("bank={bank} bank_bones={}", bank_data.bones.len());
                println!("bones_missing_from_model={}", missing.len());
                for bone in missing {
                    println!("  missing={bone}");
                }
            }
        }
        Command::SocAuditShaders { source_root, level } => {
            use vostok_resource_porter::soc::codec::{blender, level as soc_level};
            let mut source = soc::source::SocSource::open(&source_root)?;
            let library = blender::parse_library(&source.read_shaders()?)?;
            println!("shaders.xr blenders={}", library.len());

            let levels = if level == "all" {
                source.levels()
            } else {
                vec![level]
            };
            // shader name -> (class id tag, levels binding it)
            let mut bound: std::collections::BTreeMap<String, (String, Vec<String>)> =
                Default::default();
            let mut missing: std::collections::BTreeSet<String> = Default::default();
            for name in &levels {
                let Ok(bytes) = source.read_level_file(name, "level") else {
                    continue;
                };
                let Ok(table) = soc_level::parse_shaders(&bytes) else {
                    continue;
                };
                for id in 0..table.len() {
                    let Ok(entry) = table.get(id as u16) else {
                        continue;
                    };
                    match library.get(&entry.shader) {
                        Some(found) => bound
                            .entry(entry.shader.clone())
                            .or_insert_with(|| (found.class_id.tag(), Vec::new()))
                            .1
                            .push(name.clone()),
                        None => {
                            missing.insert(entry.shader.clone());
                        }
                    }
                }
            }

            let mut by_class: std::collections::BTreeMap<String, Vec<&String>> = Default::default();
            for (shader, (tag, _)) in &bound {
                by_class.entry(tag.clone()).or_default().push(shader);
            }
            println!("bound_shaders={} classes={}", bound.len(), by_class.len());
            for (tag, shaders) in &by_class {
                println!("class {tag}: {} shaders", shaders.len());
                for shader in shaders {
                    let levels = &bound[*shader].1;
                    let b = &library[*shader];
                    let p = b.common.as_ref();
                    println!(
                        "    {shader}  uses={} priority={} strict_sorting={} texture={:?}",
                        levels.len(),
                        p.map(|c| c.priority).unwrap_or(-1),
                        p.map(|c| c.strict_sorting).unwrap_or(false),
                        p.map(|c| c.texture.as_str()).unwrap_or("?"),
                    );
                }
            }
            if !missing.is_empty() {
                println!("NOT IN shaders.xr: {missing:?}");
            }
        }
        Command::BuildSocMap {
            manifest,
            source_root,
            target_db,
            output,
            camera_at_spawn,
        } => {
            let mut manifest = soc::SocMapManifest::load(manifest)?;
            manifest.camera_at_spawn = camera_at_spawn;
            let record = soc::build_bundle(&manifest, &source_root, &target_db, &output)?;
            println!("asset={}", record.asset_id);
            println!("source_level={}", record.source_level);
            println!("project={}", record.project_name);
            println!("models={}", record.models.len());
            println!("rendered_surfaces={}", record.rendered_surfaces);
            println!("converted_surfaces={}", record.converted_surfaces);
            println!("rendered_triangles={}", record.rendered_triangles);
            println!("tree_surfaces={}", record.tree_surfaces);
            println!("materials={}", record.materials);
            println!("textures={}", record.textures);
            println!("dropped_lightmaps={}", record.dropped_lightmaps);
            println!("collision_vertices={}", record.collision_vertices);
            println!("collision_triangles={}", record.collision_triangles);
            println!("collision_materials={}", record.collision_materials.len());
            for (kind, count) in &record.merge_report.dropped_by_kind {
                println!("dropped_visuals[{kind}]={count}");
            }
            println!("files={}", record.files.len());
            println!("bundle={}", output.display());
        }
        Command::BuildSocMapAddon {
            source_root,
            level,
            base_db,
            flower_png,
            output,
            grass_limit,
            conifer_limit,
            flower_center_x,
            flower_center_z,
            flower_radius,
            flower_limit,
            victory_item_count,
            skylight_multiplier,
            skylight_power,
        } => {
            let record = soc_addon::build_bundle(
                &source_root,
                &level,
                &base_db,
                &flower_png,
                &output,
                grass_limit,
                conifer_limit,
                [flower_center_x, flower_center_z],
                flower_radius,
                flower_limit,
                victory_item_count,
                skylight_multiplier,
                skylight_power,
            )?;
            println!("grass_instances={}", record.grass_instances);
            println!("grass_batches={}", record.grass_batches);
            println!("conifer_instances={}", record.conifer_instances);
            println!("conifer_batches={}", record.conifer_batches);
            println!("flower_instances={}", record.flower_instances);
            for (index, position) in record.victory_item_spawners.iter().enumerate() {
                println!(
                    "victory_item[{index}]=[{:.2}, {:.2}, {:.2}]",
                    position[0], position[1], position[2]
                );
            }
            for (index, position) in record.victory_item_containers.iter().enumerate() {
                println!(
                    "victory_item_container[{index}]=[{:.2}, {:.2}, {:.2}]",
                    position[0], position[1], position[2]
                );
            }
            println!("bundle={}", output.display());
        }
        Command::DirtyBombInventory {
            install_root,
            map,
            list_packages,
        } => {
            use vostok_resource_porter::dirty_bomb::{probe, source::DirtyBombSource};
            let source = DirtyBombSource::open(&install_root)?;
            let selected = map
                .as_deref()
                .map(|requested| source.resolve_map(requested))
                .transpose()?;
            for installed in probe::inventory(&source)? {
                if selected
                    .as_ref()
                    .is_some_and(|selected| !installed.name.eq_ignore_ascii_case(selected))
                {
                    continue;
                }
                println!(
                    "map={} packages={} disk_bytes={}",
                    installed.name, installed.package_count, installed.disk_bytes
                );
                if list_packages {
                    for package in installed.packages {
                        println!("  {}\t{}", package.path, package.disk_bytes);
                    }
                }
            }
        }
        Command::DirtyBombProbeMap {
            source_root,
            map,
            output,
        } => {
            use vostok_resource_porter::dirty_bomb::{probe, source::DirtyBombSource};
            let source = DirtyBombSource::open(&source_root)?;
            let report = probe::probe_map(&source, &map)?;
            println!(
                "map={} packages={} disk_bytes={} logical_bytes={}",
                report.map, report.package_count, report.disk_bytes, report.logical_bytes
            );
            println!(
                "names={} imports={} exports={} properties_decoded={} property_errors={}",
                report.names,
                report.imports,
                report.exports,
                report.properties_decoded,
                report.property_errors.values().sum::<usize>()
            );
            for name in [
                "StaticMesh",
                "StaticMeshActor",
                "StaticMeshComponent",
                "RB_BodySetup",
                "SDStitchingProxy",
                "Brush",
                "BrushComponent",
                "Model",
                "ModelComponent",
                "BlockingVolume",
                "SplineLoftActor",
                "Texture2D",
                "Material",
                "MaterialInstanceConstant",
                "PointLight",
                "SpotLight",
                "PlayerStart",
            ] {
                if let Some(count) = report.classes.get(name) {
                    println!("class[{name}]={count}");
                }
            }
            println!("external_packages={}", report.external_packages.len());
            println!(
                "streaming_levels={} missing={} undeclared={}",
                report.declared_streaming_levels.len(),
                report.missing_streaming_levels.len(),
                report.undeclared_sublevels.len()
            );
            println!(
                "static_mesh_references={} unresolved={}",
                report.static_mesh_references.len(),
                report.unresolved_static_mesh_references
            );
            println!(
                "static_meshes_decoded={} errors={} lod0_vertices={} lod0_triangles={} lod0_sections={}",
                report.static_meshes_decoded,
                report.static_mesh_errors.values().sum::<usize>(),
                report.static_mesh_lod0_vertices,
                report.static_mesh_lod0_triangles,
                report.static_mesh_lod0_sections
            );
            for (class, bounds) in &report.location_bounds {
                if matches!(
                    class.as_str(),
                    "StaticMeshActor"
                        | "InterpActor"
                        | "BlockingVolume"
                        | "SplineLoftActor"
                        | "PlayerStart"
                ) {
                    println!(
                        "bounds[{class}]={} [{:.2},{:.2},{:.2}]..[{:.2},{:.2},{:.2}]m",
                        bounds.count,
                        bounds.min_vostok_m[0],
                        bounds.min_vostok_m[1],
                        bounds.min_vostok_m[2],
                        bounds.max_vostok_m[0],
                        bounds.max_vostok_m[1],
                        bounds.max_vostok_m[2]
                    );
                }
            }
            for start in &report.player_starts {
                println!(
                    "player_start={} [{:.2},{:.2},{:.2}]m package={}",
                    start.object,
                    start.location_vostok_m[0],
                    start.location_vostok_m[1],
                    start.location_vostok_m[2],
                    start.package
                );
            }
            if let Some(output) = output {
                probe::write_json(&report, &output)?;
                println!("report={}", output.display());
            }
        }
        Command::DirtyBombProbeConversion {
            source_root,
            map,
            output,
        } => {
            use vostok_resource_porter::dirty_bomb::{convert, source::DirtyBombSource};
            let source = DirtyBombSource::open(&source_root)?;
            let converted = convert::convert_map(&source, &map)?;
            let report = &converted.report;
            println!(
                "map={} packages={} placements={} visible={} rendered={} imported_skipped={}",
                converted.map,
                report.packages,
                report.placements,
                report.visible_placements,
                report.rendered_placements,
                report.skipped_imported_mesh_placements
            );
            println!(
                "mirrored={} absolute_translation={} absolute_rotation={} absolute_scale={}",
                report.mirrored_placements,
                report.absolute_translation_placements,
                report.absolute_rotation_placements,
                report.absolute_scale_placements
            );
            println!(
                "surfaces={} materials={} render_sections={} render_vertices={} render_triangles={}",
                converted.surfaces.len(),
                report.materials.len(),
                report.render_sections,
                report.render_vertices,
                report.render_triangles
            );
            println!(
                "bsp_models={} bsp_nodes={} rendered={}/{}v/{}t collision={}/{}v/{}t",
                report.bsp_models,
                report.bsp_nodes,
                report.bsp_render_nodes,
                report.bsp_render_vertices,
                report.bsp_render_triangles,
                report.bsp_collision_nodes,
                report.bsp_collision_vertices,
                report.bsp_collision_triangles
            );
            println!(
                "collision_sections={} collision_vertices={} collision_triangles={} degenerate_dropped={} background_dropped={}",
                report.collision_sections,
                report.collision_vertices,
                report.collision_triangles,
                report.collision_degenerate_triangles_dropped,
                report.background_collision_triangles_dropped
            );
            if let Some(worst) = report
                .reported_bounds_comparisons
                .iter()
                .max_by(|left, right| {
                    left.outside_distance_metres
                        .total_cmp(&right.outside_distance_metres)
                })
            {
                let outside_tolerance = report
                    .reported_bounds_comparisons
                    .iter()
                    .filter(|bounds| bounds.outside_distance_metres > 0.25)
                    .count();
                println!(
                    "reported_bounds={} outside_25cm={} max_outside={:.3}m actor={} class={}",
                    report.reported_bounds_comparisons.len(),
                    outside_tolerance,
                    worst.outside_distance_metres,
                    worst.actor,
                    worst.actor_class
                );
            }
            for candidate in &report.spawn_candidates {
                if let Some(position) = candidate.target_position {
                    println!(
                        "spawn={} actor=[{:.2},{:.2},{:.2}] floor={:.2} target=[{:.2},{:.2},{:.2}]",
                        candidate.object,
                        candidate.actor_position[0],
                        candidate.actor_position[1],
                        candidate.actor_position[2],
                        candidate.floor.unwrap(),
                        position[0],
                        position[1],
                        position[2]
                    );
                }
            }
            if let Some(output) = output {
                std::fs::write(&output, serde_json::to_vec_pretty(report)?)
                    .with_context(|| format!("writing {}", output.display()))?;
                println!("report={}", output.display());
            }
        }
        Command::BuildDirtyBombMap {
            manifest,
            source_root,
            target_db,
            output,
        } => {
            let manifest = soc::SocMapManifest::load(manifest)?;
            let record = vostok_resource_porter::dirty_bomb::build::build_bundle(
                &manifest,
                &source_root,
                &target_db,
                &output,
            )?;
            println!("asset={}", record.asset_id);
            println!("source_map={}", record.source_map);
            println!("project={}", record.project_name);
            println!("models={}", record.models.len());
            println!("converted_surfaces={}", record.converted_surfaces);
            println!("rendered_triangles={}", record.rendered_triangles);
            println!("materials={}", record.materials);
            println!("placeholder_materials={}", record.placeholder_materials);
            println!(
                "unknown_materials={}",
                record
                    .material_assets
                    .material_families
                    .values()
                    .filter(
                        |family| **family == soc::convert::render_material::RenderFamily::Unknown
                    )
                    .count()
            );
            println!(
                "material_errors={}",
                record.material_assets.material_failures.len()
            );
            println!("collision_pieces={}", record.collision_pieces);
            println!("collision_vertices={}", record.collision_vertices);
            println!("collision_triangles={}", record.collision_triangles);
            println!("files={}", record.files.len());
            println!("bundle={}", output.display());
        }
        Command::LilithInventory {
            install_root,
            prefix,
            list,
            list_worlds,
        } => {
            use vostok_resource_porter::lilith::source::LilithSource;
            let source = LilithSource::open(&install_root)?;
            for archive in source.archives() {
                println!(
                    "archive={} files={}",
                    archive.path().display(),
                    archive.entries().len()
                );
            }
            let prefix = vostok_resource_porter::lilith::codec::arch::normalize(&prefix);
            let matching = source
                .paths()
                .filter(|(path, _)| path.starts_with(prefix.as_str()));
            if list {
                for (path, entry) in matching {
                    println!("{path}\t{}", entry.size);
                }
            } else {
                println!("paths={}", matching.count());
            }
            if list_worlds {
                for world in source.multiplayer_worlds() {
                    let archive = source
                        .winning_archive(&world)
                        .map(|path| path.display().to_string())
                        .unwrap_or_default();
                    let size = source.entry(&world).map(|entry| entry.size).unwrap_or(0);
                    println!("world={world}\tbytes={size}\tarchive={archive}");
                }
            }
        }
        Command::LilithProbeWorld { source_root, world } => {
            use vostok_resource_porter::lilith::{probe, source::LilithSource};
            let mut source = LilithSource::open(&source_root)?;
            let worlds = if world == "all" {
                source.multiplayer_worlds()
            } else {
                vec![probe::resolve_world(&source, &world)?]
            };
            for path in worlds {
                println!("== {path}");
                let report = match probe::probe_world(&mut source, &path) {
                    Ok(report) => report,
                    Err(error) => {
                        println!("error={error:#}");
                        continue;
                    }
                };
                println!(
                    "bounds_m=[{:.1},{:.1},{:.1}]..[{:.1},{:.1},{:.1}]",
                    report.min[0],
                    report.min[1],
                    report.min[2],
                    report.max[0],
                    report.max[1],
                    report.max[2]
                );
                println!("world_models={}", report.world_models);
                for (kind, count) in &report.physics_shapes {
                    println!("physics[{kind}]={count}");
                }
                println!("physics_triangles={}", report.physics_triangles);
                println!(
                    "render surfaces={} materials={} vertices={} triangles={}",
                    report.render_surfaces,
                    report.render_materials,
                    report.render_vertices,
                    report.render_triangles
                );
                for (index, layout) in report.layouts.iter().enumerate() {
                    println!("layout[{index}]={layout}");
                }
                for (class, count) in &report.objects_by_class {
                    println!("object[{class}]={count}");
                }
                for (name, position) in &report.start_points {
                    println!(
                        "start_point=[{:.2}, {:.2}, {:.2}] name={}",
                        position[0],
                        position[1],
                        position[2],
                        name.as_deref().unwrap_or("?")
                    );
                }
                println!("streaming_assets={}", report.streaming_assets);
            }
        }
        Command::LilithExtract {
            source_root,
            path,
            output,
        } => {
            use vostok_resource_porter::lilith::source::LilithSource;
            let mut source = LilithSource::open(&source_root)?;
            let bytes = source.read(&path)?;
            std::fs::write(&output, &bytes)
                .with_context(|| format!("writing {}", output.display()))?;
            println!("extracted={} bytes={}", output.display(), bytes.len());
        }
        Command::LilithProbeGround {
            source_root,
            world,
            x,
            z,
            bsp_flags,
        } => {
            use vostok_resource_porter::lilith::{probe, source::LilithSource};
            let mut source = LilithSource::open(&source_root)?;
            let path = probe::resolve_world(&source, &world)?;
            let layers = probe::probe_ground(&mut source, &path, x, z)?;
            if layers.is_empty() {
                println!("no collision over [{x}, {z}] from either source");
            }
            for layer in &layers {
                println!(
                    "y={:8.2}  {}  {}",
                    layer.y,
                    if layer.upward { "up  " } else { "down" },
                    layer.source
                );
            }
            if bsp_flags {
                for (y, flags, bsp) in probe::probe_bsp_flags(&mut source, &path, x, z)? {
                    println!("bsp_poly y={y:8.2} flags={flags:#06x} bsp={bsp}");
                }
            }
        }
        Command::LilithSpawnCandidates { source_root, world } => {
            use vostok_resource_porter::lilith::{probe, source::LilithSource};
            let mut source = LilithSource::open(&source_root)?;
            let worlds = if world == "all" {
                source.multiplayer_worlds()
            } else {
                vec![probe::resolve_world(&source, &world)?]
            };
            for path in worlds {
                println!("== {path}");
                match probe::spawn_candidates(&mut source, &path) {
                    Ok(candidates) => {
                        let usable = candidates.iter().filter(|c| c.usable).count();
                        println!("start_points={} usable={usable}", candidates.len());
                        for candidate in &candidates {
                            let height = |value: Option<f32>| {
                                value
                                    .map(|value| format!("{value:.2}"))
                                    .unwrap_or_else(|| "none".to_owned())
                            };
                            println!(
                                "{} [{:.2}, {:.2}, {:.2}] floor={} ceiling={} spawn_y={} name={}",
                                if candidate.usable {
                                    "usable "
                                } else {
                                    "no-fit "
                                },
                                candidate.position[0],
                                candidate.position[1],
                                candidate.position[2],
                                height(candidate.floor),
                                height(candidate.ceiling),
                                height(candidate.adjusted.map(|adjusted| adjusted[1])),
                                candidate.name.as_deref().unwrap_or("?"),
                            );
                        }
                    }
                    Err(error) => println!("error={error:#}"),
                }
            }
        }
        Command::LilithAuditShaders { source_root, world } => {
            use vostok_resource_porter::lilith::{probe, source::LilithSource};
            let mut source = LilithSource::open(&source_root)?;
            let worlds = if world == "all" {
                source.multiplayer_worlds()
            } else {
                vec![probe::resolve_world(&source, &world)?]
            };
            let census = probe::audit_shaders(&mut source, &worlds)?;
            for (shader, usage) in &census {
                println!(
                    "{shader}\tmaterials={} diffuse={} normal={} specular={}\te.g. {}",
                    usage.materials,
                    usage.with_diffuse,
                    usage.with_normal,
                    usage.with_specular,
                    usage.example
                );
            }
        }
        Command::LilithDumpObj {
            source_root,
            world,
            output,
        } => {
            use vostok_resource_porter::lilith::{probe, source::LilithSource};
            let mut source = LilithSource::open(&source_root)?;
            let path = probe::resolve_world(&source, &world)?;
            let (surfaces, vertices, triangles) = probe::dump_obj(&mut source, &path, &output)?;
            println!("world={path}");
            println!("surfaces={surfaces} vertices={vertices} triangles={triangles}");
            println!("obj={}", output.display());
        }
        Command::BuildLilithMap {
            manifest,
            source_root,
            target_db,
            output,
            camera_at_spawn,
        } => {
            let mut manifest = soc::SocMapManifest::load(manifest)?;
            manifest.camera_at_spawn = camera_at_spawn;
            let record = vostok_resource_porter::lilith::build::build_bundle(
                &manifest,
                &source_root,
                &target_db,
                &output,
            )?;
            println!("asset={}", record.asset_id);
            println!("source_world={}", record.source_world);
            println!("project={}", record.project_name);
            println!("models={}", record.models.len());
            println!("converted_surfaces={}", record.converted_surfaces);
            println!("rendered_triangles={}", record.rendered_triangles);
            println!("materials={}", record.materials);
            println!("textures={}", record.textures);
            println!("bound_normal_maps={}", record.bound_normal_maps);
            println!("collision_pieces={}", record.collision_pieces);
            println!("collision_vertices={}", record.collision_vertices);
            println!("collision_triangles={}", record.collision_triangles);
            for (shader, surfaces) in &record.dropped_materials {
                println!("dropped_surfaces[{shader}]={surfaces}");
            }
            println!(
                "collision render={} bsp={} blockers={} passable_surfaces={} empty_models={}",
                record.collision_report.render_triangles,
                record.collision_report.bsp_triangles,
                record.collision_report.blocker_triangles,
                record.collision_report.passable_surfaces,
                record.collision_report.empty_models
            );
            println!("start_points={}", record.start_points.len());
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
        Command::ExtractBundle { db, prefixes, output } => {
            let count = weapon::extract_bundle(&db, &prefixes, &output)?;
            println!("files={count}");
            println!("bundle={}", output.display());
        }
        Command::AuditCollision { db } => {
            let report = vostok_resource_porter::collision_check::audit_database(&db)?;
            for (project, checked) in &report.checked {
                let dropped = report.dropped.get(project).map_or(&[][..], Vec::as_slice);
                println!("{project} checked={checked} uncookable={}", dropped.len());
                for name in dropped {
                    println!("  {name}");
                }
            }
        }
        Command::PruneCollision { base_db, bundle } => {
            let report =
                vostok_resource_porter::collision_check::prune_uncookable_collision(&base_db, &bundle)?;
            for (project, checked) in &report.checked {
                let dropped = report.dropped.get(project).map_or(0, Vec::len);
                println!("{project} checked={checked} dropped={dropped}");
            }
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
