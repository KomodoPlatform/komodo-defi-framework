use crate::logging::error_anyhow;
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use common::log::error;
use directories::ProjectDirs;
use std::fs;
use std::mem::take;
use std::path::PathBuf;

use crate::config::{get_config, set_config, KomodefiConfig};
use crate::komodefi_proc::{KomodefiProc, ResponseHandler};
use crate::scenarios::{download_binary_and_extract_to_bin_folder, get_status, init, start_process, stop_process};
use crate::transport::SlurpTransport;

use super::cli_cmd_args::prelude::*;

const COINS_FILE_DEFAULT: &str = "coins";
const MM2_CONFIG_FILE_DEFAULT: &str = "MM2.json";
const PROJECT_QUALIFIER: &str = "com";
const PROJECT_COMPANY: &str = "komodoplatform";
const PROJECT_APP: &str = "komodefi-cli";

#[derive(Subcommand)]
enum Command {
    #[command(about = "Initialize a predefined coin set and configuration to start mm2 instance with")]
    Init {
        #[arg(long, visible_alias = "coins", help = "Coin set file path", default_value = COINS_FILE_DEFAULT)]
        mm_coins_path: String,
        #[arg(long, visible_alias = "conf", help = "mm2 configuration file path", default_value = MM2_CONFIG_FILE_DEFAULT)]
        mm_conf_path: String,
    },
    #[command(subcommand, about = "Manage rpc_password and mm2 RPC URL")]
    Config(ConfigSubcommand),
    #[command(subcommand, about = "Manage mm2 instance commands")]
    Mm2(Mm2Commands),
    #[command(subcommand, about = "Coin commands: enable, disable etc.")]
    Coin(CoinCommands),
    #[command(subcommand, about = "Wallet commands: balance, withdraw etc.")]
    Wallet(WalletCommands),
    Sell(SellOrderArgs),
    Buy(BuyOrderArgs),
    SetPrice(SetPriceArgs),
    #[command(visible_alias = "update", about = "Update order on the orderbook")]
    UpdateMakerOrder(UpdateMakerOrderArgs),
    #[command(
        subcommand,
        visible_alias = "orders",
        about = "Order listing commands: book, history, depth etc."
    )]
    Order(OrderCommands),
    #[command(subcommand, visible_alias = "swap", about = "Swap related commands")]
    Swaps(SwapCommands),
    #[command(subcommand, about = "Cancel one or many orders")]
    Cancel(CancelSubcommand),
    #[command(subcommand, visible_aliases = ["util", "pubkeys", "pubkey"], about = "Utility commands")]
    Utility(UtilityCommands),
    #[command(subcommand, about = "Message signing commands: sign, verify)")]
    Message(MessageCommands),
    #[command(subcommand, about = "Network commands")]
    Network(NetworkCommands),
    #[command(subcommand, visible_aliases = ["stat", "vstat"], about = "Version statistic commands")]
    VersionStat(VersionStatCommands),
    #[command(subcommand, about = "Tracking the status of long-running commands")]
    Task(TaskSubcommand),
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub(super) struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub(super) async fn execute<P: ResponseHandler, Cfg: KomodefiConfig + 'static>(
        args: impl Iterator<Item = String>,
        config: &Cfg,
        printer: &P,
    ) -> Result<()> {
        let transport = config.rpc_uri().map(SlurpTransport::new);

        let proc = KomodefiProc {
            transport: transport.as_ref(),
            response_handler: printer,
            config,
        };

        let mut parsed_cli = Self::parse_from(args);
        match &mut parsed_cli.command {
            Command::Init {
                mm_coins_path: coins_file,
                mm_conf_path: mm2_cfg_file,
            } => init(mm2_cfg_file, coins_file).await,
            Command::Mm2(Mm2Commands::Start {
                mm_conf_path: mm2_cfg_file,
                mm_coins_path: coins_file,
                mm_log: log_file,
            }) => start_process(mm2_cfg_file, coins_file, log_file),
            Command::Mm2(Mm2Commands::Version) => proc.get_version().await?,
            Command::Mm2(Mm2Commands::Download) => download_binary_and_extract_to_bin_folder().await?,
            Command::Mm2(Mm2Commands::Kill) => stop_process(),
            Command::Mm2(Mm2Commands::Status) => get_status(),
            Command::Mm2(Mm2Commands::Stop) => proc.send_stop().await?,
            Command::Config(ConfigSubcommand::Set(config)) => set_config(config)?,
            Command::Config(ConfigSubcommand::Get(option)) => get_config(option),
            Command::Coin(CoinCommands::Enable(args)) => {
                proc.enable(&args.coin, args.keep_progress, args.tx_history).await?
            },
            Command::Coin(CoinCommands::Disable(args)) => proc.disable(args.into()).await?,
            Command::Coin(CoinCommands::GetEnabled) => proc.get_enabled().await?,
            Command::Coin(CoinCommands::SetRequiredConf(args)) => proc.set_required_confirmations(args.into()).await?,
            Command::Coin(CoinCommands::SetRequiredNota(args)) => proc.set_required_nota(args.into()).await?,
            Command::Coin(CoinCommands::CoinsToKickStart) => proc.coins_to_kick_start().await?,
            Command::Order(OrderCommands::Orderbook(obook_args)) => {
                proc.get_orderbook(obook_args.into(), obook_args.into()).await?
            },
            Command::Order(OrderCommands::OrderbookDepth(orderbook_depth_args)) => {
                proc.orderbook_depth(orderbook_depth_args.into()).await?
            },
            Command::Order(OrderCommands::OrdersHistory(history_args)) => {
                proc.orders_history(history_args.into(), history_args.into()).await?
            },
            Command::Order(OrderCommands::OrderStatus(order_status_args)) => {
                proc.order_status(order_status_args.into()).await?
            },
            Command::Order(OrderCommands::MyOrders) => proc.my_orders().await?,
            Command::Order(OrderCommands::BestOrders(best_orders_args)) => {
                let show_orig_tickets = best_orders_args.show_orig_tickets;
                proc.best_orders(best_orders_args.into(), show_orig_tickets).await?
            },
            Command::Cancel(CancelSubcommand::Order(args)) => proc.cancel_order(args.into()).await?,
            Command::Cancel(CancelSubcommand::All) => proc.cancel_all_orders().await?,
            Command::Cancel(CancelSubcommand::ByPair(args)) => proc.cancel_by_pair(args.into()).await?,
            Command::Cancel(CancelSubcommand::ByCoin(args)) => proc.cancel_by_coin(args.into()).await?,

            Command::Sell(sell_args) => proc.sell(sell_args.into()).await?,
            Command::Buy(buy_args) => proc.buy(buy_args.into()).await?,

            Command::SetPrice(set_price_args) => proc.set_price(set_price_args.into()).await?,
            Command::UpdateMakerOrder(update_maker_args) => proc.update_maker_order(update_maker_args.into()).await?,
            Command::Swaps(SwapCommands::ActiveSwaps(args)) => proc.active_swaps(args.include_status).await?,
            Command::Swaps(SwapCommands::MySwapStatus(args)) => proc.swap_status(args.uuid).await?,
            Command::Swaps(SwapCommands::MyRecentSwaps(args)) => proc.recent_swaps(args.into()).await?,
            Command::Swaps(SwapCommands::RecoverFundsOfSwap(args)) => proc.recover_funds_of_swap(args.into()).await?,
            Command::Swaps(SwapCommands::MinTradingVol { coin }) => proc.min_trading_vol(take(coin)).await?,
            Command::Swaps(SwapCommands::MaxTakerVol { coin }) => proc.max_taker_vol(take(coin)).await?,
            Command::Swaps(SwapCommands::TradePreimage(args)) => proc.trade_preimage(args.into()).await?,
            Command::Network(NetworkCommands::GetGossipMesh) => proc.get_gossip_mesh().await?,
            Command::Network(NetworkCommands::GetRelayMesh) => proc.get_relay_mesh().await?,
            Command::Network(NetworkCommands::GetGossipPeerTopics) => proc.get_gossip_peer_topics().await?,
            Command::Network(NetworkCommands::GetGossipTopicPeers) => proc.get_gossip_topic_peers().await?,
            Command::Network(NetworkCommands::GetMyPeerId) => proc.get_my_peer_id().await?,
            Command::Network(NetworkCommands::GetPeersInfo) => proc.get_peers_info().await?,
            Command::Utility(UtilityCommands::BanPubkey(args)) => proc.ban_pubkey(args.into()).await?,
            Command::Utility(UtilityCommands::ListBannedPubkeys) => proc.list_banned_pubkeys().await?,
            Command::Utility(UtilityCommands::UnbanPubkeys(args)) => proc.unban_pubkeys(args.into()).await?,
            Command::Utility(UtilityCommands::GetCurrentMtp(args)) => proc.get_current_mtp(args.into()).await?,
            Command::Wallet(WalletCommands::MyBalance(my_balance_args)) => {
                proc.get_balance(my_balance_args.into()).await?
            },
            Command::Wallet(WalletCommands::SendRawTransaction(args)) => {
                proc.send_raw_transaction(args.into(), args.bare_output).await?
            },
            Command::Wallet(WalletCommands::Withdraw(args)) => {
                proc.withdraw(args.try_into()?, args.bare_output).await?
            },
            Command::Wallet(WalletCommands::GetRawTransaction(args)) => {
                proc.get_raw_transaction(args.into(), args.bare_output).await?
            },
            Command::Wallet(WalletCommands::TxHistory(args)) => proc.tx_history(args).await?,
            Command::Wallet(WalletCommands::ShowPrivKey(args)) => proc.show_priv_key(args.into()).await?,
            Command::Wallet(WalletCommands::ValidateAddress(args)) => proc.validate_address(args.into()).await?,
            Command::Wallet(WalletCommands::KmdRewardsInfo) => proc.get_kmd_rewards_info().await?,
            Command::Wallet(WalletCommands::ConvertAddress(args)) => proc.convert_address(args.try_into()?).await?,
            Command::Wallet(WalletCommands::ConvertUtxoAddress(args)) => proc.convert_utxo_address(args.into()).await?,
            Command::Wallet(WalletCommands::GetPublicKey) => proc.get_public_key().await?,
            Command::Wallet(WalletCommands::GetPublicKeyHash) => proc.get_public_key_hash().await?,
            Command::Task(TaskSubcommand::Status(TaskSubcommandStatus::Zcoin { task_id })) => {
                proc.enable_zcoin_status(*task_id, None).await?
            },
            Command::Task(TaskSubcommand::Cancel(TaskSubcommandCancel::Zcoin { task_id })) => {
                proc.enable_zcoin_cancel(*task_id).await?
            },
            Command::VersionStat(VersionStatCommands::AddNode(args)) => proc.version_stat_add_node(args.into()).await?,
            Command::VersionStat(VersionStatCommands::RemoveNode(args)) => {
                proc.version_stat_remove_node(args.into()).await?
            },
            Command::VersionStat(VersionStatCommands::StartCollect(args)) => {
                proc.version_stat_start_collection(args.into()).await?
            },
            Command::VersionStat(VersionStatCommands::StopCollect) => proc.version_stat_stop_collection().await?,
            Command::VersionStat(VersionStatCommands::UpdateCollect(args)) => {
                proc.version_stat_update_collection(args.into()).await?
            },
            Command::Message(MessageCommands::Sign(args)) => proc.sign_message(args.into()).await?,
            Command::Message(MessageCommands::Verify(args)) => proc.verify_message(args.into()).await?,
        }
        Ok(())
    }
}

pub fn get_cli_root() -> Result<PathBuf> {
    if let Ok(cli_root) = std::env::var("KOMODEFI_CLI_ROOT") {
        let cli_root = PathBuf::from(cli_root);
        fs::create_dir_all(&cli_root)
            .map_err(|error| error_anyhow!("Failed to create config_dir: {cli_root:?}, error: {error}"))?;
        Ok(cli_root)
    } else {
        let project_dirs = ProjectDirs::from(PROJECT_QUALIFIER, PROJECT_COMPANY, PROJECT_APP)
            .ok_or_else(|| error_anyhow!("Failed to get project_dirs"))?;
        let cli_root: PathBuf = project_dirs.config_dir().into();
        fs::create_dir_all(&cli_root)
            .map_err(|error| error_anyhow!("Failed to create config_dir: {cli_root:?}, error: {error}"))?;

        Ok(cli_root)
    }
}

#[derive(Debug, clap::Parser)]
pub(crate) struct GetOption {
    #[arg(long, short)]
    pub(crate) unhide: bool,
}

#[derive(Subcommand)]
enum ConfigSubcommand {
    #[command(about = "Set komodo komodefi cli configuration")]
    Set(SetConfigArgs),
    #[command(about = "Get komodo komodefi cli configuration")]
    Get(GetOption),
}
