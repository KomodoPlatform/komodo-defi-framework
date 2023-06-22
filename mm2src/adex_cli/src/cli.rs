use anyhow::Result;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use common::serde_derive::Serialize;
use mm2_number::{bigdecimal::ParseBigDecimalError, BigDecimal, MmNumber};
use mm2_rpc::data::legacy::{MatchBy, OrderType, SellBuyRequest};
use rpc::v1::types::H256 as H256Json;
use std::collections::HashSet;
use std::mem::take;
use std::str::FromStr;
use uuid::Uuid;

use crate::adex_config::{get_config, set_config, AdexConfig};
use crate::adex_proc::{AdexProc, OrderbookConfig, ResponseHandler};
use crate::scenarios::{get_status, init, start_process, stop_process};
use crate::transport::SlurpTransport;

const MM2_CONFIG_FILE_DEFAULT: &str = "MM2.json";
const COINS_FILE_DEFAULT: &str = "coins";
const ORDERBOOK_BIDS_LIMIT: &str = "20";
const ORDERBOOK_ASKS_LIMIT: &str = "20";

#[derive(Subcommand)]
enum Command {
    #[command(about = "Initialize a predefined coin set and configuration to start mm2 instance with")]
    Init {
        #[arg(long, help = "coin set file path", default_value = COINS_FILE_DEFAULT)]
        mm_coins_path: String,
        #[arg(long, help = "mm2 configuration file path", default_value = MM2_CONFIG_FILE_DEFAULT)]
        mm_conf_path: String,
    },
    #[command(about = "Start mm2 instance")]
    Start {
        #[arg(long, help = "mm2 configuration file path")]
        mm_conf_path: Option<String>,
        #[arg(long, help = "coin set file path")]
        mm_coins_path: Option<String>,
        #[arg(long, help = "log file path")]
        mm_log: Option<String>,
    },
    #[command(about = "Stop mm2 using API")]
    Stop,
    #[command(about = "Kill mm2 process")]
    Kill,
    #[command(about = "Get mm2 running status")]
    Status,
    #[command(about = "Gets version of intermediary mm2 service")]
    Version,
    #[command(subcommand, about = "To manage rpc_password and mm2 RPC URL")]
    Config(ConfigSubcommand),
    #[command(about = "Puts an asset to the trading index")]
    Enable {
        #[arg(name = "ASSET", help = "Asset to be included into the trading index")]
        asset: String,
    },
    #[command(about = "Gets balance of an asset")]
    Balance {
        #[arg(name = "ASSET", help = "Asset to get balance of")]
        asset: String,
    },
    #[command(about = "Lists activated assets")]
    GetEnabled,
    #[command(about = "Gets orderbook")]
    Orderbook {
        #[command(flatten)]
        orderbook_args: OrderbookCliArgs,
    },
    Sell {
        #[command(flatten)]
        order_args: SellOrderCli,
    },
    Buy {
        #[command(flatten)]
        order_args: BuyOrderCli,
    },
}

#[derive(Subcommand)]
enum ConfigSubcommand {
    #[command(about = "Sets komodo adex cli configuration")]
    Set(SetConfigArgs),
    #[command(about = "Gets komodo adex cli configuration")]
    Get,
}

#[derive(Args)]
#[group(required = true, multiple = true)]
struct SetConfigArgs {
    #[arg(long, help = "Set if you are going to set up a password")]
    set_password: bool,
    #[arg(long, name = "URI", help = "Adex RPC API Uri. http://localhost:7783")]
    adex_uri: Option<String>,
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub(super) struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub(super) async fn execute<P: ResponseHandler, Cfg: AdexConfig + 'static>(
        args: impl Iterator<Item = String>,
        config: &Cfg,
        printer: &P,
    ) -> Result<()> {
        let transport = SlurpTransport::new(config.rpc_uri()?);

        let proc = AdexProc {
            transport: &transport,
            response_handler: printer,
            config,
        };

        let mut parsed_cli = Self::parse_from(args);
        match &mut parsed_cli.command {
            Command::Init {
                mm_coins_path: coins_file,
                mm_conf_path: mm2_cfg_file,
            } => init(mm2_cfg_file, coins_file).await,
            Command::Start {
                mm_conf_path: mm2_cfg_file,
                mm_coins_path: coins_file,
                mm_log: log_file,
            } => start_process(mm2_cfg_file, coins_file, log_file),
            Command::Version => proc.get_version().await?,
            Command::Kill => stop_process(),
            Command::Status => get_status(),
            Command::Stop => proc.send_stop().await?,
            Command::Config(ConfigSubcommand::Set(SetConfigArgs { set_password, adex_uri })) => {
                set_config(*set_password, adex_uri.take())?
            },
            Command::Config(ConfigSubcommand::Get) => get_config(),
            Command::Enable { asset } => proc.enable(asset).await?,
            Command::Balance { asset } => proc.get_balance(asset).await?,
            Command::GetEnabled => proc.get_enabled().await?,
            Command::Orderbook { ref orderbook_args } => {
                proc.get_orderbook(
                    &orderbook_args.base,
                    &orderbook_args.rel,
                    OrderbookConfig::from(orderbook_args),
                )
                .await?
            },
            Command::Sell {
                order_args: SellOrderCli { order_cli },
            } => proc.sell(SellBuyRequest::from(order_cli)).await?,
            Command::Buy {
                order_args: BuyOrderCli { order_cli },
            } => proc.buy(SellBuyRequest::from(order_cli)).await?,
        }
        Ok(())
    }
}

#[derive(Args)]
#[command(about = "Puts a selling coins request")]
struct SellOrderCli {
    #[command(flatten)]
    order_cli: OrderCli,
}

#[derive(Args)]
#[command(about = "Puts a buying coins request")]
struct BuyOrderCli {
    #[command(flatten)]
    order_cli: OrderCli,
}

#[derive(Args, Serialize, Debug)]
struct OrderbookCliArgs {
    #[arg(help = "Base currency of a pair")]
    base: String,
    #[arg(help = "Related currency, also can be called \"quote currency\" according to exchange terms")]
    rel: String,
    #[arg(long, help = "Orderbook asks count limitation", default_value = ORDERBOOK_ASKS_LIMIT)]
    asks_limit: Option<usize>,
    #[arg(long, help = "Orderbook bids count limitation", default_value = ORDERBOOK_BIDS_LIMIT)]
    bids_limit: Option<usize>,
    #[arg(long, help = "Enables `uuid` column")]
    uuids: bool,
    #[arg(long, help = "Enables `min_volume` column")]
    min_volume: bool,
    #[arg(long, help = "Enables `max_volume` column")]
    max_volume: bool,
    #[arg(long, help = "Enables `public` column")]
    publics: bool,
    #[arg(long, help = "Enables `address` column")]
    address: bool,
    #[arg(long, help = "Enables `age` column")]
    age: bool,
    #[arg(long, help = "Enables order confirmation settings column")]
    conf_settings: bool,
}

impl From<&OrderbookCliArgs> for OrderbookConfig {
    fn from(value: &OrderbookCliArgs) -> Self {
        OrderbookConfig {
            uuids: value.uuids,
            min_volume: value.min_volume,
            max_volume: value.max_volume,
            publics: value.publics,
            address: value.address,
            age: value.age,
            conf_settings: value.conf_settings,
            asks_limit: value.asks_limit,
            bids_limit: value.bids_limit,
        }
    }
}

#[derive(Args, Serialize, Debug)]
struct OrderCli {
    #[arg(help = "Base currency of a pair")]
    base: String,
    #[arg(help = "Related currency")]
    rel: String,
    #[arg(help = "Amount of coins the user is willing to sell/buy of the base coin", value_parser=parse_mm_number )]
    volume: MmNumber,
    #[arg(help = "Price in rel the user is willing to receive/pay per one unit of the base coin", value_parser=parse_mm_number)]
    price: MmNumber,
    #[arg(long, value_enum, default_value_t = OrderTypeCli::GoodTillCancelled, help="The GoodTillCancelled order is automatically converted to a maker order if not matched in 30 seconds, and this maker order stays in the orderbook until explicitly cancelled. On the other hand, a FillOrKill is cancelled if not matched within 30 seconds")]
    order_type: OrderTypeCli,
    #[arg(long,
          help = "Amount of base coin that will be used as min_volume of GoodTillCancelled order after conversion to maker", 
          value_parser=parse_mm_number
    )]
    min_volume: Option<MmNumber>,
    #[arg(short='u', long="uuid", action = ArgAction::Append, help="The created order is matched using a set of uuid")]
    match_uuids: Vec<Uuid>,
    #[arg(short='p',
          long="public",
          value_parser=H256Json::from_str,
          action = ArgAction::Append,
          help="The created order is matched using a set of publics to select specific nodes (ignored if uuids not empty)")]
    match_publics: Vec<H256Json>,
    #[arg(
        long,
        help = "Number of required blockchain confirmations for base coin atomic swap transaction"
    )]
    base_confs: Option<u64>,
    #[arg(
        long,
        help = "Whether dPoW notarization is required for base coin atomic swap transaction"
    )]
    base_nota: Option<bool>,
    #[arg(
        long,
        help = "Number of required blockchain confirmations for rel coin atomic swap transaction"
    )]
    rel_confs: Option<u64>,
    #[arg(
        long,
        help = "Whether dPoW notarization is required for rel coin atomic swap transaction"
    )]
    rel_nota: Option<bool>,
    #[arg(
        long,
        help = "If true, each order's short record history is stored else the only order status will be temporarily stored while in progress"
    )]
    save_in_history: bool,
}

fn parse_mm_number(value: &str) -> Result<MmNumber, ParseBigDecimalError> {
    let decimal: BigDecimal = BigDecimal::from_str(value)?;
    Ok(MmNumber::from(decimal))
}

#[derive(Debug, Copy, Clone, ValueEnum, Serialize)]
enum OrderTypeCli {
    FillOrKill,
    GoodTillCancelled,
}

impl From<OrderTypeCli> for OrderType {
    fn from(value: OrderTypeCli) -> Self {
        match value {
            OrderTypeCli::GoodTillCancelled => OrderType::GoodTillCancelled,
            OrderTypeCli::FillOrKill => OrderType::FillOrKill,
        }
    }
}

impl From<&mut OrderCli> for SellBuyRequest {
    fn from(value: &mut OrderCli) -> Self {
        let match_by = if !value.match_uuids.is_empty() {
            MatchBy::Orders(HashSet::from_iter(value.match_uuids.drain(..)))
        } else if !value.match_publics.is_empty() {
            MatchBy::Pubkeys(HashSet::from_iter(value.match_publics.drain(..)))
        } else {
            MatchBy::Any
        };

        let will_be_substituted = String::new();
        SellBuyRequest {
            base: take(&mut value.base),
            rel: take(&mut value.rel),
            price: take(&mut value.price),
            volume: take(&mut value.volume),
            timeout: None,
            duration: None,
            method: will_be_substituted,
            gui: None,
            dest_pub_key: H256Json::default(),
            match_by,
            order_type: value.order_type.into(),
            base_confs: value.base_confs,
            base_nota: value.base_nota,
            rel_confs: value.rel_confs,
            rel_nota: value.rel_nota,
            min_volume: take(&mut value.min_volume),
            save_in_history: value.save_in_history,
        }
    }
}
