// SPDX-License-Identifier: GPL-3.0-or-later
//! Shader programs 0.26g asks for and does not have, from the donor's own
//! blobs.
//!
//! A program's path is `shaders/sm_4_0/<family>/<key>`. A key has one
//! character for each shader macro, in the order the executable fills them
//! (`KeyOrder`: read from each build's executable, checked here against the
//! build's archives), `_` where the family does not read the macro; a
//! value of ten or more (the vertex input type has fifteen) takes two.
//! Which macros a family reads is data, `shaders/masks`; a family the file
//! does not list (the vertex shaders, keyed by the mesh alone) reads the
//! macros its own keys have a value at. Both builds compile the
//! same sources' descendants, so the donor's program for a 0.26g key is the
//! one at the same macro values, in the donor's order, with the macros only
//! the donor has at 0.
//!
//! A donor program is supplied only when 0.26g can feed it
//! (`Supplier::supply`): every texture and sampler it binds, and every
//! constant it reads, is one some program of the family that players have
//! binds or reads. A constant the donor declares longer (its
//! `specular_color_parameter` is a float4, 0.26g's a float3) passes when
//! the program's code reads no byte of it past 0.26g's length (`cb_reads`).
//! A program that needs more is listed, not shipped.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};

use crate::{
    ResourceDb,
    binary_config::{BinaryConfig, ConfigValue},
};

const ROOT: &str = "shaders/sm_4_0/";

/// The macros of a build in the order of a key's characters.
#[derive(Clone, Debug)]
pub struct KeyOrder(Vec<String>);

impl KeyOrder {
    /// `<place>\t<macro>` a line.
    pub fn parse(text: &str) -> Result<Self> {
        let mut names = Vec::new();
        for (index, line) in text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).enumerate() {
            let (place, name) = line.split_once('\t').with_context(|| format!("key order line `{line}`"))?;
            ensure!(place.trim().parse::<usize>().ok() == Some(index), "key order: place {place} at line {}", index + 1);
            names.push(name.trim().to_owned());
        }
        ensure!(!names.is_empty(), "an empty key order");
        Ok(Self(names))
    }

    /// The macros, in the key's order.
    pub fn names(&self) -> &[String] {
        &self.0
    }

    pub fn place(&self, name: &str) -> Option<usize> {
        self.0.iter().position(|n| n == name)
    }
}

/// The macros each family reads (`shaders/masks`).
#[derive(Clone, Debug, Default)]
pub struct Masks(BTreeMap<String, BTreeSet<String>>);

impl Masks {
    pub fn read(db: &mut ResourceDb) -> Result<Self> {
        let root = BinaryConfig::parse(&db.read("shaders/masks")?).context("parsing shaders/masks")?.root;
        // `<family>` is `<name>.<ps|vs|gs>`: a table a name, a list a stage.
        fn walk(value: ConfigValue, name: String, masks: &mut BTreeMap<String, BTreeSet<String>>) {
            match value {
                ConfigValue::Named(fields) => {
                    for field in fields {
                        let inner = if name.is_empty() { field.name } else { format!("{name}.{}", field.name) };
                        walk(field.value, inner, masks);
                    }
                }
                ConfigValue::Indexed(names) => {
                    masks.insert(name, names.into_iter().filter_map(|n| if let ConfigValue::String(s) = n { Some(s) } else { None }).collect());
                }
                _ => {}
            }
        }
        ensure!(matches!(root, ConfigValue::Named(_)), "shaders/masks is not a table");
        let mut masks = BTreeMap::new();
        walk(root, String::new(), &mut masks);
        Ok(Self(masks))
    }
}

/// A build's keys: its order and masks.
pub struct Keys {
    pub order: KeyOrder,
    pub masks: Masks,
}

/// The one macro whose value passes nine.
const WIDE: &str = "CONFIG_VERTEX_INPUT_TYPE";

impl Keys {
    /// A key of `family` as one value a macro, in the build's order: `_`
    /// where the family does not read the macro, a number where it does.
    pub fn values<'k>(&self, family: &str, key: &'k str) -> Result<Vec<&'k str>> {
        let mask = self.masks.0.get(family).with_context(|| format!("{family}: no mask"))?;
        let count = self.order.0.len();
        ensure!(key.is_ascii() && key.len() >= count, "{family}/{key}: shorter than the build's {count} macros");
        let extra = key.len() - count;
        let wide = self.order.place(WIDE).filter(|_| mask.contains(WIDE));
        ensure!(extra == 0 || wide.is_some(), "{family}/{key}: {extra} characters more than macros, and the family does not read {WIDE}");
        let mut values = Vec::with_capacity(count);
        let mut at = 0;
        for (place, name) in self.order.0.iter().enumerate() {
            let width = if Some(place) == wide { 1 + extra } else { 1 };
            let value = &key[at..at + width];
            at += width;
            let read = mask.contains(name);
            ensure!(
                if read { value.bytes().all(|c| c.is_ascii_digit()) } else { value == "_" },
                "{family}/{key}: `{value}` at {name}, which the family {}: the key order is not this build's",
                if read { "reads" } else { "does not read" }
            );
            values.push(value);
        }
        Ok(values)
    }

    /// Checks the order against the archive: every key of `family` reads as
    /// values of the family's macros. Returns how many.
    /// The build's keys: its order, its masks, and for each family
    /// `shaders/masks` does not list the macros its keys in `db` have a
    /// value at.
    pub fn read(order: KeyOrder, db: &mut ResourceDb) -> Result<Self> {
        let mut keys = Self { order, masks: Masks::read(db)? };
        let count = keys.order.0.len();
        let wide = keys.order.place(WIDE);
        let mut inferred: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for path in db.paths_with_prefix(ROOT).filter(|p| db.is_file(p)) {
            let Some((family, key)) = path[ROOT.len()..].split_once('/') else { continue };
            if keys.masks.0.contains_key(family) || !key.is_ascii() || key.len() < count {
                continue;
            }
            let extra = key.len() - count;
            let mask = inferred.entry(family.to_owned()).or_default();
            let mut at = 0;
            for (place, name) in keys.order.0.iter().enumerate() {
                let width = if Some(place) == wide && key.as_bytes()[at] != b'_' { 1 + extra } else { 1 };
                if &key[at..at + 1] != "_" {
                    mask.insert(name.clone());
                }
                at += width;
            }
        }
        keys.masks.0.extend(inferred);
        Ok(keys)
    }

    /// These keys with `family` reading `macros` too (a mask that grows:
    /// a client patch sets them).
    pub fn with_macros(&self, family: &str, macros: &[String]) -> Result<Keys> {
        let mut masks = self.masks.clone();
        let mask = masks.0.get_mut(family).with_context(|| format!("{family}: no mask"))?;
        for name in macros {
            ensure!(self.order.place(name).is_some(), "{name} is no macro of the build's key order");
            ensure!(mask.insert(name.clone()), "{family} reads {name} already");
        }
        Ok(Keys { order: self.order.clone(), masks })
    }

    /// Whether `family` reads the macro `name`.
    pub fn reads(&self, family: &str, name: &str) -> bool {
        self.masks.0.get(family).is_some_and(|mask| mask.contains(name))
    }

    /// The families the build has keys' macros for.
    pub fn families(&self) -> impl Iterator<Item = &str> {
        self.masks.0.keys().map(String::as_str)
    }

    pub fn check_family(&self, db: &ResourceDb, family: &str) -> Result<usize> {
        let prefix = format!("{ROOT}{family}/");
        let mut fit = 0;
        for path in db.paths_with_prefix(&prefix).filter(|p| db.is_file(p)) {
            self.values(family, &path[prefix.len()..])?;
            fit += 1;
        }
        Ok(fit)
    }
}

/// The donor's key for the target key `key` of `family`.
pub fn translate(key: &str, family: &str, target: &Keys, donor: &Keys) -> Result<String> {
    let values = target.values(family, key)?;
    let target_mask = &target.masks.0[family];
    let donor_mask = donor.masks.0.get(family).with_context(|| format!("{family}: the donor has no mask for it"))?;
    // A macro 0.26g sets that the donor's family does not read: no program of
    // the donor's is that program.
    for name in target_mask {
        if let Some(at) = target.order.place(name)
            && values[at] != "0"
            && !donor_mask.contains(name)
        {
            bail!("{name} is {} and the donor's {family} does not read it", values[at]);
        }
    }
    let mut out = String::new();
    for name in &donor.order.0 {
        out.push_str(if !donor_mask.contains(name) {
            "_"
        } else {
            match target.order.place(name) {
                Some(from) if target_mask.contains(name) => values[from],
                // A macro only the donor's family reads is off.
                _ => "0",
            }
        });
    }
    Ok(out)
}

/// A constant a program reads: where its reflection puts it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Constant {
    pub name: String,
    /// The slot its constant buffer is bound at.
    pub slot: u32,
    /// Its first byte in the buffer, and its length.
    pub start: u32,
    pub size: u32,
}

/// What a program reads: its constants, the textures and samplers it binds,
/// and the bytes of each constant buffer its code reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reads {
    pub constants: BTreeSet<Constant>,
    pub bound: BTreeSet<String>,
    /// (slot, byte) of every component read; none when the code indexes a
    /// buffer by a computed element.
    pub bytes: Option<BTreeSet<(u32, u32)>>,
}

fn u32_at(bytes: &[u8], at: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(bytes.get(at..at + 4).context("a shader blob ends early")?.try_into().expect("four bytes")) as usize)
}

fn name_at(bytes: &[u8], at: usize) -> Result<String> {
    let tail = bytes.get(at..).context("a name outside the reflection chunk")?;
    let end = tail.iter().position(|b| *b == 0).context("an unterminated name")?;
    Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
}

/// The DXBC container inside a blob: after a count and that many 264-byte
/// include records.
fn dxbc(blob: &[u8]) -> Result<&[u8]> {
    let includes = u32_at(blob, 0)?;
    let code = blob.get(4 + includes * 264..).context("a shader blob ends in its include records")?;
    ensure!(code.starts_with(b"DXBC"), "a shader blob holds no DXBC");
    Ok(code)
}

fn chunk<'a>(dxbc: &'a [u8], tag: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    for index in 0..u32_at(dxbc, 28)? {
        let at = u32_at(dxbc, 32 + 4 * index)?;
        if dxbc.get(at..at + 4) == Some(tag) {
            let size = u32_at(dxbc, at + 4)?;
            return Ok(Some(dxbc.get(at + 8..at + 8 + size).context("a chunk runs past the blob")?));
        }
    }
    Ok(None)
}

/// A blob's code: its DXBC chunks by tag, without the include records
/// before it (each source file's path and time) and the container's
/// checksum. Two builds' programs with the same code are one program.
pub fn code(blob: &[u8]) -> Result<BTreeMap<[u8; 4], Vec<u8>>> {
    let container = dxbc(blob)?;
    let mut out = BTreeMap::new();
    for index in 0..u32_at(container, 28)? {
        let at = u32_at(container, 32 + 4 * index)?;
        let tag: [u8; 4] = container.get(at..at + 4).context("a chunk outside the blob")?.try_into().expect("four bytes");
        let size = u32_at(container, at + 4)?;
        out.insert(tag, container.get(at + 8..at + 8 + size).context("a chunk runs past the blob")?.to_vec());
    }
    Ok(out)
}

/// What the blob reads: its reflection (`RDEF`, shader model 4) for the
/// names, its code for the bytes. A constant counts when the compiler marked
/// it used.
pub fn reads(blob: &[u8]) -> Result<Reads> {
    let container = dxbc(blob)?;
    let rdef = chunk(container, b"RDEF")?.context("a shader blob has no reflection")?;
    let (buffers, buffers_at, bindings, bindings_at) = (u32_at(rdef, 0)?, u32_at(rdef, 4)?, u32_at(rdef, 8)?, u32_at(rdef, 12)?);
    let mut out = Reads::default();
    let mut slots = BTreeMap::new();
    for b in 0..bindings {
        let at = bindings_at + 32 * b;
        const CBUFFER: usize = 0;
        let name = name_at(rdef, u32_at(rdef, at)?)?;
        if u32_at(rdef, at + 4)? == CBUFFER {
            slots.insert(name, u32_at(rdef, at + 20)? as u32);
        } else {
            out.bound.insert(name);
        }
    }
    for b in 0..buffers {
        let at = buffers_at + 24 * b;
        let buffer = name_at(rdef, u32_at(rdef, at)?)?;
        let (variables, variables_at) = (u32_at(rdef, at + 4)?, u32_at(rdef, at + 8)?);
        for v in 0..variables {
            let var = variables_at + 24 * v;
            const USED: usize = 2;
            if u32_at(rdef, var + 12)? & USED != 0 {
                out.constants.insert(Constant {
                    name: name_at(rdef, u32_at(rdef, var)?)?,
                    slot: *slots.get(&buffer).with_context(|| format!("constant buffer {buffer} is not bound"))?,
                    start: u32_at(rdef, var + 4)? as u32,
                    size: u32_at(rdef, var + 8)? as u32,
                });
            }
        }
    }
    out.bytes = match chunk(container, b"SHDR")? {
        Some(code) => cb_reads(code)?,
        None => None,
    };
    Ok(out)
}

/// The bytes of constant buffers the code (`SHDR`, shader model 4) reads:
/// (slot, byte) for each component of each `cb<slot>[<element>]` operand.
/// None when an operand's element is computed.
pub fn cb_reads(code: &[u8]) -> Result<Option<BTreeSet<(u32, u32)>>> {
    let token = |at: usize| -> Result<u32> { Ok(u32_at(code, 4 * at)? as u32) };
    let length = token(1)? as usize;
    ensure!(length * 4 <= code.len(), "the code chunk is shorter than it says");
    let mut reads = BTreeSet::new();
    let mut at = 2;
    while at < length {
        let op = token(at)?;
        const CUSTOM_DATA: u32 = 53;
        let opcode = op & 0x7ff;
        let size = if opcode == CUSTOM_DATA { token(at + 1)? as usize } else { ((op >> 24) & 0x7f) as usize };
        ensure!(size > 0 && at + size <= length, "an instruction of {size} tokens at {at}");
        // Declarations hold more than operands (88 dcl_resource to 106
        // dcl_global_flags); they read nothing.
        if opcode != CUSTOM_DATA && !(88..=106).contains(&opcode) {
            let mut cursor = at + 1;
            let mut extended = op >> 31 != 0;
            while extended {
                extended = token(cursor)? >> 31 != 0;
                cursor += 1;
            }
            while cursor < at + size {
                if !operand(code, &mut cursor, &mut reads)? {
                    return Ok(None);
                }
            }
            ensure!(cursor == at + size, "an instruction's operands end at {cursor}, not {}", at + size);
        }
        at += size;
    }
    Ok(Some(reads))
}

/// Walks one operand; false when it indexes a constant buffer by a computed
/// element.
fn operand(code: &[u8], cursor: &mut usize, reads: &mut BTreeSet<(u32, u32)>) -> Result<bool> {
    let token = |at: usize| -> Result<u32> { Ok(u32_at(code, 4 * at)? as u32) };
    let first = token(*cursor)?;
    *cursor += 1;
    let mut extended = first >> 31 != 0;
    while extended {
        extended = token(*cursor)? >> 31 != 0;
        *cursor += 1;
    }
    let kind = (first >> 12) & 0xff;
    let components = first & 3;
    const IMMEDIATE32: u32 = 4;
    const IMMEDIATE64: u32 = 5;
    const CONSTANT_BUFFER: u32 = 8;
    if kind == IMMEDIATE32 || kind == IMMEDIATE64 {
        let values = if components == 2 { 4 } else { 1 };
        *cursor += values * if kind == IMMEDIATE64 { 2 } else { 1 };
        return Ok(true);
    }
    let dimension = (first >> 20) & 3;
    let mut index = [None; 3];
    let mut whole = true;
    for (d, slot) in index.iter_mut().enumerate().take(dimension as usize) {
        match (first >> (22 + 3 * d)) & 7 {
            0 => {
                *slot = Some(token(*cursor)?);
                *cursor += 1;
            }
            1 => *cursor += 2,
            // Relative, and an immediate plus relative: another operand.
            representation @ (2 | 3 | 4) => {
                *cursor += match representation {
                    2 => 0,
                    3 => 1,
                    _ => 2,
                };
                whole &= operand(code, cursor, reads)?;
                if kind == CONSTANT_BUFFER {
                    whole = false;
                }
            }
            other => bail!("an operand index of representation {other}"),
        }
    }
    if kind == CONSTANT_BUFFER && whole {
        let (Some(slot), Some(element)) = (index[0], index[1]) else { bail!("a constant buffer operand without its slot and element") };
        let selected: Vec<u32> = match (components, (first >> 2) & 3) {
            // A mask of four, a swizzle of four, one component.
            (2, 0) => (0..4).filter(|c| first >> (4 + c) & 1 != 0).collect(),
            (2, 1) => (0..4).map(|c| first >> (4 + 2 * c) & 3).collect(),
            (2, 2) => vec![first >> 4 & 3],
            _ => (0..4).collect(),
        };
        reads.extend(selected.into_iter().map(|c| (slot, 16 * element + 4 * c)));
    }
    Ok(whole)
}

/// What supplying every program of the donor's came to, by family.
#[derive(Debug, Default, serde::Serialize)]
pub struct FamilySupply {
    /// The donor's programs players lack, in 0.26g's keys.
    pub lacking: usize,
    pub supplied: usize,
    /// Why the others were not, with how many.
    pub not_supplied: BTreeMap<String, usize>,
}

/// A program asked for, and what came of it.
#[derive(Debug, serde::Serialize)]
pub struct Supply {
    pub path: String,
    /// The donor's program taken for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub donor: Option<String>,
    /// Why none was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_supplied: Option<String>,
}

/// Supplies the programs of `wanted` (0.26g paths) from the donor.
pub struct Supplier<'a> {
    pub target: &'a Keys,
    pub donor: &'a Keys,
    /// What the programs players have read, by family.
    family_reads: BTreeMap<String, Known>,
}

/// What a family's programs read between them: each constant's longest
/// declared size, and the textures and samplers.
#[derive(Default)]
pub struct Known {
    constants: BTreeMap<String, u32>,
    bound: BTreeSet<String>,
}

impl Known {
    /// What every program of `family` in `db` reads.
    pub fn read(db: &mut ResourceDb, family: &str) -> Result<Self> {
        let prefix = format!("{ROOT}{family}/");
        let paths: Vec<String> = db.paths_with_prefix(&prefix).filter(|p| db.is_file(p)).map(str::to_owned).collect();
        ensure!(!paths.is_empty(), "{family}: players have no program of the family");
        let mut all = Known::default();
        for path in paths {
            let one = reads(&db.read(&path)?).with_context(|| path.clone())?;
            for constant in one.constants {
                let longest = all.constants.entry(constant.name).or_default();
                *longest = (*longest).max(constant.size);
            }
            all.bound.extend(one.bound);
        }
        Ok(all)
    }

    /// A texture or sampler the family may bind that nothing sets: it reads
    /// 0 in the donor's draw and in 0.26g's alike.
    pub fn allow_unbound(&mut self, name: &str) {
        self.bound.insert(name.to_owned());
    }

    /// A constant a client patch sets for the family at `size` bytes.
    pub fn fill(&mut self, name: &str, size: u32) {
        let longest = self.constants.entry(name.to_owned()).or_default();
        *longest = (*longest).max(size);
    }

    /// Why 0.26g cannot feed the donor's `blob` (at `donor_path`), or None:
    /// every texture and sampler it binds and every constant it reads is
    /// one the family's programs bind or read, a constant declared longer
    /// read no further than they fill.
    pub fn refuses(&self, family: &str, donor_path: &str, blob: &[u8]) -> Result<Option<String>> {
        let needs = reads(blob).with_context(|| donor_path.to_owned())?;
        let mut constants = Vec::new();
        for constant in &needs.constants {
            match self.constants.get(&constant.name) {
                None => constants.push(format!("{} (not one of 0.26g's)", constant.name)),
                Some(filled) if *filled >= constant.size => {}
                // Longer than 0.26g fills: its code must stay within what is filled.
                Some(filled) => {
                    let past = constant.start + filled..constant.start + constant.size;
                    match &needs.bytes {
                        Some(bytes) if !bytes.iter().any(|(slot, byte)| *slot == constant.slot && past.contains(byte)) => {}
                        Some(_) => constants.push(format!("{} (read past the {filled} bytes 0.26g fills)", constant.name)),
                        None => constants.push(format!("{} ({} bytes for 0.26g's {filled}, and the code indexes its buffer)", constant.name, constant.size)),
                    }
                }
            }
        }
        let bound: Vec<&String> = needs.bound.difference(&self.bound).collect();
        Ok((!constants.is_empty() || !bound.is_empty())
            .then(|| format!("{donor_path} reads what no {family} program of players' does: constants {constants:?}, textures and samplers {bound:?}")))
    }
}

impl<'a> Supplier<'a> {
    pub fn new(target: &'a Keys, donor: &'a Keys) -> Self {
        Self { target, donor, family_reads: BTreeMap::new() }
    }

    fn family(&mut self, shipped: &mut ResourceDb, family: &str) -> Result<&Known> {
        if !self.family_reads.contains_key(family) {
            self.target.check_family(shipped, family)?;
            self.family_reads.insert(family.to_owned(), Known::read(shipped, family)?);
        }
        Ok(&self.family_reads[family])
    }

    /// Every program of the donor's in the families `only` that players lack
    /// and 0.26g can feed, at its 0.26g path: whatever key 0.26g computes for the donor's own
    /// materials and meshes is one the donor computed too. A family whose
    /// keys do not read as the order's (players' forward programs, since
    /// patch 14 added a macro to their mask) is left alone and said so.
    pub fn supply_all(
        &mut self,
        shipped: &mut ResourceDb,
        donor_db: &mut ResourceDb,
        only: &[String],
    ) -> Result<(BTreeMap<String, Vec<u8>>, BTreeMap<String, FamilySupply>, BTreeMap<String, String>)> {
        let mut blobs = BTreeMap::new();
        let mut by_family = BTreeMap::new();
        let mut skipped = BTreeMap::new();
        for family in only {
            ensure!(self.donor.masks.0.contains_key(family) && self.target.masks.0.contains_key(family), "{family}: not a shader family of both builds");
        }
        let families: Vec<String> = only.to_vec();
        for family in families {
            if let Err(error) = self.target.check_family(shipped, &family).and_then(|_| self.donor.check_family(donor_db, &family)) {
                skipped.insert(family, format!("{error:#}"));
                continue;
            }
            let prefix = format!("{ROOT}{family}/");
            let theirs: Vec<String> = donor_db.paths_with_prefix(&prefix).filter(|p| donor_db.is_file(p)).map(str::to_owned).collect();
            let mut count = FamilySupply::default();
            for path in theirs {
                // The donor's key in 0.26g's order; one with a macro of the
                // donor's own on is no key 0.26g computes.
                let Ok(key) = translate(&path[prefix.len()..], &family, self.donor, self.target) else { continue };
                let wanted = format!("{prefix}{key}");
                if shipped.is_file(&wanted) || blobs.contains_key(&wanted) {
                    continue;
                }
                count.lacking += 1;
                let (supply, blob) = self.supply(shipped, donor_db, &wanted)?;
                match blob {
                    // The round trip must name the same donor program.
                    Some(blob) if supply.donor.as_deref() == Some(path.as_str()) => {
                        count.supplied += 1;
                        blobs.insert(wanted, blob);
                    }
                    Some(_) => *count.not_supplied.entry("another donor program has the same 0.26g key".into()).or_default() += 1,
                    None => {
                        let why = supply.not_supplied.unwrap_or_default();
                        let why = why.split_once(" reads what ").map_or(why.clone(), |(_, w)| format!("reads what {w}"));
                        *count.not_supplied.entry(why).or_default() += 1;
                    }
                }
            }
            if count.lacking > 0 {
                by_family.insert(family, count);
            }
        }
        Ok((blobs, by_family, skipped))
    }

    /// The donor's blob for the 0.26g program at `path`, or why there is none.
    pub fn supply(&mut self, shipped: &mut ResourceDb, donor_db: &mut ResourceDb, path: &str) -> Result<(Supply, Option<Vec<u8>>)> {
        let no = |why: String| Supply { path: path.to_owned(), donor: None, not_supplied: Some(why) };
        let Some((family, key)) = path.strip_prefix(ROOT).and_then(|p| p.split_once('/')) else {
            return Ok((no("not a shader program's path".into()), None));
        };
        if shipped.is_file(path) {
            return Ok((no("players have it".into()), None));
        }
        let donor_key = match translate(key, family, self.target, self.donor) {
            Ok(k) => k,
            Err(e) => return Ok((no(format!("{e:#}")), None)),
        };
        let donor_path = format!("{ROOT}{family}/{donor_key}");
        if !donor_db.is_file(&donor_path) {
            return Ok((no(format!("the donor has no {donor_path}")), None));
        }
        self.donor.check_family(donor_db, family)?;
        let blob = donor_db.read(&donor_path)?;
        if let Some(why) = self.family(shipped, family)?.refuses(family, &donor_path, &blob)? {
            return Ok((no(why), None));
        }
        Ok((Supply { path: path.to_owned(), donor: Some(donor_path), not_supplied: None }, Some(blob)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(order: &[&str], family: &str, mask: &[&str]) -> Keys {
        let text: String = order.iter().enumerate().map(|(i, n)| format!("{i}\t{n}\n")).collect();
        let mut masks = BTreeMap::new();
        masks.insert(family.to_owned(), mask.iter().map(|m| (*m).to_owned()).collect());
        Keys { order: KeyOrder::parse(&text).unwrap(), masks: Masks(masks) }
    }

    #[test]
    fn a_key_moves_to_the_donors_order_with_its_new_macro_off() {
        let target = keys(&["Q", "TDIFFUSE", "TNORMAL", "ALPHA_TEST", "FOG"], "g.ps", &["Q", "TDIFFUSE", "TNORMAL", "ALPHA_TEST"]);
        let donor = keys(&["Q", "TDIFFUSE", "ALPHA_TEST", "ANGLE", "TNORMAL", "FOG"], "g.ps", &["Q", "TDIFFUSE", "TNORMAL", "ALPHA_TEST", "ANGLE"]);
        assert_eq!(translate("2101_", "g.ps", &target, &donor).unwrap(), "21100_");
        // 0.26g's family reads a macro the donor's does not: only its 0 has a program there.
        let narrow = keys(&["Q", "TDIFFUSE", "ALPHA_TEST", "ANGLE", "TNORMAL", "FOG"], "g.ps", &["Q", "TDIFFUSE", "ALPHA_TEST"]);
        assert_eq!(translate("2001_", "g.ps", &target, &narrow).unwrap(), "201___");
        assert!(translate("2011_", "g.ps", &target, &narrow).is_err());
        assert!(translate("2101", "g.ps", &target, &donor).is_err());
        // A character where the family reads nothing: not this build's order.
        assert!(translate("21010", "g.ps", &target, &donor).is_err());
    }

    #[test]
    fn a_mask_that_grows_widens_every_key_with_the_new_macro_off() {
        let old = keys(&["Q", "OLTA", "TDIFFUSE"], "f.ps", &["Q", "TDIFFUSE"]);
        let new = old.with_macros("f.ps", &["OLTA".to_owned()]).unwrap();
        assert_eq!(translate("2_1", "f.ps", &old, &new).unwrap(), "201");
        // Back from a key with the new macro on: the old mask cannot hold it.
        assert!(translate("211", "f.ps", &new, &old).is_err());
        assert!(new.with_macros("f.ps", &["OLTA".to_owned()]).is_err(), "a macro the family reads already");
        assert!(old.with_macros("f.ps", &["NOT_A_MACRO".to_owned()]).is_err());
    }

    #[test]
    fn the_vertex_input_type_takes_two_characters_from_ten() {
        let target = keys(&["Q", WIDE, "TDIFFUSE"], "g.ps", &["Q", WIDE, "TDIFFUSE"]);
        let donor = keys(&["TDIFFUSE", "Q", WIDE], "g.ps", &["Q", WIDE, "TDIFFUSE"]);
        assert_eq!(target.values("g.ps", "2141").unwrap(), ["2", "14", "1"]);
        assert_eq!(translate("2141", "g.ps", &target, &donor).unwrap(), "1214");
        assert_eq!(translate("231", "g.ps", &target, &donor).unwrap(), "123");
    }

    /// A blob with no include records and a DXBC holding one RDEF chunk: one
    /// constant buffer with the given variables, and the bound names.
    fn blob(variables: &[(&str, u32, bool)], bound: &[&str]) -> Vec<u8> {
        let mut names = Vec::new();
        let mut name = |s: &str, base: usize| {
            let at = base + names.len();
            names.extend_from_slice(s.as_bytes());
            names.push(0);
            at as u32
        };
        let header = 28;
        let buffers_at = header;
        let variables_at = buffers_at + 24;
        let bindings_at = variables_at + 24 * variables.len();
        let names_at = bindings_at + 32 * (bound.len() + 1);
        let mut rdef = vec![0u8; names_at];
        let put = |r: &mut Vec<u8>, at: usize, v: u32| r[at..at + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut rdef, 0, 1);
        put(&mut rdef, 4, buffers_at as u32);
        put(&mut rdef, 8, bound.len() as u32 + 1);
        put(&mut rdef, 12, bindings_at as u32);
        let cb = name("cb", names_at);
        put(&mut rdef, buffers_at, cb);
        put(&mut rdef, buffers_at + 4, variables.len() as u32);
        put(&mut rdef, buffers_at + 8, variables_at as u32);
        for (i, (n, size, used)) in variables.iter().enumerate() {
            let at = variables_at + 24 * i;
            let n = name(n, names_at);
            put(&mut rdef, at, n);
            put(&mut rdef, at + 8, *size);
            put(&mut rdef, at + 12, if *used { 2 } else { 0 });
        }
        // The constant buffer's own binding, then the textures.
        put(&mut rdef, bindings_at, cb);
        for (i, n) in bound.iter().enumerate() {
            let at = bindings_at + 32 * (i + 1);
            let n = name(n, names_at);
            put(&mut rdef, at, n);
            put(&mut rdef, at + 4, 2);
        }
        rdef.extend_from_slice(&names);
        let mut dxbc = b"DXBC".to_vec();
        dxbc.resize(28, 0);
        dxbc.extend_from_slice(&1u32.to_le_bytes());
        dxbc.extend_from_slice(&36u32.to_le_bytes());
        dxbc.extend_from_slice(b"RDEF");
        dxbc.extend_from_slice(&(rdef.len() as u32).to_le_bytes());
        dxbc.extend_from_slice(&rdef);
        let mut blob = 0u32.to_le_bytes().to_vec();
        blob.extend_from_slice(&dxbc);
        blob
    }

    #[test]
    fn a_blob_reads_its_used_constants_and_its_textures() {
        let r = reads(&blob(&[("alpha_ref", 4, true), ("unused", 16, false)], &["t_diffuse", "s_diffuse"])).unwrap();
        assert_eq!(r.constants.into_iter().map(|c| (c.name, c.size)).collect::<Vec<_>>(), [("alpha_ref".to_owned(), 4)]);
        assert_eq!(r.bound.into_iter().collect::<Vec<_>>(), ["s_diffuse", "t_diffuse"]);
    }

    #[test]
    fn the_code_reads_the_components_its_operands_name() {
        // mov o2.w, cb1[4].x ; mul r0.xyz, r0.xyzx, cb1[0].xyzx ; ret
        let cb = |element: u32, selection: u32| [0x0020_800a | selection, 1, element];
        let mut tokens: Vec<u32> = vec![0x40, 0];
        let one = cb(4, 0);
        tokens.extend([0x36 | 6 << 24, 0x0010_2082, 2, one[0], one[1], one[2]]);
        // A swizzle of four: xyzx.
        let swizzle = [0x0020_8002 | 1 << 2 | (0b00_10_01_00) << 4, 1, 0];
        tokens.extend([0x38 | 8 << 24, 0x0010_0072, 0, 0x0010_0246, 0, swizzle[0], swizzle[1], swizzle[2]]);
        tokens.push(0x3e | 1 << 24);
        tokens[1] = tokens.len() as u32;
        let code: Vec<u8> = tokens.iter().flat_map(|t| t.to_le_bytes()).collect();
        let read = cb_reads(&code).unwrap().unwrap();
        assert_eq!(read.into_iter().collect::<Vec<_>>(), [(1, 0), (1, 4), (1, 8), (1, 64)]);
    }
}
