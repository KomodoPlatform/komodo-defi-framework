use crate::{BlockHeightAndTime, Transaction, TransactionDetails, TransactionType};
use common::mm_number::BigDecimal;
use std::collections::HashSet;

pub struct Builder<'a, Addr, Tx: Transaction> {
    tx: &'a Tx,
    my_addresses: HashSet<Addr>,
    total_amount: BigDecimal,
    received_by_me: BigDecimal,
    spent_by_me: BigDecimal,
    from_addresses: HashSet<Addr>,
    to_addresses: HashSet<Addr>,
    transaction_type: TransactionType,
    block_height_and_time: Option<BlockHeightAndTime>,
}

impl<'a, Addr: Clone + Eq + std::hash::Hash, Tx: Transaction> Builder<'a, Addr, Tx> {
    pub fn new(tx: &'a Tx, block_height_and_time: Option<BlockHeightAndTime>) -> Self {
        Builder {
            tx,
            my_addresses: Default::default(),
            total_amount: Default::default(),
            received_by_me: Default::default(),
            spent_by_me: Default::default(),
            from_addresses: Default::default(),
            to_addresses: Default::default(),
            block_height_and_time,
            transaction_type: TransactionType::StandardTransfer,
        }
    }

    pub fn transferred_to(&mut self, address: Addr, amount: &BigDecimal) {
        if self.my_addresses.contains(&address) {
            self.received_by_me += amount;
        }
        self.to_addresses.insert(address);
    }

    pub fn transferred_from(&mut self, address: Addr, amount: &BigDecimal) {
        if self.my_addresses.contains(&address) {
            self.spent_by_me += amount;
        }
        self.total_amount += amount;
        self.from_addresses.insert(address);
    }

    pub fn build(self) -> TransactionDetails {
        let (block_height, timestamp) = match self.block_height_and_time {
            Some(height_with_time) => (height_with_time.height, height_with_time.timestamp),
            None => (0, 0),
        };

        TransactionDetails {
            tx_hex: self.tx.tx_hex().into(),
            tx_hash: self.tx.tx_hash(),
            from: vec![],
            to: vec![],
            total_amount: self.total_amount,
            my_balance_change: &self.received_by_me - &self.spent_by_me,
            spent_by_me: self.spent_by_me,
            received_by_me: self.received_by_me,
            block_height,
            timestamp,
            fee_details: None,
            coin: "".to_string(),
            internal_id: Default::default(),
            kmd_rewards: None,
            transaction_type: self.transaction_type,
        }
    }
}
