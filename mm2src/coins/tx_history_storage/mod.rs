use crate::my_tx_history_v2::{HistoryCoinType, TxHistoryStorage};
use crate::TransactionType;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use derive_more::Display;
use num_traits::Zero;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[cfg(target_arch = "wasm32")] pub mod wasm;

#[cfg(not(target_arch = "wasm32"))]
pub mod sql_tx_history_storage_v2;

#[cfg(any(test, target_arch = "wasm32"))]
mod tx_history_v2_tests;

pub fn token_id_from_tx_type(tx_type: &TransactionType) -> String {
    match tx_type {
        TransactionType::TokenTransfer(token_id) => format!("{:02x}", token_id),
        _ => String::new(),
    }
}

#[derive(Debug, Display)]
pub enum CreateTxHistoryStorageError {
    Internal(String),
}

/// `TxHistoryStorageBuilder` is used to create an instance that implements the `TxHistoryStorage` trait.
pub struct TxHistoryStorageBuilder<'a> {
    ctx: &'a MmArc,
}

impl<'a> TxHistoryStorageBuilder<'a> {
    pub fn new(ctx: &MmArc) -> TxHistoryStorageBuilder<'_> { TxHistoryStorageBuilder { ctx } }

    pub fn build(self) -> MmResult<impl TxHistoryStorage, CreateTxHistoryStorageError> {
        #[cfg(target_arch = "wasm32")]
        return wasm::IndexedDbTxHistoryStorage::new(self.ctx);
        #[cfg(not(target_arch = "wasm32"))]
        sql_tx_history_storage_v2::SqliteTxHistoryStorage::new(self.ctx)
    }
}

pub struct CoinTokenId {
    pub coin: String,
    pub token_id: String,
}

impl CoinTokenId {
    pub fn from_history_coin_type(coin_type: HistoryCoinType) -> CoinTokenId {
        match coin_type {
            HistoryCoinType::Coin(coin) => CoinTokenId {
                coin,
                token_id: String::new(),
            },
            HistoryCoinType::Token { platform, token_id } => CoinTokenId {
                coin: platform,
                token_id: format!("{:02x}", token_id),
            },
            HistoryCoinType::L2 { .. } => unimplemented!("Not implemented yet for HistoryCoinType::L2"),
        }
    }
}

/// Whether transaction is unconfirmed or confirmed.
/// Serializes to either `0u8` or `1u8` correspondingly.
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum ConfirmationStatus {
    Unconfirmed = 0,
    Confirmed = 1,
}

impl ConfirmationStatus {
    pub fn from_block_height<Height: Zero>(height: Height) -> ConfirmationStatus {
        if height.is_zero() {
            ConfirmationStatus::Unconfirmed
        } else {
            ConfirmationStatus::Confirmed
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn to_sql_param(self) -> String { (self as u8).to_string() }
}

impl Serialize for ConfirmationStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        (*self as u8).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ConfirmationStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let code = u8::deserialize(deserializer)?;
        match code {
            0 => Ok(ConfirmationStatus::Unconfirmed),
            1 => Ok(ConfirmationStatus::Confirmed),
            unknown => Err(D::Error::custom(format!(
                "Expected either '0' or '1' confirmation status, found '{}'",
                unknown
            ))),
        }
    }
}
