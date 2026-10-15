// SPDX-License-Identifier: GPL-3.0-or-later
//! Headroom overlays: an overlay that leaves the base archive in place.
//!
//! [`crate::overlay`] moves the whole payload when the file table grows, so the
//! output differs from the base across the entire file. A headroom overlay
//! instead keeps every base payload at its base offset, except the first few:
//! those are moved after the base end to make room for the larger file table.
//! Changed and new payloads are appended after the base end too. The output is
//! then the base file with
//!
//! * one same-size **head** region `[0, H)`: the header, the new file table,
//!   the file-table signature slot and zero padding, where `H` is the end of
//!   the last moved payload; and
//! * an appended **tail** `[base_len, new_len)`: the changed and new payloads
//!   in overlay order, then the moved payloads in base order.
//!
//! A client that already has the base needs only these two blobs. Nothing is
//! written as a whole archive: [`OverlaidFile`] reads base + head + tail as
//! one virtual file for verification.
//!
//! The file table is followed by a [`SIGNATURE_LEN`]-byte signature in the
//! shipped archive (`[24 + buffer_size, 24 + buffer_size + 256)`). The head
//! keeps that slot, zeroed; signing it is the caller's last step.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::fat::{Archive, FileEntry, NodeFlags, RawNode};
use crate::overlay::{file_entries, invalid_data, plan_overlay, OverlayPlan, OverlayStats};
use crate::pack;

/// Bytes of the file-table signature that follows the file table.
pub const SIGNATURE_LEN: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedPayload {
    /// One of the paths whose payload this is (several may share it).
    pub path: String,
    pub size: u32,
    pub from: u64,
    pub to: u64,
}

#[derive(Debug, Clone)]
pub struct HeadroomPatch {
    pub base_len: u64,
    pub new_len: u64,
    /// Length of the head region `[0, head_len)`.
    pub head_len: u64,
    /// End of the 24-byte header and the file table: the signature slot is
    /// `[fat_end, fat_end + SIGNATURE_LEN)`.
    pub fat_end: u64,
    pub tail_len: u64,
    pub moved: Vec<MovedPayload>,
    pub stats: OverlayStats,
}

/// Payload groups stored in the base region: `pos -> (size, a path)`.
fn stored_payloads(
    node: &RawNode,
    prefix: &[u8],
    limit: u64,
    out: &mut BTreeMap<u64, (u32, Vec<u8>)>,
) -> io::Result<()> {
    let path = if prefix.is_empty() {
        node.name.clone()
    } else {
        [prefix, b"/", &node.name].concat()
    };
    if node.is_folder() {
        let path = if node.name.is_empty() {
            prefix.to_vec()
        } else {
            path
        };
        for child in &node.children {
            stored_payloads(child, &path, limit, out)?;
        }
        return Ok(());
    }
    if node.hard_link_target_off.is_some()
        || node.is_inlined()
        || !node.flags.contains(NodeFlags::ARCHIVE)
        || node.size_in_db == 0
        || node.pos_in_db >= limit
    {
        return Ok(());
    }
    match out.get(&node.pos_in_db) {
        Some((size, _)) if *size != node.size_in_db => Err(invalid_data(format!(
            "payload at {} is shared with different sizes",
            node.pos_in_db
        ))),
        Some(_) => Ok(()),
        None => {
            out.insert(node.pos_in_db, (node.size_in_db, path));
            Ok(())
        }
    }
}

fn relocate(node: &mut RawNode, moves: &HashMap<u64, u64>) {
    if !node.is_folder()
        && node.hard_link_target_off.is_none()
        && !node.is_inlined()
        && node.size_in_db != 0
    {
        if let Some(&to) = moves.get(&node.pos_in_db) {
            node.pos_in_db = to;
        }
    }
    for child in &mut node.children {
        relocate(child, moves);
    }
}

/// Plan the overlay of `donor_path` onto `base_path` and write the head and
/// tail blobs. The signature slot of the head is left zeroed.
pub fn build_headroom_overlay(
    base_path: &Path,
    donor_path: &Path,
    materialized_paths: &[Vec<u8>],
    head_out: &Path,
    tail_out: &Path,
) -> io::Result<HeadroomPatch> {
    let OverlayPlan {
        changes,
        mut root,
        output_nodes,
        base_len,
        payload_end,
        stats,
        ..
    } = plan_overlay(base_path, donor_path, materialized_paths)?;

    let fat_len = pack::build_fat_with_header(&root, output_nodes).len() as u64;
    let needed = fat_len + SIGNATURE_LEN as u64;
    let mut stored = BTreeMap::new();
    stored_payloads(&root, b"", base_len, &mut stored)?;
    let mut previous_end = 0;
    for (&pos, &(size, _)) in &stored {
        if pos < previous_end {
            return Err(invalid_data(format!("base payloads overlap at {pos}")));
        }
        previous_end = pos + u64::from(size);
    }

    // Move every payload that starts inside the space the table needs. They
    // are contiguous, so the head ends where the last moved one ended.
    let mut moved = Vec::new();
    let mut moves = HashMap::new();
    let mut head_len = needed;
    let mut next = payload_end;
    for (&pos, (size, path)) in stored.range(..needed) {
        moves.insert(pos, next);
        moved.push(MovedPayload {
            path: String::from_utf8_lossy(path).into_owned(),
            size: *size,
            from: pos,
            to: next,
        });
        next += u64::from(*size);
        head_len = head_len.max(pos + u64::from(*size));
    }
    if let Some((&pos, _)) = stored.range(head_len..).next() {
        debug_assert!(pos >= head_len);
    }
    if head_len > base_len {
        return Err(invalid_data(
            "the file table outgrows the whole base archive",
        ));
    }
    relocate(&mut root, &moves);
    let fat = pack::build_fat_with_header(&root, output_nodes);
    if fat.len() as u64 != fat_len {
        return Err(invalid_data(
            "file table size changed while relocating payloads",
        ));
    }
    // Nothing else may still point into the head.
    let mut remaining = BTreeMap::new();
    stored_payloads(&root, b"", head_len, &mut remaining)?;
    if let Some((pos, (_, path))) = remaining.iter().next() {
        return Err(invalid_data(format!(
            "payload of {:?} at {pos} is still inside the head",
            String::from_utf8_lossy(path)
        )));
    }

    let mut head = vec![0_u8; head_len as usize];
    head[..fat.len()].copy_from_slice(&fat);
    File::create(head_out)?.write_all(&head)?;

    let mut tail = io::BufWriter::new(File::create(tail_out)?);
    for bytes in changes.values() {
        tail.write_all(bytes)?;
    }
    let mut base = File::open(base_path)?;
    for payload in &moved {
        base.seek(SeekFrom::Start(payload.from))?;
        let mut bytes = vec![0_u8; payload.size as usize];
        base.read_exact(&mut bytes)?;
        tail.write_all(&bytes)?;
    }
    tail.flush()?;
    drop(tail);
    let tail_len = std::fs::metadata(tail_out)?.len();
    if base_len + tail_len != next {
        return Err(invalid_data(
            "tail length does not match the planned payloads",
        ));
    }
    Ok(HeadroomPatch {
        base_len,
        new_len: next,
        head_len,
        fat_end: fat_len,
        tail_len,
        moved,
        stats,
    })
}

/// The base file with `head` over `[0, head.len())` and `tail` appended: the
/// archive a client gets after installing both blobs.
pub struct OverlaidFile {
    base: File,
    head: Vec<u8>,
    tail: File,
    base_len: u64,
    len: u64,
    pos: u64,
}

impl OverlaidFile {
    pub fn open(base: &Path, head: &Path, tail: &Path) -> io::Result<Self> {
        let base_len = std::fs::metadata(base)?.len();
        let head = std::fs::read(head)?;
        if head.len() as u64 > base_len {
            return Err(invalid_data("head region is longer than the base file"));
        }
        let tail_len = std::fs::metadata(tail)?.len();
        Ok(Self {
            base: File::open(base)?,
            head,
            tail: File::open(tail)?,
            base_len,
            len: base_len + tail_len,
            pos: 0,
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Read for OverlaidFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let head_len = self.head.len() as u64;
        let n = if self.pos < head_len {
            let n = buf.len().min((head_len - self.pos) as usize);
            let at = self.pos as usize;
            buf[..n].copy_from_slice(&self.head[at..at + n]);
            n
        } else if self.pos < self.base_len {
            let n = buf.len().min((self.base_len - self.pos) as usize);
            self.base.seek(SeekFrom::Start(self.pos))?;
            self.base.read(&mut buf[..n])?
        } else {
            let n = buf.len().min((self.len - self.pos) as usize);
            self.tail.seek(SeekFrom::Start(self.pos - self.base_len))?;
            self.tail.read(&mut buf[..n])?
        };
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for OverlaidFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
        };
        self.pos = target.ok_or_else(|| invalid_data("seek before the start of the file"))?;
        Ok(self.pos)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HeadroomCheck {
    /// Base entries whose metadata and offset are unchanged.
    pub untouched: usize,
    /// Base entries whose payload moved to the tail; bytes compared.
    pub moved: usize,
    /// Untouched entries whose bytes were compared (all of them when deep).
    pub compared: usize,
}

/// Check the overlaid archive against the base: every base path that is not
/// in `changed` keeps its size and bytes, and either keeps its base offset
/// (outside the head) or was moved into the tail. With `deep`, the bytes of
/// every untouched entry are compared too (reads the whole base twice).
pub fn verify_untouched(
    base_path: &Path,
    head: &Path,
    tail: &Path,
    changed: &dyn Fn(&[u8]) -> bool,
    deep: bool,
) -> io::Result<HeadroomCheck> {
    let overlaid = OverlaidFile::open(base_path, head, tail)?;
    let head_len = std::fs::metadata(head)?.len();
    untouched_in(
        base_path,
        Archive::from_reader(Box::new(overlaid))?,
        head_len,
        changed,
        deep,
    )
}

/// [`verify_untouched`] on a written archive: the base with a head of
/// `head_len` bytes written over its start and a tail appended.
pub fn verify_untouched_file(
    base_path: &Path,
    output_path: &Path,
    head_len: u64,
    changed: &dyn Fn(&[u8]) -> bool,
    deep: bool,
) -> io::Result<HeadroomCheck> {
    untouched_in(
        base_path,
        Archive::open(output_path)?,
        head_len,
        changed,
        deep,
    )
}

fn untouched_in(
    base_path: &Path,
    mut output: Archive,
    head_len: u64,
    changed: &dyn Fn(&[u8]) -> bool,
    deep: bool,
) -> io::Result<HeadroomCheck> {
    let mut base = Archive::open(base_path)?;
    let base_len = std::fs::metadata(base_path)?.len();
    let output_entries = file_entries(&output);
    let mut check = HeadroomCheck::default();
    for (path, entry) in file_entries(&base) {
        if changed(&path) {
            continue;
        }
        let name = || String::from_utf8_lossy(&path).into_owned();
        let found: &FileEntry = output_entries
            .get(&path)
            .ok_or_else(|| invalid_data(format!("untouched path disappeared: {}", name())))?;
        if found.size_in_db != entry.size_in_db
            || found.uncompressed_size != entry.uncompressed_size
            || found.kind != entry.kind
            || found.inline_buffer_offset.is_some() != entry.inline_buffer_offset.is_some()
        {
            return Err(invalid_data(format!("untouched entry changed: {}", name())));
        }
        let stored = entry.inline_buffer_offset.is_none() && entry.size_in_db != 0;
        let moved = stored && found.pos_in_db != entry.pos_in_db;
        if moved {
            if found.pos_in_db < base_len {
                return Err(invalid_data(format!("{} moved inside the base", name())));
            }
            check.moved += 1;
        } else if stored && entry.pos_in_db < head_len {
            return Err(invalid_data(format!(
                "{} still points into the head",
                name()
            )));
        } else {
            check.untouched += 1;
        }
        if moved || deep || !stored {
            if base.read_raw(&entry)? != output.read_raw(found)? {
                return Err(invalid_data(format!("{} reads different bytes", name())));
            }
            if !moved {
                check.compared += 1;
            }
        }
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::apply_db_overlay;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "resources-db-headroom-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive(dir: &Path, name: &str, files: &[(&str, Vec<u8>)]) -> std::path::PathBuf {
        let source = dir.join(format!("{name}-source"));
        for (path, bytes) in files {
            let p = source.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
        std::fs::create_dir_all(&source).unwrap();
        let db = dir.join(format!("{name}.db"));
        pack::assemble_to_file(&pack::build_src_tree(&source).unwrap(), &db).unwrap();
        db
    }

    /// A packed archive reserves the engine's over-estimated table size, so
    /// this base is trimmed to the shipped layout: the payload starts right
    /// after the table and the signature slot.
    fn tight_base(dir: &Path, files: &[(&str, Vec<u8>)]) -> std::path::PathBuf {
        let loose = archive(dir, "loose", files);
        let tree = Archive::open(&loose).unwrap().parse_tree();
        let fat = pack::build_fat_with_header(&tree.root, tree.num_nodes);
        let mut root = tree.root;
        let origin = crate::overlay::first_stored_payload_position(&root).unwrap();
        let tight = (fat.len() + SIGNATURE_LEN) as u64;
        fn shift(node: &mut RawNode, by: u64) {
            if !node.is_folder() && node.hard_link_target_off.is_none() && node.size_in_db != 0 {
                node.pos_in_db -= by;
            }
            node.children.iter_mut().for_each(|c| shift(c, by));
        }
        shift(&mut root, origin - tight);
        let mut out = pack::build_fat_with_header(&root, tree.num_nodes);
        out.extend(std::iter::repeat_n(0xA5, SIGNATURE_LEN));
        let bytes = std::fs::read(&loose).unwrap();
        out.extend_from_slice(&bytes[origin as usize..]);
        let db = dir.join("base.db");
        std::fs::write(&db, out).unwrap();
        db
    }

    fn read(file: &mut Archive, path: &str) -> Vec<u8> {
        let entry = file
            .list()
            .into_iter()
            .find(|e| e.path == path.as_bytes())
            .unwrap();
        file.read_file(&entry).unwrap()
    }

    fn base_files() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("a/first.cfg", b"first payload".repeat(4)),
            ("a/second.cfg", b"second".repeat(40)),
            ("b/big.bin", (0..5000_u32).map(|i| i as u8).collect()),
            ("b/copy.bin", (0..5000_u32).map(|i| i as u8).collect()),
            ("c/last.bin", b"last".repeat(300)),
        ]
    }

    #[test]
    fn keeps_base_bytes_outside_head_and_tail() {
        let dir = scratch("layout");
        let base = tight_base(&dir, &base_files());
        let donor = archive(
            &dir,
            "donor",
            &[
                ("a/second.cfg", b"replaced".to_vec()),
                ("new/folder/one.bin", b"new one".to_vec()),
                ("new/folder/two.bin", b"new two".repeat(9)),
            ],
        );
        let (head, tail) = (dir.join("head.bin"), dir.join("tail.bin"));
        let patch = build_headroom_overlay(&base, &donor, &[], &head, &tail).unwrap();
        assert!(
            !patch.moved.is_empty(),
            "a larger table must move a payload"
        );
        assert_eq!(std::fs::metadata(&head).unwrap().len(), patch.head_len);
        assert!(patch.fat_end + SIGNATURE_LEN as u64 <= patch.head_len);

        // Byte layout: base everywhere outside the head, then the tail.
        let base_bytes = std::fs::read(&base).unwrap();
        let mut overlaid = OverlaidFile::open(&base, &head, &tail).unwrap();
        let mut whole = Vec::new();
        overlaid.read_to_end(&mut whole).unwrap();
        assert_eq!(whole.len() as u64, patch.new_len);
        assert_eq!(
            &whole[patch.head_len as usize..base_bytes.len()],
            &base_bytes[patch.head_len as usize..]
        );
        assert!(whole[patch.fat_end as usize..patch.head_len as usize]
            .iter()
            .all(|b| *b == 0));

        let mut output = Archive::from_reader(Box::new(overlaid)).unwrap();
        assert_eq!(read(&mut output, "a/second.cfg"), b"replaced");
        assert_eq!(
            read(&mut output, "new/folder/two.bin"),
            b"new two".repeat(9)
        );
        assert_eq!(read(&mut output, "a/first.cfg"), b"first payload".repeat(4));
        assert_eq!(read(&mut output, "c/last.bin"), b"last".repeat(300));
        assert_eq!(
            read(&mut output, "b/copy.bin"),
            (0..5000_u32).map(|i| i as u8).collect::<Vec<_>>()
        );

        let changed = |p: &[u8]| p == b"a/second.cfg";
        let check = verify_untouched(&base, &head, &tail, &changed, true).unwrap();
        assert!(check.moved >= patch.moved.len());
        assert!(check.untouched > 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn moves_only_what_the_table_needs() {
        let dir = scratch("minimal");
        let base = tight_base(&dir, &base_files());
        // An identical donor changes nothing: the table keeps its size.
        let same = archive(&dir, "same", &[("a/first.cfg", b"first payload".repeat(4))]);
        let (head, tail) = (dir.join("head.bin"), dir.join("tail.bin"));
        let patch = build_headroom_overlay(&base, &same, &[], &head, &tail).unwrap();
        assert!(patch.moved.is_empty());
        assert_eq!(patch.tail_len, 0);
        let base_bytes = std::fs::read(&base).unwrap();
        let head_bytes = std::fs::read(&head).unwrap();
        assert_eq!(
            head_bytes[..patch.fat_end as usize],
            base_bytes[..patch.fat_end as usize]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn matches_the_relocating_overlay_resource_for_resource() {
        let dir = scratch("same-content");
        let base = tight_base(&dir, &base_files());
        let donor = archive(
            &dir,
            "donor",
            &[("b/big.bin", b"other".to_vec()), ("z.bin", b"zed".to_vec())],
        );
        let full = dir.join("full.db");
        apply_db_overlay(&base, &donor, &full).unwrap();
        let (head, tail) = (dir.join("head.bin"), dir.join("tail.bin"));
        build_headroom_overlay(&base, &donor, &[], &head, &tail).unwrap();
        let mut a = Archive::open(&full).unwrap();
        let mut b =
            Archive::from_reader(Box::new(OverlaidFile::open(&base, &head, &tail).unwrap()))
                .unwrap();
        let (la, lb) = (a.list(), b.list());
        assert_eq!(
            la.iter().map(|e| &e.path).collect::<Vec<_>>(),
            lb.iter().map(|e| &e.path).collect::<Vec<_>>()
        );
        for (ea, eb) in la.iter().zip(&lb) {
            if matches!(ea.kind, crate::fat::NodeKind::File { .. }) {
                assert_eq!(a.read_raw(ea).unwrap(), b.read_raw(eb).unwrap());
            }
        }
        // The relocating overlay rewrote every payload offset; this one kept
        // the ones outside the head.
        assert_eq!(
            read(&mut b, "b/copy.bin"),
            (0..5000_u32).map(|i| i as u8).collect::<Vec<_>>()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_a_head_that_misreads() {
        let dir = scratch("misread");
        let base = tight_base(&dir, &base_files());
        let donor = archive(&dir, "donor", &[("new.bin", b"x".repeat(100))]);
        let (head, tail) = (dir.join("head.bin"), dir.join("tail.bin"));
        build_headroom_overlay(&base, &donor, &[], &head, &tail).unwrap();
        // Corrupt one moved payload in the tail.
        let mut bytes = std::fs::read(&tail).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&tail, bytes).unwrap();
        let changed = |p: &[u8]| p == b"new.bin";
        assert!(verify_untouched(&base, &head, &tail, &changed, false).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
