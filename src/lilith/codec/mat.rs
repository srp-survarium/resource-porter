//! The Jupiter EX `.Mat00` material instance (`LTMI` version 1).
//!
//! A material is a shader reference plus typed properties:
//!
//! ```text
//! magic "LTMI", u32 version = 1
//! shader   u16 length + bytes, e.g. shaders\rigid\Solid\specular.fx
//! u32 property count, then per property:
//!   u32 type, u16 name length + name, then the value:
//!     1  string   u16 length + bytes
//!     4  integer  u32
//!     5  float    f32
//! ```
//!
//! Confirmed against `Materials\Asylum\Brick_red_corner.Mat00`: eight
//! properties — `SurfaceFlags`, `DefaultWidth`, `DefaultHeight`,
//! `fMaxSpecularPower`, and the four texture bindings `tDiffuseMap`,
//! `tEmissiveMap`, `tSpecularMap`, `tNormalMap`, each a DDS path. Strings
//! carry no NUL terminator.
//!
//! The type list is what those archives use, not the format's full range; an
//! unknown type is a parse failure rather than a guess, because a skipped
//! value desynchronises every property after it.

use anyhow::{Context, Result, bail};

use crate::soc::codec::chunks::Cursor;

const MAGIC: &[u8; 4] = b"LTMI";
const VERSION: u32 = 1;

/// One property value.
#[derive(Clone, Debug, PartialEq)]
pub enum MatValue {
    String(String),
    Integer(u32),
    Float(f32),
    /// `vTintColor`: three floats.
    Vector3([f32; 3]),
    /// `vBaseColor`: four floats.
    Vector4([f32; 4]),
}

/// One parsed material instance.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    /// The effect the material renders with, e.g. `shaders\rigid\Solid\specular.fx`.
    pub shader: String,
    pub properties: Vec<(String, MatValue)>,
}

impl Material {
    /// A named string property, if present.
    pub fn string(&self, name: &str) -> Option<&str> {
        self.properties.iter().find_map(|(key, value)| {
            match (key.eq_ignore_ascii_case(name), value) {
                (true, MatValue::String(text)) => Some(text.as_str()),
                _ => None,
            }
        })
    }

    /// The diffuse texture path, when the material binds one.
    pub fn diffuse(&self) -> Option<&str> {
        self.string("tDiffuseMap")
    }

    pub fn normal(&self) -> Option<&str> {
        self.string("tNormalMap")
    }

    pub fn specular(&self) -> Option<&str> {
        self.string("tSpecularMap")
    }

    pub fn emissive(&self) -> Option<&str> {
        self.string("tEmissiveMap")
    }
}

/// Parse one `.Mat00`.
pub fn parse(bytes: &[u8], label: &str) -> Result<Material> {
    let mut cursor = Cursor::new(bytes, label);
    let magic = cursor.take(4)?;
    if magic != MAGIC {
        bail!("{label}: magic is {magic:02x?}, not \"LTMI\"");
    }
    let version = cursor.u32()?;
    if version != VERSION {
        bail!("{label}: LTMI version {version}, only {VERSION} is understood");
    }
    let shader = lt_string(&mut cursor, label)?;
    let count = cursor.u32()?;
    let mut properties = Vec::with_capacity(count as usize);
    for index in 0..count {
        let kind = cursor.u32()?;
        let name = lt_string(&mut cursor, label)?;
        let value = match kind {
            1 => MatValue::String(lt_string(&mut cursor, label)?),
            2 => MatValue::Vector3([cursor.f32()?, cursor.f32()?, cursor.f32()?]),
            3 => MatValue::Vector4([cursor.f32()?, cursor.f32()?, cursor.f32()?, cursor.f32()?]),
            4 => MatValue::Integer(cursor.u32()?),
            5 => MatValue::Float(cursor.f32()?),
            other => bail!(
                "{label}: property {index} ({name}) has type {other}, which no reviewed \
                 material uses"
            ),
        };
        properties.push((name, value));
    }
    if !cursor.is_empty() {
        bail!(
            "{label}: {} bytes remain after the declared {count} properties",
            cursor.remaining()
        );
    }
    Ok(Material { shader, properties })
}

/// A `u16 length + bytes` string, ASCII, no terminator.
fn lt_string(cursor: &mut Cursor, label: &str) -> Result<String> {
    let length = cursor.u16()? as usize;
    let bytes = cursor.take(length)?;
    let text = std::str::from_utf8(bytes).with_context(|| format!("{label}: non-ASCII string"))?;
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_string(bytes: &mut Vec<u8>, text: &str) {
        bytes.extend_from_slice(&(text.len() as u16).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
    }

    fn synthetic() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        push_string(&mut bytes, "shaders\\rigid\\Solid\\specular.fx");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        push_string(&mut bytes, "SurfaceFlags");
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.extend_from_slice(&5u32.to_le_bytes());
        push_string(&mut bytes, "DefaultWidth");
        bytes.extend_from_slice(&175.0f32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        push_string(&mut bytes, "tDiffuseMap");
        push_string(&mut bytes, "Materials\\Asylum\\BrickRed_corner_D.dds");
        bytes
    }

    #[test]
    fn parses_typed_properties() {
        let material = parse(&synthetic(), "test").unwrap();
        assert_eq!(material.shader, "shaders\\rigid\\Solid\\specular.fx");
        assert_eq!(
            material.properties[0],
            ("SurfaceFlags".to_owned(), MatValue::Integer(10))
        );
        assert_eq!(
            material.properties[1],
            ("DefaultWidth".to_owned(), MatValue::Float(175.0))
        );
        assert_eq!(
            material.diffuse(),
            Some("Materials\\Asylum\\BrickRed_corner_D.dds")
        );
        assert_eq!(material.normal(), None);
    }

    #[test]
    fn refuses_unknown_property_type() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        push_string(&mut bytes, "shaders\\rigid\\Solid\\diffuse.fx");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&9u32.to_le_bytes());
        push_string(&mut bytes, "Mystery");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let error = parse(&bytes, "test").unwrap_err().to_string();
        assert!(error.contains("type 9"), "{error}");
    }

    #[test]
    fn refuses_trailing_bytes() {
        let mut bytes = synthetic();
        bytes.push(0);
        let error = parse(&bytes, "test").unwrap_err().to_string();
        assert!(error.contains("remain"), "{error}");
    }
}
