// SPDX-License-Identifier: GPL-3.0-or-later
//! `pack release`: compose packs into the players' final `resources.db`.
//!
//! The final database is the stock file with a same-size head written over
//! its start (header, larger file table, signature slot) and a tail appended
//! (new and changed payloads, and the few stock payloads the larger table
//! displaced); see [`resources_db::headroom`]. Every other stock payload keeps
//! its bytes and offset, so whoever turns the final database into an update
//! (skhid-srp's `make_client_026g.py --resources-db`) finds a small head, the
//! tail, and nothing else changed.
//!
//! `--replace` gives base files the bytes players have now instead of the
//! stock ones (the server's own menus), before the packs apply: a lobby movie
//! gets the packs' icons drawn onto the live movie. A replaced or edited
//! movie that still fits its stock entry is padded to that size with the
//! entry's hash and written in place; anything else goes to the tail.
//!
//! The file table is signed: signing is the last change to the head
//! ([`sign_file_table`]), before it is written into the database.

use std::io::{Read, Seek, SeekFrom, Write};
use std::process::Command;

use anyhow::ensure;
use resources_db::headroom::{self, HeadroomPatch, SIGNATURE_LEN};

use super::*;

pub struct ReleaseOptions<'a> {
    pub base_db: &'a Path,
    pub packs: &'a [PathBuf],
    /// `(archive path, file)`: the bytes players have now for a base file.
    pub replace: &'a [(String, PathBuf)],
    pub work_dir: &'a Path,
    /// The final database. Its record is written next to it, `<db>.json`.
    pub output_db: &'a Path,
    /// Signer run as `<command words…> <head file> <signature offset>`; it
    /// must write exactly the 256-byte slot at that offset.
    pub sign_command: Option<&'a str>,
    /// Compare the bytes of every untouched resource too (reads the base
    /// twice) instead of only the moved and changed ones.
    pub deep_verify: bool,
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct RangeIdentity {
    pub offset: u64,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct SignatureSlot {
    pub offset: u64,
    pub size: usize,
    /// `signed` once a signer filled the slot, else `unsigned`.
    pub status: String,
    pub signer: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MovedPayload {
    pub path: String,
    pub size: u32,
    pub from: u64,
    pub to: u64,
}

#[derive(Debug, Serialize)]
pub struct InPlace {
    pub path: String,
    pub offset: u64,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct Replacement {
    pub path: String,
    pub file: String,
    pub file_sha256: String,
    /// The bytes the final database holds (with the packs' edits, padded).
    pub size: u64,
    pub sha256: String,
    /// Written over the stock entry (same size and hash), else in the tail.
    pub in_place: bool,
}

/// The release's record, `<final db>.json`: what went in and what came out.
#[derive(Debug, Serialize)]
pub struct ReleaseManifest {
    pub format_version: u32,
    pub target_build: String,
    pub base: DbIdentity,
    pub output: DbIdentity,
    /// The bytes over the base's start: header, table, signature, moved space.
    pub head: RangeIdentity,
    /// The base's bytes over the same range.
    pub head_original_sha256: String,
    pub tail: RangeIdentity,
    pub signature: SignatureSlot,
    pub moved_payloads: Vec<MovedPayload>,
    /// Files written over their stock entries (same size and hash).
    pub in_place: Vec<InPlace>,
    pub replaced: Vec<Replacement>,
    pub packs: Vec<InstalledPack>,
    pub added_files: Vec<String>,
    pub replaced_files: Vec<String>,
    pub edited_files: Vec<String>,
    pub notes: Vec<String>,
}

const HEAD: &str = "resources-head.bin";
const TAIL: &str = "resources-tail.bin";

/// The release record's path for a final database.
pub fn record_path(output_db: &Path) -> PathBuf {
    let mut name = output_db.as_os_str().to_owned();
    name.push(".json");
    PathBuf::from(name)
}

fn sha256_range(path: &Path, offset: u64, len: u64) -> Result<String> {
    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut hasher = Sha256::new();
    let copied = std::io::copy(&mut file.take(len), &mut hasher)?;
    ensure!(
        copied == len,
        "{} ends inside [{offset}, +{len})",
        path.display()
    );
    Ok(format!("{:x}", hasher.finalize()))
}

/// Fill the file-table signature slot of `head`. This is the head's last
/// change. Without a signer the slot stays zeroed and the release is
/// recorded as unsigned.
pub fn sign_file_table(head: &Path, offset: u64, signer: Option<&str>) -> Result<SignatureSlot> {
    let Some(signer) = signer else {
        return Ok(SignatureSlot {
            offset,
            size: SIGNATURE_LEN,
            status: "unsigned".into(),
            signer: None,
        });
    };
    let before = fs::read(head)?;
    let mut words = signer.split_whitespace();
    let program = words.next().context("the sign command is empty")?;
    let status = Command::new(program)
        .args(words)
        .arg(head)
        .arg(offset.to_string())
        .status()
        .with_context(|| format!("starting the signer {signer}"))?;
    if !status.success() {
        bail!("the signer {signer} failed: {status}");
    }
    let after = fs::read(head)?;
    let slot = offset as usize..offset as usize + SIGNATURE_LEN;
    if after.len() != before.len()
        || after[..slot.start] != before[..slot.start]
        || after[slot.end..] != before[slot.end..]
    {
        bail!("the signer changed the head outside the signature slot");
    }
    if after[slot].iter().all(|b| *b == 0) {
        bail!("the signer left the signature slot empty");
    }
    Ok(SignatureSlot {
        offset,
        size: SIGNATURE_LEN,
        status: "signed".into(),
        signer: Some(signer.to_owned()),
    })
}

/// `bytes` as they can stand in place of the stock entry `stock`: as they
/// are when size and hash match, or, for a movie that is shorter, padded to
/// the entry's size with the entry's hash (the player ignores bytes past the
/// movie's own length). `None` when they must go to the tail.
pub(super) fn in_place_bytes(path: &str, bytes: &[u8], stock: &[u8]) -> Result<Option<Vec<u8>>> {
    let hash = resources_db::pack::crc32;
    if bytes.len() == stock.len() && hash(bytes) == hash(stock) {
        return Ok(Some(bytes.to_vec()));
    }
    if !path.ends_with(".swf") || bytes.len() + 4 > stock.len() {
        return Ok(None);
    }
    let mut padded = bytes.to_vec();
    padded.resize(stock.len() - 4, 0);
    let trailer = crc_trailer(&padded, crc32fast::hash(stock))?;
    padded.extend_from_slice(&trailer);
    ensure!(
        hash(&padded) == hash(stock),
        "{path}: padding did not reach the entry's hash"
    );
    Ok(Some(padded))
}

pub fn release(options: &ReleaseOptions<'_>) -> Result<ReleaseManifest> {
    let output_db = options.output_db;
    let record = record_path(output_db);
    for path in [output_db, record.as_path()] {
        if path.exists() && !options.force {
            bail!(
                "release output already exists (use --force): {}",
                path.display()
            );
        }
    }
    let packs = load_packs(options.packs)?;
    if packs.is_empty() {
        bail!("at least one pack is required");
    }
    let base_sha = sha256_file(options.base_db)?;
    let base_len = fs::metadata(options.base_db)?.len();
    let mut base = ResourceDb::open(options.base_db)?;
    let mut plan = Plan::new(&packs, &base_languages(&base));
    let mut replaced_from = BTreeMap::new();
    for (path, file) in options.replace {
        let bytes = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        ensure!(
            replaced_from
                .insert(path.clone(), (file.clone(), sha256_hex(&bytes)))
                .is_none(),
            "{path} is replaced twice"
        );
        plan.replace(path, bytes);
    }
    let resolved = plan.resolve(&mut base, Some(&base_sha))?;

    // What each touched file holds in the final database, and whether it
    // can be written over its stock entry.
    let mut finals = BTreeMap::<String, Vec<u8>>::new();
    let mut in_place = BTreeMap::<String, u64>::new();
    for (path, content) in &resolved.files {
        let mut bytes = content.bytes()?;
        let stored = base.entries().get(path).and_then(|entry| {
            (entry.inline_buffer_offset.is_none()
                && entry.size_in_db != 0
                && entry.size_in_db == entry.uncompressed_size)
                .then_some(entry.pos_in_db)
        });
        if let Some(offset) = stored.filter(|_| base.is_file(path))
            && let Some(fitted) = in_place_bytes(path, &bytes, &base.read(path)?)?
        {
            in_place.insert(path.clone(), offset);
            bytes = fitted;
        }
        finals.insert(path.clone(), bytes);
    }
    let donor = build_donor(
        options.base_db,
        resolved
            .files
            .iter()
            .filter(|(path, _)| !in_place.contains_key(*path)),
        options.work_dir,
        options.force,
    )?;
    let materialized: Vec<Vec<u8>> = resolved
        .materialize
        .iter()
        .map(|path| path.clone().into_bytes())
        .collect();
    let (head, tail) = (options.work_dir.join(HEAD), options.work_dir.join(TAIL));
    let patch: HeadroomPatch =
        headroom::build_headroom_overlay(options.base_db, &donor, &materialized, &head, &tail)?;
    fs::remove_file(&donor)?;
    for (path, offset) in &in_place {
        ensure!(
            *offset >= patch.head_len,
            "{path} at {offset} lies inside the head [0, {})",
            patch.head_len
        );
    }
    let signature = sign_file_table(&head, patch.fat_end, options.sign_command)?;

    // The final database: base, head over its start, in-place files, tail.
    if let Some(parent) = output_db.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut partial = output_db.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    {
        let mut out = fs::File::create(&partial)?;
        std::io::copy(&mut fs::File::open(options.base_db)?, &mut out)?;
        out.seek(SeekFrom::Start(0))?;
        out.write_all(&fs::read(&head)?)?;
        for (path, offset) in &in_place {
            out.seek(SeekFrom::Start(*offset))?;
            out.write_all(&finals[path])?;
        }
        out.seek(SeekFrom::End(0))?;
        std::io::copy(&mut fs::File::open(&tail)?, &mut out)?;
        out.sync_all()?;
    }
    fs::rename(&partial, output_db)?;
    ensure!(
        fs::metadata(output_db)?.len() == patch.new_len,
        "the final database is not {} bytes",
        patch.new_len
    );

    // Everything below reads the final database as written.
    let mut output = ResourceDb::open(output_db)?;
    for (path, bytes) in &finals {
        if sha256_hex(bytes) != sha256_hex(&output.read(path)?) {
            bail!("released resource {path} does not hold its composed bytes");
        }
    }
    for (path, offset) in &in_place {
        let entry = output
            .entries()
            .get(path)
            .with_context(|| format!("{path} is gone from the final table"))?;
        ensure!(
            entry.pos_in_db == *offset,
            "{path} moved although it was written in place"
        );
    }
    let operations = verify_in(&mut output, options.packs, &[])?;
    println!("verified_operations={operations}");
    let changed: BTreeSet<Vec<u8>> = finals
        .keys()
        .map(|path| path.clone().into_bytes())
        .collect();
    let check = headroom::verify_untouched_file(
        options.base_db,
        output_db,
        patch.head_len,
        &|path| changed.contains(path),
        options.deep_verify,
    )?;
    println!(
        "untouched={} moved={} compared={} in_place={}",
        check.untouched,
        check.moved,
        check.compared,
        in_place.len()
    );

    let replaced = replaced_from
        .into_iter()
        .map(|(path, (file, file_sha256))| Replacement {
            size: finals[&path].len() as u64,
            sha256: sha256_hex(&finals[&path]),
            in_place: in_place.contains_key(&path),
            file: file.display().to_string(),
            file_sha256,
            path,
        })
        .collect();
    let manifest = ReleaseManifest {
        format_version: 2,
        target_build: TARGET_BUILD.into(),
        base: DbIdentity::of(options.base_db, base_sha)?,
        output: DbIdentity::of(output_db, sha256_file(output_db)?)?,
        head: RangeIdentity {
            offset: 0,
            size: patch.head_len,
            sha256: sha256_range(output_db, 0, patch.head_len)?,
        },
        head_original_sha256: sha256_range(options.base_db, 0, patch.head_len)?,
        tail: RangeIdentity {
            offset: base_len,
            size: patch.tail_len,
            sha256: sha256_range(output_db, base_len, patch.tail_len)?,
        },
        signature,
        moved_payloads: patch
            .moved
            .iter()
            .map(|m| MovedPayload {
                path: m.path.clone(),
                size: m.size,
                from: m.from,
                to: m.to,
            })
            .collect(),
        in_place: in_place
            .iter()
            .map(|(path, offset)| InPlace {
                path: path.clone(),
                offset: *offset,
                size: finals[path].len() as u64,
                sha256: sha256_hex(&finals[path]),
            })
            .collect(),
        replaced,
        packs: installed_packs(&packs),
        added_files: kinds_of(&resolved, WholeKind::Add),
        replaced_files: kinds_of(&resolved, WholeKind::Replace),
        edited_files: resolved.edited.iter().cloned().collect(),
        notes: resolved.notes.clone(),
    };
    fs::write(&record, serde_json::to_vec_pretty(&manifest)?)?;
    fs::remove_file(&head)?;
    fs::remove_file(&tail)?;
    Ok(manifest)
}

/// Four trailing bytes that give `prefix ++ bytes` the CRC-32 `target`. The
/// engine's archive hash is the same polynomial with other initial and final
/// bits, so equal-length data with equal CRC-32s also hash equal there.
fn crc_trailer(prefix: &[u8], target: u32) -> Result<[u8; 4]> {
    let mut state = crc32fast::Hasher::new();
    state.update(prefix);
    let crc_of = |tail: u32| {
        let mut hasher = state.clone();
        hasher.update(&tail.to_le_bytes());
        hasher.finalize()
    };
    let zero = crc_of(0);
    // Gaussian elimination over GF(2): basis vectors keyed by leading bit.
    let mut basis: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for bit in 0..32 {
        let mut vector = crc_of(1 << bit) ^ zero;
        let mut mask = 1_u32 << bit;
        while vector != 0 {
            let pivot = 31 - vector.leading_zeros();
            match basis.get(&pivot) {
                Some(&(other, other_mask)) => {
                    vector ^= other;
                    mask ^= other_mask;
                }
                None => {
                    basis.insert(pivot, (vector, mask));
                    break;
                }
            }
        }
    }
    let (mut vector, mut mask) = (target ^ zero, 0_u32);
    while vector != 0 {
        let (other, other_mask) = basis
            .get(&(31 - vector.leading_zeros()))
            .context("CRC trailer is not solvable")?;
        vector ^= other;
        mask ^= other_mask;
    }
    let trailer = mask.to_le_bytes();
    ensure!(
        crc_of(mask) == target,
        "CRC trailer does not reach its target"
    );
    Ok(trailer)
}
