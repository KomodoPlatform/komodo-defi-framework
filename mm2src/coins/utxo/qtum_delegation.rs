use crate::qrc20::{ContractCallOutput, GenerateQrc20TxResult, Qrc20FeeDetails, QRC20_DUST, QRC20_GAS_PRICE_DEFAULT};
use crate::utxo::qtum::QtumCoin;
use crate::utxo::utxo_common::UtxoTxBuilder;
use crate::utxo::{sign_tx, utxo_common, UtxoCommonOps};
use crate::{DelegationError, DelegationResult, MarketCoinOps, TransactionDetails, TransactionType};
use common::mm_error::prelude::{MapMmError, MapToMmResult};
use common::now_ms;
use ethabi::Contract;
use ethereum_types::H160;
use futures::compat::Future01CompatExt;
use script::Builder as ScriptBuilder;
use serialization::serialize;
use std::str::FromStr;

pub const QTUM_DELEGATION_STANDARD_FEE: u64 = 10;
pub const QTUM_LOWER_BOUND_DELEGATION_AMOUNT: f64 = 100.0;
pub const QRC20_GAS_LIMIT_DELEGATION: u64 = 2_250_000;
pub const QTUM_ADD_DELEGATION_TOPIC: &str = "a23803f3b2b56e71f2921c22b23c32ef596a439dbe03f7250e6b58a30eb910b5";
pub const QTUM_REMOVE_DELEGATION_TOPIC: &str = "7fe28d2d0b16cf95b5ea93f4305f89133b3892543e616381a1336fc1e7a01fa0";
const QTUM_DELEGATE_CONTRACT_ABI: &str = r#"[{"anonymous":false,"inputs":[{"indexed":true,"internalType":"address","name":"_staker","type":"address"},{"indexed":true,"internalType":"address","name":"_delegate","type":"address"},{"indexed":false,"internalType":"uint8","name":"fee","type":"uint8"},{"indexed":false,"internalType":"uint256","name":"blockHeight","type":"uint256"},{"indexed":false,"internalType":"bytes","name":"PoD","type":"bytes"}],"name":"AddDelegation","type":"event"},{"anonymous":false,"inputs":[{"indexed":true,"internalType":"address","name":"_staker","type":"address"},{"indexed":true,"internalType":"address","name":"_delegate","type":"address"}],"name":"RemoveDelegation","type":"event"},{"constant":false,"inputs":[{"internalType":"address","name":"_staker","type":"address"},{"internalType":"uint8","name":"_fee","type":"uint8"},{"internalType":"bytes","name":"_PoD","type":"bytes"}],"name":"addDelegation","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"internalType":"address","name":"","type":"address"}],"name":"delegations","outputs":[{"internalType":"address","name":"staker","type":"address"},{"internalType":"uint8","name":"fee","type":"uint8"},{"internalType":"uint256","name":"blockHeight","type":"uint256"},{"internalType":"bytes","name":"PoD","type":"bytes"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[],"name":"removeDelegation","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"}]"#;

lazy_static! {
    pub static ref QTUM_DELEGATE_CONTRACT: Contract = Contract::load(QTUM_DELEGATE_CONTRACT_ABI.as_bytes()).unwrap();
    pub static ref QTUM_DELEGATE_CONTRACT_ADDRESS: H160 =
        H160::from_str("0000000000000000000000000000000000000086").unwrap();
}

// Qtum Delegate Transaction - lock UTXO mutex before calling this function
pub async fn generate_delegation_transaction(
    coin: &QtumCoin,
    contract_outputs: Vec<ContractCallOutput>,
    to_address: String,
    gas_limit: u64,
    transaction_type: TransactionType,
) -> DelegationResult {
    let utxo = coin.as_ref();
    let (unspents, _) = coin.ordered_mature_unspents(&utxo.my_address).await?;
    let mut gas_fee = 0;
    let mut outputs = Vec::with_capacity(contract_outputs.len());
    for output in contract_outputs {
        gas_fee += output.gas_limit * output.gas_price;
        outputs.push(output.into());
    }

    let (unsigned, data) = UtxoTxBuilder::new(coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs)
        .with_gas_fee(gas_fee)
        .with_dust(QRC20_DUST)
        .build()
        .await
        .mm_err(|gen_tx_error| {
            DelegationError::from_generate_tx_error(gen_tx_error, coin.ticker().to_string(), utxo.decimals)
        })?;

    let prev_script = ScriptBuilder::build_p2pkh(&utxo.my_address.hash);
    let signed = sign_tx(
        unsigned,
        &utxo.key_pair,
        prev_script,
        utxo.conf.signature_version,
        utxo.conf.fork_id,
    )
    .map_to_mm(DelegationError::InternalError)?;

    let miner_fee = data.fee_amount + data.unused_change.unwrap_or_default();
    let generated_tx = GenerateQrc20TxResult {
        signed,
        miner_fee,
        gas_fee,
    };
    let fee_details = Qrc20FeeDetails {
        // QRC20 fees are paid in base platform currency (in particular Qtum)
        coin: coin.ticker().to_string(),
        miner_fee: utxo_common::big_decimal_from_sat(generated_tx.miner_fee as i64, utxo.decimals),
        gas_limit,
        gas_price: QRC20_GAS_PRICE_DEFAULT,
        total_gas_fee: utxo_common::big_decimal_from_sat(generated_tx.gas_fee as i64, utxo.decimals),
    };
    let my_address = coin.my_address().map_to_mm(DelegationError::InternalError)?;

    // Delegation transaction always transfer all the UTXO's to ourselves
    let qtum_amount = coin.my_spendable_balance().compat().await?;
    let received_by_me = qtum_amount.clone();
    let my_balance_change = &received_by_me - &qtum_amount;

    Ok(TransactionDetails {
        tx_hex: serialize(&generated_tx.signed).into(),
        tx_hash: generated_tx.signed.hash().reversed().to_vec().into(),
        from: vec![my_address],
        to: vec![to_address],
        total_amount: qtum_amount.clone(),
        spent_by_me: qtum_amount,
        received_by_me,
        my_balance_change,
        block_height: 0,
        timestamp: now_ms() / 1000,
        fee_details: Some(fee_details.into()),
        coin: coin.ticker().to_string(),
        internal_id: vec![].into(),
        kmd_rewards: None,
        transaction_type,
    })
}
