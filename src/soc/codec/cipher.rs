// SPDX-License-Identifier: GPL-3.0-or-later
//! The X-Ray archive obfuscation.
//!
//! Retail Shadow of Chernobyl archives keep their *file table* behind a
//! reversible byte transform. It is not encryption in any security sense — the
//! whole scheme is three integers compiled into the executable, so anyone with
//! the binary has the key. Its purpose was to stop casual repacking of shipped
//! game data, not to protect a secret.
//!
//! The transform has two halves, both driven by the same 32-bit linear
//! congruential generator:
//!
//! 1. a fixed 256-byte substitution alphabet, produced by shuffling the
//!    identity permutation with a generator seeded from the key;
//! 2. a keystream XOR, from a second generator seeded differently.
//!
//! Decoding is therefore `alphabet_back[cipher_byte ^ keystream_byte]`.
//!
//! Two key sets shipped. The worldwide release used one, the Russian release
//! another; the engine tries worldwide first and falls back. Ported from
//! `xray-16/src/xrCore/Crypto/trivial_encryptor.cpp`.

/// `CRandom32` from `xray-16/src/xrCore/Math/Random32.hpp`.
///
/// A plain LCG. `random(range)` takes the high 32 bits of the product rather
/// than a modulus, so the distribution does not depend on `range` dividing
/// 2^32.
#[derive(Clone, Copy, Debug)]
struct Random32 {
    seed: u32,
}

impl Random32 {
    fn new(seed: u32) -> Self {
        Self { seed }
    }

    fn random(&mut self, range: u32) -> u32 {
        self.seed = 0x0808_8405_u32.wrapping_mul(self.seed).wrapping_add(1);
        ((u64::from(self.seed) * u64::from(range)) >> 32) as u32
    }
}

/// Which shipped key to use.
///
/// The three constants of each key are dates written as `DDMMYYYY`, which is
/// the only reason they look arbitrary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Worldwide,
    Russian,
}

impl Key {
    /// `(table_iterations, table_seed, encrypt_seed)`.
    const fn constants(self) -> (u32, u32, u32) {
        match self {
            Self::Worldwide => (1024, 6_011_979, 24_031_979),
            Self::Russian => (2048, 20_091_958, 20_031_955),
        }
    }
}

const ALPHABET_SIZE: usize = 256;

/// A prepared decoder for one key.
pub struct Decoder {
    inverse_alphabet: [u8; ALPHABET_SIZE],
    encrypt_seed: u32,
}

impl Decoder {
    pub fn new(key: Key) -> Self {
        let (iterations, table_seed, encrypt_seed) = key.constants();

        let mut alphabet = [0_u8; ALPHABET_SIZE];
        for (index, slot) in alphabet.iter_mut().enumerate() {
            *slot = index as u8;
        }

        let mut generator = Random32::new(table_seed);
        for _ in 0..iterations {
            let left = generator.random(ALPHABET_SIZE as u32) as usize;
            let mut right = generator.random(ALPHABET_SIZE as u32) as usize;
            // The reference draws again rather than skipping, so the generator
            // advances a different number of times than the loop count.
            while left == right {
                right = generator.random(ALPHABET_SIZE as u32) as usize;
            }
            alphabet.swap(left, right);
        }

        let mut inverse_alphabet = [0_u8; ALPHABET_SIZE];
        for (index, value) in alphabet.iter().enumerate() {
            inverse_alphabet[usize::from(*value)] = index as u8;
        }

        Self {
            inverse_alphabet,
            encrypt_seed,
        }
    }

    /// Reverse the transform in place over a copy of `source`.
    pub fn decode(&self, source: &[u8]) -> Vec<u8> {
        let mut generator = Random32::new(self.encrypt_seed);
        source
            .iter()
            .map(|byte| {
                let keystream = (generator.random(ALPHABET_SIZE as u32) & 0xff) as u8;
                self.inverse_alphabet[usize::from(byte ^ keystream)]
            })
            .collect()
    }

    /// Apply the transform, used only to prove the inverse in tests.
    #[cfg(test)]
    fn encode(&self, source: &[u8]) -> Vec<u8> {
        let mut alphabet = [0_u8; ALPHABET_SIZE];
        for (index, value) in self.inverse_alphabet.iter().enumerate() {
            alphabet[usize::from(*value)] = index as u8;
        }
        let mut generator = Random32::new(self.encrypt_seed);
        source
            .iter()
            .map(|byte| {
                let keystream = (generator.random(ALPHABET_SIZE as u32) & 0xff) as u8;
                alphabet[usize::from(*byte)] ^ keystream
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_matches_the_reference_sequence() {
        // 0x08088405 * 1 + 1 = 0x08088406; (0x08088406 * 256) >> 32 = 8.
        let mut generator = Random32::new(1);
        assert_eq!(generator.random(256), 8);
    }

    #[test]
    fn the_alphabet_is_a_permutation() {
        for key in [Key::Worldwide, Key::Russian] {
            let decoder = Decoder::new(key);
            let mut seen = [false; ALPHABET_SIZE];
            for value in decoder.inverse_alphabet {
                assert!(!seen[usize::from(value)], "{key:?} alphabet repeats a byte");
                seen[usize::from(value)] = true;
            }
        }
    }

    #[test]
    fn decoding_inverts_encoding() {
        let payload: Vec<u8> = (0..1024).map(|index| (index % 256) as u8).collect();
        for key in [Key::Worldwide, Key::Russian] {
            let decoder = Decoder::new(key);
            assert_eq!(decoder.decode(&decoder.encode(&payload)), payload);
        }
    }

    #[test]
    fn the_two_shipped_keys_differ() {
        let worldwide = Decoder::new(Key::Worldwide);
        let russian = Decoder::new(Key::Russian);
        assert_ne!(worldwide.inverse_alphabet, russian.inverse_alphabet);
        assert_ne!(worldwide.encrypt_seed, russian.encrypt_seed);
        // A payload decoded with the wrong key must not come back intact.
        let payload = b"levels\\l01_escape\\level".to_vec();
        assert_ne!(russian.decode(&worldwide.encode(&payload)), payload);
    }
}
