use {
    min_max_heap::MinMaxHeap,
    std::{
        cell::RefCell,
        collections::{HashMap, VecDeque},
        cmp::Ordering,
        rc::{Rc, Weak},
    },
    rand::{
        distributions::{Distribution, Uniform},
    },
};

/// storage is a nested struct, priority_flat_index flats out the underlying object, index by its
/// priority
///
/// 1. Buffer is operated at Batch level, eg insert_batch, remove_batch ...
/// 2. Prioritization is operated on packet level, by packet.priority
#[derive(Default)]
pub struct Buffer(VecDeque<Rc<RefCell<Batch>>>);

/// index lives outside of buffer for now
pub type Index = MinMaxHeap<Rc<Packet>>;

/// Batch is essentially a collection of Packet
#[derive(Debug, Default)]
pub struct Batch {
    packets: HashMap<usize, Rc<Packet>>, // batch owns packet strongly
}

/// Packet has week ref to its owner
#[derive(Debug, Default)]
pub struct Packet {
    priority: u64,
    index: usize, // same usize used in HashMap key in batch
    owner: Weak<RefCell<Batch>>, // packet ref to batch weakly
}

/// MinMaxHeap needs Ord for Packet 
impl Ord for Packet {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl PartialOrd for Packet {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Packet {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for Packet {}

impl std::ops::Deref for Buffer {
    type Target = VecDeque<Rc<RefCell<Batch>>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Buffer {
    pub fn with_capacity(capacity: usize) -> Self {
        Buffer(VecDeque::with_capacity(capacity))
    }

    /// Pushing batch into buffer then drop excessive batches if needed;
    /// This sequence allows new batch being evaluated together with existing 
    /// batches when decide which one to drop, ensures all remaining packets
    /// are have equal or higher priority than those dropped.
    pub fn insert_batch(
        &mut self,
        index: &mut Index,
        batch_limit: usize,
        batch: Rc<RefCell<Batch>>,
    ) {
        if batch.borrow().packets.is_empty() {
            return;
        }

        self.push_back(batch);

        let num_batches_to_remove = self.len().saturating_sub(batch_limit);
        if num_batches_to_remove > 0 {
            self.remove_batches_by_priority(index, num_batches_to_remove);
        }

        // NOTE: push_back() plus remove() are more expensive than swap_remove_back()
        // However, VecDeque now hold `Rc` instead of `Batch` itself, it shouldn't too 
        // bad.
    }

    /// TODO this should be Batch's associate function.
    /// make_batch implements the inner relationship between batch <--> packets.
    pub fn make_batch(
        index: &mut Index,
        // raw inputs, would be PacketBatch in real life
        packet_per_batch_count: usize,
        random_priority: bool,
    ) -> Rc<RefCell<Batch>> {
        let mut rng = rand::thread_rng();
        let distribution = Uniform::from(0..200_000);

        let batch = Rc::new(RefCell::new(Batch::default()));
        (*batch.borrow_mut()).packets = 
            (0..packet_per_batch_count).map(|m| {
                let priority = if random_priority {
                    distribution.sample(&mut rng)
                } 
                else {
                    m as u64
                };
                let packet = Rc::new(Packet {
                    index: m, 
                    priority,
                    owner: Rc::downgrade(&batch.clone()),
                });
                // update index on insertion
                index.push(Rc::clone(&packet));
                (packet.index, packet)
            })
            .collect();
        batch
    }

    /// Utilizing existing priority packet index to efficiently drop low priority packets.
    /// Compare to other approach, at the time of drop batch, it does not need to do:
    /// 1. Scan and index buffer -- it is eagerly prepared at batch insertion;
    /// 2. Lookup batch to remove low priority packet from its unprocessed list.
    /// 3. Also added a option to drop multiple batches at a time to further improve efficiency.
    fn remove_batches_by_priority(
        &mut self, 
        index: &mut Index,
        num_batches_to_remove: usize,
    ) {
        let mut removed_batch_count = 0;
        while let Some(pkt) = index.pop_min() {
            debug!("popped min from index: {:?}",  pkt);

            // index yields ref to min priority packet, using packet.owner to reference to 
            // batch, then remove the packet from batch's unprocessed list
            let batch = pkt.owner.upgrade().unwrap();
            let _popped_packet = batch.borrow_mut().packets.remove(&pkt.index).unwrap();
            // be more efficient to remove multiple batches at one go
            if batch.borrow().packets.is_empty() {
                removed_batch_count += 1;
                if removed_batch_count >= num_batches_to_remove {
                    break;
                }
            }
        }
        // still need to iterate through VecDeque buffer to remove empty batches
        self.retain(|batch| {
            !batch.borrow().packets.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
    };

    #[test]
    fn test_priority_flat_index_make_batch() {
        solana_logger::setup();

        // create one index per buffer
        let mut index = Index::default();

        let num_packets = 10;
        // batch needs to be referenced by many of its packets, so need to be Rc<>
        // batch needs to be mutable after deref from packet, so Rc<RefCell<>>
        let batch = Buffer::make_batch(&mut index, num_packets, true);

        assert_eq!(num_packets, batch.borrow().packets.len());
        assert_eq!(num_packets, index.len());

        let mut expected_pkt_count = num_packets;
        // arbitrary order
        for pkt in index.iter() {
            debug!("checking {:?}", pkt);

            // assert getting owner from child
            let batch = pkt.owner.upgrade().unwrap();
            assert_eq!(expected_pkt_count, batch.borrow().packets.len());
            // assert parent/child relationship
            assert!(batch.borrow().packets.contains_key(&pkt.index));
            // assert can do mut op on owner
            {
                // directly remove packet from batch saves one batch [index] op, plus packet O(n)
                // lookup. 
                let popped_packet = batch.borrow_mut().packets.remove(&pkt.index).unwrap();
                assert_eq!(2, Rc::strong_count(&popped_packet));
            }
            assert_eq!(1, Rc::strong_count(&pkt));
            expected_pkt_count -= 1;
            assert_eq!(expected_pkt_count, batch.borrow().packets.len());
        }
        assert!(batch.borrow().packets.is_empty());
    }

    #[test]
    fn test_priority_flat_index_insert_batch() {
        solana_logger::setup();
        let buffer_capacity = 4;
        let batch_count = 7;
        let packet_per_batch_count = 3;

        // initialize buffer and index
        let mut buffer = Buffer::with_capacity(buffer_capacity);
        let mut index = Index::with_capacity(buffer_capacity * packet_per_batch_count);

        // build Batch from provided input data, update index, then insert batch to buffer;
        // if batch_count > buffer_capacity, low priority packets will be dropped until
        // batch(es) are removed.
        (0..batch_count).for_each(|_| {
            let batch = Buffer::make_batch(&mut index, packet_per_batch_count, false);
            buffer.insert_batch(
                &mut index,
                buffer_capacity,
                batch,
            );
        });

        // assert that buffer is full, has `buffer_capacity` packets in buffer and index.
        // The reason is since each batch as priority {0, 1, 2}, when the first batch is dropped, 
        // all `0` and `1` packets would have been dropped first.
        let expected_packets_count = buffer_capacity;
        assert_eq!(expected_packets_count, index.len());
        assert_eq!(buffer_capacity, buffer.len());
        let packet_count: usize = buffer.iter().map(|x| x.borrow().packets.len()).sum();
        assert_eq!(expected_packets_count, packet_count);

        // assert what's left in buffer are abiding the priority rule. Since batch in 
        // buffer has packet priority as (0, 1, 2), after buffer is saturated, only packets
        // left in buffer should be priority `2`.
        let expected_priority = 2;
        buffer.iter().for_each(|batch| {
            let packets = &batch.borrow().packets;
            assert_eq!(1, packets.len());
            assert!(packets.contains_key(&expected_priority));
        });
    }
}


