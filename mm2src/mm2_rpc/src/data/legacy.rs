#[path = "legacy/activation.rs"] mod activation;
#[path = "legacy/orders.rs"] mod orders;
#[path = "legacy/swaps.rs"] mod swaps;
#[path = "legacy/utility.rs"] mod utility;
#[path = "legacy/wallet.rs"] mod wallet;

pub use activation::{eth::GasStationPricePolicy,
                     utxo::{ElectrumProtocol, UtxoMergeParams},
                     CoinInitResponse, EnabledCoin, GetEnabledResponse, SetRequiredConfRequest, SetRequiredNotaRequest};
pub use orders::{AggregatedOrderbookEntry, BuyRequest, CancelAllOrdersRequest, CancelAllOrdersResponse, CancelBy,
                 CancelOrderRequest, FilteringOrder, HistoricalOrder, MakerConnectedForRpc, MakerMatchForRpc,
                 MakerOrderForMyOrdersRpc, MakerOrderForRpc, MakerReservedForRpc, MatchBy, MinTradingVolResponse,
                 MyOrdersRequest, MyOrdersResponse, OrderConfirmationsSettings, OrderForRpc, OrderStatusRequest,
                 OrderStatusResponse, OrderType, OrderbookDepthRequest, OrderbookRequest, OrderbookResponse,
                 OrdersHistoryRequest, OrdersHistoryResponse, PairDepth, PairWithDepth, RpcOrderbookEntry,
                 SellBuyRequest, SellBuyResponse, SellRequest, SetPriceRequest, TakerAction, TakerConnectForRpc,
                 TakerMatchForRpc, TakerOrderForRpc, TakerRequestForRpc, UpdateMakerOrderRequest, UuidParseError};
pub use swaps::{MySwapsFilter, RecoveredSwapAction};
pub use utility::{BanPubkeysRequest, MmVersionResponse, StopRequest, UnbanPubkeysRequest, VersionRequest};
pub use wallet::{BalanceRequest, BalanceResponse};

use derive_more::Display;
use std::ops::Deref;

use common::serde_derive::{Deserialize, Serialize};

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

#[derive(Serialize, Deserialize, Display)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Success,
}
