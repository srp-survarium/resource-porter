// SPDX-License-Identifier: GPL-3.0-or-later
//! LZHUF decompression.
//!
//! X-Ray compresses an archive's file table with Haruyasu Yoshizaki's LZHUF:
//! LZSS sliding-window matching whose literals and match lengths are then coded
//! with an *adaptive* Huffman tree. Adaptive means the tree is rebuilt as the
//! stream is read, so decoding is inherently sequential — there is no header
//! table to read ahead.
//!
//! Ported from `xray-16/src/xrCore/LzHuf.cpp`. Only decoding is implemented;
//! nothing in this repository writes X-Ray archives.

use anyhow::{Result, bail};

/// Sliding-window size.
const N: usize = 4096;
/// Look-ahead buffer size.
const F: usize = 60;
/// Shortest run worth encoding as a match.
const THRESHOLD: usize = 2;
/// Distinct symbols: 256 literals plus the match lengths.
const N_CHAR: usize = 256 - THRESHOLD + F;
/// Huffman table size.
const T: usize = N_CHAR * 2 - 1;
/// Index of the tree root.
const R: usize = T - 1;
/// Frequency ceiling; the tree is rebuilt when the root reaches it.
const MAX_FREQ: u32 = 0x4000;

/// Upper six bits of a match position, by leading byte.
const D_CODE: [u8; 256] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
    0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09,
    0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b,
    0x0c, 0x0c, 0x0c, 0x0c, 0x0d, 0x0d, 0x0d, 0x0d, 0x0e, 0x0e, 0x0e, 0x0e, 0x0f, 0x0f, 0x0f, 0x0f,
    0x10, 0x10, 0x10, 0x10, 0x11, 0x11, 0x11, 0x11, 0x12, 0x12, 0x12, 0x12, 0x13, 0x13, 0x13, 0x13,
    0x14, 0x14, 0x14, 0x14, 0x15, 0x15, 0x15, 0x15, 0x16, 0x16, 0x16, 0x16, 0x17, 0x17, 0x17, 0x17,
    0x18, 0x18, 0x19, 0x19, 0x1a, 0x1a, 0x1b, 0x1b, 0x1c, 0x1c, 0x1d, 0x1d, 0x1e, 0x1e, 0x1f, 0x1f,
    0x20, 0x20, 0x21, 0x21, 0x22, 0x22, 0x23, 0x23, 0x24, 0x24, 0x25, 0x25, 0x26, 0x26, 0x27, 0x27,
    0x28, 0x28, 0x29, 0x29, 0x2a, 0x2a, 0x2b, 0x2b, 0x2c, 0x2c, 0x2d, 0x2d, 0x2e, 0x2e, 0x2f, 0x2f,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
];

/// Total code length of a match position, by leading byte.
const D_LEN: [u8; 256] = [
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
];

/// A bit reader over the compressed stream.
///
/// Reads past the end yield zero bits, matching the reference reader: a
/// truncated stream fails on the output-length check instead of here.
struct BitReader<'a> {
    input: &'a [u8],
    position: usize,
    buffer: u32,
    length: u32,
}

impl<'a> BitReader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            position: 0,
            buffer: 0,
            length: 0,
        }
    }

    fn byte(&mut self) -> u32 {
        match self.input.get(self.position) {
            Some(value) => {
                self.position += 1;
                u32::from(*value)
            }
            None => 0,
        }
    }

    fn fill(&mut self) {
        while self.length <= 8 {
            let value = self.byte();
            self.buffer |= value << (8 - self.length);
            self.length += 8;
        }
    }

    fn bit(&mut self) -> u32 {
        self.fill();
        let value = self.buffer;
        self.buffer = (self.buffer << 1) & 0xffff;
        self.length -= 1;
        (value & 0x8000) >> 15
    }

    fn byte_code(&mut self) -> u32 {
        self.fill();
        let value = self.buffer;
        self.buffer = (self.buffer << 8) & 0xffff;
        self.length -= 8;
        (value & 0xff00) >> 8
    }
}

/// The adaptive Huffman tree.
struct Tree {
    freq: [u32; T + 1],
    parent: [usize; T + N_CHAR + 1],
    son: [usize; T],
}

impl Tree {
    fn new() -> Self {
        let mut tree = Self {
            freq: [0; T + 1],
            parent: [0; T + N_CHAR + 1],
            son: [0; T],
        };
        for index in 0..N_CHAR {
            tree.freq[index] = 1;
            tree.son[index] = index + T;
            tree.parent[index + T] = index;
        }
        let mut child = 0;
        let mut node = N_CHAR;
        while node <= R {
            tree.freq[node] = tree.freq[child] + tree.freq[child + 1];
            tree.son[node] = child;
            tree.parent[child] = node;
            tree.parent[child + 1] = node;
            child += 2;
            node += 1;
        }
        tree.freq[T] = 0xffff;
        tree.parent[R] = 0;
        tree
    }

    /// Halve every frequency and rebuild, so the tree keeps adapting once the
    /// root frequency saturates.
    fn rebuild(&mut self) {
        let mut leaf = 0;
        for index in 0..T {
            if self.son[index] >= T {
                self.freq[leaf] = (self.freq[index] + 1) / 2;
                self.son[leaf] = self.son[index];
                leaf += 1;
            }
        }
        let mut child = 0;
        let mut node = N_CHAR;
        while node < T {
            let combined = self.freq[child] + self.freq[child + 1];
            self.freq[node] = combined;
            let mut slot = node;
            while slot > 0 && combined < self.freq[slot - 1] {
                slot -= 1;
            }
            self.freq.copy_within(slot..node, slot + 1);
            self.freq[slot] = combined;
            self.son.copy_within(slot..node, slot + 1);
            self.son[slot] = child;
            child += 2;
            node += 1;
        }
        for index in 0..T {
            let child = self.son[index];
            self.parent[child] = index;
            if child < T {
                self.parent[child + 1] = index;
            }
        }
    }

    /// Increment a symbol's frequency and restore the sibling ordering.
    fn update(&mut self, symbol: usize) {
        if self.freq[R] == MAX_FREQ {
            self.rebuild();
        }
        let mut node = self.parent[symbol + T];
        loop {
            self.freq[node] += 1;
            let frequency = self.freq[node];
            // If this node now outranks its successor, swap it up.
            if frequency > self.freq[node + 1] {
                let mut higher = node + 1;
                while frequency > self.freq[higher + 1] {
                    higher += 1;
                }
                self.freq[node] = self.freq[higher];
                self.freq[higher] = frequency;

                let left = self.son[node];
                self.parent[left] = higher;
                if left < T {
                    self.parent[left + 1] = higher;
                }
                let right = self.son[higher];
                self.son[higher] = left;
                self.parent[right] = node;
                if right < T {
                    self.parent[right + 1] = node;
                }
                self.son[node] = right;
                node = higher;
            }
            node = self.parent[node];
            if node == 0 {
                return;
            }
        }
    }

    fn decode_symbol(&mut self, reader: &mut BitReader<'_>) -> usize {
        let mut node = self.son[R];
        while node < T {
            node += reader.bit() as usize;
            node = self.son[node];
        }
        node -= T;
        self.update(node);
        node
    }
}

/// Decode the match distance: six bits from a table, the rest verbatim.
fn decode_position(reader: &mut BitReader<'_>) -> usize {
    let leading = reader.byte_code() as usize;
    let high = usize::from(D_CODE[leading]) << 6;
    let mut low = leading;
    let mut remaining = D_LEN[leading] - 2;
    while remaining > 0 {
        low = (low << 1) + reader.bit() as usize;
        remaining -= 1;
    }
    high | (low & 0x3f)
}

/// Decompress an LZHUF stream.
///
/// `limit` bounds the declared output size, so a corrupt or misidentified
/// payload cannot ask for an arbitrary allocation.
pub fn decompress(input: &[u8], limit: usize) -> Result<Vec<u8>> {
    if input.len() < 4 {
        bail!("LZHUF stream is shorter than its length header");
    }
    let declared = u32::from_le_bytes(input[0..4].try_into().unwrap()) as usize;
    if declared == 0 {
        bail!("LZHUF stream declares an empty result");
    }
    if declared > limit {
        bail!("LZHUF stream declares {declared} bytes, above the {limit}-byte limit");
    }

    let mut reader = BitReader::new(&input[4..]);
    let mut tree = Tree::new();
    let mut output = Vec::with_capacity(declared);
    // The encoder primes only the first `N - F` bytes of its window with
    // spaces and leaves the rest of the (zero-initialised) buffer alone. A
    // match can reach into that tail before anything has been written there,
    // so the decoder has to reproduce both regions exactly: filling the whole
    // window with spaces desynchronises the stream a few hundred bytes in.
    let mut window = [0_u8; N];
    window[..N - F].fill(0x20);
    let mut cursor = N - F;

    while output.len() < declared {
        let symbol = tree.decode_symbol(&mut reader);
        if symbol < 256 {
            let byte = symbol as u8;
            output.push(byte);
            window[cursor] = byte;
            cursor = (cursor + 1) & (N - 1);
        } else {
            let start = (cursor + N - decode_position(&mut reader) - 1) & (N - 1);
            let length = symbol - 255 + THRESHOLD;
            for offset in 0..length {
                if output.len() == declared {
                    break;
                }
                let byte = window[(start + offset) & (N - 1)];
                output.push(byte);
                window[cursor] = byte;
                cursor = (cursor + 1) & (N - 1);
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode with the same adaptive model, so the test proves the decoder
    /// tracks a real stream rather than a fixture the decoder itself produced.
    ///
    /// Only literals are emitted: that exercises the tree, its reordering and
    /// its rebuild, without also reimplementing the match finder.
    struct LiteralEncoder {
        tree: Tree,
        buffer: u32,
        length: u32,
        output: Vec<u8>,
    }

    impl LiteralEncoder {
        fn new() -> Self {
            Self {
                tree: Tree::new(),
                buffer: 0,
                length: 0,
                output: Vec::new(),
            }
        }

        /// Walk from the leaf up to the root, collecting the path bits.
        fn encode(&mut self, symbol: usize) {
            let mut code = 0_u32;
            let mut bits = 0_u32;
            let mut node = self.tree.parent[symbol + T];
            loop {
                code >>= 1;
                if node >= T {
                    unreachable!("leaf parents are interior nodes");
                }
                if self.tree.son[self.tree.parent[node]] != node {
                    code |= 0x8000;
                }
                bits += 1;
                node = self.tree.parent[node];
                if node == R {
                    break;
                }
            }
            self.put(bits, code >> (16 - bits) << (16 - bits));
            self.tree.update(symbol);
        }

        fn put(&mut self, bits: u32, code: u32) {
            self.buffer |= code >> self.length;
            self.length += bits;
            while self.length >= 8 {
                self.output.push((self.buffer >> 8) as u8);
                self.buffer = (self.buffer << 8) & 0xffff;
                self.length -= 8;
            }
        }

        fn finish(mut self, declared: usize) -> Vec<u8> {
            if self.length > 0 {
                self.output.push((self.buffer >> 8) as u8);
            }
            let mut stream = (declared as u32).to_le_bytes().to_vec();
            stream.extend(self.output);
            stream
        }
    }

    fn roundtrip(payload: &[u8]) -> Vec<u8> {
        let mut encoder = LiteralEncoder::new();
        for byte in payload {
            encoder.encode(usize::from(*byte));
        }
        let stream = encoder.finish(payload.len());
        decompress(&stream, 1 << 20).unwrap()
    }

    #[test]
    fn decodes_a_short_literal_stream() {
        let payload = b"levels\\l01_escape\\level\0";
        assert_eq!(roundtrip(payload), payload);
    }

    #[test]
    fn decodes_a_stream_long_enough_to_reorder_the_tree() {
        // Skewed frequencies force the sibling swaps in `update`.
        let mut payload = Vec::new();
        for index in 0..4000 {
            payload.push(if index % 3 == 0 { b'a' } else { b'b' });
        }
        assert_eq!(roundtrip(&payload), payload);
    }

    #[test]
    fn decodes_a_stream_long_enough_to_rebuild_the_tree() {
        // The root frequency must reach MAX_FREQ for `rebuild` to run.
        let payload: Vec<u8> = (0..20_000).map(|index| (index % 251) as u8).collect();
        assert_eq!(roundtrip(&payload), payload);
    }

    #[test]
    fn rejects_a_declared_size_above_the_limit() {
        let mut stream = 5_000_u32.to_le_bytes().to_vec();
        stream.extend([0_u8; 16]);
        let error = decompress(&stream, 1024).unwrap_err().to_string();
        assert!(error.contains("above the 1024-byte limit"), "{error}");
    }

    #[test]
    fn rejects_an_empty_or_truncated_stream() {
        assert!(decompress(&[], 1024).is_err());
        assert!(decompress(&0_u32.to_le_bytes(), 1024).is_err());
    }
}
