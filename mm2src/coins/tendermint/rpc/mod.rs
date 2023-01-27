#[cfg(not(target_arch = "wasm32"))] mod tendermint_native_rpc;
use mm2_number::BigDecimal;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use tendermint_native_rpc::*;

#[cfg(target_arch = "wasm32")] mod tendermint_wasm_rpc;
#[cfg(target_arch = "wasm32")]
pub(crate) use tendermint_wasm_rpc::*;

pub(crate) const TX_SUCCESS_CODE: u32 = 0;

#[repr(u8)]
pub enum TendermintResultOrder {
    Ascending = 1,
    Descending,
}

impl From<TendermintResultOrder> for Order {
    fn from(order: TendermintResultOrder) -> Self {
        match order {
            TendermintResultOrder::Ascending => Self::Ascending,
            TendermintResultOrder::Descending => Self::Descending,
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct IBCWithdrawRequest {
    pub(crate) ibc_source_channel: String,
    pub(crate) coin: String,
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) amount: BigDecimal,
    #[serde(default)]
    pub(crate) max: bool,
    pub(crate) memo: Option<String>,
}
