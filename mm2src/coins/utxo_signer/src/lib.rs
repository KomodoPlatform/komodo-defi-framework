use async_trait::async_trait;
use chain::Transaction as UtxoTx;
use common::mm_error::prelude::*;
use crypto::trezor::client::TrezorClient;
use crypto::trezor::utxo::TrezorUtxoCoin;
use crypto::trezor::TrezorError;
use derive_more::Display;
use keys::bytes::Bytes;
use keys::KeyPair;
use rpc::v1::types::{Transaction as RpcTransaction, H256 as H256Json};
use script::Script;

mod sign_common;
pub mod sign_params;
pub mod with_key_pair;
pub mod with_trezor;

use crate::with_key_pair::UtxoSignWithKeyPairError;
use sign_params::UtxoSignTxParams;

pub type UtxoSignTxResult<T> = Result<T, MmError<UtxoSignTxError>>;

type Signature = Bytes;

pub enum TxProviderError {
    Transport(String),
    InvalidResponse(String),
    Internal(String),
}

#[derive(Debug, Display)]
pub enum UtxoSignTxError {
    #[display(fmt = "Coin '{}' is not supported with Trezor", coin)]
    CoinNotSupportedWithTrezor { coin: String },
    #[display(fmt = "Trezor doesn't support P2WPKH outputs yet")]
    TrezorDoesntSupportP2WPKH,
    #[display(fmt = "Trezor client error: {}", _0)]
    TrezorError(TrezorError),
    #[display(fmt = "Encountered invalid parameter '{}': {}", param, description)]
    InvalidSignParam { param: String, description: String },
    #[display(
        fmt = "Hardware Device returned an invalid number of signatures: '{}', number of inputs: '{}'",
        actual,
        expected
    )]
    InvalidSignaturesNumber { actual: usize, expected: usize },
    #[display(fmt = "Error signing using a private key")]
    ErrorSigning(keys::Error),
    #[display(
        fmt = "{} script '{}' built from input key pair doesn't match expected prev script '{}'",
        script_type,
        script,
        prev_script
    )]
    MismatchScript {
        script_type: String,
        script: Script,
        prev_script: Script,
    },
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<TrezorError> for UtxoSignTxError {
    fn from(e: TrezorError) -> Self { UtxoSignTxError::TrezorError(e) }
}

impl From<UtxoSignWithKeyPairError> for UtxoSignTxError {
    fn from(error_with_key: UtxoSignWithKeyPairError) -> Self {
        let error = error_with_key.to_string();
        match error_with_key {
            UtxoSignWithKeyPairError::MismatchScript {
                script_type,
                script,
                prev_script,
            } => UtxoSignTxError::MismatchScript {
                script_type,
                script,
                prev_script,
            },
            // `with_key_pair` contains methods that checks parameters
            // that are expected to be checked by [`sign_common::UtxoSignTxParamsBuilder::build`] already.
            // So if this error happens, it's our internal error.
            UtxoSignWithKeyPairError::InputIndexOutOfBound { .. } => UtxoSignTxError::Internal(error),
            UtxoSignWithKeyPairError::ErrorSigning(sign) => UtxoSignTxError::ErrorSigning(sign),
        }
    }
}

impl From<keys::Error> for UtxoSignTxError {
    fn from(e: keys::Error) -> Self { UtxoSignTxError::ErrorSigning(e) }
}

impl From<TxProviderError> for UtxoSignTxError {
    fn from(e: TxProviderError) -> Self {
        match e {
            TxProviderError::Transport(transport) | TxProviderError::InvalidResponse(transport) => {
                UtxoSignTxError::Transport(transport)
            },
            TxProviderError::Internal(internal) => UtxoSignTxError::Internal(internal),
        }
    }
}

/// The trait declares a transaction getter.
/// The provider can use cache or RPC client.
#[async_trait]
pub trait TxProvider {
    async fn get_rpc_transaction(&self, tx_hash: &H256Json) -> Result<RpcTransaction, MmError<TxProviderError>>;
}

pub enum SignPolicy<'a> {
    WithTrezor(TrezorClient),
    WithKeyPair(&'a KeyPair),
}

#[async_trait]
pub trait UtxoSignerOps {
    type TxGetter: TxProvider + Send + Sync;

    fn trezor_coin(&self) -> UtxoSignTxResult<TrezorUtxoCoin>;

    fn fork_id(&self) -> u32;

    fn branch_id(&self) -> u32;

    fn tx_provider(&self) -> Self::TxGetter;

    async fn sign_tx(&self, params: UtxoSignTxParams, sign_policy: SignPolicy<'_>) -> UtxoSignTxResult<UtxoTx> {
        match sign_policy {
            SignPolicy::WithTrezor(trezor) => {
                let signer = with_trezor::TrezorTxSigner {
                    trezor,
                    tx_provider: self.tx_provider(),
                    trezor_coin: self.trezor_coin()?,
                    params,
                    fork_id: self.fork_id(),
                    branch_id: self.branch_id(),
                };
                signer.sign_tx().await
            },
            SignPolicy::WithKeyPair(key_pair) => {
                let signed = with_key_pair::sign_tx(
                    params.unsigned_tx,
                    key_pair,
                    params.prev_script,
                    params.signature_version,
                    self.fork_id(),
                )?;
                Ok(signed)
            },
        }
    }
}
