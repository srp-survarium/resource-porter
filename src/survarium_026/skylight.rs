// SPDX-License-Identifier: GPL-3.0-or-later
//! A map-wide environment probe carrying the 0.10b skylight of a converted
//! X-Ray map.
//!
//! 0.10b lights every pixel with its skylight stage first (`skylight.ps`): an
//! ambient cube of six per-axis colours weighted by the squared world normal,
//! times `pow(skylight_color, 2.2) * skylight_multiplier`, the height ramp and
//! SSAO x 2.5. 0.26 has no skylight stage. Its ambient is environment-probe
//! light, multiplied afterwards by the height-based ambient colour and SSAO.
//! A scene without probes therefore has no ambient at all in 0.26: every pixel
//! the sun or a lamp does not reach is black (the converted X-Ray maps).
//!
//! The probe emitted here restores the 0.10b term in 0.26's own contract: a
//! native global sphere probe enclosing the map, whose cube holds the source
//! ambient cube times the source SSAO gain. The height-based ambient colour
//! (converted from the source skylight colour and multiplier by
//! `transfer_environment`) then supplies the rest of the 0.10b product.
use super::*;

/// Placement and strength of the skylight probe. Every field has a default.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkylightProbe {
    /// Factor on the source ambient cube. 0.10b multiplies its skylight by
    /// SSAO x 2.5 (`skylight.ps`); 0.26 multiplies probe light by SSAO alone.
    #[serde(default = "default_gain")]
    pub gain: f32,
    /// Probe reflection strength. The 0.10b skylight has no reflection; the
    /// native level_03 global probe uses 0.3.
    #[serde(default = "default_specular")]
    pub specular: f32,
    /// Metres added around the map's render bounds.
    #[serde(default = "default_margin")]
    pub margin: f32,
    /// Take the height-based ambient colour from the native template (warm
    /// white on level_03 day) instead of the source skylight colour. The
    /// cube keeps the source's axis colours; `gain` then sets the level.
    /// Every converted X-Ray map inherited the same cold 0.10b template
    /// skylight (0.816, 0.897, 1.0), which reads blue under 0.26's grading.
    #[serde(default)]
    pub native_tint: bool,
    /// Box probes that darken covered interiors. 0.10b's skylight lit every
    /// room like open ground (only SSAO darkened it); native 0.26 maps place
    /// box probes with low multipliers over their interiors instead. Each box
    /// uses the same cube at `diffuse` times the map-wide probe's strength.
    /// Proposed from sky visibility by tools/playable-map-pack/interior_probes.py.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interiors: Vec<InteriorBox>,
}

/// One interior box probe, in world coordinates.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InteriorBox {
    pub name: String,
    pub centre: [f32; 3],
    /// Half extents along the box's own axes (x, y up, z) before `yaw`.
    pub half: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
    /// Strength relative to the map-wide skylight probe (diffuse, and
    /// specular alike).
    pub diffuse: f32,
    pub reason: String,
}

/// Width of a box probe's falloff (metres inside each face). 0.26 builds the
/// inner box by subtracting `probe_side_width` from each half extent and
/// fades smoothly between the inner and outer box (box PS: `t = saturate((
/// |q| * outer - inner) / (outer - inner))`, alpha = prod(1 - smoothstep)).
pub const INTERIOR_SIDE_WIDTH: f32 = 1.0;

fn default_gain() -> f32 {
    2.5
}
fn default_specular() -> f32 {
    0.3
}
fn default_margin() -> f32 {
    10.0
}

impl Default for SkylightProbe {
    fn default() -> Self {
        Self {
            gain: default_gain(),
            specular: default_specular(),
            margin: default_margin(),
            native_tint: false,
            interiors: Vec::new(),
        }
    }
}

/// The six 0.10b skylight colours as world-axis colours, linear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbientCube {
    /// +x, -x, +y, -y, +z, -z.
    pub faces: [[f32; 3]; 6],
}

impl AmbientCube {
    /// 0.10b `skylight.ps`: sum over axes of n_i^2 times that axis' colour on
    /// the side the normal points to.
    pub fn eval(&self, d: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0; 3];
        for axis in 0..3 {
            let face = &self.faces[axis * 2 + usize::from(d[axis] < 0.0)];
            for c in 0..3 {
                out[c] += d[axis] * d[axis] * face[c];
            }
        }
        out
    }
}

/// Read the ambient cube from a 0.10b post-process material. Absent keys keep
/// 0.10b's constructor value (white). The material loader maps the authored
/// names onto the shader's axes (material_manager.cpp:377-393); every
/// converted X-Ray map authors the same colour on opposite sides, so only the
/// vertical/horizontal split matters.
pub fn ambient_cube(post: &ConfigValue) -> Result<AmbientCube> {
    let color = |key: &str| -> Result<[f32; 3]> {
        let Some(value) = post.named(key).and_then(|v| v.named("value")) else {
            return Ok([1.0; 3]);
        };
        let v = match value {
            ConfigValue::Float4(v) => [v[0], v[1], v[2]],
            ConfigValue::Float3(v) => *v,
            _ => bail!("{key} is not a colour"),
        };
        ensure!(
            v.iter().all(|c| c.is_finite() && *c >= 0.0),
            "{key} is not a finite non-negative colour"
        );
        Ok(v)
    };
    Ok(AmbientCube {
        faces: [
            color("environment_skycolor_x")?,
            color("environment_skycolor_minus_x")?,
            color("environment_skycolor_up")?,
            color("environment_skycolor_down")?,
            color("environment_skycolor_z")?,
            color("environment_skycolor_minus_z")?,
        ],
    })
}

/// Direction of texel centre (x, y) of D3D cube face `face` (+x, -x, +y, -y,
/// +z, -z) at size `size`.
fn texel_direction(face: usize, x: usize, y: usize, size: usize) -> [f32; 3] {
    let u = (2.0 * (x as f32 + 0.5) / size as f32) - 1.0;
    let v = (2.0 * (y as f32 + 0.5) / size as f32) - 1.0;
    let d = match face {
        0 => [1.0, -v, -u],
        1 => [-1.0, -v, u],
        2 => [u, 1.0, v],
        3 => [u, -1.0, -v],
        4 => [u, -v, 1.0],
        _ => [-u, -v, -1.0],
    };
    let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    [d[0] / n, d[1] / n, d[2] / n]
}

/// Constant alpha of the encoded cube: 0.26 decodes a probe texel as
/// pow(rgb / a, 2.2) (native probes store a <= 1 for values above 1).
fn encoding_alpha(peak: f32) -> f32 {
    // Leave 3% colour headroom for 5:6:5 rounding.
    (0.97 / peak.max(1e-6).powf(1.0 / 2.2)).min(1.0)
}

fn to565(rgb: [f32; 3]) -> u16 {
    let q = |v: f32, max: f32| ((v.clamp(0.0, 1.0) * max).round()) as u16;
    (q(rgb[0], 31.0) << 11) | (q(rgb[1], 63.0) << 5) | q(rgb[2], 31.0)
}

fn from565(c: u16) -> [f32; 3] {
    [
        ((c >> 11) & 31) as f32 / 31.0,
        ((c >> 5) & 63) as f32 / 63.0,
        (c & 31) as f32 / 31.0,
    ]
}

/// One DXT5 block: constant alpha, two colour endpoints from the block's
/// per-channel range, nearest of the four palette entries per texel.
fn dxt5_block(texels: &[[f32; 3]; 16], alpha: u8) -> [u8; 16] {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for t in texels {
        for c in 0..3 {
            lo[c] = lo[c].min(t[c]);
            hi[c] = hi[c].max(t[c]);
        }
    }
    let (mut c0, mut c1) = (to565(hi), to565(lo));
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let mut block = [0u8; 16];
    // Alpha: a0 = a1 = alpha, every index 0.
    block[0] = alpha;
    block[1] = alpha;
    block[8..10].copy_from_slice(&c0.to_le_bytes());
    block[10..12].copy_from_slice(&c1.to_le_bytes());
    if c0 == c1 {
        return block;
    }
    let (p0, p1) = (from565(c0), from565(c1));
    let palette = [
        p0,
        p1,
        [0, 1, 2].map(|c| (2.0 * p0[c] + p1[c]) / 3.0),
        [0, 1, 2].map(|c| (p0[c] + 2.0 * p1[c]) / 3.0),
    ];
    let mut indices = 0u32;
    for (i, t) in texels.iter().enumerate() {
        let best = (0..4)
            .min_by(|&a, &b| {
                let e = |p: &[f32; 3]| (0..3).map(|c| (p[c] - t[c]).powi(2)).sum::<f32>();
                e(&palette[a]).total_cmp(&e(&palette[b]))
            })
            .unwrap();
        indices |= (best as u32) << (2 * i);
    }
    block[12..16].copy_from_slice(&indices.to_le_bytes());
    block
}

/// A DXT5 cube DDS of `size` with mips down to 4x4, every texel the gained
/// ambient cube at its direction, laid out like the native probe `header`
/// (header bytes are copied, sizes and mip count rewritten, the trailing
/// colour-space byte kept).
pub fn cube_dds(header: &[u8], cube: &AmbientCube, gain: f32, size: usize) -> Result<Vec<u8>> {
    ensure!(
        header.len() >= 129 && &header[..4] == b"DDS " && &header[84..88] == b"DXT5",
        "probe template is not a DXT5 DDS"
    );
    ensure!(
        u32::from_le_bytes(header[112..116].try_into().unwrap()) & 0xfe00 == 0xfe00,
        "probe template is not a six-face cube"
    );
    ensure!(size.is_power_of_two() && (8..=1024).contains(&size), "unsupported probe size");
    ensure!(gain.is_finite() && gain > 0.0, "invalid skylight gain");
    let peak = cube
        .faces
        .iter()
        .flatten()
        .fold(0.0f32, |m, v| m.max(*v))
        * gain;
    ensure!(peak > 0.0 && peak.is_finite(), "skylight cube is black");
    let alpha = encoding_alpha(peak);
    let alpha_byte = (alpha * 255.0).round() as u8;
    // The decode divides by the stored alpha, so use the quantized value.
    let alpha = alpha_byte as f32 / 255.0;
    let mips = size.ilog2() as usize - 1;
    let mut data = Vec::new();
    for face in 0..6 {
        for mip in 0..mips {
            let dim = size >> mip;
            for by in (0..dim).step_by(4) {
                for bx in (0..dim).step_by(4) {
                    let mut texels = [[0.0f32; 3]; 16];
                    for (i, t) in texels.iter_mut().enumerate() {
                        let d = texel_direction(face, bx + i % 4, by + i / 4, dim);
                        let linear = cube.eval(d);
                        *t = linear.map(|v| (v * gain).max(0.0).powf(1.0 / 2.2) * alpha);
                    }
                    data.extend(dxt5_block(&texels, alpha_byte));
                }
            }
        }
    }
    let mut out = header[..128].to_vec();
    // dwPitchOrLinearSize counts the trailing colour-space byte (native).
    for (offset, value) in [(12, size), (16, size), (20, data.len() + 1), (28, mips)] {
        out[offset..offset + 4].copy_from_slice(&u32::try_from(value)?.to_le_bytes());
    }
    out.extend(data);
    out.push(header[header.len() - 1]);
    Ok(out)
}

/// Union of the map's render bounds: each instanced model's render box,
/// shifted by its instance position (converted X-Ray geometry is in world
/// coordinates with identity placement).
fn render_bounds(project: &ConfigValue, donor: &mut ResourceDb, target: &mut ResourceDb) -> Result<([f32; 3], [f32; 3])> {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut models = 0;
    for visual in array(project.named("render_visuals").context("project has no render visuals")?)? {
        let Some(lib) = string(visual, "lib_name") else { continue };
        let settings = format!("models/{}.model/render/export_properties", lib.replace('\\', "/"));
        let config = if donor.is_file(&settings) {
            cfg(donor, &settings)?
        } else if target.is_file(&settings) {
            cfg(target, &settings)?
        } else {
            continue;
        };
        let (Some(ConfigValue::Float3(min)), Some(ConfigValue::Float3(max))) = (
            config.named("bounding_box").and_then(|b| b.named("min")),
            config.named("bounding_box").and_then(|b| b.named("max")),
        ) else {
            continue;
        };
        let offset = match visual.named("position") {
            Some(ConfigValue::Float3(p)) => *p,
            _ => [0.0; 3],
        };
        for c in 0..3 {
            ensure!(min[c].is_finite() && max[c].is_finite(), "non-finite bounds in {settings}");
            lo[c] = lo[c].min(min[c] + offset[c]);
            hi[c] = hi[c].max(max[c] + offset[c]);
        }
        models += 1;
    }
    ensure!(models > 0, "no render bounds for the skylight probe");
    Ok((lo, hi))
}

/// The native template's map-wide probe: the largest sphere probe.
fn global_template(target_project: &ConfigValue) -> Result<&ConfigValue> {
    array(target_project.named("client_objects").context("target objects")?)?
        .iter()
        .filter(|o| string(o, "game_object_type") == Some("environment_probe"))
        .filter(|o| o.named("geometry").map(|g| matches!(g, ConfigValue::Integer(0) | ConfigValue::Float(0.0))).unwrap_or(true))
        .max_by(|a, b| {
            let r = |o: &ConfigValue| match o.named("radius") {
                Some(ConfigValue::Float(v)) => *v,
                Some(ConfigValue::Integer(v)) => *v as f32,
                _ => 0.0,
            };
            r(a).total_cmp(&r(b))
        })
        .context("native template has no sphere probe")
}

/// The native template's box probe: the indoor box (geometry 1) with the
/// largest volume.
fn box_template(target_project: &ConfigValue) -> Result<&ConfigValue> {
    array(target_project.named("client_objects").context("target objects")?)?
        .iter()
        .filter(|o| string(o, "game_object_type") == Some("environment_probe"))
        .filter(|o| matches!(o.named("geometry"), Some(ConfigValue::Integer(1))))
        .filter(|o| string(o, "full_name").is_some_and(|n| n.contains("indoor")))
        .max_by(|a, b| {
            let v = |o: &ConfigValue| match o.named("scale") {
                Some(ConfigValue::Float3(s)) => s[0] * s[1] * s[2],
                _ => 0.0,
            };
            v(a).total_cmp(&v(b))
        })
        .context("native template has no indoor box probe")
}

fn scalar(value: Option<&ConfigValue>) -> Option<f32> {
    match value {
        Some(ConfigValue::Float(v)) => Some(*v),
        Some(ConfigValue::Integer(v)) => Some(*v as f32),
        _ => None,
    }
}

/// A native box probe object for `entry`, cloned from `template`.
///
/// The box is the unit cube (-1..1) under the probe transform, so `scale`
/// holds half extents (0.10b environment_probe.cpp: the transform's scale,
/// geometry_primitives.cpp cube_solid; native level_03 "indoor" boxes are
/// 26 x 12.6 x 43 m halls at scale 13.16, 6.32, 21.71). `radius` only orders
/// the probe draws (largest first): 0.75 x the largest half extent, as 0.10b
/// derives it, so every box draws over the map-wide sphere.
pub fn box_probe(
    template: &ConfigValue,
    entry: &InteriorBox,
    texture: &str,
    variant: &str,
    specular: f32,
    ambient: f32,
) -> Result<ConfigValue> {
    ensure!(
        entry.centre.iter().chain(&entry.half).all(|v| v.is_finite()) && entry.yaw.is_finite(),
        "non-finite interior box {}",
        entry.name
    );
    ensure!(
        entry.half.iter().all(|h| *h > INTERIOR_SIDE_WIDTH + 0.25 && *h < 200.0),
        "interior box {} must be {}..200 m in each half extent",
        entry.name,
        INTERIOR_SIDE_WIDTH + 0.25
    );
    ensure!(
        entry.diffuse.is_finite() && entry.diffuse > 0.0 && entry.diffuse <= 1.5,
        "interior box {} diffuse {} outside 0..1.5",
        entry.name,
        entry.diffuse
    );
    ensure!(
        !entry.name.is_empty() && entry.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "interior box name must be [A-Za-z0-9_]+"
    );
    let mut probe = template.clone();
    probe.set_path("geometry", ConfigValue::Integer(1))?;
    probe.set_path("position", ConfigValue::Float3(entry.centre))?;
    probe.set_path("rotation", ConfigValue::Float3([0.0, entry.yaw, 0.0]))?;
    probe.set_path("scale", ConfigValue::Float3(entry.half))?;
    let largest = entry.half.iter().fold(0.0f32, |m, v| m.max(*v));
    probe.set_path("radius", ConfigValue::Float(0.75 * largest))?;
    probe.set_path("probe_side_width", ConfigValue::Float(INTERIOR_SIDE_WIDTH))?;
    probe.set_path("texture", ConfigValue::String(texture.into()))?;
    probe.set_path(
        "textures",
        ConfigValue::Named(vec![
            NamedValue { name: variant.into(), value: ConfigValue::String(texture.into()) },
            NamedValue { name: "default".into(), value: ConfigValue::String(texture.into()) },
        ]),
    )?;
    probe.set_path("diffuse_multiplier", ConfigValue::Float(entry.diffuse))?;
    probe.set_path("specular_multiplier", ConfigValue::Float(specular * entry.diffuse))?;
    probe.set_path("ambient_multiplier", ConfigValue::Float(ambient * entry.diffuse))?;
    probe.set_path("enabled", ConfigValue::Boolean(true))?;
    probe.set_path("is_temporary", ConfigValue::Boolean(false))?;
    probe.set_path("name", ConfigValue::String(format!("interior_{}", entry.name)))?;
    probe.set_path(
        "full_name",
        ConfigValue::String(format!("environment_and_lights/probes/interior_{}", entry.name)),
    )?;
    Ok(probe)
}

/// Texture paths of the generated probe, as the probe object names them.
pub fn texture_name(map: &Map, source: SourceBuild) -> String {
    let project = map
        .donor_project(source)
        .trim_start_matches("projects/")
        .trim_end_matches("/client_project")
        .replace('/', "_");
    format!("environment_probes/ported/{}/srp/{project}_skylight", source.prefix())
}

/// Add the skylight probe to `project` (a converted early project) and emit
/// its cube textures. Returns a note for the build record.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit(
    map: &Map,
    settings: &SkylightProbe,
    source: SourceBuild,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    target_project: &ConfigValue,
    project: &mut ConfigValue,
    output: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<String> {
    ensure!(map.height_ambient, "a skylight probe needs height-based ambient (its colour carries the source skylight)");
    ensure!(
        settings.margin.is_finite() && settings.margin >= 0.0 && settings.specular.is_finite() && settings.specular >= 0.0,
        "invalid skylight probe settings"
    );
    let objects = array(project.named("client_objects").context("project objects")?)?;
    ensure!(
        !objects.iter().any(|o| string(o, "game_object_type") == Some("environment_probe")),
        "the map already has environment probes; a skylight probe would double its ambient"
    );
    // The source skylight, from the donor scene's post-process material.
    let donor_scene = cfg(donor, &map.donor_project(source))?;
    let environment = array(donor_scene.named("objects").context("donor objects")?)?
        .iter()
        .find(|o| string(o, "game_object_type") == Some("environment"))
        .context("donor scene has no environment")?;
    let post = string(environment, "post_effect").context("donor environment post effect")?;
    let material = cfg(donor, &format!("material_instances/{post}.material"))?;
    let post = material
        .named("material")
        .and_then(|m| m.named("post_process"))
        .context("donor post-process values")?;
    let cube = ambient_cube(post)?;

    let (lo, hi) = render_bounds(project, donor, target)?;
    let centre = [0, 1, 2].map(|c| 0.5 * (lo[c] + hi[c]));
    let radius = 0.5 * (0..3).map(|c| (hi[c] - lo[c]).powi(2)).sum::<f32>().sqrt() + settings.margin;
    ensure!(radius.is_finite() && radius > 1.0 && radius < 2000.0, "unreasonable skylight probe radius {radius}");

    let template = global_template(target_project)?.clone();
    let template_texture = template
        .named("textures")
        .and_then(|t| t.named(&map.variant()))
        .and_then(|t| match t {
            ConfigValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| string(&template, "texture").map(str::to_owned))
        .context("template probe texture")?;
    let name = texture_name(map, source);
    for (suffix, size) in [("", 64usize), ("_diffuse", 16usize)] {
        let header = target.read(&format!("textures/{template_texture}{suffix}.dds"))?;
        let dds = cube_dds(&header, &cube, settings.gain, size)?;
        let path = format!("textures/{name}{suffix}.dds");
        ensure!(!target.contains(&path), "skylight probe overwrites native {path}");
        emit_file(output, &path, &dds, "srp-skylight-probe", files)?;
        let options = target.read(&format!("textures/{template_texture}{suffix}.options"))?;
        emit_file(output, &format!("textures/{name}{suffix}.options"), &options, "0.26g-probe-options-template", files)?;
    }

    if settings.native_tint {
        let native = array(target_project.named("client_objects").context("target objects")?)?
            .iter()
            .find(|o| string(o, "game_object_type") == Some("environment"))
            .and_then(|o| o.named("properties"))
            .and_then(|p| p.named("height_based_ambient_high_color"))
            .cloned()
            .context("native template has no height-ambient colour")?;
        let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else {
            bail!("project objects are not an array");
        };
        let environment = objects
            .iter_mut()
            .find(|o| string(o, "game_object_type") == Some("environment"))
            .context("converted scene has no environment")?;
        environment.set_path("properties.height_based_ambient_high_color", native)?;
    }
    let boxes = if settings.interiors.is_empty() {
        Vec::new()
    } else {
        let box_template = box_template(target_project)?;
        let ambient = scalar(template.named("ambient_multiplier")).context("global probe ambient multiplier")?;
        let mut names = BTreeSet::new();
        let mut out = Vec::new();
        for entry in &settings.interiors {
            ensure!(names.insert(entry.name.clone()), "duplicate interior box {}", entry.name);
            ensure!(
                (0..3).all(|c| (entry.centre[c] - centre[c]).abs() + entry.half.iter().fold(0.0f32, |m, v| m.max(*v)) < radius),
                "interior box {} reaches outside the skylight sphere",
                entry.name
            );
            out.push(box_probe(box_template, entry, &name, &map.variant(), settings.specular, ambient)?);
        }
        out
    };
    let box_count = boxes.len();
    let mut probe = template;
    probe.set_path("position", ConfigValue::Float3(centre))?;
    probe.set_path("rotation", ConfigValue::Float3([0.0; 3]))?;
    probe.set_path("scale", ConfigValue::Float3([1.0; 3]))?;
    probe.set_path("radius", ConfigValue::Float(radius))?;
    // 0.26 contracts the outer sphere by 2% before its falloff; keep a
    // one-metre transition inside it (as transfer_probe does).
    probe.set_path("probe_side_width", ConfigValue::Float(1.0 + 0.02 * radius))?;
    probe.set_path("texture", ConfigValue::String(name.clone()))?;
    probe.set_path(
        "textures",
        ConfigValue::Named(vec![
            NamedValue { name: map.variant(), value: ConfigValue::String(name.clone()) },
            NamedValue { name: "default".into(), value: ConfigValue::String(name.clone()) },
        ]),
    )?;
    probe.set_path("diffuse_multiplier", ConfigValue::Float(1.0))?;
    probe.set_path("specular_multiplier", ConfigValue::Float(settings.specular))?;
    probe.set_path("enabled", ConfigValue::Boolean(true))?;
    probe.set_path("is_temporary", ConfigValue::Boolean(false))?;
    probe.set_path("name", ConfigValue::String("skylight_probe".into()))?;
    probe.set_path("full_name", ConfigValue::String("environment_and_lights/probes/skylight".into()))?;
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else {
        bail!("project objects are not an array");
    };
    objects.push(probe);
    objects.extend(boxes);
    Ok(format!(
        "{}: skylight probe (0.10b skylight in 0.26's probe contract): sphere at [{:.1}, {:.1}, {:.1}] radius {radius:.1} m over the render bounds; cube = source ambient cube x {}, specular {}, height-ambient colour from the {}; {box_count} interior box probes; faces {:?}",
        map.donor_project(source),
        centre[0],
        centre[1],
        centre[2],
        settings.gain,
        settings.specular,
        if settings.native_tint { "native template" } else { "source skylight" },
        cube.faces
    ))
}

fn emit_file(output: &Path, path: &str, bytes: &[u8], source: &str, files: &mut Vec<BuiltFile>) -> Result<()> {
    super::emit(output, path, bytes, source, files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(size: u32, mips: u32) -> Vec<u8> {
        let mut h = vec![0u8; 129];
        h[..4].copy_from_slice(b"DDS ");
        h[12..16].copy_from_slice(&size.to_le_bytes());
        h[16..20].copy_from_slice(&size.to_le_bytes());
        h[28..32].copy_from_slice(&mips.to_le_bytes());
        h[84..88].copy_from_slice(b"DXT5");
        h[112..116].copy_from_slice(&0xfe00u32.to_le_bytes());
        h
    }

    fn decode(block: &[u8], texel: usize) -> [f32; 4] {
        let c0 = u16::from_le_bytes([block[8], block[9]]);
        let c1 = u16::from_le_bytes([block[10], block[11]]);
        let (p0, p1) = (from565(c0), from565(c1));
        let i = (u32::from_le_bytes(block[12..16].try_into().unwrap()) >> (2 * texel)) & 3;
        let rgb = match i {
            0 => p0,
            1 => p1,
            2 => [0, 1, 2].map(|c| (2.0 * p0[c] + p1[c]) / 3.0),
            _ => [0, 1, 2].map(|c| (p0[c] + 2.0 * p1[c]) / 3.0),
        };
        [rgb[0], rgb[1], rgb[2], block[0] as f32 / 255.0]
    }

    const SOURCE: AmbientCube = AmbientCube {
        faces: [
            [0.51, 0.53, 0.565],
            [0.51, 0.53, 0.565],
            [1.0; 3],
            [1.0; 3],
            [0.51, 0.53, 0.565],
            [0.51, 0.53, 0.565],
        ],
    };

    #[test]
    fn ambient_cube_matches_the_skylight_shader_weights() {
        assert_eq!(SOURCE.eval([0.0, 1.0, 0.0]), [1.0; 3]);
        assert_eq!(SOURCE.eval([-1.0, 0.0, 0.0]), [0.51, 0.53, 0.565]);
        let d = std::f32::consts::FRAC_1_SQRT_2;
        let mixed = SOURCE.eval([d, -d, 0.0]);
        assert!((mixed[0] - 0.5 * (0.51 + 1.0)).abs() < 1e-5);
    }

    #[test]
    fn encoded_cube_decodes_to_the_gained_source_and_is_finite() {
        let dds = cube_dds(&header(256, 7), &SOURCE, 2.5, 16).unwrap();
        // 16, 8, 4: 16 + 4 + 1 blocks per face.
        assert_eq!(dds.len(), 128 + 6 * 21 * 16 + 1);
        assert_eq!(u32::from_le_bytes(dds[28..32].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(dds[12..16].try_into().unwrap()), 16);
        assert_eq!(u32::from_le_bytes(dds[20..24].try_into().unwrap()) as usize, dds.len() - 128);
        // +y face, centre block of mip 0: straight up = 1.0 x 2.5.
        let face = 2;
        let block_index = face * 21 + 5;
        let block = &dds[128 + block_index * 16..128 + block_index * 16 + 16];
        let t = decode(block, 5);
        assert!(t[3] > 0.0);
        for c in 0..3 {
            let decoded = (t[c] / t[3]).powf(2.2);
            assert!(decoded.is_finite());
            assert!((decoded - 2.5).abs() < 0.25, "decoded {decoded}");
        }
        // Every texel of every face decodes to a finite value inside the
        // source range (a zero alpha would divide by zero in the probe shader).
        for b in dds[128..dds.len() - 1].chunks_exact(16) {
            assert!(b[0] > 0 && b[0] == b[1]);
            for texel in 0..16 {
                let t = decode(b, texel);
                for c in 0..3 {
                    let v = (t[c] / t[3]).powf(2.2);
                    assert!(v.is_finite() && v > 1.0 && v < 2.8, "texel decodes to {v}");
                }
            }
        }
    }

    fn named(fields: &[(&str, ConfigValue)]) -> ConfigValue {
        ConfigValue::Named(
            fields.iter().map(|(n, v)| NamedValue { name: (*n).into(), value: v.clone() }).collect(),
        )
    }

    fn indoor_template() -> ConfigValue {
        named(&[
            ("game_object_type", ConfigValue::String("environment_probe".into())),
            ("full_name", ConfigValue::String("environment_and_lights/probes/indoor".into())),
            ("geometry", ConfigValue::Integer(1)),
            ("position", ConfigValue::Float3([38.149, 5.688, 4.438])),
            ("rotation", ConfigValue::Float3([0.0; 3])),
            ("scale", ConfigValue::Float3([13.16, 6.318, 21.711])),
            ("radius", ConfigValue::Integer(5)),
            ("probe_side_width", ConfigValue::Float(0.5)),
            ("diffuse_multiplier", ConfigValue::Integer(1)),
            ("specular_multiplier", ConfigValue::Float(0.6)),
            ("ambient_multiplier", ConfigValue::Float(0.1)),
            ("texture", ConfigValue::String("environment_probes/level_03/x".into())),
            ("face_average_color_0", ConfigValue::Float4([0.0; 4])),
        ])
    }

    fn hall() -> InteriorBox {
        InteriorBox {
            name: "pool_hall".into(),
            centre: [-88.0, 3.0, -45.0],
            half: [14.0, 6.0, 9.0],
            yaw: 0.3,
            diffuse: 0.5,
            reason: "test".into(),
        }
    }

    #[test]
    fn interior_box_probe_is_a_native_box_with_half_extents() {
        let probe = box_probe(&indoor_template(), &hall(), "environment_probes/srp/x_skylight", "day_4", 0.3, 0.2).unwrap();
        assert_eq!(probe.named("geometry"), Some(&ConfigValue::Integer(1)));
        assert_eq!(probe.named("position"), Some(&ConfigValue::Float3([-88.0, 3.0, -45.0])));
        assert_eq!(probe.named("rotation"), Some(&ConfigValue::Float3([0.0, 0.3, 0.0])));
        assert_eq!(probe.named("scale"), Some(&ConfigValue::Float3([14.0, 6.0, 9.0])));
        assert_eq!(probe.named("radius"), Some(&ConfigValue::Float(10.5)));
        assert_eq!(probe.named("probe_side_width"), Some(&ConfigValue::Float(INTERIOR_SIDE_WIDTH)));
        assert_eq!(probe.named("diffuse_multiplier"), Some(&ConfigValue::Float(0.5)));
        assert_eq!(probe.named("specular_multiplier"), Some(&ConfigValue::Float(0.15)));
        assert_eq!(probe.named("ambient_multiplier"), Some(&ConfigValue::Float(0.1)));
        assert_eq!(string(&probe, "texture"), Some("environment_probes/srp/x_skylight"));
        assert_eq!(
            probe.named("textures").and_then(|t| t.named("day_4")),
            Some(&ConfigValue::String("environment_probes/srp/x_skylight".into()))
        );
        assert_eq!(string(&probe, "full_name"), Some("environment_and_lights/probes/interior_pool_hall"));
        // Untouched template fields stay (schema), e.g. the face colours.
        assert!(probe.named("face_average_color_0").is_some());
        let mut walk = Vec::new();
        fn finite(v: &ConfigValue, out: &mut Vec<f32>) {
            match v {
                ConfigValue::Float(f) => out.push(*f),
                ConfigValue::Float3(f) => out.extend(f),
                ConfigValue::Float4(f) => out.extend(f),
                ConfigValue::Named(fs) => fs.iter().for_each(|f| finite(&f.value, out)),
                ConfigValue::Indexed(vs) => vs.iter().for_each(|v| finite(v, out)),
                _ => {}
            }
        }
        finite(&probe, &mut walk);
        assert!(walk.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn interior_boxes_refuse_bad_sizes_and_values() {
        let t = indoor_template();
        for bad in [
            InteriorBox { half: [1.0, 6.0, 9.0], ..hall() },
            InteriorBox { half: [14.0, 6.0, 250.0], ..hall() },
            InteriorBox { centre: [f32::NAN, 0.0, 0.0], ..hall() },
            InteriorBox { diffuse: 0.0, ..hall() },
            InteriorBox { diffuse: f32::INFINITY, ..hall() },
            InteriorBox { yaw: f32::NAN, ..hall() },
            InteriorBox { name: "bad name".into(), ..hall() },
        ] {
            assert!(box_probe(&t, &bad, "t", "day_4", 0.3, 0.2).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rejects_templates_and_inputs_that_are_not_cubes() {
        let mut flat = header(256, 7);
        flat[112..116].copy_from_slice(&0u32.to_le_bytes());
        assert!(cube_dds(&flat, &SOURCE, 2.5, 16).is_err());
        assert!(cube_dds(&header(256, 7), &SOURCE, 0.0, 16).is_err());
        assert!(cube_dds(&header(256, 7), &SOURCE, 2.5, 12).is_err());
        let black = AmbientCube { faces: [[0.0; 3]; 6] };
        assert!(cube_dds(&header(256, 7), &black, 2.5, 16).is_err());
    }
}
