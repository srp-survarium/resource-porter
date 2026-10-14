// SPDX-License-Identifier: GPL-3.0-or-later
//! Translate the early scene/vertex contracts without replacing authored assets.
use super::*;

fn remove(value: &mut ConfigValue, key: &str) -> Option<ConfigValue> {
    let ConfigValue::Named(fields) = value else {
        return None;
    };
    fields
        .iter()
        .position(|f| f.name == key)
        .map(|i| fields.remove(i).value)
}

fn number(value: &ConfigValue) -> Result<f32> {
    match value {
        ConfigValue::Float(v) => Ok(*v),
        ConfigValue::Integer(v) => Ok(*v as f32),
        _ => bail!("expected numeric environment value"),
    }
}

fn transfer_probe(object: &mut ConfigValue, weather: &str, source: SourceBuild) -> Result<()> {
    let texture = string(object, "texture")
        .context("early probe texture")?
        .to_owned();
    object.set_path(
        "textures",
        ConfigValue::Named(vec![
            NamedValue {
                name: format!("{weather}_2"),
                value: ConfigValue::String(texture.clone()),
            },
            NamedValue {
                name: "default".into(),
                value: ConfigValue::String(texture),
            },
        ]),
    )?;
    if source == SourceBuild::V011e {
        let width = number(
            object
                .named("probe_side_width")
                .context("0.11e authored probe width")?,
        )?;
        ensure!(width.is_finite() && width > 0., "invalid 0.11e probe width");
        // Source sphere PS applies .98 to radius; native CPU applies it before
        // upload. Preserve explicit widths, including authored reversed falloff.
        return Ok(());
    }
    if object
        .named("geometry")
        .map(number)
        .transpose()?
        .unwrap_or(0.)
        == 0.
    {
        let radius = number(object.named("radius").context("early probe radius")?)?;
        ensure!(
            radius.is_finite() && radius > 0.,
            "invalid early probe radius"
        );
        // 0.26 contracts the outer sphere by 2% before shader falloff.
        // Retain a one-metre transition inside that sphere; the template's
        // fixed width can otherwise invert the interval for large probes.
        object.set_path("probe_side_width", ConfigValue::Float(1. + 0.02 * radius))?;
    }
    Ok(())
}

/// Factor that becomes `multiplier` after 0.26 raises a colour to 2.2.
fn linear_factor(multiplier: f32) -> f32 {
    multiplier.max(0.).powf(1. / 2.2)
}

/// A source linear colour expressed for a 0.26 key that applies pow 2.2.
fn display_color(value: [f32; 4]) -> [f32; 4] {
    [
        value[0].max(0.).powf(1. / 2.2),
        value[1].max(0.).powf(1. / 2.2),
        value[2].max(0.).powf(1. / 2.2),
        value[3],
    ]
}

fn color4(value: &ConfigValue) -> Result<ConfigValue> {
    match value {
        ConfigValue::Float3(v) => Ok(ConfigValue::Float4([v[0], v[1], v[2], 1.])),
        ConfigValue::Float4(_) => Ok(value.clone()),
        _ => bail!("expected color"),
    }
}

pub(super) fn project(
    mut old: ConfigValue,
    donor: &mut ResourceDb,
    server: ConfigValue,
    target: &ConfigValue,
    map: &Map,
    source: SourceBuild,
    notes: &mut Vec<String>,
) -> Result<ConfigValue> {
    let path = map.donor_project(source);
    let objects = remove(&mut old, "objects").context("early scene has no objects")?;
    let objects = array(&objects)?;
    let target_objects = array(target.named("client_objects").context("target objects")?)?;
    let mut groups = server
        .named("objects")
        .context("early server objects")?
        .clone();
    // 0.26 keeps these on the client project rather than repeating them in the
    // server groups. Preserve their authored arrays exactly once.
    remove(&mut groups, "collision_objects");
    remove(&mut groups, "collision_geometry");
    remove(&mut old, "respawn_points");
    let converted = convert_respawn_invulnerability(&mut groups)?;
    if converted > 0 {
        notes.push(format!(
            "{path}: converted {converted} authored respawn invulnerability times from milliseconds to native seconds"
        ));
    }
    let mut client = Vec::new();
    let sun = objects
        .iter()
        .find(|o| string(o, "lib_name") == Some("sun"));
    for object in objects {
        let kind = string(object, "game_object_type").context("early object type")?;
        if kind == "victory_items_container" {
            continue;
        }
        if kind == "sky" {
            // The old sky object's material is unused by the old sky stage;
            // its visible cloud texture comes from the environment below.
            notes.push(format!(
                "{path}: sky object replaced by environment sky properties"
            ));
            continue;
        }
        // wire_set needs no reconstruction: 0.10b object_wire::load and 0.26
        // object_wire::load (0x293e90) read the same fields (points,
        // points_count, wire_width, ...); adapt_objects adds layer and
        // is_temporary, exactly as for the 0.20e wires that already ship.
        let mut object = object.clone();
        if kind == "environment" {
            let post = string(&object, "post_effect").context("early post effect")?;
            let material = cfg(donor, &format!("material_instances/{post}.material"))?;
            let post = material
                .named("material")
                .and_then(|v| v.named("post_process"))
                .context("post-process values")?;
            let template = target_objects
                .iter()
                .find(|o| string(o, "game_object_type") == Some("environment"))
                .context("environment template")?;
            let mut properties = template
                .named("properties")
                .context("environment properties")?
                .clone();
            transfer_environment(&mut properties, post, sun, notes, &path)?;
            object.set_path("properties", properties)?;
            remove(&mut object, "post_effect");
        }
        if kind == "light" && string(&object, "lib_name") == Some("sun") {
            continue;
        }
        if kind == "environment_probe" {
            transfer_probe(&mut object, &map.weather, source)?;
            if source == SourceBuild::V011e
                && object
                    .named("geometry")
                    .map(number)
                    .transpose()?
                    .unwrap_or(0.)
                    == 0.
            {
                notes.push(format!("{path}: 0.11e explicit sphere width preserves source radial falloff; source shader and native CPU both apply .98 radius; native perturbation and other probe semantics remain distinct"));
            }
        }
        if kind == "vegetation" {
            if let Some(ConfigValue::Indexed(layers)) = object.named_mut("layers") {
                for layer in layers {
                    // Editor grid metadata absent in these source scenes.
                    // Runtime grass consumes the baked .veg positions instead:
                    // pixel_size is not a grass_layer_desc/cook input.
                    layer.set_path("pixel_size", ConfigValue::Float(0.0))?;
                }
            }
        }
        if kind == "light" {
            // A cloned light's offset must not displace the old authored lamp.
            object.set_path(
                "offset",
                ConfigValue::Named(vec![
                    NamedValue {
                        name: "position".into(),
                        value: ConfigValue::Float3([0.; 3]),
                    },
                    NamedValue {
                        name: "rotation".into(),
                        value: ConfigValue::Float3([0.; 3]),
                    },
                    NamedValue {
                        name: "scale".into(),
                        value: ConfigValue::Float3([1.; 3]),
                    },
                ]),
            )?;
        }
        client.push(object);
    }
    if source == SourceBuild::V011a && map.level == "level_04" {
        if repair_chemical_collector_zone(&mut groups, objects)? {
            notes.push(format!(
                "{path}: repaired duplicated original team2 collector zone"
            ));
        }
    }
    // Old collectors are already rendered as placed static models. Bind that
    // original mesh and transform instead of inheriting a modern collector.
    let mut collector_visuals = Vec::new();
    if let Some(ConfigValue::Indexed(collectors)) = groups.named_mut("victory_items_containers") {
        let visuals = array(old.named("render_visuals").context("old visuals")?)?;
        for collector in collectors {
            let visual = collector_visual(collector, objects, visuals)?;
            ensure!(
                !collector_visuals.contains(visual),
                "collectors must not share an authored visual"
            );
            let mut geometry = visual.clone();
            geometry.set_path("model_name", visual.named("lib_name").unwrap().clone())?;
            geometry.set_path(
                "name",
                ConfigValue::String(
                    string(collector, "full_name")
                        .context("collector name")?
                        .into(),
                ),
            )?;
            collector.set_path("visual_geometry", geometry)?;
            collector_visuals.push(visual.clone());
        }
    }
    if let Some(ConfigValue::Indexed(visuals)) = old.named_mut("render_visuals") {
        visuals.retain(|v| !collector_visuals.contains(v));
    }
    old.set_path("client_objects", ConfigValue::Indexed(client))?;
    old.set_path("server_objects", groups)?;
    old.set_path("project_name", ConfigValue::String(map.level.clone()))?;
    Ok(old)
}

pub(super) fn validate_server_fallback(client: &ConfigValue, server: &ConfigValue) -> Result<()> {
    let groups = server.named("objects").context("fallback server groups")?;
    ensure!(
        client.named("respawn_points") == groups.named("respawn_points")
            && client.named("respawn_points").is_some(),
        "fallback player spawns differ from primary client"
    );
    let objects = array(client.named("objects").context("primary client objects")?)?;
    for (group, kind) in [
        ("victory_items_containers", "victory_items_container"),
        ("collision_geometry", "collision_geometry"),
    ] {
        let authored = ConfigValue::Indexed(
            objects
                .iter()
                .filter(|o| string(o, "game_object_type") == Some(kind))
                .cloned()
                .collect(),
        );
        ensure!(
            groups.named(group) == Some(&authored),
            "fallback {group} differs from primary client"
        );
    }
    Ok(())
}

/// Early respawn protection is authored in milliseconds: the 0.11e loader
/// stores `invulnerability_time` unchanged at respawn point +0x20
/// (0x6F7CD1..0x6F7CE9). Native 0.26g multiplies the same field by 1000
/// before storing that slot (0x2F460F..0x2F4630) and its own maps author 6.
/// Copying 2000 therefore grants 2000 seconds of protection after every spawn.
fn convert_respawn_invulnerability(groups: &mut ConfigValue) -> Result<usize> {
    let Some(ConfigValue::Indexed(points)) = groups.named_mut("respawn_points") else {
        return Ok(0);
    };
    let mut converted = 0;
    for point in points {
        let Some(value) = point.named("invulnerability_time") else {
            continue;
        };
        let ConfigValue::Integer(milliseconds) = value else {
            bail!("respawn invulnerability time is not an integer");
        };
        ensure!(
            *milliseconds >= 0 && milliseconds % 1000 == 0,
            "respawn invulnerability {milliseconds} ms has no exact native whole-second value"
        );
        let seconds = milliseconds / 1000;
        point.set_path("invulnerability_time", ConfigValue::Integer(seconds))?;
        converted += 1;
    }
    Ok(converted)
}

/// Repair the duplicated team1 zone reference authored in the 0.11a
/// Chemical Plant project. The corresponding team2 zone and mesh already exist.
pub fn repair_chemical_collector_zone(
    groups: &mut ConfigValue,
    objects: &[ConfigValue],
) -> Result<bool> {
    const WEST: &str = "logic/team1/victory_container_collision";
    const EAST: &str = "logic/team2/victory_container_collision";
    let ConfigValue::Indexed(collectors) = groups
        .named_mut("victory_items_containers")
        .context("collectors")?
    else {
        bail!("collector array");
    };
    ensure!(
        collectors.len() == 2,
        "Chemical Plant requires two collectors"
    );
    let west = collectors
        .iter()
        .find(|o| string(o, "full_name") == Some("logic/team1/victory_items_container"))
        .context("Chemical west collector")?;
    ensure!(
        west.named("team") == Some(&ConfigValue::Integer(0)),
        "Chemical west team"
    );
    let west_refs = array(
        west.named("collision_geometries")
            .context("west references")?,
    )?;
    ensure!(
        west_refs.len() == 1 && string(&west_refs[0], "name") == Some(WEST),
        "Chemical west zone"
    );
    let east = collectors
        .iter_mut()
        .find(|o| string(o, "full_name") == Some("logic/team2/victory_items_container"))
        .context("Chemical east collector")?;
    ensure!(
        east.named("team") == Some(&ConfigValue::Integer(1)),
        "Chemical east team"
    );
    let ConfigValue::Indexed(refs) = east
        .named_mut("collision_geometries")
        .context("east references")?
    else {
        bail!("east reference array")
    };
    ensure!(refs.len() == 1, "Chemical east zone count");
    let old = string(&refs[0], "name").context("east zone name")?;
    ensure!(old == WEST || old == EAST, "unexpected Chemical east zone");
    if old == EAST {
        return Ok(false);
    }
    let zone = objects
        .iter()
        .find(|o| {
            string(o, "full_name") == Some(EAST)
                && string(o, "game_object_type") == Some("collision_geometry")
        })
        .context("authored Chemical east zone")?;
    ensure!(
        zone.named("position") == Some(&ConfigValue::Float3([102.606, 6.87, 79.873])),
        "Chemical east zone moved"
    );
    refs[0].set_path("name", ConfigValue::String(EAST.into()))?;
    Ok(true)
}

/// The editor marker may be far from the usable object. Its named collision
/// zone, not that marker, identifies the authored collector model.
pub fn collector_visual<'a>(
    collector: &ConfigValue,
    objects: &[ConfigValue],
    visuals: &'a [ConfigValue],
) -> Result<&'a ConfigValue> {
    let mut centers = Vec::new();
    for reference in array(
        collector
            .named("collision_geometries")
            .context("collector zones")?,
    )? {
        let name = string(reference, "name").context("collector zone name")?;
        let zone = objects
            .iter()
            .find(|o| {
                string(o, "game_object_type") == Some("collision_geometry")
                    && string(o, "full_name") == Some(name)
            })
            .with_context(|| format!("missing collector zone {name}"))?;
        let Some(ConfigValue::Float3(position)) = zone.named("position") else {
            bail!("collector zone position type");
        };
        centers.push(position);
    }
    let (visual, distance) = visuals
        .iter()
        .filter(|v| {
            string(v, "lib_name").is_some_and(|s| {
                s.ends_with("item/quest/container") || s.ends_with("item/quest/atlant")
            })
        })
        .filter_map(|v| match v.named("position") {
            Some(ConfigValue::Float3(position)) => centers
                .iter()
                .map(|center| {
                    position
                        .iter()
                        .zip(center.iter())
                        .map(|(a, b)| (a - b) * (a - b))
                        .sum::<f32>()
                })
                .min_by(f32::total_cmp)
                .map(|distance| (v, distance)),
            _ => None,
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .context("no authored collector mesh near its interaction zone")?;
    ensure!(
        distance <= 25.,
        "collector mesh is more than five metres from its interaction zone"
    );
    Ok(visual)
}

fn transfer_environment(
    dst: &mut ConfigValue,
    old: &ConfigValue,
    sun: Option<&ConfigValue>,
    notes: &mut Vec<String>,
    path: &str,
) -> Result<()> {
    let aliases = BTreeMap::from([
        // Optional native fields absent from the chosen template.
        ("wind_direction", "wind_direction"),
        ("wind_strength", "wind_strength"),
        ("adaptation_speed", "eye_adaptation_speed"),
        ("use_color_grading_lut", "use_color_grading_texture"),
        ("use_dynamic_lens_flares", "use_dynamic_lens_flare"),
        ("vignette_power", "vignette_effect_power"),
        ("image_grain_scale", "image_grain_intensity"),
        ("bloom_scale", "bloom_ratio"),
        ("ssao_radius_scale", "ao_radius"),
        ("ssao_saturation", "ao_saturation"),
        ("god_rays_attenuation_power", "god_rays_attenuation"),
        ("dof_near_blurriness_amount", "dof_near_blurness_amount"),
        (
            "environment_probes_diffuse_instensity_multiplier",
            "environment_probes_diffuse_intensity_multiplier",
        ),
        (
            "atmosphere_kr4pi_multiplier",
            "atmosphere_rayleighPi_multiplier",
        ),
        ("atmosphere_km4pi_multiplier", "atmosphere_miePi_multiplier"),
        (
            "atmosphere_kresun_multiplier",
            "atmosphere_rayleighSun_multiplier",
        ),
        (
            "atmosphere_kmesun_multiplier",
            "atmosphere_mieSun_multiplier",
        ),
        ("environment_use_rain", "use_rain"),
        ("god_rays_color_0", "god_rays_color_1"),
        ("god_rays_color_1", "god_rays_color_2"),
        // Renamed in 0.26 (environment loader 0x34e1e0..0x34f72e).
        (
            "environment_probes_specular_instensity_multiplier",
            "environment_probes_specular_intensity_multiplier",
        ),
        ("use_bokeh_dof", "use_bokeh_dof"),
        ("bokeh_dof_radius", "bokeh_radius"),
        ("bokeh_dof_density", "bokeh_density"),
        ("use_bokeh_image", "use_bokeh_template_image"),
        ("dof_focus_distance", "dof_focus_distance"),
        ("dof_focus_region", "dof_focus_region"),
        ("dof_near_blurriness_far", "dof_far_blurness_amount"),
        ("enable_ssao", "use_ambient_occlusion"),
        (
            "use_sun_moon_atmosphere_illumination",
            "atmosphere_use_sun_illumination",
        ),
        ("sun_moon_color_multiplier", "sun_moon_billboard_color_intensity"),
    ]);
    let ConfigValue::Named(fields) = old else {
        bail!("invalid early post process")
    };
    let mut omitted = Vec::new();
    for field in fields {
        let Some(value) = field.value.named("value") else {
            continue;
        };
        let name = aliases
            .get(field.name.as_str())
            .copied()
            .unwrap_or(&field.name);
        if dst.named(name).is_some() || aliases.contains_key(field.name.as_str()) {
            let value = match (dst.named(name), value) {
                (Some(ConfigValue::Float(_)), ConfigValue::Integer(n)) => {
                    ConfigValue::Float(*n as f32)
                }
                (Some(ConfigValue::Float4(_)), ConfigValue::Float3(_)) => color4(value)?,
                _ => value.clone(),
            };
            dst.set_path(name, value)?;
        } else {
            omitted.push(field.name.clone());
        }
    }
    // These features were introduced later and must not inherit modern map art.
    for (key, value) in [
        ("use_stratosphere", false),
        ("use_sky_clouds_lighting", false),
        ("use_sharpen", false),
        ("use_radial_blur", false),
    ] {
        dst.set_path(key, ConfigValue::Boolean(value))?;
    }
    dst.set_path("sky_clouds_rotation_speed", ConfigValue::Float(0.0))?;
    let get = |name: &str| {
        old.named(name)
            .and_then(|v| v.named("value"))
            .with_context(|| format!("missing early environment {name}"))
    };
    dst.set_path("use_height_based_ambient", ConfigValue::Boolean(true))?;
    dst.set_path("height_based_ambient_power", get("skylight_power")?.clone())?;
    dst.set_path(
        "height_based_ambient_low_limit",
        get("skylight_lower")?.clone(),
    )?;
    dst.set_path(
        "height_based_ambient_high_limit",
        get("skylight_upper")?.clone(),
    )?;
    dst.set_path(
        "height_based_ambient_low_color",
        ConfigValue::Float4([0., 0., 0., 1.]),
    )?;
    let ConfigValue::Float4(mut color) = color4(get("skylight_color")?)? else {
        unreachable!()
    };
    // The source uploads pow(skylight_color, 2.2) * skylight_multiplier
    // (0.11e stage_ambient_lighting::execute 0x625d53..0x625d85; 0.10b
    // stage_ambient_lighting.cpp:156). 0.26 raises this colour to 2.2 at
    // load (0x34e32d -> helper 0x1bfcb0) and has no separate multiplier, so
    // the multiplier must enter before that power as m^(1/2.2).
    let multiplier = number(get("skylight_multiplier")?)?;
    for c in &mut color[..3] {
        *c *= linear_factor(multiplier);
    }
    dst.set_path(
        "height_based_ambient_high_color",
        ConfigValue::Float4(color),
    )?;
    // Source haze grows from the camera (min(d, D)/D); 0.26 delays it by
    // this distance, which the native template sets to 35-40 m.
    dst.set_path("rayleigh_fog_near_distance", ConfigValue::Float(0.))?;
    // Colours the source uploads without gamma, while 0.26 applies 2.2 at
    // load through the same helper (god rays 0x607454..0x6074d5 in 0.11e;
    // 0.10b god_rays.ps:74 and gather_bloom.ps:78 use them raw).
    for key in ["god_rays_color_1", "god_rays_color_2"] {
        if let Some(ConfigValue::Float4(value)) = dst.named(key).cloned() {
            dst.set_path(key, ConfigValue::Float4(display_color(value)))?;
        }
    }
    if let Some(halo) = old.named("bloom_halo_color").and_then(|v| v.named("value")) {
        let ConfigValue::Float4(halo) = color4(halo)? else {
            unreachable!()
        };
        dst.set_path("bloom_color", ConfigValue::Float4(display_color(halo)))?;
    }
    // 0.10b material_manager.cpp:416-417 uploads pow(color, 2.2) times the
    // cloud multiplier; 0.26 has no multiplier key.
    if let Some(cloud_multiplier) = old
        .named("sky_clouds_color_multiplier")
        .and_then(|v| v.named("value"))
    {
        let factor = linear_factor(number(cloud_multiplier)?);
        if let Some(ConfigValue::Float4(mut clouds)) = dst.named("sky_clouds_color").cloned() {
            for c in &mut clouds[..3] {
                *c *= factor;
            }
            dst.set_path("sky_clouds_color", ConfigValue::Float4(clouds))?;
        }
    }
    dst.set_path(
        "mie_fog_near_distance",
        get("environment_near_fog_distance")?.clone(),
    )?;
    dst.set_path(
        "mie_fog_far_distance",
        get("environment_far_fog_distance")?.clone(),
    )?;
    dst.set_path("mie_fog_color", get("environment_far_fog_color")?.clone())?;
    dst.set_path("mie_fog_density", get("environment_fog_alpha")?.clone())?;
    dst.set_path(
        "rayleigh_fog_far_distance",
        get("atmosphere_inscatter_distance")?.clone(),
    )?;
    if let Some(sun) = sun {
        dst.set_path(
            "sun_color",
            color4(sun.named("color").context("sun color")?)?,
        )?;
        dst.set_path(
            "sun_intensity",
            sun.named("intensity").context("sun intensity")?.clone(),
        )?;
        dst.set_path(
            "use_sun_shadows",
            sun.named("is_cast_shadow").context("sun shadows")?.clone(),
        )?;
        let ConfigValue::Float3(r) = sun.named("rotation").context("sun rotation")? else {
            bail!("sun rotation type")
        };
        // Negated k axis of math::create_rotation(float3), as consumed by
        // lights_db::update_light_props. Its convention includes roll.
        let (sx, cx) = r[0].sin_cos();
        let (sy, cy) = r[1].sin_cos();
        let (sz, cz) = r[2].sin_cos();
        let direction = [cx * sy * cz - sx * sz, -cx * sy * sz - sx * cz, -cx * cy];
        dst.set_path(
            "sun_angle",
            ConfigValue::Float(direction[1].clamp(-1., 1.).asin().to_degrees()),
        )?;
        dst.set_path(
            "sun_position_azimut",
            ConfigValue::Float(
                // Target to-sun direction is (-cos(az)*cos(el), sin(el),
                // -sin(az)*cos(el)); azimuth zero points toward -X.
                (-direction[2])
                    .atan2(-direction[0])
                    .to_degrees()
                    .rem_euclid(360.),
            ),
        )?;
    }
    notes.push(format!("{path}: original sky texture, post-process values and sun translated; height ambient uses source pow(colour, 2.2) * multiplier; source six-face sky and AO factor have no 0.26 counterpart; unmapped controls require visual comparison: {}",omitted.join(", ")));
    Ok(())
}

fn half(value: f32) -> Result<u16> {
    ensure!(
        value.is_finite() && value.abs() <= 65504.,
        "UV outside finite half range"
    );
    let sign = ((value.to_bits() >> 16) & 0x8000) as u16;
    let value = value.abs();
    if value < 2f32.powi(-14) {
        return Ok(sign | (value * 2f32.powi(24)).round_ties_even() as u16);
    }
    let mut exponent = ((value.to_bits() >> 23) & 255) as i32 - 127;
    let mut mantissa = ((value / 2f32.powi(exponent) - 1.) * 1024.).round_ties_even() as u16;
    if mantissa == 1024 {
        exponent += 1;
        mantissa = 0;
    }
    Ok(sign | (((exponent + 15) as u16) << 10) | mantissa)
}

pub(super) fn geometry(bytes: &[u8], rebase_wrapped_uv: bool) -> Result<Vec<u8>> {
    ensure!(
        matches!(
            crate::map_port::upgrade_stripped_static_model(bytes)?,
            crate::map_port::StaticUpgrade::AlreadyCompatible
        ),
        "expected explicit early static layout"
    );
    let u32_at =
        |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    let end = 8 + u32_at(4);
    let declaration = (8..end)
        .step_by(8)
        .find(|&offset| bytes.get(offset..offset + 8) == Some(&[255, 0, 0, 0, 17, 0, 0, 0]))
        .context("declaration terminator")?;
    let count = u32_at(declaration + 8);
    let start = declaration + 12;
    let stride = (end - start) / count;
    ensure!(matches!(stride, 32 | 36), "unsupported early static stride");
    let mut uv: Vec<[f32; 2]> = bytes[start..end]
        .chunks_exact(stride)
        .map(|v| {
            [24, 28].map(|offset| f32::from_le_bytes(v[offset..offset + 4].try_into().unwrap()))
        })
        .collect();
    if rebase_wrapped_uv {
        ensure!(uv.iter().flatten().all(|v| v.is_finite()), "nonfinite UV");
        ensure!(
            bytes.get(end..end + 4) == Some(&4u32.to_le_bytes()),
            "expected index chunk"
        );
        let index_bytes = bytes
            .get(end + 12..end + 8 + u32_at(end + 4))
            .context("truncated indices")?;
        ensure!(
            index_bytes.len() == u32_at(end + 8) * 2 && index_bytes.len() % 6 == 0,
            "invalid triangle indices"
        );
        let mut parent: Vec<_> = (0..count).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for triangle in index_bytes.chunks_exact(6) {
            let ids = [0, 2, 4]
                .map(|i| u16::from_le_bytes(triangle[i..i + 2].try_into().unwrap()) as usize);
            ensure!(ids.iter().all(|&i| i < count), "index outside vertices");
            let first = root(&mut parent, ids[0]);
            for i in &ids[1..] {
                let other = root(&mut parent, *i);
                parent[other] = first;
            }
        }
        let mut islands: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in 0..count {
            islands.entry(root(&mut parent, i)).or_default().push(i);
        }
        for island in islands.values() {
            let shift = [0, 1].map(|axis| {
                let min = island
                    .iter()
                    .map(|&i| uv[i][axis])
                    .fold(f32::INFINITY, f32::min);
                let max = island
                    .iter()
                    .map(|&i| uv[i][axis])
                    .fold(f32::NEG_INFINITY, f32::max);
                ((min + max) * 0.5).round_ties_even()
            });
            for &i in island {
                for axis in 0..2 {
                    uv[i][axis] -= shift[axis];
                }
            }
        }
    }
    let mut result = Vec::new();
    result.extend(3u32.to_le_bytes());
    result.extend(u32::try_from(4 + count * (stride - 4))?.to_le_bytes());
    result.extend(u32::try_from(count)?.to_le_bytes());
    for (vertex, uv) in bytes[start..end].chunks_exact(stride).zip(uv) {
        result.extend(&vertex[..24]);
        for value in uv {
            result.extend(half(value)?.to_le_bytes());
        }
        if stride == 36 {
            result.extend(&vertex[32..36]);
        }
    }
    result.extend(&bytes[end..]);
    Ok(result)
}

/// The runtime consumes float3 positions (0x58) and normals (0x59).
/// 0x4d/0x4e are editor painting data; retain them without resampling.
pub(super) fn vegetation_instances(bytes: &[u8]) -> Result<usize> {
    let mut chunks = BTreeMap::new();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        ensure!(remaining.len() >= 8, "truncated vegetation chunk header");
        let id = u32::from_le_bytes(remaining[..4].try_into().unwrap());
        let size = u32::from_le_bytes(remaining[4..8].try_into().unwrap()) as usize;
        ensure!(size <= remaining.len() - 8, "truncated vegetation chunk");
        ensure!(
            chunks.insert(id, &remaining[8..8 + size]).is_none(),
            "duplicate vegetation chunk"
        );
        remaining = &remaining[8 + size..];
    }
    let positions = chunks.get(&0x58).context("missing vegetation positions")?;
    let normals = chunks.get(&0x59).context("missing vegetation normals")?;
    ensure!(
        positions.len() % 12 == 0 && positions.len() == normals.len(),
        "vegetation position/normal count mismatch"
    );
    for values in [positions, normals] {
        ensure!(
            values
                .chunks_exact(4)
                .all(|v| f32::from_le_bytes(v.try_into().unwrap()).is_finite()),
            "nonfinite vegetation coordinates"
        );
    }
    Ok(positions.len() / 12)
}

/// 0.26 asks for an additional diffuse cube. Preserve the source probe's
/// coarse mip tail instead of borrowing lighting baked in another map.
/// This is a blurred-cube approximation, not an irradiance convolution.
pub(super) fn diffuse_probe(bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() >= 129 && &bytes[..4] == b"DDS ",
        "invalid probe DDS"
    );
    let get = |o| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
    let size = get(12);
    ensure!(
        size == get(16) && size.is_power_of_two() && (16..=4096).contains(&size),
        "unsupported probe size"
    );
    ensure!(
        get(112) & 0xfe00 == 0xfe00,
        "probe is not a six-face cubemap"
    );
    let block = match &bytes[84..88] {
        b"DXT1" => 8,
        b"DXT5" => 16,
        _ => bail!("unsupported probe compression"),
    };
    let mips = get(28);
    ensure!(
        mips == size.ilog2() as usize - 1,
        "probe mip chain must end at 4x4"
    );
    let mut total = 0;
    let mut skip = 0;
    for mip in 0..mips {
        let dim = size >> mip;
        let n = (dim / 4).pow(2) * block;
        total += n;
        if dim > 16 {
            skip += n;
        }
    }
    ensure!(
        bytes.len() == 128 + total * 6 + 1,
        "probe cube length mismatch"
    );
    let mut result = bytes[..128].to_vec();
    for (o, n) in [(12, 16), (16, 16), (20, (total - skip) * 6), (28, 3)] {
        result[o..o + 4].copy_from_slice(&(n as u32).to_le_bytes());
    }
    for face in 0..6 {
        result.extend(&bytes[128 + face * total + skip..128 + (face + 1) * total]);
    }
    result.push(bytes[bytes.len() - 1]);
    Ok(result)
}

/// Native RMS cache: sample period, duration in milliseconds, count, floats.
/// Decode the first channel as the original sound_rms constructor does.
pub(super) fn sound_rms(ogg: &[u8]) -> Result<Vec<u8>> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            "pipe:0",
            "-af",
            "pan=mono|c0=c0",
            "-ar",
            "48000",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("ffmpeg is required to convert early sound caches")?;
    let mut input = child.stdin.take().unwrap();
    let data = ogg.to_vec();
    let writer = std::thread::spawn(move || input.write_all(&data));
    let output = child.wait_with_output()?;
    let written = writer
        .join()
        .map_err(|_| anyhow::anyhow!("audio input writer panicked"))?;
    ensure!(
        output.status.success(),
        "ffmpeg: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    written?;
    rms_from_pcm(&output.stdout)
}

fn rms_from_pcm(pcm: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        !pcm.is_empty() && pcm.len() % 4 == 0,
        "invalid decoded audio"
    );
    let samples = pcm.len() / 4;
    let count = samples.div_ceil(960);
    let mut result = Vec::new();
    result.extend(0.02f32.to_le_bytes());
    result.extend(u32::try_from(samples * 1000 / 48000)?.to_le_bytes());
    result.extend(u32::try_from(count)?.to_le_bytes());
    for window in pcm.chunks(960 * 4) {
        let mut sum = 0f32;
        for value in window.chunks_exact(4) {
            let value = f32::from_le_bytes(value.try_into().unwrap());
            ensure!(value.is_finite(), "nonfinite decoded audio");
            sum += value * value;
        }
        result.extend((sum / (window.len() / 4) as f32).sqrt().to_le_bytes());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chemical_duplicate_zone_is_repaired_once_and_guarded() {
        let mut collectors = Vec::new();
        for team in 0..2 {
            let mut collector = ConfigValue::Named(vec![]);
            collector
                .set_path("team", ConfigValue::Integer(team))
                .unwrap();
            collector
                .set_path(
                    "full_name",
                    ConfigValue::String(format!("logic/team{}/victory_items_container", team + 1)),
                )
                .unwrap();
            let reference = ConfigValue::Named(vec![NamedValue {
                name: "name".into(),
                value: ConfigValue::String("logic/team1/victory_container_collision".into()),
            }]);
            collector
                .set_path(
                    "collision_geometries",
                    ConfigValue::Indexed(vec![reference]),
                )
                .unwrap();
            collectors.push(collector);
        }
        let mut groups = ConfigValue::Named(vec![]);
        groups
            .set_path("victory_items_containers", ConfigValue::Indexed(collectors))
            .unwrap();
        let mut zone = ConfigValue::Named(vec![]);
        zone.set_path(
            "game_object_type",
            ConfigValue::String("collision_geometry".into()),
        )
        .unwrap();
        zone.set_path(
            "full_name",
            ConfigValue::String("logic/team2/victory_container_collision".into()),
        )
        .unwrap();
        zone.set_path("position", ConfigValue::Float3([102.606, 6.87, 79.873]))
            .unwrap();
        assert!(repair_chemical_collector_zone(&mut groups.clone(), &[]).is_err());
        let before = groups.clone();
        assert!(repair_chemical_collector_zone(&mut groups, &[zone.clone()]).unwrap());
        assert!(!repair_chemical_collector_zone(&mut groups, &[zone.clone()]).unwrap());
        let old = array(before.named("victory_items_containers").unwrap()).unwrap();
        let new = array(groups.named("victory_items_containers").unwrap()).unwrap();
        assert_eq!(new[0], old[0]);
        assert_eq!(new[1].named("team"), old[1].named("team"));
        zone.set_path("position", ConfigValue::Float3([0.; 3]))
            .unwrap();
        assert!(repair_chemical_collector_zone(&mut before.clone(), &[zone]).is_err());
    }

    #[test]
    fn early_respawn_protection_uses_native_seconds() {
        let point = |value: Option<i32>| {
            let mut point = ConfigValue::Named(vec![]);
            point.set_path("team", ConfigValue::Integer(1)).unwrap();
            if let Some(value) = value {
                point
                    .set_path("invulnerability_time", ConfigValue::Integer(value))
                    .unwrap();
            }
            point
        };
        let groups = |values: &[Option<i32>]| {
            let mut groups = ConfigValue::Named(vec![]);
            groups
                .set_path(
                    "respawn_points",
                    ConfigValue::Indexed(values.iter().map(|v| point(*v)).collect()),
                )
                .unwrap();
            groups
        };
        // 0.11a/0.11e author 2000 ms; 0.10b omits the field (native default).
        let mut early = groups(&[Some(2000), None, Some(0)]);
        assert_eq!(convert_respawn_invulnerability(&mut early).unwrap(), 2);
        assert_eq!(early, groups(&[Some(2), None, Some(0)]));
        // Inexact or already-native values must not be silently reinterpreted.
        assert!(convert_respawn_invulnerability(&mut groups(&[Some(2500)])).is_err());
        assert!(convert_respawn_invulnerability(&mut early.clone()).is_err());
        assert_eq!(
            convert_respawn_invulnerability(&mut ConfigValue::Named(vec![])).unwrap(),
            0
        );
    }

    #[test]
    fn collectors_follow_named_zones_when_editor_markers_overlap() {
        fn object(fields: &[(&str, ConfigValue)]) -> ConfigValue {
            ConfigValue::Named(
                fields
                    .iter()
                    .map(|(name, value)| NamedValue {
                        name: (*name).into(),
                        value: value.clone(),
                    })
                    .collect(),
            )
        }
        let text = |s: &str| ConfigValue::String(s.into());
        let zone = |name: &str, x| {
            object(&[
                ("game_object_type", text("collision_geometry")),
                ("full_name", text(name)),
                ("position", ConfigValue::Float3([x, 6., 40.])),
            ])
        };
        let visual = |x| {
            object(&[
                ("lib_name", text("item/quest/container")),
                ("position", ConfigValue::Float3([x, 5.6, 40.])),
            ])
        };
        let collector = |name: &str| {
            object(&[
                ("position", ConfigValue::Float3([-73., 5.6, 40.])),
                (
                    "collision_geometries",
                    ConfigValue::Indexed(vec![object(&[("name", text(name))])]),
                ),
            ])
        };
        let zones = [zone("west", -74.), zone("east", 103.)];
        let visuals = [visual(-74.), visual(103.)];
        assert_eq!(
            collector_visual(&collector("west"), &zones, &visuals).unwrap(),
            &visuals[0]
        );
        assert_eq!(
            collector_visual(&collector("east"), &zones, &visuals).unwrap(),
            &visuals[1]
        );
        assert!(collector_visual(&collector("missing"), &zones, &visuals).is_err());
        assert!(collector_visual(&collector("east"), &zones, &visuals[..1]).is_err());
        let respawns = ConfigValue::Indexed(vec![visual(12.)]);
        let client = object(&[
            ("respawn_points", respawns.clone()),
            ("objects", ConfigValue::Indexed(zones.to_vec())),
        ]);
        let mut server = object(&[(
            "objects",
            object(&[
                ("respawn_points", respawns),
                ("collision_geometry", ConfigValue::Indexed(zones.to_vec())),
                ("victory_items_containers", ConfigValue::Indexed(vec![])),
            ]),
        )]);
        validate_server_fallback(&client, &server).unwrap();
        server
            .set_path(
                "objects.respawn_points",
                ConfigValue::Indexed(vec![visual(13.)]),
            )
            .unwrap();
        assert!(validate_server_fallback(&client, &server).is_err());
        server
            .set_path(
                "objects.respawn_points",
                client.named("respawn_points").unwrap().clone(),
            )
            .unwrap();
        server
            .set_path(
                "objects.collision_geometry",
                ConfigValue::Indexed(vec![zone("west", -70.), zones[1].clone()]),
            )
            .unwrap();
        assert!(validate_server_fallback(&client, &server).is_err());
    }

    #[test]
    fn early_wire_sets_are_kept_with_their_authored_points() {
        fn object(fields: &[(&str, ConfigValue)]) -> ConfigValue {
            ConfigValue::Named(
                fields
                    .iter()
                    .map(|(name, value)| NamedValue {
                        name: (*name).into(),
                        value: value.clone(),
                    })
                    .collect(),
            )
        }
        let text = |s: &str| ConfigValue::String(s.into());
        let wire = object(&[
            ("game_object_type", text("wire_set")),
            ("lib_name", text("wire_set")),
            ("full_name", text("objects/wire/wire_set(9)")),
            ("position", ConfigValue::Float3([-2.006, 8.775, -49.581])),
            ("points_count", ConfigValue::Integer(2)),
            (
                "points",
                ConfigValue::Indexed(vec![
                    ConfigValue::Float3([0., 0., 0.]),
                    ConfigValue::Float3([-18.874, 0.037, 0.071]),
                ]),
            ),
            ("wire_width", ConfigValue::Float(0.03)),
        ]);
        let old = object(&[
            ("objects", ConfigValue::Indexed(vec![wire.clone()])),
            ("render_visuals", ConfigValue::Indexed(vec![])),
        ]);
        let server = object(&[("objects", object(&[]))]);
        let target = object(&[("client_objects", ConfigValue::Indexed(vec![]))]);
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("input");
        fs::create_dir_all(root.join("unused")).unwrap();
        fs::write(root.join("unused/file"), b"x").unwrap();
        let db_path = temp.path().join("donor.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&root).unwrap(),
            &db_path,
        )
        .unwrap();
        let mut donor = ResourceDb::open(db_path).unwrap();
        let map = Map {
            id: 195,
            level: "level_03".into(),
            weather: "day".into(),
        };
        let project = project(
            old,
            &mut donor,
            server,
            &target,
            &map,
            SourceBuild::V011e,
            &mut Vec::new(),
        )
        .unwrap();
        let client = array(project.named("client_objects").unwrap()).unwrap();
        assert_eq!(client, &[wire]);
    }

    #[test]
    fn early_environment_preserves_height_wind_and_sun() {
        let mut old = ConfigValue::Named(Vec::new());
        for (name, value) in [
            ("skylight_lower", ConfigValue::Integer(0)),
            ("skylight_upper", ConfigValue::Integer(5)),
            ("skylight_power", ConfigValue::Float(0.1)),
            ("skylight_color", ConfigValue::Float3([0.816, 0.897, 1.])),
            ("skylight_multiplier", ConfigValue::Float(0.5)),
            ("wind_strength", ConfigValue::Float(0.1)),
            ("wind_direction", ConfigValue::Float3([1., 0., 1.])),
            ("environment_near_fog_distance", ConfigValue::Integer(0)),
            ("environment_far_fog_distance", ConfigValue::Integer(200)),
            (
                "environment_far_fog_color",
                ConfigValue::Float4([0., 0., 0., 1.]),
            ),
            ("environment_fog_alpha", ConfigValue::Float(0.)),
            ("atmosphere_inscatter_distance", ConfigValue::Integer(200)),
            (
                "god_rays_color_0",
                ConfigValue::Float4([1., 0.2068795, 0.1102941, 1.]),
            ),
            ("bloom_halo_color", ConfigValue::Float3([0.7, 0.5, 1.])),
            (
                "sky_clouds_color",
                ConfigValue::Float4([0.9860015, 0.9614621, 0.8960299, 1.]),
            ),
            ("sky_clouds_color_multiplier", ConfigValue::Integer(3)),
            ("dof_near_blurriness_far", ConfigValue::Float(0.8)),
        ] {
            old.set_path(
                name,
                ConfigValue::Named(vec![NamedValue {
                    name: "value".into(),
                    value,
                }]),
            )
            .unwrap();
        }
        let mut dst = ConfigValue::Named(Vec::new());
        dst.set_path("use_height_based_ambient", ConfigValue::Boolean(false))
            .unwrap();
        dst.set_path("sky_clouds_color", ConfigValue::Float4([1.; 4]))
            .unwrap();
        dst.set_path("rayleigh_fog_near_distance", ConfigValue::Float(35.))
            .unwrap();
        let mut sun = ConfigValue::Named(Vec::new());
        for (name, value) in [
            ("color", ConfigValue::Float3([0.973, 0.973, 1.])),
            ("intensity", ConfigValue::Float(0.07)),
            ("is_cast_shadow", ConfigValue::Boolean(true)),
            ("rotation", ConfigValue::Float3([2.481, 0.88, 3.138])),
        ] {
            sun.set_path(name, value).unwrap();
        }
        transfer_environment(&mut dst, &old, Some(&sun), &mut Vec::new(), "test").unwrap();
        let az = number(dst.named("sun_position_azimut").unwrap())
            .unwrap()
            .to_radians();
        let el = number(dst.named("sun_angle").unwrap())
            .unwrap()
            .to_radians();
        let actual = [-az.cos() * el.cos(), el.sin(), -az.sin() * el.cos()];
        // Factory rain's original negated rotation k-axis, also checked in
        // the target runtime constant buffer after converting its azimuth.
        for (actual, expected) in actual.into_iter().zip([0.6063892, 0.6157675, 0.5031128]) {
            assert!((actual - expected).abs() < 1e-5);
        }
        assert_eq!(dst.named("wind_strength"), Some(&ConfigValue::Float(0.1)));
        assert_eq!(
            dst.named("wind_direction"),
            Some(&ConfigValue::Float3([1., 0., 1.]))
        );
        assert_eq!(
            dst.named("use_height_based_ambient"),
            Some(&ConfigValue::Boolean(true))
        );
        assert_eq!(
            dst.named("height_based_ambient_power"),
            Some(&ConfigValue::Float(0.1))
        );
        // 0.26 raises each colour to 2.2 at load; the result must equal the
        // source upload pow(colour, 2.2) * multiplier (and raw god rays/bloom).
        let loaded = |key: &str| {
            let Some(ConfigValue::Float4(value)) = dst.named(key) else {
                panic!("{key} missing")
            };
            [value[0].powf(2.2), value[1].powf(2.2), value[2].powf(2.2)]
        };
        let close = |actual: [f32; 3], expected: [f32; 3]| {
            for (a, e) in actual.into_iter().zip(expected) {
                assert!((a - e).abs() < 1e-5, "{actual:?} != {expected:?}");
            }
        };
        let source = |c: [f32; 3], m: f32| c.map(|v| v.powf(2.2) * m);
        close(
            loaded("height_based_ambient_high_color"),
            source([0.816, 0.897, 1.], 0.5),
        );
        close(
            loaded("sky_clouds_color"),
            source([0.9860015, 0.9614621, 0.8960299], 3.),
        );
        close(loaded("god_rays_color_1"), [1., 0.2068795, 0.1102941]);
        close(loaded("bloom_color"), [0.7, 0.5, 1.]);
        assert_eq!(
            dst.named("rayleigh_fog_near_distance"),
            Some(&ConfigValue::Float(0.))
        );
        assert_eq!(
            dst.named("dof_far_blurness_amount"),
            Some(&ConfigValue::Float(0.8))
        );
    }

    #[test]
    fn early_global_probe_keeps_full_weight_inside_its_sphere() {
        let mut probe = ConfigValue::Named(vec![
            NamedValue {
                name: "texture".into(),
                value: ConfigValue::String("rain/probe".into()),
            },
            NamedValue {
                name: "radius".into(),
                value: ConfigValue::Integer(300),
            },
            NamedValue {
                name: "geometry".into(),
                value: ConfigValue::Integer(0),
            },
        ]);
        transfer_probe(&mut probe, "rain", SourceBuild::V010b).unwrap();
        let width = number(probe.named("probe_side_width").unwrap()).unwrap();
        // Observed 0.26 upload: outer = 0.98*radius, inner = radius-width-0.05.
        let outer = 294.;
        let inner = 300. - width - 0.05;
        assert!(inner < outer);
        let t = ((100. - inner) / (outer - inner)).clamp(0., 1.);
        assert_eq!(1. - t * t * (3. - 2. * t), 1.);
        let textures = probe.named("textures").unwrap();
        assert_eq!(textures.named("rain_2"), textures.named("default"));
        assert_eq!(
            textures.named("default"),
            Some(&ConfigValue::String("rain/probe".into()))
        );
        probe
            .set_path("probe_side_width", ConfigValue::Float(0.5))
            .unwrap();
        transfer_probe(&mut probe, "day", SourceBuild::V011e).unwrap();
        assert_eq!(
            probe.named("probe_side_width"),
            Some(&ConfigValue::Float(0.5))
        );
    }

    #[test]
    fn rms_cache_has_native_header_and_includes_partial_window() {
        let pcm: Vec<_> = (0..961).flat_map(|_| 0.5f32.to_le_bytes()).collect();
        let rms = rms_from_pcm(&pcm).unwrap();
        assert_eq!(rms.len(), 20);
        assert_eq!(&rms[4..8], &20u32.to_le_bytes());
        assert_eq!(&rms[8..12], &2u32.to_le_bytes());
        assert_eq!(&rms[12..16], &0.5f32.to_le_bytes());
        assert_eq!(&rms[16..20], &0.5f32.to_le_bytes());
        assert!(rms_from_pcm(&[0]).is_err());
    }

    #[test]
    fn early_sound_reference_tracks_renamed_options() {
        let paths = BTreeSet::from(["sounds/single/weather/rain/rain.single_sound_options".into()]);
        let mut name = ConfigValue::String("weather/rain/rain".into());
        rewrite_inner(
            &mut name,
            &replacements(&paths, SourceBuild::V010b),
            true,
            Some("sound"),
        );
        assert_eq!(
            name,
            ConfigValue::String("ported/v010b/weather/rain/rain".into())
        );
    }

    #[test]
    fn static_and_colored_geometry_preserve_every_field_except_uv_encoding() {
        for colored in [false, true] {
            let mut native = Vec::new();
            native.extend(3u32.to_le_bytes());
            native.extend((if colored { 36u32 } else { 32u32 }).to_le_bytes());
            native.extend(1u32.to_le_bytes());
            for p in [12f32, -1., 55.] {
                native.extend(p.to_le_bytes());
            }
            native.extend([127, 255, 0, 127, 255, 127, 127, 0, 127, 0, 255, 64]);
            native.extend(0x3400u16.to_le_bytes());
            native.extend(0xb800u16.to_le_bytes());
            if colored {
                native.extend([1, 2, 3, 4]);
            }
            native.extend(4u32.to_le_bytes());
            native.extend(10u32.to_le_bytes());
            native.extend(3u32.to_le_bytes());
            native.extend([0; 6]);
            let crate::map_port::StaticUpgrade::Converted(old) =
                crate::map_port::upgrade_stripped_static_model(&native).unwrap()
            else {
                panic!("expected old layout");
            };
            assert_eq!(geometry(&old, false).unwrap(), native);
            assert!(geometry(&old[..old.len() - 1], false).is_err());
        }
        // Large authored repeat offsets can collapse distinct UVs in half precision.
        let mut native = Vec::new();
        native.extend(3u32.to_le_bytes());
        native.extend(88u32.to_le_bytes());
        native.extend(3u32.to_le_bytes());
        for _ in 0..3 {
            native.extend([0u8; 28]);
        }
        native.extend(4u32.to_le_bytes());
        native.extend(10u32.to_le_bytes());
        native.extend(3u32.to_le_bytes());
        native.extend([0, 0, 1, 0, 2, 0]);
        let crate::map_port::StaticUpgrade::Converted(mut old) =
            crate::map_port::upgrade_stripped_static_model(&native).unwrap()
        else {
            panic!("expected old layout");
        };
        let end = 8 + u32::from_le_bytes(old[4..8].try_into().unwrap()) as usize;
        let start = end - 3 * 32;
        for (i, uv) in [[-128.9f32, 112.4f32], [-128.89, 112.4], [-128.9, 112.41]]
            .iter()
            .enumerate()
        {
            for (axis, v) in uv.iter().enumerate() {
                old[start + i * 32 + 24 + axis * 4..start + i * 32 + 28 + axis * 4]
                    .copy_from_slice(&v.to_le_bytes());
            }
        }
        let packed = geometry(&old, false).unwrap();
        let fixed = geometry(&old, true).unwrap();
        assert_eq!(&packed[36..40], &packed[64..68]);
        assert_ne!(&fixed[36..40], &fixed[64..68]);
        for i in 0..3 {
            assert_eq!(
                &fixed[12 + i * 28..36 + i * 28],
                &packed[12 + i * 28..36 + i * 28]
            );
        }
        assert_eq!(&fixed[96..], &packed[96..]);
        assert_eq!(
            &fixed[36..40],
            &[
                half(-128.9 + 129.).unwrap().to_le_bytes(),
                half(112.4 - 112.).unwrap().to_le_bytes()
            ]
            .concat()
        );
    }

    #[test]
    fn diffuse_cube_keeps_each_original_face_and_color_space() {
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        for (o, n) in [(12, 32u32), (16, 32), (28, 4), (112, 0xfe00)] {
            bytes[o..o + 4].copy_from_slice(&n.to_le_bytes());
        }
        bytes[84..88].copy_from_slice(b"DXT1");
        for face in 0..6 {
            bytes.extend(vec![face; 680]);
        }
        bytes.push(1);
        let result = diffuse_probe(&bytes).unwrap();
        assert_eq!(result.len(), 128 + 6 * 168 + 1);
        for face in 0..6 {
            assert!(
                result[128 + face * 168..128 + (face + 1) * 168]
                    .iter()
                    .all(|v| *v == face as u8)
            );
        }
        assert_eq!(result.last(), Some(&1));
        assert!(diffuse_probe(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn weather_projects_keep_separate_original_grass_directories() {
        let chemical = Map {
            id: 194,
            level: "level_04".into(),
            weather: "day".into(),
        };
        assert_eq!(
            chemical.output_project(SourceBuild::V011e),
            "projects/v011e_level_04/day_2/client_project"
        );
        assert_eq!(SourceBuild::V011e.label(), "0.11e-884");
        assert_eq!(
            serde_json::from_str::<SourceBuild>("\"0.11e\"").unwrap(),
            SourceBuild::V011e
        );
        let map = Map {
            id: 181,
            level: "level_03".into(),
            weather: "evening".into(),
        };
        assert_eq!(
            map.output_project(SourceBuild::V010b),
            "projects/v010b_level_03_evn/evening_2/client_project"
        );
        assert_eq!(
            relocate_path("projects/level_03_evn/vegetation/a.veg", SourceBuild::V010b),
            "projects/v010b_level_03_evn/vegetation/a.veg"
        );
    }

    #[test]
    fn vegetation_keeps_authored_positions_and_rejects_truncation() {
        let mut bytes = Vec::new();
        for (id, values) in [
            (0x4du32, vec![]),
            (0x58, vec![12f32, 2., -9.]),
            (0x59, vec![0., 1., 0.]),
        ] {
            bytes.extend(id.to_le_bytes());
            bytes.extend(((values.len() * 4) as u32).to_le_bytes());
            for v in values {
                bytes.extend(v.to_le_bytes());
            }
        }
        assert_eq!(vegetation_instances(&bytes).unwrap(), 1);
        assert!(vegetation_instances(&bytes[..bytes.len() - 1]).is_err());
        bytes[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(vegetation_instances(&bytes).is_err());
    }

    #[test]
    fn half_uv_rounding_and_limits() {
        for (v, bits) in [
            (0., 0),
            (-0., 0x8000),
            (1., 0x3c00),
            (-2., 0xc000),
            (65504., 0x7bff),
            (2f32.powi(-24), 1),
            (1. + 2f32.powi(-11), 0x3c00),
        ] {
            assert_eq!(half(v).unwrap(), bits);
        }
        for v in [f32::NAN, f32::INFINITY, 65536.] {
            assert!(half(v).is_err());
        }
    }
}
