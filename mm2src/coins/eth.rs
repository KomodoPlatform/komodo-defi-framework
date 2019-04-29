/******************************************************************************
 * Copyright © 2014-2019 The SuperNET Developers.                             *
 *                                                                            *
 * See the AUTHORS, DEVELOPER-AGREEMENT and LICENSE files at                  *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * SuperNET software, including this file may be copied, modified, propagated *
 * or distributed except according to the terms contained in the LICENSE file *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  eth.rs
//  marketmaker
//
//  Copyright © 2017-2019 SuperNET. All rights reserved.
//
use bitcrypto::sha256;
use common::{lp, MutexGuardWrapper, slurp_url};
use secp256k1::key::PublicKey;
use ethabi::{Contract, Token};
use ethcore_transaction::{ Action, Transaction as UnSignedEthTx, UnverifiedTransaction};
use ethereum_types::{Address, U256, H160};
use ethkey::{ KeyPair, Public, public_to_address, SECP256K1 };
use futures::Future;
use futures::future::{loop_fn, Loop};
use futures_timer::Delay;
use gstuff::now_ms;
use hashbrown::HashMap;
use hyper::StatusCode;
use rand::{Rng, thread_rng};
use rand::seq::SliceRandom;
use rpc::v1::types::{Bytes as BytesJson};
use serde_json::{self as json, Value as Json};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::ffi::CStr;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use web3::{ self, Web3 };
use web3::types::{Action as TraceAction, BlockId, BlockNumber, Bytes, CallRequest, FilterBuilder, Log, Transaction as Web3Transaction, TransactionId, H256, Trace, TraceFilterBuilder};

use super::{IguanaInfo, MarketCoinOps, MmCoin, SwapOps, TransactionFut, TransactionEnum, Transaction, TransactionDetails};

pub use ethcore_transaction::SignedTransaction as SignedEthTx;

mod web3_transport;
use self::web3_transport::Web3Transport;
use futures::future::Either;
use common::mm_ctx::MmArc;

/// https://github.com/artemii235/etomic-swap/blob/master/contracts/EtomicSwap.sol
/// Dev chain (195.201.0.6:8565) contract address: 0xa09ad3cd7e96586ebd05a2607ee56b56fb2db8fd
/// Ropsten: https://ropsten.etherscan.io/address/0x7bc1bbdd6a0a722fc9bffc49c921b685ecb84b94
/// ETH mainnet: https://etherscan.io/address/0x8500AFc0bc5214728082163326C2FF0C73f4a871
const SWAP_CONTRACT_ABI: &'static str = r#"[{"constant":false,"inputs":[{"name":"_id","type":"bytes32"},{"name":"_amount","type":"uint256"},{"name":"_secret","type":"bytes32"},{"name":"_tokenAddress","type":"address"},{"name":"_sender","type":"address"}],"name":"receiverSpend","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"","type":"bytes32"}],"name":"payments","outputs":[{"name":"paymentHash","type":"bytes20"},{"name":"lockTime","type":"uint64"},{"name":"state","type":"uint8"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_id","type":"bytes32"},{"name":"_receiver","type":"address"},{"name":"_secretHash","type":"bytes20"},{"name":"_lockTime","type":"uint64"}],"name":"ethPayment","outputs":[],"payable":true,"stateMutability":"payable","type":"function"},{"constant":false,"inputs":[{"name":"_id","type":"bytes32"},{"name":"_amount","type":"uint256"},{"name":"_paymentHash","type":"bytes20"},{"name":"_tokenAddress","type":"address"},{"name":"_receiver","type":"address"}],"name":"senderRefund","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"_id","type":"bytes32"},{"name":"_amount","type":"uint256"},{"name":"_tokenAddress","type":"address"},{"name":"_receiver","type":"address"},{"name":"_secretHash","type":"bytes20"},{"name":"_lockTime","type":"uint64"}],"name":"erc20Payment","outputs":[],"payable":true,"stateMutability":"payable","type":"function"},{"inputs":[],"payable":false,"stateMutability":"nonpayable","type":"constructor"},{"anonymous":false,"inputs":[{"indexed":false,"name":"id","type":"bytes32"}],"name":"PaymentSent","type":"event"},{"anonymous":false,"inputs":[{"indexed":false,"name":"id","type":"bytes32"},{"indexed":false,"name":"secret","type":"bytes32"}],"name":"ReceiverSpent","type":"event"},{"anonymous":false,"inputs":[{"indexed":false,"name":"id","type":"bytes32"}],"name":"SenderRefunded","type":"event"}]"#;
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-20.md
const ERC20_ABI: &'static str = r#"[{"constant":true,"inputs":[],"name":"name","outputs":[{"name":"","type":"string"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_spender","type":"address"},{"name":"_value","type":"uint256"}],"name":"approve","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[],"name":"totalSupply","outputs":[{"name":"","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_from","type":"address"},{"name":"_to","type":"address"},{"name":"_value","type":"uint256"}],"name":"transferFrom","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[],"name":"decimals","outputs":[{"name":"","type":"uint8"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_spender","type":"address"},{"name":"_subtractedValue","type":"uint256"}],"name":"decreaseApproval","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"_owner","type":"address"}],"name":"balanceOf","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[],"name":"symbol","outputs":[{"name":"","type":"string"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_to","type":"address"},{"name":"_value","type":"uint256"}],"name":"transfer","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"_spender","type":"address"},{"name":"_addedValue","type":"uint256"}],"name":"increaseApproval","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"_owner","type":"address"},{"name":"_spender","type":"address"}],"name":"allowance","outputs":[{"name":"","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"inputs":[],"payable":false,"stateMutability":"nonpayable","type":"constructor"},{"anonymous":false,"inputs":[{"indexed":true,"name":"owner","type":"address"},{"indexed":true,"name":"spender","type":"address"},{"indexed":false,"name":"value","type":"uint256"}],"name":"Approval","type":"event"},{"anonymous":false,"inputs":[{"indexed":true,"name":"from","type":"address"},{"indexed":true,"name":"to","type":"address"},{"indexed":false,"name":"value","type":"uint256"}],"name":"Transfer","type":"event"}]"#;

/// Payment states from etomic swap smart contract: https://github.com/artemii235/etomic-swap/blob/master/contracts/EtomicSwap.sol#L5
const PAYMENT_STATE_UNINITIALIZED: u8 = 0;
const PAYMENT_STATE_SENT: u8 = 1;
const PAYMENT_STATE_SPENT: u8 = 2;
const PAYMENT_STATE_REFUNDED: u8 = 3;

lazy_static! {
    static ref SWAP_CONTRACT: Contract = unwrap!(Contract::load(SWAP_CONTRACT_ABI.as_bytes()));

    static ref ERC20_CONTRACT: Contract = unwrap!(Contract::load(ERC20_ABI.as_bytes()));
}

#[derive(Debug, PartialEq, Eq)]
enum EthCoinType {
    /// Ethereum itself or it's forks: ETC/others
    Eth,
    /// ERC20 token with smart contract address
    /// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-20.md
    Erc20(Address),
}

#[derive(Debug)]
pub struct EthCoinImpl {  // pImpl idiom.
    ticker: String,
    coin_type: EthCoinType,
    key_pair: KeyPair,
    my_address: Address,
    swap_contract_address: Address,
    web3: Web3<Web3Transport>,
    decimals: u8,
    gas_station_url: Option<String>,
}

impl EthCoinImpl {
    /// Gets Transfer events from ERC20 smart contract `addr` between `from_block` and `to_block`
    fn erc20_transfer_events(
        &self,
        contract: Address,
        from_addr: Option<Address>,
        to_addr: Option<Address>,
        from_block: BlockNumber,
        to_block: BlockNumber,
        limit: Option<usize>
    ) -> Box<Future<Item=Vec<Log>, Error=String>> {
        let contract_event = try_fus!(ERC20_CONTRACT.event("Transfer"));
        let topic0 = Some(vec![contract_event.signature()]);
        let topic1 = from_addr.map(|addr| vec![addr.into()]);
        let topic2 = to_addr.map(|addr| vec![addr.into()]);
        let mut filter = FilterBuilder::default()
            .topics(topic0, topic1, topic2, None)
            .from_block(from_block)
            .to_block(to_block)
            .address(vec![contract]);

        if let Some(l) = limit {
            filter = filter.limit(l);
        }

        Box::new(self.web3.eth().logs(filter.build()).map_err(|e| ERRL!("{:?}", e)))
    }

    /// Gets ETH traces between addresses in `from_block` and `to_block`
    fn eth_traces(
        &self,
        from_addr: Vec<Address>,
        to_addr: Vec<Address>,
        from_block: BlockNumber,
        to_block: BlockNumber,
        limit: Option<usize>
    ) -> Box<Future<Item=Vec<Trace>, Error=String>> {
        let mut filter = TraceFilterBuilder::default()
            .from_address(from_addr)
            .to_address(to_addr)
            .from_block(from_block)
            .to_block(to_block);

        if let Some(l) = limit {
            filter = filter.count(l);
        }

        Box::new(self.web3.trace().filter(filter.build()).map_err(|e| ERRL!("{:?}", e)))
    }
}

#[derive(Clone, Debug)]
pub struct EthCoin(Arc<EthCoinImpl>);
impl Deref for EthCoin {type Target = EthCoinImpl; fn deref (&self) -> &EthCoinImpl {&*self.0}}

impl SwapOps for EthCoin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: u64) -> TransactionFut {
        let address = try_fus!(addr_from_raw_pubkey(fee_addr));

        Box::new(self.send_to_address(
            address,
            u256_denominate_from_satoshis(amount, self.decimals),
        ).map(TransactionEnum::from))
    }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: u64
    ) -> TransactionFut {
        let taker_addr = try_fus!(addr_from_raw_pubkey(taker_pub));

        Box::new(self.send_hash_time_locked_payment(
            u256_denominate_from_satoshis(amount, self.decimals),
            time_lock,
            priv_bn_hash,
            taker_addr,
        ).map(TransactionEnum::from))
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: u64,
    ) -> TransactionFut {
        let maker_addr = try_fus!(addr_from_raw_pubkey(maker_pub));

        Box::new(self.send_hash_time_locked_payment(
            u256_denominate_from_satoshis(amount, self.decimals),
            time_lock,
            priv_bn_hash,
            maker_addr,
        ).map(TransactionEnum::from))
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_fus!(rlp::decode(taker_payment_tx));
        let signed = try_fus!(SignedEthTx::new(tx));

        Box::new(self.spend_hash_time_locked_payment(signed, secret).map(TransactionEnum::from))
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_fus!(rlp::decode(maker_payment_tx));
        let signed = try_fus!(SignedEthTx::new(tx));
        Box::new(self.spend_hash_time_locked_payment(signed, secret).map(TransactionEnum::from))
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_fus!(rlp::decode(taker_payment_tx));
        let signed = try_fus!(SignedEthTx::new(tx));

        Box::new(self.refund_hash_time_locked_payment(signed).map(TransactionEnum::from))
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        _secret_hash: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_fus!(rlp::decode(maker_payment_tx));
        let signed = try_fus!(SignedEthTx::new(tx));

        Box::new(self.refund_hash_time_locked_payment(signed).map(TransactionEnum::from))
    }

    fn validate_fee(
        &self,
        fee_tx: TransactionEnum,
        fee_addr: &[u8],
        amount: u64
    ) -> Result<(), String> {
        let tx = match fee_tx {
            TransactionEnum::SignedEthTx(t) => t,
            _ => panic!(),
        };

        let expected_value = u256_denominate_from_satoshis(amount, self.decimals);
        let fee_addr = try_s!(addr_from_raw_pubkey(fee_addr));
        let tx_from_rpc = try_s!(self.web3.eth().transaction(TransactionId::Hash(tx.hash)).wait());
        let tx_from_rpc = match tx_from_rpc {
            Some(t) => t,
            None => return ERR!("Didn't find provided tx {:?} on ETH node", tx),
        };

        match self.coin_type {
            EthCoinType::Eth => {
                if tx_from_rpc.to != Some(fee_addr) {
                    return ERR!("Fee tx {:?} was sent to wrong address, expected {:?}", tx_from_rpc, fee_addr);
                }

                if tx_from_rpc.value != expected_value {
                    return ERR!("Fee tx {:?} value is invalid, expected {:?}", tx_from_rpc, expected_value);
                }
            },
            EthCoinType::Erc20(token_addr) => {
                if tx_from_rpc.to != Some(token_addr) {
                    return ERR!("ERC20 Fee tx {:?} called wrong smart contract, expected {:?}", tx_from_rpc, token_addr);
                }

                let function = try_s!(ERC20_CONTRACT.function("transfer"));
                let expected_data = try_s!(function.encode_input(&[
                    Token::Address(fee_addr),
                    Token::Uint(expected_value),
                ]));

                if tx_from_rpc.input.0 != expected_data {
                    return ERR!("ERC20 Fee tx {:?} input is invalid, expected {:?}", tx_from_rpc, expected_data);
                }
            },
        }

        Ok(())
    }

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: u64,
    ) -> Result<(), String> {
        self.validate_payment(
            payment_tx,
            time_lock,
            maker_pub,
            secret_hash,
            amount,
        )
    }

    fn validate_taker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: u64,
    ) -> Result<(), String> {
        self.validate_payment(
            payment_tx,
            time_lock,
            taker_pub,
            secret_hash,
            amount,
        )
    }
}

impl MarketCoinOps for EthCoin {
    fn my_address(&self) -> Cow<str> {
        format!("{:#02x}", self.my_address).into()
    }

    fn my_balance(&self) -> Box<Future<Item=f64, Error=String> + Send> {
        let decimals = self.decimals;
        Box::new(self.my_balance().and_then(move |result| {
            Ok(try_s!(u256_to_f64(result, decimals)))
        }))
    }

    fn send_raw_tx(&self, mut tx: &str) -> Box<Future<Item=String, Error=String> + Send> {
        if tx.starts_with("0x") {
            tx = &tx[2..];
        }
        let bytes = try_fus!(hex::decode(tx));
        Box::new(
            self.web3.eth().send_raw_transaction(bytes.into())
                .map(|res| format!("{:02x}", res))
                .map_err(|e| ERRL!("{}", e))
        )
    }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u32,
        wait_until: u64,
    ) -> Result<(), String> {
        let unsigned: UnverifiedTransaction = try_s!(rlp::decode(tx));
        let tx = try_s!(SignedEthTx::new(unsigned));

        let required_confirms = U256::from(confirmations);
        loop {
            if now_ms() / 1000 > wait_until {
                return ERR!("Waited too long until {} for transaction {:?} confirmation ", wait_until, tx);
            }

            let web3_receipt = match self.web3.eth().transaction_receipt(tx.hash()).wait() {
                Ok(r) => r,
                Err(e) => {
                    log!("Error " [e] " getting the " (self.ticker()) " transaction " [tx.tx_hash()] ", retrying in 15 seconds");
                    thread::sleep(Duration::from_secs(15));
                    continue;
                }
            };
            if let Some(receipt) = web3_receipt {
                if receipt.status != Some(1.into()) {
                    return ERR!("Tx receipt {:?} status of {} tx {:?} is failed", receipt, self.ticker(), tx.tx_hash());
                }

                if let Some(confirmed_at) = receipt.block_number {
                    let current_block = match self.web3.eth().block_number().wait() {
                        Ok(b) => b,
                        Err(e) => {
                            log!("Error " [e] " getting the " (self.ticker()) " block number retrying in 15 seconds");
                            thread::sleep(Duration::from_secs(15));
                            continue;
                        }
                    };
                    if current_block - confirmed_at + 1 >= required_confirms {
                        return Ok(());
                    }
                }
            }
            thread::sleep(Duration::from_secs(15));
        }
    }

    fn wait_for_tx_spend(&self, tx_bytes: &[u8], wait_until: u64) -> Result<TransactionEnum, String> {
        let unverified: UnverifiedTransaction = try_s!(rlp::decode(tx_bytes));
        let tx = try_s!(SignedEthTx::new(unverified));

        let func_name = match self.coin_type {
            EthCoinType::Eth => "ethPayment",
            EthCoinType::Erc20(_token_addr) => "erc20Payment",
        };

        let payment_func = try_s!(SWAP_CONTRACT.function(func_name));
        let decoded = try_s!(payment_func.decode_input(&tx.data));
        let id = match &decoded[0] {
            Token::FixedBytes(bytes) => bytes.clone(),
            _ => panic!(),
        };

        loop {
            if now_ms() / 1000 > wait_until {
                return ERR!("Waited too long until {} for transaction {:?} to be spent ", wait_until, tx);
            }

            let events = try_s!(self.spend_events(0).wait());

            let found = events.iter().find(|event| &event.data.0[..32] == id.as_slice());

            if let Some(event) = found {
                if let Some(tx_hash) = event.transaction_hash {
                    let transaction = try_s!(self.web3.eth().transaction(TransactionId::Hash(tx_hash)).wait()).unwrap();

                    return Ok(TransactionEnum::from(try_s!(signed_tx_from_web3_tx(transaction))))
                }
            }
            thread::sleep(Duration::from_secs(15));
            continue;
        }
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> {
        let tx: UnverifiedTransaction = try_s!(rlp::decode(bytes));
        let signed = try_s!(SignedEthTx::new(tx));
        Ok(TransactionEnum::from(signed))
    }

    fn current_block(&self) -> Box<Future<Item=u64, Error=String> + Send> {
        Box::new(self.web3.eth().block_number().map(|res| res.into()).map_err(|e| ERRL!("{:?}", e)))
    }
}

// We can use a shared nonce lock for all ETH coins.
// It's highly likely that we won't experience any issues with it as we won't need to send "a lot" of transactions concurrently.
// For ETH it makes even more sense because different ERC20 tokens can be running on same ETH blockchain.
// So we would need to handle shared locks anyway.
lazy_static! {static ref NONCE_LOCK: Mutex<()> = Mutex::new(());}

type EthTxFut = Box<Future<Item=SignedEthTx, Error=String> + Send + 'static>;

impl EthCoin {
    fn sign_and_send_transaction(
        &self,
        value: U256,
        action: Action,
        data: Vec<u8>,
        gas: U256,
    ) -> EthTxFut {
        let arc = self.clone();
        let nonce_lock = MutexGuardWrapper(try_fus!(NONCE_LOCK.lock()));
        let nonce_fut = self.web3.eth().transaction_count(self.my_address, Some(BlockNumber::Pending)).map_err(|e| ERRL!("{}", e));
        Box::new(nonce_fut.then(move |nonce| -> EthTxFut {
            let nonce = try_fus!(nonce);
            let gas_price_fut = if let Some(url) = &arc.gas_station_url {
                GasStationData::get_gas_price(&url.clone())
            } else {
                Box::new(arc.web3.eth().gas_price().map_err(|e| ERRL!("{}", e)))
            };
            Box::new(gas_price_fut.then(move |gas_price| -> EthTxFut {
                let gas_price = try_fus!(gas_price);
                let tx = UnSignedEthTx {
                    nonce,
                    value,
                    action,
                    data,
                    gas,
                    gas_price,
                };

                let signed = tx.sign(arc.key_pair.secret(), None);
                let bytes = web3::types::Bytes(rlp::encode(&signed).to_vec());
                let send_fut = arc.web3.eth().send_raw_transaction(bytes).map_err(|e| ERRL!("{}", e));
                Box::new(send_fut.and_then(move |res| {
                    // Check every second till ETH nodes recognize that nonce is increased
                    // Parity has reliable "nextNonce" method that always returns correct nonce for address
                    // But we can't expect that all nodes will always be Parity.
                    // Some of ETH forks use Geth only so they don't have Parity nodes at all.
                    loop_fn((res, arc, nonce, nonce_lock), move |(res, arc, nonce, nonce_lock)| {
                        let delay_f = Delay::new(Duration::from_secs(1)).map_err(|e| ERRL!("{}", e));
                        delay_f.and_then(move |_res| {
                            arc.web3.eth().transaction_count(arc.my_address, Some(BlockNumber::Pending)).then(move |new_nonce| {
                                let new_nonce = match new_nonce {
                                    Ok(n) => n,
                                    Err(e) => {
                                        log!("Error " [e] " getting " [arc.ticker()] " " [arc.my_address] " nonce");
                                        // we can just keep looping in case of error hoping it will go away
                                        return Ok(Loop::Continue((res, arc, nonce, nonce_lock)));
                                    }
                                };
                                if new_nonce > nonce {
                                    drop(nonce_lock);
                                    Ok(Loop::Break(res))
                                } else {
                                    Ok(Loop::Continue((res, arc, nonce, nonce_lock)))
                                }
                            })
                        })
                    })
                }).map(move |_res| signed))
            }))
        }))
    }

    fn send_to_address(&self, address: Address, value: U256) -> EthTxFut {
        match self.coin_type {
            EthCoinType::Eth => self.sign_and_send_transaction(value, Action::Call(address), vec![], U256::from(21000)),
            EthCoinType::Erc20(token_addr) => {
                let abi = try_fus!(Contract::load(ERC20_ABI.as_bytes()));
                let function = try_fus!(abi.function("transfer"));
                let data = try_fus!(function.encode_input(&[
                    Token::Address(address),
                    Token::Uint(value)
                ]));
                self.sign_and_send_transaction(0.into(), Action::Call(token_addr), data, U256::from(210000))
            }
        }
    }

    fn send_hash_time_locked_payment(
        &self,
        value: U256,
        time_lock: u32,
        secret_hash: &[u8],
        receiver_addr: Address,
    ) -> EthTxFut {
        let mut rng = rand::thread_rng();
        let id: [u8; 32] = rng.gen();

        match self.coin_type {
            EthCoinType::Eth => {
                let function = try_fus!(SWAP_CONTRACT.function("ethPayment"));
                let data = try_fus!(function.encode_input(&[
                    Token::FixedBytes(id.to_vec()),
                    Token::Address(receiver_addr),
                    Token::FixedBytes(secret_hash.to_vec()),
                    Token::Uint(U256::from(time_lock))
                ]));
                self.sign_and_send_transaction(value, Action::Call(self.swap_contract_address), data, U256::from(150000))
            },
            EthCoinType::Erc20(token_addr) => {
                let allowance_fut = self.allowance(self.swap_contract_address);

                let function = try_fus!(SWAP_CONTRACT.function("erc20Payment"));
                let data = try_fus!(function.encode_input(&[
                    Token::FixedBytes(id.to_vec()),
                    Token::Uint(U256::from(value)),
                    Token::Address(token_addr),
                    Token::Address(receiver_addr),
                    Token::FixedBytes(secret_hash.to_vec()),
                    Token::Uint(U256::from(time_lock))
                ]));

                let arc = self.clone();
                Box::new(allowance_fut.and_then(move |allowed| -> EthTxFut {
                    if allowed < value {
                        let balance_f = arc.my_balance();
                        Box::new(balance_f.and_then(move |balance| {
                            arc.approve(arc.swap_contract_address, balance).and_then(move |_approved| {
                                arc.sign_and_send_transaction(0.into(), Action::Call(arc.swap_contract_address), data, U256::from(150000))
                            })
                        }))
                    } else {
                        Box::new(arc.sign_and_send_transaction(0.into(), Action::Call(arc.swap_contract_address), data, U256::from(150000)))
                    }
                }))
            }
        }
    }

    fn spend_hash_time_locked_payment(
        &self,
        payment: SignedEthTx,
        secret: &[u8],
    ) -> EthTxFut {
        let spend_func = try_fus!(SWAP_CONTRACT.function("receiverSpend"));
        let clone = self.clone();
        let secret_vec = secret.to_vec();

        match self.coin_type {
            EthCoinType::Eth => {
                let payment_func = try_fus!(SWAP_CONTRACT.function("ethPayment"));
                let decoded = try_fus!(payment_func.decode_input(&payment.data));

                let state_f = self.payment_state(decoded[0].clone());
                Box::new(state_f.and_then(move |state| -> EthTxFut {
                    if state != PAYMENT_STATE_SENT.into() {
                        return Box::new(futures::future::err(ERRL!("Payment {:?} state is not PAYMENT_STATE_SENT, got {}", payment, state)));
                    }

                    let value = payment.value;
                    let data = try_fus!(spend_func.encode_input(&[
                        decoded[0].clone(),
                        Token::Uint(value),
                        Token::FixedBytes(secret_vec),
                        Token::Address(Address::default()),
                        Token::Address(payment.sender()),
                    ]));

                    clone.sign_and_send_transaction(0.into(), Action::Call(clone.swap_contract_address), data, U256::from(150000))
                }))
            },
            EthCoinType::Erc20(token_addr) => {
                let payment_func = try_fus!(SWAP_CONTRACT.function("erc20Payment"));
                let decoded = try_fus!(payment_func.decode_input(&payment.data));
                let state_f = self.payment_state(decoded[0].clone());

                Box::new(state_f.and_then(move |state| -> EthTxFut {
                    if state != PAYMENT_STATE_SENT.into() {
                        return Box::new(futures::future::err(ERRL!("Payment {:?} state is not PAYMENT_STATE_SENT, got {}", payment, state)));
                    }
                    let data = try_fus!(spend_func.encode_input(&[
                        decoded[0].clone(),
                        decoded[1].clone(),
                        Token::FixedBytes(secret_vec),
                        Token::Address(token_addr),
                        Token::Address(payment.sender()),
                    ]));

                    clone.sign_and_send_transaction(0.into(), Action::Call(clone.swap_contract_address), data, U256::from(150000))
                }))
            }
        }
    }

    fn refund_hash_time_locked_payment(
        &self,
        payment: SignedEthTx,
    ) -> EthTxFut {
        let refund_func = try_fus!(SWAP_CONTRACT.function("senderRefund"));
        let clone = self.clone();

        match self.coin_type {
            EthCoinType::Eth => {
                let payment_func = try_fus!(SWAP_CONTRACT.function("ethPayment"));
                let decoded = try_fus!(payment_func.decode_input(&payment.data));

                let state_f = self.payment_state(decoded[0].clone());
                Box::new(state_f.and_then(move |state| -> EthTxFut {
                    if state != PAYMENT_STATE_SENT.into() {
                        return Box::new(futures::future::err(ERRL!("Payment {:?} state is not PAYMENT_STATE_SENT, got {}", payment, state)));
                    }

                    let value = payment.value;
                    let data = try_fus!(refund_func.encode_input(&[
                        decoded[0].clone(),
                        Token::Uint(value),
                        decoded[2].clone(),
                        Token::Address(Address::default()),
                        Token::Address(payment.sender()),
                    ]));

                    clone.sign_and_send_transaction(0.into(), Action::Call(clone.swap_contract_address), data, U256::from(150000))
                }))
            },
            EthCoinType::Erc20(token_addr) => {
                let payment_func = try_fus!(SWAP_CONTRACT.function("erc20Payment"));
                let decoded = try_fus!(payment_func.decode_input(&payment.data));
                let state_f = self.payment_state(decoded[0].clone());
                Box::new(state_f.and_then(move |state| -> EthTxFut {
                    if state != PAYMENT_STATE_SENT.into() {
                        return Box::new(futures::future::err(ERRL!("Payment {:?} state is not PAYMENT_STATE_SENT, got {}", payment, state)));
                    }

                    let data = try_fus!(refund_func.encode_input(&[
                        decoded[0].clone(),
                        decoded[1].clone(),
                        decoded[4].clone(),
                        Token::Address(token_addr),
                        Token::Address(payment.sender()),
                    ]));

                    clone.sign_and_send_transaction(0.into(), Action::Call(clone.swap_contract_address), data, U256::from(150000))
                }))
            }
        }
    }

    fn my_balance(&self) -> Box<Future<Item=U256, Error=String> + Send> {
        match self.coin_type {
            EthCoinType::Eth => Box::new(self.web3.eth().balance(self.my_address, Some(BlockNumber::Latest)).map_err(|e| ERRL!("{:?}", e))),
            EthCoinType::Erc20(token_addr) => {
                let function = try_fus!(ERC20_CONTRACT.function("balanceOf"));
                let data = try_fus!(function.encode_input(&[
                    Token::Address(self.my_address),
                ]));

                let call_fut = self.call_request(token_addr, None, Some(data.into()));

                Box::new(call_fut.and_then(move |res| {
                    let decoded = try_s!(function.decode_output(&res.0));

                    match decoded[0] {
                        Token::Uint(number) => Ok(number),
                        _ => ERR!("Expected U256 as balanceOf result but got {:?}", decoded),
                    }
                }))
            }
        }
    }

    fn eth_balance(&self) -> Box<Future<Item=U256, Error=String> + Send> {
        Box::new(self.web3.eth().balance(self.my_address, Some(BlockNumber::Latest)).map_err(|e| ERRL!("{:?}", e)))
    }

    fn call_request(&self, to: Address, value: Option<U256>, data: Option<Bytes>) -> impl Future<Item=Bytes, Error=String> {
        let request = CallRequest {
            from: Some(self.my_address),
            to,
            gas: None,
            gas_price: None,
            value,
            data
        };

        self.web3.eth().call(request, Some(BlockNumber::Latest)).map_err(|e| ERRL!("{:?}", e))
    }

    fn allowance(&self, spender: Address) -> Box<Future<Item=U256, Error=String> + Send + 'static> {
        match self.coin_type {
            EthCoinType::Eth => panic!(),
            EthCoinType::Erc20(token_addr) => {
                let function = try_fus!(ERC20_CONTRACT.function("allowance"));
                let data = try_fus!(function.encode_input(&[
                    Token::Address(self.my_address),
                    Token::Address(spender),
                ]));

                let call_fut = self.call_request(token_addr, None, Some(data.into()));

                Box::new(call_fut.and_then(move |res| {
                    let decoded = try_s!(function.decode_output(&res.0));

                    match decoded[0] {
                        Token::Uint(number) => Ok(number),
                        _ => ERR!("Expected U256 as allowance result but got {:?}", decoded),
                    }
                }))
            }
        }
    }

    fn approve(&self, spender: Address, amount: U256) -> EthTxFut {
        match self.coin_type {
            EthCoinType::Eth => panic!(),
            EthCoinType::Erc20(token_addr) => {
                let function = try_fus!(ERC20_CONTRACT.function("approve"));
                let data = try_fus!(function.encode_input(&[
                    Token::Address(spender),
                    Token::Uint(amount),
                ]));

                self.sign_and_send_transaction(0.into(), Action::Call(token_addr), data, U256::from(150000))
            }
        }
    }

    /// Gets `ReceiverSpent` events from etomic swap smart contract (`self.swap_contract_address` ) since `from_block`
    fn spend_events(&self, from_block: u64) -> Box<Future<Item=Vec<Log>, Error=String>> {
        let contract_event = try_fus!(SWAP_CONTRACT.event("ReceiverSpent"));
        let filter = FilterBuilder::default()
            .topics(Some(vec![contract_event.signature()]), None, None, None)
            .from_block(BlockNumber::Number(from_block))
            .address(vec![self.swap_contract_address])
            .build();

        Box::new(self.web3.eth().logs(filter).map_err(|e| ERRL!("{:?}", e)))
    }

    fn validate_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        sender_pub: &[u8],
        secret_hash: &[u8],
        amount: u64,
    ) -> Result<(), String> {
        let unsigned: UnverifiedTransaction = try_s!(rlp::decode(payment_tx));
        let tx = try_s!(SignedEthTx::new(unsigned));

        let expected_value = u256_denominate_from_satoshis(amount, self.decimals);
        let tx_from_rpc = try_s!(self.web3.eth().transaction(TransactionId::Hash(tx.hash)).wait());
        let tx_from_rpc = match tx_from_rpc {
            Some(t) => t,
            None => return ERR!("Didn't find provided tx {:?} on ETH node", tx),
        };

        let sender = try_s!(addr_from_raw_pubkey(sender_pub));
        if tx_from_rpc.from != sender {
            return ERR!("Payment tx {:?} was sent from wrong address, expected {:?}", tx_from_rpc, sender);
        }

        match self.coin_type {
            EthCoinType::Eth => {
                if tx_from_rpc.to != Some(self.swap_contract_address) {
                    return ERR!("Payment tx {:?} was sent to wrong address, expected {:?}", tx_from_rpc, self.swap_contract_address);
                }

                if tx_from_rpc.value != expected_value {
                    return ERR!("Payment tx {:?} value is invalid, expected {:?}", tx_from_rpc, expected_value);
                }

                let function = try_s!(SWAP_CONTRACT.function("ethPayment"));
                let decoded = try_s!(function.decode_input(&tx_from_rpc.input.0));
                if decoded[1] != Token::Address(self.my_address) {
                    return ERR!("Payment tx receiver arg {:?} is invalid, expected {:?}", decoded[1], Token::Address(self.my_address));
                }

                if decoded[2] != Token::FixedBytes(secret_hash.to_vec()) {
                    return ERR!("Payment tx secret_hash arg {:?} is invalid, expected {:?}", decoded[2], Token::FixedBytes(secret_hash.to_vec()));
                }

                if decoded[3] != Token::Uint(U256::from(time_lock)) {
                    return ERR!("Payment tx time_lock arg {:?} is invalid, expected {:?}", decoded[3], Token::Uint(U256::from(time_lock)));
                }
            },
            EthCoinType::Erc20(token_addr) => {
                if tx_from_rpc.to != Some(self.swap_contract_address) {
                    return ERR!("Payment tx {:?} was sent to wrong address, expected {:?}", tx_from_rpc, self.swap_contract_address);
                }

                let function = try_s!(SWAP_CONTRACT.function("erc20Payment"));
                let decoded = try_s!(function.decode_input(&tx_from_rpc.input.0));
                if decoded[1] != Token::Uint(expected_value) {
                    return ERR!("Payment tx value arg {:?} is invalid, expected {:?}", decoded[1], Token::Uint(expected_value));
                }

                if decoded[2] != Token::Address(token_addr) {
                    return ERR!("Payment tx token_addr arg {:?} is invalid, expected {:?}", decoded[2], Token::Address(token_addr));
                }

                if decoded[3] != Token::Address(self.my_address) {
                    return ERR!("Payment tx receiver arg {:?} is invalid, expected {:?}", decoded[3], Token::Address(self.my_address));
                }

                if decoded[4] != Token::FixedBytes(secret_hash.to_vec()) {
                    return ERR!("Payment tx secret_hash arg {:?} is invalid, expected {:?}", decoded[4], Token::FixedBytes(secret_hash.to_vec()));
                }

                if decoded[5] != Token::Uint(U256::from(time_lock)) {
                    return ERR!("Payment tx time_lock arg {:?} is invalid, expected {:?}", decoded[5], Token::Uint(U256::from(time_lock)));
                }
            },
        }

        Ok(())
    }

    fn payment_state(&self, token: Token) -> Box<Future<Item=U256, Error=String> + Send + 'static> {
        let function = try_fus!(SWAP_CONTRACT.function("payments"));

        let data = try_fus!(function.encode_input(&[token]));

        Box::new(self.call_request(self.swap_contract_address, None, Some(data.into())).and_then(move |bytes| {
            let decoded_tokens = try_s!(function.decode_output(&bytes.0));
            match decoded_tokens[2] {
                Token::Uint(state) => Ok(state),
                _ => ERR!("Payment status must be uint, got {:?}", decoded_tokens[2]),
            }
        }))
    }

    /// Downloads and saves ERC20 transaction history of my_address
    fn process_erc20_history(&self, token_addr: H160, ctx: &MmArc) {
        loop {
            let mut existing_history = self.load_history_from_file(ctx);

            // find the earliest and latest blocks for which we have history
            // if downloading is interrupted for some reason we might not have all transactions from the past
            // transactions are sorted by block number in descending order so it's ok to get first and last elements
            let (earliest_block, latest_block) = if existing_history.is_empty() {
                (BlockNumber::Earliest, BlockNumber::Earliest)
            } else {
                // can safely unwrap as history is not empty here
                let max = unwrap!(existing_history.first()).block_height + 1;
                let min = unwrap!(existing_history.last()).block_height - 1;
                (BlockNumber::Number(min), BlockNumber::Number(max))
            };

            // AP: AFAIK ETH RPC doesn't support conditional filters like `get this OR this` so we have
            // to run several queries to get transfer events including our address as sender `or` receiver
            // TODO refactor this to batch requests instead of single request per query
            let from_events_before_earliest = match self.erc20_transfer_events(
                token_addr,
                Some(self.my_address),
                None,
                BlockNumber::Earliest,
                earliest_block,
                None,
            ).wait() {
                Ok(events) => events,
                Err(e) => {
                    ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on erc20_transfer_events, retrying", e));
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

            let to_events_before_earliest = match self.erc20_transfer_events(
                token_addr,
                None,
                Some(self.my_address),
                BlockNumber::Earliest,
                earliest_block,
                None,
            ).wait() {
                Ok(events) => events,
                Err(e) => {
                    ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on erc20_transfer_events, retrying", e));
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

            let from_events_after_latest = match self.erc20_transfer_events(
                token_addr,
                Some(self.my_address),
                None,
                latest_block,
                BlockNumber::Latest,
                None,
            ).wait() {
                Ok(events) => events,
                Err(e) => {
                    ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on erc20_transfer_events, retrying", e));
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

            let to_events_after_latest = match self.erc20_transfer_events(
                token_addr,
                None,
                Some(self.my_address),
                latest_block,
                BlockNumber::Latest,
                None,
            ).wait() {
                Ok(events) => events,
                Err(e) => {
                    ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on erc20_transfer_events, retrying", e));
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

            let all_events = from_events_before_earliest.into_iter()
                .chain(to_events_before_earliest)
                .chain(from_events_after_latest)
                .chain(to_events_after_latest);

            let all_events: HashMap<H256, Log> = all_events
                .filter(|e| e.block_number.is_some() && e.transaction_hash.is_some() && !e.is_removed())
                .map(|e| (e.transaction_hash.clone().unwrap(), e)).collect();
            let mut all_events: Vec<Log> = all_events.into_iter().map(|(_, log)| log).collect();
            all_events.sort_by(|a, b| b.block_number.unwrap().cmp(&a.block_number.unwrap()));

            for event in all_events {
                let internal_id = sha256(&json::to_vec(&event).unwrap());
                let amount = U256::from(event.data.0.as_slice());
                let total_amount: f64 = display_u256_with_decimal_point(amount, 18).parse().unwrap();
                let mut received_by_me = 0.;
                let mut spent_by_me = 0.;

                let from_addr = H160::from(event.topics[1]);
                let to_addr = H160::from(event.topics[2]);

                if from_addr == self.my_address {
                    spent_by_me = total_amount;
                }

                if to_addr == self.my_address {
                    received_by_me = total_amount;
                }

                let web3_tx = match self.web3.eth().transaction(TransactionId::Hash(event.transaction_hash.unwrap())).wait() {
                    Ok(tx) => tx,
                    Err(e) => {
                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on getting transaction {:?}", e, event.transaction_hash.unwrap()));
                        continue;
                    }
                };
                let web3_tx = match web3_tx {
                    Some(t) => t,
                    None => {
                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("No such transaction {:?}", event.transaction_hash.unwrap()));
                        continue;
                    }
                };

                let receipt = match self.web3.eth().transaction_receipt(event.transaction_hash.unwrap()).wait() {
                    Ok(r) => r,
                    Err(e) => {
                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on getting transaction {:?} receipt", e, event.transaction_hash.unwrap()));
                        continue;
                    }
                };
                let fee_details = match receipt {
                    Some(r) => Some(unwrap!(EthTxFeeDetails::new(r.gas_used.unwrap_or(0.into()), web3_tx.gas_price, "ETH"))),
                    None => None,
                };
                let block_number = event.block_number.unwrap();
                let block = match self.web3.eth().block(BlockId::Number(BlockNumber::Number(block_number.into()))).wait() {
                    Ok(b) => unwrap!(b),
                    Err(e) => {
                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on getting block {} data", e, block_number));
                        continue;
                    }
                };

                let raw = signed_tx_from_web3_tx(web3_tx).unwrap();
                let details = TransactionDetails {
                    spent_by_me,
                    received_by_me,
                    total_amount,
                    to: vec![format!("{:#02x}", to_addr)],
                    from: vec![format!("{:#02x}", from_addr)],
                    coin: self.ticker.clone(),
                    fee_details: unwrap!(json::to_value(fee_details)),
                    block_height: block_number.into(),
                    my_balance_change: received_by_me - spent_by_me,
                    tx_hash: BytesJson(raw.hash.to_vec()),
                    tx_hex: BytesJson(rlp::encode(&raw)),
                    internal_id: BytesJson(internal_id.to_vec()),
                    timestamp: block.timestamp.into(),
                };

                existing_history.push(details);
                existing_history.sort_unstable_by(|a, b| if a.block_height == 0 {
                    Ordering::Less
                } else if b.block_height == 0 {
                    Ordering::Greater
                } else {
                    b.block_height.cmp(&a.block_height)
                });
                self.save_history_to_file(&unwrap!(json::to_vec(&existing_history)), &ctx);
            }
            thread::sleep(Duration::from_secs(30));
        }
    }

    /// Downloads and saves ETH transaction history of my_address, relies on Parity trace_filter API
    /// https://wiki.parity.io/JSONRPC-trace-module#trace_filter, this requires tracing to be enabled
    /// in node config. Other ETH clients (Geth, etc.) are `not` supported (yet).
    fn process_eth_history(&self, ctx: &MmArc) {
        loop {
            let mut existing_history = self.load_history_from_file(ctx);

            // find the earliest and latest blocks for which we have history
            // if downloading is interrupted for some reason we might not have all transactions from the past
            // transactions are sorted by block number in descending order so it's ok to get first and last elements
            let (earliest_block, latest_block) = if existing_history.is_empty() {
                (BlockNumber::Earliest, BlockNumber::Earliest)
            } else {
                // can safely unwrap as history is not empty here
                let max = unwrap!(existing_history.first()).block_height + 1;
                let min = unwrap!(existing_history.last()).block_height - 1;
                (BlockNumber::Number(min), BlockNumber::Number(max))
            };

            // AP: AFAIK ETH RPC doesn't support conditional filters like `get this OR this` so we have
            // to run several queries to get trace events including our address as sender `or` receiver
            // TODO refactor this to batch requests instead of single request per query
            let from_traces_before_earliest = match self.eth_traces(
                vec![self.my_address],
                vec![],
                BlockNumber::Earliest,
                earliest_block,
                None,
            ).wait() {
                Ok(traces) => traces,
                Err(e) => {
                    ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on eth_traces, retrying", e));
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

            let to_traces_before_earliest = match self.eth_traces(
                vec![],
                vec![self.my_address],
                BlockNumber::Earliest,
                earliest_block,
                None,
            ).wait() {
                Ok(traces) => traces,
                Err(e) => {
                    ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on eth_traces, retrying", e));
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

            let from_traces_after_latest = match self.eth_traces(
                vec![self.my_address],
                vec![],
                latest_block,
                BlockNumber::Latest,
                None,
            ).wait() {
                Ok(traces) => traces,
                Err(e) => {
                    ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on eth_traces, retrying", e));
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

            let to_traces_after_latest = match self.eth_traces(
                vec![],
                vec![self.my_address],
                latest_block,
                BlockNumber::Latest,
                None,
            ).wait() {
                Ok(traces) => traces,
                Err(e) => {
                    ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on eth_traces, retrying", e));
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

            let mut all_traces: Vec<Trace> = from_traces_before_earliest.into_iter()
                .chain(to_traces_before_earliest)
                .chain(from_traces_after_latest)
                .chain(to_traces_after_latest)
                .collect();
            all_traces.sort_by(|a, b| b.block_number.cmp(&a.block_number));

            for trace in all_traces {
                let internal_id = sha256(&json::to_vec(&trace).unwrap());

                // TODO Only standard Call traces are supported, contract creations, suicides and block rewards will be supported later
                let call_data = match trace.action {
                    TraceAction::Call(d) => d,
                    _ => continue,
                };

                let web3_tx = match self.web3.eth().transaction(TransactionId::Hash(trace.transaction_hash.unwrap())).wait() {
                    Ok(tx) => tx,
                    Err(e) => {
                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on getting transaction {:?}", e, trace.transaction_hash.unwrap()));
                        continue;
                    }
                };
                let web3_tx = match web3_tx {
                    Some(t) => t,
                    None => {
                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("No such transaction {:?}", trace.transaction_hash.unwrap()));
                        continue;
                    }
                };

                let receipt = match self.web3.eth().transaction_receipt(trace.transaction_hash.unwrap()).wait() {
                    Ok(r) => r,
                    Err(e) => {
                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on getting transaction {:?} receipt", e, trace.transaction_hash.unwrap()));
                        continue;
                    }
                };
                let fee_details: Option<EthTxFeeDetails> = match receipt {
                    Some(r) => Some(unwrap!(EthTxFeeDetails::new(r.gas_used.unwrap_or(0.into()), web3_tx.gas_price, "ETH"))),
                    None => None,
                };

                let total_amount: f64 = display_u256_with_decimal_point(call_data.value, 18).parse().unwrap();
                let mut received_by_me = 0.;
                let mut spent_by_me = 0.;

                if call_data.from == self.my_address {
                    spent_by_me = total_amount;
                    if let Some(ref fee) = fee_details {
                        spent_by_me += fee.total_fee;
                    }
                }

                if call_data.to == self.my_address {
                    received_by_me = total_amount;
                }

                let raw = signed_tx_from_web3_tx(web3_tx).unwrap();
                let block = match self.web3.eth().block(BlockId::Number(BlockNumber::Number(trace.block_number))).wait() {
                    Ok(b) => unwrap!(b),
                    Err(e) => {
                        ctx.log.log("", &[&"tx_history", &self.ticker], &ERRL!("Error {} on getting block {} data", e, trace.block_number));
                        continue;
                    }
                };

                let details = TransactionDetails {
                    spent_by_me,
                    received_by_me,
                    total_amount,
                    to: vec![format!("{:#02x}", call_data.to)],
                    from: vec![format!("{:#02x}", call_data.from)],
                    coin: self.ticker.clone(),
                    fee_details: unwrap!(json::to_value(fee_details)),
                    block_height: trace.block_number,
                    my_balance_change: received_by_me - spent_by_me,
                    tx_hash: BytesJson(raw.hash.to_vec()),
                    tx_hex: BytesJson(rlp::encode(&raw)),
                    internal_id: BytesJson::from(internal_id.to_vec()),
                    timestamp: block.timestamp.into(),
                };

                existing_history.push(details);
                existing_history.sort_unstable_by(|a, b| if a.block_height == 0 {
                    Ordering::Less
                } else if b.block_height == 0 {
                    Ordering::Greater
                } else {
                    b.block_height.cmp(&a.block_height)
                });
                self.save_history_to_file(&unwrap!(json::to_vec(&existing_history)), &ctx);
            }
            thread::sleep(Duration::from_secs(30));
        }
    }
}

impl IguanaInfo for EthCoin {
    fn ticker<'a> (&'a self) -> &'a str {&self.ticker[..]}
}

#[derive(Serialize)]
struct EthTxFeeDetails {
    coin: String,
    gas: u64,
    /// ETH units per 1 gas
    gas_price: f64,
    total_fee: f64,
}

impl EthTxFeeDetails {
    fn new(gas: U256, gas_price: U256, coin: &str) -> Result<EthTxFeeDetails, String> {
        let total_fee = gas * gas_price;
        // Fees are always paid in ETH, can use 18 decimals by default
        let total_fee = try_s!(u256_to_f64(total_fee, 18));
        let gas_price = try_s!(u256_to_f64(gas_price, 18));

        Ok(EthTxFeeDetails {
            coin: coin.to_owned(),
            gas: gas.into(),
            gas_price,
            total_fee,
        })
    }
}

impl MmCoin for EthCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn check_i_have_enough_to_trade(&self, amount: f64, maker: bool) -> Box<Future<Item=(), Error=String> + Send> {
        let arc = self.clone();
        let decimals = self.decimals;
        let ticker = self.ticker.clone();
        let required = if maker {
            amount
        } else {
            amount + amount / 777.0
        };
        Box::new(self.my_balance().and_then(move |balance| -> Box<Future<Item=(), Error=String> + Send> {
            let balance_f64: f64 = try_fus!(display_u256_with_decimal_point(balance, decimals).parse());
            match arc.coin_type {
                EthCoinType::Eth => {
                    let required = required + 0.0002;
                    if balance_f64 < required {
                        Box::new(futures::future::err(ERRL!("{} balance {} too low, required {}", ticker, balance_f64, required)))
                    } else {
                        Box::new(futures::future::ok(()))
                    }
                },
                EthCoinType::Erc20(_addr) => {
                    if balance_f64 < required {
                        Box::new(futures::future::err(ERRL!("{} balance {} too low, required {}", ticker, balance_f64, amount)))
                    } else {
                        // need to check ETH balance too, address should have some to cover gas fees
                        Box::new(arc.eth_balance().and_then(move |eth_balance| {
                            let eth_balance_f64: f64 = try_s!(display_u256_with_decimal_point(eth_balance, 18).parse());
                            if eth_balance_f64 < 0.0002 {
                                ERR!("{} balance is enough, but base coin balance {} is too low to cover gas fee, required {}", ticker, eth_balance_f64, 0.0002)
                            } else {
                                Ok(())
                            }
                        }))
                    }
                }
            }
        }))
    }

    fn can_i_spend_other_payment(&self) -> Box<Future<Item=(), Error=String> + Send> {
        Box::new(self.eth_balance().and_then(move |eth_balance| {
            let eth_balance_f64: f64 = try_s!(display_u256_with_decimal_point(eth_balance, 18).parse());
            if eth_balance_f64 < 0.0002 {
                ERR!("Base coin balance {} is too low to cover gas fee, required {}", eth_balance_f64, 0.0002)
            } else {
                Ok(())
            }
        }))
    }

    fn withdraw(&self, to: &str, amount: f64) -> Box<Future<Item=TransactionDetails, Error=String> + Send> {
        let to_addr = try_fus!(addr_from_str(to));
        let wei_amount = try_fus!(wei_from_f64(amount, self.decimals));
        let (value, data, call_addr) = match self.coin_type {
            EthCoinType::Eth => (wei_amount, vec![], to_addr),
            EthCoinType::Erc20(token_addr) => {
                let function = try_fus!(ERC20_CONTRACT.function("transfer"));
                let data = try_fus!(function.encode_input(&[Token::Address(to_addr), Token::Uint(wei_amount)]));
                (0.into(), data, token_addr)
            }
        };
        let arc = self.clone();
        let nonce_lock = MutexGuardWrapper(try_fus!(NONCE_LOCK.lock()));
        let nonce_fut = self.web3.eth().transaction_count(self.my_address, Some(BlockNumber::Pending)).map_err(|e| ERRL!("{}", e));
        Box::new(nonce_fut.and_then(move |nonce| {
            let gas_price_fut = if let Some(url) = &arc.gas_station_url {
                Either::A(GasStationData::get_gas_price(&url.clone()))
            } else {
                Either::B(arc.web3.eth().gas_price().map_err(|e| ERRL!("{}", e)))
            };
            gas_price_fut.and_then(move |gas_price| {
                let estimate_gas_req = CallRequest {
                    value: Some(value),
                    data: Some(data.clone().into()),
                    from: Some(arc.my_address),
                    to: call_addr,
                    gas: None,
                    gas_price: Some(gas_price)
                };

                let estimate_gas_fut = arc.web3.eth().estimate_gas(estimate_gas_req, None).map_err(|e| ERRL!("{}", e));
                estimate_gas_fut.and_then(move |gas| {
                    let tx = UnSignedEthTx { nonce, value, action: Action::Call(call_addr), data, gas, gas_price };

                    let signed = tx.sign(arc.key_pair.secret(), None);
                    let bytes = rlp::encode(&signed);
                    let amount_f64 = try_s!(u256_to_f64(wei_amount, arc.decimals));
                    let mut spent_by_me = amount_f64;
                    let mut received_by_me = 0.;
                    if to_addr == arc.my_address {
                        received_by_me = amount_f64;
                    }
                    let fee_details = try_s!(EthTxFeeDetails::new(gas, gas_price, "ETH"));
                    if arc.coin_type == EthCoinType::Eth {
                        spent_by_me += fee_details.total_fee;
                    }
                    let fee_details = try_s!(json::to_value(fee_details));
                    drop(nonce_lock);
                    Ok(TransactionDetails {
                        to: vec![format!("{:#02x}", to_addr)],
                        from: vec![arc.my_address().into()],
                        total_amount: amount_f64,
                        spent_by_me,
                        received_by_me,
                        my_balance_change: received_by_me - spent_by_me,
                        tx_hex: bytes.into(),
                        tx_hash: signed.tx_hash(),
                        block_height: 0,
                        fee_details,
                        coin: arc.ticker.clone(),
                        internal_id: vec![].into(),
                        timestamp: now_ms() / 1000,
                    })
                })
            })
        }))
    }

    fn decimals(&self) -> u8 {
        self.decimals
    }

    fn tx_details_by_hash(&self, hash: &[u8]) -> Result<TransactionDetails, String> {
        let hash = H256::from(hash);
        let tx = try_s!(self.web3.eth().transaction(TransactionId::Hash(hash)).wait());
        let tx: Web3Transaction = try_s!(tx.ok_or(format!("tx hash {:02x} is not found", hash)));
        let raw = try_s!(signed_tx_from_web3_tx(tx.clone()));
        let mut received_by_me = 0f64;
        let mut spent_by_me = 0f64;

        let to = match tx.to {
            Some(addr) => vec![format!("{:#02x}", addr)],
            None => vec![],
        };
        let total_amount = try_s!(display_u256_with_decimal_point(tx.value, self.decimals).parse());

        match self.coin_type {
            EthCoinType::Eth => {
                if tx.to == Some(self.my_address) {
                    received_by_me = total_amount;
                }

                if tx.from == self.my_address {
                    spent_by_me = total_amount;
                }

                Ok(TransactionDetails {
                    from: vec![format!("{:#02x}", tx.from)],
                    to,
                    coin: self.ticker.clone(),
                    block_height: tx.block_number.unwrap_or(U256::from(0)).into(),
                    tx_hex: rlp::encode(&raw).into(),
                    tx_hash: tx.hash.0.to_vec().into(),
                    received_by_me,
                    spent_by_me,
                    my_balance_change: received_by_me - spent_by_me,
                    total_amount,
                    fee_details: Json::Null,
                    internal_id: vec![0].into(),
                    timestamp: now_ms() / 1000,
                })
            },
            EthCoinType::Erc20(_addr) => {
                Ok(TransactionDetails {
                    from: vec![format!("{:#02x}", tx.from)],
                    to,
                    coin: self.ticker.clone(),
                    block_height: tx.block_number.unwrap_or(U256::from(0)).into(),
                    tx_hex: rlp::encode(&raw).into(),
                    tx_hash: tx.hash.0.to_vec().into(),
                    received_by_me,
                    spent_by_me,
                    my_balance_change: received_by_me - spent_by_me,
                    total_amount,
                    fee_details: Json::Null,
                    internal_id: vec![0].into(),
                    timestamp: now_ms() / 1000,
                })
            },
        }
    }

    fn process_history_loop(&self, ctx: MmArc) {
        match self.coin_type {
            EthCoinType::Eth => self.process_eth_history(&ctx),
            EthCoinType::Erc20(token) => self.process_erc20_history(token, &ctx),
        }
    }
}

fn addr_from_raw_pubkey(pubkey: &[u8]) -> Result<Address, String> {
    let pubkey = try_s!(PublicKey::from_slice(&SECP256K1, &pubkey));
    let eth_public = Public::from(&pubkey.serialize_vec(&SECP256K1, false)[1..65]);
    Ok(public_to_address(&eth_public))
}

fn u256_denominate_from_satoshis(satoshis: u64, decimals: u8) -> U256 {
    if decimals < 8 {
        U256::from(satoshis) / U256::exp10(8 - decimals as usize)
    } else {
        U256::from(satoshis) * U256::exp10(decimals as usize - 8)
    }
}

fn display_u256_with_decimal_point(number: U256, decimals: u8) -> String {
    let mut string = number.to_string();
    let decimals = decimals as usize;
    if string.len() <= decimals {
        string.insert_str(0, &"0".repeat(decimals - string.len() + 1));
    }

    string.insert(string.len() - decimals, '.');
    string.trim_end_matches('0').into()
}

#[test]
fn display_u256_with_point() {
    let number = U256::from_dec_str("1000000000000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 18);
    assert_eq!("1.", string);

    let number = U256::from_dec_str("1234567890000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 18);
    assert_eq!("1.23456789", string);

    let number = U256::from_dec_str("1234567890000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 16);
    assert_eq!("123.456789", string);

    let number = U256::from_dec_str("1234567890000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 0);
    assert_eq!("1234567890000000000.", string);

    let number = U256::from_dec_str("1000000000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 18);
    assert_eq!("0.001", string);

    let number = U256::from_dec_str("0").unwrap();
    let string = display_u256_with_decimal_point(number, 18);
    assert_eq!("0.", string);

    let number = U256::from_dec_str("0").unwrap();
    let string = display_u256_with_decimal_point(number, 0);
    assert_eq!("0.", string);
}

fn u256_to_f64(number: U256, decimals: u8) -> Result<f64, String> {
    let string = display_u256_with_decimal_point(number, decimals);
    Ok(try_s!(string.parse()))
}

fn wei_from_f64(amount: f64, decimals: u8) -> Result<U256, String> {
    let mut amount = amount.to_string();
    let dot = amount.find(|c| c == '.');
    let decimals = decimals as usize;
    if let Some(index) = dot {
        let mut fractional = amount.split_off(index);
        // remove the dot from fractional part
        fractional.remove(0);
        if fractional.len() < decimals {
            fractional.insert_str(fractional.len(), &"0".repeat(decimals - fractional.len()));
        }
        fractional.truncate(decimals);
        amount.push_str(&fractional);
    } else {
        amount.insert_str(amount.len(), &"0".repeat(decimals));
    }
    Ok(try_s!(U256::from_dec_str(&amount).map_err(|e| ERRL!("{:?}", e))))
}

#[test]
fn test_wei_from_f64() {
    let amount = 0.000001;
    let wei = wei_from_f64(amount, 18).unwrap();
    let expected_wei: U256 = 1000000000000u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1.000001;
    let wei = wei_from_f64(amount, 18).unwrap();
    let expected_wei: U256 = 1000001000000000000u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1.;
    let wei = wei_from_f64(amount, 18).unwrap();
    let expected_wei: U256 = 1000000000000000000u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 0.000000000000000001;
    let wei = wei_from_f64(amount, 18).unwrap();
    let expected_wei: U256 = 1u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1234.;
    let wei = wei_from_f64(amount, 9).unwrap();
    let expected_wei: U256 = 1234000000000u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1234.;
    let wei = wei_from_f64(amount, 0).unwrap();
    let expected_wei: U256 = 1234u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1234.;
    let wei = wei_from_f64(amount, 1).unwrap();
    let expected_wei: U256 = 12340u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1234.12345;
    let wei = wei_from_f64(amount, 1).unwrap();
    let expected_wei: U256 = 12341u64.into();
    assert_eq!(expected_wei, wei);
}

impl Transaction for SignedEthTx {
    fn tx_hex(&self) -> Vec<u8> { rlp::encode(self).to_vec() }

    fn extract_secret(&self) -> Result<Vec<u8>, String> {
        let function = try_s!(SWAP_CONTRACT.function("receiverSpend"));
        let tokens = try_s!(function.decode_input(&self.data));
        match &tokens[2] {
            Token::FixedBytes(secret) => Ok(secret.to_vec()),
            _ => ERR!("Expected secret to be fixed bytes, decoded function data is {:?}", tokens),
        }
    }

    fn tx_hash(&self) -> BytesJson { self.hash.to_vec().into() }

    fn amount(&self, decimals: u8) -> Result<f64, String> {
        Ok(try_s!(display_u256_with_decimal_point(self.value, decimals).parse()))
    }

    fn from(&self) -> Vec<String> { vec![format!("{:#02x}", self.sender)] }

    fn to(&self) -> Vec<String> {
        match self.action {
            Action::Create => vec!["null".into()],
            Action::Call(addr) => vec![format!("{:#02x}", addr)],
        }
    }

    fn fee_details(&self) -> Result<Json, String> {
        let fee = try_s!(EthTxFeeDetails::new(self.gas, self.gas_price, "ETH"));
        Ok(try_s!(json::to_value(fee)))
    }
}

fn signed_tx_from_web3_tx(transaction: Web3Transaction) -> Result<SignedEthTx, String> {
    let unverified = UnverifiedTransaction {
        r: transaction.r,
        s: transaction.s,
        v: transaction.v.as_u64(),
        hash: transaction.hash,
        unsigned: UnSignedEthTx {
            data: transaction.input.0,
            gas_price: transaction.gas_price,
            gas: transaction.gas,
            value: transaction.value,
            nonce: transaction.nonce,
            action: match transaction.to {
                Some(addr) => Action::Call(addr),
                None => Action::Create,
            }
        }
    };

    Ok(try_s!(SignedEthTx::new(unverified)))
}

#[derive(Deserialize, Debug)]
struct GasStationData {
    fast: f64,
    speed: f64,
    fastest: f64,
    #[serde(rename = "avgWait")]
    avg_wait: f64,
    #[serde(rename = "fastWait")]
    fast_wait: f64,
    #[serde(rename = "blockNum")]
    block_num: u64,
    #[serde(rename = "safeLowWait")]
    safe_low_wait: f64,
    block_time: f64,
    #[serde(rename = "fastestWait")]
    fastest_wait: f64,
    #[serde(rename = "safeLow")]
    safe_low: f64,
    average: f64
}

impl GasStationData {
    fn average_gwei(&self) -> U256 {
        U256::from(self.average as u64 + 10) * U256::exp10(8)
    }

    fn get_gas_price(uri: &str) -> Box<Future<Item=U256, Error=String> + Send> {
        Box::new(slurp_url(uri).and_then(|res| -> Result<U256, String> {
            if res.0 != StatusCode::OK {
                return ERR!("Gas price request failed with status code {}", res.0);
            }

            let result: GasStationData = try_s!(json::from_slice(&res.2));
            Ok(result.average_gwei())
        }))
    }
}

fn get_token_decimals(web3: &Web3<Web3Transport>, token_addr: Address) -> Result<u8, String> {
    let function = try_s!(ERC20_CONTRACT.function("decimals"));
    let data = try_s!(function.encode_input(&[]));
    let request = CallRequest {
        from: Some(Address::default()),
        to: token_addr,
        gas: None,
        gas_price: None,
        value: Some(0.into()),
        data: Some(data.into())
    };

    let f = web3.eth().call(request, Some(BlockNumber::Latest)).map_err(|e| ERRL!("{:?}", e));
    let res = try_s!(f.wait());
    let tokens = try_s!(function.decode_output(&res.0));
    let decimals: u64 = match tokens[0] {
        Token::Uint(dec) => dec.into(),
        _ => return ERR!("Invalid decimals type {:?}", tokens),
    };
    Ok(decimals as u8)
}

fn addr_from_str(mut addr_str: &str) -> Result<Address, String> {
    if addr_str.starts_with("0x") {
        addr_str = &addr_str[2..];
    }
    Ok(try_s!(Address::from_str(addr_str)))
}

pub fn eth_coin_from_iguana_info(info: *mut lp::iguana_info, req: &Json) -> Result<EthCoin, String> {
    let info = unsafe { *info };
    let ticker = try_s! (unsafe {CStr::from_ptr (info.symbol.as_ptr())} .to_str()) .into();

    let mut urls: Vec<String> = try_s!(json::from_value(req["urls"].clone()));
    if urls.is_empty() {
        return ERR!("Enable request for ETH coin must have at least 1 node URL");
    }
    let mut rng = thread_rng();
    urls.as_mut_slice().shuffle(&mut rng);

    let swap_contract_address: Address = try_s!(json::from_value(req["swap_contract_address"].clone()));
    if swap_contract_address == Address::default() {
        return ERR!("swap_contract_address can't be zero address");
    }

    let key_pair: KeyPair = try_s!(KeyPair::from_secret_slice(unsafe { &lp::G.LP_privkey.bytes }));
    let my_address = key_pair.address();

    let transport = try_s!(Web3Transport::new(urls));
    let web3 = Web3::new(transport);

    let etomic = try_s!(unsafe { CStr::from_ptr(info.etomic.as_ptr()).to_str() } );
    let (coin_type, decimals) = if etomic == "0x0000000000000000000000000000000000000000" {
        (EthCoinType::Eth, 18)
    } else {
        let token_addr = try_s!(addr_from_str(etomic));
        let decimals = if info.decimals > 0 {
            info.decimals
        } else {
            try_s!(get_token_decimals(&web3, token_addr))
        };
        (EthCoinType::Erc20(token_addr), decimals)
    };

    let coin = EthCoinImpl {
        key_pair,
        my_address,
        coin_type,
        swap_contract_address,
        decimals,
        ticker,
        gas_station_url: try_s!(json::from_value(req["gas_station_url"].clone())),
        web3,
    };
    Ok(EthCoin(Arc::new(coin)))
}
