//! 64-bit `SimHash` over shingled body text.
//!
//! `SimHash` turns a document into a fingerprint where near-duplicates differ in only a
//! few bits, so candidate pairs can be found by banding and Hamming distance rather than
//! pairwise comparison (`docs/crawler/parser.md` § Fingerprints). The hashing is
//! BLAKE3-derived and thus stable across runs and processes, which matters because the
//! fingerprint is persisted and compared long after it was written.

use lynx_common::hash::ContentHash;

/// Shingle width, in tokens.
const SHINGLE: usize = 3;

/// Compute a 64-bit `SimHash` over the body text.
#[must_use]
pub fn of_text(text: &str) -> u64 {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return 0;
    }

    let shingles = shingles(&tokens, SHINGLE);
    let mut counts = [0i32; 64];
    for shingle in &shingles {
        let bit = hash64(shingle);
        // `hash_bits` is a raw 64-bit fingerprint; counting its bits is the whole
        // algorithm, so the cast is lossless and intentional.
        #[allow(clippy::cast_possible_truncation)]
        for (i, slot) in counts.iter_mut().enumerate() {
            if bit & (1 << i) != 0 {
                *slot += 1;
            } else {
                *slot -= 1;
            }
        }
    }

    let mut out = 0u64;
    for (i, count) in counts.iter().enumerate() {
        if *count > 0 {
            out |= 1 << i;
        }
    }
    out
}

/// Split into lowercase alphanumeric tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Sliding windows of `width` tokens. Documents shorter than one window fall back to a
/// single shingle of everything they have.
fn shingles(tokens: &[String], width: usize) -> Vec<String> {
    if tokens.len() <= width {
        return vec![tokens.join(" ")];
    }
    tokens
        .windows(width)
        .map(|window| window.join(" "))
        .collect()
}

/// A stable 64-bit hash of a shingle, taken from the first eight BLAKE3 digest bytes.
fn hash64(input: &str) -> u64 {
    let hex = ContentHash::of(input.as_bytes());
    let hex = hex.as_str();
    // BLAKE3 hex is always 64 chars; the first 16 hex digits are the high 64 bits.
    u64::from_str_radix(&hex[..16], 16).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_zero() {
        assert_eq!(of_text(""), 0);
        assert_eq!(of_text("   \n\t "), 0);
    }

    #[test]
    fn identical_text_is_identical() {
        let a = of_text("the quick brown fox jumps over the lazy dog");
        let b = of_text("the quick brown fox jumps over the lazy dog");
        assert_eq!(a, b);
    }

    #[test]
    fn case_and_punctuation_do_not_change_the_fingerprint() {
        let a = of_text("Hello, World! This is a test of the fingerprint.");
        let b = of_text("hello world this is a test of the fingerprint");
        assert_eq!(a, b);
    }

    #[test]
    fn near_duplicates_differ_in_few_bits() {
        let base = "the quick brown fox jumps over the lazy dog and runs away into the woods";
        let edited = "the quick brown fox jumps over the lazy dog and runs away into the forest";
        let distance = (of_text(base) ^ of_text(edited)).count_ones();
        assert!(distance <= 24, "near-duplicates drifted {distance} bits");
    }

    #[test]
    fn unrelated_text_differs_widely() {
        let a = of_text("alpha beta gamma delta epsilon zeta eta theta");
        let b = of_text("quantum entanglement superconductivity lattice gauge boson");
        let distance = (a ^ b).count_ones();
        assert!(distance >= 8, "unrelated texts collided ({distance} bits)");
    }
}
