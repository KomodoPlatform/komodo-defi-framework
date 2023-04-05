const MM2_CONFIG_FILE_DEFAULT: &str = "MM2.json";
const COINS_FILE_DEFAULT: &str = "coins";

use clap::{Parser, Subcommand};

use crate::api_commands::{activate, balance, get_config, get_enabled, get_orderbook, get_version, send_stop,
                          set_config};
use crate::scenarios::{get_status, init, start_process, stop_process};

#[derive(Subcommand)]
enum Command {
    #[command(about = "Initialize predefined mm2 coin set and configuration")]
    Init {
        #[arg(long, help = "coin set file path", default_value = COINS_FILE_DEFAULT)]
        mm_coins_path: String,
        #[arg(long, help = "mm2 configuration file path", default_value = MM2_CONFIG_FILE_DEFAULT)]
        mm_conf_path: String,
    },
    #[command(about = "Start mm2 service")]
    Start {
        #[arg(long, help = "mm2 configuration file path")]
        mm_conf_path: Option<String>,
        #[arg(long, help = "coin set file path")]
        mm_coins_path: Option<String>,
        #[arg(long, help = "log file path")]
        mm_log: Option<String>,
    },
    #[command(about = "Stop mm2 instance")]
    Stop,
    #[command(about = "Get mm2 running status")]
    Status,
    #[command(subcommand, about = "Gets version of intermediary mm2 service")]
    Version,
    #[command(subcommand, about = "Config management command set")]
    Config(ConfigSubcommand),
    #[command(subcommand, about = "Assets related operations: activate, balance etc.")]
    Asset(AssetSubcommand),
    #[command(subcommand, about = "Kill mm2 processes")]
    Kill,
    #[command(about = "Gets orderbook")]
    Orderbook {
        #[arg(help = "Base currency of a pair")]
        base: String,
        #[arg(help = "Related currency, also can be called \"quote currency\" according to exchange terms")]
        rel: String,
    },
}

#[derive(Subcommand)]
enum ConfigSubcommand {
    #[command(about = "Sets komodo adex cli configuration")]
    Set {
        #[arg(long, help = "Adex RPC API Uri. http://localhost::7783")]
        set_password: bool,
        #[arg(long, name = "URI", help = "Set if you are going to set up a password")]
        adex_uri: Option<String>,
    },
    #[command(about = "Gets komodo adex cli configuration")]
    Get,
}

#[derive(Subcommand)]
enum AssetSubcommand {
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
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub async fn execute() {
        let mut parsed_cli = Self::parse();
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
            Command::Version => get_version().await,
            Command::Kill => stop_process(),
            Command::Status => get_status(),
            Command::Stop => send_stop().await,
            Command::Config(ConfigSubcommand::Set { set_password, adex_uri }) => {
                set_config(*set_password, adex_uri.take())
            },
            Command::Config(ConfigSubcommand::Get) => get_config(),
            Command::Asset(AssetSubcommand::Enable { asset }) => activate(asset).await,
            Command::Asset(AssetSubcommand::Balance { asset }) => balance(asset).await,
            Command::Asset(AssetSubcommand::GetEnabled) => get_enabled().await,
            Command::Orderbook { base, rel } => get_orderbook(base, rel).await,
        }
    }
}
