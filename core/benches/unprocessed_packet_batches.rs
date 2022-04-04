#![allow(clippy::integer_arithmetic)]
#![feature(test)]

extern crate test;

use {
    min_max_heap::MinMaxHeap,
    rand::{
        distributions::{Distribution, Uniform},
        seq::SliceRandom,
    },
    solana_core::{
        unprocessed_packet_batches::*,
    },
    solana_measure::measure::Measure,
    solana_perf::packet::{Packet, PacketBatch},
    solana_sdk::{
        hash::Hash,
        signature::Keypair,
        system_transaction,
    },
    test::Bencher,
};

fn build_packet_batch(
    packet_per_batch_count: usize,
    ) -> (PacketBatch, Vec<usize>) {
    let packet_batch = PacketBatch::new(
        (0..packet_per_batch_count)
        .map(|sender_stake| {
            let tx = system_transaction::transfer(
                &Keypair::new(),
                &solana_sdk::pubkey::new_rand(),
                1,
                Hash::new_unique(),
            );
            let mut packet = Packet::from_data(None, &tx).unwrap();
            packet.meta.sender_stake = sender_stake as u64;
            packet
        })
        .collect()
    );
    let packet_indexes: Vec<usize> = (0..packet_per_batch_count).collect();

    (packet_batch, packet_indexes)
}

fn build_randomized_packet_batch(
    packet_per_batch_count: usize,
    ) -> (PacketBatch, Vec<usize>) {
    let mut rng = rand::thread_rng();
    let distribution = Uniform::from(0..200_000);

    let packet_batch = PacketBatch::new(
        (0..packet_per_batch_count)
        .map(|_| {
            let tx = system_transaction::transfer(
                &Keypair::new(),
                &solana_sdk::pubkey::new_rand(),
                1,
                Hash::new_unique(),
            );
            let mut packet = Packet::from_data(None, &tx).unwrap();
            let sender_stake = distribution.sample(&mut rng);
            packet.meta.sender_stake = sender_stake as u64;
            packet
        })
        .collect()
    );
    let packet_indexes: Vec<usize> = (0..packet_per_batch_count).collect();

    (packet_batch, packet_indexes)
}

fn insert_packet_batches(
    buffer_max_size: usize,
    batch_count: usize,
    packet_per_batch_count: usize,
    randomize: bool,
){
    solana_logger::setup();
    let mut unprocessed_packet_batches = UnprocessedPacketBatches::with_capacity(buffer_max_size);

    let mut timer = Measure::start("insert_batch");
    (0..batch_count).for_each(|_| {
        let (packet_batch, packet_indexes) = if randomize { 
            build_randomized_packet_batch(packet_per_batch_count)
        } else {
            build_packet_batch(packet_per_batch_count)
        };
        unprocessed_packet_batches.insert_batch(
            DeserializedPacketBatch::new(packet_batch, packet_indexes, false),
            buffer_max_size
        );
    });
    timer.stop();
    log::info!("inserted {} batch, elapsed {}", buffer_max_size, timer.as_us());
}

//*
// bench: 5,600,038,163 ns/iter (+/- 940,818,988)
#[bench]
fn bench_unprocessed_packet_batches_within_limit(bencher: &mut Bencher) {
    let buffer_capacity = 1_000;
    let batch_count = 1_000;
    let packet_per_batch_count = 128;

    bencher.iter(|| {
        insert_packet_batches(buffer_capacity, batch_count, packet_per_batch_count, false);
    });
}

// bench: 6,607,014,940 ns/iter (+/- 768,191,361)
#[bench]
fn bench_unprocessed_packet_batches_beyond_limit(bencher: &mut Bencher) {
    let buffer_capacity = 1_000;
    let batch_count = 1_100;
    let packet_per_batch_count = 128;

    // this is the worst scenario testing: all batches are uniformly populated with packets from
    // priority 100..228, so in order to drop a batch, algo will have to drop all packets that has
    // priority < 228, plus one 228. That's 2000 batch * 127 packets + 1
    // Also, since all batches have same stake distribution, the new one is always the one got
    // dropped. Tho it does not change algo complexity. 
    bencher.iter(|| {
        insert_packet_batches(buffer_capacity, batch_count, packet_per_batch_count, false);
    });
}
// */

// bench: 5,843,307,086 ns/iter (+/- 844,249,298)
#[bench]
fn bench_unprocessed_packet_batches_randomized_within_limit(bencher: &mut Bencher) {
    let buffer_capacity = 1_000;
    let batch_count = 1_000;
    let packet_per_batch_count = 128;

    bencher.iter(|| {
        insert_packet_batches(buffer_capacity, batch_count, packet_per_batch_count, false);
    });
}

// bench: 6,497,623,849 ns/iter (+/- 3,206,382,212)
#[bench]
fn bench_unprocessed_packet_batches_randomized_beyond_limit(bencher: &mut Bencher) {
    let buffer_capacity = 1_000;
    let batch_count = 1_100;
    let packet_per_batch_count = 128;

    bencher.iter(|| {
        insert_packet_batches(buffer_capacity, batch_count, packet_per_batch_count, true);
    });
}

//*
// bench:     125,923 ns/iter (+/- 12,539)
#[bench]
fn bench_unprocessed_packet_batches_vector(bencher: &mut Bencher) {
    let buffer_max_size = 100_000;
    let mut unprocessed_packet_batches = vec![];

    bencher.iter(|| {
        for i in 0..buffer_max_size {
            unprocessed_packet_batches.push(i);
        }

        unprocessed_packet_batches.clear();
    });
}

// bench:   2,264,168 ns/iter (+/- 61,353)
#[bench]
fn bench_unprocessed_packet_batches_min_max_heap(bencher: &mut Bencher) {
    let buffer_max_size = 100_000;
    let mut unprocessed_packet_batches = MinMaxHeap::default();
    let mut weights: Vec<usize> = (0..buffer_max_size).collect();
    weights.shuffle(&mut rand::thread_rng());

    bencher.iter(|| {
        // min_max_heap has O(log n) for insertion 
        for weight in &weights {
            unprocessed_packet_batches.push(weight);
        }

        unprocessed_packet_batches.clear();
    });
}
// */
