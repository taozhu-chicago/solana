use {
    crate::{prioritization_fee::*, transaction_priority_details::TransactionPriorityDetails},
    crossbeam_channel::{unbounded, Receiver, Sender},
    log::*,
    solana_measure::measure,
    solana_sdk::{
        clock::Slot,
        pubkey::Pubkey,
        transaction::{Result, TransactionAccountLocks},
    },
    std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc, Mutex, RwLock,
        },
        thread::{Builder, JoinHandle},
    },
};

/// The maximum number of blocks to keep in `PrioritizationFeeCache`, ie.
/// the amount of history generally desired to estimate the prioritization fee needed to
/// land a transaction in the current block.
const MAX_NUM_RECENT_BLOCKS: u64 = 150;

#[derive(Debug, Default)]
struct PrioritizationFeeCacheMetrics {
    // Accumulated time spent on tracking prioritization fee for each slot.
    total_update_elapsed_us: AtomicU64,

    // Accumulated time spent on acquiring cache write lock.
    total_cache_lock_elapsed_us: AtomicU64,

    // Accumulated time spent on acquiring each block entry's lock..
    total_entry_lock_elapsed_us: AtomicU64,

    // Accumulated time spent on removing old block's data from cache
    total_evict_old_blocks_elapsed_us: AtomicU64,
}

impl PrioritizationFeeCacheMetrics {
    fn increment_total_update_elapsed_us(&self, val: u64) {
        self.total_update_elapsed_us
            .fetch_add(val, Ordering::Relaxed);
    }

    fn increment_total_cache_lock_elapsed_us(&self, val: u64) {
        self.total_cache_lock_elapsed_us
            .fetch_add(val, Ordering::Relaxed);
    }

    fn increment_total_entry_lock_elapsed_us(&self, val: u64) {
        self.total_entry_lock_elapsed_us
            .fetch_add(val, Ordering::Relaxed);
    }

    fn increment_total_evict_old_blocks_elapsed_us(&self, val: u64) {
        self.total_evict_old_blocks_elapsed_us
            .fetch_add(val, Ordering::Relaxed);
    }

    fn report(&self, slot: Slot) {
        datapoint_info!(
            "block_prioritization_fee_counters",
            ("slot", slot as i64, i64),
            (
                "total_update_elapsed_us",
                self.total_update_elapsed_us.swap(0, Ordering::Relaxed) as i64,
                i64
            ),
            (
                "total_cache_lock_elapsed_us",
                self.total_cache_lock_elapsed_us.swap(0, Ordering::Relaxed) as i64,
                i64
            ),
            (
                "total_entry_lock_elapsed_us",
                self.total_entry_lock_elapsed_us.swap(0, Ordering::Relaxed) as i64,
                i64
            ),
            (
                "total_evict_old_blocks_elapsed_us",
                self.total_evict_old_blocks_elapsed_us
                    .swap(0, Ordering::Relaxed) as i64,
                i64
            ),
        );
    }
}

/// Each block's PrioritizationFee entry is wrapped in Arc<Mutex<...>>
/// Reader and writer should avoid to contend `PrioritizationFeeCache` but on individual block's PrioritizationFeeEntry.
/// Each entry is assigned a unique incremental sequence_number, which is used to enforce eviction
/// policy.
struct PrioritizationFeeEntry {
    entry: Arc<Mutex<PrioritizationFee>>,
    sequence_number: u64,
}

impl PrioritizationFeeEntry {
    pub fn new(entry: Arc<Mutex<PrioritizationFee>>, sequence_number: u64) -> Self {
        PrioritizationFeeEntry {
            entry,
            sequence_number,
        }
    }

    pub fn entry(&self) -> Arc<Mutex<PrioritizationFee>> {
        self.entry.clone()
    }

    pub fn sequence_number(&self) -> u64 {
        self.sequence_number
    }
}

enum FinalizingServiceUpdate {
    TransactionUpdate {
        slot: Slot,
        transaction_fee: u64,
        writable_accounts: Arc<Vec<Pubkey>>,
    },
    BankFrozen {
        slot: Slot,
    },
    Exit,
}

/// Stores up to MAX_NUM_RECENT_BLOCKS recent block's prioritization fee,
/// A separate internal thread `finalizing_thread` handles additional tasks when a bank is frozen,
/// includes pruning PrioritizationFee's HashMap, collecting stats and reporting metrics.
pub struct PrioritizationFeeCache {
    cache: Arc<RwLock<HashMap<Slot, Arc<PrioritizationFeeEntry>>>>,
    current_sequence_number: AtomicU64,
    // Asynchronously finalize prioritization fee when a bank is completed replay.
    finalizing_thread: Option<JoinHandle<()>>,
    sender: Sender<FinalizingServiceUpdate>,
    metrics: Arc<PrioritizationFeeCacheMetrics>,
}

impl Default for PrioritizationFeeCache {
    fn default() -> Self {
        Self::new(MAX_NUM_RECENT_BLOCKS)
    }
}

impl Drop for PrioritizationFeeCache {
    fn drop(&mut self) {
        let _ = self.sender.send(FinalizingServiceUpdate::Exit);
        self.finalizing_thread
            .take()
            .unwrap()
            .join()
            .expect("Prioritization fee cache finalizing thread failed to join");
    }
}

impl PrioritizationFeeCache {
    pub fn new(capacity: u64) -> Self {
        let metrics = Arc::new(PrioritizationFeeCacheMetrics::default());
        let (sender, receiver) = unbounded();
        let cache = Arc::new(RwLock::new(HashMap::with_capacity(capacity as usize)));

        let cache_clone = cache.clone();
        let metrics_clone = metrics.clone();
        let finalizing_thread = Some(
            Builder::new()
                .name("prioritization-fee-cache-finalizing-thread".to_string())
                .spawn(move || {
                    Self::finalizing_loop(cache_clone, receiver, metrics_clone);
                })
                .unwrap(),
        );

        PrioritizationFeeCache {
            cache,
            current_sequence_number: AtomicU64::default(),
            finalizing_thread,
            sender,
            metrics,
        }
    }

    /// Get prioritization fee entry, create new entry if necessary
    fn get_prioritization_fee(
        cache: Arc<RwLock<HashMap<Slot, Arc<PrioritizationFeeEntry>>>>,
        slot: &Slot,
    ) -> Arc<PrioritizationFeeEntry> {
        let mut cache = cache.write().unwrap();
        match cache.get(slot) {
            Some(entry) => Arc::clone(entry),
            None => {
                /* TODO TAO - old block eviction is broken, will be replaced with LruCache
                //let sequence_number = self.current_sequence_number.fetch_add(1, Ordering::Relaxed);
                // */
                let sequence_number = 1;

                let entry = Arc::new(PrioritizationFeeEntry::new(
                    Arc::new(Mutex::new(PrioritizationFee::default())),
                    sequence_number,
                ));
                cache.insert(*slot, Arc::clone(&entry));
                entry
            }
        }
    }

    /// Update with a list of transactions' tx_priority_details and tx_account_locks; Only
    /// transactions have both valid priority_detail and account_locks will be used to update
    /// fee_cache asynchronously.
    pub fn update(
        &self,
        slot: Slot,
        tx_priority_details: &[Option<TransactionPriorityDetails>],
        tx_account_locks: &[Result<TransactionAccountLocks>],
    ) {
        for (account_locks, priority_detail) in
            tx_account_locks.iter().zip(tx_priority_details.iter())
        {
            if account_locks.is_ok() && priority_detail.is_some() {
                let writable_accounts = Arc::new(
                    account_locks
                        .as_ref()
                        .unwrap()
                        .writable
                        .iter()
                        .map(|key| **key)
                        .collect(),
                );

                self.sender
                    .send(FinalizingServiceUpdate::TransactionUpdate {
                        slot,
                        transaction_fee: priority_detail.as_ref().unwrap().priority,
                        writable_accounts,
                    })
                    .unwrap_or_else(|err| {
                        warn!(
                            "prioritization fee cache transaction updates failed: {:?}",
                            err
                        );
                    });
            }
        }
    }

    /// Finalize prioritization fee when it's bank is completely replayed from blockstore,
    /// by pruning irrelevant accounts to save space, and marking its availability for queries.
    pub fn finalize_priority_fee(&self, slot: Slot) {
        self.evict_old_blocks(MAX_NUM_RECENT_BLOCKS);

        self.sender
            .send(FinalizingServiceUpdate::BankFrozen { slot })
            .unwrap_or_else(|err| {
                warn!(
                    "prioritization fee cache signalling bank frozen failed: {:?}",
                    err
                )
            });
    }

    /// Internal function is invoked by worker thread to update slot's minimum prioritization fee,
    /// Cache lock contends here.
    fn update_transactions(
        cache: Arc<RwLock<HashMap<Slot, Arc<PrioritizationFeeEntry>>>>,
        slot: &Slot,
        transaction_fee: u64,
        writable_accounts: Arc<Vec<Pubkey>>,
        metrics: Arc<PrioritizationFeeCacheMetrics>,
    ) {
        let ((cache_lock_time, entry_lock_time), cache_update_time) = measure!(
            {
                let (block_prioritization_fee, cache_lock_time) = measure!(
                    Self::get_prioritization_fee(cache, slot).entry(),
                    "cache_lock_time",
                );

                // Hold lock of slot's prioritization fee entry until all transactions are
                // processed
                let (mut block_prioritization_fee, entry_lock_time) =
                    measure!(block_prioritization_fee.lock().unwrap(), "entry_lock_time",);

                let _ = block_prioritization_fee.update(transaction_fee, &writable_accounts);

                (cache_lock_time, entry_lock_time)
            },
            "cache_update"
        );

        metrics.increment_total_cache_lock_elapsed_us(cache_lock_time.as_us());
        metrics.increment_total_entry_lock_elapsed_us(entry_lock_time.as_us());
        metrics.increment_total_update_elapsed_us(cache_update_time.as_us());
    }

    /// PrioritizationFeeCache holds up to MAX_NUM_RECENT_BLOCKS, older blocks are evicted by
    /// checking its sequence number against cache current sequence.
    fn evict_old_blocks(&self, max_age: u64) {
        let (_, evict_old_blocks_time) = measure!(
            {
                let mut cache = self.cache.write().unwrap();
                cache.retain(|_key, prioritization_fee| {
                    self.current_sequence_number
                        .load(Ordering::Relaxed)
                        .saturating_sub(prioritization_fee.sequence_number())
                        <= max_age
                });
            },
            "evict_old_blocks_time"
        );

        self.metrics
            .increment_total_evict_old_blocks_elapsed_us(evict_old_blocks_time.as_us());
    }

    fn finalizing_loop(
        cache: Arc<RwLock<HashMap<Slot, Arc<PrioritizationFeeEntry>>>>,
        receiver: Receiver<FinalizingServiceUpdate>,
        metrics: Arc<PrioritizationFeeCacheMetrics>,
    ) {
        for update in receiver.iter() {
            match update {
                FinalizingServiceUpdate::TransactionUpdate {
                    slot,
                    transaction_fee,
                    writable_accounts,
                } => Self::update_transactions(
                    cache.clone(),
                    &slot,
                    transaction_fee,
                    writable_accounts,
                    metrics.clone(),
                ),
                FinalizingServiceUpdate::BankFrozen { slot } => {
                    let (prioritization_fee, cache_lock_time) = measure!(
                        Self::get_prioritization_fee(cache.clone(), &slot).entry(),
                        "cache_lock_time",
                    );
                    metrics.increment_total_cache_lock_elapsed_us(cache_lock_time.as_us());

                    // prune cache by evicting write account entry from prioritization fee if its fee is less
                    // or equal to block's minimum transaction fee, because they are irrelevant in calculating
                    // block minimum fee.
                    {
                        let mut prioritization_fee = prioritization_fee.lock().unwrap();
                        let _ = prioritization_fee.mark_block_completed();
                        prioritization_fee.report_metrics(slot);
                    }
                    metrics.report(slot);
                }
                FinalizingServiceUpdate::Exit => {
                    break;
                }
            }
        }
    }

    /// Returns number of blocks that have finalized minimum fees collection
    pub fn available_block_count(&self) -> usize {
        self.cache
            .read()
            .unwrap()
            .iter()
            .filter(|(_slot, prioritization_fee)| {
                prioritization_fee.entry().lock().unwrap().is_finalized()
            })
            .count()
    }

    /// Query block minimum fees from finalized blocks in cache,
    /// Returns a vector of fee; call site can use it to produce
    /// average, or top 5% etc.
    pub fn get_prioritization_fees(&self) -> Vec<u64> {
        self.cache
            .read()
            .unwrap()
            .iter()
            .filter_map(|(_slot, prioritization_fee)| {
                let prioritization_fee = prioritization_fee.entry();
                let prioritization_fee_read = prioritization_fee.lock().unwrap();
                prioritization_fee_read
                    .is_finalized()
                    .then(|| prioritization_fee_read.get_min_transaction_fee())
            })
            .flatten()
            .collect()
    }

    /// Query given account minimum fees from finalized blocks in cache,
    /// Returns a vector of fee; call site can use it to produce
    /// average, or top 5% etc.
    pub fn get_account_prioritization_fees(&self, account_key: &Pubkey) -> Vec<u64> {
        self.cache
            .read()
            .unwrap()
            .iter()
            .filter_map(|(_slot, prioritization_fee)| {
                let prioritization_fee = prioritization_fee.entry();
                let prioritization_fee_read = prioritization_fee.lock().unwrap();
                prioritization_fee_read
                    .is_finalized()
                    .then(|| prioritization_fee_read.get_writable_account_fee(account_key))
            })
            .flatten()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::transaction_priority_details::GetTransactionPriorityDetails,
        solana_sdk::{
            compute_budget::ComputeBudgetInstruction,
            message::Message,
            pubkey::Pubkey,
            system_instruction,
            transaction::{SanitizedTransaction, Transaction},
        },
    };

    fn build_sanitized_transaction_for_test(
        compute_unit_price: u64,
        signer_account: &Pubkey,
        write_account: &Pubkey,
    ) -> SanitizedTransaction {
        let transaction = Transaction::new_unsigned(Message::new(
            &[
                system_instruction::transfer(signer_account, write_account, 1),
                ComputeBudgetInstruction::set_compute_unit_price(compute_unit_price),
            ],
            Some(signer_account),
        ));

        SanitizedTransaction::try_from_legacy_transaction(transaction).unwrap()
    }

    fn get_tx_priority_details(
        transactions: &[SanitizedTransaction],
    ) -> Vec<Option<TransactionPriorityDetails>> {
        transactions
            .iter()
            .map(|tx| tx.get_transaction_priority_details())
            .collect()
    }

    fn get_tx_account_locks(
        transactions: &[SanitizedTransaction],
    ) -> Vec<Result<TransactionAccountLocks>> {
        transactions
            .iter()
            .map(|tx| tx.get_account_locks(64))
            .collect()
    }

    // finalization is asynchronous, this test helper will block until finalization is completed.
    fn sync_finalize_priority_fee_for_test(
        prioritization_fee_cache: &mut PrioritizationFeeCache,
        slot: Slot,
    ) {
        prioritization_fee_cache.finalize_priority_fee(slot);
        let fee = PrioritizationFeeCache::get_prioritization_fee(
            prioritization_fee_cache.cache.clone(),
            &slot,
        )
        .entry();

        // wait till finalization is done
        while !fee.lock().unwrap().is_finalized() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[test]
    fn test_prioritization_fee_cache_update() {
        solana_logger::setup();
        let write_account_a = Pubkey::new_unique();
        let write_account_b = Pubkey::new_unique();
        let write_account_c = Pubkey::new_unique();

        // Set up test with 3 transactions, in format of [fee, write-accounts...],
        // Shall expect fee cache is updated in following sequence:
        // transaction                    block minimum prioritization fee cache
        // [fee, write_accounts...]  -->  [block, account_a, account_b, account_c]
        // -----------------------------------------------------------------------
        // [5,   a, b             ]  -->  [5,     5,         5,         nil      ]
        // [9,      b, c          ]  -->  [5,     5,         5,         9        ]
        // [2,   a,    c          ]  -->  [2,     2,         5,         2        ]
        //
        let txs = vec![
            build_sanitized_transaction_for_test(5, &write_account_a, &write_account_b),
            build_sanitized_transaction_for_test(9, &write_account_b, &write_account_c),
            build_sanitized_transaction_for_test(2, &write_account_a, &write_account_c),
        ];

        let slot = 1;

        let mut prioritization_fee_cache = PrioritizationFeeCache::default();
        prioritization_fee_cache.update(
            slot,
            &get_tx_priority_details(&txs),
            &get_tx_account_locks(&txs),
        );

        // assert block minimum fee and account a, b, c fee accordingly
        {
            let fee = PrioritizationFeeCache::get_prioritization_fee(
                prioritization_fee_cache.cache.clone(),
                &slot,
            )
            .entry();
            let fee = fee.lock().unwrap();
            assert_eq!(2, fee.get_min_transaction_fee().unwrap());
            assert_eq!(2, fee.get_writable_account_fee(&write_account_a).unwrap());
            assert_eq!(5, fee.get_writable_account_fee(&write_account_b).unwrap());
            assert_eq!(2, fee.get_writable_account_fee(&write_account_c).unwrap());
            // assert unknown account d fee
            assert!(fee
                .get_writable_account_fee(&Pubkey::new_unique())
                .is_none());
        }

        // assert after prune, account a and c should be removed from cache to save space
        {
            sync_finalize_priority_fee_for_test(&mut prioritization_fee_cache, slot);
            let fee = PrioritizationFeeCache::get_prioritization_fee(
                prioritization_fee_cache.cache.clone(),
                &slot,
            )
            .entry();
            let fee = fee.lock().unwrap();
            assert_eq!(2, fee.get_min_transaction_fee().unwrap());
            assert!(fee.get_writable_account_fee(&write_account_a).is_none());
            assert_eq!(5, fee.get_writable_account_fee(&write_account_b).unwrap());
            assert!(fee.get_writable_account_fee(&write_account_c).is_none());
        }
    }

    #[test]
    fn test_available_block_count() {
        let prioritization_fee_cache = PrioritizationFeeCache::default();

        assert!(PrioritizationFeeCache::get_prioritization_fee(
            prioritization_fee_cache.cache.clone(),
            &1
        )
        .entry()
        .lock()
        .unwrap()
        .mark_block_completed()
        .is_ok());
        assert!(PrioritizationFeeCache::get_prioritization_fee(
            prioritization_fee_cache.cache.clone(),
            &2
        )
        .entry()
        .lock()
        .unwrap()
        .mark_block_completed()
        .is_ok());
        // add slot 3 entry to cache, but not finalize it
        PrioritizationFeeCache::get_prioritization_fee(prioritization_fee_cache.cache.clone(), &3);

        // assert available block count should be 2 finalized blocks
        assert_eq!(2, prioritization_fee_cache.available_block_count());
    }

    fn assert_vec_eq(expected: &mut Vec<u64>, actual: &mut Vec<u64>) {
        expected.sort_unstable();
        actual.sort_unstable();
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_get_prioritization_fees() {
        solana_logger::setup();
        let write_account_a = Pubkey::new_unique();
        let write_account_b = Pubkey::new_unique();
        let write_account_c = Pubkey::new_unique();

        let mut prioritization_fee_cache = PrioritizationFeeCache::default();

        // Assert no minimum fee from empty cache
        assert!(prioritization_fee_cache
            .get_prioritization_fees()
            .is_empty());

        // Assert after add one transaction for slot 1
        {
            let txs = vec![build_sanitized_transaction_for_test(
                5,
                &write_account_a,
                &write_account_b,
            )];
            prioritization_fee_cache.update(
                1,
                &get_tx_priority_details(&txs),
                &get_tx_account_locks(&txs),
            );
            assert_eq!(
                5,
                PrioritizationFeeCache::get_prioritization_fee(
                    prioritization_fee_cache.cache.clone(),
                    &1
                )
                .entry
                .lock()
                .unwrap()
                .get_min_transaction_fee()
                .unwrap()
            );
            // before block is marked as completed
            assert!(prioritization_fee_cache
                .get_prioritization_fees()
                .is_empty());
            // after block is completed
            sync_finalize_priority_fee_for_test(&mut prioritization_fee_cache, 1);
            assert_eq!(vec![5], prioritization_fee_cache.get_prioritization_fees());
        }

        // Assert after add one transaction for slot 2
        {
            let txs = vec![build_sanitized_transaction_for_test(
                9,
                &write_account_b,
                &write_account_c,
            )];
            prioritization_fee_cache.update(
                2,
                &get_tx_priority_details(&txs),
                &get_tx_account_locks(&txs),
            );
            assert_eq!(
                9,
                PrioritizationFeeCache::get_prioritization_fee(
                    prioritization_fee_cache.cache.clone(),
                    &2
                )
                .entry
                .lock()
                .unwrap()
                .get_min_transaction_fee()
                .unwrap()
            );
            // before block is marked as completed
            assert_eq!(vec![5], prioritization_fee_cache.get_prioritization_fees());
            // after block is completed
            sync_finalize_priority_fee_for_test(&mut prioritization_fee_cache, 2);
            assert_vec_eq(
                &mut vec![5, 9],
                &mut prioritization_fee_cache.get_prioritization_fees(),
            );
        }

        // Assert after add one transaction for slot 3
        {
            let txs = vec![build_sanitized_transaction_for_test(
                2,
                &write_account_a,
                &write_account_c,
            )];
            prioritization_fee_cache.update(
                3,
                &get_tx_priority_details(&txs),
                &get_tx_account_locks(&txs),
            );
            assert_eq!(
                2,
                PrioritizationFeeCache::get_prioritization_fee(
                    prioritization_fee_cache.cache.clone(),
                    &3
                )
                .entry
                .lock()
                .unwrap()
                .get_min_transaction_fee()
                .unwrap()
            );
            // before block is marked as completed
            assert_vec_eq(
                &mut vec![5, 9],
                &mut prioritization_fee_cache.get_prioritization_fees(),
            );
            // after block is completed
            sync_finalize_priority_fee_for_test(&mut prioritization_fee_cache, 3);
            assert_vec_eq(
                &mut vec![5, 9, 2],
                &mut prioritization_fee_cache.get_prioritization_fees(),
            );
        }
    }

    #[test]
    fn test_get_account_prioritization_fees() {
        solana_logger::setup();
        let write_account_a = Pubkey::new_unique();
        let write_account_b = Pubkey::new_unique();
        let write_account_c = Pubkey::new_unique();

        let mut prioritization_fee_cache = PrioritizationFeeCache::default();

        // Assert no minimum fee from empty cache
        assert!(prioritization_fee_cache
            .get_account_prioritization_fees(&write_account_a)
            .is_empty());
        assert!(prioritization_fee_cache
            .get_account_prioritization_fees(&write_account_b)
            .is_empty());
        assert!(prioritization_fee_cache
            .get_account_prioritization_fees(&write_account_c)
            .is_empty());

        // Assert after add one transaction for slot 1
        {
            let txs = vec![
                build_sanitized_transaction_for_test(5, &write_account_a, &write_account_b),
                build_sanitized_transaction_for_test(
                    0,
                    &Pubkey::new_unique(),
                    &Pubkey::new_unique(),
                ),
            ];
            prioritization_fee_cache.update(
                1,
                &get_tx_priority_details(&txs),
                &get_tx_account_locks(&txs),
            );
            // before block is marked as completed
            assert!(prioritization_fee_cache
                .get_account_prioritization_fees(&write_account_a)
                .is_empty());
            assert!(prioritization_fee_cache
                .get_account_prioritization_fees(&write_account_b)
                .is_empty());
            assert!(prioritization_fee_cache
                .get_account_prioritization_fees(&write_account_c)
                .is_empty());
            // after block is completed
            sync_finalize_priority_fee_for_test(&mut prioritization_fee_cache, 1);
            assert_eq!(
                vec![5],
                prioritization_fee_cache.get_account_prioritization_fees(&write_account_a)
            );
            assert_eq!(
                vec![5],
                prioritization_fee_cache.get_account_prioritization_fees(&write_account_b)
            );
            assert!(prioritization_fee_cache
                .get_account_prioritization_fees(&write_account_c)
                .is_empty());
        }

        // Assert after add one transaction for slot 2
        {
            let txs = vec![
                build_sanitized_transaction_for_test(9, &write_account_b, &write_account_c),
                build_sanitized_transaction_for_test(
                    0,
                    &Pubkey::new_unique(),
                    &Pubkey::new_unique(),
                ),
            ];
            prioritization_fee_cache.update(
                2,
                &get_tx_priority_details(&txs),
                &get_tx_account_locks(&txs),
            );
            // before block is marked as completed
            assert_eq!(
                vec![5],
                prioritization_fee_cache.get_account_prioritization_fees(&write_account_a)
            );
            assert_eq!(
                vec![5],
                prioritization_fee_cache.get_account_prioritization_fees(&write_account_b)
            );
            assert!(prioritization_fee_cache
                .get_account_prioritization_fees(&write_account_c)
                .is_empty());
            // after block is completed
            sync_finalize_priority_fee_for_test(&mut prioritization_fee_cache, 2);
            assert_eq!(
                vec![5],
                prioritization_fee_cache.get_account_prioritization_fees(&write_account_a)
            );
            assert_vec_eq(
                &mut vec![5, 9],
                &mut prioritization_fee_cache.get_account_prioritization_fees(&write_account_b),
            );
            assert_eq!(
                vec![9],
                prioritization_fee_cache.get_account_prioritization_fees(&write_account_c)
            );
        }

        // Assert after add one transaction for slot 3
        {
            let txs = vec![
                build_sanitized_transaction_for_test(2, &write_account_a, &write_account_c),
                build_sanitized_transaction_for_test(
                    0,
                    &Pubkey::new_unique(),
                    &Pubkey::new_unique(),
                ),
            ];
            prioritization_fee_cache.update(
                3,
                &get_tx_priority_details(&txs),
                &get_tx_account_locks(&txs),
            );
            // before block is marked as completed
            assert_eq!(
                vec![5],
                prioritization_fee_cache.get_account_prioritization_fees(&write_account_a)
            );
            assert_vec_eq(
                &mut vec![5, 9],
                &mut prioritization_fee_cache.get_account_prioritization_fees(&write_account_b),
            );
            assert_eq!(
                vec![9],
                prioritization_fee_cache.get_account_prioritization_fees(&write_account_c)
            );
            // after block is completed
            sync_finalize_priority_fee_for_test(&mut prioritization_fee_cache, 3);
            assert_vec_eq(
                &mut vec![5, 2],
                &mut prioritization_fee_cache.get_account_prioritization_fees(&write_account_a),
            );
            assert_vec_eq(
                &mut vec![5, 9],
                &mut prioritization_fee_cache.get_account_prioritization_fees(&write_account_b),
            );
            assert_vec_eq(
                &mut vec![9, 2],
                &mut prioritization_fee_cache.get_account_prioritization_fees(&write_account_c),
            );
        }
    }

    #[test]
    fn test_evict_old_blocks() {
        let prioritization_fee_cache = PrioritizationFeeCache::default();

        // add 3 blocks (slot 1, 3, 7) into cache
        PrioritizationFeeCache::get_prioritization_fee(prioritization_fee_cache.cache.clone(), &1);
        PrioritizationFeeCache::get_prioritization_fee(prioritization_fee_cache.cache.clone(), &3);
        PrioritizationFeeCache::get_prioritization_fee(prioritization_fee_cache.cache.clone(), &7);
        PrioritizationFeeCache::get_prioritization_fee(prioritization_fee_cache.cache.clone(), &3);
        PrioritizationFeeCache::get_prioritization_fee(prioritization_fee_cache.cache.clone(), &1);

        // assert there are 3 blocks in cache
        {
            let cache = prioritization_fee_cache.cache.read().unwrap();
            assert_eq!(3, cache.len());
            assert!(cache.contains_key(&1));
            assert!(cache.contains_key(&3));
            assert!(cache.contains_key(&7));
        }

        // evict with up to 2 recent blocks
        prioritization_fee_cache.evict_old_blocks(2);

        // assert that oldest slot 1 is evicted
        {
            let cache = prioritization_fee_cache.cache.read().unwrap();
            assert_eq!(2, cache.len());
            assert!(!cache.contains_key(&1));
            assert!(cache.contains_key(&3));
            assert!(cache.contains_key(&7));
        }
    }
}
