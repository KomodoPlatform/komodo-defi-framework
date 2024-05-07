use crate::proto::{messages_ethereum as proto_ethereum, messages_ethereum_definitions as proto_ethereum_definitions};
use crate::response_processor::ProcessTrezorResponse;
use crate::result_handler::ResultHandler;
use crate::{serialize_derivation_path, OperationFailure, TrezorError, TrezorResponse, TrezorResult, TrezorSession};
use ethcore_transaction::{signature, Action, Transaction as UnSignedEthTx, UnverifiedTransaction as UnverifiedEthTx};
use ethereum_types::H256;
use ethkey::Signature;
use hw_common::primitives::{DerivationPath, XPub};
use lazy_static::lazy_static;
use mm2_err_handle::map_mm_error::MapMmError;
use mm2_err_handle::or_mm_error::OrMmError;
use mm2_err_handle::prelude::MmError;
use std::collections::BTreeMap;

type ChainId = u64;
type StaticDefinitionBytes = &'static [u8];
type StaticAddressBytes = &'static [u8];

// new supported eth networks:
const SEPOLIA_ID: u64 = 11155111;

lazy_static! {

    // External eth network definitions
    static ref ETH_NETWORK_DEFS: BTreeMap<ChainId, StaticDefinitionBytes> = [
        (SEPOLIA_ID, SEPOLIA_NETWORK_DEF.as_ref())
    ].iter().cloned().collect();

    // External eth token definitions
    static ref ETH_TOKEN_DEFS: BTreeMap<StaticAddressBytes, (ChainId, StaticDefinitionBytes)> = [
    ].iter().cloned().collect();

    static ref SEPOLIA_NETWORK_DEF: Vec<u8> = include_bytes!("definitions/sepolia.dat").to_vec();
    // add more files with external network or token definitions
}

/// Get external network definition by chain id
/// check this doc how to find network definition files https://docs.trezor.io/trezor-firmware/common/ethereum-definitions.html
fn get_eth_network_def(chain_id: ChainId) -> Option<Vec<u8>> {
    ETH_NETWORK_DEFS
        .iter()
        .find(|(id, _def)| id == &&chain_id)
        .map(|found| found.1.to_vec())
}

/// Get external token definition by token contract address and chain id
/// check this doc how to find token definition files https://docs.trezor.io/trezor-firmware/common/ethereum-definitions.html
#[allow(dead_code)]
fn get_eth_token_def(address_bytes: &[u8], chain_id: ChainId) -> Option<Vec<u8>> {
    ETH_TOKEN_DEFS
        .iter()
        .find(|(address, def)| address == &&address_bytes && def.0 == chain_id)
        .map(|found| found.1 .1.to_vec())
}

/// trim leading zeros in array
macro_rules! trim_left {
    ($param:expr) => {{
        $param.iter().skip_while(|el| el == &&0).cloned().collect::<Vec<_>>()
    }};
}

impl<'a> TrezorSession<'a> {
    /// Retrieves the EVM address associated with a given derivation path from the Trezor device.
    pub async fn get_eth_address<'b>(
        &'b mut self,
        derivation_path: DerivationPath,
        show_display: bool,
    ) -> TrezorResult<TrezorResponse<'a, 'b, Option<String>>> {
        let req = proto_ethereum::EthereumGetAddress {
            address_n: derivation_path.iter().map(|child| child.0).collect(),
            show_display: Some(show_display),
            encoded_network: None,
            chunkify: None,
        };
        let result_handler = ResultHandler::new(|m: proto_ethereum::EthereumAddress| Ok(m.address));
        self.call(req, result_handler).await
    }

    /// Retrieves the EVM public key associated with a given derivation path from the Trezor device.
    pub async fn get_eth_public_key<'b>(
        &'b mut self,
        derivation_path: &DerivationPath,
        show_display: bool,
    ) -> TrezorResult<TrezorResponse<'a, 'b, XPub>> {
        let req = proto_ethereum::EthereumGetPublicKey {
            address_n: serialize_derivation_path(derivation_path),
            show_display: Some(show_display),
        };
        let result_handler = ResultHandler::new(|m: proto_ethereum::EthereumPublicKey| Ok(m.xpub));
        self.call(req, result_handler).await
    }

    /// Signs a transaction for any EVM-based chain using the Trezor device.
    pub async fn sign_eth_tx(
        &mut self,
        derivation_path: &DerivationPath,
        unsigned_tx: &UnSignedEthTx,
        chain_id: u64,
    ) -> TrezorResult<UnverifiedEthTx> {
        let mut data: Vec<u8> = vec![];
        let req = to_sign_eth_message(unsigned_tx, derivation_path, chain_id, &mut data);
        let processor = self
            .processor
            .as_ref()
            .or_mm_err(|| TrezorError::InternalNoProcessor)?
            .clone();
        let mut tx_request = self
            .send_sign_eth_tx(req)
            .await?
            .process(processor.clone())
            .await
            .mm_err(|e| TrezorError::Internal(e.to_string()))?;

        while let Some(data_length) = tx_request.data_length {
            if data_length > 0 {
                let req = proto_ethereum::EthereumTxAck {
                    data_chunk: data.splice(..data_length as usize, []).collect(),
                };
                tx_request = self
                    .send_eth_tx_ack(req)
                    .await?
                    .process(processor.clone())
                    .await
                    .mm_err(|e| TrezorError::Internal(e.to_string()))?;
            } else {
                break;
            }
        }

        let sig = extract_eth_signature(&tx_request)?;
        Ok(unsigned_tx.clone().with_signature(sig, Some(chain_id)))
    }

    async fn send_sign_eth_tx<'b>(
        &'b mut self,
        req: proto_ethereum::EthereumSignTx,
    ) -> TrezorResult<TrezorResponse<'a, 'b, proto_ethereum::EthereumTxRequest>> {
        let result_handler = ResultHandler::<proto_ethereum::EthereumTxRequest>::new(Ok);
        self.call(req, result_handler).await
    }

    async fn send_eth_tx_ack<'b>(
        &'b mut self,
        req: proto_ethereum::EthereumTxAck,
    ) -> TrezorResult<TrezorResponse<'a, 'b, proto_ethereum::EthereumTxRequest>> {
        let result_handler = ResultHandler::<proto_ethereum::EthereumTxRequest>::new(Ok);
        self.call(req, result_handler).await
    }
}

fn to_sign_eth_message(
    unsigned_tx: &UnSignedEthTx,
    derivation_path: &DerivationPath,
    chain_id: u64,
    data: &mut Vec<u8>,
) -> proto_ethereum::EthereumSignTx {
    // if we have it, pass network or token definition info to show on the device screen:
    let eth_defs = proto_ethereum_definitions::EthereumDefinitions {
        encoded_network: get_eth_network_def(chain_id),
        encoded_token: None, // TODO add looking for tokens defs
    };

    let mut nonce: [u8; 32] = [0; 32];
    let mut gas_price: [u8; 32] = [0; 32];
    let mut gas_limit: [u8; 32] = [0; 32];
    let mut value: [u8; 32] = [0; 32];

    unsigned_tx.nonce.to_big_endian(&mut nonce);
    unsigned_tx.gas_price.to_big_endian(&mut gas_price);
    unsigned_tx.gas.to_big_endian(&mut gas_limit);
    unsigned_tx.value.to_big_endian(&mut value);

    let addr_hex = if let Action::Call(addr) = unsigned_tx.action {
        Some(format!("{:X}", addr)) // Trezor works okay with both '0x' prefixed and non-prefixed addresses in hex
    } else {
        None
    };
    *data = unsigned_tx.data.clone();
    let data_length = if data.is_empty() { None } else { Some(data.len() as u32) };
    proto_ethereum::EthereumSignTx {
        address_n: serialize_derivation_path(derivation_path),
        nonce: Some(trim_left!(nonce)),
        gas_price: trim_left!(gas_price),
        gas_limit: trim_left!(gas_limit),
        to: addr_hex,
        value: Some(trim_left!(value)),
        data_initial_chunk: Some(data.splice(..std::cmp::min(1024, data.len()), []).collect()),
        data_length,
        chain_id,
        tx_type: None,
        definitions: Some(eth_defs),
        chunkify: if data.is_empty() { None } else { Some(true) },
    }
}

fn extract_eth_signature(tx_request: &proto_ethereum::EthereumTxRequest) -> TrezorResult<Signature> {
    match (
        tx_request.signature_r.as_ref(),
        tx_request.signature_s.as_ref(),
        tx_request.signature_v,
    ) {
        (Some(r), Some(s), Some(v)) => {
            let v_refined = signature::check_replay_protection(v as u64); // remove replay protection added by trezor as the ethcore lib will add it itself
            if v_refined == 4 {
                return Err(MmError::new(TrezorError::Failure(OperationFailure::InvalidSignature)));
            }
            Ok(Signature::from_rsv(
                &H256::from_slice(r.as_slice()),
                &H256::from_slice(s.as_slice()),
                v_refined,
            ))
        },
        (_, _, _) => Err(MmError::new(TrezorError::Failure(OperationFailure::InvalidSignature))),
    }
}
