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

fn transfer_probe(object: &mut ConfigValue, weather: &str) -> Result<()> {
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
    target: &ConfigValue,
    map: &Map,
    source: SourceBuild,
    notes: &mut Vec<String>,
) -> Result<ConfigValue> {
    let path = map.donor_project(source);
    let server = cfg(donor, &path.replace("client_project", "server_project"))?;
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
        if kind == "wire_set" {
            // Kept for static wire emission after dependency discovery.
            notes.push(format!(
                "{path}: wire requires static reconstruction: {}",
                string(object, "full_name").unwrap_or("wire")
            ));
            continue;
        }
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
            transfer_probe(&mut object, &map.weather)?;
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
    // Old collectors are already rendered as placed static models. Bind that
    // original mesh and transform instead of inheriting a modern collector.
    let mut collector_visuals = Vec::new();
    if let Some(ConfigValue::Indexed(collectors)) = groups.named_mut("victory_items_containers") {
        let visuals = array(old.named("render_visuals").context("old visuals")?)?;
        for collector in collectors {
            let ConfigValue::Float3(pos) =
                collector.named("position").context("collector position")?
            else {
                bail!("collector position type")
            };
            let visual = visuals
                .iter()
                .filter(|v| {
                    string(v, "lib_name")
                        .is_some_and(|s| s.contains("container") || s.contains("atlant"))
                })
                .filter_map(|v| match v.named("position") {
                    Some(ConfigValue::Float3(p)) => Some((
                        v,
                        p.iter()
                            .zip(pos)
                            .map(|(a, b)| (a - b) * (a - b))
                            .sum::<f32>(),
                    )),
                    _ => None,
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .context("no old collector mesh")?
                .0;
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
    // Restore the pre-brightening mapping pending a renderer-compatible fix.
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
    let multiplier = number(get("skylight_multiplier")?)?;
    for c in &mut color[..3] {
        *c *= multiplier;
    }
    dst.set_path(
        "height_based_ambient_high_color",
        ConfigValue::Float4(color),
    )?;
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
    notes.push(format!("{path}: original sky texture, post-process values and sun translated; pre-brightening HBA mapping restored; this is not equivalent to early skylight; unmapped controls require visual comparison: {}",omitted.join(", ")));
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

pub(super) fn geometry(bytes: &[u8]) -> Result<Vec<u8>> {
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
    let mut result = Vec::new();
    result.extend(3u32.to_le_bytes());
    result.extend(u32::try_from(4 + count * (stride - 4))?.to_le_bytes());
    result.extend(u32::try_from(count)?.to_le_bytes());
    for vertex in bytes[start..end].chunks_exact(stride) {
        result.extend(&vertex[..24]);
        for offset in [24, 28] {
            result.extend(
                half(f32::from_le_bytes(
                    vertex[offset..offset + 4].try_into().unwrap(),
                ))?
                .to_le_bytes(),
            );
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
        assert_eq!(
            dst.named("height_based_ambient_high_color"),
            Some(&ConfigValue::Float4([0.408, 0.4485, 0.5, 1.]))
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
        transfer_probe(&mut probe, "rain").unwrap();
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
            assert_eq!(geometry(&old).unwrap(), native);
            assert!(geometry(&old[..old.len() - 1]).is_err());
        }
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
