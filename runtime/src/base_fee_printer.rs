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

use solana_sdk::{compute_unit_pricer::ComputeUnitPricer, pubkey::Pubkey, signature::Signature};

#[derive(Debug, Default)]
pub struct BaseFeePrinter {
    pub payer_pubkey: Pubkey,
    pub payer_pre_balance: u64,
    pub payer_post_balance: u64, // if this field is 0/nil, then the payer account was invalid. (eg Not_paid)
    pub tx_sig: Signature,
    pub tx_cost: u64, // the total CU of the TX
    pub tx_priority_fee: u64,
    pub tx_base_fee_orig: u64, // original base fee
    pub tx_base_fee_expt: u64, // the expriment base fee
}

impl BaseFeePrinter {
    pub fn print(&self, compute_unit_pricer: &ComputeUnitPricer) {
        println!(
            "BFP: payer {:?} payer_pre_bal {:?} payer_post_bal {:?} \
            slot {:?} tx_sig {:?} tx_cost {:?} \
            block_utilization_ema {:?} block_utilization_stddev {:?} \
            cu_price {:?} \
            tx_priority_fee {} tx_base_fee {} tx_base_fee_expt {}",
            self.payer_pubkey,
            self.payer_pre_balance,
            self.payer_post_balance,
            compute_unit_pricer.slot,
            self.tx_sig,
            self.tx_cost,
            compute_unit_pricer.block_utilization.get_ema(),
            compute_unit_pricer.block_utilization.get_stddev(),
            compute_unit_pricer.cu_price,
            self.tx_priority_fee,
            self.tx_base_fee_orig,
            self.tx_base_fee_expt,
        );
    }
}
