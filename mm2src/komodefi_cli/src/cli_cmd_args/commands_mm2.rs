use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum Mm2Commands {
    #[command(about = "Start mm2 instance")]
    Start {
        #[arg(long, visible_alias = "conf", help = "mm2 configuration file path")]
        mm_conf_path: Option<String>,
        #[arg(long, visible_alias = "coins", help = "Coin set file path")]
        mm_coins_path: Option<String>,
        #[arg(long, visible_alias = "log", help = "Log file path")]
        mm_log: Option<String>,
    },
    #[command(about = "Stop mm2 using API")]
    Stop,
    #[command(about = "Kill mm2 process")]
    Kill,
    #[command(about = "Check if mm2 is running")]
    Status,
    #[command(about = "Get version of intermediary mm2 service")]
    Version,
    #[command(about = "Download latest available mm2 binary version and extract to bin folder for use.")]
    Download,
}
