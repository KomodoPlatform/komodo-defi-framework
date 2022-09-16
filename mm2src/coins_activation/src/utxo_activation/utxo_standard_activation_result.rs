use crate::prelude::{CurrentBlock, GetAddressesBalances};
use coins::coin_balance::CoinBalanceReport;
use mm2_number::BigDecimal;
use serde_derive::Serialize;
use std::collections::HashMap;

#[derive(Clone, Serialize)]
pub struct UtxoStandardActivationResult {
    pub ticker: String,
    pub current_block: u64,
    pub wallet_balance: CoinBalanceReport,
}

impl CurrentBlock for UtxoStandardActivationResult {
    fn current_block(&self) -> u64 { self.current_block }
}

impl GetAddressesBalances for UtxoStandardActivationResult {
    fn get_addresses_balances(&self) -> HashMap<String, BigDecimal> {
        self.wallet_balance.to_addresses_total_balances()
    }
}
