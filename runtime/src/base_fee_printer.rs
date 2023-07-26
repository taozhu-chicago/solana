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

use {
    solana_cost_model::cost_tracker::ComputeUnitPricer,
    solana_sdk::{pubkey::Pubkey, signature::Signature},
};

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
    pub fn print(&self, compute_unit_pricer: &ComputeUnitPricer) {
        println!(
            "BFP: payer {:?} payer_pre_bal {:?} payer_post_bal {:?} slot {:?} tx_sig {:?} tx_cost {:?} block_utilization {:?} cu_price {:?} tx_base_fee {:?}",
            self.payer_pubkey,
            self.payer_pre_balance,
            self.payer_post_balance,
            compute_unit_pricer.slot,
            self.tx_sig,
            self.tx_cost,
            compute_unit_pricer.block_utilization.get_ema(),
            compute_unit_pricer.cu_price,
            self.tx_base_fee,
        );
    }
}
