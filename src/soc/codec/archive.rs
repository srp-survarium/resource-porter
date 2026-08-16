//! The X-Ray `.db` / `.xdb` archive container.
//!
//! An archive is the same `u32 id / u32 size / payload` chunk sequence used
//! everywhere else in X-Ray, with two chunks that matter:
//!
//! * chunk `0` — every file payload concatenated into one blob;
//! * chunk `1` — the file table, which is LZHUF-compressed and, for retail
//!   Shadow of Chernobyl, also obfuscated;
//! * chunk `666` — an optional plain-text INI header. Its absence is how the
//!   engine decides an archive is an obfuscated retail one.
//!
//! Payload blobs are hundreds of megabytes, so the reader keeps a file handle
//! and seeks rather than loading the archive.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::cipher::{Decoder, Key};
use super::lzhuf;

/// `CFS_CompressMark`.
const COMPRESS_MARK: u32 = 1 << 31;
/// `CFS_HeaderChunkID`.
const HEADER_CHUNK: u32 = 666;
/// The file-table chunk.
const TABLE_CHUNK: u32 = 1;
/// A table record is `u16 size, u32 size_real, u32 size_compressed, u32 crc,
/// name..., u32 ptr`, where `size` counts only the members *after* itself. So
/// the record occupies `2 + size` bytes and the name is `size - ELEMENTS_SIZE`
/// long.
const TABLE_RECORD_ELEMENTS: usize = 16;
/// Ceiling for a decompressed file table.
const MAX_TABLE_BYTES: usize = 256 << 20;

/// One file inside an archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// The name as stored, with X-Ray's backslash separators.
    pub name: String,
    /// Absolute byte offset of the payload within the archive.
    pub offset: u32,
    pub size: u32,
    pub stored_size: u32,
    pub crc: u32,
}

impl ArchiveEntry {
    /// Whether the payload is LZO-compressed rather than stored.
    pub fn is_compressed(&self) -> bool {
        self.stored_size != self.size
    }
}

/// One opened archive.
#[derive(Debug)]
pub struct SocArchive {
    path: PathBuf,
    file: File,
    entries: Vec<ArchiveEntry>,
    /// The plain-text INI header, when the archive carries one.
    pub header: Option<String>,
    /// Which key reversed the file table, or `None` for a plain archive.
    pub key: Option<Key>,
}

/// The decoded, decompressed file table of one archive.
///
/// Exposed on its own because a table that decodes but does not parse is the
/// symptom of a codec bug, and the bytes are the only way to tell which.
pub fn decoded_file_table(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    Ok(SocArchive::open_parts(path.as_ref())?.2)
}

impl SocArchive {
    /// Read the chunk layout and reverse the file table's transforms.
    fn open_parts(path: &Path) -> Result<(File, Option<String>, Vec<u8>, Option<Key>)> {
        let path = path.to_path_buf();
        let mut file =
            File::open(&path).with_context(|| format!("opening archive {}", path.display()))?;
        let length = file
            .metadata()
            .with_context(|| format!("reading metadata of {}", path.display()))?
            .len();

        let mut header = None;
        let mut table = None;
        let mut offset = 0_u64;
        while offset < length {
            let (id, size) = read_chunk_header(&mut file, offset, &path)?;
            let payload_start = offset + 8;
            let bare = id & !COMPRESS_MARK;
            if bare == HEADER_CHUNK || bare == TABLE_CHUNK {
                let payload = read_exact_at(&mut file, payload_start, size as usize, &path)?;
                if bare == HEADER_CHUNK {
                    header = Some(String::from_utf8_lossy(&payload).into_owned());
                } else {
                    table = Some((payload, id & COMPRESS_MARK != 0));
                }
            }
            offset = payload_start
                .checked_add(u64::from(size))
                .with_context(|| format!("chunk size overflow in {}", path.display()))?;
        }

        let (payload, compressed) = table
            .with_context(|| format!("{} has no file table chunk", path.display()))?;

        // The engine treats a missing INI header as the signal that a non-`.xdb`
        // archive is obfuscated.
        let obfuscated = header.is_none()
            && !path
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains(".xdb");
        let (table_bytes, key) = decode_table(&payload, compressed, obfuscated, &path)?;
        Ok((file, header, table_bytes, key))
    }

    /// Open an archive and decode its file table.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let (file, header, table_bytes, key) = Self::open_parts(&path)?;
        let entries = parse_table(&table_bytes, &path)?;
        Ok(Self {
            path,
            file,
            entries,
            header,
            key,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    /// Read one entry's payload.
    pub fn read(&mut self, entry: &ArchiveEntry) -> Result<Vec<u8>> {
        if entry.is_compressed() {
            // Per-file payloads use LZO1X, which retail Shadow of Chernobyl
            // does not appear to use. Fail by name rather than return wrong
            // bytes.
            bail!(
                "{}: {} is LZO-compressed ({} stored, {} real); per-file decompression is not \
                 implemented",
                self.path.display(),
                entry.name,
                entry.stored_size,
                entry.size
            );
        }
        read_exact_at(
            &mut self.file,
            u64::from(entry.offset),
            entry.size as usize,
            &self.path,
        )
    }
}

fn read_chunk_header(file: &mut File, offset: u64, path: &Path) -> Result<(u32, u32)> {
    let mut header = [0_u8; 8];
    file.seek(SeekFrom::Start(offset))
        .with_context(|| format!("seeking to byte {offset} of {}", path.display()))?;
    file.read_exact(&mut header)
        .with_context(|| format!("reading a chunk header at byte {offset} of {}", path.display()))?;
    Ok((
        u32::from_le_bytes(header[0..4].try_into().unwrap()),
        u32::from_le_bytes(header[4..8].try_into().unwrap()),
    ))
}

fn read_exact_at(file: &mut File, offset: u64, size: usize, path: &Path) -> Result<Vec<u8>> {
    let mut payload = vec![0_u8; size];
    file.seek(SeekFrom::Start(offset))
        .with_context(|| format!("seeking to byte {offset} of {}", path.display()))?;
    file.read_exact(&mut payload)
        .with_context(|| format!("reading {size} bytes at byte {offset} of {}", path.display()))?;
    Ok(payload)
}

/// Reverse whichever transforms the file table carries.
///
/// The engine does not record which key an archive used, so an obfuscated
/// table is tried worldwide-first and then Russian, exactly as the engine does.
fn decode_table(
    payload: &[u8],
    compressed: bool,
    obfuscated: bool,
    path: &Path,
) -> Result<(Vec<u8>, Option<Key>)> {
    if !compressed {
        return Ok((payload.to_vec(), None));
    }
    if !obfuscated {
        return Ok((
            lzhuf::decompress(payload, MAX_TABLE_BYTES)
                .with_context(|| format!("decompressing the file table of {}", path.display()))?,
            None,
        ));
    }
    let mut failure = None;
    for key in [Key::Worldwide, Key::Russian] {
        let decoded = Decoder::new(key).decode(payload);
        match lzhuf::decompress(&decoded, MAX_TABLE_BYTES) {
            Ok(table) => return Ok((table, Some(key))),
            Err(error) => failure = Some(error),
        }
    }
    Err(failure.expect("at least one key was tried")).with_context(|| {
        format!(
            "decoding the file table of {} with either shipped key",
            path.display()
        )
    })
}

fn parse_table(bytes: &[u8], path: &Path) -> Result<Vec<ArchiveEntry>> {
    let mut entries = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let declared = usize::from(u16::from_le_bytes(
            bytes
                .get(offset..offset + 2)
                .with_context(|| format!("truncated file table in {}", path.display()))?
                .try_into()
                .unwrap(),
        ));
        if declared <= TABLE_RECORD_ELEMENTS {
            bail!(
                "{}: file table record {} at byte {offset} declares {declared} bytes, leaving no \
                 room for a name after its {TABLE_RECORD_ELEMENTS} fixed bytes (table is {} bytes; \
                 previous entry {:?})",
                path.display(),
                entries.len(),
                bytes.len(),
                entries.last().map(|entry: &ArchiveEntry| entry.name.as_str())
            );
        }
        let total = 2 + declared;
        let record = bytes
            .get(offset..offset + total)
            .with_context(|| format!("truncated file table record in {}", path.display()))?;
        let read = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
        let name_end = total - 4;
        let name = std::str::from_utf8(&record[14..name_end])
            .with_context(|| format!("non-ASCII file name in {}", path.display()))?
            .trim_end_matches('\0')
            .to_owned();
        if name.is_empty() {
            bail!(
                "{}: file table record at byte {offset} has no name",
                path.display()
            );
        }
        entries.push(ArchiveEntry {
            name,
            size: read(2),
            stored_size: read(6),
            crc: read(10),
            offset: read(name_end),
        });
        offset += total;
    }
    if entries.is_empty() {
        bail!("{} has an empty file table", path.display());
    }
    Ok(entries)
}

/// Several archives resolved as one namespace.
///
/// X-Ray registers archives in order and lets a later registration replace an
/// earlier one, which is how the patch archives (`gamedata.dba`..`dbd`)
/// supersede the base ones. The set preserves that: later archives win.
pub struct SocArchiveSet {
    archives: Vec<SocArchive>,
    /// Normalised name -> (archive index, entry index).
    index: BTreeMap<String, (usize, usize)>,
}

impl SocArchiveSet {
    /// Open every archive in `paths`, in increasing precedence order.
    pub fn open(paths: &[PathBuf]) -> Result<Self> {
        if paths.is_empty() {
            bail!("no archives to open");
        }
        let mut archives = Vec::with_capacity(paths.len());
        let mut index = BTreeMap::new();
        for (archive_index, path) in paths.iter().enumerate() {
            let archive = SocArchive::open(path)?;
            for (entry_index, entry) in archive.entries().iter().enumerate() {
                index.insert(normalize(&entry.name), (archive_index, entry_index));
            }
            archives.push(archive);
        }
        Ok(Self { archives, index })
    }

    /// Open the `gamedata.db*` archives of a retail installation, in the order
    /// the engine registers them.
    pub fn open_installation(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let mut paths: Vec<PathBuf> = std::fs::read_dir(root)
            .with_context(|| format!("listing {}", root.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        let name = name.to_ascii_lowercase();
                        name.starts_with("gamedata.db") && name.len() == "gamedata.dbX".len()
                    })
            })
            .collect();
        if paths.is_empty() {
            bail!("{} contains no gamedata.db* archives", root.display());
        }
        // `db0`..`db9` then `dba`..`dbd`: ASCII order is registration order.
        paths.sort();
        Self::open(&paths)
    }

    pub fn archives(&self) -> &[SocArchive] {
        &self.archives
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Every stored name, normalised, in sorted order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(String::as_str)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(&normalize(name))
    }

    /// Read one file by name, accepting either separator and any case.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let (archive_index, entry_index) = *self
            .index
            .get(&normalize(name))
            .with_context(|| format!("{name} is not present in any archive"))?;
        let archive = &mut self.archives[archive_index];
        let entry = archive.entries[entry_index].clone();
        archive.read(&entry)
    }
}

/// X-Ray writes names with backslashes and matches them case-insensitively.
fn normalize(name: &str) -> String {
    name.replace('\\', "/").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = id.to_le_bytes().to_vec();
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn record(name: &str, offset: u32, size: u32, stored: u32) -> Vec<u8> {
        // The declared size covers everything after the u16 itself.
        let declared = TABLE_RECORD_ELEMENTS + name.len();
        let mut bytes = (declared as u16).to_le_bytes().to_vec();
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&stored.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes
    }

    /// Build a plain (uncompressed table, no INI header) archive.
    fn write_archive(directory: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let mut blob = Vec::new();
        let mut table = Vec::new();
        // The payload blob is chunk 0, so payloads start after its header.
        let base = 8_u32;
        for (file_name, payload) in files {
            table.push(record(
                file_name,
                base + blob.len() as u32,
                payload.len() as u32,
                payload.len() as u32,
            ));
            blob.extend_from_slice(payload);
        }
        let mut bytes = chunk(0, &blob);
        bytes.extend(chunk(TABLE_CHUNK, &table.concat()));

        let path = directory.join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(&bytes).unwrap();
        path
    }

    #[test]
    fn reads_a_plain_archive() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_archive(
            directory.path(),
            "gamedata.xdb0",
            &[
                ("levels\\l01_escape\\level", b"LEVEL"),
                ("config\\gamemtl.xr", b"MTL"),
            ],
        );
        let mut archive = SocArchive::open(&path).unwrap();
        assert_eq!(archive.entries().len(), 2);
        assert_eq!(archive.key, None);
        let entry = archive.entries()[0].clone();
        assert_eq!(entry.name, "levels\\l01_escape\\level");
        assert!(!entry.is_compressed());
        assert_eq!(archive.read(&entry).unwrap(), b"LEVEL");
    }

    #[test]
    fn later_archives_supersede_earlier_ones() {
        let directory = tempfile::tempdir().unwrap();
        let first = write_archive(directory.path(), "a.xdb0", &[("config\\x.ltx", b"OLD")]);
        let second = write_archive(directory.path(), "b.xdb0", &[("config\\x.ltx", b"NEW")]);
        let mut set = SocArchiveSet::open(&[first, second]).unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set.read("config/x.ltx").unwrap(), b"NEW");
    }

    #[test]
    fn resolves_names_case_insensitively_and_by_either_separator() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_archive(
            directory.path(),
            "a.xdb0",
            &[("Levels\\L01_Escape\\level", b"L")],
        );
        let mut set = SocArchiveSet::open(&[path]).unwrap();
        assert!(set.contains("levels/l01_escape/level"));
        assert!(set.contains("LEVELS\\L01_ESCAPE\\LEVEL"));
        assert_eq!(set.read("levels/l01_escape/level").unwrap(), b"L");
    }

    #[test]
    fn reports_a_compressed_payload_by_name_instead_of_guessing() {
        let directory = tempfile::tempdir().unwrap();
        let mut blob = b"XXXX".to_vec();
        blob.extend_from_slice(b"pad");
        let mut bytes = chunk(0, &blob);
        bytes.extend(chunk(TABLE_CHUNK, &record("a\\b.dds", 8, 16, 4)));
        let path = directory.path().join("c.xdb0");
        File::create(&path).unwrap().write_all(&bytes).unwrap();

        let mut archive = SocArchive::open(&path).unwrap();
        let entry = archive.entries()[0].clone();
        assert!(entry.is_compressed());
        let error = archive.read(&entry).unwrap_err().to_string();
        assert!(error.contains("LZO-compressed"), "{error}");
        assert!(error.contains("a\\b.dds"), "{error}");
    }

    #[test]
    fn rejects_an_archive_without_a_file_table() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("d.xdb0");
        File::create(&path).unwrap().write_all(&chunk(0, b"x")).unwrap();
        let error = SocArchive::open(&path).unwrap_err().to_string();
        assert!(error.contains("no file table chunk"), "{error}");
    }

    #[test]
    fn rejects_a_truncated_table_record() {
        let directory = tempfile::tempdir().unwrap();
        let mut bytes = chunk(0, b"x");
        bytes.extend(chunk(TABLE_CHUNK, &[4_u8, 0]));
        let path = directory.path().join("e.xdb0");
        File::create(&path).unwrap().write_all(&bytes).unwrap();
        let error = SocArchive::open(&path).unwrap_err().to_string();
        assert!(error.contains("no room for a name"), "{error}");
    }
}
