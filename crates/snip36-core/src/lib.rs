//! Use-case-independent SDK for SNIP-36 virtual block proving on Starknet.
//!
//! Provides configuration, RPC, signing, and proof utilities that any SNIP-36
//! application can build on.

#[cfg(feature = "cli")]
pub mod cli_util;
pub mod config;
pub mod proof;
pub mod rpc;
pub mod signing;
pub mod types;

pub use config::Config;
pub use starknet_crypto::pedersen_hash;
pub use starknet_crypto::poseidon_hash_many;
pub use types::*;
