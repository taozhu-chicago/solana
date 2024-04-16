#![cfg_attr(RUSTC_WITH_SPECIALIZATION, feature(min_specialization))]
#![allow(clippy::arithmetic_side_effects)]
use {lazy_static::lazy_static, solana_sdk::pubkey::Pubkey, std::collections::HashMap};

// Number of compute units for each built-in programs
lazy_static! {
    /// Number of compute units for each built-in programs
    pub static ref BUILT_IN_INSTRUCTION_COSTS: HashMap<Pubkey, u64> = [
        (solana_sdk::address_lookup_table::program::id(), solana_sdk::address_lookup_table::instruction::DEFAULT_COMPUTE_UNITS),
        (solana_sdk::bpf_loader::id(), solana_sdk::bpf_loader::DEFAULT_COMPUTE_UNITS),
        (solana_sdk::bpf_loader_deprecated::id(), solana_sdk::bpf_loader_deprecated::DEFAULT_COMPUTE_UNITS),
        (solana_sdk::bpf_loader_upgradeable::id(), solana_sdk::bpf_loader_upgradeable::DEFAULT_COMPUTE_UNITS),
        (solana_sdk::compute_budget::id(), solana_sdk::compute_budget::DEFAULT_COMPUTE_UNITS),
        (solana_sdk::config::program::id(), solana_sdk::config::program::DEFAULT_COMPUTE_UNITS),
        (solana_sdk::loader_v4::id(), solana_sdk::loader_v4::DEFAULT_COMPUTE_UNITS),
        (solana_sdk::stake::program::id(), solana_sdk::stake::instruction::DEFAULT_COMPUTE_UNITS),
        (solana_sdk::system_program::id(), solana_sdk::system_program::DEFAULT_COMPUTE_UNITS),
        (solana_sdk::vote::program::id(), solana_sdk::vote::instruction::DEFAULT_COMPUTE_UNITS),
        // Note: These are precompile, run directly in bank during sanitizing;
        (solana_sdk::ed25519_program::id(), 0),
        (solana_sdk::secp256k1_program::id(), 0),
    ]
    .iter()
    .cloned()
    .collect();
}
