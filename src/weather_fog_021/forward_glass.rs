//! Original forward fog: the three passes observed on 0.21 vehicle glass
//! ([`build`], pinned program pairs), and every other forward program that
//! computes the native fog ([`Fog`]). Lighting and transparency stay native;
//! only the fog computation changes.
use super::{Profile, assemble, bytes, checksum, chunks, instructions, pinned_code, u32_at, words};
use anyhow::{Context, Result, ensure};

#[derive(Clone, Copy)]
pub enum Kind {
    Base,
    Probe,
    Direct,
}

impl Kind {
    pub fn paths(self) -> (&'static str, &'static str) {
        match self {
            Self::Base => (
                "shaders/sm_4_0/forward_base.ps/______0________1__010__1________0_0_____________1__0_0_0________0__",
                "shaders/sm_4_0/forward_base.ps/______0________1_010__1________0_0______________1__0_0_0________0__",
            ),
            Self::Probe => (
                "shaders/sm_4_0/forward_probe_lighting.ps/_0___11_0___001___00___1________0_0_____________0__0_0_____________",
                "shaders/sm_4_0/forward_probe_lighting.ps/_____11_0___001__00___1________0_0______________0__0_0__________0__",
            ),
            Self::Direct => (
                "shaders/sm_4_0/forward_lighting.ps/_0__111_0___001__000___1___0____0_0_____________0__0_0_____________",
                "shaders/sm_4_0/forward_lighting.ps/____111_0___001_000___1___0____0_0______________0__0_0__________0__",
            ),
        }
    }

    fn specification(
        self,
    ) -> (
        &'static str,
        &'static str,
        std::ops::Range<usize>,
        std::ops::Range<usize>,
        u32,
    ) {
        match self {
            Self::Base => (
                "a3ca3f7366221b6b80b00c4fe7a9c79b7a359730c29ba1f2780af7ca984855a9",
                "11a0d17017593951bf8c850e2a5b5f7646413f03a196a1b236995822f5c88028",
                13..61,
                14..70,
                2,
            ),
            Self::Probe => (
                "7d8b804f706e6d3bacb1bc3cffb46a97131b3492d0d6bd83ee84719c129b8437",
                "b15ac0326fad1fbecb7b6e5457f05543a98d2461e2f9b241575136e98fc42046",
                134..183,
                135..192,
                3,
            ),
            Self::Direct => (
                "ae3cf994899c05c015fed0ec4ec0701391045f844eac9b9fb0d710edec8d1a66",
                "891830b564aa6b58fcdfaa6bd0c453b497483b274191ad96f9194d36df842299",
                389..439,
                393..451,
                3,
            ),
        }
    }
}

fn program(code: &[u8]) -> Result<Vec<Vec<u32>>> {
    let entries = chunks(code)?;
    instructions(&words(
        &entries
            .iter()
            .find(|(tag, _)| tag == b"SHDR")
            .context("SHDR")?
            .1,
    )?)
}

/// Transplant the pinned original block into scratch registers behind the
/// existing scene marker. None means this permutation uses a different block.
pub fn build(
    kind: Kind,
    source: &[u8],
    template: &[u8],
    target: &[u8],
    profiles: &[Profile],
) -> Result<Option<Vec<u8>>> {
    ensure!(!profiles.is_empty(), "no forward fog profiles");
    let (source_hash, native_hash, source_range, native_range, globals) = kind.specification();
    let (_, original) = pinned_code(source, source_hash)?;
    let (_, reference) = pinned_code(template, native_hash)?;
    let original_ops = program(original)?;
    let reference_ops = program(reference)?;
    let original_block = &original_ops[source_range];
    let native_block = &reference_ops[native_range];
    let offset = 4 + u32_at(target, 0)? as usize * 264;
    let code = target.get(offset..).context("truncated forward shader")?;
    ensure!(checksum(code)? == code[4..20], "forward checksum mismatch");
    let mut entries = chunks(code)?;
    let index = entries
        .iter()
        .position(|(tag, _)| tag == b"SHDR")
        .context("SHDR")?;
    let old_words = words(&entries[index].1)?;
    let mut ops = instructions(&old_words)?;
    let matches: Vec<_> = ops
        .windows(native_block.len())
        .enumerate()
        .filter_map(|(i, block)| (block == native_block).then_some(i))
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    ensure!(matches.len() == 1, "ambiguous forward fog block");
    let start = matches[0];
    let end = start + native_block.len();
    match kind {
        Kind::Base => {
            if ops.get(end)
                != Some(&vec![
                    0x08000038, 0x00100062, 0, 0x00101106, 0, 0x00208ba6, 2, 14,
                ])
            {
                // Another transparency variant shares its fog prefix: it is
                // one of `Fog`'s.
                return Ok(None);
            }
        }
        Kind::Probe | Kind::Direct => ensure!(
            ops[end..] == [vec![0x0100003e]],
            "lighting fog must end before return"
        ),
    }
    let temp = ops
        .iter()
        .position(|op| op[0] & 2047 == 104)
        .context("temporaries")?;
    let scratch = ops[temp][1];
    ensure!(scratch <= 64, "unexpected forward temporaries");
    ops[temp][1] = scratch + 5;
    let bindings = Bindings {
        scratch,
        globals: (globals, globals),
        input: None,
        output: None,
    };
    let body = transplanted(original_block, &bindings)?;
    let (before, after): (&[u32], &[u32]) = match kind {
        // The unchanged native transparency tail consumes only r0.x.
        Kind::Base => (&[], &[0x05000036, 0x00100012, 0, 0x0010000a, scratch]),
        Kind::Probe | Kind::Direct => (&[0x05000036, 0x001000f2, scratch, 0x00100e46, 0], &[]),
    };
    let updated = guarded(
        &old_words[..2],
        &ops,
        start..end,
        profiles,
        &bindings,
        before,
        &body,
        after,
    )?;
    entries[index].1 = bytes(&updated);
    let mut result = target[..offset].to_vec();
    result.extend(assemble(code, &entries)?);
    Ok(Some(result))
}

/// Where the transplanted original block lives in a target program.
struct Bindings {
    /// The first of five fresh temporaries: four for the block, one for the
    /// scene's profile.
    scratch: u32,
    /// The globals buffer of the original program and of the target.
    globals: (u32, u32),
    /// The world-position input of the original program and of the target,
    /// where they differ.
    input: Option<(u32, u32)>,
    /// The output register the original program and the target write the fog
    /// to, where the block writes it itself.
    output: Option<(u32, u32)>,
}

impl Bindings {
    fn profile(&self) -> u32 {
        self.scratch + 4
    }
}

/// The original block in the target's registers, with the established schema
/// bindings: the far distance, the infinite height multiplier and the scene's
/// Rayleigh colour.
fn transplanted(original_block: &[Vec<u32>], bindings: &Bindings) -> Result<Vec<Vec<u32>>> {
    let profile_register = bindings.profile();
    let mut body = Vec::new();
    let mut substitutions = [0; 3];
    for instruction in original_block {
        let mut fixed = vec![instruction[0]];
        let mut at = 1;
        while at < instruction.len() {
            let mut token = instruction[at];
            at += 1;
            let extension = if token & 0x80000000 != 0 {
                let v = instruction[at];
                at += 1;
                Some(v)
            } else {
                None
            };
            let operand = (token >> 12) & 255;
            let dimensions = ((token >> 20) & 3) as usize;
            let mut indices = instruction[at..at + dimensions].to_vec();
            at += dimensions;
            if operand == 4 {
                ensure!(
                    dimensions == 0 && extension.is_none(),
                    "unexpected forward literal"
                );
                let count = if token & 3 == 2 { 4 } else { 1 };
                fixed.push(token);
                fixed.extend(&instruction[at..at + count]);
                at += count;
                continue;
            }
            match operand {
                0 => {
                    ensure!(
                        dimensions == 1 && indices[0] < 4,
                        "unexpected source temporary"
                    );
                    indices[0] += bindings.scratch;
                }
                8 if indices[0] == bindings.globals.0 => {
                    ensure!(dimensions == 2, "unexpected globals operand");
                    indices[0] = bindings.globals.1;
                    if indices[1] == 18 {
                        let component = (token >> 4) & 3;
                        if component == 0 {
                            token += 0x10;
                            substitutions[0] += 1;
                        } else {
                            ensure!(component == 1, "unexpected source height operand");
                            token = (token & !(0x00300000 | 0x000ff000 | 0x30)) | 0x00100000 | 0x30;
                            indices = vec![profile_register];
                            substitutions[1] += 1;
                        }
                    } else if indices[1] == 15 && (token >> 2) & 3 == 1 {
                        token = (token & !(0x00300000 | 0x000ff000)) | 0x00100000;
                        indices = vec![profile_register];
                        substitutions[2] += 1;
                    }
                }
                1 => {
                    if let Some((from, to)) = bindings.input {
                        ensure!(
                            dimensions == 1 && indices[0] == from,
                            "unexpected forward fog input"
                        );
                        indices[0] = to;
                    }
                }
                2 => {
                    if let Some((from, to)) = bindings.output {
                        ensure!(
                            dimensions == 1 && indices[0] == from,
                            "unexpected forward fog output"
                        );
                        indices[0] = to;
                    }
                }
                8 => {}
                _ => anyhow::bail!("unexpected forward fog operand {operand}"),
            }
            fixed.push(token);
            fixed.extend(extension);
            fixed.extend(indices);
        }
        fixed[0] = (fixed[0] & !0x7f000000) | (fixed.len() as u32) << 24;
        body.push(fixed);
    }
    ensure!(
        substitutions == [1, 2, 1],
        "original forward bindings changed"
    );
    Ok(body)
}

/// The program with `block` behind the scene guards: each marked scene runs
/// `before`, the transplanted `body` and `after`; every other scene runs the
/// native block.
#[allow(clippy::too_many_arguments)]
fn guarded(
    header: &[u32],
    ops: &[Vec<u32>],
    block: std::ops::Range<usize>,
    profiles: &[Profile],
    bindings: &Bindings,
    before: &[u32],
    body: &[Vec<u32>],
    after: &[u32],
) -> Result<Vec<u32>> {
    let profile_register = bindings.profile();
    let mut updated = header.to_vec();
    updated.extend(ops[..block.start].iter().flatten());
    let mut markers = std::collections::BTreeSet::new();
    for profile in profiles {
        ensure!(
            profile.marker.is_finite()
                && profile.marker < -32000.0
                && markers.insert(profile.marker.to_bits())
                && profile.color.iter().all(|v| v.is_finite() && *v >= 0.0),
            "invalid forward profile"
        );
        updated.extend([
            0x08000018,
            0x00100082,
            profile_register,
            0x0020803a,
            bindings.globals.1,
            18,
            0x00004001,
            profile.marker.to_bits(),
            0x0304001f,
            0x0010003a,
            profile_register,
        ]);
        // Same observed original lerp bug and schema conversion as the day
        // fullscreen/particle/water adapters; do not invent a new fog setting.
        updated.extend([
            0x08000036,
            0x001000f2,
            profile_register,
            0x00004002,
            profile.color[0].to_bits(),
            profile.color[1].to_bits(),
            profile.color[2].to_bits(),
            f32::INFINITY.to_bits(),
        ]);
        updated.extend(before);
        updated.extend(body.iter().flatten());
        updated.extend(after);
        updated.push(0x01000012);
    }
    updated.extend(ops[block.clone()].iter().flatten());
    updated.extend(std::iter::repeat_n(0x01000015, profiles.len()));
    updated.extend(ops[block.end..].iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    Ok(updated)
}

/// The program families whose pixel programs fog their own output.
pub const FAMILIES: [&str; 3] = [
    "shaders/sm_4_0/forward_base.ps/",
    "shaders/sm_4_0/forward_probe_lighting.ps/",
    "shaders/sm_4_0/forward_lighting.ps/",
];

/// One component of a temporary register.
type Slot = (u32, u32);

/// Operand types of the SM4 bytecode this module tells apart.
const TEMPORARY: u32 = 0;
const INPUT: u32 = 1;
const OUTPUT: u32 = 2;
const LITERAL: u32 = 4;
const SAMPLER: u32 = 6;
const RESOURCE: u32 = 7;
const BUFFER: u32 = 8;
const NULL: u32 = 13;

struct Operand {
    kind: u32,
    /// Four components (else one, or none).
    vector: bool,
    selection: Selection,
    extension: Option<u32>,
    indices: Vec<u32>,
    literal: Vec<u32>,
}

/// Which components of a four-component operand an instruction names.
#[derive(Clone, Copy)]
enum Selection {
    Mask(u32),
    Swizzle(u32),
    One(u32),
}

impl Operand {
    /// The components a destination writes.
    fn mask(&self) -> Option<impl Iterator<Item = u32> + use<>> {
        match (self.vector, self.selection) {
            (true, Selection::Mask(mask)) => Some((0..4).filter(move |c| mask & (1 << c) != 0)),
            _ => None,
        }
    }

    /// The component this source gives lane `lane` of an instruction.
    fn component(&self, lane: u32) -> Option<u32> {
        match (self.vector, self.selection) {
            (true, Selection::Swizzle(swizzle)) => Some((swizzle >> (2 * lane)) & 3),
            (true, Selection::One(component)) => Some(component),
            _ => None,
        }
    }
}

/// An instruction's operands, or None for an encoding this module does not
/// read (extended opcodes, relative indices).
fn operands(instruction: &[u32]) -> Option<Vec<Operand>> {
    if instruction[0] & 0x80000000 != 0 {
        return None;
    }
    let mut at = 1;
    let mut result = Vec::new();
    while at < instruction.len() {
        let token = instruction[at];
        at += 1;
        let extension = if token & 0x80000000 != 0 {
            let value = *instruction.get(at)?;
            at += 1;
            (value & 0x80000000 == 0).then_some(value)?;
            Some(value)
        } else {
            None
        };
        let dimensions = ((token >> 20) & 3) as usize;
        if (0..dimensions).any(|i| (token >> (22 + 3 * i)) & 7 != 0) {
            return None;
        }
        let kind = (token >> 12) & 255;
        let vector = token & 3 == 2;
        let literals = match (kind, vector) {
            (LITERAL, true) => 4,
            (LITERAL, false) => 1,
            _ => 0,
        };
        let indices = instruction.get(at..at + dimensions)?.to_vec();
        at += dimensions;
        let literal = instruction.get(at..at + literals)?.to_vec();
        at += literals;
        let bits = (token >> 4) & 255;
        result.push(Operand {
            kind,
            vector,
            selection: match (token >> 2) & 3 {
                0 => Selection::Mask(bits & 15),
                1 => Selection::Swizzle(bits),
                2 => Selection::One(bits & 3),
                _ => return None,
            },
            extension,
            indices,
            literal,
        });
    }
    Some(result)
}

/// How many components a dot product reads.
fn dot(opcode: u32) -> Option<u32> {
    match opcode {
        15 => Some(2),
        16 => Some(3),
        17 => Some(4),
        _ => None,
    }
}

/// Opcodes whose first operand is their only destination.
fn writes_first(opcode: u32) -> bool {
    matches!(
        opcode,
        0 | 1 | 11 | 12 | 14..=17 | 24..=30 | 32..=37 | 39..=43 | 45 | 47 | 49..=52 | 54..=57
            | 59..=61 | 64..=75 | 79 | 80 | 82..=87
    )
}

/// The lanes of an instruction: a componentwise operation's are the
/// components it writes, a dot product's or a sample's the leading ones.
fn lanes(opcode: u32, destination: Option<&Operand>) -> Vec<u32> {
    let componentwise = writes_first(opcode) && !matches!(opcode, 45 | 61 | 69..=74);
    match (dot(opcode), destination.and_then(Operand::mask)) {
        (Some(count), _) => (0..count).collect(),
        (None, Some(mask)) if componentwise => mask.collect(),
        _ => (0..4).collect(),
    }
}

/// What a component holds, by how it was computed: two components hold the
/// same value exactly when their computations are the same, whatever order
/// and registers the compiler chose.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Value {
    Literal(u32),
    /// A component of the world position the fog starts from.
    Position(u32),
    /// An element and component of the globals buffer.
    Global(u32, u32),
    /// An operand modifier (negation, absolute value) on a value.
    Modified(u32, u32),
    /// A componentwise operation (its opcode token without the length).
    Lane(u32, Vec<u32>),
    Dot(u32, Vec<Vec<u32>>),
    /// A component of the fog texture's sample at coordinates, with its level.
    Sample(u32, Vec<u32>, u32, Vec<u32>),
}

/// Values numbered once each.
#[derive(Default)]
struct Values(std::collections::BTreeMap<Value, u32>);

impl Values {
    fn id(&mut self, value: Value) -> u32 {
        let next = self.0.len() as u32;
        *self.0.entry(value).or_insert(next)
    }
}

/// A run of instructions that computes from the world position, the globals,
/// literals and the fog texture alone.
#[derive(Default)]
struct Region {
    held: std::collections::BTreeMap<Slot, u32>,
    written: std::collections::BTreeSet<Slot>,
    /// What it has written to output registers, by register and component.
    sent: std::collections::BTreeMap<Slot, u32>,
    input: Option<u32>,
    globals: Option<u32>,
    texture: Option<u32>,
    sampler: Option<u32>,
}

/// Whether `bound` is, or now becomes, `index`: a region reads one input,
/// one globals buffer, one texture and one sampler.
fn bind(bound: &mut Option<u32>, index: u32) -> Option<()> {
    (*bound.get_or_insert(index) == index).then_some(())
}

impl Region {
    /// The value lane `lane` of `source` has.
    fn value(&mut self, values: &mut Values, source: &Operand, lane: u32) -> Option<u32> {
        let value = match source.kind {
            LITERAL => {
                Value::Literal(source.literal[if source.vector { lane as usize } else { 0 }])
            }
            TEMPORARY => {
                return self.modified(
                    values,
                    source,
                    *self
                        .held
                        .get(&(source.indices[0], source.component(lane)?))?,
                );
            }
            INPUT => {
                bind(&mut self.input, source.indices[0])?;
                Value::Position(source.component(lane)?)
            }
            BUFFER => {
                bind(&mut self.globals, source.indices[0])?;
                Value::Global(source.indices[1], source.component(lane)?)
            }
            _ => return None,
        };
        let id = values.id(value);
        self.modified(values, source, id)
    }

    fn modified(&self, values: &mut Values, source: &Operand, id: u32) -> Option<u32> {
        Some(match source.extension {
            Some(extension) => values.id(Value::Modified(extension, id)),
            None => id,
        })
    }

    /// Runs one instruction; None when it is not one of the fog's operations
    /// on the region's own values.
    fn step(&mut self, values: &mut Values, instruction: &[u32]) -> Option<()> {
        let opcode = instruction[0] & 2047;
        // add div dp3 exp log lt mad min max mov movc mul rsq sqrt sample_l
        matches!(opcode, 0 | 14 | 16 | 25 | 47 | 49..=52 | 54..=56 | 68 | 72 | 75).then_some(())?;
        let token = instruction[0] & 0x80ffffff;
        let operands = operands(instruction)?;
        let (destination, sources) = operands.split_first()?;
        (matches!(destination.kind, TEMPORARY | OUTPUT) && destination.extension.is_none())
            .then_some(())?;
        let written: Vec<u32> = destination.mask()?.collect();
        let mut results = Vec::new();
        if let Some(count) = dot(opcode) {
            let mut arguments = Vec::new();
            for source in sources {
                arguments.push(
                    (0..count)
                        .map(|lane| self.value(values, source, lane))
                        .collect::<Option<Vec<_>>>()?,
                );
            }
            let id = values.id(Value::Dot(token, arguments));
            results.extend(written.iter().map(|_| id));
        } else if opcode == 72 {
            let [coordinates, texture, sampler, level] = sources else {
                return None;
            };
            (coordinates.kind == TEMPORARY
                && texture.kind == RESOURCE
                && sampler.kind == SAMPLER
                && level.kind == LITERAL)
                .then_some(())?;
            bind(&mut self.texture, texture.indices[0])?;
            bind(&mut self.sampler, sampler.indices[0])?;
            let at = (0..4)
                .map(|lane| self.value(values, coordinates, lane))
                .collect::<Option<Vec<_>>>()?;
            for &component in &written {
                results.push(values.id(Value::Sample(
                    token,
                    at.clone(),
                    texture.component(component)?,
                    level.literal.clone(),
                )));
            }
        } else {
            for &component in &written {
                let arguments = sources
                    .iter()
                    .map(|source| self.value(values, source, component))
                    .collect::<Option<Vec<_>>>()?;
                results.push(values.id(Value::Lane(token, arguments)));
            }
        }
        for (component, id) in written.into_iter().zip(results) {
            let slot = (destination.indices[0], component);
            if destination.kind == OUTPUT {
                self.sent.insert(slot, id);
            } else {
                self.held.insert(slot, id);
                self.written.insert(slot);
            }
        }
        Some(())
    }

    /// The output register whose x, y, z and w were sent `results`.
    fn sending(&self, results: &[u32; 4]) -> Option<u32> {
        let register = self.sent.iter().find(|(_, sent)| **sent == results[0])?.0.0;
        (0..4)
            .all(|c| self.sent.get(&(register, c)) == Some(&results[c as usize]))
            .then_some(register)
    }

    /// The components that hold `results`, when each is held.
    fn holding(&self, results: &[u32; 4]) -> Option<[Slot; 4]> {
        let slot = |id| {
            self.held
                .iter()
                .find(|(_, held)| **held == id)
                .map(|(slot, _)| *slot)
        };
        Some([
            slot(results[0])?,
            slot(results[1])?,
            slot(results[2])?,
            slot(results[3])?,
        ])
    }
}

/// The native fog found in a program.
struct Found {
    block: std::ops::Range<usize>,
    input: u32,
    globals: u32,
    /// Where it leaves the fog colour's three components and the fog amount.
    results: [Slot; 4],
    /// Every component it writes.
    written: std::collections::BTreeSet<Slot>,
}

/// Whether the instructions after the block read nothing of what it wrote but
/// its results: the transplanted block computes only those. A component
/// written again is the program's own from there on: to the end outside any
/// branch, to the end of its branch inside one.
fn reads_only_results(
    tail: &[Vec<u32>],
    written: &std::collections::BTreeSet<Slot>,
    results: &[Slot],
) -> bool {
    let mut own = std::collections::BTreeSet::new();
    // What was the program's own where each open branch or loop began.
    let mut outer: Vec<std::collections::BTreeSet<Slot>> = Vec::new();
    for instruction in tail {
        let opcode = instruction[0] & 2047;
        // Declarations and anything later than SM4's last operation.
        if opcode >= 88 {
            return false;
        }
        let Some(operands) = operands(instruction) else {
            return false;
        };
        // imul, sincos, udiv and umul write their first two operands (either
        // may be the null operand); the others here write at most the first.
        let wrote = match opcode {
            38 | 77 | 78 | 81 => 2,
            _ => usize::from(writes_first(opcode)),
        };
        let Some((destinations, sources)) = operands.split_at_checked(wrote) else {
            return false;
        };
        if !destinations
            .iter()
            .all(|d| matches!(d.kind, TEMPORARY | OUTPUT | NULL))
        {
            return false;
        }
        let destination = match destinations {
            [one] => Some(one),
            _ => None,
        };
        for source in sources {
            match source.kind {
                TEMPORARY => {
                    for lane in lanes(opcode, destination) {
                        let Some(component) = source.component(lane) else {
                            return false;
                        };
                        let slot = (source.indices[0], component);
                        if written.contains(&slot)
                            && !own.contains(&slot)
                            && !results.contains(&slot)
                        {
                            return false;
                        }
                    }
                }
                // An indexable temporary: not followed.
                3 => return false,
                _ => {}
            }
        }
        match opcode {
            // if, loop, switch
            31 | 48 | 76 => outer.push(own.clone()),
            // else, case, default: another path from where the branch began
            18 | 6 | 10 => match outer.last() {
                Some(began) => own = began.clone(),
                None => return false,
            },
            // endif, endloop, endswitch: a path may have skipped the writes
            21..=23 => match outer.pop() {
                Some(began) => own = began,
                None => return false,
            },
            _ => {
                for destination in destinations.iter().filter(|d| d.kind == TEMPORARY) {
                    let Some(mask) = destination.mask() else {
                        return false;
                    };
                    own.extend(mask.map(|c| (destination.indices[0], c)));
                }
            }
        }
    }
    true
}

/// Whether the instructions end inside a branch or loop: what follows them
/// there is not all that follows.
fn nested(before: &[Vec<u32>]) -> bool {
    before.iter().fold(0i32, |depth, op| match op[0] & 2047 {
        31 | 48 | 76 => depth + 1,
        21..=23 => depth - 1,
        _ => depth,
    }) != 0
}

/// `mov` of the original block's results (colour in its r0.xyz, amount in its
/// r1.y) into the components the program's tail reads.
fn results_moved(results: [Slot; 4], scratch: u32) -> Vec<u32> {
    let [red, green, blue, amount] = results;
    let mut moves = Vec::new();
    if red.0 == green.0 && red.0 == blue.0 {
        let colour = [red.1, green.1, blue.1];
        let mask: u32 = colour.iter().map(|c| 1 << c).sum();
        // A component the mask leaves out repeats the first one's source.
        let swizzle: u32 = (0..4)
            .map(|c| (colour.iter().position(|&x| x == c).unwrap_or(0) as u32) << (2 * c))
            .sum();
        moves.extend([
            0x05000036,
            0x00100002 | mask << 4,
            red.0,
            0x00100006 | swizzle << 4,
            scratch,
        ]);
    } else {
        for (component, (register, to)) in [red, green, blue].into_iter().enumerate() {
            moves.extend([
                0x05000036,
                0x00100002 | (1 << to) << 4,
                register,
                0x0010000a | (component as u32) << 4,
                scratch,
            ]);
        }
    }
    moves.extend([
        0x05000036,
        0x00100002 | (1 << amount.1) << 4,
        amount.0,
        0x0010001a,
        scratch + 1,
    ]);
    moves
}

/// Whether a native fog's values are what its results are taken for: the
/// colour `(texture.component * mie + rayleigh) / max(amount, tiny)` of the
/// fog texture's red, green and blue, and the amount clamped.
fn is_native_fog(values: &Values, way: &[u32; 4]) -> bool {
    let value = |id: u32| {
        values
            .0
            .iter()
            .find(|(_, held)| **held == id)
            .map(|(value, _)| value)
    };
    let colour = |id: u32, component: u32| {
        let Some(Value::Lane(divide, over)) = value(id) else {
            return false;
        };
        let Some(Value::Lane(sum, terms)) = over.first().and_then(|id| value(*id)) else {
            return false;
        };
        divide & 0x27ff == 14
            && sum & 0x27ff == 50
            && matches!(terms.first().and_then(|id| value(*id)), Some(Value::Sample(_, _, sampled, _)) if *sampled == component)
    };
    (0..3).all(|c| colour(way[c as usize], c))
        && matches!(value(way[3]), Some(Value::Lane(token, _)) if token & 0x27ff == 0x2036)
}

/// The original fog for any forward program that computes the native one.
///
/// Every fogged 0.21 forward program computes one fog (its block is the same
/// in all of them); the native programs compute theirs in three ways, each in
/// whatever order and registers the compiler chose. A program is adapted when
/// a run of its instructions computes one of the three from the world
/// position, the globals and the fog texture alone, and nothing after it reads
/// anything the run wrote but the fog colour and amount.
pub struct Fog {
    original_block: Vec<Vec<u32>>,
    values: Values,
    /// Each native way's fog colour (three components) and amount.
    native: Vec<[u32; 4]>,
}

impl Fog {
    /// 0.21's additive base program: its fog block is instructions 10..55,
    /// the colour left in r0.xyz and the amount in r1.y.
    pub const ORIGINAL: &str = "shaders/sm_4_0/forward_base.ps/______0________2__010__3________0_0_____________0__0_0_0________0__";
    const ORIGINAL_HASH: &str = "22698bac066933e07f1785714f743ee0767809bd2579d0c55c70ab134454d6d9";
    const ORIGINAL_BLOCK: std::ops::Range<usize> = 10..55;
    /// The original program's globals buffer and world-position input.
    const ORIGINAL_GLOBALS: u32 = 2;
    const ORIGINAL_INPUT: u32 = 6;

    /// A native program for each way the native fog is computed, in the
    /// order [`Fog::new`] takes them: the base and probe passes' (and 400
    /// direct-light programs'), and two more among the direct-light programs.
    pub const NATIVE: [&str; 3] = [
        "shaders/sm_4_0/forward_base.ps/______0________2_010__3________0_0______________0__0_0_0________0__",
        "shaders/sm_4_0/forward_lighting.ps/____000_0___000_000___0___0____0_0______________0__0_0__________0__",
        "shaders/sm_4_0/forward_lighting.ps/____001_0___000_001___0___0____0_0______________1__0_0__________1__",
    ];
    /// Each one's hash, its fog's instructions and where they leave the
    /// colour and the amount.
    const NATIVE_PINS: [(&str, std::ops::Range<usize>, [Slot; 4]); 3] = [
        (
            "3c12ca61033cb5c2bef08e2e96839a40bedce5ce3e6ce5a4e17912b469715bc2",
            11..64,
            [(0, 0), (0, 2), (0, 3), (0, 1)],
        ),
        (
            "9cb6d4aca31e98266d21670a6114c87c3758d78cbb6cd9d358db39bbd6e4a542",
            364..417,
            [(1, 1), (1, 2), (1, 3), (1, 0)],
        ),
        (
            "13e5b75c2f554d81857206317c7b30bcd1afca915a61998a2a8e7bf03c0b12c7",
            407..460,
            [(1, 0), (1, 1), (1, 2), (0, 2)],
        ),
    ];

    pub fn new(original: &[u8], native: [&[u8]; 3]) -> Result<Self> {
        let (_, original) = pinned_code(original, Self::ORIGINAL_HASH)?;
        let original_block = program(original)?[Self::ORIGINAL_BLOCK].to_vec();
        let mut values = Values::default();
        let mut ways = Vec::new();
        for (blob, (hash, block, results)) in native.into_iter().zip(Self::NATIVE_PINS) {
            let (_, code) = pinned_code(blob, hash)?;
            let ops = program(code)?;
            let mut region = Region::default();
            for instruction in &ops[block] {
                region
                    .step(&mut values, instruction)
                    .context("a pinned native fog block is not a fog computation")?;
            }
            let held = |slot: Slot| region.held.get(&slot).copied().context("pinned fog result");
            let way = [
                held(results[0])?,
                held(results[1])?,
                held(results[2])?,
                held(results[3])?,
            ];
            ensure!(
                is_native_fog(&values, &way),
                "a pinned native fog's results are not its colour and amount"
            );
            ensure!(!ways.contains(&way), "two pinned native fogs are one");
            ways.push(way);
        }
        Ok(Self {
            original_block,
            values,
            native: ways,
        })
    }

    /// The native fog in `ops`, when one run of instructions computes it.
    fn find(&mut self, ops: &[Vec<u32>]) -> Result<Option<Found>> {
        let mut found = None;
        for start in 0..ops.len() {
            // Every native fog starts with the world position less the eye's.
            if ops[start][0] & 2047 != 0 {
                continue;
            }
            let mut region = Region::default();
            for (end, instruction) in ops.iter().enumerate().skip(start) {
                if region.step(&mut self.values, instruction).is_none() {
                    break;
                }
                let Some(results) = self.native.iter().find_map(|way| region.holding(way)) else {
                    continue;
                };
                ensure!(found.is_none(), "ambiguous forward fog block");
                found = Some(Found {
                    block: start..end + 1,
                    input: region.input.context("fog without a position")?,
                    globals: region.globals.context("fog without globals")?,
                    results,
                    written: region.written,
                });
                break;
            }
        }
        Ok(found)
    }

    /// `target` with the original fog on the marked scenes. None: it computes
    /// no native fog in one run, or reads more of that run than the fog colour
    /// and amount; such a program stays native.
    pub fn build(&mut self, target: &[u8], profiles: &[Profile]) -> Result<Option<Vec<u8>>> {
        ensure!(!profiles.is_empty(), "no forward fog profiles");
        let offset = 4 + u32_at(target, 0)? as usize * 264;
        let code = target.get(offset..).context("truncated forward shader")?;
        ensure!(checksum(code)? == code[4..20], "forward checksum mismatch");
        let mut entries = chunks(code)?;
        let index = entries
            .iter()
            .position(|(tag, _)| tag == b"SHDR")
            .context("SHDR")?;
        let old_words = words(&entries[index].1)?;
        let mut ops = instructions(&old_words)?;
        let Some(found) = self.find(&ops)? else {
            return Ok(None);
        };
        if nested(&ops[..found.block.start])
            || !reads_only_results(&ops[found.block.end..], &found.written, &found.results)
        {
            return Ok(None);
        }
        let temp = ops
            .iter()
            .position(|op| op[0] & 2047 == 104)
            .context("temporaries")?;
        let scratch = ops[temp][1];
        ensure!(scratch <= 64, "unexpected forward temporaries");
        ops[temp][1] = scratch + 5;
        let bindings = Bindings {
            scratch,
            globals: (Self::ORIGINAL_GLOBALS, found.globals),
            input: Some((Self::ORIGINAL_INPUT, found.input)),
            output: None,
        };
        let body = transplanted(&self.original_block, &bindings)?;
        let updated = guarded(
            &old_words[..2],
            &ops,
            found.block.clone(),
            profiles,
            &bindings,
            &[],
            &body,
            &results_moved(found.results, scratch),
        )?;
        entries[index].1 = bytes(&updated);
        let mut result = target[..offset].to_vec();
        result.extend(assemble(code, &entries)?);
        Ok(Some(result))
    }
}

/// The original fog for the lit-particle vertex programs, which hand the fog
/// to their pixel program in an output register: the same recognition as
/// [`Fog`], the results being what the run writes to that register (the colour
/// in its xyz, the amount in its w) and nothing of the run being read after.
pub struct VertexFog {
    original_block: Vec<Vec<u32>>,
    values: Values,
    native: Vec<[u32; 4]>,
}

impl VertexFog {
    /// 0.21's program: its fog block is instructions 169..214, written to o7.
    pub const ORIGINAL: &str = super::PARTICLE_SOURCE;
    const ORIGINAL_HASH: &str = "955ddb23052e167e2842e990ff7698f3ed8ee05dd06ebda01096c21bb893e529";
    const ORIGINAL_BLOCK: std::ops::Range<usize> = 169..214;
    /// The original program's globals buffer, world-position input and fog
    /// output.
    const ORIGINAL_GLOBALS: u32 = 2;
    const ORIGINAL_INPUT: u32 = 0;
    const ORIGINAL_OUTPUT: u32 = 7;

    /// A native program for each of the two ways the family computes its fog.
    pub const NATIVE: [&str; 2] = [
        super::PARTICLE_TEMPLATE,
        "shaders/sm_4_0/vertex_base_offscreen_particle_lighting.vs/0___0____________0___07___0____0_0_________________0_0_____________",
    ];
    /// Each one's hash, its fog's instructions and the output they write.
    const NATIVE_PINS: [(&str, std::ops::Range<usize>, u32); 2] = [
        (
            "28ab00cfa95506fefed79f14bc8474df332e804d6ffc6be11846c4f42fd72a5f",
            37..90,
            7,
        ),
        (
            "b3479842605eee10128651f4e21bb2beeed4d491ae0b71d6c123419dd1b8407a",
            38..91,
            7,
        ),
    ];

    pub fn new(original: &[u8], native: [&[u8]; 2]) -> Result<Self> {
        let (_, original) = pinned_code(original, Self::ORIGINAL_HASH)?;
        let original_block = program(original)?[Self::ORIGINAL_BLOCK].to_vec();
        let mut values = Values::default();
        let sent = |values: &mut Values, block: &[Vec<u32>], output: u32| -> Result<[u32; 4]> {
            let mut region = Region::default();
            for instruction in block {
                region
                    .step(values, instruction)
                    .context("a pinned particle fog block is not a fog computation")?;
            }
            let at = |c| {
                region
                    .sent
                    .get(&(output, c))
                    .copied()
                    .context("pinned fog output")
            };
            ensure!(
                region.sent.keys().all(|(register, _)| *register == output),
                "a pinned particle fog block writes another output"
            );
            Ok([at(0)?, at(1)?, at(2)?, at(3)?])
        };
        // The original block must be one too, and write only its fog output.
        sent(&mut values, &original_block, Self::ORIGINAL_OUTPUT)?;
        let mut ways = Vec::new();
        for (blob, (hash, block, output)) in native.into_iter().zip(Self::NATIVE_PINS) {
            let (_, code) = pinned_code(blob, hash)?;
            let way = sent(&mut values, &program(code)?[block], output)?;
            ensure!(
                is_native_fog(&values, &way),
                "a pinned native particle fog's results are not its colour and amount"
            );
            ensure!(
                !ways.contains(&way),
                "two pinned native particle fogs are one"
            );
            ways.push(way);
        }
        Ok(Self {
            original_block,
            values,
            native: ways,
        })
    }

    /// `target` with the original fog on the marked scenes. None: it computes
    /// no native fog in one run outside any branch, or reads some of that run
    /// afterwards; such a program stays native.
    pub fn build(&mut self, target: &[u8], profiles: &[Profile]) -> Result<Option<Vec<u8>>> {
        ensure!(!profiles.is_empty(), "no particle fog profiles");
        let offset = 4 + u32_at(target, 0)? as usize * 264;
        let code = target.get(offset..).context("truncated particle shader")?;
        ensure!(checksum(code)? == code[4..20], "particle checksum mismatch");
        let mut entries = chunks(code)?;
        let index = entries
            .iter()
            .position(|(tag, _)| tag == b"SHDR")
            .context("SHDR")?;
        let old_words = words(&entries[index].1)?;
        let mut ops = instructions(&old_words)?;
        let mut found = None;
        for start in 0..ops.len() {
            if ops[start][0] & 2047 != 0 {
                continue;
            }
            let mut region = Region::default();
            for (end, instruction) in ops.iter().enumerate().skip(start) {
                if region.step(&mut self.values, instruction).is_none() {
                    break;
                }
                let Some(output) = self.native.iter().find_map(|way| region.sending(way)) else {
                    continue;
                };
                ensure!(found.is_none(), "ambiguous particle fog block");
                // Nothing else may leave the run but its fog.
                if region.sent.keys().all(|(register, _)| *register == output) {
                    found = Some((start..end + 1, output, region));
                }
                break;
            }
        }
        let Some((block, output, region)) = found else {
            return Ok(None);
        };
        if nested(&ops[..block.start])
            || !reads_only_results(&ops[block.end..], &region.written, &[])
        {
            return Ok(None);
        }
        let temp = ops
            .iter()
            .position(|op| op[0] & 2047 == 104)
            .context("temporaries")?;
        let scratch = ops[temp][1];
        ensure!(scratch <= 64, "unexpected particle temporaries");
        ops[temp][1] = scratch + 5;
        let bindings = Bindings {
            scratch,
            globals: (
                Self::ORIGINAL_GLOBALS,
                region.globals.context("fog without globals")?,
            ),
            input: Some((
                Self::ORIGINAL_INPUT,
                region.input.context("fog without a position")?,
            )),
            output: Some((Self::ORIGINAL_OUTPUT, output)),
        };
        let body = transplanted(&self.original_block, &bindings)?;
        let updated = guarded(
            &old_words[..2],
            &ops,
            block,
            profiles,
            &bindings,
            &[],
            &body,
            &[],
        )?;
        entries[index].1 = bytes(&updated);
        let mut result = target[..offset].to_vec();
        result.extend(assemble(code, &entries)?);
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(results: [Slot; 4], written: &[Slot]) -> Found {
        Found {
            block: 0..0,
            input: 1,
            globals: 2,
            results,
            written: written.iter().copied().collect(),
        }
    }

    /// `mul r1.x, r1.w, l(2.0)`: reads r1.w.
    const READS_R1_W: [u32; 7] = [
        0x07000038, 0x00100012, 1, 0x0010003a, 1, 0x00004001, 0x40000000,
    ];
    /// `mov r1.xyzw, l(0, 0, 0, 0)`.
    const WRITES_R1: [u32; 8] = [0x08000036, 0x001000f2, 1, 0x00004002, 0, 0, 0, 0];
    const RET: [u32; 1] = [0x0100003e];

    #[test]
    fn a_tail_that_reads_the_blocks_scratch_stays_native() {
        let block = found(
            [(0, 0), (0, 2), (0, 3), (0, 1)],
            &[(0, 0), (0, 1), (0, 2), (0, 3), (1, 3)],
        );
        assert!(!reads_only_results(
            &[READS_R1_W.to_vec(), RET.to_vec()],
            &block.written,
            &block.results
        ));
        // Written again first, the component is the tail's own.
        assert!(reads_only_results(
            &[WRITES_R1.to_vec(), READS_R1_W.to_vec(), RET.to_vec()],
            &block.written,
            &block.results
        ));
        // Inside the branch that wrote it, it is the tail's own.
        assert!(reads_only_results(
            &[
                vec![0x0304001f, 0x0010000a, 0],
                WRITES_R1.to_vec(),
                READS_R1_W.to_vec(),
                vec![0x01000015],
                RET.to_vec()
            ],
            &block.written,
            &block.results
        ));
        // A write inside a branch is not a write on every path.
        let branch = [0x0304001f, 0x0010000a, 0];
        assert!(!reads_only_results(
            &[
                branch.to_vec(),
                WRITES_R1.to_vec(),
                vec![0x01000015],
                READS_R1_W.to_vec(),
                RET.to_vec()
            ],
            &block.written,
            &block.results
        ));
    }

    #[test]
    fn results_move_as_one_colour_and_one_amount() {
        // The base pass keeps the colour in r0.xzw and the amount in r0.y.
        assert_eq!(
            results_moved([(0, 0), (0, 2), (0, 3), (0, 1)], 3),
            [
                0x05000036, 0x001000d2, 0, 0x00100906, 3, 0x05000036, 0x00100022, 0, 0x0010001a, 4
            ]
        );
        // The probe pass: r1.xyz and r0.w.
        assert_eq!(
            results_moved([(1, 0), (1, 1), (1, 2), (0, 3)], 4),
            [
                0x05000036, 0x00100072, 1, 0x00100246, 4, 0x05000036, 0x00100082, 0, 0x0010001a, 5
            ]
        );
    }

    #[test]
    fn the_same_computation_in_another_order_is_the_same_value() {
        // add r0.x, r1.x, r2.x after r1.x = v0.x + 1 and r2.x = v0.y * 2,
        // computed in either order and in other registers.
        let add = |to: u32, a: u32, b: u32| {
            vec![0x07000000, 0x00100012, to, 0x0010000a, a, 0x0010000a, b]
        };
        let from_input = |opcode: u32, to: u32, component: u32, literal: u32| {
            vec![
                0x07000000 | opcode,
                0x00100012,
                to,
                0x0010100a | component << 4,
                0,
                0x00004001,
                literal,
            ]
        };
        let mut values = Values::default();
        let mut run = |program: &[Vec<u32>], result: Slot| {
            let mut region = Region::default();
            for instruction in program {
                region.step(&mut values, instruction).unwrap();
            }
            region.held[&result]
        };
        let one = run(
            &[
                from_input(0, 1, 0, 1),
                from_input(56, 2, 1, 2),
                add(0, 1, 2),
            ],
            (0, 0),
        );
        let other = run(
            &[
                from_input(56, 5, 1, 2),
                from_input(0, 3, 0, 1),
                add(4, 3, 5),
            ],
            (4, 0),
        );
        let swapped = run(
            &[
                from_input(0, 1, 0, 1),
                from_input(56, 2, 1, 2),
                add(0, 2, 1),
            ],
            (0, 0),
        );
        assert_eq!(one, other);
        // Operand order is the compiler's too, but is not assumed to commute.
        assert_ne!(one, swapped);
    }
}
