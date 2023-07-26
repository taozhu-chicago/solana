//! experimenting pricing CU for dynamic base fee.
//! 1. vote transactino continue use signature based base-fee cal (no prio fee either)
//! 2. only for non-vote transaction:
//!    a. transaction base_fee is CU_price * get_cu(&tx), where `get_cu` can be either just
//!       sum(sig, requested_cu, account data size), or entire TX's cu from cost_model;
//!    b. where cu_price is initial to be 1 lamport/cu, or cal based on simple tx tx,
//!    c. and if N blocks on average > 90% full increase the CU cost by 1.125x
//!           if N blocks on average < 50% full, decrease the CU cost by 0.875x
//!           where N could be 16 to start with
//!    d. add the min/max if necessary

use solana_sdk::{clock::Slot, pubkey::Pubkey, signature::Signature};

// add this to `bank` as member,
// use Welford's variance algorithm to calculate mean and variance last N blocks utilization when
//     new bank is created
// pass this object to accounts::load_accounts()
// at `accounts` call BaseFeePrinter.print() per tx with info from it
//
#[derive(Debug, Default)]
pub struct PricedComputeUnits {
    pub slot: Slot,
    pub block_utilization: u8, // Wolford's calculated simple moving average, in percentage number (10 means 10%)
    pub cu_price: u64,         // the number of lamports per CU
}

impl PricedComputeUnits {}

#[derive(Debug, Default)]
pub struct BaseFeePrinter {
    pub payer_pubkey: Pubkey,
    pub payer_pre_balance: u64,
    pub payer_post_balance: u64, // if this field is 0/nil, then the payer account was invalid. (eg Not_paid)
    pub tx_sig: Signature,
    pub tx_cost: u64, // the total CU of the TX that contributes to base fee.
    // two possibilities to experiment:
    // sig + requested_cu + loaded_accounts_size (basically current state), or
    // tx.calculate_cost() (eg, the future state, all CU of tx used for
    // scheduling and paying)
    pub tx_base_fee: u64, // whatever returned from calculate_fee()
}

impl BaseFeePrinter {
    pub fn print(&self, priced_compute_units: &PricedComputeUnits) {
        println!(
            "BFP: payer {:?} payer_pre_bal {:?} payer_post_bal {:?} slot {:?} tx_sig {:?} tx_cost {:?} block_utilization {:?} cu_price {:?} tx_base_fee {:?}",
            self.payer_pubkey,
            self.payer_pre_balance,
            self.payer_post_balance,
            priced_compute_units.slot,
            self.tx_sig,
            self.tx_cost,
            priced_compute_units.block_utilization,
            priced_compute_units.cu_price,
            self.tx_base_fee,
        );
    }
}
