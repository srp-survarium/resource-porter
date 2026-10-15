// SPDX-License-Identifier: GPL-3.0-or-later
//! `pack release`: compose packs into the blobs an installed client needs.
//!
//! Instead of a whole new `resources.db`, a release is the retail file plus
//! a same-size head region (header, larger file table, signature slot) and an
//! appended tail (new and changed payloads, and the few base payloads the
//! larger table displaced); see [`resources_db::headroom`]. Paths listed as
//! excluded (the lobby movie, which ships as its own updater region) stay out
//! of the archive; their composed bytes are written next to the blobs for
//! reference only.
//!
//! The file table is signed in the shipped archive. Signing the new table is
//! the last step: [`sign_file_table`] runs the configured signer, or leaves
//! the slot zeroed and records the release as unsigned.

use std::io::Read;
use std::process::Command;

use anyhow::ensure;
use resources_db::headroom::{self, HeadroomPatch, OverlaidFile, SIGNATURE_LEN};

use super::*;

/// Where the release's archive lives inside a client installation.
pub const CLIENT_DB_PATH: &str = "game/resources.db";

pub struct ReleaseOptions<'a> {
    pub base_db: &'a Path,
    pub packs: &'a [PathBuf],
    pub work_dir: &'a Path,
    pub output_dir: &'a Path,
    /// Composed paths kept out of the archive.
    pub exclude: &'a [String],
    pub server_export: Option<&'a Path>,
    /// Signer run as `<command words…> <head file> <signature offset>`; it
    /// must write exactly the 256-byte slot at that offset.
    pub sign_command: Option<&'a str>,
    /// Compare the bytes of every untouched resource too (reads the base
    /// twice) instead of only the moved and changed ones.
    pub deep_verify: bool,
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct BlobIdentity {
    pub file: String,
    pub offset: u64,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct HeadIdentity {
    #[serde(flatten)]
    pub blob: BlobIdentity,
    /// The base's bytes over the same range.
    pub original_sha256: String,
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
pub struct ExcludedFile {
    pub path: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct ReleaseManifest {
    pub format_version: u32,
    pub target_build: String,
    /// The archive's path inside a client installation.
    pub path: String,
    pub base: DbIdentity,
    pub output: DbIdentity,
    pub head: HeadIdentity,
    pub tail: BlobIdentity,
    pub signature: SignatureSlot,
    pub moved_payloads: Vec<MovedPayload>,
    pub excluded: Vec<ExcludedFile>,
    /// Updater plan lines: the head as a region of the original-size file,
    /// and the tail as an append.
    pub plan_lines: Vec<String>,
    pub packs: Vec<InstalledPack>,
    pub added_files: Vec<String>,
    pub replaced_files: Vec<String>,
    pub edited_files: Vec<String>,
    pub notes: Vec<String>,
}

pub const MANIFEST: &str = "release.json";
pub const HEAD: &str = "resources-head.bin";
pub const TAIL: &str = "resources-tail.bin";

fn sha256_range(path: &Path, len: u64) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut reader = fs::File::open(path)?.take(len);
    std::io::copy(&mut reader, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Fill the file-table signature slot of `head`. This is the release's last
/// write: nothing may change the head afterwards. Without a signer the slot
/// stays zeroed and the release is marked unsigned; clients then need the
/// table check skipped (or our key, once the exe accepts it).
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

pub fn release(options: &ReleaseOptions<'_>) -> Result<ReleaseManifest> {
    let out = options.output_dir;
    if out.exists() {
        if !options.force {
            bail!(
                "release output already exists (use --force): {}",
                out.display()
            );
        }
        fs::remove_dir_all(out)?;
    }
    fs::create_dir_all(out)?;
    if let Some(export) = options.server_export {
        if options.force && export.exists() {
            fs::remove_dir_all(export)?;
        }
        crate::weapon::prepare_empty_output(export)?;
    }
    let packs = load_packs(options.packs)?;
    if packs.is_empty() {
        bail!("at least one pack is required");
    }
    let base_sha = sha256_file(options.base_db)?;
    let mut base = ResourceDb::open(options.base_db)?;
    let plan = Plan::new(&packs, &base_languages(&base));
    let resolved = plan.resolve(&mut base, Some(&base_sha))?;

    let mut excluded = Vec::new();
    for path in options.exclude {
        let content = resolved
            .files
            .get(path)
            .with_context(|| format!("excluded path {path} is not part of the composition"))?;
        let bytes = content.bytes()?;
        let file = format!("excluded/{path}");
        let destination = safe_join(out, &file)?;
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::write(&destination, &bytes)?;
        excluded.push(ExcludedFile {
            path: path.clone(),
            file,
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
    }
    let kept = |path: &String| !options.exclude.contains(path);
    let donor = build_donor(
        options.base_db,
        resolved.files.iter().filter(|(path, _)| kept(path)),
        options.work_dir,
        options.force,
    )?;
    let materialized: Vec<Vec<u8>> = resolved
        .materialize
        .iter()
        .map(|path| path.clone().into_bytes())
        .collect();
    let (head, tail) = (out.join(HEAD), out.join(TAIL));
    let patch: HeadroomPatch =
        headroom::build_headroom_overlay(options.base_db, &donor, &materialized, &head, &tail)?;
    fs::remove_file(&donor)?;
    let signature = sign_file_table(&head, patch.fat_end, options.sign_command)?;

    // Everything below reads the client's resulting archive virtually.
    let overlaid =
        || -> Result<OverlaidFile> { Ok(OverlaidFile::open(options.base_db, &head, &tail)?) };
    let mut output = ResourceDb::from_reader(Box::new(overlaid()?))?;
    for (path, content) in resolved.files.iter().filter(|(path, _)| kept(path)) {
        if sha256_hex(&content.bytes()?) != sha256_hex(&output.read(path)?) {
            bail!("released resource {path} does not match its effective content");
        }
    }
    for path in options.exclude {
        if sha256_hex(&output.read(path)?) != sha256_hex(&base.read(path)?) {
            bail!("excluded resource {path} changed in the archive");
        }
    }
    let operations = verify_in(&mut output, options.packs, options.exclude)?;
    println!("verified_operations={operations}");
    let changed: BTreeSet<Vec<u8>> = resolved
        .files
        .keys()
        .filter(|path| kept(path))
        .map(|path| path.clone().into_bytes())
        .collect();
    let check = headroom::verify_untouched(
        options.base_db,
        &head,
        &tail,
        &|path| changed.contains(path),
        options.deep_verify,
    )?;
    println!(
        "untouched={} moved={} compared={}",
        check.untouched, check.moved, check.compared
    );

    let mut hasher = Sha256::new();
    std::io::copy(&mut overlaid()?, &mut hasher)?;
    let output_sha = format!("{:x}", hasher.finalize());
    let head_identity = HeadIdentity {
        blob: BlobIdentity {
            file: HEAD.into(),
            offset: 0,
            size: patch.head_len,
            sha256: sha256_file(&head)?,
        },
        original_sha256: sha256_range(options.base_db, patch.head_len)?,
    };
    let tail_identity = BlobIdentity {
        file: TAIL.into(),
        offset: patch.base_len,
        size: patch.tail_len,
        sha256: sha256_file(&tail)?,
    };
    let plan_lines = vec![
        format!(
            "region {CLIENT_DB_PATH} {} 0 {} {} {} resources_head",
            patch.base_len,
            patch.head_len,
            head_identity.original_sha256,
            head_identity.blob.sha256
        ),
        format!(
            "tail {CLIENT_DB_PATH} {} {} {} resources_tail",
            patch.base_len, patch.new_len, tail_identity.sha256
        ),
    ];
    let manifest = ReleaseManifest {
        format_version: 1,
        target_build: TARGET_BUILD.into(),
        path: CLIENT_DB_PATH.into(),
        base: DbIdentity::of(options.base_db, base_sha)?,
        output: DbIdentity {
            path: format!("{} + {HEAD} + {TAIL}", options.base_db.display()),
            size: patch.new_len,
            sha256: output_sha,
        },
        head: head_identity,
        tail: tail_identity,
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
        excluded,
        plan_lines,
        packs: installed_packs(&packs),
        added_files: kinds_of(&resolved, WholeKind::Add),
        replaced_files: kinds_of(&resolved, WholeKind::Replace),
        edited_files: resolved.edited.iter().cloned().collect(),
        notes: resolved.notes.clone(),
    };
    fs::write(out.join(MANIFEST), serde_json::to_vec_pretty(&manifest)?)?;
    if let Some(export) = options.server_export {
        let record = InstallRecord {
            format_version: 1,
            target_build: TARGET_BUILD.into(),
            base_db: DbIdentity::of(options.base_db, manifest.base.sha256.clone())?,
            output_db: DbIdentity {
                path: manifest.output.path.clone(),
                size: manifest.output.size,
                sha256: manifest.output.sha256.clone(),
            },
            packs: installed_packs(&packs),
            added_files: manifest.added_files.clone(),
            replaced_files: manifest.replaced_files.clone(),
            edited_files: manifest.edited_files.clone(),
            notes: manifest.notes.clone(),
        };
        export_server(&mut base, &mut output, &resolved, &record, export)?;
    }
    Ok(manifest)
}

pub struct LobbyRegionOptions<'a> {
    /// The lobby movie players have now (e.g. the live updater region).
    pub swf: &'a Path,
    /// The retail archive entry: the region keeps its size and CRC.
    pub original: &'a Path,
    pub packs: &'a [PathBuf],
    pub output: &'a Path,
    /// Required atlas width before drawing (the stock atlas is 855).
    pub expect_width: Option<usize>,
    /// The retail archive, to locate the entry for the updater plan line.
    pub base_db: Option<&'a Path>,
}

#[derive(Debug, Serialize)]
pub struct LobbyRegion {
    pub size: u64,
    pub sha256: String,
    pub input_sha256: String,
    pub original_sha256: String,
    /// Length of the movie itself; the rest is padding and the CRC trailer.
    pub movie_len: u64,
    pub cells: Vec<(u32, u32)>,
    pub atlas_width: (usize, usize),
    /// Updater plan line for this region, when the base archive was given.
    pub plan_line: Option<String>,
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

/// Draw the packs' lobby icons into `swf` and pad the movie to the retail
/// entry's size with its CRC, so the archive's file table entry stays valid.
pub fn lobby_region(options: &LobbyRegionOptions<'_>) -> Result<LobbyRegion> {
    let input = fs::read(options.swf)?;
    let original = fs::read(options.original)?;
    ensure!(
        input.len() == original.len(),
        "the lobby movie is {} bytes, the retail entry {}",
        input.len(),
        original.len()
    );
    let packs = load_packs(options.packs)?;
    let symbol = atlas::DEFAULT_SYMBOL;
    let live = atlas::Swf::parse(&input).context("parsing the lobby movie")?;
    let before = live.bitmap(symbol)?;
    if let Some(width) = options.expect_width {
        ensure!(
            before.width == width,
            "the lobby atlas is {} wide, expected {width}",
            before.width
        );
    }
    let mut bitmap = before.clone();
    let mut cells = Vec::new();
    let mut drawn = Vec::new();
    for pack in &packs {
        for operation in &pack.manifest.operations {
            let Operation::AtlasIconsAdd {
                swf,
                symbol: s,
                icons,
            } = operation
            else {
                continue;
            };
            ensure!(
                swf.as_deref().unwrap_or(atlas::DEFAULT_SWF) == atlas::DEFAULT_SWF
                    && s.as_deref().unwrap_or(symbol) == symbol,
                "{}: only the lobby equipment atlas is supported",
                pack.id()
            );
            for icon in icons {
                let pixels =
                    atlas::read_icon_png(&safe_join(&pack.dir, &icon.source)?, icon.premultiplied)?;
                ensure!(
                    bitmap.cell(icon.row, icon.col)?.iter().all(|b| *b == 0),
                    "{}: atlas cell [{}, {}] is already occupied",
                    pack.id(),
                    icon.row,
                    icon.col
                );
                bitmap.put_cell(icon.row, icon.col, &pixels)?;
                cells.push((icon.row, icon.col));
                drawn.push((icon.row, icon.col, pixels));
            }
        }
    }
    ensure!(!cells.is_empty(), "the packs draw no lobby icons");
    let mut edited = atlas::Swf::parse(&input)?;
    edited.set_bitmap(symbol, &bitmap)?;
    let movie = edited.to_bytes()?;

    // Re-read: only the atlas changed, old pixels kept, new cells drawn.
    let check = atlas::Swf::parse(&movie)?;
    ensure!(
        check.same_except_bitmap(&live, symbol)?,
        "the lobby movie changed outside its atlas"
    );
    let after = check.bitmap(symbol)?;
    ensure!(after.height == before.height, "atlas height changed");
    for line in 0..before.height {
        let old = &before.argb[line * before.width * 4..(line + 1) * before.width * 4];
        let new = &after.argb[line * after.width * 4..line * after.width * 4 + before.width * 4];
        ensure!(old == new, "atlas row {line} changed");
    }
    for (row, col, pixels) in &drawn {
        ensure!(
            &after.cell(*row, *col)? == pixels,
            "cell [{row}, {col}] differs"
        );
    }

    let size = original.len();
    ensure!(
        movie.len() + 4 <= size,
        "the lobby movie grew to {} bytes, the region holds {size}",
        movie.len()
    );
    let mut region = movie.clone();
    region.resize(size - 4, 0);
    let trailer = crc_trailer(&region, crc32fast::hash(&original))?;
    region.extend_from_slice(&trailer);
    ensure!(
        resources_db::pack::crc32(&region) == resources_db::pack::crc32(&original),
        "the region's archive hash differs from the retail entry"
    );
    fs::write(options.output, &region)?;
    let plan_line = match options.base_db {
        Some(db) => {
            let base = ResourceDb::open(db)?;
            let entry = base
                .entries()
                .get(atlas::DEFAULT_SWF)
                .context("the base archive has no lobby movie")?;
            ensure!(
                entry.size_in_db as usize == size && entry.inline_buffer_offset.is_none(),
                "the base lobby entry does not match the retail movie"
            );
            Some(format!(
                "region {CLIENT_DB_PATH} {} {} {size} {} {} lobby_menu",
                fs::metadata(db)?.len(),
                entry.pos_in_db,
                sha256_hex(&original),
                sha256_hex(&region)
            ))
        }
        None => None,
    };
    Ok(LobbyRegion {
        size: size as u64,
        sha256: sha256_hex(&region),
        input_sha256: sha256_hex(&input),
        original_sha256: sha256_hex(&original),
        movie_len: movie.len() as u64,
        cells,
        atlas_width: (before.width, after.width),
        plan_line,
    })
}
