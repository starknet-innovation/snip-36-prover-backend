//! Pure coin-flip outcome computation.
//!
//! The outcome is the least-significant bit of `pedersen(seed, player)` — the
//! same computation the on-chain `play` function performs and that the SNIP-36
//! settlement message commits to. Keeping it as a pure function lets the e2e
//! flows verify the proven result client-side, and lets it be unit-tested
//! without a chain.

use starknet_types_core::felt::Felt;

/// Coin-flip outcome from the public inputs: `0` (heads) or `1` (tails).
///
/// `outcome = pedersen(seed, player) mod 2`, i.e. the LSB of the hash.
pub fn coinflip_outcome(seed: Felt, player: Felt) -> u8 {
    snip36_core::pedersen_hash(&seed, &player).to_bytes_be()[31] & 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_is_binary() {
        for seed in 0u64..32 {
            let o = coinflip_outcome(Felt::from(seed), Felt::from(0x1234u64));
            assert!(o == 0 || o == 1, "outcome must be 0 or 1, got {o}");
        }
    }

    #[test]
    fn outcome_is_deterministic() {
        let seed = Felt::from(42u64);
        let player = Felt::from_hex("0xabc").unwrap();
        assert_eq!(
            coinflip_outcome(seed, player),
            coinflip_outcome(seed, player)
        );
    }

    #[test]
    fn outcome_matches_pedersen_lsb() {
        // Independently recompute the definition and confirm the helper agrees.
        let seed = Felt::from(7u64);
        let player = Felt::from_hex("0xdead").unwrap();
        let expected = snip36_core::pedersen_hash(&seed, &player).to_bytes_be()[31] & 1;
        assert_eq!(coinflip_outcome(seed, player), expected);
    }

    #[test]
    fn outcome_depends_on_inputs() {
        // Different players almost certainly flip some seed's outcome; assert the
        // function is not constant across inputs.
        let player_a = Felt::from(1u64);
        let player_b = Felt::from(2u64);
        let differs = (0u64..64).any(|s| {
            coinflip_outcome(Felt::from(s), player_a) != coinflip_outcome(Felt::from(s), player_b)
        });
        assert!(differs, "outcome should vary with the player input");
    }
}
