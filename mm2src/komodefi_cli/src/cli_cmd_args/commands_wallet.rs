use anyhow::{anyhow, bail, Error, Result};
use clap::{Args, Subcommand, ValueEnum};
use hex::FromHexError;
use rpc::v1::types::Bytes as BytesJson;
use std::mem::take;
use std::num::NonZeroUsize;
use std::str::FromStr;
use std::{f64, u64};

use common::log::error;
use common::PagingOptionsEnum;
use mm2_number::BigDecimal;
use mm2_rpc::data::legacy::BalanceRequest;
use mm2_rpc::data::version2::GetRawTransactionRequest;

use crate::rpc_data::wallet::{CashAddressNetwork as RpcCashAddressNetwork,
                              ConvertAddressFormat as RpcConvertAddressFormat, ConvertAddressRequest,
                              ConvertUtxoAddressRequest, MyTxHistoryRequest, MyTxHistoryRequestV2,
                              ShowPrivateKeyRequest, StandardHDCoinAddress, ValidateAddressRequest};
use crate::rpc_data::{SendRawTransactionRequest, WithdrawFee, WithdrawFrom, WithdrawRequest};
use crate::{error_anyhow, error_bail};

#[derive(Subcommand)]
pub(crate) enum WalletCommands {
    #[command(visible_alias = "balance", about = "Get coin balance")]
    MyBalance(MyBalanceArgs),
    #[command(
        about = "Generates, signs, and returns a transaction that transfers the amount of coin to \
                 the address indicated in the to argument"
    )]
    Withdraw(WithdrawArgs),
    #[command(
        visible_aliases = ["send-raw", "send"],
        about = "Broadcasts the transaction to the network of the given coin"
    )]
    SendRawTransaction(SendRawTransactionArgs),
    #[command(
        visible_aliases = ["get-raw", "raw-tx", "get"],
        about = "Returns the full signed raw transaction hex for any transaction that is confirmed \
                 or within the mempool"
    )]
    GetRawTransaction(GetRawTransactionArgs),
    #[command(
        visible_aliases = ["history"],
        about = "Returns the blockchain transactions involving the Komodo DeFi Framework node's coin address"
    )]
    TxHistory(TxHistoryArgs),
    #[command(
        visible_aliases = ["private", "private-key"],
        about = "Returns the private key of the specified coin in a format compatible with coin wallets"
    )]
    ShowPrivKey(ShowPrivKeyArgs),
    #[command(
        visible_aliases = ["validate"],
        about = "Checks if an input string is a valid address of the specified coin"
    )]
    ValidateAddress(ValidateAddressArgs),
    #[command(
        visible_aliases = ["rewards"],
        about = "Informs about the active user rewards that can be claimed by an address's unspent outputs"
    )]
    KmdRewardsInfo,
    #[command(
        visible_aliases = ["convert"],
        about = "Converts an input address to a specified address format"
    )]
    ConvertAddress(ConvertAddressArgs),
    #[command(
        visible_aliases = ["convert-utxo"],
        about = "Takes a UTXO address as input, and returns the equivalent address for another \
                 UTXO coin (e.g. from BTC address to RVN address)")]
    ConvertUtxoAddress(ConvertUtxoArgs),
    #[command(
        visible_aliases = ["get-public", "public-key", "public"],
        about = "Returns the compressed secp256k1 pubkey corresponding to the user's seed phrase"
    )]
    GetPublicKey,
    #[command(
        visible_aliases = ["pubkey-hash", "hash", "pubhash"],
        about = "Returns the RIPEMD-160 hash version of your public key"
    )]
    GetPublicKeyHash,
}

#[derive(Args)]
pub(crate) struct MyBalanceArgs {
    #[arg(name = "COIN", help = "Coin to get balance")]
    coin: String,
}

impl From<&mut MyBalanceArgs> for BalanceRequest {
    fn from(value: &mut MyBalanceArgs) -> Self {
        BalanceRequest {
            coin: take(&mut value.coin),
        }
    }
}

#[derive(Args)]
pub(crate) struct SendRawTransactionArgs {
    #[arg(long, short, help = "Name of the coin network on which to broadcast the transaction")]
    coin: String,
    #[arg(
        long,
        short,
        value_parser=parse_bytes,
        help="Transaction bytes in hexadecimal format;"
    )]
    tx_hex: BytesJson,
    #[arg(
        long,
        short,
        default_value_t = false,
        visible_alias = "bare",
        help = "Whether to output only tx_hash"
    )]
    pub(crate) bare_output: bool,
}

fn parse_bytes(value: &str) -> Result<BytesJson, FromHexError> {
    let bytes = hex::decode(value)?;
    Ok(BytesJson(bytes))
}

impl From<&mut SendRawTransactionArgs> for SendRawTransactionRequest {
    fn from(value: &mut SendRawTransactionArgs) -> Self {
        SendRawTransactionRequest {
            coin: take(&mut value.coin),
            tx_hex: take(&mut value.tx_hex),
        }
    }
}

#[derive(Args)]
pub(crate) struct WithdrawArgs {
    #[arg(help = "Coin the user desires to withdraw")]
    coin: String,
    #[arg(help = "Address the user desires to withdraw to")]
    to: String,
    #[command(flatten)]
    amount: WithdrawAmountArg,
    #[arg(
        long,
        short,
        value_parser = parse_withdraw_fee,
        help = "Transaction fee [possible-values: <utxo-fixed:amount|utxo-per-kbyte:amount|eth:gas_price:gas|qrc:gas_limit:gas_price|cosmos:gas_limit:gas_price>]"
    )]
    fee: Option<WithdrawFeeArg>,
    #[command(flatten)]
    from: Option<WithdrawFromArgs>,
    #[arg(
        long,
        short,
        default_value_t = false,
        visible_alias = "bare",
        help = "Whether to output only tx_hex"
    )]
    pub(crate) bare_output: bool,
}

#[derive(Args)]
#[group(required = false, multiple = true)]
struct WithdrawFromArgs {
    #[arg(
        long,
        help = "Derivation path to determine the source of the derived value in more detail"
    )]
    derivation_path: Option<String>,
    #[arg(long, help = "Account index of the same crypto currency")]
    hd_account_id: Option<u32>,
    #[arg(long, value_enum, help = "Is change")]
    hd_is_change: Option<bool>,
    #[arg(long, help = "An incremental address index for the account")]
    hd_address_index: Option<u32>,
}

#[derive(Clone, Debug, ValueEnum)]
enum Bip44ChainArg {
    External,
    Internal,
}

#[derive(Args)]
#[group(required = false, multiple = false)]
pub(crate) struct WithdrawAmountArg {
    #[arg(
        long,
        short = 'M',
        group = "withdraw-amount",
        default_value_t = false,
        help = "Withdraw the maximum available amount"
    )]
    pub(crate) max: bool,
    #[arg(long, short, group = "withdraw-amount", help = "Amount the user desires to withdraw")]
    pub(crate) amount: Option<BigDecimal>,
}

#[derive(Clone)]
pub(crate) enum WithdrawFeeArg {
    UtxoFixed { amount: BigDecimal },
    UtxoPerKbyte { amount: BigDecimal },
    EthGas { gas_price: BigDecimal, gas: u64 },
    Qrc20Gas { gas_limit: u64, gas_price: u64 },
    CosmosGas { gas_limit: u64, gas_price: f64 },
}

fn parse_withdraw_fee(value: &str) -> Result<WithdrawFeeArg> {
    #[derive(Clone, ValueEnum)]
    pub(crate) enum WithdrawFeeTag {
        UtxoFixed,
        UtxoPerKbyte,
        Eth,
        Qrc,
        Cosmos,
    }
    let mut parts = value.split(':');
    match parts.next() {
        Some(tag) => match WithdrawFeeTag::from_str(tag, false) {
            Ok(WithdrawFeeTag::UtxoFixed) => {
                let Some(amount) = parts.next() else {
                    error_bail!("Failed to parse utxo-fixed fee, no amount literal");
                };
                let amount = BigDecimal::from_str(amount)
                    .map_err(|error| error_anyhow!("Failed to parse amount from: {}, error: {}", amount, error))?;
                Ok(WithdrawFeeArg::UtxoFixed { amount })
            },
            Ok(WithdrawFeeTag::UtxoPerKbyte) => {
                let Some(amount) = parts.next() else {
                    error_bail!("Failed to parse utxo-per-kbyte fee, no amount literal");
                };
                let amount = BigDecimal::from_str(amount)
                    .map_err(|error| error_anyhow!("Failed to parse amount from: {}, error: {}", amount, error))?;
                Ok(WithdrawFeeArg::UtxoPerKbyte { amount })
            },
            Ok(WithdrawFeeTag::Eth) => {
                let Some(gas_price) = parts.next() else {
                    error_bail!("Failed to parse eth fee, no gas_price literal");
                };
                let gas_price = BigDecimal::from_str(gas_price).map_err(|error| {
                    error_anyhow!("Failed to parse gas_price from: {}, error: {}", gas_price, error)
                })?;
                let Some(gas) = parts.next() else {
                    error_bail!("Failed to parse eth fee, no gas literal");
                };
                let gas = u64::from_str(gas)
                    .map_err(|error| error_anyhow!("Failed to parse gas from: {}, error: {}", gas, error))?;

                Ok(WithdrawFeeArg::EthGas { gas_price, gas })
            },
            Ok(WithdrawFeeTag::Qrc) => {
                let Some(gas_limit) = parts.next() else {
                    error_bail!("Failed to parse qrc fee, no gas_limit literal");
                };
                let gas_limit = u64::from_str(gas_limit).map_err(|error| {
                    error_anyhow!("Failed to parse gas_limit from: {}, error: {}", gas_limit, error)
                })?;

                let Some(gas_price) = parts.next() else {
                    error_bail!("Failed to parse qrc fee, no gas_price literal");
                };
                let gas_price = u64::from_str(gas_price).map_err(|error| {
                    error_anyhow!("Failed to parse gas_price from: {}, error: {}", gas_price, error)
                })?;
                Ok(WithdrawFeeArg::Qrc20Gas { gas_limit, gas_price })
            },
            Ok(WithdrawFeeTag::Cosmos) => {
                let Some(gas_limit) = parts.next() else {
                    error_bail!("Failed to parse cosomos fee, no gas_limit literal");
                };
                let gas_limit = u64::from_str(gas_limit).map_err(|error| {
                    error_anyhow!("Failed to parse gas_limit from: {}, error: {}", gas_limit, error)
                })?;

                let Some(gas_price) = parts.next() else {
                    error_bail!("Failed to parse cosmos fee, no gas_price literal");
                };
                let gas_price = f64::from_str(gas_price).map_err(|error| {
                    error_anyhow!("Failed to parse gas_price from: {}, error: {}", gas_price, error)
                })?;
                Ok(WithdrawFeeArg::CosmosGas { gas_limit, gas_price })
            },
            Err(error) => {
                error_bail!("Failed to parse fee_tag: {}", error)
            },
        },
        None => {
            error_bail!("Failed to parse withdraw_fee: tag literal has not been found");
        },
    }
}

impl TryFrom<&mut WithdrawArgs> for WithdrawRequest {
    type Error = anyhow::Error;

    fn try_from(value: &mut WithdrawArgs) -> std::result::Result<Self, Self::Error> {
        let from = if let Some(from) = value.from.as_mut() {
            Some(WithdrawFrom::try_from(from)?)
        } else {
            None
        };
        Ok(WithdrawRequest {
            coin: take(&mut value.coin),
            from,
            to: take(&mut value.to),
            amount: value.amount.amount.take().unwrap_or_default(),
            max: value.amount.max,
            fee: value.fee.as_mut().map(WithdrawFee::from),
        })
    }
}

impl From<&mut WithdrawFeeArg> for WithdrawFee {
    fn from(value: &mut WithdrawFeeArg) -> Self {
        match value {
            WithdrawFeeArg::UtxoFixed { amount } => WithdrawFee::UtxoFixed { amount: take(amount) },
            WithdrawFeeArg::UtxoPerKbyte { amount } => WithdrawFee::UtxoPerKbyte { amount: take(amount) },
            WithdrawFeeArg::EthGas { gas_price, gas } => WithdrawFee::EthGas {
                gas_price: take(gas_price),
                gas: take(gas),
            },
            WithdrawFeeArg::Qrc20Gas { gas_limit, gas_price } => WithdrawFee::Qrc20Gas {
                gas_limit: take(gas_limit),
                gas_price: take(gas_price),
            },
            WithdrawFeeArg::CosmosGas { gas_limit, gas_price } => WithdrawFee::CosmosGas {
                gas_limit: take(gas_limit),
                gas_price: take(gas_price),
            },
        }
    }
}

impl TryFrom<&mut WithdrawFromArgs> for WithdrawFrom {
    type Error = anyhow::Error;

    fn try_from(value: &mut WithdrawFromArgs) -> std::result::Result<Self, Self::Error> {
        if let Some(derivation_path) = value.derivation_path.take() {
            return Ok(WithdrawFrom::DerivationPath { derivation_path });
        };
        let account_id = value.hd_account_id.take();
        let is_change = value.hd_is_change.take();
        let address_id = value.hd_address_index.take();
        if let (Some(account), Some(is_change), Some(address_index)) = (account_id, is_change, address_id) {
            return Ok(WithdrawFrom::HDWalletAddress(StandardHDCoinAddress {
                account,
                is_change,
                address_index,
            }));
        };
        error_bail!(
            "Failed to get withdraw_from due to params incompatibility: account_id: {:?}, chain: {:?}, address_id: {:?}",
            account_id,
            is_change,
            address_id
        )
    }
}

#[derive(Args)]
pub(crate) struct GetRawTransactionArgs {
    #[arg(long, short, help = "Coin the user desires to request for the transaction")]
    pub(crate) coin: String,
    #[arg(
        long,
        value_parser=parse_bytes,
        visible_alias = "hash",
        short = 'H',
        help = "Hash of the transaction"
    )]
    pub(crate) tx_hash: BytesJson,
    #[arg(
        long,
        short,
        default_value_t = false,
        visible_alias = "bare",
        help = "Whether to output only tx_hex"
    )]
    pub(crate) bare_output: bool,
}

impl From<&mut GetRawTransactionArgs> for GetRawTransactionRequest {
    fn from(value: &mut GetRawTransactionArgs) -> Self {
        GetRawTransactionRequest {
            coin: take(&mut value.coin),
            tx_hash: hex::encode(value.tx_hash.as_slice()),
        }
    }
}

#[derive(Args)]
pub(crate) struct TxHistoryArgs {
    #[arg(help = "The name of the coin for the history request")]
    pub(crate) coin: String,
    #[command(flatten)]
    limit: TxHistoryLimitGroup,
    #[command(flatten)]
    from_id: FromIdGroup,
    #[arg(
        long,
        visible_alias = "page",
        short = 'p',
        help = "The name of the coin for the history request"
    )]
    page_number: Option<usize>,
}

#[derive(Default, Args)]
#[group(required = false, multiple = false)]
struct FromIdGroup {
    #[arg(
        long,
        short = 'H',
        value_parser = parse_bytes,
        help = "Skips records until it reaches this hash, skipping the from_id as well"
    )]
    from_tx_hash: Option<BytesJson>,
    #[arg(
        long,
        short = 'i',
        help = "For zcoin compatibility, skips records until it reaches this ID, skipping the from_id as well"
    )]
    from_tx_id: Option<i64>,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct TxHistoryLimitGroup {
    #[arg(
        group = "limit-group",
        long,
        short,
        help = "Limits the number of returned transactions"
    )]
    limit: Option<usize>,
    #[arg(
        group = "limit-group",
        long,
        short,
        default_value_t = false,
        help = "Whether to return all available records"
    )]
    max: bool,
}

impl From<&mut TxHistoryArgs> for MyTxHistoryRequest {
    fn from(value: &mut TxHistoryArgs) -> Self {
        MyTxHistoryRequest {
            coin: take(&mut value.coin),
            from_id: value.from_id.from_tx_hash.take(),
            max: value.limit.max,
            limit: value.limit.limit.unwrap_or(10),
            page_number: value.page_number.take(),
        }
    }
}

trait FromId<T> {
    fn get(self) -> Option<T>;
}

impl FromId<BytesJson> for FromIdGroup {
    fn get(mut self) -> Option<BytesJson> { self.from_tx_hash.take() }
}

impl FromId<i64> for FromIdGroup {
    fn get(mut self) -> Option<i64> { self.from_tx_id.take() }
}

impl<T> From<&mut TxHistoryArgs> for MyTxHistoryRequestV2<T>
where
    FromIdGroup: FromId<T>,
{
    fn from(value: &mut TxHistoryArgs) -> Self {
        let paging_options = if let Some(from_id) = take(&mut value.from_id).get() {
            PagingOptionsEnum::FromId(from_id)
        } else if let Some(page_number) = value.page_number.take() {
            if page_number > 0 {
                PagingOptionsEnum::PageNumber(
                    NonZeroUsize::new(page_number).expect("Page number is expected to be greater than zero"),
                )
            } else {
                PagingOptionsEnum::default()
            }
        } else {
            PagingOptionsEnum::default()
        };

        MyTxHistoryRequestV2 {
            coin: take(&mut value.coin),
            limit: if value.limit.max {
                u32::MAX as usize
            } else {
                value
                    .limit
                    .limit
                    .expect("limit option is expected to be set due to group rules")
            },
            paging_options,
        }
    }
}

#[derive(Args)]
pub(crate) struct ShowPrivKeyArgs {
    #[arg(help = "The name of the coin of the private key to show")]
    coin: String,
}

impl From<&mut ShowPrivKeyArgs> for ShowPrivateKeyRequest {
    fn from(value: &mut ShowPrivKeyArgs) -> Self {
        ShowPrivateKeyRequest {
            coin: take(&mut value.coin),
        }
    }
}

#[derive(Args)]
pub(crate) struct ValidateAddressArgs {
    #[arg(help = "The coin to validate address for")]
    coin: String,
    #[arg(help = "The input string to validate")]
    address: String,
}

impl From<&mut ValidateAddressArgs> for ValidateAddressRequest {
    fn from(value: &mut ValidateAddressArgs) -> Self {
        ValidateAddressRequest {
            coin: take(&mut value.coin),
            address: take(&mut value.address),
        }
    }
}

#[derive(Args)]
pub(crate) struct ConvertAddressArgs {
    #[arg(long, short, help = "The name of the coin address context")]
    coin: String,
    #[arg(long, short, help = "Input address")]
    from: String,
    #[arg(
        long,
        short = 'F',
        value_enum,
        help = "Address format to which the input address should be converted"
    )]
    format: ConvertAddressFormat,
    #[arg(long, short = 'C', value_enum, help = "Network prefix for cashaddress format")]
    cash_address_network: Option<CashAddressNetwork>,
}

#[derive(Clone, ValueEnum)]
pub(crate) enum ConvertAddressFormat {
    MixedCase,
    CashAddress,
    Standard,
}

#[derive(Clone, ValueEnum)]
pub(crate) enum CashAddressNetwork {
    BitcoinCash,
    BchTest,
    BchReg,
}

impl TryFrom<&mut ConvertAddressArgs> for ConvertAddressRequest {
    type Error = Error;
    fn try_from(value: &mut ConvertAddressArgs) -> Result<Self> {
        let to_address_format = match value.format {
            ConvertAddressFormat::Standard => RpcConvertAddressFormat::Standard,
            ConvertAddressFormat::MixedCase => RpcConvertAddressFormat::MixedCase,
            ConvertAddressFormat::CashAddress => match value.cash_address_network {
                Some(CashAddressNetwork::BitcoinCash) => {
                    RpcConvertAddressFormat::CashAddress(RpcCashAddressNetwork::BitcoinCash)
                },
                Some(CashAddressNetwork::BchReg) => RpcConvertAddressFormat::CashAddress(RpcCashAddressNetwork::BchReg),
                Some(CashAddressNetwork::BchTest) => {
                    RpcConvertAddressFormat::CashAddress(RpcCashAddressNetwork::BchTest)
                },
                None => error_bail!("Failed to construct request from arguments, cash_address is not set"),
            },
        };
        Ok(ConvertAddressRequest {
            coin: take(&mut value.coin),
            from: take(&mut value.from),
            to_address_format,
        })
    }
}

#[derive(Args)]
pub(crate) struct ConvertUtxoArgs {
    #[arg(help = "Input UTXO address")]
    address: String,
    #[arg(help = "Coin to convert input given address to")]
    to_coin: String,
}

impl From<&mut ConvertUtxoArgs> for ConvertUtxoAddressRequest {
    fn from(value: &mut ConvertUtxoArgs) -> Self {
        ConvertUtxoAddressRequest {
            address: take(&mut value.address),
            to_coin: take(&mut value.to_coin),
        }
    }
}
