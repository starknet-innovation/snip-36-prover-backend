//! Well-known selectors for example contracts (Counter, CoinFlip, Messenger).
//!
//! These are **not** part of the core SDK — your application should define its
//! own selectors.  They live here for convenience so the bundled E2E tests and
//! playground server can reference them without duplicating hex literals.

/// Selector for `increment(amount)` — Counter contract.
pub const INCREMENT_SELECTOR: &str =
    "0x7a44dde9fea32737a5cf3f9683b3235138654aa2d189f6fe44af37a61dc60d";

/// Selector for `get_counter()` — Counter contract.
pub const GET_COUNTER_SELECTOR: &str =
    "0x3370263ab53343580e77063a719a5865004caff7f367ec136a6cdd34b6786ca";

/// Selector for `send_message(to_address, payload)` — Messenger contract.
pub const SEND_MESSAGE_SELECTOR: &str =
    "0x12ead94ae9d3f9d2bdb6b847cf255f1f398193a1f88884a0ae8e18f24a037b6";

/// Selector for `play(seed, player, bet)` — CoinFlip contract.
pub const PLAY_SELECTOR: &str = "0x21c4a0db2b08b026c4e31bf76d5dd9b92aa54c0978df57474355786073775e8";
