use crate::prelude::GetCurrentBlock;
use coins::coin_balance::EnableCoinBalance;
use serde_derive::Serialize;

#[derive(Clone, Serialize)]
pub struct UtxoStandardActivationResult {
    pub current_block: u64,
    pub wallet_balance: EnableCoinBalance,
}

impl GetCurrentBlock for UtxoStandardActivationResult {
    fn get_current_block(&self) -> u64 { self.current_block }
}
