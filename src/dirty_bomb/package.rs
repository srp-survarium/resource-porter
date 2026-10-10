// SPDX-License-Identifier: GPL-3.0-or-later
//! Fail-closed reader for the cooked UE3 package ABI used by Dirty Bomb.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use encoding_rs::WINDOWS_1252;
use serde::{Deserialize, Serialize};

const PACKAGE_TAG: u32 = 0x9e2a_83c1;
const PACKAGE_VERSION: u16 = 868;
const LICENSEE_VERSION: u16 = 9;
const ENGINE_VERSION: i32 = 8710;
const COOKED_CONTENT_VERSION: i32 = 137;
const COMPRESS_NONE: u32 = 0;
const COMPRESS_LZO: u32 = 2;
const RF_CLASS_DEFAULT_OBJECT: u64 = 0x0000_0000_0000_0200;
const RF_HAS_STACK: u64 = 0x0200_0000_0000_0000;
const MAX_TABLE_ENTRIES: usize = 2_000_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CompressedChunk {
    pub uncompressed_offset: usize,
    pub uncompressed_size: usize,
    pub compressed_offset: usize,
    pub compressed_size: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageSummary {
    pub version: u16,
    pub licensee_version: u16,
    pub total_header_size: usize,
    pub folder_name: String,
    pub package_flags: u32,
    pub name_count: usize,
    pub name_offset: usize,
    pub export_count: usize,
    pub export_offset: usize,
    pub import_count: usize,
    pub import_offset: usize,
    pub depends_offset: usize,
    pub engine_version: i32,
    pub cooked_content_version: i32,
    pub compression_flags: u32,
    pub compressed_chunks: Vec<CompressedChunk>,
    pub package_source: u32,
}

#[derive(Clone, Debug)]
pub struct ImportEntry {
    pub class_package: String,
    pub class_name: String,
    pub outer_index: i32,
    pub object_name: String,
}

#[derive(Clone, Debug)]
pub struct ExportEntry {
    pub class_index: i32,
    pub super_index: i32,
    pub outer_index: i32,
    pub object_name: String,
    pub archetype_index: i32,
    pub object_flags: u64,
    pub serial_size: usize,
    pub serial_offset: usize,
    pub export_flags: u32,
    pub class_name: String,
}

#[derive(Clone, Debug)]
pub struct Property<'a> {
    pub name: String,
    pub type_name: String,
    pub struct_name: Option<String>,
    pub enum_name: Option<String>,
    pub array_index: i32,
    pub bool_value: Option<bool>,
    pub payload: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct TaggedProperties<'a> {
    pub values: Vec<Property<'a>>,
    /// Absolute logical-package offset of the native class payload.
    pub native_offset: usize,
    pub native_data: &'a [u8],
}

/// A fully decompressed package and its object tables.
#[derive(Debug)]
pub struct Package {
    pub summary: PackageSummary,
    pub disk_size: usize,
    pub logical_size: usize,
    pub names: Vec<String>,
    pub imports: Vec<ImportEntry>,
    pub exports: Vec<ExportEntry>,
    logical: Vec<u8>,
}

impl Package {
    pub fn read(path: &Path) -> Result<Self> {
        let raw = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&raw, &path.display().to_string())
    }

    pub fn parse(raw: &[u8], label: &str) -> Result<Self> {
        let summary = parse_summary(raw, label)?;
        validate_summary(&summary, label)?;
        let logical = decompress_package(raw, &summary, label)?;

        let mut names_reader = Reader::at(&logical, summary.name_offset, label)?;
        let mut names = Vec::with_capacity(summary.name_count);
        for index in 0..summary.name_count {
            names.push(
                names_reader
                    .fstring()
                    .with_context(|| format!("{label}: name table entry {index}"))?,
            );
            names_reader.u64()?; // EObjectFlags
        }

        let mut imports_reader = Reader::at(&logical, summary.import_offset, label)?;
        let mut imports = Vec::with_capacity(summary.import_count);
        for index in 0..summary.import_count {
            let class_package = read_fname(&mut imports_reader, &names)
                .with_context(|| format!("{label}: import {index} class package"))?;
            let class_name = read_fname(&mut imports_reader, &names)
                .with_context(|| format!("{label}: import {index} class"))?;
            let outer_index = imports_reader.i32()?;
            let object_name = read_fname(&mut imports_reader, &names)
                .with_context(|| format!("{label}: import {index} object name"))?;
            imports.push(ImportEntry {
                class_package,
                class_name,
                outer_index,
                object_name,
            });
        }

        let mut exports_reader = Reader::at(&logical, summary.export_offset, label)?;
        let mut exports = Vec::with_capacity(summary.export_count);
        for index in 0..summary.export_count {
            let class_index = exports_reader.i32()?;
            let super_index = exports_reader.i32()?;
            let outer_index = exports_reader.i32()?;
            let object_name = read_fname(&mut exports_reader, &names)
                .with_context(|| format!("{label}: export {index} object name"))?;
            let archetype_index = exports_reader.i32()?;
            let object_flags = exports_reader.u64()?;
            let serial_size = exports_reader.nonnegative_i32("export serial size")?;
            let serial_offset = exports_reader.nonnegative_i32("export serial offset")?;
            let export_flags = exports_reader.u32()?;
            let generation_net_count = exports_reader.count("generation net object count")?;
            exports_reader.take(
                generation_net_count
                    .checked_mul(4)
                    .context("generation net object table size overflow")?,
            )?;
            exports_reader.take(16)?; // package GUID
            exports_reader.u32()?; // package flags
            if serial_size != 0 {
                logical
                    .get(serial_offset..serial_offset.saturating_add(serial_size))
                    .with_context(|| {
                        format!(
                            "{label}: export {index} serial range {serial_offset:#x}+{serial_size:#x} exceeds logical package"
                        )
                    })?;
            }
            exports.push(ExportEntry {
                class_index,
                super_index,
                outer_index,
                object_name,
                archetype_index,
                object_flags,
                serial_size,
                serial_offset,
                export_flags,
                class_name: String::new(),
            });
        }
        for index in 0..exports.len() {
            exports[index].class_name =
                object_name(class_index_at(&exports, index), &imports, &exports)
                    .with_context(|| format!("{label}: resolving export {index} class"))?;
        }

        Ok(Self {
            summary,
            disk_size: raw.len(),
            logical_size: logical.len(),
            names,
            imports,
            exports,
            logical,
        })
    }

    pub fn external_packages(&self) -> BTreeSet<String> {
        self.imports
            .iter()
            .filter(|import| import.class_name == "Package" && import.outer_index == 0)
            .map(|import| import.object_name.clone())
            .collect()
    }

    pub fn object_name(&self, index: i32) -> Result<String> {
        object_name(index, &self.imports, &self.exports)
    }

    pub fn export(&self, index: i32) -> Result<&ExportEntry> {
        if index <= 0 {
            bail!("object index {index} does not name a package export");
        }
        self.exports
            .get(resource_slot(index)?)
            .with_context(|| format!("export object index {index} is outside the export table"))
    }

    pub fn object_path(&self, index: i32) -> Result<String> {
        let mut components = Vec::new();
        let mut current = index;
        let mut seen = BTreeSet::new();
        while current != 0 {
            if !seen.insert(current) {
                bail!("object outer chain contains a cycle at index {current}");
            }
            if current < 0 {
                let item = self.imports.get(resource_slot(current)?).with_context(|| {
                    format!("import object index {current} is outside the import table")
                })?;
                components.push(item.object_name.clone());
                current = item.outer_index;
            } else {
                let item = self.exports.get(resource_slot(current)?).with_context(|| {
                    format!("export object index {current} is outside the export table")
                })?;
                components.push(item.object_name.clone());
                current = item.outer_index;
            }
            if components.len() > 1024 {
                bail!("object outer chain is implausibly deep");
            }
        }
        components.reverse();
        Ok(components.join("."))
    }

    /// Decode the standard UE3 UObject state frame, NetIndex and tagged
    /// properties. Native class payload after `None` is deliberately not read.
    pub fn properties(&self, export: &ExportEntry) -> Result<Vec<Property<'_>>> {
        Ok(self.tagged_properties(export)?.values)
    }

    /// Decode tagged UObject properties and retain the native class tail.
    pub fn tagged_properties(&self, export: &ExportEntry) -> Result<TaggedProperties<'_>> {
        if export.serial_size == 0 {
            return Ok(TaggedProperties {
                values: Vec::new(),
                native_offset: export.serial_offset,
                native_data: &[],
            });
        }
        let end = export
            .serial_offset
            .checked_add(export.serial_size)
            .context("export serial range overflow")?;
        let serial = self
            .logical
            .get(export.serial_offset..end)
            .context("export serial range exceeds logical package")?;
        let label = export.object_name.as_str();
        let mut reader = Reader::new(serial, label);
        if export.object_flags & RF_HAS_STACK != 0 {
            let node = reader.i32()?;
            reader.i32()?; // StateNode
            reader.u32()?; // ProbeMask
            reader.u16()?; // LatentAction
            let pushed_states = reader.count("pushed state count")?;
            reader.take(
                pushed_states
                    .checked_mul(12)
                    .context("pushed-state table size overflow")?,
            )?;
            if node != 0 {
                reader.i32()?; // State code offset
            }
        }
        // UObject::Serialize invokes UComponent::PreSerialize before the
        // NetIndex. Persistent component instances always carry their old
        // TemplateOwnerClass pointer; class-default components additionally
        // carry TemplateName. See Core/Src/UnCoreNative.cpp.
        if export.class_name.ends_with("Component") {
            reader.i32()?; // TemplateOwnerClass UObject index
            if export.object_flags & RF_CLASS_DEFAULT_OBJECT != 0 {
                read_fname(&mut reader, &self.names)?; // TemplateName
            }
        }
        reader.i32()?; // UObject NetIndex in cooked packages

        let mut properties = Vec::new();
        for _ in 0..100_000 {
            let name = read_fname(&mut reader, &self.names)?;
            if name == "None" {
                let native_offset = export
                    .serial_offset
                    .checked_add(reader.offset)
                    .context("native payload offset overflow")?;
                return Ok(TaggedProperties {
                    values: properties,
                    native_offset,
                    native_data: &serial[reader.offset..],
                });
            }
            let type_name = read_fname(&mut reader, &self.names)?;
            let size = reader.nonnegative_i32("property size")?;
            let array_index = reader.i32()?;
            let mut struct_name = None;
            let mut enum_name = None;
            let mut bool_value = None;
            match type_name.as_str() {
                "StructProperty" => struct_name = Some(read_fname(&mut reader, &self.names)?),
                "BoolProperty" => bool_value = Some(reader.u8()? != 0),
                "ByteProperty" => enum_name = Some(read_fname(&mut reader, &self.names)?),
                _ => {}
            }
            let payload = reader
                .take(size)
                .with_context(|| format!("{label}: property {name} ({type_name})"))?;
            properties.push(Property {
                name,
                type_name,
                struct_name,
                enum_name,
                array_index,
                bool_value,
                payload,
            });
        }
        bail!("{label}: more than 100000 tagged properties")
    }

    pub fn property_vector(&self, property: &Property<'_>) -> Option<[f32; 3]> {
        if property.type_name != "StructProperty"
            || property.struct_name.as_deref() != Some("Vector")
            || property.payload.len() != 12
        {
            return None;
        }
        Some([
            f32::from_le_bytes(property.payload[0..4].try_into().unwrap()),
            f32::from_le_bytes(property.payload[4..8].try_into().unwrap()),
            f32::from_le_bytes(property.payload[8..12].try_into().unwrap()),
        ])
    }

    pub fn property_rotator(&self, property: &Property<'_>) -> Option<[i32; 3]> {
        if property.type_name != "StructProperty"
            || property.struct_name.as_deref() != Some("Rotator")
            || property.payload.len() != 12
        {
            return None;
        }
        Some([
            i32::from_le_bytes(property.payload[0..4].try_into().unwrap()),
            i32::from_le_bytes(property.payload[4..8].try_into().unwrap()),
            i32::from_le_bytes(property.payload[8..12].try_into().unwrap()),
        ])
    }

    pub fn property_box_sphere_bounds(
        &self,
        property: &Property<'_>,
    ) -> Option<([f32; 3], [f32; 3])> {
        if property.type_name != "StructProperty"
            || property.struct_name.as_deref() != Some("BoxSphereBounds")
        {
            return None;
        }
        if property.payload.len() == 28 {
            let origin = float3(&property.payload[0..12]);
            let extent = float3(&property.payload[12..24]);
            return Some((origin, extent));
        }

        // BoxSphereBounds is not marked atomic in this branch, so tagged
        // properties persist its Origin, BoxExtent and SphereRadius members.
        let mut reader = Reader::new(property.payload, "BoxSphereBounds");
        let mut origin = None;
        let mut extent = None;
        for _ in 0..16 {
            let name = read_fname(&mut reader, &self.names).ok()?;
            if name == "None" {
                return origin.zip(extent);
            }
            let type_name = read_fname(&mut reader, &self.names).ok()?;
            let size = reader.nonnegative_i32("nested property size").ok()?;
            reader.i32().ok()?; // ArrayIndex
            let struct_name = (type_name == "StructProperty")
                .then(|| read_fname(&mut reader, &self.names).ok())
                .flatten();
            if type_name == "BoolProperty" {
                reader.u8().ok()?;
            } else if type_name == "ByteProperty" {
                read_fname(&mut reader, &self.names).ok()?;
            }
            let payload = reader.take(size).ok()?;
            if type_name == "StructProperty"
                && struct_name.as_deref() == Some("Vector")
                && payload.len() == 12
            {
                match name.as_str() {
                    "Origin" => origin = Some(float3(payload)),
                    "BoxExtent" => extent = Some(float3(payload)),
                    _ => {}
                }
            }
        }
        None
    }

    pub fn property_float(&self, property: &Property<'_>) -> Option<f32> {
        (property.type_name == "FloatProperty" && property.payload.len() == 4)
            .then(|| f32::from_le_bytes(property.payload.try_into().unwrap()))
    }

    pub fn property_linear_color(&self, property: &Property<'_>) -> Option<[f32; 4]> {
        if property.type_name != "StructProperty"
            || property.struct_name.as_deref() != Some("LinearColor")
            || property.payload.len() != 16
        {
            return None;
        }
        Some([
            f32::from_le_bytes(property.payload[0..4].try_into().unwrap()),
            f32::from_le_bytes(property.payload[4..8].try_into().unwrap()),
            f32::from_le_bytes(property.payload[8..12].try_into().unwrap()),
            f32::from_le_bytes(property.payload[12..16].try_into().unwrap()),
        ])
    }

    pub fn property_i32(&self, property: &Property<'_>) -> Option<i32> {
        (property.type_name == "IntProperty" && property.payload.len() == 4)
            .then(|| i32::from_le_bytes(property.payload.try_into().unwrap()))
    }

    pub fn property_byte(&self, property: &Property<'_>) -> Option<u8> {
        (property.type_name == "ByteProperty" && property.payload.len() == 1)
            .then(|| property.payload[0])
    }

    pub fn property_enum_value(&self, property: &Property<'_>) -> Result<Option<String>> {
        if property.type_name != "ByteProperty" || property.payload.len() != 8 {
            return Ok(None);
        }
        let mut reader = Reader::new(property.payload, &property.name);
        Ok(Some(read_fname(&mut reader, &self.names)?))
    }

    pub fn property_name(&self, property: &Property<'_>) -> Result<Option<String>> {
        if property.type_name != "NameProperty" || property.payload.len() != 8 {
            return Ok(None);
        }
        let mut reader = Reader::new(property.payload, &property.name);
        Ok(Some(read_fname(&mut reader, &self.names)?))
    }

    pub fn property_object(&self, property: &Property<'_>) -> Result<Option<i32>> {
        if !matches!(
            property.type_name.as_str(),
            "ObjectProperty" | "ComponentProperty" | "InterfaceProperty"
        ) || property.payload.len() != 4
        {
            return Ok(None);
        }
        Ok(Some(i32::from_le_bytes(
            property.payload.try_into().unwrap(),
        )))
    }

    /// Decode a cooked `TArray<UObject*>` payload.
    ///
    /// The caller is responsible for selecting an `ArrayProperty` whose UE3
    /// declaration is object-valued (for example
    /// `StaticMeshComponent.Materials`). The serialized payload itself carries
    /// only the count and package object indices.
    pub fn property_object_array(&self, property: &Property<'_>) -> Result<Option<Vec<i32>>> {
        if property.type_name != "ArrayProperty" {
            return Ok(None);
        }
        let count_bytes = property
            .payload
            .get(..4)
            .context("object-array property has no element count")?;
        let count = i32::from_le_bytes(count_bytes.try_into().unwrap());
        if count < 0 {
            bail!("object-array property has negative element count {count}");
        }
        let count = usize::try_from(count)?;
        let expected = 4_usize
            .checked_add(count.checked_mul(4).context("object-array size overflow")?)
            .context("object-array size overflow")?;
        if property.payload.len() != expected {
            bail!(
                "object-array property contains {} bytes for {count} elements, expected {expected}",
                property.payload.len()
            );
        }
        Ok(Some(
            property.payload[4..]
                .chunks_exact(4)
                .map(|bytes| i32::from_le_bytes(bytes.try_into().unwrap()))
                .collect(),
        ))
    }

    /// Decode `MaterialInstanceConstant.TextureParameterValues`.
    ///
    /// The struct is not marked immutable, so each array element is itself a
    /// tagged-property stream terminated by `None`.
    pub fn property_texture_parameters(
        &self,
        property: &Property<'_>,
    ) -> Result<Option<Vec<(String, i32)>>> {
        if property.type_name != "ArrayProperty" {
            return Ok(None);
        }
        let mut reader = Reader::new(property.payload, &property.name);
        let count = reader.count("texture parameter count")?;
        let mut values = Vec::with_capacity(count);
        for parameter_index in 0..count {
            let mut parameter_name = None;
            let mut parameter_value = None;
            for _ in 0..16 {
                let name = read_fname(&mut reader, &self.names)?;
                if name == "None" {
                    break;
                }
                let type_name = read_fname(&mut reader, &self.names)?;
                let size = reader.nonnegative_i32("texture parameter member size")?;
                reader.i32()?; // ArrayIndex
                match type_name.as_str() {
                    "StructProperty" => {
                        read_fname(&mut reader, &self.names)?;
                    }
                    "BoolProperty" => {
                        reader.u8()?;
                    }
                    "ByteProperty" => {
                        read_fname(&mut reader, &self.names)?;
                    }
                    _ => {}
                }
                let payload = reader.take(size)?;
                match (name.as_str(), type_name.as_str(), payload.len()) {
                    ("ParameterName", "NameProperty", 8) => {
                        let mut payload_reader = Reader::new(payload, "ParameterName");
                        parameter_name = Some(read_fname(&mut payload_reader, &self.names)?);
                    }
                    ("ParameterValue", "ObjectProperty", 4) => {
                        parameter_value = Some(i32::from_le_bytes(payload.try_into().unwrap()));
                    }
                    _ => {}
                }
            }
            match (parameter_name, parameter_value) {
                (Some(name), Some(value)) => values.push((name, value)),
                (None, _) => bail!("texture parameter {parameter_index} has no ParameterName"),
                (Some(_), None) => {}
            }
        }
        if !reader.is_empty() {
            bail!(
                "{} has {} trailing texture-parameter bytes",
                property.name,
                reader.remaining()
            );
        }
        Ok(Some(values))
    }

    /// Decode `MaterialInstanceConstant.ScalarParameterValues`.
    ///
    /// Like texture parameters, these are mutable UE3 structs and therefore
    /// serialize as one tagged-property stream per array element.
    pub fn property_scalar_parameters(
        &self,
        property: &Property<'_>,
    ) -> Result<Option<Vec<(String, f32)>>> {
        if property.type_name != "ArrayProperty" {
            return Ok(None);
        }
        let mut reader = Reader::new(property.payload, &property.name);
        let count = reader.count("scalar parameter count")?;
        let mut values = Vec::with_capacity(count);
        for parameter_index in 0..count {
            let mut parameter_name = None;
            let mut parameter_value = None;
            for _ in 0..16 {
                let name = read_fname(&mut reader, &self.names)?;
                if name == "None" {
                    break;
                }
                let type_name = read_fname(&mut reader, &self.names)?;
                let size = reader.nonnegative_i32("scalar parameter member size")?;
                reader.i32()?; // ArrayIndex
                match type_name.as_str() {
                    "StructProperty" => {
                        read_fname(&mut reader, &self.names)?;
                    }
                    "BoolProperty" => {
                        reader.u8()?;
                    }
                    "ByteProperty" => {
                        read_fname(&mut reader, &self.names)?;
                    }
                    _ => {}
                }
                let payload = reader.take(size)?;
                match (name.as_str(), type_name.as_str(), payload.len()) {
                    ("ParameterName", "NameProperty", 8) => {
                        let mut payload_reader = Reader::new(payload, "ParameterName");
                        parameter_name = Some(read_fname(&mut payload_reader, &self.names)?);
                    }
                    ("ParameterValue", "FloatProperty", 4) => {
                        parameter_value = Some(f32::from_le_bytes(payload.try_into().unwrap()));
                    }
                    _ => {}
                }
            }
            match (parameter_name, parameter_value) {
                (Some(name), Some(value)) => values.push((name, value)),
                (None, _) => bail!("scalar parameter {parameter_index} has no ParameterName"),
                (Some(_), None) => {}
            }
        }
        if !reader.is_empty() {
            bail!(
                "{} has {} trailing scalar-parameter bytes",
                property.name,
                reader.remaining()
            );
        }
        Ok(Some(values))
    }

    /// Decode `MaterialInstanceConstant.VectorParameterValues`.
    pub fn property_vector_parameters(
        &self,
        property: &Property<'_>,
    ) -> Result<Option<Vec<(String, [f32; 4])>>> {
        if property.type_name != "ArrayProperty" {
            return Ok(None);
        }
        let mut reader = Reader::new(property.payload, &property.name);
        let count = reader.count("vector parameter count")?;
        let mut values = Vec::with_capacity(count);
        for parameter_index in 0..count {
            let mut parameter_name = None;
            let mut parameter_value = None;
            for _ in 0..16 {
                let name = read_fname(&mut reader, &self.names)?;
                if name == "None" {
                    break;
                }
                let type_name = read_fname(&mut reader, &self.names)?;
                let size = reader.nonnegative_i32("vector parameter member size")?;
                reader.i32()?; // ArrayIndex
                let struct_name = match type_name.as_str() {
                    "StructProperty" => Some(read_fname(&mut reader, &self.names)?),
                    "BoolProperty" => {
                        reader.u8()?;
                        None
                    }
                    "ByteProperty" => {
                        read_fname(&mut reader, &self.names)?;
                        None
                    }
                    _ => None,
                };
                let payload = reader.take(size)?;
                match (
                    name.as_str(),
                    type_name.as_str(),
                    struct_name.as_deref(),
                    payload.len(),
                ) {
                    ("ParameterName", "NameProperty", _, 8) => {
                        let mut payload_reader = Reader::new(payload, "ParameterName");
                        parameter_name = Some(read_fname(&mut payload_reader, &self.names)?);
                    }
                    ("ParameterValue", "StructProperty", Some("LinearColor"), 16) => {
                        parameter_value = Some([
                            f32::from_le_bytes(payload[0..4].try_into().unwrap()),
                            f32::from_le_bytes(payload[4..8].try_into().unwrap()),
                            f32::from_le_bytes(payload[8..12].try_into().unwrap()),
                            f32::from_le_bytes(payload[12..16].try_into().unwrap()),
                        ]);
                    }
                    _ => {}
                }
            }
            match (parameter_name, parameter_value) {
                (Some(name), Some(value)) => values.push((name, value)),
                (None, _) => bail!("vector parameter {parameter_index} has no ParameterName"),
                (Some(_), None) => {}
            }
        }
        if !reader.is_empty() {
            bail!(
                "{} has {} trailing vector-parameter bytes",
                property.name,
                reader.remaining()
            );
        }
        Ok(Some(values))
    }

    pub fn property_string(&self, property: &Property<'_>) -> Result<Option<String>> {
        if property.type_name != "StrProperty" {
            return Ok(None);
        }
        let mut reader = Reader::new(property.payload, &property.name);
        let value = reader.fstring()?;
        if !reader.is_empty() {
            bail!(
                "{}: string property has {} trailing bytes",
                property.name,
                reader.remaining()
            );
        }
        Ok(Some(value))
    }
}

fn float3(payload: &[u8]) -> [f32; 3] {
    [
        f32::from_le_bytes(payload[0..4].try_into().unwrap()),
        f32::from_le_bytes(payload[4..8].try_into().unwrap()),
        f32::from_le_bytes(payload[8..12].try_into().unwrap()),
    ]
}

fn class_index_at(exports: &[ExportEntry], index: usize) -> i32 {
    exports[index].class_index
}

fn object_name(index: i32, imports: &[ImportEntry], exports: &[ExportEntry]) -> Result<String> {
    if index == 0 {
        return Ok("Class".to_owned());
    }
    let slot = resource_slot(index)?;
    if index < 0 {
        imports
            .get(slot)
            .map(|entry| entry.object_name.clone())
            .with_context(|| format!("import object index {index} is outside the import table"))
    } else {
        exports
            .get(slot)
            .map(|entry| entry.object_name.clone())
            .with_context(|| format!("export object index {index} is outside the export table"))
    }
}

fn resource_slot(index: i32) -> Result<usize> {
    let magnitude = index
        .checked_abs()
        .with_context(|| format!("invalid object index {index}"))?;
    usize::try_from(magnitude - 1).context("object index does not fit usize")
}

fn validate_summary(summary: &PackageSummary, label: &str) -> Result<()> {
    if summary.version != PACKAGE_VERSION || summary.licensee_version != LICENSEE_VERSION {
        bail!(
            "{label}: unsupported UE3 package ABI {}/{} (Dirty Bomb retail maps use {PACKAGE_VERSION}/{LICENSEE_VERSION})",
            summary.version,
            summary.licensee_version
        );
    }
    if summary.engine_version != ENGINE_VERSION
        || summary.cooked_content_version != COOKED_CONTENT_VERSION
    {
        bail!(
            "{label}: unsupported engine/cooked version {}/{} (expected {ENGINE_VERSION}/{COOKED_CONTENT_VERSION})",
            summary.engine_version,
            summary.cooked_content_version
        );
    }
    match summary.compression_flags {
        COMPRESS_NONE if summary.compressed_chunks.is_empty() => Ok(()),
        COMPRESS_LZO if !summary.compressed_chunks.is_empty() => Ok(()),
        flags => bail!(
            "{label}: unsupported or inconsistent compression flags {flags:#x} with {} chunks",
            summary.compressed_chunks.len()
        ),
    }
}

fn parse_summary(raw: &[u8], label: &str) -> Result<PackageSummary> {
    let mut reader = Reader::new(raw, label);
    let tag = reader.u32()?;
    if tag != PACKAGE_TAG {
        bail!("{label}: bad UE3 package tag {tag:#010x}");
    }
    let packed_version = reader.u32()?;
    let version = packed_version as u16;
    let licensee_version = (packed_version >> 16) as u16;
    let total_header_size = reader.nonnegative_i32("total header size")?;
    let folder_name = reader.fstring()?;
    let package_flags = reader.u32()?;
    let name_count = reader.table_count("name count")?;
    let name_offset = reader.nonnegative_i32("name offset")?;
    let export_count = reader.table_count("export count")?;
    let export_offset = reader.nonnegative_i32("export offset")?;
    let import_count = reader.table_count("import count")?;
    let import_offset = reader.nonnegative_i32("import offset")?;
    let depends_offset = reader.nonnegative_i32("depends offset")?;
    reader.i32()?; // cross-level reference offset
    reader.i32()?; // import GUID offset/count (licensee field)
    reader.i32()?; // export GUID offset/count (licensee field)
    reader.i32()?; // thumbnail table offset
    reader.take(16)?; // package GUID
    let generation_count = reader.count("generation count")?;
    reader.take(
        generation_count
            .checked_mul(12)
            .context("generation table size overflow")?,
    )?;
    let engine_version = reader.i32()?;
    let cooked_content_version = reader.i32()?;
    let compression_flags = reader.u32()?;
    let chunk_count = reader.count("compressed chunk count")?;
    let mut compressed_chunks = Vec::with_capacity(chunk_count);
    for _ in 0..chunk_count {
        compressed_chunks.push(CompressedChunk {
            uncompressed_offset: reader.nonnegative_i32("chunk uncompressed offset")?,
            uncompressed_size: reader.nonnegative_i32("chunk uncompressed size")?,
            compressed_offset: reader.nonnegative_i32("chunk compressed offset")?,
            compressed_size: reader.nonnegative_i32("chunk compressed size")?,
        });
    }
    let package_source = reader.u32()?;
    Ok(PackageSummary {
        version,
        licensee_version,
        total_header_size,
        folder_name,
        package_flags,
        name_count,
        name_offset,
        export_count,
        export_offset,
        import_count,
        import_offset,
        depends_offset,
        engine_version,
        cooked_content_version,
        compression_flags,
        compressed_chunks,
        package_source,
    })
}

fn decompress_package(raw: &[u8], summary: &PackageSummary, label: &str) -> Result<Vec<u8>> {
    if summary.compressed_chunks.is_empty() {
        return Ok(raw.to_vec());
    }
    let first = summary.compressed_chunks[0].uncompressed_offset;
    let prefix = raw
        .get(..first)
        .with_context(|| format!("{label}: uncompressed prefix ends beyond physical file"))?;
    let mut logical_size = summary.total_header_size.max(first);
    for chunk in &summary.compressed_chunks {
        logical_size = logical_size.max(
            chunk
                .uncompressed_offset
                .checked_add(chunk.uncompressed_size)
                .context("logical package size overflow")?,
        );
    }
    let mut logical = vec![0; logical_size];
    logical[..first].copy_from_slice(prefix);
    let mut expected_logical_offset = first;
    let mut previous_physical_end = 0usize;
    for (chunk_index, chunk) in summary.compressed_chunks.iter().enumerate() {
        if chunk.uncompressed_offset != expected_logical_offset {
            bail!(
                "{label}: compressed chunk {chunk_index} starts at logical {:#x}, expected {expected_logical_offset:#x}",
                chunk.uncompressed_offset
            );
        }
        let physical_end = chunk
            .compressed_offset
            .checked_add(chunk.compressed_size)
            .context("physical compressed chunk size overflow")?;
        if chunk_index != 0 && chunk.compressed_offset < previous_physical_end {
            bail!("{label}: compressed physical chunks overlap at chunk {chunk_index}");
        }
        let physical = raw
            .get(chunk.compressed_offset..physical_end)
            .with_context(|| {
                format!("{label}: compressed chunk {chunk_index} exceeds physical file")
            })?;
        let mut reader = Reader::new(physical, label);
        let tag = reader.u32()?;
        if tag != PACKAGE_TAG {
            bail!("{label}: compressed chunk {chunk_index} has bad container tag {tag:#010x}");
        }
        let block_size = reader.nonnegative_i32("compression block size")?;
        if block_size == 0 || block_size > 16 * 1024 * 1024 {
            bail!("{label}: implausible compression block size {block_size}");
        }
        let total_compressed = reader.nonnegative_i32("total compressed size")?;
        let total_uncompressed = reader.nonnegative_i32("total uncompressed size")?;
        if total_uncompressed != chunk.uncompressed_size {
            bail!(
                "{label}: chunk {chunk_index} expands to {total_uncompressed}, package table declares {}",
                chunk.uncompressed_size
            );
        }
        let block_count = total_uncompressed.div_ceil(block_size);
        let mut blocks = Vec::with_capacity(block_count);
        let mut described_compressed = 0usize;
        let mut described_uncompressed = 0usize;
        for _ in 0..block_count {
            let compressed = reader.nonnegative_i32("compressed block size")?;
            let uncompressed = reader.nonnegative_i32("uncompressed block size")?;
            if uncompressed == 0 || uncompressed > block_size {
                bail!("{label}: invalid compression block output size {uncompressed}");
            }
            described_compressed = described_compressed
                .checked_add(compressed)
                .context("compressed block sum overflow")?;
            described_uncompressed = described_uncompressed
                .checked_add(uncompressed)
                .context("uncompressed block sum overflow")?;
            blocks.push((compressed, uncompressed));
        }
        if described_compressed != total_compressed || described_uncompressed != total_uncompressed
        {
            bail!(
                "{label}: chunk {chunk_index} block table sums to {described_compressed}/{described_uncompressed}, summary says {total_compressed}/{total_uncompressed}"
            );
        }
        let mut output_offset = chunk.uncompressed_offset;
        for (block_index, (compressed_size, uncompressed_size)) in blocks.into_iter().enumerate() {
            let compressed = reader.take(compressed_size)?;
            let output_end = output_offset
                .checked_add(uncompressed_size)
                .context("block output offset overflow")?;
            let output = &mut logical[output_offset..output_end];
            let written = lzo::decompress_into(compressed, output)
                .with_context(|| format!("{label}: LZO chunk {chunk_index} block {block_index}"))?;
            if written != uncompressed_size {
                bail!(
                    "{label}: LZO chunk {chunk_index} block {block_index} wrote {written}, expected {uncompressed_size}"
                );
            }
            output_offset = output_end;
        }
        if !reader.is_empty() {
            bail!(
                "{label}: compressed chunk {chunk_index} has {} trailing bytes",
                reader.remaining()
            );
        }
        expected_logical_offset = chunk
            .uncompressed_offset
            .checked_add(chunk.uncompressed_size)
            .context("logical chunk end overflow")?;
        previous_physical_end = physical_end;
    }
    Ok(logical)
}

fn read_fname(reader: &mut Reader<'_, '_>, names: &[String]) -> Result<String> {
    let index = reader.i32()?;
    let number = reader.i32()?;
    if index < 0 || number < 0 {
        bail!("invalid FName index/number {index}/{number}");
    }
    let base = names
        .get(usize::try_from(index)?)
        .with_context(|| format!("FName index {index} is outside the name table"))?;
    if number == 0 {
        Ok(base.clone())
    } else {
        Ok(format!("{base}_{}", number - 1))
    }
}

#[derive(Clone, Copy, Debug)]
struct Reader<'a, 'label> {
    bytes: &'a [u8],
    offset: usize,
    label: &'label str,
}

impl<'a, 'label> Reader<'a, 'label> {
    fn new(bytes: &'a [u8], label: &'label str) -> Self {
        Self {
            bytes,
            offset: 0,
            label,
        }
    }

    fn at(bytes: &'a [u8], offset: usize, label: &'label str) -> Result<Self> {
        if offset > bytes.len() {
            bail!("{label}: table offset {offset:#x} exceeds logical package");
        }
        Ok(Self {
            bytes,
            offset,
            label,
        })
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .context("reader offset overflow")?;
        let bytes = self.bytes.get(self.offset..end).with_context(|| {
            format!(
                "{}: wanted {count} bytes at {:#x}, only {} remain",
                self.label,
                self.offset,
                self.remaining()
            )
        })?;
        self.offset = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn nonnegative_i32(&mut self, field: &str) -> Result<usize> {
        let value = self.i32()?;
        if value < 0 {
            bail!("{}: {field} is negative ({value})", self.label);
        }
        usize::try_from(value).context("nonnegative i32 does not fit usize")
    }

    fn count(&mut self, field: &str) -> Result<usize> {
        let count = self.nonnegative_i32(field)?;
        if count > MAX_TABLE_ENTRIES {
            bail!("{}: implausible {field} {count}", self.label);
        }
        Ok(count)
    }

    fn table_count(&mut self, field: &str) -> Result<usize> {
        self.count(field)
    }

    fn fstring(&mut self) -> Result<String> {
        let count = self.i32()?;
        if count == 0 {
            return Ok(String::new());
        }
        if count > 0 {
            let bytes = self.take(usize::try_from(count)?)?;
            let text = bytes
                .strip_suffix(&[0])
                .with_context(|| format!("{}: ANSI FString is not NUL terminated", self.label))?;
            let (decoded, _, malformed) = WINDOWS_1252.decode(text);
            if malformed {
                bail!("{}: malformed ANSI FString", self.label);
            }
            return Ok(decoded.into_owned());
        }
        let units = count
            .checked_neg()
            .with_context(|| format!("{}: invalid FString length {count}", self.label))?;
        let byte_count = usize::try_from(units)?
            .checked_mul(2)
            .context("UTF-16 FString byte size overflow")?;
        let bytes = self.take(byte_count)?;
        let mut text = Vec::with_capacity(usize::try_from(units)?);
        for pair in bytes.chunks_exact(2) {
            text.push(u16::from_le_bytes(pair.try_into().unwrap()));
        }
        if text.pop() != Some(0) {
            bail!("{}: UTF-16 FString is not NUL terminated", self.label);
        }
        String::from_utf16(&text).context("invalid UTF-16 FString")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lzo_decoder_accepts_reference_liblzo_block() {
        let block = [
            34, 104, 101, 108, 108, 111, 44, 32, 108, 122, 111, 32, 119, 111, 114, 108, 100, 33,
            17, 0, 0,
        ];
        let mut output = [0; 17];
        assert_eq!(lzo::decompress_into(&block, &mut output).unwrap(), 17);
        assert_eq!(&output, b"hello, lzo world!");
    }

    #[test]
    fn rejects_a_non_package() {
        let error = Package::parse(&[0; 32], "bad.umap")
            .unwrap_err()
            .to_string();
        assert!(error.contains("bad UE3 package tag"), "{error}");
    }

    #[test]
    fn reads_both_fstring_encodings() {
        let ansi = [4, 0, 0, 0, b'm', b'a', b'p', 0];
        assert_eq!(Reader::new(&ansi, "ansi").fstring().unwrap(), "map");
        let wide = [0xfd, 0xff, 0xff, 0xff, b'm', 0, b'a', 0, 0, 0];
        assert_eq!(Reader::new(&wide, "wide").fstring().unwrap(), "ma");
    }
}
