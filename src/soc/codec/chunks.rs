//! X-Ray chunked-container reader.
//!
//! Every X-Ray 1.x container file is a flat sequence of
//!
//! ```text
//! u32 chunk_id
//! u32 payload_size
//! payload bytes
//! ```
//!
//! records. Container chunks nest the same layout inside their payload, and
//! `fsL_VISUALS` abuses the identifier as an array index: its children are
//! numbered `0, 1, 2, ...` rather than named by role.
//!
//! The engine tolerates a compressed-payload mark on the identifier. Level
//! files never use it, so this reader rejects it instead of guessing which of
//! the several historic LZ variants produced the payload.

use anyhow::{Context, Result, bail};

/// `CFS_CompressMark` from `xrCore/FS.h`.
const COMPRESS_MARK: u32 = 1 << 31;

/// A borrowed view over one X-Ray chunked container.
#[derive(Clone, Copy, Debug)]
pub struct Chunks<'a> {
    bytes: &'a [u8],
    label: &'a str,
}

/// One decoded chunk header plus its payload.
#[derive(Clone, Copy, Debug)]
pub struct Chunk<'a> {
    pub id: u32,
    pub payload: &'a [u8],
}

impl<'a> Chunks<'a> {
    pub fn new(bytes: &'a [u8], label: &'a str) -> Self {
        Self { bytes, label }
    }

    /// Iterate every top-level chunk in file order.
    pub fn iter(&self) -> ChunkIter<'a> {
        ChunkIter {
            bytes: self.bytes,
            offset: 0,
            label: self.label,
        }
    }

    /// Return the payload of the first chunk with `id`.
    pub fn find(&self, id: u32) -> Result<Option<&'a [u8]>> {
        for chunk in self.iter() {
            let chunk = chunk?;
            if chunk.id == id {
                return Ok(Some(chunk.payload));
            }
        }
        Ok(None)
    }

    /// Return the payload of the first chunk with `id`, or fail.
    pub fn require(&self, id: u32) -> Result<&'a [u8]> {
        self.find(id)?
            .with_context(|| format!("{}: chunk {id:#x} is missing", self.label))
    }
}

pub struct ChunkIter<'a> {
    bytes: &'a [u8],
    offset: usize,
    label: &'a str,
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = Result<Chunk<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.bytes.len() {
            return None;
        }
        Some(self.read_next())
    }
}

impl<'a> ChunkIter<'a> {
    fn read_next(&mut self) -> Result<Chunk<'a>> {
        let header = self
            .bytes
            .get(self.offset..self.offset + 8)
            .with_context(|| {
                format!(
                    "{}: truncated chunk header at byte {}",
                    self.label, self.offset
                )
            })?;
        let id = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let size = u32::from_le_bytes(header[4..8].try_into().unwrap());
        if id & COMPRESS_MARK != 0 {
            bail!(
                "{}: chunk {:#x} at byte {} is compressed; unpack the container first",
                self.label,
                id & !COMPRESS_MARK,
                self.offset
            );
        }
        let start = self
            .offset
            .checked_add(8)
            .context("chunk payload offset overflow")?;
        let end = start
            .checked_add(usize::try_from(size)?)
            .context("chunk payload size overflow")?;
        let payload = self.bytes.get(start..end).with_context(|| {
            format!(
                "{}: chunk {id:#x} declares {size} payload bytes, but only {} remain",
                self.label,
                self.bytes.len().saturating_sub(start)
            )
        })?;
        self.offset = end;
        Ok(Chunk { id, payload })
    }
}

/// Sequential little-endian reader used by the level and OGF parsers.
#[derive(Clone, Copy, Debug)]
pub struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    label: &'a str,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8], label: &'a str) -> Self {
        Self {
            bytes,
            offset: 0,
            label,
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .context("cursor offset overflow")?;
        let slice = self.bytes.get(self.offset..end).with_context(|| {
            format!(
                "{}: wanted {count} bytes at byte {}, but only {} remain",
                self.label,
                self.offset,
                self.remaining()
            )
        })?;
        self.offset = end;
        Ok(slice)
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn float3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    /// Read a NUL-terminated string. X-Ray writes resource names in the
    /// engine's ASCII subset; anything else is a parse failure rather than a
    /// silent replacement character.
    pub fn stringz(&mut self) -> Result<String> {
        let tail = &self.bytes[self.offset..];
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .with_context(|| format!("{}: unterminated string at byte {}", self.label, self.offset))?;
        let text = std::str::from_utf8(&tail[..end])
            .with_context(|| format!("{}: non-ASCII string at byte {}", self.label, self.offset))?
            .to_owned();
        self.offset += end + 1;
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn finds_chunks_by_identifier() {
        let mut bytes = chunk(1, &[7, 7]);
        bytes.extend(chunk(3, &[9]));
        let chunks = Chunks::new(&bytes, "level");
        assert_eq!(chunks.require(1).unwrap(), &[7, 7]);
        assert_eq!(chunks.require(3).unwrap(), &[9]);
        assert!(chunks.find(2).unwrap().is_none());
    }

    #[test]
    fn iterates_indexed_children_in_order() {
        let mut bytes = chunk(0, b"a");
        bytes.extend(chunk(1, b"b"));
        bytes.extend(chunk(2, b"c"));
        let chunks = Chunks::new(&bytes, "visuals");
        let ids: Vec<u32> = chunks.iter().map(|chunk| chunk.unwrap().id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn rejects_a_compressed_payload_mark() {
        let bytes = chunk(1 | COMPRESS_MARK, &[0]);
        let chunks = Chunks::new(&bytes, "level");
        let error = chunks.require(1).unwrap_err().to_string();
        assert!(error.contains("compressed"), "{error}");
    }

    #[test]
    fn rejects_a_truncated_payload() {
        let mut bytes = chunk(1, &[1, 2, 3]);
        bytes.truncate(bytes.len() - 1);
        let chunks = Chunks::new(&bytes, "level");
        assert!(chunks.require(1).is_err());
    }

    #[test]
    fn reads_terminated_strings() {
        let mut cursor = Cursor::new(b"default\0levels\\escape\0", "shaders");
        assert_eq!(cursor.stringz().unwrap(), "default");
        assert_eq!(cursor.stringz().unwrap(), "levels\\escape");
        assert!(cursor.is_empty());
    }
}
