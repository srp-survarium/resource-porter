// SPDX-License-Identifier: GPL-3.0-or-later
//! `draw-lobby-bake`: a built lobby pack drawn from its path-traced bake
//! (`scripts/lobby-bake`, `bake-pack.sh`), whichever builder made the pack.
//!
//! What `build-stk2-lobby-026 --bake-dir` does inside the STK2 builder, done
//! to the pack's files: the bake's pages become textures and materials
//! (black, unshiny and emitting the light, `set_baked`; or, a hybrid bake,
//! the albedo lit by the project's lights and the indirect light emitted,
//! `set_hybrid`), the baked surfaces one
//! model at the lobby's origin, and the project loses the placements the bake
//! holds (a placement with surfaces the engine still draws keeps those in a
//! model of its own: glass and forward decals the bake left as they were, and
//! the parts inside the unbaked zones, where the characters and the weapon
//! cast their shadows: prep.py cut them out), the projected decals
//! painted into the pages, and the ambient-light boxes inside the lobby (a
//! bounce or a contact shadow the trace now makes); the probes the bake
//! traced get its cubemaps. The pack's files nothing names any more are left
//! out. The bake is refused unless it was made of this very project and
//! these surfaces (prep.py records their sha256).

use super::*;

use crate::archive::folder_files;
use crate::probe_bake::Cube;

/// The page materials' templates and the textures' and models' (the
/// bunker's picks: stock, so every lobby's pages draw as its do).
const PAGE_TEMPLATE: &str = "tiled/concrete/concrete_01";
const ALPHA_PAGE_TEMPLATE: &str = "tiled/roof/roof_03_alpha_2side";
const TEXTURE_OPTIONS_TEMPLATE: &str = "textures/tiled/concrete/concrete_01.options";
const MODEL_SETTINGS_TEMPLATE: &str = "models/level/lobby/base.model/settings";
const MODEL_RENDER_TEMPLATE: &str = "models/level/lobby/base.model/render/export_properties";
const SURFACE_TEMPLATE: &str = "models/level/lobby/base.model/render/conc_low_s/export_properties";

pub struct DrawOptions<'a> {
    /// The built pack: `files/` (and `pack.json`, kept for its description,
    /// requirements and server part).
    pub pack: &'a Path,
    /// bake-pack.sh's folder: prep.py's scene.json, bake.py's baked.json,
    /// pages, baked models and probes.
    pub bake_dir: &'a Path,
    /// The stock 0.26g resources.db, and patches over it: the templates, the
    /// files the pack does not hold, and what a file the pack replaces was.
    pub target_db: &'a Path,
    pub base_patches: &'a [PathBuf],
    /// A new, empty folder.
    pub output: &'a Path,
    /// Scratch space for the page conversions.
    pub work: &'a Path,
    pub id: &'a str,
    pub version: &'a str,
    /// Where what this step makes goes: `<prefix>/baked_v<version>/...`.
    pub prefix: &'a str,
}

pub struct DrawReport {
    pub files: usize,
    pub notes: Vec<String>,
}

/// prep.py's scene.json, what this step reads of it.
#[derive(Deserialize)]
struct SceneJson {
    /// The project baked (a pack's: build-stk2-lobby-026's slices have none).
    project: Option<SceneProject>,
    surfaces: Vec<SceneSurface>,
    #[serde(default)]
    decals: Vec<Named>,
    /// The surfaces' parts inside the unbaked zones (prep.py).
    #[serde(default)]
    unbaked: Vec<UnbakedPart>,
    #[serde(default, rename = "box")]
    seen: Option<[[f32; 3]; 2]>,
    #[serde(default)]
    zones: Vec<Named>,
    /// The project's lights the trace and the project leave out (fills).
    #[serde(default)]
    dropped_lights: BTreeSet<String>,
    /// Lights given another colour and intensity (a lamp's temperature).
    #[serde(default)]
    relit: BTreeMap<String, Relit>,
    #[serde(default)]
    lights: Vec<SceneLight>,
    /// A hybrid's indirect pages emitted this much brighter.
    #[serde(default = "one")]
    indirect_scale: f32,
}

#[derive(Deserialize)]
struct Relit {
    color: [f32; 3],
    intensity: f32,
}

/// A light of the trace; those prep.py added (`bake_lights/...`, a light in
/// a stove) a hybrid's project gets too.
#[derive(Deserialize)]
struct SceneLight {
    name: String,
    position: [f32; 3],
    color: Vec<f32>,
    intensity: f32,
    range: f32,
    attenuation_power: f32,
    #[serde(default)]
    is_cast_shadow: bool,
}

/// The prefix of the lights prep.py adds to the trace (spec `bake_lights`).
const BAKE_LIGHTS: &str = "bake_lights/";

#[derive(Deserialize)]
struct SceneProject {
    path: String,
    sha256: String,
}

#[derive(Deserialize)]
struct SceneSurface {
    name: String,
    source: Option<SurfaceSource>,
}

/// A baked surface's origin: the project's `render_visuals[index]`, its
/// model's surface and that surface's converted_model sha256.
#[derive(Deserialize)]
struct SurfaceSource {
    index: usize,
    visual: String,
    surface: String,
    sha256: String,
}

/// A surface's part the engine draws: the file prep.py cut (in the bake's
/// folder), none for the whole surface.
#[derive(Deserialize)]
struct UnbakedPart {
    #[serde(flatten)]
    source: SurfaceSource,
    file: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

/// A placement the bake holds: its surfaces baked (whole or in part), and
/// those, or their parts, the engine still draws.
struct Placement {
    visual: String,
    baked: BTreeSet<String>,
    drawn: BTreeMap<String, Drawn>,
}

enum Drawn {
    /// The model's own surface (glass, a forward decal, or one inside a zone).
    Whole,
    /// The part inside the unbaked zones: prep.py's file, in the bake's folder.
    Part(PathBuf),
}

impl Placement {
    /// What the engine still draws, if not the whole placement as it was.
    fn engine_model(&self, index: usize, made: &str) -> Option<String> {
        if self.baked.is_empty() || self.drawn.is_empty() {
            return None;
        }
        Some(if self.drawn.values().all(|d| matches!(d, Drawn::Whole)) {
            // the same surfaces of the same model: one model for every placement
            format!("{made}/drawn/{}/{}", self.visual, self.drawn.keys().cloned().collect::<Vec<_>>().join("-"))
        } else {
            format!("{made}/drawn/{}_{index:03}", self.visual)
        })
    }
}

/// The input pack's files and the database under them.
struct Source<'a> {
    files: BTreeMap<String, PathBuf>,
    db: &'a mut ResourceDb,
}

impl Source<'_> {
    fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        match self.files.get(path) {
            Some(file) => fs::read(file).with_context(|| format!("reading {}", file.display())),
            None => self.db.read(path).with_context(|| format!("{path}: neither in the pack nor the database")),
        }
    }

    fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.db.contains(path)
    }

    /// The paths under `prefix/`, the pack's and the database's.
    fn under(&self, prefix: &str) -> BTreeSet<String> {
        let folder = format!("{prefix}/");
        let mut out: BTreeSet<String> =
            self.files.range(folder.clone()..).take_while(|(p, _)| p.starts_with(&folder)).map(|(p, _)| p.clone()).collect();
        out.extend(self.db.paths_with_prefix(&folder).map(str::to_owned));
        out
    }
}

pub fn draw(options: &DrawOptions<'_>) -> Result<DrawReport> {
    crate::weapon::prepare_empty_output(options.output)?;
    fs::create_dir_all(options.work)?;
    crate::content_pack::validate_resource_path(options.prefix)?;
    let mut db = ResourceDb::open_layers(&crate::content_pack::patch::base_layers(
        options.target_db,
        options.base_patches,
    )?)?;
    let input: Option<PackManifest> = match fs::read(options.pack.join("pack.json")) {
        Ok(bytes) => Some(serde_json::from_slice(&bytes).context("the pack's pack.json")?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).context("reading the pack's pack.json"),
    };
    let mut source = Source { files: folder_files(&options.pack.join("files"))?, db: &mut db };
    let read_json = |name: &str| -> Result<Vec<u8>> {
        fs::read(options.bake_dir.join(name))
            .with_context(|| format!("the bake: {}/{name} (scripts/lobby-bake/bake-pack.sh)", options.bake_dir.display()))
    };
    let scene: SceneJson = serde_json::from_slice(&read_json("scene.json")?).context("the bake's scene.json")?;
    let baked: BakedJson = serde_json::from_slice(&read_json("baked.json")?).context("the bake's baked.json")?;
    ensure!(baked.emission.is_finite() && baked.emission > 0.0, "the bake's emission is not positive");

    // The bake must be of this project and these surfaces.
    let project_record = scene.project.as_ref().context(
        "the bake is of a build-stk2-lobby-026 slice (no project in its scene.json): that builder draws it (--bake-dir)",
    )?;
    let project_bytes = source.read(&project_record.path)?;
    ensure!(
        crate::content_pack::sha256_hex(&project_bytes) == project_record.sha256,
        "{}: not the project the bake was made of: bake the pack again",
        project_record.path
    );
    let names: BTreeSet<&String> = scene.surfaces.iter().map(|s| &s.name).collect();
    ensure!(
        names == baked.sources.keys().collect::<BTreeSet<_>>(),
        "the bake's baked.json is not of its scene.json: bake again"
    );
    let kept: BTreeSet<&String> = baked.kept.iter().collect();
    let mut placements: BTreeMap<usize, Placement> = BTreeMap::new();
    for surface in &scene.surfaces {
        let from = surface.source.as_ref().with_context(|| format!("{}: no source in scene.json (prep.py)", surface.name))?;
        let file = format!("models/{}.model/render/{}/converted_model", from.visual, from.surface);
        ensure!(
            crate::content_pack::sha256_hex(&source.read(&file)?) == from.sha256,
            "{file}: not the surface the bake was made of: bake the pack again"
        );
        let placement = placements.entry(from.index).or_insert_with(|| Placement {
            visual: from.visual.clone(),
            baked: BTreeSet::new(),
            drawn: BTreeMap::new(),
        });
        ensure!(placement.visual == from.visual, "render_visuals[{}]: two models in the bake", from.index);
        if kept.contains(&surface.name) {
            placement.drawn.insert(from.surface.clone(), Drawn::Whole);
        } else {
            placement.baked.insert(from.surface.clone());
        }
    }
    for part in &scene.unbaked {
        let from = &part.source;
        let file = format!("models/{}.model/render/{}/converted_model", from.visual, from.surface);
        ensure!(
            crate::content_pack::sha256_hex(&source.read(&file)?) == from.sha256,
            "{file}: not the surface the bake was made of: bake the pack again"
        );
        let placement = placements.entry(from.index).or_insert_with(|| Placement {
            visual: from.visual.clone(),
            baked: BTreeSet::new(),
            drawn: BTreeMap::new(),
        });
        ensure!(placement.visual == from.visual, "render_visuals[{}]: two models in the bake", from.index);
        let drawn = part.file.as_ref().map_or(Drawn::Whole, |f| Drawn::Part(f.clone()));
        ensure!(
            placement.drawn.insert(from.surface.clone(), drawn).is_none(),
            "render_visuals[{}]: {} drawn twice",
            from.index,
            from.surface
        );
    }

    let version = options.version;
    let made = format!("{}/baked_v{version}", options.prefix);
    let mut pack = PackWriter {
        files: options.output.join("files"),
        paths: BTreeSet::new(),
        materials_used: BTreeSet::new(),
    };
    let mut notes = Vec::new();
    let templates = StaticModelTemplates {
        settings: parse(source.db, MODEL_SETTINGS_TEMPLATE)?,
        render_properties: parse(source.db, MODEL_RENDER_TEMPLATE)?,
        surface_properties: parse(source.db, SURFACE_TEMPLATE)?,
    };

    // The pages: textures, and materials emitting them.
    let tree = Stk2Tree { root: Path::new(""), dry: false };
    let mut textures = TextureMaker {
        stk2: &tree,
        prefix: &made,
        dir: "",
        more_dirs: &[],
        max_size: 8192,
        caps: &[],
        work: options.work,
        options_template: parse(source.db, TEXTURE_OPTIONS_TEMPLATE)?,
        made: BTreeMap::new(),
        failed: Vec::new(),
    };
    let mut materials = BTreeMap::new();
    // a hybrid bake's flat normal and roughness maps (the template's would
    // tile over the pages' layout)
    let flat_normal = match &baked.flat {
        Some(flat) => {
            Some(textures.make_page_as("flat_normal", &options.bake_dir.join(&flat.normal), TextureKind::Normal, false, &mut pack)?)
        }
        None => None,
    };
    // the flat roughness only for a page without its own gloss
    let mut flat_roughness: Option<String> = None;
    for page in &baked.pages {
        let template = if page.alpha { ALPHA_PAGE_TEMPLATE } else { PAGE_TEMPLATE };
        let mut material = parse(source.db, &format!("material_instances/{template}.material"))?;
        let texture = match &page.light {
            // the light the surfaces send, emitted by a black, unshiny material
            BakedLightJson::Radiance { png } => {
                let texture = textures.make_page(&page.name, &options.bake_dir.join(png), page.alpha, &mut pack)?;
                set_baked(&mut material.root, &texture, baked.emission)
                    .with_context(|| format!("{template}: the baked material"))?;
                texture
            }
            // the albedo lit by the project's lights, the indirect light emitted
            BakedLightJson::Hybrid { albedo, indirect, gloss } => {
                let normal = flat_normal.as_ref().context("a hybrid page without the bake's flat maps")?;
                let texture = textures.make_page(&page.name, &options.bake_dir.join(albedo), page.alpha, &mut pack)?;
                let light = textures.make_page(
                    &format!("{}_indirect", page.name),
                    &options.bake_dir.join(indirect),
                    false,
                    &mut pack,
                )?;
                // the surfaces' own gloss, else the flat stand-in
                let roughness = match gloss {
                    Some(gloss) => textures.make_page_as(
                        &format!("{}_gloss", page.name),
                        &options.bake_dir.join(gloss),
                        TextureKind::Linear,
                        false,
                        &mut pack,
                    )?,
                    None => match &flat_roughness {
                        Some(made) => made.clone(),
                        None => {
                            let flat = baked.flat.as_ref().context("a hybrid page without the bake's flat maps")?;
                            let made = textures.make_page_as(
                                "flat_roughness",
                                &options.bake_dir.join(&flat.roughness),
                                TextureKind::Linear,
                                false,
                                &mut pack,
                            )?;
                            flat_roughness = Some(made.clone());
                            made
                        }
                    },
                };
                set_hybrid(
                    &mut material.root,
                    &texture,
                    &light,
                    baked.emission * scene.indirect_scale,
                    normal,
                    &roughness,
                )
                .with_context(|| format!("{template}: the hybrid material"))?;
                texture
            }
        };
        material
            .root
            .set_path("material.g_stage.effect.two_sided.value", ConfigValue::Boolean(page.two_sided))?;
        if matches!(page.light, BakedLightJson::Hybrid { gloss: Some(_), .. }) {
            // the gloss page holds the g-buffer's value: read through as it is
            // (the alpha template's range is 0..0.3)
            for (field, value) in [
                ("constant_roughness_min", 0.0),
                ("constant_roughness_max", 1.0),
                ("min_roughness", 0.0),
                ("max_roughness", 1.0),
            ] {
                let path = format!("material.g_stage.effect.{field}.value");
                let value = number_like(&material.root, &path, value);
                material.root.set_path(&path, value)?;
            }
        }
        let name = format!("{made}/{}", page.name);
        pack.config(&format!("material_instances/{name}.material"), &material)?;
        materials.insert(page.name.clone(), (name, texture));
    }

    // The baked surfaces: one model at the origin.
    let mut surfaces = Vec::new();
    for surface in &baked.surfaces {
        let (material, texture) =
            materials.get(&surface.page).with_context(|| format!("{}: no page {}", surface.name, surface.page))?;
        let mesh = fs::read(options.bake_dir.join(&surface.file))
            .with_context(|| format!("the bake: {}", surface.file.display()))?;
        let (vertices, indices, bounds) = converted_model_shape(&mesh, &surface.name)?;
        surfaces.push(BuiltSurface {
            name: surface.name.clone(),
            material: material.clone(),
            mesh,
            bounds,
            vertices,
            indices,
            mesh_name: format!("|bake|{}", surface.page),
            texture_name: texture.clone(),
        });
    }
    let lobby_model = format!("{made}/lobby");
    write_static_model(&lobby_model, "made by vostok-port draw-lobby-bake: a lobby's surfaces with their path-traced light", &surfaces, None, &templates, &mut pack)?;

    // A placement whose surfaces are all baked goes; one with surfaces the
    // engine still draws keeps them in a model of their own.
    let mut engine_models = BTreeSet::new();
    for (index, placement) in &placements {
        if let Some(name) = placement.engine_model(*index, &made)
            && engine_models.insert(name.clone())
        {
            write_engine_model(&name, placement, options.bake_dir, &templates, &mut source, &mut pack)?;
        }
    }

    // The project.
    let mut project = BinaryConfig::parse(&project_bytes).with_context(|| project_record.path.clone())?;
    let Some(ConfigValue::Indexed(visuals)) = project.root.named("render_visuals").cloned() else {
        bail!("{}: no render_visuals", project_record.path);
    };
    let (mut removed, mut swapped) = (0, 0);
    let mut new_visuals = Vec::new();
    for (index, mut visual) in visuals.into_iter().enumerate() {
        let Some(placement) = placements.get(&index) else {
            new_visuals.push(visual);
            continue;
        };
        let name = visual.named("visual_name").or_else(|| visual.named("lib_name"));
        ensure!(
            matches!(name, Some(ConfigValue::String(n)) if *n == placement.visual),
            "render_visuals[{index}]: not {} as the bake saw it",
            placement.visual
        );
        if placement.baked.is_empty() {
            new_visuals.push(visual);
        } else if placement.drawn.is_empty() {
            removed += 1;
        } else {
            let model = placement.engine_model(index, &made).expect("a placement both baked and drawn");
            for field in ["lib_name", "visual_name"] {
                if visual.named(field).is_some() {
                    visual.set_path(field, ConfigValue::String(model.clone()))?;
                }
            }
            new_visuals.push(visual);
            swapped += 1;
        }
    }
    new_visuals.push(ConfigValue::Named(vec![
        named("rotation", ConfigValue::Float3([0.0; 3])),
        named("position", ConfigValue::Float3([0.0; 3])),
        named("lib_name", ConfigValue::String(lobby_model.clone())),
        named("visual_name", ConfigValue::String(lobby_model.clone())),
        named("scale", ConfigValue::Float3([1.0; 3])),
    ]));
    project.root.set_path("render_visuals", ConfigValue::Indexed(new_visuals))?;
    notes.push(format!(
        "drawn from a path-traced bake (scripts/lobby-bake, vostok-port draw-lobby-bake): {} pages, {} surfaces in {lobby_model}; {removed} placements baked away, {swapped} keeping what the engine still draws (glass, forward decals{}) in models of their own under {made}/drawn",
        baked.pages.len(),
        surfaces.len(),
        if scene.zones.is_empty() {
            String::new()
        } else {
            format!(", the parts in the unbaked zones: {}", scene.zones.iter().map(|z| z.name.as_str()).collect::<Vec<_>>().join(", "))
        }
    ));

    let decals: BTreeSet<&str> = scene.decals.iter().map(|d| d.name.as_str()).collect();
    let in_box = |p: [f32; 3]| {
        scene.seen.is_none_or(|b| (0..3).all(|k| b[0][k].min(b[1][k]) <= p[k] && p[k] <= b[0][k].max(b[1][k])))
    };
    let Some(ConfigValue::Indexed(objects)) = project.root.named_mut("client_objects") else {
        bail!("{}: no client_objects", project_record.path);
    };
    let full_name = |o: &ConfigValue| match (o.named("full_name"), o.named("name")) {
        (Some(ConfigValue::String(n)), _) if !n.is_empty() => n.clone(),
        (_, Some(ConfigValue::String(n))) => n.clone(),
        _ => String::new(),
    };
    let kind = |o: &ConfigValue| match o.named("game_object_type") {
        Some(ConfigValue::String(k)) => k.clone(),
        _ => String::new(),
    };
    let before = objects.len();
    let (mut decals_out, mut ambient_out, mut lights_out) = (0, 0, 0);
    objects.retain(|o| match kind(o).as_str() {
        "light" if scene.dropped_lights.contains(&full_name(o)) => {
            lights_out += 1;
            false
        }
        "decal" if decals.contains(full_name(o).as_str()) => {
            decals_out += 1;
            false
        }
        "ambient_light" if matches!(o.named("position"), Some(ConfigValue::Float3(p)) if in_box(*p)) => {
            ambient_out += 1;
            false
        }
        _ => true,
    });
    ensure!(decals_out == decals.len(), "the bake painted {} decals, the project has {decals_out} of them", decals.len());
    ensure!(
        lights_out == scene.dropped_lights.len(),
        "the bake left out {} lights, the project has {lights_out} of them",
        scene.dropped_lights.len()
    );
    notes.push(format!(
        "{} client objects left out: the {decals_out} projected decals painted into the pages, the {ambient_out} ambient-light boxes inside the lobby (the trace's bounce and shadows stand for them), the {lights_out} fill lights the bake left out{}",
        before - objects.len(),
        if scene.dropped_lights.is_empty() {
            String::new()
        } else {
            format!(" ({})", scene.dropped_lights.iter().cloned().collect::<Vec<_>>().join(", "))
        }
    ));
    for (name, relit) in &scene.relit {
        let object = objects
            .iter_mut()
            .find(|o| kind(o) == "light" && full_name(o) == *name)
            .with_context(|| format!("the bake relit light {name:?}, the project has none"))?;
        object.set_path("color", ConfigValue::Float3(relit.color))?;
        let intensity = number_like(object, "intensity", relit.intensity);
        object.set_path("intensity", intensity)?;
    }
    if !scene.relit.is_empty() {
        notes.push(format!("{} lamps given a black body's colour, as bright as before (the bake's)", scene.relit.len()));
    }
    let hybrid = baked.pages.iter().all(|p| matches!(p.light, BakedLightJson::Hybrid { .. }));
    let added: Vec<&SceneLight> = scene.lights.iter().filter(|l| l.name.starts_with(BAKE_LIGHTS)).collect();
    if hybrid && !added.is_empty() {
        // the engine lights a hybrid's pages: a light only the trace had
        // (in a stove) is the project's too, shadowed by what is round it
        let template = objects
            .iter()
            .find(|o| kind(o) == "light" && matches!(o.named("light_type"), Some(ConfigValue::Integer(0))))
            .context("a light to add, and no point light in the project to copy")?
            .clone();
        for light in &added {
            let mut object = template.clone();
            let name = light.name.trim_start_matches(BAKE_LIGHTS);
            ensure!(light.color.len() >= 3, "light {}: no colour", light.name);
            for (path, value) in [
                ("name", ConfigValue::String(name.to_owned())),
                ("full_name", ConfigValue::String(format!("environment_and_lights/baked/{name}"))),
                ("position", ConfigValue::Float3(light.position)),
                ("color", ConfigValue::Float3([light.color[0], light.color[1], light.color[2]])),
                ("intensity", number_like(&template, "intensity", light.intensity)),
                ("range", number_like(&template, "range", light.range)),
                ("attenuation_power", number_like(&template, "attenuation_power", light.attenuation_power)),
                ("is_cast_shadow", ConfigValue::Boolean(light.is_cast_shadow)),
                ("enabled", ConfigValue::Boolean(true)),
            ] {
                object.set_path(path, value)?;
            }
            objects.push(object);
        }
        notes.push(format!(
            "{} lights the trace had and the project had not added: {}",
            added.len(),
            added.iter().map(|l| l.name.trim_start_matches(BAKE_LIGHTS)).collect::<Vec<_>>().join(", ")
        ));
    }
    for probe in &baked.probes {
        let object = objects
            .iter_mut()
            .find(|o| full_name(o) == probe.object)
            .with_context(|| format!("the bake traced probe {:?}, the project has none", probe.object))?;
        let bytes = fs::read(options.bake_dir.join(&probe.file)).with_context(|| format!("the bake: {}", probe.file.display()))?;
        let traced: Cube = traced_cube(&bytes, probe.size, &probe.object)?;
        let texture = format!("{made}/probes/{}", probe.object.to_ascii_lowercase().replace(['/', ' '], "_"));
        let mean = set_probe_traced(object, &traced, &texture, &mut |path| source.read(path), &mut pack)?;
        notes.push(format!(
            "probe {}: cubemaps path-traced from the bake (over its diffuse_multiplier); mean irradiance over pi ({:.3}, {:.3}, {:.3})",
            probe.object, mean[0], mean[1], mean[2]
        ));
    }
    pack.config(&project_record.path, &project)?;
    let lua = format!("{}.lua", project_record.path);
    if source.files.contains_key(&lua) {
        pack.write(&lua, config_to_lua(&project.root)?.as_bytes())?;
    }

    // The pack's other files: those something still names.
    let copied = copy_named(&mut source, &project_record.path, &mut pack)?;
    let left_out = source.files.len() - copied - usize::from(source.files.contains_key(&project_record.path))
        - usize::from(source.files.contains_key(&lua));
    notes.push(format!("{copied} of the pack's files kept, {left_out} left out (nothing names them any more: the baked models' lit materials and textures)"));
    check_names(&pack, &mut source)?;

    let mut operations = Vec::new();
    for path in &pack.paths {
        operations.push(if source.db.contains(path) {
            let base = source.db.read(path)?;
            notes.push(format!("{path}: replaced (the database's is {})", crate::content_pack::sha256_hex(&base)));
            Operation::ReplaceFile { path: path.clone(), source: None, base_sha256: crate::content_pack::sha256_hex(&base) }
        } else {
            Operation::AddFile { path: path.clone(), source: None }
        });
    }
    let description = input.as_ref().and_then(|m| m.description.clone()).map_or_else(
        || format!("the lobby {} drawn from a path-traced bake", project_record.path),
        |d| format!("{d}; drawn from a path-traced bake"),
    );
    let manifest = PackManifest {
        format_version: 1,
        id: options.id.to_owned(),
        version: version.to_owned(),
        target_build: TARGET_BUILD.to_owned(),
        description: Some(description),
        requires: input.as_ref().map(|m| m.requires.clone()).unwrap_or_default(),
        base_db_sha256: input.as_ref().and_then(|m| m.base_db_sha256.clone()),
        operations,
        server: input.as_ref().and_then(|m| m.server.clone()),
        notes: input.as_ref().map(|m| m.notes.clone()).unwrap_or_default().into_iter().chain(notes.clone()).collect(),
    };
    fs::write(options.output.join("pack.json"), serde_json::to_vec_pretty(&manifest)?)?;
    Ok(DrawReport { files: pack.paths.len(), notes })
}

/// A model of what the engine still draws of a placement: the model's own
/// surfaces, or prep.py's parts of them in the unbaked zones (the surface's
/// export properties with their counts), under the static template's settings.
fn write_engine_model(
    name: &str,
    placement: &Placement,
    bake_dir: &Path,
    templates: &StaticModelTemplates,
    source: &mut Source<'_>,
    pack: &mut PackWriter,
) -> Result<()> {
    let from = format!("models/{}.model", placement.visual);
    let settings = BinaryConfig::parse(&source.read(&format!("{from}/settings"))?)?;
    let root = format!("models/{name}.model");
    pack.write(&format!("{root}/render/export_properties"), &source.read(&format!("{from}/render/export_properties"))?)?;
    let mut material_settings = Vec::new();
    for (surface, drawn) in &placement.drawn {
        let folder = format!("{from}/render/{surface}");
        match drawn {
            Drawn::Whole => {
                for path in source.under(&folder) {
                    let bytes = source.read(&path)?;
                    pack.write(&format!("{root}{}", &path[from.len()..]), &bytes)?;
                }
            }
            Drawn::Part(file) => {
                let mesh = fs::read(bake_dir.join(file)).with_context(|| format!("the bake: {}", file.display()))?;
                let (vertices, indices) = (read_u32(&mesh, 8)?, {
                    let at = 8 + usize::try_from(read_u32(&mesh, 4)?)?;
                    read_u32(&mesh, at + 8)?
                });
                let mut properties = BinaryConfig::parse(&source.read(&format!("{folder}/export_properties"))?)?;
                for (path, value) in [("source.vertices", vertices), ("source.indices", indices), ("source.primitives", indices / 3)] {
                    if crate::content_pack::value::value_at(&properties.root, path).is_some() {
                        properties.root.set_path(path, ConfigValue::Integer(i32::try_from(value)?))?;
                    }
                }
                pack.write(&format!("{root}/render/{surface}/converted_model"), &mesh)?;
                pack.config(&format!("{root}/render/{surface}/export_properties"), &properties)?;
            }
        }
        let material = match crate::content_pack::value::value_at(
            &settings.root,
            &format!("material_settings.{surface}.material_name"),
        ) {
            Some(ConfigValue::String(m)) => m.clone(),
            _ => bail!("{from}: no material for {surface}"),
        };
        pack.materials_used.insert(material.clone());
        material_settings.push(named(
            surface,
            ConfigValue::Named(vec![
                named("material_name", ConfigValue::String(material)),
                named("used_in_portals", ConfigValue::Boolean(false)),
            ]),
        ));
    }
    let mut out = templates.settings.clone();
    out.root.set_path(
        "lod_hierrarchy",
        ConfigValue::Named(vec![named(
            "LOD0",
            ConfigValue::Named(vec![named(
                "surfaces",
                ConfigValue::Indexed(placement.drawn.keys().map(|s| ConfigValue::String(s.clone())).collect()),
            )]),
        )]),
    )?;
    out.root.set_path("material_settings", ConfigValue::Named(material_settings))?;
    pack.config(&format!("{root}/settings"), &out)
}

/// The strings a file holds: a binary config's, else (a vegetation file)
/// its runs of printable characters.
fn strings_of(bytes: &[u8]) -> Vec<String> {
    if let Ok(config) = BinaryConfig::parse(bytes) {
        let mut out = Vec::new();
        config.root.collect_strings(&mut out);
        return out.into_iter().map(str::to_owned).collect();
    }
    bytes
        .split(|b| !(b.is_ascii_graphic() || *b == b' '))
        .filter(|run| run.len() >= 4)
        .map(|run| String::from_utf8_lossy(run).into_owned())
        .collect()
}

/// The input pack's files a name can mean: a model (its folder, or the
/// folder a path into it names), a material, a texture (and a probe's
/// `_diffuse`), or the path itself.
fn named_files(files: &BTreeMap<String, PathBuf>, name: &str) -> Vec<String> {
    // a collision's name can end in its box's half extents: `...#[x][y][z]`
    let name = name.trim().trim_start_matches("resources/");
    let name = name.split_once('#').map_or(name, |(path, _)| path);
    if name.is_empty() {
        return Vec::new();
    }
    let mut prefixes = vec![format!("models/{name}.model/")];
    if name.contains(".model/") {
        prefixes.push(format!("{name}/"));
    }
    let mut out: Vec<String> = prefixes
        .iter()
        .flat_map(|prefix| files.range(prefix.clone()..).take_while(|(p, _)| p.starts_with(prefix.as_str())).map(|(p, _)| p.clone()))
        .collect();
    for path in [
        name.to_owned(),
        format!("material_instances/{name}.material"),
        format!("textures/{name}.dds"),
        format!("textures/{name}.options"),
        format!("textures/{name}_diffuse.dds"),
        format!("textures/{name}_diffuse.options"),
    ] {
        if files.contains_key(&path) {
            out.push(path);
        }
    }
    out
}

/// The input pack's files that the written ones, or the pack's other
/// project files (vegetation), name, and those these name, copied into the
/// pack. Returns how many.
fn copy_named(source: &mut Source<'_>, project: &str, pack: &mut PackWriter) -> Result<usize> {
    let mut queue: Vec<Vec<u8>> = Vec::new();
    for path in &pack.paths {
        queue.push(fs::read(safe_join(&pack.files, path)?)?);
    }
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut wanted: Vec<String> = source
        .files
        .keys()
        .filter(|p| p.starts_with("projects/") && !p.starts_with(project))
        .cloned()
        .collect();
    loop {
        for bytes in queue.drain(..) {
            for name in strings_of(&bytes) {
                wanted.extend(named_files(&source.files, &name));
            }
        }
        let fresh: Vec<String> =
            wanted.drain(..).filter(|p| !pack.paths.contains(p) && taken.insert(p.clone())).collect();
        if fresh.is_empty() {
            break;
        }
        for path in fresh {
            let bytes = source.read(&path)?;
            pack.write(&path, &bytes)?;
            queue.push(bytes);
        }
    }
    Ok(taken.len())
}

/// What the pack's project and models name is there, in the pack or the
/// database: every placed model, collision, material and texture.
fn check_names(pack: &PackWriter, source: &mut Source<'_>) -> Result<()> {
    let there = |path: &str, source: &Source<'_>| pack.paths.contains(path) || source.contains(path);
    let read = |path: &str, source: &mut Source<'_>| -> Result<Vec<u8>> {
        if pack.paths.contains(path) { Ok(fs::read(safe_join(&pack.files, path)?)?) } else { source.read(path) }
    };
    let mut missing = Vec::new();
    let mut models = BTreeSet::new();
    for path in pack.paths.iter().filter(|p| p.starts_with("projects/") && p.ends_with("client_project")) {
        let project = BinaryConfig::parse(&read(path, source)?)?;
        for (table, field) in [("render_visuals", "lib_name"), ("collision_objects", "lib_name")] {
            let Some(ConfigValue::Indexed(rows)) = project.root.named(table) else { continue };
            for row in rows {
                let Some(ConfigValue::String(name)) = row.named(field) else { continue };
                if table == "render_visuals" {
                    models.insert(name.clone());
                } else {
                    let folder = name.trim_start_matches("resources/");
                    let folder = folder.split_once('#').map_or(folder, |(path, _)| path);
                    let any = pack.paths.iter().any(|p| p.starts_with(&format!("{folder}/"))) || !source.under(folder).is_empty();
                    if !any {
                        missing.push(format!("{path}: collision {name}"));
                    }
                }
            }
        }
    }
    let mut materials = BTreeSet::new();
    for model in &models {
        let settings = format!("models/{model}.model/settings");
        if !there(&settings, source) {
            missing.push(format!("model {model}"));
            continue;
        }
        let config = BinaryConfig::parse(&read(&settings, source)?)?;
        if let Some(ConfigValue::Named(surfaces)) = config.root.named("material_settings") {
            for s in surfaces {
                if let Some(ConfigValue::String(m)) = s.value.named("material_name") {
                    materials.insert(m.clone());
                }
            }
        }
    }
    for material in &materials {
        let path = format!("material_instances/{material}.material");
        if !there(&path, source) {
            missing.push(format!("material {material}"));
            continue;
        }
        if !pack.paths.contains(&path) {
            continue; // the database's own
        }
        let config = BinaryConfig::parse(&read(&path, source)?)?;
        for key in ["texture_diffuse", "texture_normal", "texture_emissive", "texture_specular_intensity"] {
            if let Some(ConfigValue::String(t)) =
                crate::content_pack::value::value_at(&config.root, &format!("material.g_stage.effect.{key}.value"))
                && !t.is_empty()
                && !there(&format!("textures/{t}.dds"), source)
            {
                missing.push(format!("{material}: {key} {t}"));
            }
        }
    }
    ensure!(missing.is_empty(), "named but in neither the pack nor the database:\n  {}", missing.join("\n  "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_means_its_model_collision_material_or_texture() {
        let files: BTreeMap<String, PathBuf> = [
            "models/p/table.model/settings",
            "models/p/table.model/render/top_s/converted_model",
            "models/p/table.model/collision/vertices",
            "models/p/tables.model/settings",
            "material_instances/p/wood.material",
            "textures/p/wood.dds",
            "textures/p/wood.options",
            "textures/p/probe.dds",
            "textures/p/probe_diffuse.dds",
            "projects/lobby_scene/vegetation/x/green.veg",
        ]
        .into_iter()
        .map(|p| (p.to_owned(), PathBuf::new()))
        .collect();
        let named = |n: &str| named_files(&files, n);
        assert_eq!(
            named("p/table"),
            ["models/p/table.model/collision/vertices", "models/p/table.model/render/top_s/converted_model", "models/p/table.model/settings"]
        );
        assert_eq!(named("resources/models/p/table.model/collision"), ["models/p/table.model/collision/vertices"]);
        assert_eq!(named("resources/models/p/table.model/collision#[0.5][1][2]"), ["models/p/table.model/collision/vertices"]);
        assert_eq!(named("p/wood"), ["material_instances/p/wood.material", "textures/p/wood.dds", "textures/p/wood.options"]);
        assert_eq!(named("p/probe"), ["textures/p/probe.dds", "textures/p/probe_diffuse.dds"]);
        assert!(named("p/tab").is_empty() && named("").is_empty());
    }

    #[test]
    fn a_file_that_is_no_config_gives_its_printable_runs() {
        let bytes = b"\x01\x02flora/grass_01\x00\x05ab\x00level/x\xff";
        assert_eq!(strings_of(bytes), ["flora/grass_01", "level/x"]);
    }
}
