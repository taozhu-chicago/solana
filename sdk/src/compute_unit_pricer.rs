use crate::{clock::Slot, ema::AggregatedVarianceStats};

#[derive(Clone, Debug)]
pub struct ComputeUnitPricer {
    /// only for exprimenting println!
    pub slot: Slot,

    /// moving average block_utilization read from previous blocks in percentage (10 means 10%);
    /// this block's tracking stats contribute to next block's average block_utilization
    pub block_utilization: AggregatedVarianceStats,

    /// milli-lamports per CU. The rate dynamically floats based on block_utilization. In general,
    ///    if block_utilization > 90% full, increase the cu_price by 1.125x
    ///    if block_utilization < 50% full, decrease the cu_price by 0.875x
    /// it starts w 1000 milli-lamport/cu
    pub cu_price: u64, // the number of lamports per CU
}

const PRICE_CHANGE_RATE: u64 = 125;
const PRICE_CHANGE_SCALE: u64 = 1_000;
// TODO - make them cli arg?
const BLOCK_UTILIZATION_UPPER_BOUND: u64 = 90;
const BLOCK_UTILIZATION_LOWER_BOUND: u64 = 50;

// NOTE, not setting MIN/MAX cu_price yet for expriment, perhaps a good idea to have them when go
// out of exprimenting
//

impl Default for ComputeUnitPricer {
    fn default() -> Self {
        Self {
            slot: 0,
            block_utilization: AggregatedVarianceStats::default(),
            cu_price: 1_000,
        }
    }
}

impl ComputeUnitPricer {
    // use currently cu_price to calculate total fee in lamports
    pub fn calculate_fee(&self, compute_units: u64) -> u64 {
        compute_units.saturating_mul(self.cu_price).saturating_div(1_000)
    }

    pub fn update(&mut self, slot: Slot, block_cost: u64, block_cost_limit: u64) {
        let prev_block_utilization_ema = self.block_utilization.get_ema();
        let prev_cu_price = self.cu_price;
        let this_block_utilization = block_cost * 100 / block_cost_limit;

        self.slot = slot;
        self.block_utilization.aggregate(this_block_utilization);
        let post_block_utilization_ema = self.block_utilization.get_ema();

        if post_block_utilization_ema >= BLOCK_UTILIZATION_UPPER_BOUND {
            self.cu_price = PRICE_CHANGE_SCALE
                .saturating_add(PRICE_CHANGE_RATE)
                .saturating_mul(self.cu_price.max(10)) // quick hack for in case cu_priced reduced to `0`,
                .saturating_div(PRICE_CHANGE_SCALE);
        } else if post_block_utilization_ema <= BLOCK_UTILIZATION_LOWER_BOUND {
            self.cu_price = PRICE_CHANGE_SCALE
                .saturating_sub(PRICE_CHANGE_RATE)
                .saturating_mul(self.cu_price)
                .saturating_div(PRICE_CHANGE_SCALE);
        }

        println!("=== slot {} block_cost {} block_cost_limit {} this_block_util {} prev_block_util_ems {} post_block_util_ema {} prev_cu_price {} post_cu_price {}",
                 self.slot,
                 block_cost,
                 block_cost_limit,
                 this_block_utilization,
                 prev_block_utilization_ema,
                 post_block_utilization_ema,
                 prev_cu_price,
                 self.cu_price,
                 );
    }
}
