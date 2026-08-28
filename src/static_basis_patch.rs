use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::to_writer_pretty;

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::manifest::BuiltFile;
use crate::weapon::{prepare_empty_output, record_file};

#[derive(Debug, Serialize)]
struct StaticBasisPatchRecord {
    format_version: u32,
    asset_id: String,
    resource_prefix: String,
    files: Vec<BuiltFile>,
}

pub fn build_flat_basis_bundle(source_db: &Path, prefix: &str, output: &Path) -> Result<()> {
    build_bundle(source_db, prefix, output, PatchKind::Basis)
}

pub fn build_flat_height_bundle(source_db: &Path, prefix: &str, output: &Path) -> Result<()> {
    build_bundle(source_db, prefix, output, PatchKind::Height)
}

pub fn build_canonical_alpha_bundle(source_db: &Path, prefix: &str, output: &Path) -> Result<()> {
    build_bundle(source_db, prefix, output, PatchKind::CanonicalAlpha)
}

pub fn build_uv_scale_bundle(
    source_db: &Path,
    prefix: &str,
    scale: f32,
    output: &Path,
) -> Result<()> {
    if !scale.is_finite() || scale <= 0.0 {
        bail!("static UV scale must be finite and positive");
    }
    build_bundle(source_db, prefix, output, PatchKind::UvScale(scale))
}

#[derive(Clone, Copy)]
enum PatchKind {
    Basis,
    Height,
    CanonicalAlpha,
    UvScale(f32),
}

fn build_bundle(
    source_db: &Path,
    prefix: &str,
    output: &Path,
    patch_kind: PatchKind,
) -> Result<()> {
    validate_prefix(prefix)?;
    let mut source = ResourceDb::open(source_db)?;
    let paths = source
        .paths_with_prefix(prefix)
        .filter(|path| path.ends_with("/converted_model"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        bail!("no converted static models found below prefix {prefix:?}");
    }

    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let mut bytes = source
            .read(&path)
            .with_context(|| format!("reading converted static model {path}"))?;
        let source_label = match patch_kind {
            PatchKind::Basis => {
                flatten_static_basis(&mut bytes, &path)?;
                "diagnostic:flat-static-basis"
            }
            PatchKind::Height => {
                flatten_static_height(&mut bytes, &path)?;
                "diagnostic:flat-static-height"
            }
            PatchKind::CanonicalAlpha => {
                canonicalize_static_basis_alpha(&mut bytes, &path)?;
                "target-abi:canonical-static-basis-alpha"
            }
            PatchKind::UvScale(scale) => {
                scale_static_uvs(&mut bytes, &path, scale)?;
                "diagnostic:scaled-static-uv"
            }
        };
        let destination = safe_join(&overlay, &path)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, &bytes)?;
        files.push(record_file(&path, source_label, &bytes));
    }

    let record = StaticBasisPatchRecord {
        format_version: 1,
        asset_id: match patch_kind {
            PatchKind::Basis => "flat-static-vertex-basis-diagnostic",
            PatchKind::Height => "flat-static-vertex-height-diagnostic",
            PatchKind::CanonicalAlpha => "target-abi-static-vertex-basis-alpha-fix",
            PatchKind::UvScale(_) => "static-vertex-uv-scale-diagnostic",
        }
        .into(),
        resource_prefix: prefix.to_owned(),
        files,
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(())
}

fn scale_static_uvs(bytes: &mut [u8], path: &str, scale: f32) -> Result<()> {
    const DECLARATION_SIZE: usize = 48;
    const VERTEX_STRIDE: usize = 32;

    if read_u32(bytes, 0)? != 3 {
        bail!("{path} does not start with a static vertex chunk");
    }
    let vertex_chunk_size = usize::try_from(read_u32(bytes, 4)?)?;
    if vertex_chunk_size < DECLARATION_SIZE + 4 {
        bail!("{path} has a truncated static vertex chunk");
    }
    let vertex_count = usize::try_from(read_u32(bytes, 8 + DECLARATION_SIZE)?)?;
    let expected_size = DECLARATION_SIZE
        .checked_add(4)
        .and_then(|size| size.checked_add(vertex_count.checked_mul(VERTEX_STRIDE)?))
        .context("static vertex chunk size overflow")?;
    if vertex_chunk_size != expected_size || bytes.len() < 8 + expected_size {
        bail!("{path} has static vertex chunk size {vertex_chunk_size}, expected {expected_size}");
    }

    let vertices = 8 + DECLARATION_SIZE + 4;
    for index in 0..vertex_count {
        let vertex = vertices + index * VERTEX_STRIDE;
        for offset in [24, 28] {
            let scaled = f32::from_bits(read_u32(bytes, vertex + offset)?) * scale;
            if !scaled.is_finite() {
                bail!("{path} produced a non-finite scaled UV at vertex {index}");
            }
            write_u32(bytes, vertex + offset, scaled.to_bits())?;
        }
    }
    Ok(())
}

fn canonicalize_static_basis_alpha(bytes: &mut [u8], path: &str) -> Result<()> {
    const DECLARATION_SIZE: usize = 48;
    const VERTEX_STRIDE: usize = 32;

    if read_u32(bytes, 0)? != 3 {
        bail!("{path} does not start with a static vertex chunk");
    }
    let vertex_chunk_size = usize::try_from(read_u32(bytes, 4)?)?;
    if vertex_chunk_size < DECLARATION_SIZE + 4 {
        bail!("{path} has a truncated static vertex chunk");
    }
    let vertex_count = usize::try_from(read_u32(bytes, 8 + DECLARATION_SIZE)?)?;
    let expected_size = DECLARATION_SIZE
        .checked_add(4)
        .and_then(|size| size.checked_add(vertex_count.checked_mul(VERTEX_STRIDE)?))
        .context("static vertex chunk size overflow")?;
    if vertex_chunk_size != expected_size || bytes.len() < 8 + expected_size {
        bail!("{path} has static vertex chunk size {vertex_chunk_size}, expected {expected_size}");
    }

    let vertices = 8 + DECLARATION_SIZE + 4;
    for index in 0..vertex_count {
        let vertex = vertices + index * VERTEX_STRIDE;
        bytes[vertex + 19] = 255;
        bytes[vertex + 23] = 255;
    }
    Ok(())
}

fn flatten_static_height(bytes: &mut [u8], path: &str) -> Result<()> {
    const DECLARATION_SIZE: usize = 48;
    const VERTEX_STRIDE: usize = 32;

    if read_u32(bytes, 0)? != 3 {
        bail!("{path} does not start with a static vertex chunk");
    }
    let vertex_chunk_size = usize::try_from(read_u32(bytes, 4)?)?;
    if vertex_chunk_size < DECLARATION_SIZE + 4 {
        bail!("{path} has a truncated static vertex chunk");
    }
    let vertex_count = usize::try_from(read_u32(bytes, 8 + DECLARATION_SIZE)?)?;
    let expected_size = DECLARATION_SIZE
        .checked_add(4)
        .and_then(|size| size.checked_add(vertex_count.checked_mul(VERTEX_STRIDE)?))
        .context("static vertex chunk size overflow")?;
    if vertex_chunk_size != expected_size || bytes.len() < 8 + expected_size {
        bail!("{path} has static vertex chunk size {vertex_chunk_size}, expected {expected_size}");
    }

    let vertices = 8 + DECLARATION_SIZE + 4;
    for index in 0..vertex_count {
        let vertex = vertices + index * VERTEX_STRIDE;
        write_u32(bytes, vertex + 4, 0.0_f32.to_bits())?;
    }
    Ok(())
}

fn validate_prefix(prefix: &str) -> Result<()> {
    if prefix.is_empty()
        || prefix.starts_with('/')
        || prefix
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("resource prefix is not safe: {prefix:?}");
    }
    Ok(())
}

fn flatten_static_basis(bytes: &mut [u8], path: &str) -> Result<()> {
    const DECLARATION_SIZE: usize = 48;
    const VERTEX_STRIDE: usize = 32;

    if read_u32(bytes, 0)? != 3 {
        bail!("{path} does not start with a static vertex chunk");
    }
    let vertex_chunk_size = usize::try_from(read_u32(bytes, 4)?)?;
    if vertex_chunk_size < DECLARATION_SIZE + 4 {
        bail!("{path} has a truncated static vertex chunk");
    }
    let vertex_count = usize::try_from(read_u32(bytes, 8 + DECLARATION_SIZE)?)?;
    let expected_size = DECLARATION_SIZE
        .checked_add(4)
        .and_then(|size| size.checked_add(vertex_count.checked_mul(VERTEX_STRIDE)?))
        .context("static vertex chunk size overflow")?;
    if vertex_chunk_size != expected_size || bytes.len() < 8 + expected_size {
        bail!("{path} has static vertex chunk size {vertex_chunk_size}, expected {expected_size}");
    }

    let vertices = 8 + DECLARATION_SIZE + 4;
    for index in 0..vertex_count {
        let vertex = vertices + index * VERTEX_STRIDE;
        let tangent_alpha = bytes[vertex + 19];
        let binormal_alpha = bytes[vertex + 23];
        write_u32(bytes, vertex + 12, packed_basis([0.0, 1.0, 0.0], 127))?;
        write_u32(
            bytes,
            vertex + 16,
            packed_basis([1.0, 0.0, 0.0], tangent_alpha),
        )?;
        write_u32(
            bytes,
            vertex + 20,
            packed_basis([0.0, 0.0, -1.0], binormal_alpha),
        )?;
    }
    Ok(())
}

fn packed_basis(vector: [f32; 3], alpha: u8) -> u32 {
    let component = |value: f32| (((value + 1.0) * 127.5).floor() as i32).clamp(0, 255) as u32;
    ((alpha as u32) << 24)
        | (component(vector[0]) << 16)
        | (component(vector[1]) << 8)
        | component(vector[2])
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .with_context(|| format!("reading u32 at offset {offset}"))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<()> {
    let destination = bytes
        .get_mut(offset..offset + 4)
        .with_context(|| format!("writing u32 at offset {offset}"))?;
    destination.copy_from_slice(&value.to_le_bytes());
    Ok(())
}
