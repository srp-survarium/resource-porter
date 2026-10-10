// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::to_writer_pretty;

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::manifest::{
    ArmorBuildRecord, ArmorPortManifest, BuiltFile, SkinnedModelSpec, SurfaceSelection,
    TargetModelSpec, TextureCompression,
};
use crate::maya_binary::{self, MayaSkin};
use crate::weapon::{
    add_dictionary_item, add_localization_strings, apply_patches, parse_config,
    prepare_empty_output, record_file, reject_duplicate_outputs, replace_strings,
    write_generated_config,
};

#[derive(Clone, Debug)]
struct BindBone {
    name: String,
    frame: Matrix4,
}

#[derive(Clone, Copy, Debug)]
struct BoneRetarget {
    target_index: u8,
    source_to_target: Matrix4,
}

#[derive(Clone, Debug)]
struct WeightTransferVertex {
    position: [f32; 3],
    normal: [f32; 3],
    weights: [(u8, f64); 4],
}

#[derive(Clone, Debug)]
struct WeightTransferReference {
    vertices: Vec<WeightTransferVertex>,
}

enum GeometrySource<'a> {
    Skinned {
        source_bones: &'a [BindBone],
        bone_remap: &'a [BoneRetarget],
    },
    MayaStatic {
        skin: &'a MayaSkin,
        bone_remap: &'a [BoneRetarget],
        weight_remap: &'a [Vec<(u8, f64)>],
        correspondence_remap: &'a [BoneRetarget],
        weight_transfer: Option<&'a WeightTransferReference>,
        transfer_influences: &'a [bool],
        transfer_threshold: f64,
        trim_influences: &'a [bool],
        trim_threshold: f64,
    },
}

#[derive(Clone, Copy, Debug)]
struct Matrix4([[f32; 4]; 4]);

impl Matrix4 {
    fn identity() -> Self {
        Self([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    fn from_frame(translation: [f32; 3], rotation: [f32; 3], scale: [f32; 3]) -> Self {
        let [x, y, z] = rotation;
        let (xs, xc) = x.sin_cos();
        let (ys, yc) = y.sin_cos();
        let (zs, zc) = z.sin_cos();
        let rotation = Self([
            [yc * zc, -yc * zs, ys, 0.0],
            [
                ys * xs * zc + xc * zs,
                -xs * ys * zs + xc * zc,
                -xs * yc,
                0.0,
            ],
            [
                -xc * ys * zc + xs * zs,
                xc * ys * zs + xs * zc,
                xc * yc,
                0.0,
            ],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        let scale = Self([
            [scale[0], 0.0, 0.0, 0.0],
            [0.0, scale[1], 0.0, 0.0],
            [0.0, 0.0, scale[2], 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        let translation = Self([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [translation[0], translation[1], translation[2], 1.0],
        ]);
        scale.multiply(rotation).multiply(translation)
    }

    fn multiply(self, right: Self) -> Self {
        let mut result = [[0.0; 4]; 4];
        for (row, result_row) in result.iter_mut().enumerate() {
            for (column, result_value) in result_row.iter_mut().enumerate() {
                *result_value = (0..4)
                    .map(|inner| self.0[row][inner] * right.0[inner][column])
                    .sum();
            }
        }
        Self(result)
    }

    fn inverse_affine(self) -> Result<Self> {
        let m = self.0;
        let determinant = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if determinant.abs() <= 1.0e-7 {
            bail!("bind-pose frame has a singular transform");
        }
        let reciprocal = determinant.recip();
        let inverse_basis = [
            [
                (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * reciprocal,
                (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * reciprocal,
                (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * reciprocal,
            ],
            [
                (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * reciprocal,
                (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * reciprocal,
                (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * reciprocal,
            ],
            [
                (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * reciprocal,
                (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * reciprocal,
                (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * reciprocal,
            ],
        ];
        let translation = [m[3][0], m[3][1], m[3][2]];
        let inverse_translation = [
            -(translation[0] * inverse_basis[0][0]
                + translation[1] * inverse_basis[1][0]
                + translation[2] * inverse_basis[2][0]),
            -(translation[0] * inverse_basis[0][1]
                + translation[1] * inverse_basis[1][1]
                + translation[2] * inverse_basis[2][1]),
            -(translation[0] * inverse_basis[0][2]
                + translation[1] * inverse_basis[1][2]
                + translation[2] * inverse_basis[2][2]),
        ];
        Ok(Self([
            [
                inverse_basis[0][0],
                inverse_basis[0][1],
                inverse_basis[0][2],
                0.0,
            ],
            [
                inverse_basis[1][0],
                inverse_basis[1][1],
                inverse_basis[1][2],
                0.0,
            ],
            [
                inverse_basis[2][0],
                inverse_basis[2][1],
                inverse_basis[2][2],
                0.0,
            ],
            [
                inverse_translation[0],
                inverse_translation[1],
                inverse_translation[2],
                1.0,
            ],
        ]))
    }

    fn transform_position(self, value: [f32; 3]) -> [f32; 3] {
        [
            value[0] * self.0[0][0]
                + value[1] * self.0[1][0]
                + value[2] * self.0[2][0]
                + self.0[3][0],
            value[0] * self.0[0][1]
                + value[1] * self.0[1][1]
                + value[2] * self.0[2][1]
                + self.0[3][1],
            value[0] * self.0[0][2]
                + value[1] * self.0[1][2]
                + value[2] * self.0[2][2]
                + self.0[3][2],
        ]
    }

    fn transform_direction(self, value: [f32; 3]) -> [f32; 3] {
        [
            value[0] * self.0[0][0] + value[1] * self.0[1][0] + value[2] * self.0[2][0],
            value[0] * self.0[0][1] + value[1] * self.0[1][1] + value[2] * self.0[2][1],
            value[0] * self.0[0][2] + value[1] * self.0[1][2] + value[2] * self.0[2][2],
        ]
    }

    fn mirrored_x(self) -> Self {
        let mirror = Self([
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        mirror.multiply(self).multiply(mirror)
    }

    fn donor_to_target_coordinates() -> Self {
        Self([
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    fn from_f64(value: [[f64; 4]; 4]) -> Result<Self> {
        let mut result = [[0.0_f32; 4]; 4];
        for row in 0..4 {
            for column in 0..4 {
                let converted = value[row][column] as f32;
                if !converted.is_finite() {
                    bail!("Maya bind matrix value cannot be represented as f32");
                }
                result[row][column] = converted;
            }
        }
        Ok(Self(result))
    }

    fn maya_to_model_coordinates(rotate_y_180: bool) -> Self {
        let signs = if rotate_y_180 {
            [-0.01, 0.01, 0.01]
        } else {
            [0.01, 0.01, -0.01]
        };
        Self([
            [signs[0], 0.0, 0.0, 0.0],
            [0.0, signs[1], 0.0, 0.0],
            [0.0, 0.0, signs[2], 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    fn to_frame(self) -> Result<([f32; 3], [f32; 3], [f32; 3])> {
        let translation = [self.0[3][0], self.0[3][1], self.0[3][2]];
        let mut rotation = [[0.0_f32; 3]; 3];
        let mut scale = [0.0_f32; 3];
        for (row, output) in rotation.iter_mut().enumerate() {
            scale[row] = self.0[row][..3]
                .iter()
                .map(|value| value * value)
                .sum::<f32>()
                .sqrt();
            if !scale[row].is_finite() || scale[row] <= f32::EPSILON {
                bail!("bind frame has an invalid scale");
            }
            for (column, value) in output.iter_mut().enumerate() {
                *value = self.0[row][column] / scale[row];
            }
        }
        let y = rotation[0][2].clamp(-1.0, 1.0).asin();
        let cosine_y = y.cos();
        let (x, z) = if cosine_y.abs() > 1.0e-5 {
            (
                (-rotation[1][2]).atan2(rotation[2][2]),
                (-rotation[0][1]).atan2(rotation[0][0]),
            )
        } else {
            ((rotation[1][0] * rotation[0][2]).atan2(rotation[1][1]), 0.0)
        };
        if !translation
            .into_iter()
            .chain([x, y, z])
            .chain(scale)
            .all(f32::is_finite)
        {
            bail!("bind frame decomposition produced non-finite values");
        }
        Ok((translation, [x, y, z], scale))
    }
}

#[derive(Serialize)]
struct SrpArmorContentManifest {
    format_version: u32,
    extra_armor_dict_ids: Vec<u16>,
    primary_armor_dict_id: u16,
}

pub fn build_bundle(
    manifest: &ArmorPortManifest,
    target_db: &Path,
    donor_model_archive: &Path,
    donor_maya_scene: Option<&Path>,
    donor_texture_dir: Option<&Path>,
    output: &Path,
) -> Result<ArmorBuildRecord> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    let mut target = ResourceDb::open(target_db)?;
    let mut model = ResourceDb::open(donor_model_archive)?;
    let mut files = Vec::new();

    if !manifest.armors.is_empty() {
        let mut dictionary =
            parse_config(&mut target, &manifest.dictionary.path, "target dictionary")?;
        for armor_spec in &manifest.armors {
            let mut armor = parse_config(
                &mut target,
                &armor_spec.template_config,
                "target armor template",
            )?;
            apply_patches(&mut armor.root, &armor_spec.patch)?;
            replace_strings(&mut armor.root, &armor_spec.replace_strings);
            validate_armor_visual_reference(manifest, armor_spec, &armor.root)?;
            write_generated_config(
                &overlay,
                &armor_spec.target_config,
                &armor,
                "generated:armor-config",
                &mut files,
            )?;
            add_dictionary_item(
                &mut dictionary.root,
                armor_spec.dict_id,
                &armor_spec.target_config,
                armor_spec.item_category,
                false,
            )?;
        }
        write_generated_config(
            &overlay,
            &manifest.dictionary.path,
            &dictionary,
            "generated:dictionary",
            &mut files,
        )?;

        let mut localization = parse_config(
            &mut target,
            &manifest.localization.path,
            "target localization",
        )?;
        add_localization_strings(&mut localization.root, &manifest.localization.strings)?;
        write_generated_config(
            &overlay,
            &manifest.localization.path,
            &localization,
            "generated:localization",
            &mut files,
        )?;
    }

    for generated in &manifest.generated_configs {
        let mut config = parse_config(
            &mut target,
            &generated.template_config,
            "target generated-config template",
        )?;
        apply_patches(&mut config.root, &generated.patch)?;
        replace_strings(&mut config.root, &generated.replace_strings);
        write_generated_config(
            &overlay,
            &generated.target_config,
            &config,
            "generated:character-config",
            &mut files,
        )?;
    }

    build_materials_and_textures(
        manifest,
        &mut target,
        donor_texture_dir,
        &overlay,
        &mut files,
    )?;
    convert_skinned_model(
        manifest,
        &mut target,
        &mut model,
        donor_maya_scene,
        &overlay,
        &mut files,
    )?;

    files.sort_by(|a, b| a.path.cmp(&b.path));
    reject_duplicate_outputs(&files)?;

    let record = ArmorBuildRecord {
        format_version: 1,
        asset_id: manifest.asset.id.clone(),
        donor_build: manifest.asset.donor_build.clone(),
        target_build: manifest.asset.target_build.clone(),
        armor_dict_ids: manifest.armors.iter().map(|armor| armor.dict_id).collect(),
        files,
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;

    let armor_ids: Vec<u16> = manifest
        .armors
        .iter()
        .map(|armor| {
            u16::try_from(armor.dict_id)
                .context("armor dictionary ID does not fit the SRP protocol")
        })
        .collect::<Result<_>>()?;
    if let Some(&primary_armor_dict_id) = armor_ids.first() {
        let server_dir = output.join("server");
        fs::create_dir_all(&server_dir)?;
        to_writer_pretty(
            fs::File::create(server_dir.join("srp-content.json"))?,
            &SrpArmorContentManifest {
                format_version: 1,
                extra_armor_dict_ids: armor_ids.clone(),
                primary_armor_dict_id,
            },
        )?;
    }
    Ok(record)
}

fn build_materials_and_textures(
    manifest: &ArmorPortManifest,
    target: &mut ResourceDb,
    donor_texture_dir: Option<&Path>,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    for material_spec in &manifest.materials {
        let mut material =
            parse_config(target, &material_spec.template, "target material template")?;
        replace_strings(&mut material.root, &material_spec.replace_strings);
        write_generated_config(
            overlay,
            &material_spec.target,
            &material,
            "generated:target-schema-material",
            files,
        )?;
    }

    if manifest.textures.is_empty() {
        return Ok(());
    }
    let donor_texture_dir = donor_texture_dir
        .context("armor manifest declares textures but --donor-texture-dir is missing")?;
    for texture in &manifest.textures {
        let source = safe_join(donor_texture_dir, &texture.source_file)?;
        if !source.is_file() {
            bail!("donor texture does not exist: {}", source.display());
        }
        let dds_path = format!("{}.dds", texture.target);
        let output = safe_join(overlay, &dds_path)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        convert_texture(&source, &output, texture.compression)
            .with_context(|| format!("converting donor texture {}", source.display()))?;
        let dds = fs::read(&output)?;
        validate_dds(&dds, texture.compression, &dds_path)?;
        files.push(record_file(
            &dds_path,
            &format!("converted:STK2:{}", texture.source_file),
            &dds,
        ));

        let options_path = format!("{}.options", texture.target);
        let options = target.read(&texture.template_options)?;
        write_raw_resource(
            overlay,
            &options_path,
            &options,
            &format!("target-template:{}", texture.template_options),
            files,
        )?;
    }
    Ok(())
}

pub(crate) fn convert_texture(
    source: &Path,
    output: &Path,
    compression: TextureCompression,
) -> Result<()> {
    let mut command = Command::new("magick");
    let mut normal_intermediate = None;
    match compression {
        TextureCompression::Dxt1 => {
            command
                .arg("-define")
                .arg("dds:compression=dxt1")
                .arg(source);
        }
        TextureCompression::Dxt5Normal => {
            let intermediate = output.with_extension("dxt5nm.tga");
            write_dxt5_normal_tga(source, &intermediate)?;
            command
                .arg("-define")
                .arg("dds:compression=dxt5")
                .arg(&intermediate);
            normal_intermediate = Some(intermediate);
        }
    }
    let result = command.arg(output).output().with_context(
        || "starting ImageMagick; install `magick` or enter the repository Nix shell",
    )?;
    if let Some(normal_intermediate) = normal_intermediate {
        fs::remove_file(normal_intermediate)?;
    }
    if !result.status.success() {
        bail!(
            "ImageMagick failed with {}: {}",
            result.status,
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    Ok(())
}

fn write_dxt5_normal_tga(source: &Path, output: &Path) -> Result<()> {
    let bytes = fs::read(source)?;
    if bytes.len() < 18 {
        bail!("TGA header is truncated");
    }
    let id_length = usize::from(bytes[0]);
    if bytes[1] != 0 || bytes[2] != 2 {
        bail!("only uncompressed true-color TGA sources are supported");
    }
    let width = usize::from(u16::from_le_bytes([bytes[12], bytes[13]]));
    let height = usize::from(u16::from_le_bytes([bytes[14], bytes[15]]));
    let source_channels = match bytes[16] {
        24 => 3,
        32 => 4,
        depth => bail!("unsupported TGA pixel depth {depth}"),
    };
    let pixel_count = width
        .checked_mul(height)
        .context("TGA dimensions overflow")?;
    let pixel_offset = 18_usize
        .checked_add(id_length)
        .context("TGA pixel offset overflow")?;
    let pixel_bytes = pixel_count
        .checked_mul(source_channels)
        .context("TGA payload size overflow")?;
    let pixels = bytes
        .get(pixel_offset..pixel_offset + pixel_bytes)
        .context("TGA pixel payload is truncated")?;

    let mut converted = Vec::with_capacity(18 + pixel_count * 4);
    converted.extend_from_slice(&[
        0,
        0,
        2,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
        32,
        (bytes[17] & 0x30) | 8,
    ]);
    for pixel in pixels.chunks_exact(source_channels) {
        // TGA stores BGRA. DXT5nm stores X in alpha and Y in green; the
        // v0.100b shader ignores the red/blue channels.
        converted.extend_from_slice(&[0, pixel[1], 0, pixel[2]]);
    }
    fs::write(output, converted)?;
    Ok(())
}

pub(crate) fn validate_dds(
    bytes: &[u8],
    compression: TextureCompression,
    label: &str,
) -> Result<()> {
    if bytes.len() < 128 || &bytes[..4] != b"DDS " || read_u32(bytes, 4)? != 124 {
        bail!("{label}: invalid DDS header");
    }
    let width = read_u32(bytes, 16)?;
    let height = read_u32(bytes, 12)?;
    let mip_count = read_u32(bytes, 28)?;
    if width == 0 || height == 0 || mip_count < 2 {
        bail!("{label}: invalid dimensions/mipmap count {width}x{height}, mips={mip_count}");
    }
    let expected = match compression {
        TextureCompression::Dxt1 => b"DXT1",
        TextureCompression::Dxt5Normal => b"DXT5",
    };
    if bytes.get(84..88) != Some(expected) {
        bail!(
            "{label}: expected {} FourCC",
            String::from_utf8_lossy(expected)
        );
    }
    Ok(())
}

fn validate_armor_visual_reference(
    manifest: &ArmorPortManifest,
    armor: &crate::manifest::ArmorSpec,
    config: &ConfigValue,
) -> Result<()> {
    let part_name = config_string(config, "data.part_name")
        .with_context(|| format!("armor {} has no usable part name", armor.dict_id))?;
    if part_name.is_empty() {
        bail!("armor {} has an empty data.part_name", armor.dict_id);
    }

    let base_model = config_string(config, "data.base_model")
        .with_context(|| format!("armor {} has no usable base model", armor.dict_id))?;
    if base_model.is_empty() {
        bail!("armor {} has an empty data.base_model", armor.dict_id);
    }

    let target_root = format!("models/{base_model}.skinned_model");
    let model = manifest
        .model
        .target_models
        .get(&target_root)
        .with_context(|| {
            format!(
                "armor {} data.base_model {base_model:?} does not name a generated target model",
                armor.dict_id
            )
        })?;
    if !model.surfaces.contains_key(part_name) {
        bail!(
            "armor {} data.part_name {part_name:?} is not a surface of {target_root}",
            armor.dict_id
        );
    }
    Ok(())
}

fn config_string<'a>(root: &'a ConfigValue, path: &str) -> Result<&'a str> {
    let mut value = root;
    for component in path.split('.') {
        value = value
            .named(component)
            .with_context(|| format!("config path does not exist: {path}"))?;
    }
    let ConfigValue::String(value) = value else {
        bail!("config path is not a string: {path}");
    };
    Ok(value)
}

fn convert_skinned_model(
    manifest: &ArmorPortManifest,
    target: &mut ResourceDb,
    model: &mut ResourceDb,
    donor_maya_scene: Option<&Path>,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let spec = &manifest.model;
    let target_bind_pose_template = target.read(&spec.bind_pose_template)?;
    let target_bones = parse_bind_pose(&target_bind_pose_template, "target bind-pose template")?;

    let source_surfaces: BTreeSet<String> = model
        .paths_with_prefix("render/")
        .filter_map(|path| {
            path.strip_prefix("render/")
                .and_then(|path| path.strip_suffix("/vertices"))
                .map(ToOwned::to_owned)
        })
        .collect();
    let declared_surfaces: BTreeSet<String> = spec.surfaces.keys().cloned().collect();
    if source_surfaces != declared_surfaces {
        bail!(
            "model surface declaration differs from archive: archive={source_surfaces:?}, manifest={declared_surfaces:?}"
        );
    }

    if let Some(scene) = donor_maya_scene {
        let skin = maya_binary::read_skin(scene)?;
        let model_bones = make_maya_model_bind_pose(
            &skin,
            &target_bones,
            &spec.maya_bind_pose_map,
            spec.rotate_y_180,
        )?;
        let model_bind_pose = if spec.maya_bind_pose_map.is_empty() {
            target_bind_pose_template.clone()
        } else {
            encode_bind_pose(&model_bones)?
        };
        let bone_remap = make_maya_bone_remap(
            &skin,
            &model_bones,
            &spec.bone_map,
            spec.rotate_y_180,
            spec.retarget_maya_bind_pose,
        )?;
        let weight_remap =
            make_maya_weight_remap(&skin, &model_bones, &bone_remap, &spec.bone_weight_blends)?;
        let correspondence_remap = if spec.weight_transfer_template.is_some() {
            make_maya_bone_remap(
                &skin,
                &target_bones,
                &spec.bone_map,
                spec.rotate_y_180,
                true,
            )?
        } else {
            Vec::new()
        };
        let weight_transfer = spec
            .weight_transfer_template
            .as_deref()
            .map(|path| {
                target
                    .read(path)
                    .with_context(|| format!("reading weight-transfer template {path}"))
                    .and_then(|bytes| parse_weight_transfer_reference(&bytes))
            })
            .transpose()?;
        let transfer_influences: Vec<bool> = skin
            .influence_names
            .iter()
            .map(|name| spec.weight_transfer_influences.contains(name))
            .collect();
        let trim_influences: Vec<bool> = skin
            .influence_names
            .iter()
            .map(|name| spec.trim_maya_influences.contains(name))
            .collect();
        let source = GeometrySource::MayaStatic {
            skin: &skin,
            bone_remap: &bone_remap,
            weight_remap: &weight_remap,
            correspondence_remap: &correspondence_remap,
            weight_transfer: weight_transfer.as_ref(),
            transfer_influences: &transfer_influences,
            transfer_threshold: spec.weight_transfer_threshold,
            trim_influences: &trim_influences,
            trim_threshold: spec.trim_maya_weight_threshold,
        };
        for (target_root, target_model) in &spec.target_models {
            write_target_model(
                manifest,
                spec,
                target_model,
                target_root.trim_end_matches('/'),
                target,
                model,
                &model_bind_pose,
                &source,
                overlay,
                files,
            )?;
        }
    } else {
        let source_bind_pose = model.read("render/bind_pose")?;
        let source_bones = parse_bind_pose(&source_bind_pose, "donor bind pose")?;
        let bone_remap = make_bone_remap(&source_bones, &target_bones, &spec.bone_map)?;
        let source = GeometrySource::Skinned {
            source_bones: &source_bones,
            bone_remap: &bone_remap,
        };
        for (target_root, target_model) in &spec.target_models {
            write_target_model(
                manifest,
                spec,
                target_model,
                target_root.trim_end_matches('/'),
                target,
                model,
                &target_bind_pose_template,
                &source,
                overlay,
                files,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_target_model(
    manifest: &ArmorPortManifest,
    spec: &SkinnedModelSpec,
    target_model: &TargetModelSpec,
    target_root: &str,
    target: &mut ResourceDb,
    model: &mut ResourceDb,
    target_bind_pose: &[u8],
    source: &GeometrySource<'_>,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    write_raw_resource(
        overlay,
        &format!("{target_root}/render/bind_pose"),
        target_bind_pose,
        "target-template:model-bind-pose",
        files,
    )?;

    let mut render_properties = parse_config(
        target,
        &spec.render_properties_template,
        "target render-properties template",
    )?;
    render_properties.root.set_path(
        "src_filename",
        ConfigValue::String(format!("STK2/{}", manifest.asset.id)),
    )?;
    write_generated_config(
        overlay,
        &format!("{target_root}/render/export_properties"),
        &render_properties,
        "generated:model-render-properties",
        files,
    )?;

    let mut skeleton = parse_config(
        target,
        &spec.skeleton_template,
        "target model-skeleton template",
    )?;
    skeleton.root.set_path(
        "skeleton",
        ConfigValue::String(spec.animation_skeleton.clone()),
    )?;
    write_generated_config(
        overlay,
        &format!("{target_root}/skeleton"),
        &skeleton,
        "generated:model-skeleton",
        files,
    )?;

    let settings_template = parse_config(
        target,
        &spec.settings_template,
        "target model-settings template",
    )?;
    let output_materials: BTreeMap<String, String> = target_model
        .surfaces
        .iter()
        .map(|(output_surface, output)| {
            (
                output_surface.clone(),
                spec.surfaces[&output.source].clone(),
            )
        })
        .collect();
    let settings = if spec.preserve_settings_template {
        make_settings_from_template(settings_template, &output_materials)?
    } else {
        make_settings(&output_materials)
    };
    write_generated_config(
        overlay,
        &format!("{target_root}/settings"),
        &settings,
        "generated:model-settings",
        files,
    )?;

    for (output_surface, output) in &target_model.surfaces {
        let material = &spec.surfaces[&output.source];
        let vertices = model.read(&format!("render/{}/vertices", output.source))?;
        let source_indices = model.read(&format!("render/{}/indices", output.source))?;
        let source_properties =
            model.read(&format!("render/{}/export_properties", output.source))?;
        let (indices, converted) = match source {
            GeometrySource::Skinned {
                source_bones,
                bone_remap,
            } => {
                let indices =
                    select_triangles(&vertices, &source_indices, source_bones, output.selection)
                        .with_context(|| {
                            format!(
                                "selecting {:?} geometry from surface {}",
                                output.selection, output.source
                            )
                        })?;
                validate_source_surface_properties(&source_properties, &output.source)?;
                let converted = convert_skinned_surface(&vertices, &indices, bone_remap)
                    .with_context(|| format!("converting model surface {output_surface}"))?;
                (indices, converted)
            }
            GeometrySource::MayaStatic {
                skin,
                bone_remap,
                weight_remap,
                correspondence_remap,
                weight_transfer,
                transfer_influences,
                transfer_threshold,
                trim_influences,
                trim_threshold,
            } => {
                validate_static_surface_properties(&source_properties, &output.source)?;
                let indices = match output.selection {
                    SurfaceSelection::All => source_indices,
                    SurfaceSelection::Hidden => hidden_triangle_indices(&vertices)?,
                    selection => {
                        bail!(
                            "Maya-skinned static surface {} cannot use {selection:?} triangle selection",
                            output.source
                        )
                    }
                };
                let (converted, indices) = convert_maya_static_surface(
                    &vertices,
                    &indices,
                    skin,
                    bone_remap,
                    weight_remap,
                    spec.rotate_y_180,
                    correspondence_remap,
                    *weight_transfer,
                    transfer_influences,
                    *transfer_threshold,
                    trim_influences,
                    *trim_threshold,
                    &output.include_maya_influences,
                    output.include_maya_weight_threshold,
                    &output.trim_triangles,
                )
                .with_context(|| {
                    format!("skinning static model surface {output_surface} from Maya")
                })?;
                (indices, converted)
            }
        };
        write_raw_resource(
            overlay,
            &format!("{target_root}/render/{output_surface}/converted_model"),
            &converted,
            "converted:model-geometry",
            files,
        )?;

        let vertex_count = read_u32(&vertices, 0)?;
        let index_count = read_u32(&indices, 0)?;
        let mut properties = parse_config(
            target,
            &spec.surface_properties_template,
            "target surface-properties template",
        )?;
        properties.root.set_path(
            "source.indices",
            ConfigValue::Integer(i32::try_from(index_count)?),
        )?;
        properties.root.set_path(
            "source.primitives",
            ConfigValue::Integer(i32::try_from(index_count / 3)?),
        )?;
        properties.root.set_path(
            "source.mesh_name",
            ConfigValue::String(format!("|{}|{}", manifest.asset.id, output_surface)),
        )?;
        properties.root.set_path(
            "source.texture_name",
            ConfigValue::String(material.trim_start_matches("characters/").to_owned()),
        )?;
        properties.root.set_path(
            "source.vertices",
            ConfigValue::Integer(i32::try_from(vertex_count)?),
        )?;
        properties.root.set_path("type", ConfigValue::Integer(47))?;
        write_generated_config(
            overlay,
            &format!("{target_root}/render/{output_surface}/export_properties"),
            &properties,
            "generated:model-surface-properties",
            files,
        )?;
    }
    Ok(())
}

fn make_maya_bone_remap(
    skin: &MayaSkin,
    target_bones: &[BindBone],
    bone_map: &BTreeMap<String, String>,
    rotate_y_180: bool,
    retarget_bind_pose: bool,
) -> Result<Vec<BoneRetarget>> {
    let target_indices: BTreeMap<&str, usize> = target_bones
        .iter()
        .enumerate()
        .map(|(index, bone)| (bone.name.as_str(), index))
        .collect();
    skin.influence_names
        .iter()
        .enumerate()
        .map(|(source_index, source)| {
            let target = bone_map
                .get(source)
                .with_context(|| format!("no target mapping for Maya influence {source}"))?;
            let index = *target_indices.get(target.as_str()).with_context(|| {
                format!("mapped target bone {target} for Maya influence {source} does not exist")
            })?;
            let target_index = u8::try_from(index).with_context(|| {
                format!("target bone index {index} for {target} does not fit in u8")
            })?;
            let source_to_target = if retarget_bind_pose && requires_maya_pose_retarget(source) {
                let source_bind = maya_bind_frame(skin, source_index, rotate_y_180)?;
                source_bind
                    .inverse_affine()
                    .with_context(|| format!("inverting Maya bind frame {source}"))?
                    .multiply(target_bones[index].frame)
            } else {
                Matrix4::identity()
            };
            Ok(BoneRetarget {
                target_index,
                source_to_target,
            })
        })
        .collect()
}

fn make_maya_weight_remap(
    skin: &MayaSkin,
    target_bones: &[BindBone],
    bone_remap: &[BoneRetarget],
    blends: &BTreeMap<String, BTreeMap<String, f64>>,
) -> Result<Vec<Vec<(u8, f64)>>> {
    let target_indices: BTreeMap<&str, usize> = target_bones
        .iter()
        .enumerate()
        .map(|(index, bone)| (bone.name.as_str(), index))
        .collect();
    skin.influence_names
        .iter()
        .enumerate()
        .map(|(source_index, source)| {
            let Some(blend) = blends.get(source) else {
                return Ok(vec![(bone_remap[source_index].target_index, 1.0)]);
            };
            blend
                .iter()
                .map(|(target, weight)| {
                    let index = *target_indices.get(target.as_str()).with_context(|| {
                        format!("blended target bone {target} for Maya influence {source} does not exist")
                    })?;
                    Ok((u8::try_from(index)?, *weight))
                })
                .collect()
        })
        .collect()
}

fn maya_bind_frame(skin: &MayaSkin, source_index: usize, rotate_y_180: bool) -> Result<Matrix4> {
    let coordinates = Matrix4::maya_to_model_coordinates(rotate_y_180);
    let inverse_coordinates = coordinates.inverse_affine()?;
    let bind_pre = Matrix4::from_f64(skin.bind_pre_matrices[source_index])?;
    Ok(inverse_coordinates
        .multiply(bind_pre.inverse_affine()?)
        .multiply(coordinates))
}

fn make_maya_model_bind_pose(
    skin: &MayaSkin,
    target_bones: &[BindBone],
    bind_pose_map: &BTreeMap<String, String>,
    rotate_y_180: bool,
) -> Result<Vec<BindBone>> {
    let source_indices: BTreeMap<&str, usize> = skin
        .influence_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), index))
        .collect();
    let target_indices: BTreeMap<&str, usize> = target_bones
        .iter()
        .enumerate()
        .map(|(index, bone)| (bone.name.as_str(), index))
        .collect();
    let mut result = target_bones.to_vec();
    for (target, source) in bind_pose_map {
        let target_index = *target_indices
            .get(target.as_str())
            .with_context(|| format!("Maya bind-pose target bone {target} does not exist"))?;
        let source_index = *source_indices
            .get(source.as_str())
            .with_context(|| format!("Maya bind-pose source influence {source} does not exist"))?;
        result[target_index].frame = maya_bind_frame(skin, source_index, rotate_y_180)?;
    }
    Ok(result)
}

fn encode_bind_pose(bones: &[BindBone]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    output.extend_from_slice(&u16::try_from(bones.len())?.to_le_bytes());
    for bone in bones {
        if bone.name.as_bytes().contains(&0) {
            bail!("bind-pose bone name contains a NUL byte");
        }
        output.extend_from_slice(bone.name.as_bytes());
        output.push(0);
        let (translation, rotation, scale) = bone.frame.to_frame()?;
        for value in translation.into_iter().chain(rotation).chain(scale) {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    Ok(output)
}

fn requires_maya_pose_retarget(name: &str) -> bool {
    let Some(suffix) = name
        .strip_prefix("bn_r_")
        .or_else(|| name.strip_prefix("bn_l_"))
    else {
        return false;
    };
    matches!(
        suffix,
        "sholder"
            | "muscle_sholder"
            | "UpperArm"
            | "elbow"
            | "foreArm"
            | "twist_ForeArm"
            | "hand"
            | "twist_UpperArm"
            | "muscle_UpperArm"
    ) || suffix.starts_with("thumb_")
        || suffix.starts_with("index_")
        || suffix.starts_with("middle_")
        || suffix.starts_with("ringFinger_")
        || suffix.starts_with("pinky_")
}

fn parse_bind_pose(bytes: &[u8], label: &str) -> Result<Vec<BindBone>> {
    let count = usize::from(read_u16(bytes, 0)?);
    let mut offset = 2_usize;
    let mut bones = Vec::with_capacity(count);
    for index in 0..count {
        let rest = bytes
            .get(offset..)
            .with_context(|| format!("{label}: missing bone {index} name"))?;
        let name_length = rest
            .iter()
            .position(|byte| *byte == 0)
            .with_context(|| format!("{label}: unterminated bone {index} name"))?;
        let name = std::str::from_utf8(&rest[..name_length])
            .with_context(|| format!("{label}: bone {index} name is not UTF-8"))?;
        if name.is_empty() {
            bail!("{label}: bone {index} has an empty name");
        }
        offset = offset
            .checked_add(name_length + 1)
            .context("bind-pose byte offset overflow")?;
        let translation = read_float3(bytes, offset)?;
        let rotation = read_float3(bytes, offset + 12)?;
        let scale = read_float3(bytes, offset + 24)?;
        offset = offset
            .checked_add(36)
            .context("bind-pose byte offset overflow")?;
        if offset > bytes.len()
            || !translation.into_iter().all(f32::is_finite)
            || !rotation.into_iter().all(f32::is_finite)
            || !scale.into_iter().all(f32::is_finite)
        {
            bail!("{label}: bone {index} has a truncated frame");
        }
        bones.push(BindBone {
            name: name.to_owned(),
            frame: Matrix4::from_frame(translation, rotation, scale),
        });
    }
    if offset != bytes.len() {
        bail!(
            "{label}: {} trailing bytes after {count} bone frames",
            bytes.len() - offset
        );
    }
    Ok(bones)
}

fn make_bone_remap(
    source_bones: &[BindBone],
    target_bones: &[BindBone],
    bone_map: &BTreeMap<String, String>,
) -> Result<Vec<BoneRetarget>> {
    let target_indices: BTreeMap<&str, usize> = target_bones
        .iter()
        .enumerate()
        .map(|(index, bone)| (bone.name.as_str(), index))
        .collect();
    if target_indices.len() != target_bones.len() {
        bail!("target bind-pose template contains duplicate bone names");
    }

    let source_indices: BTreeMap<&str, usize> = source_bones
        .iter()
        .enumerate()
        .map(|(index, bone)| (bone.name.as_str(), index))
        .collect();
    let mut remap = Vec::with_capacity(source_bones.len());
    for source_bone in source_bones {
        let target_name = bone_map
            .get(&source_bone.name)
            .with_context(|| format!("no target mapping for donor bone {}", source_bone.name))?;
        let target_index = *target_indices.get(target_name.as_str()).with_context(|| {
            format!(
                "mapped target bone {target_name} for donor bone {} does not exist",
                source_bone.name
            )
        })?;
        let target_index_u8 = u8::try_from(target_index).with_context(|| {
            format!("target bone index {target_index} for {target_name} does not fit in u8")
        })?;
        let source_to_target = if requires_pose_retarget(&source_bone.name) {
            source_bone
                .frame
                .inverse_affine()
                .with_context(|| format!("inverting donor bind frame {}", source_bone.name))?
                .multiply(target_bones[target_index].frame)
        } else {
            // The trunk, head/helmet, and legs already fit the old character
            // after the coordinate-system conversion. Applying their unrelated
            // bind-frame Euler rotations warped the torso and turned the helmet
            // by 90 degrees.
            Matrix4::donor_to_target_coordinates()
        };
        remap.push(BoneRetarget {
            target_index: target_index_u8,
            source_to_target,
        });
    }
    // The STK2 archive's right-arm Euler frames are not geometrically
    // equivalent to its mirrored left-arm frames: transforming the next joint
    // misses the elbow/wrist by more than half a metre. The mesh and joint
    // positions are symmetric, as is the v0.100b target skeleton, so mirror the
    // verified left-side conversion for every paired right-side bone.
    for (source_index, source_bone) in source_bones.iter().enumerate() {
        let Some(suffix) = source_bone.name.strip_prefix("Right") else {
            continue;
        };
        let left_name = format!("Left{suffix}");
        let Some(&left_index) = source_indices.get(left_name.as_str()) else {
            continue;
        };
        let Some(target_name) = bone_map.get(&source_bone.name) else {
            continue;
        };
        let Some(left_target_name) = bone_map.get(&left_name) else {
            continue;
        };
        if target_name.strip_prefix("Right") != left_target_name.strip_prefix("Left") {
            continue;
        }
        remap[source_index].source_to_target = remap[left_index].source_to_target.mirrored_x();
    }
    Ok(remap)
}

fn requires_pose_retarget(name: &str) -> bool {
    let arm_bone = |side: &str| {
        matches!(
            name.strip_prefix(side),
            Some("Arm" | "SholderRoll" | "ForeArm" | "HandRoll" | "Hand")
        ) || name
            .strip_prefix(side)
            .is_some_and(|suffix| suffix.starts_with("Hand"))
    };
    arm_bone("Left") || arm_bone("Right")
}

fn make_settings(surfaces: &BTreeMap<String, String>) -> BinaryConfig {
    let surface_names = surfaces.keys().cloned().map(ConfigValue::String).collect();
    let material_settings = surfaces
        .iter()
        .map(|(surface, material)| NamedValue {
            name: surface.clone(),
            value: ConfigValue::Named(vec![
                named("material_name", ConfigValue::String(material.clone())),
                named("used_in_portals", ConfigValue::Boolean(false)),
            ]),
        })
        .collect();
    BinaryConfig {
        root: ConfigValue::Named(vec![
            named("hit_params", ConfigValue::String(String::new())),
            named(
                "lod_hierrarchy",
                ConfigValue::Named(vec![
                    named(
                        "LOD0",
                        ConfigValue::Named(vec![named(
                            "surfaces",
                            ConfigValue::Indexed(surface_names),
                        )]),
                    ),
                    named("LOD1", ConfigValue::Named(Vec::new())),
                    named("LOD2", ConfigValue::Named(Vec::new())),
                ]),
            ),
            named("material_settings", ConfigValue::Named(material_settings)),
            named("calculate_ao", ConfigValue::Boolean(false)),
        ]),
    }
}

fn make_settings_from_template(
    mut settings: BinaryConfig,
    surfaces: &BTreeMap<String, String>,
) -> Result<BinaryConfig> {
    let surface_names = surfaces.keys().cloned().map(ConfigValue::String).collect();
    let material_settings = surfaces
        .iter()
        .map(|(surface, material)| NamedValue {
            name: surface.clone(),
            value: ConfigValue::Named(vec![
                named("material_name", ConfigValue::String(material.clone())),
                named("used_in_portals", ConfigValue::Boolean(true)),
            ]),
        })
        .collect();
    settings.root.set_path(
        "lod_hierrarchy.LOD0.surfaces",
        ConfigValue::Indexed(surface_names),
    )?;
    settings
        .root
        .set_path("material_settings", ConfigValue::Named(material_settings))?;
    Ok(settings)
}

fn validate_source_surface_properties(bytes: &[u8], surface: &str) -> Result<()> {
    let text = std::str::from_utf8(bytes)
        .with_context(|| format!("surface properties are not UTF-8 text: {surface}"))?;
    if !text.lines().any(|line| line.trim() == "type = 47") {
        bail!("surface {surface} is not a four-weight skinned submesh");
    }
    Ok(())
}

fn validate_static_surface_properties(bytes: &[u8], surface: &str) -> Result<()> {
    let text = std::str::from_utf8(bytes)
        .with_context(|| format!("surface properties are not UTF-8 text: {surface}"))?;
    if !text.lines().any(|line| line.trim() == "type = 0") {
        bail!("surface {surface} is not a static submesh");
    }
    Ok(())
}

fn hidden_triangle_indices(vertices: &[u8]) -> Result<Vec<u8>> {
    if read_u32(vertices, 0)? == 0 {
        bail!("cannot hide a surface whose source has no vertices");
    }
    let mut indices = Vec::with_capacity(10);
    push_u32(&mut indices, 3);
    indices.extend_from_slice(&0_u16.to_le_bytes());
    indices.extend_from_slice(&0_u16.to_le_bytes());
    indices.extend_from_slice(&0_u16.to_le_bytes());
    Ok(indices)
}

fn select_triangles(
    vertices: &[u8],
    indices: &[u8],
    source_bones: &[BindBone],
    selection: SurfaceSelection,
) -> Result<Vec<u8>> {
    if selection == SurfaceSelection::All {
        return Ok(indices.to_vec());
    }

    let vertex_count = usize::try_from(read_u32(vertices, 0)?)?;
    let expected_vertices = 4_usize
        .checked_add(
            vertex_count
                .checked_mul(76)
                .context("vertex byte count overflow")?,
        )
        .context("vertex byte count overflow")?;
    if vertices.len() != expected_vertices {
        bail!(
            "four-weight vertex payload has size {}, expected {expected_vertices} for {vertex_count} vertices",
            vertices.len()
        );
    }

    let index_count = usize::try_from(read_u32(indices, 0)?)?;
    let expected_indices = 4_usize
        .checked_add(
            index_count
                .checked_mul(2)
                .context("index byte count overflow")?,
        )
        .context("index byte count overflow")?;
    if indices.len() != expected_indices {
        bail!(
            "index payload has size {}, expected {expected_indices} for {index_count} indices",
            indices.len()
        );
    }
    if index_count % 3 != 0 {
        bail!("index count {index_count} is not divisible by three");
    }

    let is_hands = matches!(
        selection,
        SurfaceSelection::Hands | SurfaceSelection::ExcludeHands
    );
    let exclude = matches!(
        selection,
        SurfaceSelection::ExcludeHands | SurfaceSelection::ExcludeFeet
    );
    let mut output_indices = Vec::new();
    for triangle in 0..index_count / 3 {
        let mut region_weight = 0.0_f32;
        let mut triangle_indices = [0_u16; 3];
        for corner in 0..3 {
            let offset = 4 + (triangle * 3 + corner) * 2;
            let vertex_index = read_u16(indices, offset)?;
            if usize::from(vertex_index) >= vertex_count {
                bail!(
                    "triangle {triangle} references vertex {vertex_index}, but the surface has only {vertex_count} vertices"
                );
            }
            triangle_indices[corner] = vertex_index;
            let corner_region_weight = vertex_region_weight(
                &vertices[4 + usize::from(vertex_index) * 76..][..76],
                source_bones,
                is_hands,
            )?;
            region_weight += corner_region_weight;
        }
        // Native v0.100b torso meshes retain some Hand weight around the wrist.
        // A majority-weight boundary reproduces that overlap without pulling
        // long forearm triangles into the separate glove model.
        let belongs_to_region = region_weight / 3.0 >= 0.5;
        if belongs_to_region != exclude {
            for index in triangle_indices {
                output_indices.extend_from_slice(&index.to_le_bytes());
            }
        }
    }
    if output_indices.is_empty() {
        bail!("selection {selection:?} produced no triangles");
    }

    let mut output = Vec::with_capacity(4 + output_indices.len());
    push_u32(&mut output, u32::try_from(output_indices.len() / 2)?);
    output.extend_from_slice(&output_indices);
    Ok(output)
}

fn vertex_region_weight(vertex: &[u8], source_bones: &[BindBone], hands: bool) -> Result<f32> {
    let mut weights = [
        read_f32(vertex, 56)?,
        read_f32(vertex, 60)?,
        read_f32(vertex, 64)?,
        0.0,
    ];
    weights[3] = 1.0 - weights[0] - weights[1] - weights[2];

    let mut result = 0.0_f32;
    for (slot, offset) in [0_usize, 2, 4, 6].into_iter().enumerate() {
        let bone_index = read_u16(vertex, offset)?;
        if bone_index == u16::MAX {
            continue;
        }
        let bone_name = &source_bones
            .get(usize::from(bone_index))
            .with_context(|| format!("vertex references missing donor bone {bone_index}"))?
            .name;
        let belongs = if hands {
            bone_name.starts_with("LeftHand") || bone_name.starts_with("RightHand")
        } else {
            matches!(
                bone_name.as_str(),
                "LeftFoot" | "LeftToe" | "RightFoot" | "RightToe"
            )
        };
        if belongs {
            result += weights[slot].max(0.0);
        }
    }
    Ok(result.clamp(0.0, 1.0))
}

fn convert_skinned_surface(
    vertices: &[u8],
    indices: &[u8],
    bone_remap: &[BoneRetarget],
) -> Result<Vec<u8>> {
    let count = usize::try_from(read_u32(vertices, 0)?)?;
    let expected = 4_usize
        .checked_add(
            count
                .checked_mul(76)
                .context("vertex byte count overflow")?,
        )
        .context("vertex byte count overflow")?;
    if vertices.len() != expected {
        bail!(
            "four-weight vertex payload has size {}, expected {expected} for {count} vertices",
            vertices.len()
        );
    }
    if indices.len() < 4 {
        bail!("index payload is shorter than its count");
    }

    let vertex_chunk_size = 4_usize
        .checked_add(count.checked_mul(48).context("vertex output overflow")?)
        .context("vertex output overflow")?;
    let mut output = Vec::with_capacity(16 + vertex_chunk_size + indices.len());
    push_u32(&mut output, 3);
    push_u32(&mut output, u32::try_from(vertex_chunk_size)?);
    push_u32(&mut output, u32::try_from(count)?);

    for index in 0..count {
        let start = 4 + index * 76;
        let vertex = &vertices[start..start + 76];

        let m0 = read_u16(vertex, 0)?;
        let first_bone = remap_bone(m0, bone_remap, index)?;
        let mut retargets = [first_bone; 4];
        let mut bones = [0_u8; 4];
        for (bone_index, offset) in [0_usize, 2, 4, 6].into_iter().enumerate() {
            let value = read_u16(vertex, offset)?;
            retargets[bone_index] = if value == u16::MAX {
                first_bone
            } else {
                remap_bone(value, bone_remap, index)?
            };
            bones[bone_index] = retargets[bone_index].target_index;
        }

        let mut weights = [
            read_f32(vertex, 56)?,
            read_f32(vertex, 60)?,
            read_f32(vertex, 64)?,
            0.0,
        ];
        weights[3] = 1.0 - weights[0] - weights[1] - weights[2];
        write_float3(
            &mut output,
            blend_position(read_float3(vertex, 8)?, &retargets, weights),
        );
        output.extend_from_slice(&bones);
        output.extend_from_slice(&vertex[56..68]); // three weights

        let uv_x = read_f32(vertex, 68)?;
        let uv_y = read_f32(vertex, 72)?;
        push_u32(
            &mut output,
            packed_basis(
                blend_direction(read_float3(vertex, 20)?, &retargets, weights),
                127,
            ),
        );
        push_u32(
            &mut output,
            packed_basis(
                blend_direction(read_float3(vertex, 32)?, &retargets, weights),
                quantized_uv_fraction(uv_x),
            ),
        );
        push_u32(
            &mut output,
            packed_basis(
                blend_direction(read_float3(vertex, 44)?, &retargets, weights),
                quantized_uv_fraction(uv_y),
            ),
        );
        output.extend_from_slice(&vertex[68..76]); // uv
    }

    push_u32(&mut output, 4);
    push_u32(&mut output, u32::try_from(indices.len())?);
    output.extend_from_slice(&indices);
    Ok(output)
}

fn parse_weight_transfer_reference(bytes: &[u8]) -> Result<WeightTransferReference> {
    if read_u32(bytes, 0)? != 3 {
        bail!("weight-transfer template does not start with vertex chunk 3");
    }
    let chunk_size = usize::try_from(read_u32(bytes, 4)?)?;
    let count = usize::try_from(read_u32(bytes, 8)?)?;
    let expected_chunk_size = 4_usize
        .checked_add(count.checked_mul(48).context("reference vertex overflow")?)
        .context("reference vertex overflow")?;
    if chunk_size != expected_chunk_size {
        bail!("weight-transfer vertex chunk has size {chunk_size}, expected {expected_chunk_size}");
    }
    let vertex_end = 8_usize
        .checked_add(chunk_size)
        .context("reference vertex offset overflow")?;
    if read_u32(bytes, vertex_end)? != 4 {
        bail!("weight-transfer template has no index chunk after its vertices");
    }

    let mut vertices = Vec::with_capacity(count);
    for index in 0..count {
        let vertex = bytes
            .get(12 + index * 48..12 + (index + 1) * 48)
            .context("truncated weight-transfer vertex")?;
        let bones = [vertex[12], vertex[13], vertex[14], vertex[15]];
        let mut values = [
            f64::from(read_f32(vertex, 16)?),
            f64::from(read_f32(vertex, 20)?),
            f64::from(read_f32(vertex, 24)?),
            0.0,
        ];
        values[3] = 1.0 - values[0] - values[1] - values[2];
        if !values.iter().all(|weight| weight.is_finite()) {
            bail!("weight-transfer vertex {index} has non-finite weights");
        }
        vertices.push(WeightTransferVertex {
            position: read_float3(vertex, 0)?,
            normal: unpack_packed_basis(read_u32(vertex, 28)?),
            weights: [
                (bones[0], values[0].max(0.0)),
                (bones[1], values[1].max(0.0)),
                (bones[2], values[2].max(0.0)),
                (bones[3], values[3].max(0.0)),
            ],
        });
    }
    if vertices.is_empty() {
        bail!("weight-transfer template contains no vertices");
    }
    Ok(WeightTransferReference { vertices })
}

impl WeightTransferReference {
    fn interpolate_weights(&self, position: [f32; 3], normal: [f32; 3]) -> Result<Vec<(u8, f64)>> {
        const NEIGHBORS: usize = 8;
        const NORMAL_GRADIENT_SCALE: f32 = 0.0025;

        let mut matches = Vec::with_capacity(self.vertices.len());
        for vertex in &self.vertices {
            // Do not let the nearest search cross the body's symmetry plane.
            if position[0].abs() > 0.05 && position[0] * vertex.position[0] < 0.0 {
                continue;
            }
            let delta = [
                position[0] - vertex.position[0],
                position[1] - vertex.position[1],
                position[2] - vertex.position[2],
            ];
            let distance_squared = delta.into_iter().map(|value| value * value).sum::<f32>();
            let normal_alignment = normal
                .into_iter()
                .zip(vertex.normal)
                .map(|(left, right)| left * right)
                .sum::<f32>()
                .clamp(-1.0, 1.0);
            let cost = distance_squared + NORMAL_GRADIENT_SCALE * (1.0 - normal_alignment).powi(2);
            matches.push((cost, vertex));
        }
        matches.sort_by(|left, right| left.0.total_cmp(&right.0));
        if matches.is_empty() {
            bail!("weight-transfer search found no same-side reference vertices");
        }

        let mut combined: BTreeMap<u8, f64> = BTreeMap::new();
        let mut total = 0.0_f64;
        for (cost, vertex) in matches.into_iter().take(NEIGHBORS) {
            let match_weight = 1.0 / f64::from(cost.max(1.0e-6));
            total += match_weight;
            for (bone, skin_weight) in vertex.weights {
                *combined.entry(bone).or_default() += skin_weight * match_weight;
            }
        }
        if !total.is_finite() || total <= f64::EPSILON {
            bail!("weight-transfer interpolation produced an invalid total");
        }
        let mut result: Vec<(u8, f64)> = combined
            .into_iter()
            .map(|(bone, weight)| (bone, weight / total))
            .filter(|(_, weight)| *weight > f64::EPSILON)
            .collect();
        result.sort_by(|left, right| right.1.total_cmp(&left.1));
        Ok(result)
    }
}

fn unpack_packed_basis(value: u32) -> [f32; 3] {
    let component = |shift| f32::from(((value >> shift) & 0xff_u32) as u8) / 127.5 - 1.0;
    normalized_direction([component(16), component(8), component(0)])
}

fn normalized_direction(mut value: [f32; 3]) -> [f32; 3] {
    let length = value
        .into_iter()
        .map(|component| component * component)
        .sum::<f32>()
        .sqrt();
    if length > f32::EPSILON {
        for component in &mut value {
            *component /= length;
        }
    }
    value
}

fn convert_maya_static_surface(
    vertices: &[u8],
    indices: &[u8],
    skin: &MayaSkin,
    bone_remap: &[BoneRetarget],
    weight_remap: &[Vec<(u8, f64)>],
    rotate_y_180: bool,
    correspondence_remap: &[BoneRetarget],
    weight_transfer: Option<&WeightTransferReference>,
    transfer_influences: &[bool],
    transfer_threshold: f64,
    trim_influences: &[bool],
    trim_threshold: f64,
    include_influence_names: &BTreeSet<String>,
    include_threshold: f64,
    trim_triangles: &BTreeSet<usize>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let count = usize::try_from(read_u32(vertices, 0)?)?;
    let expected = 4_usize
        .checked_add(
            count
                .checked_mul(56)
                .context("static vertex byte count overflow")?,
        )
        .context("static vertex byte count overflow")?;
    if vertices.len() != expected {
        bail!(
            "static vertex payload has size {}, expected {expected} for {count} vertices",
            vertices.len()
        );
    }
    if indices.len() < 4 {
        bail!("index payload is shorter than its count");
    }
    if bone_remap.len() != skin.influence_names.len()
        || weight_remap.len() != skin.influence_names.len()
    {
        bail!(
            "Maya bone/weight remaps have {}/{} entries for {} influences",
            bone_remap.len(),
            weight_remap.len(),
            skin.influence_names.len()
        );
    }
    if weight_transfer.is_some()
        && (correspondence_remap.len() != skin.influence_names.len()
            || transfer_influences.len() != skin.influence_names.len())
    {
        bail!("Maya weight-transfer tables do not match the influence count");
    }
    if trim_influences.len() != skin.influence_names.len() {
        bail!("Maya triangle-trim table does not match the influence count");
    }

    let positions: Vec<[f32; 3]> = (0..count)
        .map(|index| read_float3(vertices, 4 + index * 56))
        .collect::<Result<_>>()?;
    let maya_vertices = skin.match_static_positions(&positions)?;
    let include_influences: Vec<bool> = skin
        .influence_names
        .iter()
        .map(|name| include_influence_names.contains(name))
        .collect();
    let indices = trim_maya_triangles(
        indices,
        &maya_vertices,
        skin,
        trim_influences,
        trim_threshold,
        &include_influences,
        include_threshold,
        trim_triangles,
    )?;

    let vertex_chunk_size = 4_usize
        .checked_add(count.checked_mul(48).context("vertex output overflow")?)
        .context("vertex output overflow")?;
    let mut output = Vec::with_capacity(16 + vertex_chunk_size + indices.len());
    push_u32(&mut output, 3);
    push_u32(&mut output, u32::try_from(vertex_chunk_size)?);
    push_u32(&mut output, u32::try_from(count)?);

    for (index, &maya_vertex) in maya_vertices.iter().enumerate() {
        let source = &vertices[4 + index * 56..][..56];
        let mut combined: BTreeMap<u8, f64> = BTreeMap::new();
        for &(influence, weight) in &skin.weights[maya_vertex] {
            let targets = weight_remap.get(influence).with_context(|| {
                format!("Maya vertex {maya_vertex} references missing influence {influence}")
            })?;
            for &(target, factor) in targets {
                *combined.entry(target).or_default() += weight * factor;
            }
        }
        let mut weights: Vec<(u8, f64)> = combined
            .into_iter()
            .filter(|(_, weight)| *weight > f64::EPSILON)
            .collect();
        let orient = |value: [f32; 3]| {
            if rotate_y_180 {
                [-value[0], value[1], -value[2]]
            } else {
                value
            }
        };
        let source_weights = &skin.weights[maya_vertex];
        let transfer_weight: f64 = source_weights
            .iter()
            .filter(|(influence, _)| transfer_influences.get(*influence) == Some(&true))
            .map(|(_, weight)| weight)
            .sum();
        if transfer_weight >= transfer_threshold
            && let Some(reference) = weight_transfer
        {
            let match_position = blend_maya_value(
                orient(read_float3(source, 0)?),
                source_weights,
                correspondence_remap,
                Matrix4::transform_position,
            )?;
            let match_normal = normalized_direction(blend_maya_value(
                orient(read_float3(source, 12)?),
                source_weights,
                correspondence_remap,
                Matrix4::transform_direction,
            )?);
            weights = reference.interpolate_weights(match_position, match_normal)?;
        }
        weights.sort_by(|left, right| right.1.total_cmp(&left.1));
        weights.truncate(4);
        let total: f64 = weights.iter().map(|(_, weight)| weight).sum();
        if !total.is_finite() || total <= f64::EPSILON {
            bail!("Maya vertex {maya_vertex} has no usable target skin weights");
        }
        for (_, weight) in &mut weights {
            *weight /= total;
        }

        let first_bone = weights[0].0;
        let mut bones = [first_bone; 4];
        let mut normalized = [0.0_f32; 4];
        for (slot, &(bone, weight)) in weights.iter().enumerate() {
            bones[slot] = bone;
            normalized[slot] = weight as f32;
        }

        write_float3(
            &mut output,
            blend_maya_value(
                orient(read_float3(source, 0)?),
                source_weights,
                bone_remap,
                Matrix4::transform_position,
            )?,
        );
        output.extend_from_slice(&bones);
        for weight in &normalized[..3] {
            output.extend_from_slice(&weight.to_le_bytes());
        }
        let uv_x = read_f32(source, 48)?;
        let uv_y = read_f32(source, 52)?;
        push_u32(
            &mut output,
            packed_basis(
                blend_maya_value(
                    orient(read_float3(source, 12)?),
                    source_weights,
                    bone_remap,
                    Matrix4::transform_direction,
                )?,
                127,
            ),
        );
        push_u32(
            &mut output,
            packed_basis(
                blend_maya_value(
                    orient(read_float3(source, 24)?),
                    source_weights,
                    bone_remap,
                    Matrix4::transform_direction,
                )?,
                quantized_uv_fraction(uv_x),
            ),
        );
        push_u32(
            &mut output,
            packed_basis(
                blend_maya_value(
                    orient(read_float3(source, 36)?),
                    source_weights,
                    bone_remap,
                    Matrix4::transform_direction,
                )?,
                quantized_uv_fraction(uv_y),
            ),
        );
        output.extend_from_slice(&source[48..56]); // uv
    }

    push_u32(&mut output, 4);
    push_u32(&mut output, u32::try_from(indices.len())?);
    output.extend_from_slice(&indices);
    Ok((output, indices))
}

fn trim_maya_triangles(
    indices: &[u8],
    maya_vertices: &[usize],
    skin: &MayaSkin,
    trim_influences: &[bool],
    threshold: f64,
    include_influences: &[bool],
    include_threshold: f64,
    trim_triangles: &BTreeSet<usize>,
) -> Result<Vec<u8>> {
    let selecting = include_influences.iter().any(|enabled| *enabled);
    if !trim_influences.iter().any(|enabled| *enabled) && !selecting && trim_triangles.is_empty() {
        return Ok(indices.to_vec());
    }
    let count = usize::try_from(read_u32(indices, 0)?)?;
    if count % 3 != 0 || indices.len() != 4 + count * 2 {
        bail!("static index payload is not a complete triangle list");
    }
    if let Some(triangle) = trim_triangles
        .iter()
        .copied()
        .find(|triangle| *triangle >= count / 3)
    {
        bail!(
            "requested Maya triangle {triangle} is outside source triangle count {}",
            count / 3
        );
    }
    let should_trim = |vertex: usize| -> Result<bool> {
        let maya_vertex = *maya_vertices
            .get(vertex)
            .with_context(|| format!("triangle references missing static vertex {vertex}"))?;
        let weight: f64 = skin.weights[maya_vertex]
            .iter()
            .filter(|(influence, _)| trim_influences.get(*influence) == Some(&true))
            .map(|(_, weight)| weight)
            .sum();
        Ok(weight >= threshold)
    };
    let include_weight = |vertex: usize| -> Result<f64> {
        let maya_vertex = *maya_vertices
            .get(vertex)
            .with_context(|| format!("triangle references missing static vertex {vertex}"))?;
        Ok(skin.weights[maya_vertex]
            .iter()
            .filter(|(influence, _)| include_influences.get(*influence) == Some(&true))
            .map(|(_, weight)| weight)
            .sum())
    };
    let mut kept = Vec::with_capacity(indices.len());
    push_u32(&mut kept, 0);
    let mut removed = 0_usize;
    for triangle in 0..count / 3 {
        let offset = 4 + triangle * 6;
        let vertices = [
            usize::from(read_u16(indices, offset)?),
            usize::from(read_u16(indices, offset + 2)?),
            usize::from(read_u16(indices, offset + 4)?),
        ];
        let included = !selecting
            || vertices
                .into_iter()
                .map(&include_weight)
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .sum::<f64>()
                / 3.0
                >= include_threshold;
        if !included
            || trim_triangles.contains(&triangle)
            || vertices
                .into_iter()
                .map(&should_trim)
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .any(|trim| trim)
        {
            removed += 1;
        } else {
            kept.extend_from_slice(&indices[offset..offset + 6]);
        }
    }
    let kept_count = (count / 3 - removed) * 3;
    kept[..4].copy_from_slice(&u32::try_from(kept_count)?.to_le_bytes());
    eprintln!(
        "trimmed_maya_triangles={removed} kept_maya_triangles={}",
        kept_count / 3
    );
    Ok(kept)
}

fn blend_maya_value(
    value: [f32; 3],
    weights: &[(usize, f64)],
    bone_remap: &[BoneRetarget],
    transform: fn(Matrix4, [f32; 3]) -> [f32; 3],
) -> Result<[f32; 3]> {
    let total: f64 = weights.iter().map(|(_, weight)| weight).sum();
    if !total.is_finite() || total <= f64::EPSILON {
        bail!("Maya skin vertex has no usable weights");
    }
    let mut result = [0.0_f32; 3];
    for &(influence, weight) in weights {
        let retarget = bone_remap
            .get(influence)
            .with_context(|| format!("missing Maya bone retarget {influence}"))?;
        let transformed = transform(retarget.source_to_target, value);
        let normalized = (weight / total) as f32;
        for axis in 0..3 {
            result[axis] += transformed[axis] * normalized;
        }
    }
    Ok(result)
}

fn remap_bone(
    source: u16,
    bone_remap: &[BoneRetarget],
    vertex_index: usize,
) -> Result<BoneRetarget> {
    bone_remap.get(usize::from(source)).copied().with_context(|| {
        format!(
            "vertex {vertex_index} references donor bone {source}, but the bind pose has only {} bones",
            bone_remap.len()
        )
    })
}

fn blend_position(value: [f32; 3], retargets: &[BoneRetarget; 4], weights: [f32; 4]) -> [f32; 3] {
    blend_transformed(value, retargets, weights, Matrix4::transform_position)
}

fn blend_direction(value: [f32; 3], retargets: &[BoneRetarget; 4], weights: [f32; 4]) -> [f32; 3] {
    blend_transformed(value, retargets, weights, Matrix4::transform_direction)
}

fn blend_transformed(
    value: [f32; 3],
    retargets: &[BoneRetarget; 4],
    weights: [f32; 4],
    transform: fn(Matrix4, [f32; 3]) -> [f32; 3],
) -> [f32; 3] {
    let mut result = [0.0; 3];
    for (retarget, weight) in retargets.iter().zip(weights) {
        let transformed = transform(retarget.source_to_target, value);
        for axis in 0..3 {
            result[axis] += transformed[axis] * weight;
        }
    }
    result
}

fn write_float3(output: &mut Vec<u8>, vector: [f32; 3]) {
    for component in vector {
        output.extend_from_slice(&component.to_le_bytes());
    }
}

fn packed_basis(mut vector: [f32; 3], alpha: u8) -> u32 {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length > f32::EPSILON {
        for component in &mut vector {
            *component /= length;
        }
    }
    let component = |value: f32| (((value + 1.0) * 127.5).floor() as i32).clamp(0, 255) as u32;
    ((alpha as u32) << 24)
        | (component(vector[0]) << 16)
        | (component(vector[1]) << 8)
        | component(vector[2])
}

fn quantized_uv_fraction(value: f32) -> u8 {
    let rebased = value * 1024.0;
    let primary = rebased.floor().clamp(-32768.0, 32767.0);
    (255.5 * (rebased - primary)).floor().clamp(0.0, 255.0) as u8
}

fn read_float3(bytes: &[u8], offset: usize) -> Result<[f32; 3]> {
    Ok([
        read_f32(bytes, offset)?,
        read_f32(bytes, offset + 4)?,
        read_f32(bytes, offset + 8)?,
    ])
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<f32> {
    Ok(f32::from_bits(read_u32(bytes, offset)?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .with_context(|| format!("missing u32 at byte {offset}"))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let value = bytes
        .get(offset..offset + 2)
        .with_context(|| format!("missing u16 at byte {offset}"))?;
    Ok(u16::from_le_bytes(value.try_into().unwrap()))
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_raw_resource(
    overlay: &Path,
    path: &str,
    bytes: &[u8],
    source: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let output = safe_join(overlay, path)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, bytes)?;
    files.push(record_file(path, source, bytes));
    Ok(())
}

fn named(name: impl Into<String>, value: ConfigValue) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decomposes_bind_frame_without_changing_its_matrix() {
        let original = Matrix4::from_frame(
            [0.657293, 1.05544, 0.114311],
            [-1.2, 0.37, 2.1],
            [1.0, 1.0, 1.0],
        );
        let (translation, rotation, scale) = original.to_frame().unwrap();
        let rebuilt = Matrix4::from_frame(translation, rotation, scale);
        for row in 0..4 {
            for column in 0..4 {
                assert!(
                    (original.0[row][column] - rebuilt.0[row][column]).abs() < 1.0e-5,
                    "matrix mismatch at [{row}][{column}]"
                );
            }
        }
    }

    #[test]
    fn converts_one_four_weight_vertex_to_chunked_hardware_layout() {
        let mut vertices = Vec::new();
        push_u32(&mut vertices, 1);
        vertices.extend_from_slice(&1_u16.to_le_bytes());
        vertices.extend_from_slice(&2_u16.to_le_bytes());
        vertices.extend_from_slice(&u16::MAX.to_le_bytes());
        vertices.extend_from_slice(&4_u16.to_le_bytes());
        for value in [
            1.0_f32, 2.0, 3.0, // P
            0.0, 1.0, 0.0, // N
            1.0, 0.0, 0.0, // T
            0.0, 0.0, 1.0, // B
            0.2, 0.3, 0.4, // weights
            0.5, 0.25, // uv
        ] {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
        let indices = [3_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0];

        let remap: Vec<_> = [9, 8, 7, 6, 5]
            .into_iter()
            .map(|target_index| BoneRetarget {
                target_index,
                source_to_target: Matrix4::identity(),
            })
            .collect();
        let converted = convert_skinned_surface(&vertices, &indices, &remap).unwrap();

        assert_eq!(read_u32(&converted, 0).unwrap(), 3);
        assert_eq!(read_u32(&converted, 4).unwrap(), 52);
        assert_eq!(read_u32(&converted, 8).unwrap(), 1);
        assert_eq!(read_f32(&converted, 12).unwrap(), 1.0);
        assert_eq!(read_f32(&converted, 16).unwrap(), 2.0);
        assert_eq!(read_f32(&converted, 20).unwrap(), 3.0);
        assert_eq!(&converted[24..28], &[8, 7, 8, 5]);
        assert_eq!(read_u32(&converted, 60).unwrap(), 4);
        assert_eq!(read_u32(&converted, 64).unwrap(), indices.len() as u32);
        assert_eq!(&converted[68..], &indices);
    }

    #[test]
    fn parses_and_maps_bind_pose_bones() {
        fn bind_pose(names: &[&str]) -> Vec<u8> {
            let mut result = Vec::new();
            result.extend_from_slice(&(names.len() as u16).to_le_bytes());
            for name in names {
                result.extend_from_slice(name.as_bytes());
                result.push(0);
                for value in [0.0_f32; 6].into_iter().chain([1.0_f32; 3]) {
                    result.extend_from_slice(&value.to_le_bytes());
                }
            }
            result
        }

        let source = parse_bind_pose(&bind_pose(&["Hips", "Spine"]), "source").unwrap();
        let target = parse_bind_pose(&bind_pose(&["Hip", "Spin"]), "target").unwrap();
        let map = BTreeMap::from([
            ("Hips".to_owned(), "Hip".to_owned()),
            ("Spine".to_owned(), "Spin".to_owned()),
        ]);
        let remap = make_bone_remap(&source, &target, &map).unwrap();
        assert_eq!(
            remap
                .iter()
                .map(|bone| bone.target_index)
                .collect::<Vec<_>>(),
            [0, 1]
        );
    }

    #[test]
    fn retargets_a_position_between_global_bind_frames() {
        let source =
            Matrix4::from_frame([0.0, 0.0, 1.0], [0.0, std::f32::consts::PI, 0.0], [1.0; 3]);
        let target = Matrix4::from_frame([2.0, 0.0, 0.0], [0.0; 3], [1.0; 3]);
        let source_to_target = source.inverse_affine().unwrap().multiply(target);

        let result = source_to_target.transform_position([-1.0, 0.0, 1.0]);

        assert!((result[0] - 3.0).abs() < 1.0e-5);
        assert!(result[1].abs() < 1.0e-5);
        assert!(result[2].abs() < 1.0e-5);
    }

    #[test]
    fn limits_pose_retargeting_to_arm_and_hand_chains() {
        for name in [
            "LeftArm",
            "RightForeArm",
            "LeftHandRoll",
            "RightHandMiddle_3",
        ] {
            assert!(requires_pose_retarget(name), "{name}");
        }
        for name in ["Spine", "Head", "LeftCollar", "RightUpLeg", "Neck"] {
            assert!(!requires_pose_retarget(name), "{name}");
        }
    }
}
