// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{HashMap, VecDeque};

use anyhow::{Context, Result, anyhow, bail};
use encoding_rs::WINDOWS_1251;

const VALUE_SIZE: usize = 24;

#[derive(Clone, Debug, PartialEq)]
pub struct BinaryConfig {
    pub root: ConfigValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NamedValue {
    pub name: String,
    pub value: ConfigValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    Boolean(bool),
    Integer(i32),
    Float(f32),
    String(String),
    Float2([f32; 2]),
    Float3([f32; 3]),
    Float4([f32; 4]),
    Named(Vec<NamedValue>),
    Indexed(Vec<ConfigValue>),
}

#[derive(Clone, Copy, Debug)]
struct RawValue {
    data: u64,
    id: u64,
    id_crc: u32,
    kind: u16,
    count: u16,
}

impl BinaryConfig {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < VALUE_SIZE {
            bail!("binary config is shorter than one value");
        }
        let root = parse_value(bytes, read_raw(bytes, 0)?, 0)?;
        Ok(Self { root })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        encode_root(&self.root)
    }
}

impl ConfigValue {
    pub fn named(&self, key: &str) -> Option<&ConfigValue> {
        let Self::Named(values) = self else {
            return None;
        };
        values
            .iter()
            .find(|value| value.name == key)
            .map(|value| &value.value)
    }

    pub fn named_mut(&mut self, key: &str) -> Option<&mut ConfigValue> {
        let Self::Named(values) = self else {
            return None;
        };
        values
            .iter_mut()
            .find(|value| value.name == key)
            .map(|value| &mut value.value)
    }

    pub fn set_path(&mut self, path: &str, replacement: ConfigValue) -> Result<()> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
            bail!("empty component in patch path: {path}");
        }
        set_path_parts(self, &parts, replacement, path)
    }

    pub fn collect_strings<'a>(&'a self, output: &mut Vec<&'a str>) {
        match self {
            Self::String(value) => output.push(value),
            Self::Named(values) => {
                for value in values {
                    value.value.collect_strings(output);
                }
            }
            Self::Indexed(values) => {
                for value in values {
                    value.collect_strings(output);
                }
            }
            _ => {}
        }
    }
}

fn set_path_parts(
    current: &mut ConfigValue,
    parts: &[&str],
    replacement: ConfigValue,
    full_path: &str,
) -> Result<()> {
    let (part, tail) = parts
        .split_first()
        .with_context(|| format!("empty patch path: {full_path}"))?;
    match current {
        ConfigValue::Named(values) => {
            if tail.is_empty() {
                if let Some(field) = values.iter_mut().find(|value| value.name == *part) {
                    field.value = replacement;
                } else {
                    values.push(NamedValue {
                        name: (*part).to_owned(),
                        value: replacement,
                    });
                }
                return Ok(());
            }
            let child = values
                .iter_mut()
                .find(|value| value.name == *part)
                .with_context(|| format!("patch path does not exist: {full_path}"))?;
            set_path_parts(&mut child.value, tail, replacement, full_path)
        }
        ConfigValue::Indexed(values) => {
            let index: usize = part
                .parse()
                .with_context(|| format!("expected array index in patch path: {full_path}"))?;
            let child = values.get_mut(index).with_context(|| {
                format!("array index {index} is outside patch path: {full_path}")
            })?;
            if tail.is_empty() {
                *child = replacement;
                Ok(())
            } else {
                set_path_parts(child, tail, replacement, full_path)
            }
        }
        _ => bail!("patch path traverses a scalar at {part}: {full_path}"),
    }
}

fn parse_value(bytes: &[u8], raw: RawValue, depth: usize) -> Result<ConfigValue> {
    if depth > 256 {
        bail!("binary config nesting exceeds 256 tables");
    }
    Ok(match raw.kind {
        0 => ConfigValue::Boolean(raw.data != 0),
        1 => ConfigValue::Integer(raw.data as u32 as i32),
        2 => ConfigValue::Float(f32::from_bits(raw.data as u32)),
        3 | 4 => {
            let count = raw.count as usize;
            let table_offset = usize::try_from(raw.data)?;
            let table_size = count
                .checked_mul(VALUE_SIZE)
                .context("table size overflow")?;
            checked_slice(bytes, table_offset, table_size)?;
            if raw.kind == 3 {
                let mut values = Vec::with_capacity(count);
                for index in 0..count {
                    let child = read_raw(bytes, table_offset + index * VALUE_SIZE)?;
                    let name = read_cp1251_cstr(bytes, usize::try_from(child.id)?)
                        .with_context(|| format!("reading key for named child {index}"))?;
                    let expected_crc = crc32fast::hash(name.as_bytes());
                    if child.id_crc != expected_crc {
                        bail!(
                            "key CRC mismatch for {name:?}: stored {:08x}, expected {:08x}",
                            child.id_crc,
                            expected_crc
                        );
                    }
                    values.push(NamedValue {
                        name,
                        value: parse_value(bytes, child, depth + 1)?,
                    });
                }
                ConfigValue::Named(values)
            } else {
                let mut values = Vec::with_capacity(count);
                for index in 0..count {
                    values.push(parse_value(
                        bytes,
                        read_raw(bytes, table_offset + index * VALUE_SIZE)?,
                        depth + 1,
                    )?);
                }
                ConfigValue::Indexed(values)
            }
        }
        5 => {
            let offset = usize::try_from(raw.data)?;
            let count = raw.count as usize;
            if count == 0 {
                bail!("string value has zero length");
            }
            let data = checked_slice(bytes, offset, count)?;
            if data.last() != Some(&0) {
                bail!("string value is not NUL terminated");
            }
            decode_cp1251(&data[..data.len() - 1])?
        }
        6 => ConfigValue::Float2(read_floats::<2>(bytes, usize::try_from(raw.data)?)?),
        7 => ConfigValue::Float3(read_floats::<3>(bytes, usize::try_from(raw.data)?)?),
        8 => ConfigValue::Float4(read_floats::<4>(bytes, usize::try_from(raw.data)?)?),
        other => bail!("unsupported binary config value type {other}"),
    })
}

fn decode_cp1251(bytes: &[u8]) -> Result<ConfigValue> {
    let (decoded, _, errors) = WINDOWS_1251.decode(bytes);
    if errors {
        bail!("invalid Windows-1251 string");
    }
    Ok(ConfigValue::String(decoded.into_owned()))
}

fn read_cp1251_cstr(bytes: &[u8], offset: usize) -> Result<String> {
    let tail = bytes
        .get(offset..)
        .with_context(|| format!("string offset outside config: {offset}"))?;
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .context("unterminated config key")?;
    let (decoded, _, errors) = WINDOWS_1251.decode(&tail[..end]);
    if errors {
        bail!("invalid Windows-1251 config key");
    }
    Ok(decoded.into_owned())
}

fn read_floats<const N: usize>(bytes: &[u8], offset: usize) -> Result<[f32; N]> {
    let raw = checked_slice(bytes, offset, N * 4)?;
    Ok(std::array::from_fn(|index| {
        f32::from_le_bytes(raw[index * 4..index * 4 + 4].try_into().unwrap())
    }))
}

fn read_raw(bytes: &[u8], offset: usize) -> Result<RawValue> {
    let value = checked_slice(bytes, offset, VALUE_SIZE)?;
    Ok(RawValue {
        data: u64::from_le_bytes(value[0..8].try_into().unwrap()),
        id: u64::from_le_bytes(value[8..16].try_into().unwrap()),
        id_crc: u32::from_le_bytes(value[16..20].try_into().unwrap()),
        kind: u16::from_le_bytes(value[20..22].try_into().unwrap()),
        count: u16::from_le_bytes(value[22..24].try_into().unwrap()),
    })
}

fn checked_slice(bytes: &[u8], offset: usize, size: usize) -> Result<&[u8]> {
    let end = offset.checked_add(size).context("config offset overflow")?;
    bytes
        .get(offset..end)
        .with_context(|| format!("config range outside buffer: {offset}..{end}"))
}

fn encode_root(root: &ConfigValue) -> Result<Vec<u8>> {
    let count = count_values(root);
    if count > u16::MAX as usize {
        bail!("binary config contains too many values: {count}");
    }
    let mut output = vec![0_u8; count * VALUE_SIZE];
    let mut names = HashMap::<String, (u64, u32)>::new();
    let mut queue = VecDeque::new();
    queue.push_back((root, 0_u64, 0_u32));
    let mut index = 0;

    while let Some((value, id, id_crc)) = queue.pop_front() {
        let raw = match value {
            ConfigValue::Boolean(value) => RawValue {
                data: u64::from(*value),
                id,
                id_crc,
                kind: 0,
                count: 1,
            },
            ConfigValue::Integer(value) => RawValue {
                data: u64::from(*value as u32),
                id,
                id_crc,
                kind: 1,
                count: 4,
            },
            ConfigValue::Float(value) => RawValue {
                data: u64::from(value.to_bits()),
                id,
                id_crc,
                kind: 2,
                count: 4,
            },
            ConfigValue::String(value) => {
                let encoded = encode_cp1251(value)?;
                let offset = output.len() as u64;
                let count = u16::try_from(encoded.len() + 1)
                    .context("binary config string exceeds u16 length")?;
                output.extend_from_slice(&encoded);
                output.push(0);
                RawValue {
                    data: offset,
                    id,
                    id_crc,
                    kind: 5,
                    count,
                }
            }
            ConfigValue::Float2(values) => encode_vector(&mut output, values, id, id_crc, 6)?,
            ConfigValue::Float3(values) => encode_vector(&mut output, values, id, id_crc, 7)?,
            ConfigValue::Float4(values) => encode_vector(&mut output, values, id, id_crc, 8)?,
            ConfigValue::Named(values) => {
                let mut ordered: Vec<&NamedValue> = values.iter().collect();
                ordered.sort_by_key(|value| crc32fast::hash(value.name.as_bytes()));
                let data = ((index + queue.len() + 1) * VALUE_SIZE) as u64;
                for child in ordered {
                    let (child_id, child_crc) = intern_name(&mut output, &mut names, &child.name)?;
                    queue.push_back((&child.value, child_id, child_crc));
                }
                RawValue {
                    data,
                    id,
                    id_crc,
                    kind: 3,
                    count: u16::try_from(values.len())?,
                }
            }
            ConfigValue::Indexed(values) => {
                let data = ((index + queue.len() + 1) * VALUE_SIZE) as u64;
                for (child_index, child) in values.iter().enumerate() {
                    queue.push_back((child, child_index as u64, 0));
                }
                RawValue {
                    data,
                    id,
                    id_crc,
                    kind: 4,
                    count: u16::try_from(values.len())?,
                }
            }
        };
        write_raw(&mut output, index * VALUE_SIZE, raw)?;
        index += 1;
    }
    Ok(output)
}

fn encode_vector<const N: usize>(
    output: &mut Vec<u8>,
    values: &[f32; N],
    id: u64,
    id_crc: u32,
    kind: u16,
) -> Result<RawValue> {
    let offset = output.len() as u64;
    for value in values {
        output.extend_from_slice(&value.to_le_bytes());
    }
    Ok(RawValue {
        data: offset,
        id,
        id_crc,
        kind,
        count: u16::try_from(N * 4)?,
    })
}

fn intern_name(
    output: &mut Vec<u8>,
    names: &mut HashMap<String, (u64, u32)>,
    name: &str,
) -> Result<(u64, u32)> {
    if let Some(value) = names.get(name) {
        return Ok(*value);
    }
    let encoded = encode_cp1251(name)?;
    let offset = output.len() as u64;
    output.extend_from_slice(&encoded);
    output.push(0);
    let value = (offset, crc32fast::hash(name.as_bytes()));
    names.insert(name.to_owned(), value);
    Ok(value)
}

fn encode_cp1251(value: &str) -> Result<Vec<u8>> {
    let (encoded, _, errors) = WINDOWS_1251.encode(value);
    if errors {
        bail!("text cannot be represented in Windows-1251: {value:?}");
    }
    Ok(encoded.into_owned())
}

fn write_raw(output: &mut [u8], offset: usize, raw: RawValue) -> Result<()> {
    let slot = output
        .get_mut(offset..offset + VALUE_SIZE)
        .ok_or_else(|| anyhow!("binary value offset outside output"))?;
    slot[0..8].copy_from_slice(&raw.data.to_le_bytes());
    slot[8..16].copy_from_slice(&raw.id.to_le_bytes());
    slot[16..20].copy_from_slice(&raw.id_crc.to_le_bytes());
    slot[20..22].copy_from_slice(&raw.kind.to_le_bytes());
    slot[22..24].copy_from_slice(&raw.count.to_le_bytes());
    Ok(())
}

fn count_values(value: &ConfigValue) -> usize {
    1 + match value {
        ConfigValue::Named(values) => values.iter().map(|value| count_values(&value.value)).sum(),
        ConfigValue::Indexed(values) => values.iter().map(count_values).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_roundtrip_preserves_all_types() {
        let source = ConfigValue::Named(vec![
            NamedValue {
                name: "boolean".into(),
                value: ConfigValue::Boolean(true),
            },
            NamedValue {
                name: "integer".into(),
                value: ConfigValue::Integer(-7),
            },
            NamedValue {
                name: "float".into(),
                value: ConfigValue::Float(1.25),
            },
            NamedValue {
                name: "string".into(),
                value: ConfigValue::String("Проверка".into()),
            },
            NamedValue {
                name: "vector".into(),
                value: ConfigValue::Float3([1.0, 2.0, 3.0]),
            },
            NamedValue {
                name: "array".into(),
                value: ConfigValue::Indexed(vec![ConfigValue::Integer(1), ConfigValue::Integer(2)]),
            },
        ]);
        let bytes = BinaryConfig {
            root: source.clone(),
        }
        .to_bytes()
        .unwrap();
        let parsed = BinaryConfig::parse(&bytes).unwrap();

        // Named tables are emitted in key-CRC order.
        assert_eq!(
            parsed.root.named("string"),
            Some(&ConfigValue::String("Проверка".into()))
        );
        assert_eq!(
            parsed.root.named("vector"),
            Some(&ConfigValue::Float3([1.0, 2.0, 3.0]))
        );
        assert_eq!(
            parsed.root.named("array"),
            Some(&ConfigValue::Indexed(vec![
                ConfigValue::Integer(1),
                ConfigValue::Integer(2)
            ]))
        );
    }
}
