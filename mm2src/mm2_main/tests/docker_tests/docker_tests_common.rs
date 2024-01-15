pub use common::{block_on, now_ms, now_sec, wait_until_ms, wait_until_sec};
pub use mm2_number::MmNumber;
use mm2_rpc::data::legacy::BalanceResponse;
pub use mm2_test_helpers::for_tests::{check_my_swap_status, check_recent_swaps, check_stats_swap_status,
                                      enable_native, enable_native_bch, eth_jst_testnet_conf, eth_sepolia_conf,
                                      eth_testnet_conf, jst_sepolia_conf, mm_dump, MarketMakerIt, ETH_DEV_NODES,
                                      ETH_DEV_SWAP_CONTRACT, ETH_DEV_TOKEN_CONTRACT, MAKER_ERROR_EVENTS,
                                      MAKER_SUCCESS_EVENTS, TAKER_ERROR_EVENTS, TAKER_SUCCESS_EVENTS};

use bitcrypto::{dhash160, ChecksumType};
use chain::TransactionOutput;
use coins::eth::{eth_coin_from_conf_and_request, EthCoin};
use coins::qrc20::rpc_clients::for_tests::Qrc20NativeWalletOps;
use coins::qrc20::{qrc20_coin_with_priv_key, Qrc20ActivationParams, Qrc20Coin};
use coins::utxo::bch::{bch_coin_with_priv_key, BchActivationRequest, BchCoin};
use coins::utxo::qtum::{qtum_coin_with_priv_key, QtumBasedCoin, QtumCoin};
use coins::utxo::rpc_clients::{NativeClient, UtxoRpcClientEnum, UtxoRpcClientOps};
use coins::utxo::slp::{slp_genesis_output, SlpOutput, SlpToken};
use coins::utxo::utxo_common::send_outputs_from_my_address;
use coins::utxo::utxo_standard::{utxo_standard_coin_with_priv_key, UtxoStandardCoin};
use coins::utxo::{coin_daemon_data_dir, sat_from_big_decimal, zcash_params_path, UtxoActivationParams,
                  UtxoAddressFormat, UtxoCoinFields, UtxoCommonOps};
use coins::{CoinProtocol, ConfirmPaymentInput, MarketCoinOps, PrivKeyBuildPolicy, Transaction};
use crypto::privkey::key_pair_from_seed;
use crypto::Secp256k1Secret;
use ethereum_types::H160 as H160Eth;
use futures01::Future;
use http::StatusCode;
use keys::{Address, AddressHashEnum, KeyPair, NetworkPrefix as CashAddrPrefix};
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_number::BigDecimal;
use mm2_test_helpers::get_passphrase;
use mm2_test_helpers::structs::TransactionDetails;
use primitives::hash::{H160, H256};
use script::Builder;
use secp256k1::Secp256k1;
pub use secp256k1::{PublicKey, SecretKey};
use serde_json::{self as json, Value as Json};
pub use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
pub use std::thread;
use std::time::Duration;
use testcontainers::clients::Cli;
use testcontainers::core::WaitFor;
use testcontainers::{Container, GenericImage, RunnableImage};
use web3::transports::Http;
use web3::types::TransactionRequest;
use web3::Web3;

lazy_static! {
    static ref MY_COIN_LOCK: Mutex<()> = Mutex::new(());
    static ref MY_COIN1_LOCK: Mutex<()> = Mutex::new(());
    static ref QTUM_LOCK: Mutex<()> = Mutex::new(());
    static ref FOR_SLP_LOCK: Mutex<()> = Mutex::new(());
    pub static ref SLP_TOKEN_ID: Mutex<H256> = Mutex::new(H256::default());
    // Private keys supplied with 1000 SLP tokens on tests initialization.
    // Due to the SLP protocol limitations only 19 outputs (18 + change) can be sent in one transaction, which is sufficient for now though.
    // Supply more privkeys when 18 will be not enough.
    pub static ref SLP_TOKEN_OWNERS: Mutex<Vec<[u8; 32]>> = Mutex::new(Vec::with_capacity(18));
    static ref ETH_DISTRIBUTOR: EthCoin = eth_distributor();
    pub static ref MM_CTX: MmArc = MmCtxBuilder::new().into_mm_arc();
    pub static ref GETH_WEB3: Web3<Http> = Web3::new(Http::new(GETH_RPC_URL).unwrap());
}

pub static mut QICK_TOKEN_ADDRESS: Option<H160Eth> = None;
pub static mut QORTY_TOKEN_ADDRESS: Option<H160Eth> = None;
pub static mut QRC20_SWAP_CONTRACT_ADDRESS: Option<H160Eth> = None;
pub static mut QTUM_CONF_PATH: Option<PathBuf> = None;
/// The account supplied with ETH on Geth dev node creation
pub static mut GETH_ACCOUNT: H160Eth = H160Eth::zero();
/// ERC20 token address on Geth dev node
pub static mut GETH_ERC20_CONTRACT: H160Eth = H160Eth::zero();
/// Swap contract address on Geth dev node
pub static mut GETH_SWAP_CONTRACT: H160Eth = H160Eth::zero();
pub static GETH_RPC_URL: &str = "http://127.0.0.1:8545";

pub const UTXO_ASSET_DOCKER_IMAGE: &str = "docker.io/artempikulin/testblockchain";
pub const UTXO_ASSET_DOCKER_IMAGE_WITH_TAG: &str = "docker.io/artempikulin/testblockchain:multiarch";
pub const GETH_DOCKER_IMAGE: &str = "docker.io/ethereum/client-go";
pub const GETH_DOCKER_IMAGE_WITH_TAG: &str = "docker.io/ethereum/client-go:stable";

pub const QTUM_ADDRESS_LABEL: &str = "MM2_ADDRESS_LABEL";

/// Ticker of MYCOIN dockerized blockchain.
pub const MYCOIN: &str = "MYCOIN";
/// Ticker of MYCOIN1 dockerized blockchain.
pub const MYCOIN1: &str = "MYCOIN1";

pub const ERC20_TOKEN_BYTES: &str = "6080604052600860ff16600a0a633b9aca000260005534801561002157600080fd5b50600054600160003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002081905550610c69806100776000396000f3006080604052600436106100a4576000357c0100000000000000000000000000000000000000000000000000000000900463ffffffff16806306fdde03146100a9578063095ea7b31461013957806318160ddd1461019e57806323b872dd146101c9578063313ce5671461024e5780635a3b7e421461027f57806370a082311461030f57806395d89b4114610366578063a9059cbb146103f6578063dd62ed3e1461045b575b600080fd5b3480156100b557600080fd5b506100be6104d2565b6040518080602001828103825283818151815260200191508051906020019080838360005b838110156100fe5780820151818401526020810190506100e3565b50505050905090810190601f16801561012b5780820380516001836020036101000a031916815260200191505b509250505060405180910390f35b34801561014557600080fd5b50610184600480360381019080803573ffffffffffffffffffffffffffffffffffffffff1690602001909291908035906020019092919050505061050b565b604051808215151515815260200191505060405180910390f35b3480156101aa57600080fd5b506101b36106bb565b6040518082815260200191505060405180910390f35b3480156101d557600080fd5b50610234600480360381019080803573ffffffffffffffffffffffffffffffffffffffff169060200190929190803573ffffffffffffffffffffffffffffffffffffffff169060200190929190803590602001909291905050506106c1565b604051808215151515815260200191505060405180910390f35b34801561025a57600080fd5b506102636109a1565b604051808260ff1660ff16815260200191505060405180910390f35b34801561028b57600080fd5b506102946109a6565b6040518080602001828103825283818151815260200191508051906020019080838360005b838110156102d45780820151818401526020810190506102b9565b50505050905090810190601f1680156103015780820380516001836020036101000a031916815260200191505b509250505060405180910390f35b34801561031b57600080fd5b50610350600480360381019080803573ffffffffffffffffffffffffffffffffffffffff1690602001909291905050506109df565b6040518082815260200191505060405180910390f35b34801561037257600080fd5b5061037b6109f7565b6040518080602001828103825283818151815260200191508051906020019080838360005b838110156103bb5780820151818401526020810190506103a0565b50505050905090810190601f1680156103e85780820380516001836020036101000a031916815260200191505b509250505060405180910390f35b34801561040257600080fd5b50610441600480360381019080803573ffffffffffffffffffffffffffffffffffffffff16906020019092919080359060200190929190505050610a30565b604051808215151515815260200191505060405180910390f35b34801561046757600080fd5b506104bc600480360381019080803573ffffffffffffffffffffffffffffffffffffffff169060200190929190803573ffffffffffffffffffffffffffffffffffffffff169060200190929190505050610be1565b6040518082815260200191505060405180910390f35b6040805190810160405280600881526020017f515243205445535400000000000000000000000000000000000000000000000081525081565b60008260008173ffffffffffffffffffffffffffffffffffffffff161415151561053457600080fd5b60008314806105bf57506000600260003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002060008673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002054145b15156105ca57600080fd5b82600260003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002060008673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055508373ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925856040518082815260200191505060405180910390a3600191505092915050565b60005481565b60008360008173ffffffffffffffffffffffffffffffffffffffff16141515156106ea57600080fd5b8360008173ffffffffffffffffffffffffffffffffffffffff161415151561071157600080fd5b610797600260008873ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002060003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205485610c06565b600260008873ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002060003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002081905550610860600160008873ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205485610c06565b600160008873ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055506108ec600160008773ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205485610c1f565b600160008773ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055508473ffffffffffffffffffffffffffffffffffffffff168673ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef866040518082815260200191505060405180910390a36001925050509392505050565b600881565b6040805190810160405280600981526020017f546f6b656e20302e31000000000000000000000000000000000000000000000081525081565b60016020528060005260406000206000915090505481565b6040805190810160405280600381526020017f515443000000000000000000000000000000000000000000000000000000000081525081565b60008260008173ffffffffffffffffffffffffffffffffffffffff1614151515610a5957600080fd5b610aa2600160003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205484610c06565b600160003373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200190815260200160002081905550610b2e600160008673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205484610c1f565b600160008673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055508373ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef856040518082815260200191505060405180910390a3600191505092915050565b6002602052816000526040600020602052806000526040600020600091509150505481565b6000818310151515610c1457fe5b818303905092915050565b6000808284019050838110151515610c3357fe5b80915050929150505600a165627a7a723058207f2e5248b61b80365ea08a0f6d11ac0b47374c4dfd538de76bc2f19591bbbba40029";
pub const SWAP_CONTRACT_BYTES: &str = "608060405234801561001057600080fd5b50611437806100206000396000f3fe60806040526004361061004a5760003560e01c806302ed292b1461004f5780630716326d146100de578063152cf3af1461017b57806346fc0294146101f65780639b415b2a14610294575b600080fd5b34801561005b57600080fd5b506100dc600480360360a081101561007257600080fd5b81019080803590602001909291908035906020019092919080359060200190929190803573ffffffffffffffffffffffffffffffffffffffff169060200190929190803573ffffffffffffffffffffffffffffffffffffffff169060200190929190505050610339565b005b3480156100ea57600080fd5b506101176004803603602081101561010157600080fd5b8101908080359060200190929190505050610867565b60405180846bffffffffffffffffffffffff19166bffffffffffffffffffffffff191681526020018367ffffffffffffffff1667ffffffffffffffff16815260200182600381111561016557fe5b60ff168152602001935050505060405180910390f35b6101f46004803603608081101561019157600080fd5b8101908080359060200190929190803573ffffffffffffffffffffffffffffffffffffffff16906020019092919080356bffffffffffffffffffffffff19169060200190929190803567ffffffffffffffff1690602001909291905050506108bf565b005b34801561020257600080fd5b50610292600480360360a081101561021957600080fd5b81019080803590602001909291908035906020019092919080356bffffffffffffffffffffffff19169060200190929190803573ffffffffffffffffffffffffffffffffffffffff169060200190929190803573ffffffffffffffffffffffffffffffffffffffff169060200190929190505050610bd9565b005b610337600480360360c08110156102aa57600080fd5b810190808035906020019092919080359060200190929190803573ffffffffffffffffffffffffffffffffffffffff169060200190929190803573ffffffffffffffffffffffffffffffffffffffff16906020019092919080356bffffffffffffffffffffffff19169060200190929190803567ffffffffffffffff169060200190929190505050610fe2565b005b6001600381111561034657fe5b600080878152602001908152602001600020600001601c9054906101000a900460ff16600381111561037457fe5b1461037e57600080fd5b6000600333836003600288604051602001808281526020019150506040516020818303038152906040526040518082805190602001908083835b602083106103db57805182526020820191506020810190506020830392506103b8565b6001836020036101000a038019825116818451168082178552505050505050905001915050602060405180830381855afa15801561041d573d6000803e3d6000fd5b5050506040513d602081101561043257600080fd5b8101908080519060200190929190505050604051602001808281526020019150506040516020818303038152906040526040518082805190602001908083835b602083106104955780518252602082019150602081019050602083039250610472565b6001836020036101000a038019825116818451168082178552505050505050905001915050602060405180830381855afa1580156104d7573d6000803e3d6000fd5b5050506040515160601b8689604051602001808673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b81526014018573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b8152601401846bffffffffffffffffffffffff19166bffffffffffffffffffffffff191681526014018373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b8152601401828152602001955050505050506040516020818303038152906040526040518082805190602001908083835b602083106105fc57805182526020820191506020810190506020830392506105d9565b6001836020036101000a038019825116818451168082178552505050505050905001915050602060405180830381855afa15801561063e573d6000803e3d6000fd5b5050506040515160601b905060008087815260200190815260200160002060000160009054906101000a900460601b6bffffffffffffffffffffffff1916816bffffffffffffffffffffffff19161461069657600080fd5b6002600080888152602001908152602001600020600001601c6101000a81548160ff021916908360038111156106c857fe5b0217905550600073ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff16141561074e573373ffffffffffffffffffffffffffffffffffffffff166108fc869081150290604051600060405180830381858888f19350505050158015610748573d6000803e3d6000fd5b50610820565b60008390508073ffffffffffffffffffffffffffffffffffffffff1663a9059cbb33886040518363ffffffff1660e01b8152600401808373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200182815260200192505050602060405180830381600087803b1580156107da57600080fd5b505af11580156107ee573d6000803e3d6000fd5b505050506040513d602081101561080457600080fd5b810190808051906020019092919050505061081e57600080fd5b505b7f36c177bcb01c6d568244f05261e2946c8c977fa50822f3fa098c470770ee1f3e8685604051808381526020018281526020019250505060405180910390a1505050505050565b60006020528060005260406000206000915090508060000160009054906101000a900460601b908060000160149054906101000a900467ffffffffffffffff169080600001601c9054906101000a900460ff16905083565b600073ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff16141580156108fc5750600034115b801561094057506000600381111561091057fe5b600080868152602001908152602001600020600001601c9054906101000a900460ff16600381111561093e57fe5b145b61094957600080fd5b60006003843385600034604051602001808673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b81526014018573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b8152601401846bffffffffffffffffffffffff19166bffffffffffffffffffffffff191681526014018373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b8152601401828152602001955050505050506040516020818303038152906040526040518082805190602001908083835b60208310610a6c5780518252602082019150602081019050602083039250610a49565b6001836020036101000a038019825116818451168082178552505050505050905001915050602060405180830381855afa158015610aae573d6000803e3d6000fd5b5050506040515160601b90506040518060600160405280826bffffffffffffffffffffffff191681526020018367ffffffffffffffff16815260200160016003811115610af757fe5b81525060008087815260200190815260200160002060008201518160000160006101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908360601c021790555060208201518160000160146101000a81548167ffffffffffffffff021916908367ffffffffffffffff160217905550604082015181600001601c6101000a81548160ff02191690836003811115610b9357fe5b02179055509050507fccc9c05183599bd3135da606eaaf535daffe256e9de33c048014cffcccd4ad57856040518082815260200191505060405180910390a15050505050565b60016003811115610be657fe5b600080878152602001908152602001600020600001601c9054906101000a900460ff166003811115610c1457fe5b14610c1e57600080fd5b600060038233868689604051602001808673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b81526014018573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b8152601401846bffffffffffffffffffffffff19166bffffffffffffffffffffffff191681526014018373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b8152601401828152602001955050505050506040516020818303038152906040526040518082805190602001908083835b60208310610d405780518252602082019150602081019050602083039250610d1d565b6001836020036101000a038019825116818451168082178552505050505050905001915050602060405180830381855afa158015610d82573d6000803e3d6000fd5b5050506040515160601b905060008087815260200190815260200160002060000160009054906101000a900460601b6bffffffffffffffffffffffff1916816bffffffffffffffffffffffff1916148015610e10575060008087815260200190815260200160002060000160149054906101000a900467ffffffffffffffff1667ffffffffffffffff164210155b610e1957600080fd5b6003600080888152602001908152602001600020600001601c6101000a81548160ff02191690836003811115610e4b57fe5b0217905550600073ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff161415610ed1573373ffffffffffffffffffffffffffffffffffffffff166108fc869081150290604051600060405180830381858888f19350505050158015610ecb573d6000803e3d6000fd5b50610fa3565b60008390508073ffffffffffffffffffffffffffffffffffffffff1663a9059cbb33886040518363ffffffff1660e01b8152600401808373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200182815260200192505050602060405180830381600087803b158015610f5d57600080fd5b505af1158015610f71573d6000803e3d6000fd5b505050506040513d6020811015610f8757600080fd5b8101908080519060200190929190505050610fa157600080fd5b505b7f1797d500133f8e427eb9da9523aa4a25cb40f50ebc7dbda3c7c81778973f35ba866040518082815260200191505060405180910390a1505050505050565b600073ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff161415801561101f5750600085115b801561106357506000600381111561103357fe5b600080888152602001908152602001600020600001601c9054906101000a900460ff16600381111561106157fe5b145b61106c57600080fd5b60006003843385888a604051602001808673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b81526014018573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b8152601401846bffffffffffffffffffffffff19166bffffffffffffffffffffffff191681526014018373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1660601b8152601401828152602001955050505050506040516020818303038152906040526040518082805190602001908083835b6020831061118e578051825260208201915060208101905060208303925061116b565b6001836020036101000a038019825116818451168082178552505050505050905001915050602060405180830381855afa1580156111d0573d6000803e3d6000fd5b5050506040515160601b90506040518060600160405280826bffffffffffffffffffffffff191681526020018367ffffffffffffffff1681526020016001600381111561121957fe5b81525060008089815260200190815260200160002060008201518160000160006101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908360601c021790555060208201518160000160146101000a81548167ffffffffffffffff021916908367ffffffffffffffff160217905550604082015181600001601c6101000a81548160ff021916908360038111156112b557fe5b021790555090505060008590508073ffffffffffffffffffffffffffffffffffffffff166323b872dd33308a6040518463ffffffff1660e01b8152600401808473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020018373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020018281526020019350505050602060405180830381600087803b15801561137d57600080fd5b505af1158015611391573d6000803e3d6000fd5b505050506040513d60208110156113a757600080fd5b81019080805190602001909291905050506113c157600080fd5b7fccc9c05183599bd3135da606eaaf535daffe256e9de33c048014cffcccd4ad57886040518082815260200191505060405180910390a1505050505050505056fea265627a7a723158208c83db436905afce0b7be1012be64818c49323c12d451fe2ab6bce76ff6421c964736f6c63430005110032";

pub trait CoinDockerOps {
    fn rpc_client(&self) -> &UtxoRpcClientEnum;

    fn native_client(&self) -> &NativeClient {
        match self.rpc_client() {
            UtxoRpcClientEnum::Native(native) => native,
            _ => panic!("UtxoRpcClientEnum::Native is expected"),
        }
    }

    fn wait_ready(&self, expected_tx_version: i32) {
        let timeout = wait_until_ms(120000);
        loop {
            match self.rpc_client().get_block_count().wait() {
                Ok(n) => {
                    if n > 1 {
                        if let UtxoRpcClientEnum::Native(client) = self.rpc_client() {
                            let hash = client.get_block_hash(n).wait().unwrap();
                            let block = client.get_block(hash).wait().unwrap();
                            let coinbase = client.get_verbose_transaction(&block.tx[0]).wait().unwrap();
                            println!("Coinbase tx {:?} in block {}", coinbase, n);
                            if coinbase.version == expected_tx_version {
                                break;
                            }
                        }
                    }
                },
                Err(e) => log!("{:?}", e),
            }
            assert!(now_ms() < timeout, "Test timed out");
            thread::sleep(Duration::from_secs(1));
        }
    }
}

pub struct UtxoAssetDockerOps {
    #[allow(dead_code)]
    ctx: MmArc,
    coin: UtxoStandardCoin,
}

impl CoinDockerOps for UtxoAssetDockerOps {
    fn rpc_client(&self) -> &UtxoRpcClientEnum { &self.coin.as_ref().rpc_client }
}

impl UtxoAssetDockerOps {
    pub fn from_ticker(ticker: &str) -> UtxoAssetDockerOps {
        let conf = json!({"asset": ticker, "txfee": 1000, "network": "regtest"});
        let req = json!({"method":"enable"});
        let priv_key = Secp256k1Secret::from("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f");
        let ctx = MmCtxBuilder::new().into_mm_arc();
        let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

        let coin = block_on(utxo_standard_coin_with_priv_key(&ctx, ticker, &conf, &params, priv_key)).unwrap();
        UtxoAssetDockerOps { ctx, coin }
    }
}

pub struct BchDockerOps {
    #[allow(dead_code)]
    ctx: MmArc,
    coin: BchCoin,
}

// builds the EthCoin using the external dev Parity/OpenEthereum node
// the address belonging to the default passphrase has million of ETH that it can distribute to
// random privkeys generated in tests
pub fn eth_distributor() -> EthCoin {
    let req = json!({
        "method": "enable",
        "coin": "ETH",
        "urls": ETH_DEV_NODES,
        "swap_contract_address": ETH_DEV_SWAP_CONTRACT,
    });
    let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
    let keypair = key_pair_from_seed(&alice_passphrase).unwrap();
    let priv_key_policy = PrivKeyBuildPolicy::IguanaPrivKey(keypair.private().secret);
    block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ETH",
        &eth_testnet_conf(),
        &req,
        CoinProtocol::ETH,
        priv_key_policy,
    ))
    .unwrap()
}

// pass address without 0x prefix to this fn
pub fn _fill_eth(to_addr: &str) {
    ETH_DISTRIBUTOR
        .send_to_address(to_addr.parse().unwrap(), 1_000_000_000_000_000_000u64.into())
        .wait()
        .unwrap();
}

// Generates an ethereum coin in the sepolia network with the given seed
pub fn generate_eth_coin_with_seed(seed: &str) -> EthCoin {
    let req = json!({
        "method": "enable",
        "coin": "ETH",
        "urls": ETH_DEV_NODES,
        "swap_contract_address": ETH_DEV_SWAP_CONTRACT,
    });
    let keypair = key_pair_from_seed(seed).unwrap();
    let priv_key_policy = PrivKeyBuildPolicy::IguanaPrivKey(keypair.private().secret);
    block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ETH",
        &eth_testnet_conf(),
        &req,
        CoinProtocol::ETH,
        priv_key_policy,
    ))
    .unwrap()
}

pub fn generate_jst_with_seed(seed: &str) -> EthCoin {
    let req = json!({
        "method": "enable",
        "coin": "JST",
        "urls": ETH_DEV_NODES,
        "swap_contract_address": ETH_DEV_SWAP_CONTRACT,
    });

    let keypair = key_pair_from_seed(seed).unwrap();
    let priv_key_policy = PrivKeyBuildPolicy::IguanaPrivKey(keypair.private().secret);
    block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "JST",
        &eth_jst_testnet_conf(),
        &req,
        CoinProtocol::ERC20 {
            platform: "ETH".into(),
            contract_address: String::from(ETH_DEV_TOKEN_CONTRACT),
        },
        priv_key_policy,
    ))
    .unwrap()
}

impl BchDockerOps {
    pub fn from_ticker(ticker: &str) -> BchDockerOps {
        let conf = json!({"asset": ticker,"txfee":1000,"network": "regtest","txversion":4,"overwintered":1});
        let req = json!({"method":"enable", "bchd_urls": [], "allow_slp_unsafe_conf": true});
        let priv_key = Secp256k1Secret::from("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f");
        let ctx = MmCtxBuilder::new().into_mm_arc();
        let params = BchActivationRequest::from_legacy_req(&req).unwrap();

        let coin = block_on(bch_coin_with_priv_key(
            &ctx,
            ticker,
            &conf,
            params,
            CashAddrPrefix::SlpTest,
            priv_key,
        ))
        .unwrap();
        BchDockerOps { ctx, coin }
    }

    pub fn initialize_slp(&self) {
        fill_address(&self.coin, &self.coin.my_address().unwrap(), 100000.into(), 30);
        let mut slp_privkeys = vec![];

        let slp_genesis_op_ret = slp_genesis_output("ADEXSLP", "ADEXSLP", None, None, 8, None, 1000000_00000000);
        let slp_genesis = TransactionOutput {
            value: self.coin.as_ref().dust_amount,
            script_pubkey: Builder::build_p2pkh(&self.coin.my_public_key().unwrap().address_hash().into()).to_bytes(),
        };

        let mut bch_outputs = vec![slp_genesis_op_ret, slp_genesis];
        let mut slp_outputs = vec![];

        for _ in 0..18 {
            let key_pair = KeyPair::random_compressed();
            let address_hash = key_pair.public().address_hash();
            let address = Address {
                prefix: self.coin.as_ref().conf.pub_addr_prefix,
                t_addr_prefix: self.coin.as_ref().conf.pub_t_addr_prefix,
                hrp: None,
                hash: address_hash.into(),
                checksum_type: Default::default(),
                addr_format: Default::default(),
            };

            self.native_client()
                .import_address(&address.to_string(), &address.to_string(), false)
                .wait()
                .unwrap();

            let script_pubkey = Builder::build_p2pkh(&address_hash.into());

            bch_outputs.push(TransactionOutput {
                value: 1000_00000000,
                script_pubkey: script_pubkey.to_bytes(),
            });

            slp_outputs.push(SlpOutput {
                amount: 1000_00000000,
                script_pubkey: script_pubkey.to_bytes(),
            });
            slp_privkeys.push(*key_pair.private_ref());
        }

        let slp_genesis_tx = send_outputs_from_my_address(self.coin.clone(), bch_outputs)
            .wait()
            .unwrap();
        let confirm_payment_input = ConfirmPaymentInput {
            payment_tx: slp_genesis_tx.tx_hex(),
            confirmations: 1,
            requires_nota: false,
            wait_until: wait_until_sec(30),
            check_every: 1,
        };
        self.coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

        let adex_slp = SlpToken::new(
            8,
            "ADEXSLP".into(),
            slp_genesis_tx.tx_hash().as_slice().into(),
            self.coin.clone(),
            1,
        )
        .unwrap();

        let tx = block_on(adex_slp.send_slp_outputs(slp_outputs)).unwrap();
        let confirm_payment_input = ConfirmPaymentInput {
            payment_tx: tx.tx_hex(),
            confirmations: 1,
            requires_nota: false,
            wait_until: wait_until_sec(30),
            check_every: 1,
        };
        self.coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();
        *SLP_TOKEN_OWNERS.lock().unwrap() = slp_privkeys;
        *SLP_TOKEN_ID.lock().unwrap() = slp_genesis_tx.tx_hash().as_slice().into();
    }
}

impl CoinDockerOps for BchDockerOps {
    fn rpc_client(&self) -> &UtxoRpcClientEnum { &self.coin.as_ref().rpc_client }
}

pub struct DockerNode<'a> {
    #[allow(dead_code)]
    pub container: Container<'a, GenericImage>,
    #[allow(dead_code)]
    pub ticker: String,
    #[allow(dead_code)]
    pub port: u16,
}

pub fn random_secp256k1_secret() -> Secp256k1Secret {
    let priv_key = SecretKey::new(&mut rand6::thread_rng());
    Secp256k1Secret::from(*priv_key.as_ref())
}

pub fn utxo_asset_docker_node<'a>(docker: &'a Cli, ticker: &'static str, port: u16) -> DockerNode<'a> {
    let image = GenericImage::new(UTXO_ASSET_DOCKER_IMAGE, "multiarch")
        .with_volume(zcash_params_path().display().to_string(), "/root/.zcash-params")
        .with_env_var("CLIENTS", "2")
        .with_env_var("CHAIN", ticker)
        .with_env_var("TEST_ADDY", "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF")
        .with_env_var("TEST_WIF", "UqqW7f766rADem9heD8vSBvvrdfJb3zg5r8du9rJxPtccjWf7RG9")
        .with_env_var(
            "TEST_PUBKEY",
            "021607076d7a2cb148d542fb9644c04ffc22d2cca752f80755a0402a24c567b17a",
        )
        .with_env_var("DAEMON_URL", "http://test:test@127.0.0.1:7000")
        .with_env_var("COIN", "Komodo")
        .with_env_var("COIN_RPC_PORT", port.to_string())
        .with_wait_for(WaitFor::message_on_stdout("config is ready"));
    let image = RunnableImage::from(image).with_mapped_port((port, port));
    let container = docker.run(image);
    let mut conf_path = coin_daemon_data_dir(ticker, true);
    std::fs::create_dir_all(&conf_path).unwrap();
    conf_path.push(format!("{}.conf", ticker));
    Command::new("docker")
        .arg("cp")
        .arg(format!("{}:/data/node_0/{}.conf", container.id(), ticker))
        .arg(&conf_path)
        .status()
        .expect("Failed to execute docker command");
    let timeout = wait_until_ms(3000);
    loop {
        if conf_path.exists() {
            break;
        };
        assert!(now_ms() < timeout, "Test timed out");
    }
    DockerNode {
        container,
        ticker: ticker.into(),
        port,
    }
}

pub fn geth_docker_node<'a>(docker: &'a Cli, ticker: &'static str, port: u16) -> DockerNode<'a> {
    let image = GenericImage::new(GETH_DOCKER_IMAGE, "stable");
    let args = vec!["--dev".into(), "--http".into(), "--http.addr=0.0.0.0".into()];
    let image = RunnableImage::from((image, args)).with_mapped_port((port, port));
    let container = docker.run(image);
    DockerNode {
        container,
        ticker: ticker.into(),
        port,
    }
}

pub fn rmd160_from_priv(privkey: Secp256k1Secret) -> H160 {
    let secret = SecretKey::from_slice(privkey.as_slice()).unwrap();
    let public = PublicKey::from_secret_key(&Secp256k1::new(), &secret);
    dhash160(&public.serialize())
}

pub fn get_prefilled_slp_privkey() -> [u8; 32] { SLP_TOKEN_OWNERS.lock().unwrap().remove(0) }

pub fn get_slp_token_id() -> String { hex::encode(SLP_TOKEN_ID.lock().unwrap().as_slice()) }

pub fn import_address<T>(coin: &T)
where
    T: MarketCoinOps + AsRef<UtxoCoinFields>,
{
    match coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(ref native) => {
            let my_address = coin.my_address().unwrap();
            native.import_address(&my_address, &my_address, false).wait().unwrap()
        },
        UtxoRpcClientEnum::Electrum(_) => panic!("Expected NativeClient"),
    }
}

/// Build `Qrc20Coin` from ticker and privkey without filling the balance.
pub fn qrc20_coin_from_privkey(ticker: &str, priv_key: Secp256k1Secret) -> (MmArc, Qrc20Coin) {
    let (contract_address, swap_contract_address) = unsafe {
        let contract_address = match ticker {
            "QICK" => QICK_TOKEN_ADDRESS.expect("QICK_TOKEN_ADDRESS must be set already"),
            "QORTY" => QORTY_TOKEN_ADDRESS.expect("QORTY_TOKEN_ADDRESS must be set already"),
            _ => panic!("Expected QICK or QORTY ticker"),
        };
        (
            contract_address,
            QRC20_SWAP_CONTRACT_ADDRESS.expect("QRC20_SWAP_CONTRACT_ADDRESS must be set already"),
        )
    };
    let platform = "QTUM";
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    let conf = json!({
        "coin":ticker,
        "decimals": 8,
        "required_confirmations":0,
        "pubtype":120,
        "p2shtype":110,
        "wiftype":128,
        "mm2":1,
        "mature_confirmations":500,
        "network":"regtest",
        "confpath": confpath,
        "dust": 72800,
    });
    let req = json!({
        "method": "enable",
        "swap_contract_address": format!("{:#02x}", swap_contract_address),
    });
    let params = Qrc20ActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(qrc20_coin_with_priv_key(
        &ctx,
        ticker,
        platform,
        &conf,
        &params,
        priv_key,
        contract_address,
    ))
    .unwrap();

    import_address(&coin);
    (ctx, coin)
}

fn qrc20_coin_conf_item(ticker: &str) -> Json {
    let contract_address = unsafe {
        match ticker {
            "QICK" => QICK_TOKEN_ADDRESS.expect("QICK_TOKEN_ADDRESS must be set already"),
            "QORTY" => QORTY_TOKEN_ADDRESS.expect("QORTY_TOKEN_ADDRESS must be set already"),
            _ => panic!("Expected either QICK or QORTY ticker, found {}", ticker),
        }
    };
    let contract_address = format!("{:#02x}", contract_address);

    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    json!({
        "coin":ticker,
        "required_confirmations":1,
        "pubtype":120,
        "p2shtype":110,
        "wiftype":128,
        "mature_confirmations":500,
        "confpath":confpath,
        "network":"regtest",
        "protocol":{"type":"QRC20","protocol_data":{"platform":"QTUM","contract_address":contract_address}}})
}

/// Build asset `UtxoStandardCoin` from ticker and privkey without filling the balance.
pub fn utxo_coin_from_privkey(ticker: &str, priv_key: Secp256k1Secret) -> (MmArc, UtxoStandardCoin) {
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let conf = json!({"asset":ticker,"txversion":4,"overwintered":1,"txfee":1000,"network":"regtest"});
    let req = json!({"method":"enable"});
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(utxo_standard_coin_with_priv_key(&ctx, ticker, &conf, &params, priv_key)).unwrap();
    import_address(&coin);
    (ctx, coin)
}

/// Create a UTXO coin for the given privkey and fill it's address with the specified balance.
pub fn generate_utxo_coin_with_privkey(ticker: &str, balance: BigDecimal, priv_key: Secp256k1Secret) {
    let (_, coin) = utxo_coin_from_privkey(ticker, priv_key);
    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, balance, timeout);
}

/// Generate random privkey, create a UTXO coin and fill it's address with the specified balance.
pub fn generate_utxo_coin_with_random_privkey(
    ticker: &str,
    balance: BigDecimal,
) -> (MmArc, UtxoStandardCoin, Secp256k1Secret) {
    let priv_key = random_secp256k1_secret();
    let (ctx, coin) = utxo_coin_from_privkey(ticker, priv_key);
    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, balance, timeout);
    (ctx, coin, priv_key)
}

/// Get only one address assigned the specified label.
pub fn get_address_by_label<T>(coin: T, label: &str) -> String
where
    T: AsRef<UtxoCoinFields>,
{
    let native = match coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(ref native) => native,
        UtxoRpcClientEnum::Electrum(_) => panic!("NativeClient expected"),
    };
    let mut addresses = native
        .get_addresses_by_label(label)
        .wait()
        .expect("!getaddressesbylabel")
        .into_iter();
    match addresses.next() {
        Some((addr, _purpose)) if addresses.next().is_none() => addr,
        Some(_) => panic!("Expected only one address by {:?}", label),
        None => panic!("Expected one address by {:?}", label),
    }
}

pub fn fill_qrc20_address(coin: &Qrc20Coin, amount: BigDecimal, timeout: u64) {
    // prevent concurrent fill since daemon RPC returns errors if send_to_address
    // is called concurrently (insufficient funds) and it also may return other errors
    // if previous transaction is not confirmed yet
    let _lock = QTUM_LOCK.lock().unwrap();
    let timeout = wait_until_sec(timeout);
    let client = match coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(ref client) => client,
        UtxoRpcClientEnum::Electrum(_) => panic!("Expected NativeClient"),
    };

    let from_addr = get_address_by_label(coin, QTUM_ADDRESS_LABEL);
    let to_addr = coin.my_addr_as_contract_addr().unwrap();
    let satoshis = sat_from_big_decimal(&amount, coin.as_ref().decimals).expect("!sat_from_big_decimal");

    let hash = client
        .transfer_tokens(
            &coin.contract_address,
            &from_addr,
            to_addr,
            satoshis.into(),
            coin.as_ref().decimals,
        )
        .wait()
        .expect("!transfer_tokens")
        .txid;

    let tx_bytes = client.get_transaction_bytes(&hash).wait().unwrap();
    log!("{:02x}", tx_bytes);
    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: tx_bytes.0,
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();
}

/// Generate random privkey, create a QRC20 coin and fill it's address with the specified balance.
pub fn generate_qrc20_coin_with_random_privkey(
    ticker: &str,
    qtum_balance: BigDecimal,
    qrc20_balance: BigDecimal,
) -> (MmArc, Qrc20Coin, Secp256k1Secret) {
    let priv_key = random_secp256k1_secret();
    let (ctx, coin) = qrc20_coin_from_privkey(ticker, priv_key);

    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, qtum_balance, timeout);
    fill_qrc20_address(&coin, qrc20_balance, timeout);
    (ctx, coin, priv_key)
}

pub fn generate_qtum_coin_with_random_privkey(
    ticker: &str,
    balance: BigDecimal,
    txfee: Option<u64>,
) -> (MmArc, QtumCoin, [u8; 32]) {
    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    let conf = json!({
        "coin":ticker,
        "decimals":8,
        "required_confirmations":0,
        "pubtype":120,
        "p2shtype": 110,
        "wiftype":128,
        "txfee": txfee,
        "txfee_volatility_percent":0.1,
        "mm2":1,
        "mature_confirmations":500,
        "network":"regtest",
        "confpath": confpath,
        "dust": 72800,
    });
    let req = json!({"method": "enable"});
    let priv_key = random_secp256k1_secret();
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(&ctx, "QTUM", &conf, &params, priv_key)).unwrap();

    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, balance, timeout);
    (ctx, coin, priv_key.take())
}

pub fn generate_segwit_qtum_coin_with_random_privkey(
    ticker: &str,
    balance: BigDecimal,
    txfee: Option<u64>,
) -> (MmArc, QtumCoin, Secp256k1Secret) {
    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    let conf = json!({
        "coin":ticker,
        "decimals":8,
        "required_confirmations":0,
        "pubtype":120,
        "p2shtype": 110,
        "wiftype":128,
        "segwit":true,
        "txfee": txfee,
        "txfee_volatility_percent":0.1,
        "mm2":1,
        "mature_confirmations":500,
        "network":"regtest",
        "confpath": confpath,
        "dust": 72800,
        "bech32_hrp":"qcrt",
        "address_format": {
            "format": "segwit",
        },
    });
    let req = json!({"method": "enable"});
    let priv_key = random_secp256k1_secret();
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(&ctx, "QTUM", &conf, &params, priv_key)).unwrap();

    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, balance, timeout);
    (ctx, coin, priv_key)
}

pub fn fill_address<T>(coin: &T, address: &str, amount: BigDecimal, timeout: u64)
where
    T: MarketCoinOps + AsRef<UtxoCoinFields>,
{
    // prevent concurrent fill since daemon RPC returns errors if send_to_address
    // is called concurrently (insufficient funds) and it also may return other errors
    // if previous transaction is not confirmed yet
    let mutex = match coin.ticker() {
        "MYCOIN" => &*MY_COIN_LOCK,
        "MYCOIN1" => &*MY_COIN1_LOCK,
        "QTUM" | "QICK" | "QORTY" => &*QTUM_LOCK,
        "FORSLP" => &*FOR_SLP_LOCK,
        ticker => panic!("Unknown ticker {}", ticker),
    };
    let _lock = mutex.lock().unwrap();
    let timeout = wait_until_sec(timeout);

    if let UtxoRpcClientEnum::Native(client) = &coin.as_ref().rpc_client {
        client.import_address(address, address, false).wait().unwrap();
        let hash = client.send_to_address(address, &amount).wait().unwrap();
        let tx_bytes = client.get_transaction_bytes(&hash).wait().unwrap();
        let confirm_payment_input = ConfirmPaymentInput {
            payment_tx: tx_bytes.clone().0,
            confirmations: 1,
            requires_nota: false,
            wait_until: timeout,
            check_every: 1,
        };
        coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();
        log!("{:02x}", tx_bytes);
        loop {
            let unspents = client
                .list_unspent_impl(0, std::i32::MAX, vec![address.to_string()])
                .wait()
                .unwrap();
            if !unspents.is_empty() {
                break;
            }
            assert!(now_sec() < timeout, "Test timed out");
            thread::sleep(Duration::from_secs(1));
        }
    };
}

/// Wait for the `estimatesmartfee` returns no errors.
pub fn wait_for_estimate_smart_fee(timeout: u64) -> Result<(), String> {
    enum EstimateSmartFeeState {
        Idle,
        Ok,
        NotAvailable,
    }
    lazy_static! {
        static ref LOCK: Mutex<EstimateSmartFeeState> = Mutex::new(EstimateSmartFeeState::Idle);
    }

    let state = &mut *LOCK.lock().unwrap();
    match state {
        EstimateSmartFeeState::Ok => return Ok(()),
        EstimateSmartFeeState::NotAvailable => return ERR!("estimatesmartfee not available"),
        EstimateSmartFeeState::Idle => log!("Start wait_for_estimate_smart_fee"),
    }

    let priv_key = random_secp256k1_secret();
    let (_ctx, coin) = qrc20_coin_from_privkey("QICK", priv_key);
    let timeout = wait_until_sec(timeout);
    let client = match coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(ref client) => client,
        UtxoRpcClientEnum::Electrum(_) => panic!("Expected NativeClient"),
    };
    while now_sec() < timeout {
        if let Ok(res) = client.estimate_smart_fee(&None, 1).wait() {
            if res.errors.is_empty() {
                *state = EstimateSmartFeeState::Ok;
                return Ok(());
            }
        }
        thread::sleep(Duration::from_secs(1));
    }

    *state = EstimateSmartFeeState::NotAvailable;
    ERR!("Waited too long for estimate_smart_fee to work")
}

pub async fn enable_qrc20_native(mm: &MarketMakerIt, coin: &str) -> Json {
    let swap_contract_address =
        unsafe { QRC20_SWAP_CONTRACT_ADDRESS.expect("QRC20_SWAP_CONTRACT_ADDRESS must be set already") };

    let native = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "enable",
            "coin": coin,
            "swap_contract_address": format!("{:#02x}", swap_contract_address),
            "mm2": 1,
        }))
        .await
        .unwrap();
    assert_eq!(native.0, StatusCode::OK, "'enable' failed: {}", native.1);
    json::from_str(&native.1).unwrap()
}

pub fn trade_base_rel((base, rel): (&str, &str)) {
    /// Generate a wallet with the random private key and fill the wallet with Qtum (required by gas_fee) and specified in `ticker` coin.
    fn generate_and_fill_priv_key(ticker: &str) -> Secp256k1Secret {
        let timeout = 30; // timeout if test takes more than 30 seconds to run

        match ticker {
            "QTUM" => {
                //Segwit QTUM
                wait_for_estimate_smart_fee(timeout).expect("!wait_for_estimate_smart_fee");
                let (_ctx, _coin, priv_key) = generate_segwit_qtum_coin_with_random_privkey("QTUM", 10.into(), Some(0));

                priv_key
            },
            "QICK" | "QORTY" => {
                let priv_key = random_secp256k1_secret();
                let (_ctx, coin) = qrc20_coin_from_privkey(ticker, priv_key);
                let my_address = coin.my_address().expect("!my_address");
                fill_address(&coin, &my_address, 10.into(), timeout);
                fill_qrc20_address(&coin, 10.into(), timeout);

                priv_key
            },
            "MYCOIN" | "MYCOIN1" => {
                let priv_key = random_secp256k1_secret();
                let (_ctx, coin) = utxo_coin_from_privkey(ticker, priv_key);
                let my_address = coin.my_address().expect("!my_address");
                fill_address(&coin, &my_address, 10.into(), timeout);
                // also fill the Qtum
                let (_ctx, coin) = qrc20_coin_from_privkey("QICK", priv_key);
                let my_address = coin.my_address().expect("!my_address");
                fill_address(&coin, &my_address, 10.into(), timeout);

                priv_key
            },
            "ADEXSLP" | "FORSLP" => Secp256k1Secret::from(get_prefilled_slp_privkey()),
            _ => panic!("Expected either QICK or QORTY or MYCOIN or MYCOIN1, found {}", ticker),
        }
    }

    let bob_priv_key = generate_and_fill_priv_key(base);
    let alice_priv_key = generate_and_fill_priv_key(rel);

    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    let coins = json! ([
        qrc20_coin_conf_item("QICK"),
        qrc20_coin_conf_item("QORTY"),
        {"coin":"MYCOIN","asset":"MYCOIN","required_confirmations":0,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        {"coin":"MYCOIN1","asset":"MYCOIN1","required_confirmations":0,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        {"coin":"QTUM","asset":"QTUM","required_confirmations":0,"decimals":8,"pubtype":120,"p2shtype":110,"wiftype":128,"segwit":true,"txfee":0,"txfee_volatility_percent":0.1,
        "mm2":1,"network":"regtest","confpath":confpath,"protocol":{"type":"UTXO"},"bech32_hrp":"qcrt","address_format":{"format":"segwit"}},
        {"coin":"FORSLP","asset":"FORSLP","required_confirmations":0,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"BCH","protocol_data":{"slp_prefix":"slptest"}}},
        {"coin":"ADEXSLP","protocol":{"type":"SLPTOKEN","protocol_data":{"decimals":8,"token_id":get_slp_token_id(),"platform":"FORSLP"}}}
    ]);
    let mut mm_bob = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    block_on(mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))).unwrap();

    let mut mm_alice = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    block_on(mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))).unwrap();

    log!("{:?}", block_on(enable_qrc20_native(&mm_bob, "QICK")));
    log!("{:?}", block_on(enable_qrc20_native(&mm_bob, "QORTY")));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "QTUM", &[], None)));
    log!("{:?}", block_on(enable_native_bch(&mm_bob, "FORSLP", &[])));
    log!("{:?}", block_on(enable_native(&mm_bob, "ADEXSLP", &[], None)));

    log!("{:?}", block_on(enable_qrc20_native(&mm_alice, "QICK")));
    log!("{:?}", block_on(enable_qrc20_native(&mm_alice, "QORTY")));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "QTUM", &[], None)));
    log!("{:?}", block_on(enable_native_bch(&mm_alice, "FORSLP", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "ADEXSLP", &[], None)));
    let rc = block_on(mm_bob.rpc(&json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": base,
        "rel": rel,
        "price": 1,
        "volume": "3",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    thread::sleep(Duration::from_secs(1));

    log!("Issue alice {}/{} buy request", base, rel);
    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": base,
        "rel": rel,
        "price": 1,
        "volume": "2",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);
    let buy_json: Json = serde_json::from_str(&rc.1).unwrap();
    let uuid = buy_json["result"]["uuid"].as_str().unwrap().to_owned();

    // ensure the swaps are started
    block_on(mm_bob.wait_for_log(22., |log| {
        log.contains(&format!("Entering the maker_swap_loop {}/{}", base, rel))
    }))
    .unwrap();
    block_on(mm_alice.wait_for_log(22., |log| {
        log.contains(&format!("Entering the taker_swap_loop {}/{}", base, rel))
    }))
    .unwrap();

    // ensure the swaps are finished
    block_on(mm_bob.wait_for_log(600., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))).unwrap();
    block_on(mm_alice.wait_for_log(600., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))).unwrap();

    log!("Checking alice/taker status..");
    block_on(check_my_swap_status(
        &mm_alice,
        &uuid,
        "2".parse().unwrap(),
        "2".parse().unwrap(),
    ));

    log!("Checking bob/maker status..");
    block_on(check_my_swap_status(
        &mm_bob,
        &uuid,
        "2".parse().unwrap(),
        "2".parse().unwrap(),
    ));

    log!("Waiting 3 seconds for nodes to broadcast their swaps data..");
    thread::sleep(Duration::from_secs(3));

    log!("Checking alice status..");
    block_on(check_stats_swap_status(&mm_alice, &uuid));

    log!("Checking bob status..");
    block_on(check_stats_swap_status(&mm_bob, &uuid));

    log!("Checking alice recent swaps..");
    block_on(check_recent_swaps(&mm_alice, 1));
    log!("Checking bob recent swaps..");
    block_on(check_recent_swaps(&mm_bob, 1));

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

pub fn slp_supplied_node() -> MarketMakerIt {
    let coins = json! ([
        {"coin":"FORSLP","asset":"FORSLP","required_confirmations":0,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"BCH","protocol_data":{"slp_prefix":"slptest"}}},
        {"coin":"ADEXSLP","protocol":{"type":"SLPTOKEN","protocol_data":{"decimals":8,"token_id":get_slp_token_id(),"platform":"FORSLP"}}}
    ]);

    let priv_key = get_prefilled_slp_privkey();
    let mm = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();

    mm
}

pub fn _solana_supplied_node() -> MarketMakerIt {
    let coins = json! ([
        {"coin": "SOL-DEVNET","name": "solana","fname": "Solana","rpcport": 80,"mm2": 1,"required_confirmations": 1,"avg_blocktime": 0.25,"protocol": {"type": "SOLANA"}},
        {"coin":"USDC-SOL-DEVNET","protocol":{"type":"SPLTOKEN","protocol_data":{"decimals":6,"token_contract_address":"4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU","platform":"SOL-DEVNET"}},"mm2": 1},
        {"coin":"ADEX-SOL-DEVNET","protocol":{"type":"SPLTOKEN","protocol_data":{"decimals":9,"token_contract_address":"5tSm6PqMosy1rz1AqV3kD28yYT5XqZW3QYmZommuFiPJ","platform":"SOL-DEVNET"}},"mm2": 1},
    ]);

    MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap()
}

pub fn get_balance(mm: &MarketMakerIt, coin: &str) -> BalanceResponse {
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "my_balance",
        "coin": coin,
    })))
    .unwrap();
    assert_eq!(rc.0, StatusCode::OK, "my_balance request failed {}", rc.1);
    json::from_str(&rc.1).unwrap()
}

pub fn utxo_burn_address() -> Address {
    Address {
        prefix: 60,
        hash: AddressHashEnum::default_address_hash(),
        t_addr_prefix: 0,
        checksum_type: ChecksumType::DSHA256,
        hrp: None,
        addr_format: UtxoAddressFormat::Standard,
    }
}

pub fn withdraw_max_and_send_v1(mm: &MarketMakerIt, coin: &str, to: &str) -> TransactionDetails {
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": coin,
        "max": true,
        "to": to,
    })))
    .unwrap();
    assert_eq!(rc.0, StatusCode::OK, "withdraw request failed {}", rc.1);
    let tx_details: TransactionDetails = json::from_str(&rc.1).unwrap();

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "send_raw_transaction",
        "tx_hex": tx_details.tx_hex,
        "coin": coin,
    })))
    .unwrap();
    assert_eq!(rc.0, StatusCode::OK, "send_raw_transaction request failed {}", rc.1);

    tx_details
}

pub fn init_geth_node() {
    unsafe {
        let accounts = block_on(GETH_WEB3.eth().accounts()).unwrap();
        GETH_ACCOUNT = accounts[0];
        println!("GETH ACCOUNT {:?}", GETH_ACCOUNT);

        let tx_request_deploy_erc20 = TransactionRequest {
            from: GETH_ACCOUNT,
            to: None,
            gas: None,
            gas_price: None,
            value: None,
            data: Some(hex::decode(ERC20_TOKEN_BYTES).unwrap().into()),
            nonce: None,
            condition: None,
            transaction_type: None,
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
        };

        let deploy_erc20_tx_hash = block_on(GETH_WEB3.eth().send_transaction(tx_request_deploy_erc20)).unwrap();
        println!("Sent ERC20 deploy transaction {:?}", deploy_erc20_tx_hash);

        loop {
            let deploy_tx_receipt = block_on(GETH_WEB3.eth().transaction_receipt(deploy_erc20_tx_hash)).unwrap();

            if let Some(receipt) = deploy_tx_receipt {
                GETH_ERC20_CONTRACT = receipt.contract_address.unwrap();
                println!("GETH_ERC20_CONTRACT {:?}", GETH_ERC20_CONTRACT);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        let tx_request_deploy_swap_contract = TransactionRequest {
            from: GETH_ACCOUNT,
            to: None,
            gas: None,
            gas_price: None,
            value: None,
            data: Some(hex::decode(SWAP_CONTRACT_BYTES).unwrap().into()),
            nonce: None,
            condition: None,
            transaction_type: None,
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
        };
        let deploy_swap_tx_hash = block_on(GETH_WEB3.eth().send_transaction(tx_request_deploy_swap_contract)).unwrap();
        println!("Sent deploy swap contract transaction {:?}", deploy_swap_tx_hash);

        loop {
            let deploy_swap_tx_receipt = block_on(GETH_WEB3.eth().transaction_receipt(deploy_swap_tx_hash)).unwrap();

            if let Some(receipt) = deploy_swap_tx_receipt {
                GETH_SWAP_CONTRACT = receipt.contract_address.unwrap();
                println!("GETH_SWAP_CONTRACT {:?}", GETH_SWAP_CONTRACT);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}
