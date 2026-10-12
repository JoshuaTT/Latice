//! The engine's only source of randomness (AGENTS.md §8).
//!
//! A ChaCha-based RNG seeded from the match seed and a stream ID. Map
//! generation, `me.rand()`, and engine tie-breaks each draw from their own
//! stream, so adding a draw in one place never shifts another.
//!
//! Key derivation: the 32-byte key is the little-endian `match_seed` repeated
//! four times, and the stream is applied via [`ChaCha20Rng::set_stream`].
//! Every [`StreamId`] maps to a distinct 128-bit stream value, so distinct
//! streams are independent.
//!
//! This module is `no_std`-friendly with respect to entropy: no `thread_rng`,
//! `rand::random`, `getrandom`, or wall clock is used anywhere. The restored
//! state is the word position (ChaCha block counter plus offset within the
//! block), which is sufficient to continue a stream exactly after a snapshot
//! restore (needed by spec issue #40 and #51).

use rand::RngCore;
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use serde::{Deserialize, Serialize};

/// Reserved stream ranges.
///
/// Each variant owns a disjoint 64-bit ChaCha stream value so no two
/// consumers can ever share a sequence:
/// - [`StreamId::Mapgen`]: stream 0.
/// - [`StreamId::TieBreak`]: stream 1.
/// - [`StreamId::Drone`]: streams `2 + drone_id`, one per drone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StreamId {
    /// Map generation draws.
    Mapgen,
    /// Engine tie-breaks.
    TieBreak,
    /// Per-drone stream (`me.rand()`), keyed by drone id.
    Drone(u64),
}

impl StreamId {
    /// The ChaCha stream value for this stream.
    pub fn value(self) -> u64 {
        match self {
            StreamId::Mapgen => 0,
            StreamId::TieBreak => 1,
            StreamId::Drone(id) => 2 + id,
        }
    }
}

/// Serializable RNG state: enough to restore the exact stream position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RngState {
    /// Word position: ChaCha block counter times 16 plus offset within block.
    pub word_pos: u128,
}

/// The match-seeded, per-stream RNG.
#[derive(Debug, Clone)]
pub struct MatchRng {
    inner: ChaCha20Rng,
}

impl MatchRng {
    /// Creates a stream seeded from `match_seed` and `stream`.
    pub fn new(match_seed: u64, stream: StreamId) -> Self {
        Self::seeded(match_seed, stream)
    }

    fn seeded(match_seed: u64, stream: StreamId) -> Self {
        let mut key = [0u8; 32];
        for chunk in key.as_chunks_mut::<8>().0 {
            *chunk = match_seed.to_le_bytes();
        }
        let mut rng = ChaCha20Rng::from_seed(key);
        rng.set_stream(stream.value());
        Self { inner: rng }
    }

    /// Next raw 64-bit value in the stream.
    pub fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    /// Next raw 32-bit value in the stream.
    pub fn next_u32(&mut self) -> u32 {
        self.inner.next_u32()
    }

    /// Uniform value in `0..n` by rejection sampling; never modulo-biased.
    /// Returns 0 for `n == 0`.
    pub fn below(&mut self, n: u64) -> u64 {
        if n <= 1 {
            return 0;
        }
        // Reject values >= threshold so the accepted range is a multiple of n.
        let threshold = n.wrapping_neg() % n;
        loop {
            let v = self.next_u64();
            if v >= threshold {
                return v % n;
            }
        }
    }

    /// Captures the exact stream position for snapshot restore.
    pub fn state(&self) -> RngState {
        RngState {
            word_pos: self.inner.get_word_pos(),
        }
    }

    /// Restores a stream to a captured position; continuation is identical.
    pub fn from_state(match_seed: u64, stream: StreamId, state: RngState) -> Self {
        let mut rng = Self::seeded(match_seed, stream);
        rng.inner.set_word_pos(state.word_pos);
        rng
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first8(seed: u64, stream: StreamId) -> [u64; 8] {
        let mut rng = MatchRng::new(seed, stream);
        let mut out = [0u64; 8];
        for v in &mut out {
            *v = rng.next_u64();
        }
        out
    }

    /// Pinned vectors: any platform or dependency change that alters the
    /// stream must fail here.
    #[test]
    fn test_vectors_seed1_mapgen() {
        assert_eq!(
            first8(1, StreamId::Mapgen),
            [
                14698744503296514894,
                4853555873038029046,
                16012919734872187166,
                17099782052758638768,
                315814880807099952,
                1785248162116088301,
                15656837725612215966,
                10421467670974425299,
            ]
        );
    }

    #[test]
    fn test_vectors_seed1_drone0() {
        assert_eq!(
            first8(1, StreamId::Drone(0)),
            [
                15997431775068984875,
                9072798441701073080,
                561029754738825544,
                9376028918377699071,
                1427220660405378078,
                10529072739542297248,
                17789127311412840021,
                11556521445937330538,
            ]
        );
    }

    #[test]
    fn different_streams_differ() {
        let a = first8(7, StreamId::Mapgen);
        let b = first8(7, StreamId::TieBreak);
        let c = first8(7, StreamId::Drone(0));
        let d = first8(7, StreamId::Drone(1));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(c, d);
    }

    #[test]
    fn save_restore_continues_identically() {
        let seed = 42;
        let mut rng = MatchRng::new(seed, StreamId::Drone(3));
        for _ in 0..37 {
            rng.next_u64();
        }
        let state = rng.state();
        let expected: Vec<u64> = (0..16).map(|_| rng.next_u64()).collect();

        let mut restored = MatchRng::from_state(seed, StreamId::Drone(3), state);
        assert_eq!(restored.state(), state);
        let actual: Vec<u64> = (0..16).map(|_| restored.next_u64()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn state_serializes_to_json() {
        let rng = MatchRng::new(5, StreamId::Mapgen);
        let json = serde_json::to_string(&rng.state()).unwrap();
        let back: RngState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rng.state());
    }

    #[test]
    fn below_zero_and_one() {
        let mut rng = MatchRng::new(9, StreamId::TieBreak);
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.below(1), 0);
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn below_always_in_range(seed in any::<u64>(), n in 1u64..1024, draws in 1usize..64) {
            let mut rng = MatchRng::new(seed, StreamId::TieBreak);
            for _ in 0..draws {
                prop_assert!(rng.below(n) < n);
            }
        }

        #[test]
        fn below_one_is_zero(seed in any::<u64>()) {
            let mut rng = MatchRng::new(seed, StreamId::Mapgen);
            prop_assert_eq!(rng.below(1), 0);
        }
    }
}
