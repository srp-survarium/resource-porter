// SPDX-License-Identifier: GPL-3.0-or-later
//! Armour modelled for 0.26g (not ported from another build), as v0.26g
//! equipment models.
//!
//! A piece is a skinned glTF 2.0 binary (`.glb`, as Blender exports it) and
//! four PNG maps. This builder turns each piece of a manifest
//! (`ports/*-model-to-026g.json`) into the files a v0.26g item config can
//! draw:
//!
//! - **Surface.** One LOD0 surface in the v0.26g four-bone layout
//!   ([`crate::v069d_skinned::to_026_four_bone`]). The glTF joints are the
//!   native bones by name; the vertices are already where the native rig
//!   binds them. glTF is right-handed, the game left-handed: a vertex at
//!   glTF `(x, y, z)` (+Y up, the wearer facing +Z) is at `(-x, y, z)` in the
//!   game, and each triangle turns the other way round. The tangent frame is
//!   the game's: the tangent along +u, the binormal along +v (v down the
//!   texture, as glTF and Direct3D have it), checked against the UVs.
//! - **Model folder, fit.** As [`crate::armour_069d`] makes them: the bind
//!   pose, skeleton, settings and properties of a native model of the same
//!   slot (`template_model`), and a back piece measured against every stock
//!   torso that leaves the back slot free.
//! - **Material.** The native material `template_material` with the piece's
//!   four maps in its diffuse, normal, gloss (`texture_roughness`) and
//!   Fresnel slots; nothing else changes, so it draws with the template's
//!   compiled shader.
//! - **Textures.** The PNGs block-compressed as 0.26g's character maps are:
//!   the diffuse, gloss and Fresnel maps DXT1, the normal map DXT5 with X in
//!   alpha and Y in green (the PNG's red and green, OpenGL convention: +Y up
//!   the texture, as 0.26g's maps have it), a full mip chain down to 4x4, the
//!   native container and the template's options for each role.
//!
//! What the item is (its config, level, price, texts, icon) is not this
//! builder's: the pack that ships the model writes it.
use crate::armour_069d::{
    BackReference, Fit, Surface, TorsoFit, effect, fit_back, model_files, released_combinations,
    set, surface_mesh, switches,
};
use crate::manifest::BuiltFile;
use crate::outfit_026::Writer;
use crate::v069d_skinned::{Surface as Skinned, Vertex, to_026_four_bone};
use crate::weapon::parse_config;
use crate::{ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The pieces to build.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub asset_id: String,
    /// Maps may be no larger than this edge (native character maps are 1024).
    pub max_texture_size: u32,
    pub pieces: Vec<Piece>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(&fs::read(path)?)
            .with_context(|| format!("reading {}", path.display()))?;
        ensure!(!manifest.pieces.is_empty(), "{}: no pieces", path.display());
        ensure!(
            manifest.max_texture_size.is_power_of_two() && manifest.max_texture_size >= 64,
            "max_texture_size must be a power of two"
        );
        Ok(manifest)
    }
}

/// One modelled piece.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    /// The piece's name, for provenance.
    pub item: String,
    /// The skinned mesh, relative to the source folder.
    pub mesh: String,
    /// The PNG maps, relative to the source folder.
    pub maps: Maps,
    /// The model to write, under `models/`, without `.skinned_model`.
    pub model: String,
    /// Its one surface's name.
    pub surface: String,
    /// The material to write, under `material_instances/`, without
    /// `.material`.
    pub material: String,
    /// The textures' stem under `textures/`: `<stem>.dds`, `<stem>_nmap.dds`,
    /// `<stem>_smap.dds`, `<stem>_fmap.dds`.
    pub texture: String,
    /// A native model of the same slot, under `models/`.
    pub template_model: String,
    /// The native material the piece's starts from, under
    /// `material_instances/`, without `.material`.
    pub template_material: String,
    pub fit: Fit,
}

/// A piece's maps.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Maps {
    pub diffuse: String,
    pub normal: String,
    pub gloss: String,
    pub fresnel: String,
}

/// A map of the material, by role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Diffuse,
    Normal,
    Gloss,
    Fresnel,
}

impl Role {
    const ALL: [Self; 4] = [Self::Diffuse, Self::Normal, Self::Gloss, Self::Fresnel];

    /// The v0.26g texture field and its switch.
    fn field(self) -> (&'static str, &'static str) {
        match self {
            Self::Diffuse => ("texture_diffuse", "use_tdiffuse"),
            Self::Normal => ("texture_normal", "use_nmap"),
            Self::Gloss => ("texture_roughness", "use_troughness"),
            Self::Fresnel => ("texture_fresnel", "use_tfresnel"),
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Diffuse => "",
            Self::Normal => "_nmap",
            Self::Gloss => "_smap",
            Self::Fresnel => "_fmap",
        }
    }

    fn source(self, maps: &Maps) -> &str {
        match self {
            Self::Diffuse => &maps.diffuse,
            Self::Normal => &maps.normal,
            Self::Gloss => &maps.gloss,
            Self::Fresnel => &maps.fresnel,
        }
    }
}

// ---------------------------------------------------------------------------
// glTF

/// The parts of a `.glb` this builder reads.
struct Glb {
    json: Value,
    bin: Vec<u8>,
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4).context("truncated")?.try_into()?,
    ))
}

impl Glb {
    fn parse(bytes: &[u8], label: &str) -> Result<Self> {
        ensure!(
            bytes.get(..4) == Some(b"glTF"),
            "{label}: not a binary glTF"
        );
        ensure!(u32_at(bytes, 4)? == 2, "{label}: not glTF 2");
        let mut at = 12;
        let (mut json, mut bin) = (None, None);
        while at < bytes.len() {
            let size = usize::try_from(u32_at(bytes, at)?)?;
            let kind = bytes.get(at + 4..at + 8).context("truncated chunk")?;
            let body = bytes
                .get(at + 8..at + 8 + size)
                .with_context(|| format!("{label}: a chunk runs past the file"))?;
            match kind {
                b"JSON" => json = Some(serde_json::from_slice(body)?),
                b"BIN\0" => bin = Some(body.to_vec()),
                _ => {}
            }
            at += 8 + size;
        }
        Ok(Self {
            json: json.with_context(|| format!("{label}: no JSON chunk"))?,
            bin: bin.with_context(|| format!("{label}: no binary chunk"))?,
        })
    }

    fn get(&self, kind: &str, index: usize) -> Result<&Value> {
        self.json[kind]
            .get(index)
            .with_context(|| format!("no {kind}[{index}]"))
    }

    /// An accessor's elements as floats (normalized integers scaled to 0..1).
    fn floats(&self, index: usize, width: usize) -> Result<Vec<Vec<f32>>> {
        let a = self.get("accessors", index)?;
        ensure!(a.get("sparse").is_none(), "sparse accessor {index}");
        let components = match a["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            other => bail!("accessor {index} is {other:?}"),
        };
        ensure!(
            components == width,
            "accessor {index} has {components} components, {width} wanted"
        );
        let kind = a["componentType"].as_u64().context("componentType")?;
        let size = match kind {
            5126 | 5125 => 4,
            5123 => 2,
            5121 => 1,
            other => bail!("accessor {index}: component type {other}"),
        };
        let normalized = a["normalized"].as_bool().unwrap_or(false);
        let count = usize::try_from(a["count"].as_u64().context("count")?)?;
        let view = self.get(
            "bufferViews",
            usize::try_from(a["bufferView"].as_u64().context("bufferView")?)?,
        )?;
        ensure!(
            view["buffer"].as_u64() == Some(0),
            "accessor {index} is not in the binary chunk"
        );
        let start = usize::try_from(
            view["byteOffset"].as_u64().unwrap_or(0) + a["byteOffset"].as_u64().unwrap_or(0),
        )?;
        let stride = match view["byteStride"].as_u64() {
            Some(s) => usize::try_from(s)?,
            None => size * components,
        };
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let mut element = Vec::with_capacity(components);
            for c in 0..components {
                let at = start + i * stride + c * size;
                let b = self
                    .bin
                    .get(at..at + size)
                    .with_context(|| format!("accessor {index} runs past the buffer"))?;
                element.push(match kind {
                    5126 => f32::from_le_bytes(b.try_into()?),
                    5125 => u32::from_le_bytes(b.try_into()?) as f32,
                    5123 => {
                        let v = f32::from(u16::from_le_bytes(b.try_into()?));
                        if normalized { v / 65535.0 } else { v }
                    }
                    _ => {
                        let v = f32::from(b[0]);
                        if normalized { v / 255.0 } else { v }
                    }
                });
            }
            out.push(element);
        }
        Ok(out)
    }

    fn indices(&self, index: usize) -> Result<Vec<u32>> {
        let a = self.get("accessors", index)?;
        ensure!(
            matches!(a["componentType"].as_u64(), Some(5121 | 5123 | 5125)),
            "index accessor {index} is not unsigned integers"
        );
        Ok(self
            .floats(index, 1)?
            .into_iter()
            .map(|v| v[0] as u32)
            .collect())
    }
}

/// A node's transform is none (no matrix, no translation, rotation or scale
/// other than the identity's).
fn untransformed(node: &Value) -> bool {
    let is = |key: &str, identity: &[f64]| match node.get(key) {
        None => true,
        Some(Value::Array(values)) => values
            .iter()
            .zip(identity)
            .all(|(v, i)| v.as_f64().is_some_and(|v| (v - i).abs() < 1e-6)),
        Some(_) => false,
    };
    node.get("matrix").is_none()
        && is("translation", &[0.0; 3])
        && is("rotation", &[0.0, 0.0, 0.0, 1.0])
        && is("scale", &[1.0; 3])
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let l = dot3(v, v).sqrt();
    if l > f32::EPSILON {
        v.map(|x| x / l)
    } else {
        v
    }
}

fn mirror(v: &[f32]) -> [f32; 3] {
    [-v[0], v[1], v[2]]
}

/// The glb's one skinned mesh as a game surface on the native rig (`bones`:
/// the native bind pose's bone names, in order).
fn game_surface(glb: &Glb, bones: &[String], label: &str) -> Result<Skinned> {
    let nodes = glb.json["nodes"].as_array().context("no nodes")?;
    let meshes: Vec<usize> = (0..nodes.len())
        .filter(|&i| nodes[i].get("mesh").is_some())
        .collect();
    ensure!(
        meshes.len() == 1,
        "{label}: {} mesh nodes, one wanted",
        meshes.len()
    );
    let node = &nodes[meshes[0]];
    // the mesh's vertices are where the game draws them: no transform on it
    // or on any node above it
    let mut at = meshes[0];
    loop {
        ensure!(
            untransformed(&nodes[at]),
            "{label}: node {} moves the mesh",
            nodes[at]["name"]
        );
        match (0..nodes.len()).find(|&p| {
            nodes[p]["children"]
                .as_array()
                .is_some_and(|c| c.iter().any(|c| c.as_u64() == Some(at as u64)))
        }) {
            Some(parent) => at = parent,
            None => break,
        }
    }
    let skin = glb.get(
        "skins",
        usize::try_from(node["skin"].as_u64().context("the mesh has no skin")?)?,
    )?;
    let mut joint_bone = vec![];
    for joint in skin["joints"].as_array().context("skin without joints")? {
        let name = nodes
            .get(usize::try_from(joint.as_u64().context("joint")?)?)
            .and_then(|n| n["name"].as_str())
            .context("a joint without a name")?;
        let bone = bones
            .iter()
            .position(|b| b == name)
            .with_context(|| format!("{label}: joint {name} is no native bone"))?;
        joint_bone.push(u8::try_from(bone)?);
    }
    let mesh = glb.get(
        "meshes",
        usize::try_from(node["mesh"].as_u64().context("mesh")?)?,
    )?;
    let mut vertices = vec![];
    let mut indices = vec![];
    for primitive in mesh["primitives"].as_array().context("no primitives")? {
        ensure!(
            primitive["mode"].as_u64().unwrap_or(4) == 4,
            "{label}: a primitive that is not triangles"
        );
        let attr = |key: &str| -> Result<usize> {
            Ok(usize::try_from(
                primitive["attributes"][key]
                    .as_u64()
                    .with_context(|| format!("{label}: no {key}"))?,
            )?)
        };
        let positions = glb.floats(attr("POSITION")?, 3)?;
        let normals = glb.floats(attr("NORMAL")?, 3)?;
        let tangents = glb.floats(attr("TANGENT")?, 4)?;
        let uvs = glb.floats(attr("TEXCOORD_0")?, 2)?;
        let joints = glb.floats(attr("JOINTS_0")?, 4)?;
        let weights = glb.floats(attr("WEIGHTS_0")?, 4)?;
        ensure!(
            primitive["attributes"].get("JOINTS_1").is_none(),
            "{label}: more than four bones a vertex"
        );
        let base = u32::try_from(vertices.len())?;
        for i in 0..positions.len() {
            ensure!(
                positions[i].iter().all(|p| p.is_finite()),
                "{label}: vertex {i} is not finite"
            );
            // the native layout: up to four bones, weights summing to one
            let mut influences: Vec<(u8, f32)> = joints[i]
                .iter()
                .zip(&weights[i])
                .filter(|(_, w)| **w > 1e-4)
                .map(|(j, w)| {
                    let bone = joint_bone
                        .get(*j as usize)
                        .copied()
                        .with_context(|| format!("{label}: vertex {i} names joint {j}"))?;
                    Ok((bone, *w))
                })
                .collect::<Result<_>>()?;
            ensure!(!influences.is_empty(), "{label}: vertex {i} has no bone");
            influences.sort_by(|a, b| b.1.total_cmp(&a.1));
            let total: f32 = influences.iter().map(|(_, w)| w).sum();
            let mut vbones = [influences[0].0; 4];
            let mut vweights = [0.0; 4];
            for (k, (bone, w)) in influences.iter().enumerate() {
                vbones[k] = *bone;
                vweights[k] = w / total;
            }
            let n = unit(mirror(&normals[i]));
            let t_gltf = [tangents[i][0], tangents[i][1], tangents[i][2]];
            let sign = tangents[i][3];
            // glTF's bitangent runs up the texture (OpenGL); the game's
            // binormal down it, as v does
            let b_gltf = cross3(unit([normals[i][0], normals[i][1], normals[i][2]]), t_gltf)
                .map(|x| x * sign);
            vertices.push(Vertex {
                position: mirror(&positions[i]),
                bones: vbones,
                weights: vweights,
                normal: n,
                tangent: unit(mirror(&t_gltf)),
                binormal: unit(mirror(&b_gltf).map(|x| -x)),
                uv: [uvs[i][0], uvs[i][1]],
            });
        }
        let tris = glb.indices(usize::try_from(
            primitive["indices"]
                .as_u64()
                .with_context(|| format!("{label}: unindexed primitive"))?,
        )?)?;
        ensure!(
            tris.len().is_multiple_of(3),
            "{label}: indices are not triangles"
        );
        for t in tris.chunks_exact(3) {
            // mirrored: the other way round
            for i in [t[0], t[2], t[1]] {
                indices.push(u16::try_from(base + i).context("too many vertices")?);
            }
        }
    }
    ensure!(
        vertices.len() <= usize::from(u16::MAX),
        "{label}: too many vertices"
    );
    if let Some(bad) = indices.iter().find(|&&i| usize::from(i) >= vertices.len()) {
        bail!("{label}: index {bad} past {} vertices", vertices.len());
    }
    let used: std::collections::BTreeSet<u8> = vertices
        .iter()
        .flat_map(|v| {
            v.bones
                .iter()
                .zip(v.weights)
                .filter(|(_, w)| *w > 0.0)
                .map(|(b, _)| *b)
        })
        .collect();
    let surface = Skinned {
        vertices,
        indices,
        bones: used.into_iter().collect(),
    };
    check_basis(&surface, label)?;
    Ok(surface)
}

/// The game's frame: each triangle's front by its winding, the tangent along
/// +u and the binormal along +v (the medians over the triangles).
fn check_basis(surface: &Skinned, label: &str) -> Result<()> {
    let (mut facing, mut along_u, mut along_v) = (vec![], vec![], vec![]);
    for t in surface.indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|k| &surface.vertices[usize::from(t[k])]);
        let (e1, e2) = (sub3(b.position, a.position), sub3(c.position, a.position));
        let (d1, d2) = (
            [b.uv[0] - a.uv[0], b.uv[1] - a.uv[1]],
            [c.uv[0] - a.uv[0], c.uv[1] - a.uv[1]],
        );
        let r = d1[0] * d2[1] - d2[0] * d1[1];
        let normal = cross3(e1, e2);
        if r.abs() < 1e-12 || dot3(normal, normal) < 1e-14 {
            continue;
        }
        let dpdu = unit([0, 1, 2].map(|k| (e1[k] * d2[1] - e2[k] * d1[1]) / r));
        let dpdv = unit([0, 1, 2].map(|k| (e2[k] * d1[0] - e1[k] * d2[0]) / r));
        facing.push(dot3(unit(normal), a.normal));
        along_u.push(dot3(dpdu, a.tangent));
        along_v.push(dot3(dpdv, a.binormal));
    }
    for (name, values) in [
        ("faces turn towards their normals", &mut facing),
        ("tangents run along +u", &mut along_u),
        ("binormals run along +v", &mut along_v),
    ] {
        ensure!(!values.is_empty(), "{label}: no measurable triangle");
        values.sort_by(f32::total_cmp);
        let median = values[values.len() / 2];
        ensure!(
            median > 0.5,
            "{label}: not so that {name} (median {median:.2})"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Textures

/// An 8-bit image, four channels a texel.
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<[u8; 4]>,
}

fn read_png(path: &Path) -> Result<Image> {
    let decoder = png::Decoder::new(fs::File::open(path)?);
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer)?;
    ensure!(
        info.bit_depth == png::BitDepth::Eight,
        "{}: not 8 bits a channel",
        path.display()
    );
    let data = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect(),
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::Grayscale => data.iter().map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        other => bail!("{}: colour type {other:?}", path.display()),
    };
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

impl Image {
    /// Half the size, each texel the mean of four; a normal map's (X, Y in
    /// red and green) mean renormalised.
    fn half(&self, normal: bool) -> Self {
        let (w, h) = (self.width / 2, self.height / 2);
        let mut rgba = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let t = self.rgba[((2 * y + dy) * self.width + 2 * x + dx) as usize];
                    for k in 0..4 {
                        sum[k] += u32::from(t[k]);
                    }
                }
                let mut texel = sum.map(|s| ((s + 2) / 4) as u8);
                if normal {
                    let nx = f32::from(texel[0]) / 127.5 - 1.0;
                    let ny = f32::from(texel[1]) / 127.5 - 1.0;
                    let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
                    let l = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
                    texel[0] = ((nx / l + 1.0) * 127.5).round() as u8;
                    texel[1] = ((ny / l + 1.0) * 127.5).round() as u8;
                }
                rgba.push(texel);
            }
        }
        Self {
            width: w,
            height: h,
            rgba,
        }
    }

    fn block(&self, bx: u32, by: u32) -> [[u8; 4]; 16] {
        let mut out = [[0; 4]; 16];
        for (k, texel) in out.iter_mut().enumerate() {
            let (x, y) = (bx * 4 + k as u32 % 4, by * 4 + k as u32 / 4);
            *texel = self.rgba[(y * self.width + x) as usize];
        }
        out
    }
}

fn to565(c: [f32; 3]) -> u16 {
    let q = |v: f32, bits: u32| {
        ((v.clamp(0.0, 255.0) / 255.0 * ((1 << bits) - 1) as f32).round()) as u16
    };
    (q(c[0], 5) << 11) | (q(c[1], 6) << 5) | q(c[2], 5)
}

fn from565(c: u16) -> [f32; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [
        f32::from((r << 3) | (r >> 2)),
        f32::from((g << 2) | (g >> 4)),
        f32::from((b << 3) | (b >> 2)),
    ]
}

/// A DXT1 colour block (four-colour mode): the endpoints are the block's
/// extremes along its principal axis.
fn bc1_block(texels: &[[u8; 4]; 16]) -> [u8; 8] {
    let px: Vec<[f32; 3]> = texels
        .iter()
        .map(|t| [f32::from(t[0]), f32::from(t[1]), f32::from(t[2])])
        .collect();
    let mean = [0, 1, 2].map(|k| px.iter().map(|p| p[k]).sum::<f32>() / 16.0);
    let mut cov = [[0.0f32; 3]; 3];
    for p in &px {
        let d = sub3(*p, mean);
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }
    let mut axis = [1.0f32, 1.0, 1.0];
    for _ in 0..8 {
        let next = [0, 1, 2].map(|i| dot3(cov[i], axis));
        let l = dot3(next, next).sqrt();
        if l < 1e-6 {
            break;
        }
        axis = next.map(|x| x / l);
    }
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for p in &px {
        let t = dot3(sub3(*p, mean), axis);
        lo = lo.min(t);
        hi = hi.max(t);
    }
    let at = |t: f32| [0, 1, 2].map(|k| mean[k] + axis[k] * t);
    let (mut c0, mut c1) = (to565(at(hi)), to565(at(lo)));
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    if c0 == c1 {
        return out;
    }
    let (e0, e1) = (from565(c0), from565(c1));
    let palette = [
        e0,
        e1,
        [0, 1, 2].map(|k| (2.0 * e0[k] + e1[k]) / 3.0),
        [0, 1, 2].map(|k| (e0[k] + 2.0 * e1[k]) / 3.0),
    ];
    let mut bits = 0u32;
    for (i, p) in px.iter().enumerate() {
        let best = (0..4)
            .min_by(|&a, &b| {
                let da = sub3(*p, palette[a]);
                let db = sub3(*p, palette[b]);
                dot3(da, da).total_cmp(&dot3(db, db))
            })
            .unwrap_or(0);
        bits |= (best as u32) << (2 * i);
    }
    out[4..8].copy_from_slice(&bits.to_le_bytes());
    out
}

/// A DXT5 alpha block (eight-value mode: the block's extremes).
fn bc3_alpha_block(values: [u8; 16]) -> [u8; 8] {
    let (a0, a1) = (
        *values.iter().max().unwrap_or(&0),
        *values.iter().min().unwrap_or(&0),
    );
    let mut out = [0u8; 8];
    out[0] = a0;
    out[1] = a1;
    if a0 == a1 {
        return out;
    }
    let palette: Vec<f32> = (0..8)
        .map(|i| match i {
            0 => f32::from(a0),
            1 => f32::from(a1),
            k => ((8 - k) as f32 * f32::from(a0) + (k - 1) as f32 * f32::from(a1)) / 7.0,
        })
        .collect();
    let mut bits = 0u64;
    for (i, v) in values.iter().enumerate() {
        let best = (0..8)
            .min_by(|&a, &b| {
                (f32::from(*v) - palette[a])
                    .abs()
                    .total_cmp(&(f32::from(*v) - palette[b]).abs())
            })
            .unwrap_or(0);
        bits |= (best as u64) << (3 * i);
    }
    out[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
    out
}

/// The map as 0.26g's: block-compressed with its mips down to 4x4, in the
/// native container.
fn encode(image: &Image, role: Role, label: &str, max: u32) -> Result<Vec<u8>> {
    ensure!(
        image.width.is_power_of_two() && image.height.is_power_of_two(),
        "{label}: {}x{} is not a power of two",
        image.width,
        image.height
    );
    ensure!(
        image.width >= 4 && image.height >= 4 && image.width <= max && image.height <= max,
        "{label}: {}x{} is outside 4..{max}",
        image.width,
        image.height
    );
    // the role's texels: the normal map's X to alpha and Y to green; gloss
    // and Fresnel grey
    let texels = image
        .rgba
        .iter()
        .map(|t| match role {
            Role::Diffuse => [t[0], t[1], t[2], 255],
            Role::Normal => [t[0], t[1], 0, 255],
            Role::Gloss | Role::Fresnel => [t[0], t[0], t[0], 255],
        })
        .collect();
    let mut level = Image {
        width: image.width,
        height: image.height,
        rgba: texels,
    };
    let dxt5 = role == Role::Normal;
    let mut payload = vec![];
    let mut levels = 0u32;
    loop {
        for by in 0..level.height / 4 {
            for bx in 0..level.width / 4 {
                let block = level.block(bx, by);
                if dxt5 {
                    payload.extend_from_slice(&bc3_alpha_block(block.map(|t| t[0])));
                    // the colour block carries Y in green, red and blue none
                    payload.extend_from_slice(&bc1_block(&block.map(|t| [0, t[1], 0, 255])));
                } else {
                    payload.extend_from_slice(&bc1_block(&block));
                }
            }
        }
        levels += 1;
        if level.width <= 4 || level.height <= 4 {
            break;
        }
        level = level.half(dxt5);
    }
    let mut dds = Vec::with_capacity(128 + payload.len());
    dds.extend_from_slice(b"DDS ");
    for v in [124u32, 0xA100F, image.height, image.width, 0, 0, levels] {
        dds.extend_from_slice(&v.to_le_bytes());
    }
    dds.extend_from_slice(&[0; 44]);
    for v in [32u32, 4] {
        dds.extend_from_slice(&v.to_le_bytes());
    }
    dds.extend_from_slice(if dxt5 { b"DXT5" } else { b"DXT1" });
    dds.extend_from_slice(&[0; 20]);
    dds.extend_from_slice(&0x401008u32.to_le_bytes());
    dds.extend_from_slice(&[0; 16]);
    debug_assert_eq!(dds.len(), 128);
    dds.extend_from_slice(&payload);
    crate::duty_026::texture_dds(&dds, label, role == Role::Diffuse)
}

// ---------------------------------------------------------------------------

/// What the builder made of a piece.
#[derive(Debug, Serialize)]
pub struct PieceRecord {
    pub item: String,
    pub model: String,
    pub surface: String,
    pub vertices: usize,
    pub triangles: usize,
    pub bones: BTreeMap<String, usize>,
    pub material: String,
    pub textures: Vec<String>,
    pub fit: Vec<TorsoFit>,
}

/// The piece's material: the template's with the four maps, written beside
/// it with the template's options for each role.
fn material_files(
    out: &Writer<'_>,
    target: &mut ResourceDb,
    piece: &Piece,
    source: &Path,
    max: u32,
    files: &mut Vec<BuiltFile>,
) -> Result<Vec<String>> {
    let path = format!("material_instances/{}.material", piece.material);
    ensure!(!target.contains(&path), "{path} is in the released game");
    let template_path = format!("material_instances/{}.material", piece.template_material);
    let mut material = parse_config(target, &template_path, "template material")?;
    let template_effect = effect(&material)?.clone();
    let mut textures = vec![];
    for role in Role::ALL {
        let (field, switch) = role.field();
        ensure!(
            template_effect.named(switch).and_then(|s| s.named("value")) == Some(&V::Boolean(true)),
            "{template_path}: {switch} is off"
        );
        let Some(V::String(template_texture)) =
            template_effect.named(field).and_then(|f| f.named("value"))
        else {
            bail!("{template_path}: no {field}");
        };
        let texture = format!("{}{}", piece.texture, role.suffix());
        let dds = format!("textures/{texture}.dds");
        ensure!(!target.contains(&dds), "{dds} is in the released game");
        let png = source.join(role.source(&piece.maps));
        let image = read_png(&png).with_context(|| format!("reading {}", png.display()))?;
        out.raw(&dds, &encode(&image, role, &dds, max)?, files)?;
        let options = parse_config(
            target,
            &format!("textures/{template_texture}.options"),
            "template texture options",
        )?;
        out.config(&format!("textures/{texture}.options"), &options, files)?;
        set(
            &mut material,
            &format!("material.g_stage.effect.{field}.value"),
            V::String(texture.clone()),
        )?;
        textures.push(texture);
    }
    ensure!(
        switches(&material)?
            == switches(&parse_config(target, &template_path, "template material")?)?,
        "{path}: the switches changed"
    );
    out.config(&path, &material, files)?;
    Ok(textures)
}

/// Builds the manifest's pieces from `source` (the meshes and maps) into
/// `output` (`overlay/`, `port-manifest.json`, `fit-report.json`).
/// `patches`: the released `resources.db.patch_<n>` over `db`, as players
/// have them.
pub fn build(
    manifest: &Manifest,
    source: &Path,
    db: &Path,
    patches: &[PathBuf],
    output: &Path,
) -> Result<Vec<PieceRecord>> {
    crate::weapon::prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    let out = Writer {
        overlay: &overlay,
        tag: "Armour-model",
    };
    let mut target = crate::novice_026::game_view(db, patches)?;
    let released = released_combinations(&mut target)?;
    let reference = BackReference::new(&mut target)?;
    let mut files = vec![];
    let mut records = vec![];
    for piece in &manifest.pieces {
        let template_root = format!("models/{}.skinned_model", piece.template_model);
        let pose = target.read(&format!("{template_root}/render/bind_pose"))?;
        let bones: Vec<String> = crate::armor::bind_pose_frames(&pose, &template_root)?
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let mesh_path = source.join(&piece.mesh);
        let glb = Glb::parse(
            &fs::read(&mesh_path).with_context(|| format!("reading {}", mesh_path.display()))?,
            &piece.mesh,
        )?;
        let skinned = game_surface(&glb, &bones, &piece.mesh)?;
        let mut bone_use = BTreeMap::new();
        for v in &skinned.vertices {
            for (b, w) in v.bones.iter().zip(v.weights) {
                if w > 0.0 {
                    *bone_use.entry(bones[usize::from(*b)].clone()).or_insert(0) += 1;
                }
            }
        }
        let surface = Surface {
            name: piece.surface.clone(),
            bytes: to_026_four_bone(&skinned)?,
            material: piece.material.clone(),
            source_names: vec![
                ("mesh_name", V::String(piece.mesh.clone())),
                ("texture_name", V::String(piece.texture.clone())),
            ],
        };
        let textures = material_files(
            &out,
            &mut target,
            piece,
            source,
            manifest.max_texture_size,
            &mut files,
        )?;
        let on = switches(&parse_config(
            &mut target,
            &format!("material_instances/{}.material", piece.template_material),
            "template material",
        )?)?;
        ensure!(
            released.contains(&on),
            "{}: no released character material draws {on:?}",
            piece.template_material
        );
        model_files(
            &out,
            &mut target,
            &piece.model,
            &piece.template_model,
            std::slice::from_ref(&surface),
            Some(V::String(piece.mesh.clone())),
            &mut files,
        )?;
        let points = surface_mesh(&surface.bytes, &surface.name)?.positions;
        let fit = match piece.fit {
            Fit::Back => fit_back(&reference, &piece.item, &points)?,
        };
        let above_median = fit.iter().filter(|f| f.share > f.native_median).count();
        let worst = fit
            .iter()
            .max_by(|a, b| a.share.total_cmp(&b.share))
            .context("no torso measured")?;
        println!(
            "{} ({}): {} vertices, {} triangles; sunk into {} torsos above the native median, at most {:.1} % ({}; native up to {:.1} %)",
            piece.item,
            piece.model,
            skinned.vertices.len(),
            skinned.indices.len() / 3,
            above_median,
            worst.share * 100.0,
            worst.torso,
            worst.native_max * 100.0
        );
        records.push(PieceRecord {
            item: piece.item.clone(),
            model: piece.model.clone(),
            surface: piece.surface.clone(),
            vertices: skinned.vertices.len(),
            triangles: skinned.indices.len() / 3,
            bones: bone_use,
            material: piece.material.clone(),
            textures,
            fit,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    crate::weapon::reject_duplicate_outputs(&files)?;
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": manifest.asset_id,
            "donor_build": "modelled",
            "target_build": "0.26g",
            "files": files,
        }))?,
    )?;
    fs::write(
        output.join("fit-report.json"),
        serde_json::to_vec_pretty(&records)?,
    )?;
    crate::model_validation::validate_overlay_on(&mut target, &overlay)?;
    println!(
        "Built {} modelled pieces: {} resources",
        records.len(),
        files.len()
    );
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-triangle glb: three vertices on the joints `names`.
    fn glb(
        names: &[&str],
        positions: [[f32; 3]; 3],
        uvs: [[f32; 2]; 3],
        winding: [u16; 3],
    ) -> Vec<u8> {
        let mut bin = vec![];
        let mut views = vec![];
        let mut push = |bytes: Vec<u8>| {
            views.push(json!({"buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len()}));
            bin.extend_from_slice(&bytes);
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
        };
        let floats = |v: &[f32]| v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
        // the face's normal by its winding in glTF space
        let p = positions;
        let n = unit(cross3(sub3(p[1], p[0]), sub3(p[2], p[0])));
        push(floats(&p.concat()));
        push(floats(&[n, n, n].concat()));
        // tangent along +u in glTF space; bitangent (cross(N, T) w) up the
        // texture, i.e. against +v
        let e1 = sub3(p[1], p[0]);
        let e2 = sub3(p[2], p[0]);
        let (d1, d2) = (
            [uvs[1][0] - uvs[0][0], uvs[1][1] - uvs[0][1]],
            [uvs[2][0] - uvs[0][0], uvs[2][1] - uvs[0][1]],
        );
        let r = d1[0] * d2[1] - d2[0] * d1[1];
        let t = unit([0, 1, 2].map(|k| (e1[k] * d2[1] - e2[k] * d1[1]) / r));
        let dpdv = unit([0, 1, 2].map(|k| (e2[k] * d1[0] - e1[k] * d2[0]) / r));
        let w = if dot3(cross3(n, t), dpdv) < 0.0 {
            1.0
        } else {
            -1.0
        };
        push(floats(&[t[0], t[1], t[2], w].repeat(3)));
        push(floats(&uvs.concat()));
        push(vec![1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
        push(floats(&[1.0, 0.0, 0.0, 0.0].repeat(3)));
        push(winding.iter().flat_map(|i| i.to_le_bytes()).collect());
        let accessor = |view: usize, kind: u32, ty: &str, count: usize| json!({"bufferView": view, "componentType": kind, "type": ty, "count": count});
        let mut nodes: Vec<Value> = names.iter().map(|n| json!({"name": n})).collect();
        nodes.push(json!({"name": "mesh", "mesh": 0, "skin": 0}));
        let document = json!({
            "asset": {"version": "2.0"},
            "nodes": nodes,
            "skins": [{"joints": (0..names.len()).collect::<Vec<_>>()}],
            "meshes": [{"primitives": [{"attributes": {
                "POSITION": 0, "NORMAL": 1, "TANGENT": 2, "TEXCOORD_0": 3,
                "JOINTS_0": 4, "WEIGHTS_0": 5}, "indices": 6}]}],
            "accessors": [accessor(0, 5126, "VEC3", 3), accessor(1, 5126, "VEC3", 3),
                accessor(2, 5126, "VEC4", 3), accessor(3, 5126, "VEC2", 3),
                accessor(4, 5121, "VEC4", 3), accessor(5, 5126, "VEC4", 3),
                accessor(6, 5123, "SCALAR", 3)],
            "bufferViews": views,
            "buffers": [{"byteLength": bin.len()}],
        });
        let mut text = serde_json::to_vec(&document).unwrap();
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + 8 + text.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&text);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }

    const TRIANGLE: [[f32; 3]; 3] = [[0.0, 1.0, -0.3], [0.1, 1.0, -0.3], [0.0, 1.1, -0.3]];
    // u right, v down the texture
    const UVS: [[f32; 2]; 3] = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];

    #[test]
    fn a_glb_triangle_is_mirrored_into_the_game_frame() {
        let bytes = glb(&["Root", "Spin_1"], TRIANGLE, UVS, [0, 1, 2]);
        let bones = ["Root", "Hip", "Spin_1"].map(String::from);
        let s = game_surface(&Glb::parse(&bytes, "t").unwrap(), &bones, "t").unwrap();
        assert_eq!(s.vertices[1].position, [-0.1, 1.0, -0.3]);
        // joint 1 is Spin_1, the rig's bone 2
        assert_eq!(s.vertices[0].bones[0], 2);
        assert_eq!(s.vertices[0].weights, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(s.indices, vec![0, 2, 1]);
        // the triangle faces +Z by its winding in glTF and, wound the other
        // way round, by the game's (check_basis passed on it)
        assert!(s.vertices[0].normal[2] > 0.99);
        // the tangent runs along +u: glTF +x, the game's -x
        assert!(s.vertices[0].tangent[0] < -0.99);
        // the binormal along +v, down the texture: -y
        assert!(s.vertices[0].binormal[1] < -0.99);
    }

    #[test]
    fn a_joint_the_rig_has_not_got_is_refused() {
        let bytes = glb(&["Root", "Tail"], TRIANGLE, UVS, [0, 1, 2]);
        let bones = ["Root", "Spin_1"].map(String::from);
        let err = game_surface(&Glb::parse(&bytes, "t").unwrap(), &bones, "t").unwrap_err();
        assert!(err.to_string().contains("Tail"), "{err}");
    }

    #[test]
    fn a_flat_colour_block_round_trips_and_a_ramp_keeps_its_ends() {
        let flat = [[100, 120, 60, 255]; 16];
        let b = bc1_block(&flat);
        let c0 = u16::from_le_bytes([b[0], b[1]]);
        let e = from565(c0);
        assert!((e[0] - 100.0).abs() < 5.0 && (e[1] - 120.0).abs() < 3.0);
        let mut ramp = [[0u8; 4]; 16];
        for (i, t) in ramp.iter_mut().enumerate() {
            *t = [(i * 16) as u8, (i * 16) as u8, (i * 16) as u8, 255];
        }
        let b = bc1_block(&ramp);
        let (c0, c1) = (
            u16::from_le_bytes([b[0], b[1]]),
            u16::from_le_bytes([b[2], b[3]]),
        );
        assert!(c0 > c1, "four-colour mode");
        assert!(from565(c0)[0] > 230.0 && from565(c1)[0] < 10.0);
        // the first texel (black) takes the dark end, the last the bright one
        let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        assert_eq!(bits & 3, 1);
        assert_eq!(bits >> 30, 0);
    }

    #[test]
    fn an_alpha_block_keeps_its_extremes() {
        let mut values = [128u8; 16];
        values[0] = 10;
        values[15] = 250;
        let b = bc3_alpha_block(values);
        assert_eq!((b[0], b[1]), (250, 10));
        let bits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
        assert_eq!(bits & 7, 1);
        assert_eq!((bits >> 45) & 7, 0);
    }

    #[test]
    fn a_map_gets_mips_down_to_four_texels() {
        let image = Image {
            width: 64,
            height: 64,
            rgba: vec![[128, 128, 255, 255]; 64 * 64],
        };
        let dds = encode(&image, Role::Normal, "t", 1024).unwrap();
        assert_eq!(&dds[84..88], b"DXT5");
        assert_eq!(u32::from_le_bytes(dds[28..32].try_into().unwrap()), 5);
        // 64, 32, 16, 8, 4: 256 + 64 + 16 + 4 + 1 blocks of 16 bytes, and the
        // container's trailing byte
        assert_eq!(dds.len(), 128 + 341 * 16 + 1);
        let too_big = Image {
            width: 2048,
            height: 2048,
            rgba: vec![[0; 4]; 2048 * 2048],
        };
        assert!(encode(&too_big, Role::Diffuse, "t", 1024).is_err());
    }
}
