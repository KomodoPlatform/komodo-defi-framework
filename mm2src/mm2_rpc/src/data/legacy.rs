#[path = "legacy/activation.rs"] mod activation;
#[path = "legacy/orders.rs"] mod orders;
#[path = "legacy/utility.rs"] mod utility;
#[path = "legacy/wallet.rs"] mod wallet;

use std::ops::Deref;

pub use activation::{eth::GasStationPricePolicy,
                     utxo::{ElectrumProtocol, UtxoMergeParams},
                     CoinInitResponse, EnabledCoin, GetEnabledResponse};
use common::serde_derive::{Deserialize, Serialize};
pub use orders::{AggregatedOrderbookEntry, MatchBy, OrderConfirmationsSettings, OrderType, OrderbookRequest,
                 OrderbookResponse, RpcOrderbookEntry, SellBuyRequest, SellBuyResponse, TakerAction,
                 TakerRequestForRpc};
pub use utility::{MmVersionResponse, Status};
pub use wallet::BalanceResponse;

#[derive(Serialize, Deserialize)]
pub struct Mm2RpcResult<T> {
    pub result: T,
}

impl<T> Mm2RpcResult<T> {
    pub fn new(result: T) -> Self { Self { result } }
}

impl<T> Deref for Mm2RpcResult<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target { &self.result }
}
