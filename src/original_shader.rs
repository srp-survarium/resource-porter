// SPDX-License-Identifier: GPL-3.0-or-later
//! Hash-pinned original shaders, copied unchanged into a map's own namespace.
//!
//! 0.11e and 0.26g shader files both contain a u32 source-record count followed
//! by 264-byte records and DXBC. Native converted_shader_loaded (0x3dc950)
//! computes 4 + count * 0x108; the complete donor file remains intact here.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Pixel,
    Vertex,
    Geometry,
}

impl Stage {
    fn extension(self) -> &'static str {
        match self {
            Self::Pixel => "ps",
            Self::Vertex => "vs",
            Self::Geometry => "gs",
        }
    }

    fn program_type(self) -> u32 {
        match self {
            Self::Pixel => 0,
            Self::Vertex => 1,
            Self::Geometry => 2,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "String")]
struct ShaderPath {
    path: String,
    family: String,
    stage: Stage,
}

impl TryFrom<String> for ShaderPath {
    type Error = String;

    fn try_from(path: String) -> std::result::Result<Self, Self::Error> {
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() != 4 || parts[..2] != ["shaders", "sm_4_0"] {
            return Err("original shader must name shaders/sm_4_0/family.ext/permutation".into());
        }
        let (family, extension) = parts[2].rsplit_once('.').ok_or("missing shader stage")?;
        if family.is_empty()
            || family.len() > 64
            || !family
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
            || parts[3].is_empty()
            || parts[3].len() > 128
            || !parts[3].bytes().all(|c| c.is_ascii_digit() || c == b'_')
        {
            return Err("invalid original shader family or permutation".into());
        }
        let stage = match extension {
            "ps" => Stage::Pixel,
            "vs" => Stage::Vertex,
            "gs" => Stage::Geometry,
            _ => return Err("original shader must be a pixel, vertex or geometry shader".into()),
        };
        let family = family.to_owned();
        Ok(Self {
            path,
            family,
            stage,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "String")]
struct Sha256Pin([u8; 32]);

impl TryFrom<String> for Sha256Pin {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("shader hash must be 64 lowercase hexadecimal characters".into());
        }
        let mut bytes = [0; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
                .map_err(|e| e.to_string())?;
        }
        Ok(Self(bytes))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "String")]
struct TextureName(String);

impl TryFrom<String> for TextureName {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 128
            || value.split('/').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
            })
        {
            return Err(
                "renderer texture must be a relative resource name without an extension".into(),
            );
        }
        Ok(Self(value))
    }
}

/// An original shader's fixed texture dependency, including its sampler options.
/// These exact bytes have been reviewed for the native DDS loader; no mip or
/// compression conversion is permitted on this explicit path.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalTexture {
    name: TextureName,
    dds_sha256: Sha256Pin,
    options_sha256: Sha256Pin,
}

pub struct TextureFile {
    pub source: String,
    pub destination: String,
    pub bytes: Vec<u8>,
}

impl OriginalTexture {
    pub fn files(
        &self,
        donor: &mut crate::ResourceDb,
        namespace: &crate::survarium_026::ResourceNamespace,
    ) -> Result<[TextureFile; 2]> {
        let mut read = |extension: &str, pin: &Sha256Pin| -> Result<TextureFile> {
            let source = format!("textures/{}.{}", self.name.0, extension);
            let bytes = donor.read(&source)?;
            ensure!(
                Sha256::digest(&bytes).as_slice() == pin.0,
                "{source}: original renderer texture hash mismatch"
            );
            Ok(TextureFile {
                destination: format!("textures/ported/{namespace}/{}.{}", self.name.0, extension),
                source,
                bytes,
            })
        };
        Ok([
            read("dds", &self.dds_sha256)?,
            read("options", &self.options_sha256)?,
        ])
    }
}

/// A safe suffix for explicitly selected programs from one source family.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "String")]
struct ShaderVariant(String);

impl TryFrom<String> for ShaderVariant {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 16
            || !value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        {
            return Err(
                "shader variant must contain 1..16 lowercase letters, digits or underscores".into(),
            );
        }
        Ok(Self(value))
    }
}

/// One exact original program, with a derived output path. An optional variant
/// gives distinct original programs independent native
/// families, so the runtime explicitly selects quality/feature combinations.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalShader {
    source: ShaderPath,
    #[serde(default)]
    variant: Option<ShaderVariant>,
    blob_sha256: Sha256Pin,
    dxbc_sha256: Sha256Pin,
}

impl OriginalShader {
    pub fn source(&self) -> &str {
        &self.source.path
    }

    /// A new family absent from the native shader mask takes 67 underscores.
    /// The chosen donor permutation is explicit; native configuration bits
    /// must not silently select another source program.
    pub fn destination(&self, namespace: &crate::survarium_026::ResourceNamespace) -> String {
        let variant = self
            .variant
            .as_ref()
            .map_or(String::new(), |v| format!("_{}", v.0));
        format!(
            "shaders/sm_4_0/{}_{namespace}{variant}.{}/{}",
            self.source.family,
            self.source.stage.extension(),
            "_".repeat(67)
        )
    }

    /// Check archive identity, the native-readable wrapper, DXBC bounds and
    /// shader stage before emitting the *whole* original file byte for byte.
    pub fn verify(&self, bytes: &[u8]) -> Result<()> {
        ensure!(
            Sha256::digest(bytes).as_slice() == self.blob_sha256.0,
            "{}: original shader blob hash mismatch",
            self.source()
        );
        let records = word(bytes, 0)? as usize;
        let offset = records
            .checked_mul(264)
            .and_then(|v| v.checked_add(4))
            .context("shader source-record count overflow")?;
        let code = bytes
            .get(offset..)
            .context("truncated shader source records")?;
        ensure!(
            code.get(..4) == Some(b"DXBC"),
            "missing DXBC after shader source records"
        );
        ensure!(
            word(code, 24)? as usize == code.len(),
            "shader DXBC length mismatch"
        );
        ensure!(
            Sha256::digest(code).as_slice() == self.dxbc_sha256.0,
            "{}: original shader bytecode hash mismatch",
            self.source()
        );
        let chunks = word(code, 28)? as usize;
        let table_end = chunks
            .checked_mul(4)
            .and_then(|v| v.checked_add(32))
            .context("shader chunk count overflow")?;
        ensure!(table_end <= code.len(), "truncated DXBC chunk table");
        let mut programs = Vec::new();
        for index in 0..chunks {
            let start = word(code, 32 + index * 4)? as usize;
            ensure!(start >= table_end, "DXBC chunk overlaps header");
            let header_end = start
                .checked_add(8)
                .context("shader chunk offset overflow")?;
            let header = code
                .get(start..header_end)
                .context("truncated DXBC chunk header")?;
            let length = word(header, 4)? as usize;
            let end = header_end
                .checked_add(length)
                .context("shader chunk length overflow")?;
            let chunk = code.get(header_end..end).context("truncated DXBC chunk")?;
            if &header[..4] == b"SHDR" || &header[..4] == b"SHEX" {
                programs.push(word(chunk, 0)?);
                ensure!(
                    (word(chunk, 4)? as u64) * 4 == length as u64,
                    "shader program token length mismatch"
                );
            }
        }
        ensure!(programs.len() == 1, "expected one DXBC shader program");
        ensure!(
            programs[0] >> 16 == self.source.stage.program_type(),
            "shader stage does not match path"
        );
        ensure!(programs[0] & 0xffff == 0x40, "expected shader model 4.0");
        Ok(())
    }
}

fn word(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .context("shader word offset overflow")?;
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..end)
            .context("truncated shader word")?
            .try_into()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(bytes: &[u8], stage: &str) -> OriginalShader {
        serde_json::from_value(serde_json::json!({
            "source": format!("shaders/sm_4_0/rain.{stage}/___"),
            "blob_sha256": format!("{:x}", Sha256::digest(bytes)),
            "dxbc_sha256": format!("{:x}", Sha256::digest(&bytes[268..])),
        }))
        .unwrap()
    }

    fn shader() -> Vec<u8> {
        // One 264-byte metadata record and a minimal, structurally valid PS4.
        let mut bytes = vec![0; 268 + 52];
        bytes[..4].copy_from_slice(&1u32.to_le_bytes());
        bytes[4..12].copy_from_slice(b"rain.ps\0");
        let code = &mut bytes[268..];
        code[..4].copy_from_slice(b"DXBC");
        for (offset, value) in [(24, 52u32), (28, 1), (32, 36), (40, 8), (44, 0x40), (48, 2)] {
            code[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        code[36..40].copy_from_slice(b"SHDR");
        bytes
    }

    #[test]
    fn imports_whole_original_file_and_derives_independent_native_path() {
        let bytes = shader();
        let import = fixture(&bytes, "ps");
        import.verify(&bytes).unwrap();
        let ns = "v011w".to_owned().try_into().unwrap();
        assert_eq!(
            import.destination(&ns),
            format!("shaders/sm_4_0/rain_v011w.ps/{}", "_".repeat(67))
        );
        assert_eq!(import.source(), "shaders/sm_4_0/rain.ps/___");
        let mut variant = fixture(&bytes, "ps");
        variant.variant = Some("q3_bokeh".to_owned().try_into().unwrap());
        assert_eq!(
            variant.destination(&ns),
            format!("shaders/sm_4_0/rain_v011w_q3_bokeh.ps/{}", "_".repeat(67))
        );
        variant.verify(&bytes).unwrap(); // The alias never modifies source bytes.
        let mut edited = bytes.clone();
        edited[8] ^= 1; // Even a metadata-only edit must break the whole-file pin.
        assert!(import.verify(&edited).is_err());
        let mut stale = fixture(&bytes, "ps");
        stale.dxbc_sha256 = Sha256Pin([0; 32]);
        assert!(stale.verify(&bytes).is_err());
    }

    #[test]
    fn refuses_wrong_stage_truncation_and_malformed_wrappers() {
        let bytes = shader();
        assert!(fixture(&bytes, "vs").verify(&bytes).is_err());
        for (offset, value) in [
            (0, u32::MAX),
            (268 + 24, 4000),
            (268 + 28, u32::MAX),
            (268 + 32, 0),
            (268 + 40, 1000),
            (268 + 44, 0x50),
            (268 + 48, 1),
        ] {
            let mut malformed = bytes.clone();
            malformed[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(
                fixture(&malformed, "ps").verify(&malformed).is_err(),
                "offset {offset}"
            );
        }
        for end in 0..bytes.len() {
            assert!(fixture(&bytes, "ps").verify(&bytes[..end]).is_err());
        }
    }

    #[test]
    fn parses_only_shader_resources_and_complete_hash_pins() {
        for path in [
            "textures/rain.dds",
            "shaders/sm_4_0/../___",
            "shaders/sm_4_0/rain.ps/../x",
            "shaders/sm_4_0/rain.cs/___",
            "shaders/sm_4_0/rain.ps/",
        ] {
            assert!(ShaderPath::try_from(path.to_owned()).is_err());
        }
        for digest in ["", "abcd", &"F".repeat(64), &"x".repeat(64)] {
            assert!(Sha256Pin::try_from(digest.to_owned()).is_err());
        }
        for name in [
            "",
            "/engine/rain",
            "engine/../rain",
            "engine//rain",
            "engine/rain.dds",
        ] {
            assert!(TextureName::try_from(name.to_owned()).is_err());
        }
        assert!(TextureName::try_from("engine/scroll_rain_0".to_owned()).is_ok());
        for variant in ["", "../q3", "q3.ps", "Q3", "a_very_long_variant"] {
            assert!(ShaderVariant::try_from(variant.to_owned()).is_err());
        }
    }
}
