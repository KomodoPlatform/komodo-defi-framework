use std::collections::HashMap;

use mm2_number::BigRational;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BestOrdersAction {
    Buy,
    Sell,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OrdermatchRequest {
    /// Get an orderbook for the given pair.
    GetOrderbook {
        base: String,
        rel: String,
    },
    /// Sync specific pubkey orderbook state if our known Patricia trie state doesn't match the latest keep alive message
    SyncPubkeyOrderbookState {
        pubkey: String,
        /// Request using this condition
        /// trie_roots: HashMap<AlbOrderedOrderbookPair, H64>,
        /// TODO: use FxHashMap
        trie_roots: HashMap<String, [u8; 8]>,
    },
    BestOrders {
        coin: String,
        action: BestOrdersAction,
        volume: BigRational,
    },
    OrderbookDepth {
        pairs: Vec<(String, String)>,
    },
    BestOrdersByNumber {
        coin: String,
        action: BestOrdersAction,
        number: usize,
    },
}
