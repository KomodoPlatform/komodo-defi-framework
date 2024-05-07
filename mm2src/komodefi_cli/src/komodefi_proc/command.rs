use anyhow::{anyhow, Result};
use derive_more::Display;
use serde::Serialize;

use common::log::error;
use mm2_rpc::data::version2::{MmRpcRequest, MmRpcVersion};

use crate::error_anyhow;

#[derive(Clone, Serialize)]
pub(crate) struct Command<T>
where
    T: Serialize + Sized,
{
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub(crate) flatten_data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) userpass: Option<String>,
}

#[derive(Clone, Display, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum V2Method {
    BestOrders,
    TradePreimage,
    Withdraw,
    GetPublicKey,
    GetPublicKeyHash,
    GetRawTransaction,
    EnableBchWithTokens,
    EnableSlp,
    EnableTendermintWithAssets,
    EnableTendermintToken,
    EnableErc20,
    EnableEthWithTokens,
    #[serde(rename = "task::enable_z_coin::init")]
    EnableZCoin,
    #[serde(rename = "task::enable_z_coin::status")]
    EnableZCoinStatus,
    #[serde(rename = "task::enable_z_coin::cancel")]
    EnableZCoinCancel,
    AddNodeToVersionStat,
    RemoveNodeFromVersionStat,
    StartVersionStatCollection,
    StopVersionStatCollection,
    UpdateVersionStatCollection,
    SignMessage,
    VerifyMessage,
    GetCurrentMtp,
    MyTxHistory,
    ZCoinTxHistory,
}

impl<T> Command<T>
where
    T: Serialize + Sized,
{
    pub(super) fn builder() -> CommandBuilder<T> { CommandBuilder::new() }
}

pub(super) struct CommandBuilder<T> {
    userpass: Option<String>,
    method: Option<V2Method>,
    flatten_data: Option<T>,
}

impl<T> CommandBuilder<T>
where
    T: Serialize,
{
    fn new() -> Self {
        CommandBuilder {
            userpass: None,
            method: None,
            flatten_data: None,
        }
    }

    pub(super) fn userpass(&mut self, userpass: String) -> &mut Self {
        self.userpass = Some(userpass);
        self
    }

    pub(super) fn v2_method(&mut self, method: V2Method) -> &mut Self {
        self.method = Some(method);
        self
    }

    pub(super) fn flatten_data(&mut self, flatten_data: T) -> &mut Self {
        self.flatten_data = Some(flatten_data);
        self
    }

    pub(super) fn build(&mut self) -> Result<Command<T>> {
        Ok(Command {
            userpass: self.userpass.take(),
            flatten_data: self.flatten_data.take(),
        })
    }

    pub(crate) fn build_v2(&mut self) -> Result<MmRpcRequest<V2Method, Option<T>>> {
        let mm2_rpc_request = MmRpcRequest {
            mmrpc: MmRpcVersion::V2,
            userpass: Some(
                self.userpass
                    .take()
                    .ok_or_else(|| error_anyhow!("Failed to build v2 request, password is not set"))?,
            ),
            method: self
                .method
                .take()
                .ok_or_else(|| error_anyhow!("Failed to build v2 request, method is not set"))?,
            params: self.flatten_data.take(),
            id: None,
        };

        Ok(mm2_rpc_request)
    }
}
