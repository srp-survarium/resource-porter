//! The Jupiter EX `.Arch00` archive container (`LTAR` version 3).
//!
//! Layout, confirmed against every archive in the F.E.A.R. Ultimate Shooter
//! Edition install (29,260 files across 41 archives, all parsed cleanly):
//!
//! ```text
//! 48-byte header   magic "LTAR", u32 version = 3, u32 name dictionary size,
//!                  u32 directory count, u32 file count, three u32s this
//!                  reader preserves but does not interpret, 16-byte GUID
//! name dictionary  NUL-terminated strings, each padded to 4-byte alignment
//! file records     u32 name offset, u64 payload offset, u64 stored size,
//!                  u64 real size, u32 compression — 32 bytes each
//! dir records      u32 name offset, i32 first child, i32 next sibling,
//!                  u32 file count — 16 bytes each, forming a tree
//! ```
//!
//! Files belong to directories *positionally*: walking the directory array in
//! order, each directory owns the next `file_count` records. The tree links
//! (child/sibling) only exist to give directories their full paths.
//!
//! Every file in every retail archive is stored (`compression == 0`), so this
//! reader refuses compressed entries instead of carrying a decompressor for a
//! case that never occurs. If a future archive proves that wrong, the failure
//! names the file.
//!
//! Archives run to 3.8 GiB, so the reader keeps a file handle and seeks
//! rather than loading the archive.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::soc::codec::chunks::Cursor;

const MAGIC: &[u8; 4] = b"LTAR";
const VERSION: u32 = 3;
const HEADER_SIZE: usize = 48;
const FILE_RECORD_SIZE: usize = 32;
const DIR_RECORD_SIZE: usize = 16;

/// One file inside an archive, with its directory path already joined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchEntry {
    /// Full path as stored: backslash separators, original case.
    pub path: String,
    /// Absolute byte offset of the payload within the archive.
    pub offset: u64,
    /// Bytes occupied in the archive.
    pub stored_size: u64,
    /// Bytes after decompression; equal to `stored_size` for stored entries.
    pub size: u64,
    /// `0` for stored. Anything else is unread — see the module docs.
    pub compression: u32,
}

/// One opened archive.
#[derive(Debug)]
pub struct Arch00 {
    path: PathBuf,
    file: File,
    entries: Vec<ArchEntry>,
}

/// Lower-case a stored path for lookups, forward slashes folded to backslashes.
///
/// The archives themselves disagree on case — `FEAR_7.Arch00` stores
/// `worlds\worlds\releasemultiplayer` where every other archive stores
/// `Worlds\Worlds\ReleaseMultiplayer` — and the engine treats them as one
/// directory, so lookups must too.
pub fn normalize(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

impl Arch00 {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut file =
            File::open(path).with_context(|| format!("opening archive {}", path.display()))?;
        let label = path.display().to_string();

        let mut header = [0u8; HEADER_SIZE];
        file.read_exact(&mut header)
            .with_context(|| format!("{label}: reading the 48-byte header"))?;
        let mut cursor = Cursor::new(&header, &label);
        let magic = cursor.take(4)?;
        if magic != MAGIC {
            bail!("{label}: magic is {magic:02x?}, not \"LTAR\"");
        }
        let version = cursor.u32()?;
        if version != VERSION {
            bail!("{label}: LTAR version {version}, only {VERSION} is understood");
        }
        let dict_size = cursor.u32()? as usize;
        let dir_count = cursor.u32()? as usize;
        let file_count = cursor.u32()? as usize;

        let mut tables = vec![
            0u8;
            dict_size
                + file_count
                    .checked_mul(FILE_RECORD_SIZE)
                    .context("file table size overflow")?
                + dir_count
                    .checked_mul(DIR_RECORD_SIZE)
                    .context("directory table size overflow")?
        ];
        file.read_exact(&mut tables).with_context(|| {
            format!(
                "{label}: reading {dict_size} dictionary bytes, \
                 {file_count} file records and {dir_count} directory records"
            )
        })?;
        let (names, records) = tables.split_at(dict_size);

        let mut cursor = Cursor::new(records, &label);
        struct FileRecord {
            name: u32,
            offset: u64,
            stored_size: u64,
            size: u64,
            compression: u32,
        }
        let mut files = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            files.push(FileRecord {
                name: cursor.u32()?,
                offset: u64::from(cursor.u32()?) | u64::from(cursor.u32()?) << 32,
                stored_size: u64::from(cursor.u32()?) | u64::from(cursor.u32()?) << 32,
                size: u64::from(cursor.u32()?) | u64::from(cursor.u32()?) << 32,
                compression: cursor.u32()?,
            });
        }
        struct DirRecord {
            name: u32,
            first_child: i32,
            next_sibling: i32,
            file_count: u32,
        }
        let mut dirs = Vec::with_capacity(dir_count);
        for _ in 0..dir_count {
            dirs.push(DirRecord {
                name: cursor.u32()?,
                first_child: cursor.i32()?,
                next_sibling: cursor.i32()?,
                file_count: cursor.u32()?,
            });
        }

        // Resolve each directory's full path by walking the tree. Every
        // directory must be reached exactly once from the root, or the file
        // assignment below would attach files to a directory with no name.
        let mut dir_paths: Vec<Option<String>> = vec![None; dir_count];
        if dir_count > 0 {
            // (directory index, parent path) pairs still to visit.
            let mut pending: Vec<(usize, String)> = vec![(0, String::new())];
            while let Some((index, prefix)) = pending.pop() {
                let record = dirs
                    .get(index)
                    .with_context(|| format!("{label}: directory index {index} out of range"))?;
                if dir_paths[index].is_some() {
                    bail!("{label}: directory {index} is linked twice; the tree has a cycle");
                }
                let name = dictionary_string(names, record.name, &label)?;
                let full = match (prefix.is_empty(), name.is_empty()) {
                    (true, _) => name,
                    (false, true) => {
                        bail!("{label}: non-root directory {index} has an empty name")
                    }
                    (false, false) => format!("{prefix}\\{name}"),
                };
                if record.next_sibling >= 0 {
                    pending.push((record.next_sibling as usize, prefix));
                }
                if record.first_child >= 0 {
                    pending.push((record.first_child as usize, full.clone()));
                }
                dir_paths[index] = Some(full);
            }
        }

        let mut entries = Vec::with_capacity(file_count);
        let mut next_file = 0usize;
        for (index, record) in dirs.iter().enumerate() {
            let dir_path = dir_paths[index]
                .as_ref()
                .with_context(|| format!("{label}: directory {index} is unreachable"))?;
            for _ in 0..record.file_count {
                let file = files.get(next_file).with_context(|| {
                    format!(
                        "{label}: directories claim more than the {file_count} files present"
                    )
                })?;
                next_file += 1;
                let name = dictionary_string(names, file.name, &label)?;
                if name.is_empty() {
                    bail!("{label}: file {} has an empty name", next_file - 1);
                }
                let path = if dir_path.is_empty() {
                    name
                } else {
                    format!("{dir_path}\\{name}")
                };
                entries.push(ArchEntry {
                    path,
                    offset: file.offset,
                    stored_size: file.stored_size,
                    size: file.size,
                    compression: file.compression,
                });
            }
        }
        if next_file != file_count {
            bail!(
                "{label}: directories claim {next_file} files but the table has {file_count}"
            );
        }

        Ok(Self {
            path: path.to_path_buf(),
            file,
            entries,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[ArchEntry] {
        &self.entries
    }

    /// Read one entry's payload.
    pub fn read(&mut self, index: usize) -> Result<Vec<u8>> {
        let entry = self
            .entries
            .get(index)
            .with_context(|| format!("{}: no entry {index}", self.path.display()))?;
        if entry.compression != 0 {
            bail!(
                "{}: {} uses compression {}, which no retail archive does and this \
                 reader does not carry",
                self.path.display(),
                entry.path,
                entry.compression
            );
        }
        let mut payload = vec![
            0u8;
            usize::try_from(entry.size)
                .with_context(|| format!("{} is too large for this platform", entry.path))?
        ];
        self.file.seek(SeekFrom::Start(entry.offset)).with_context(|| {
            format!("{}: seeking to {}", self.path.display(), entry.path)
        })?;
        self.file.read_exact(&mut payload).with_context(|| {
            format!(
                "{}: {} declares {} bytes at offset {}, which run past the archive",
                self.path.display(),
                entry.path,
                entry.size,
                entry.offset
            )
        })?;
        Ok(payload)
    }
}

/// A NUL-terminated ASCII string at `offset` in the name dictionary.
fn dictionary_string(names: &[u8], offset: u32, label: &str) -> Result<String> {
    let start = offset as usize;
    let tail = names
        .get(start..)
        .with_context(|| format!("{label}: name offset {offset} is outside the dictionary"))?;
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .with_context(|| format!("{label}: unterminated name at dictionary offset {offset}"))?;
    let text = std::str::from_utf8(&tail[..end])
        .with_context(|| format!("{label}: non-ASCII name at dictionary offset {offset}"))?;
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a two-directory archive: `\` holding `root.txt`, `Sub` holding
    /// `nested.bin`, mirroring the layout of the 214-byte retail stub
    /// `FEAR_4.Arch00` that pinned the format down.
    fn synthetic() -> Vec<u8> {
        let mut names = Vec::new();
        let mut offsets = Vec::new();
        for name in ["", "Sub", "root.txt", "nested.bin"] {
            offsets.push(names.len() as u32);
            names.extend_from_slice(name.as_bytes());
            names.push(0);
            while names.len() % 4 != 0 {
                names.push(0);
            }
        }
        let payload_root = b"hello";
        let payload_nested = b"world!";
        let tables = names.len() + 2 * FILE_RECORD_SIZE + 2 * DIR_RECORD_SIZE;
        let data_start = (HEADER_SIZE + tables) as u64;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&(names.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&[0; 16]);
        bytes.extend_from_slice(&names);
        // root.txt in the root directory.
        bytes.extend_from_slice(&offsets[2].to_le_bytes());
        bytes.extend_from_slice(&data_start.to_le_bytes());
        bytes.extend_from_slice(&(payload_root.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(payload_root.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // nested.bin in Sub.
        bytes.extend_from_slice(&offsets[3].to_le_bytes());
        bytes.extend_from_slice(&(data_start + payload_root.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(payload_nested.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(payload_nested.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // Directory records: root (child 1, one file), Sub (leaf, one file).
        bytes.extend_from_slice(&offsets[0].to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&(-1i32).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&offsets[1].to_le_bytes());
        bytes.extend_from_slice(&(-1i32).to_le_bytes());
        bytes.extend_from_slice(&(-1i32).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(payload_root);
        bytes.extend_from_slice(payload_nested);
        bytes
    }

    fn write_archive(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(bytes).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn parses_paths_and_payloads() {
        let file = write_archive(&synthetic());
        let mut archive = Arch00::open(file.path()).unwrap();
        let paths: Vec<&str> = archive.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["root.txt", "Sub\\nested.bin"]);
        assert_eq!(archive.read(0).unwrap(), b"hello");
        assert_eq!(archive.read(1).unwrap(), b"world!");
    }

    #[test]
    fn refuses_compressed_entries() {
        let mut bytes = synthetic();
        // The compression word of the first file record.
        let names_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let record = HEADER_SIZE + names_len;
        bytes[record + 28..record + 32].copy_from_slice(&9u32.to_le_bytes());
        let file = write_archive(&bytes);
        let mut archive = Arch00::open(file.path()).unwrap();
        let error = archive.read(0).unwrap_err().to_string();
        assert!(error.contains("compression 9"), "{error}");
    }

    #[test]
    fn refuses_wrong_version() {
        let mut bytes = synthetic();
        bytes[4] = 2;
        let file = write_archive(&bytes);
        let error = Arch00::open(file.path()).unwrap_err().to_string();
        assert!(error.contains("version 2"), "{error}");
    }

    #[test]
    fn normalizes_case_and_separators() {
        assert_eq!(
            normalize("Worlds/Worlds\\ReleaseMultiplayer\\Office.World00p"),
            "worlds\\worlds\\releasemultiplayer\\office.world00p"
        );
    }
}
