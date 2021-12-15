use crate::{BlockHeightAndTime, Transaction, TransactionDetails, TransactionType, TxFeeDetails};
use bitcrypto::sha256;
use common::mm_number::BigDecimal;
use keys::{Address, CashAddress};
use std::collections::HashSet;

pub trait DisplayAddress {
    fn display_address(&self) -> String;
}

impl DisplayAddress for Address {
    fn display_address(&self) -> String { self.to_string() }
}

impl DisplayAddress for CashAddress {
    fn display_address(&self) -> String { self.encode().expect("A valid cash address") }
}

pub struct Builder<'a, Addr: DisplayAddress, Tx: Transaction> {
    coin: String,
    tx: &'a Tx,
    my_addresses: HashSet<Addr>,
    total_amount: BigDecimal,
    received_by_me: BigDecimal,
    spent_by_me: BigDecimal,
    from_addresses: HashSet<Addr>,
    to_addresses: HashSet<Addr>,
    transaction_type: TransactionType,
    block_height_and_time: Option<BlockHeightAndTime>,
    tx_fee: Option<TxFeeDetails>,
}

impl<'a, Addr: Clone + DisplayAddress + Eq + std::hash::Hash, Tx: Transaction> Builder<'a, Addr, Tx> {
    pub fn new(
        coin: String,
        tx: &'a Tx,
        block_height_and_time: Option<BlockHeightAndTime>,
        my_addresses: impl IntoIterator<Item = Addr>,
    ) -> Self {
        Builder {
            coin,
            tx,
            my_addresses: my_addresses.into_iter().collect(),
            total_amount: Default::default(),
            received_by_me: Default::default(),
            spent_by_me: Default::default(),
            from_addresses: Default::default(),
            to_addresses: Default::default(),
            block_height_and_time,
            transaction_type: TransactionType::StandardTransfer,
            tx_fee: None,
        }
    }

    pub fn set_tx_fee(&mut self, tx_fee: Option<TxFeeDetails>) { self.tx_fee = tx_fee; }

    pub fn set_transaction_type(&mut self, tx_type: TransactionType) { self.transaction_type = tx_type; }

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

        let mut from: Vec<_> = self
            .from_addresses
            .iter()
            .map(DisplayAddress::display_address)
            .collect();
        from.sort();

        let mut to: Vec<_> = self.to_addresses.iter().map(DisplayAddress::display_address).collect();
        to.sort();

        let tx_hash = self.tx.tx_hash();
        let internal_id = match &self.transaction_type {
            TransactionType::TokenTransfer(token_id) => {
                let mut bytes_for_hash = tx_hash.0.clone();
                bytes_for_hash.extend_from_slice(&token_id.0);
                sha256(&bytes_for_hash).to_vec().into()
            },
            TransactionType::StakingDelegation
            | TransactionType::RemoveDelegation
            | TransactionType::StandardTransfer => tx_hash.clone(),
        };

        TransactionDetails {
            coin: self.coin,
            tx_hex: self.tx.tx_hex().into(),
            tx_hash,
            from,
            to,
            total_amount: self.total_amount,
            my_balance_change: &self.received_by_me - &self.spent_by_me,
            spent_by_me: self.spent_by_me,
            received_by_me: self.received_by_me,
            block_height,
            timestamp,
            fee_details: self.tx_fee,
            internal_id,
            kmd_rewards: None,
            transaction_type: self.transaction_type,
        }
    }
}
