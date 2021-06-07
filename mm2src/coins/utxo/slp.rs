#![allow(dead_code)]
#![allow(unused_variables)]

use super::utxo_standard::UtxoStandardCoin;
use crate::utxo::utxo_common::big_decimal_from_sat_unsigned;
use crate::utxo::{UtxoCommonOps, UtxoTx};
use crate::{BalanceFut, CoinBalance, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin,
            NegotiateSwapContractAddrErr, SwapOps, TradeFee, TradePreimageFut, TradePreimageValue, TransactionEnum,
            TransactionFut, ValidateAddressResult, WithdrawFut, WithdrawRequest};
use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use common::mm_number::{BigDecimal, MmNumber};
use futures::compat::Future01CompatExt;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use primitives::hash::H256;
use rpc::v1::types::Bytes as BytesJson;
use script::{Opcode, Script};
use serde_json::Value as Json;
use serialization::{deserialize, Deserializable, Error, Reader};
use serialization_derive::Deserializable;
use std::convert::TryInto;

#[derive(Clone, Debug)]
struct SlpToken {
    decimals: u8,
    ticker: String,
    token_id: H256,
    platform_utxo: UtxoStandardCoin,
}

/// https://slp.dev/specs/slp-token-type-1/#transaction-detail
#[derive(Debug, Eq, PartialEq)]
enum SlpTransaction {
    /// https://slp.dev/specs/slp-token-type-1/#send-spend-transaction
    Genesis {
        token_ticker: String,
        token_name: String,
        token_document_url: String,
        token_document_hash: Vec<u8>,
        decimals: Vec<u8>,
        mint_baton_vout: Vec<u8>,
        initial_token_mint_quantity: Vec<u8>,
    },
    /// https://slp.dev/specs/slp-token-type-1/#send-spend-transaction
    Send { token_id: Vec<u8>, amount: Vec<u8> },
}

impl Deserializable for SlpTransaction {
    fn deserialize<T>(reader: &mut Reader<T>) -> Result<Self, Error>
    where
        Self: Sized,
        T: std::io::Read,
    {
        let transaction_type: String = reader.read()?;
        match transaction_type.as_str() {
            "GENESIS" => {
                let token_ticker = reader.read()?;
                let token_name = reader.read()?;
                let maybe_push_op_code: u8 = reader.read()?;
                let token_document_url = if maybe_push_op_code == Opcode::OP_PUSHDATA1 as u8 {
                    reader.read()?
                } else {
                    let mut url = vec![0; maybe_push_op_code as usize];
                    reader.read_slice(&mut url)?;
                    String::from_utf8(url).map_err(|e| Error::Custom(e.to_string()))?
                };

                let maybe_push_op_code: u8 = reader.read()?;
                let token_document_hash = if maybe_push_op_code == Opcode::OP_PUSHDATA1 as u8 {
                    reader.read_list()?
                } else {
                    let mut hash = vec![0; maybe_push_op_code as usize];
                    reader.read_slice(&mut hash)?;
                    hash
                };
                let decimals = reader.read_list()?;
                let maybe_push_op_code: u8 = reader.read()?;
                let mint_baton_vout = if maybe_push_op_code == Opcode::OP_PUSHDATA1 as u8 {
                    reader.read_list()?
                } else {
                    let mut baton = vec![0; maybe_push_op_code as usize];
                    reader.read_slice(&mut baton)?;
                    baton
                };
                let initial_token_mint_quantity = reader.read_list()?;

                Ok(SlpTransaction::Genesis {
                    token_ticker,
                    token_name,
                    token_document_url,
                    token_document_hash,
                    decimals,
                    mint_baton_vout,
                    initial_token_mint_quantity,
                })
            },
            "SEND" => Ok(SlpTransaction::Send {
                token_id: reader.read_list()?,
                amount: reader.read_list()?,
            }),
            _ => Err(Error::Custom(format!(
                "Unsupported transaction type {}",
                transaction_type
            ))),
        }
    }
}

#[derive(Deserializable)]
struct SlpTxDetails {
    op_code: u8,
    lokad_id: String,
    token_type: String,
    transaction: SlpTransaction,
}

#[derive(Debug)]
enum ParseSlpScriptError {
    NotOpReturn,
    NotSlp,
}

fn parse_slp_script(script: &[u8]) -> Result<SlpTxDetails, ParseSlpScriptError> {
    let details: SlpTxDetails = deserialize(script).unwrap();
    Ok(details)
}

impl MarketCoinOps for SlpToken {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> Result<String, String> { unimplemented!() }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            let (unspents, _) = coin
                .platform_utxo
                .list_unspent_ordered(&coin.platform_utxo.as_ref().my_address)
                .await?;
            let mut spendable = 0.into();
            for unspent in unspents {
                if unspent.value != coin.platform_utxo.as_ref().dust_amount {
                    continue;
                }
                let prev_tx_bytes = coin
                    .platform_utxo
                    .as_ref()
                    .rpc_client
                    .get_transaction_bytes(unspent.outpoint.hash.reversed().into())
                    .compat()
                    .await?;
                let prev_tx: UtxoTx = deserialize(prev_tx_bytes.0.as_slice()).unwrap();
                let script: Script = prev_tx.outputs[0].script_pubkey.clone().into();
                if let Ok(slp_data) = parse_slp_script(&script) {
                    match slp_data.transaction {
                        SlpTransaction::Send { token_id, amount } => {
                            if H256::from(token_id.as_slice()) == coin.token_id && amount.len() == 8 {
                                let satoshi = u64::from_be_bytes(amount.try_into().unwrap());
                                let decimal = big_decimal_from_sat_unsigned(satoshi, coin.decimals);
                                spendable += decimal;
                            }
                        },
                        SlpTransaction::Genesis {
                            initial_token_mint_quantity,
                            ..
                        } => {
                            if prev_tx.hash().reversed() == coin.token_id && initial_token_mint_quantity.len() == 8 {
                                let satoshi = u64::from_be_bytes(initial_token_mint_quantity.try_into().unwrap());
                                let decimal = big_decimal_from_sat_unsigned(satoshi, coin.decimals);
                                spendable += decimal;
                            }
                        },
                    }
                }
            }
            Ok(CoinBalance {
                spendable,
                unspendable: 0.into(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { unimplemented!() }

    /// Receives raw transaction bytes in hexadecimal format as input and returns tx hash in hexadecimal format
    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> { unimplemented!() }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn wait_for_tx_spend(
        &self,
        transaction: &[u8],
        wait_until: u64,
        from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> { unimplemented!() }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> { unimplemented!() }

    fn address_from_pubkey_str(&self, pubkey: &str) -> Result<String, String> { unimplemented!() }

    fn display_priv_key(&self) -> String { unimplemented!() }

    fn min_tx_amount(&self) -> BigDecimal { unimplemented!() }

    fn min_trading_vol(&self) -> MmNumber { MmNumber::from("0.00777") }
}

impl SwapOps for SlpToken {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal) -> TransactionFut { unimplemented!() }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        expected_sender: &[u8],
        fee_addr: &[u8],
        amount: &BigDecimal,
        min_block_number: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn validate_taker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        unimplemented!()
    }

    fn search_for_swap_tx_spend_my(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    fn search_for_swap_tx_spend_other(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> { unimplemented!() }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        Ok(None)
    }
}

impl MmCoin for SlpToken {
    fn is_asset_chain(&self) -> bool { unimplemented!() }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut { unimplemented!() }

    fn decimals(&self) -> u8 { unimplemented!() }

    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> { unimplemented!() }

    fn validate_address(&self, address: &str) -> ValidateAddressResult { unimplemented!() }

    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { unimplemented!() }

    fn history_sync_status(&self) -> HistorySyncState { unimplemented!() }

    /// Get fee to be paid per 1 swap transaction
    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { unimplemented!() }

    fn get_sender_trade_fee(&self, value: TradePreimageValue, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        unimplemented!()
    }

    fn get_receiver_trade_fee(&self, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { unimplemented!() }

    fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: BigDecimal,
        stage: FeeApproxStage,
    ) -> TradePreimageFut<TradeFee> {
        unimplemented!()
    }

    fn required_confirmations(&self) -> u64 { 1 }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, _confirmations: u64) { unimplemented!() }

    fn set_requires_notarization(&self, _requires_nota: bool) { unimplemented!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { unimplemented!() }

    fn mature_confirmations(&self) -> Option<u32> { unimplemented!() }
}

#[test]
fn test_parse_slp_script() {
    let script = hex::decode("6a04534c500001010453454e4420e73b2b28c14db8ebbf97749988b539508990e1708021067f206f49d55807dbf4080000000005f5e100").unwrap();
    let slp_data = parse_slp_script(&script).unwrap();
    assert_eq!(slp_data.lokad_id, "SLP\0");
    let expected_amount = 100000000u64.to_be_bytes().to_vec();
    let expected_transaction = SlpTransaction::Send {
        token_id: hex::decode("e73b2b28c14db8ebbf97749988b539508990e1708021067f206f49d55807dbf4").unwrap(),
        amount: expected_amount,
    };

    assert_eq!(expected_transaction, slp_data.transaction);

    let script =
        hex::decode("6a04534c500001010747454e45534953044144455804414445584c004c0001084c0008000000174876e800").unwrap();
    let slp_data = parse_slp_script(&script).unwrap();
    assert_eq!(slp_data.lokad_id, "SLP\0");
    let initial_token_mint_quantity = 1000_0000_0000u64.to_be_bytes().to_vec();
    let expected_transaction = SlpTransaction::Genesis {
        token_ticker: "ADEX".to_string(),
        token_name: "ADEX".to_string(),
        token_document_url: "".to_string(),
        token_document_hash: vec![],
        decimals: vec![8],
        mint_baton_vout: vec![],
        initial_token_mint_quantity,
    };

    assert_eq!(expected_transaction, slp_data.transaction);

    let script =
        hex::decode("6a04534c500001010747454e45534953045553445423546574686572204c74642e20555320646f6c6c6172206261636b656420746f6b656e734168747470733a2f2f7465746865722e746f2f77702d636f6e74656e742f75706c6f6164732f323031362f30362f546574686572576869746550617065722e70646620db4451f11eda33950670aaf59e704da90117ff7057283b032cfaec77793139160108010208002386f26fc10000").unwrap();
    let slp_data = parse_slp_script(&script).unwrap();
    assert_eq!(slp_data.lokad_id, "SLP\0");
    let initial_token_mint_quantity = 10000000000000000u64.to_be_bytes().to_vec();
    let expected_transaction = SlpTransaction::Genesis {
        token_ticker: "USDT".to_string(),
        token_name: "Tether Ltd. US dollar backed tokens".to_string(),
        token_document_url: "https://tether.to/wp-content/uploads/2016/06/TetherWhitePaper.pdf".to_string(),
        token_document_hash: hex::decode("db4451f11eda33950670aaf59e704da90117ff7057283b032cfaec7779313916").unwrap(),
        decimals: vec![8],
        mint_baton_vout: vec![2],
        initial_token_mint_quantity,
    };

    assert_eq!(expected_transaction, slp_data.transaction);
}
