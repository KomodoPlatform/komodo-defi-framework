use clap::{App, Arg, SubCommand};
use log::error;
use std::env;

use crate::api_commands::{get_version, send_stop, set_config, show_config};
use crate::scenarios::{get_status, init, start_process, stop_process};

enum Command {
    Init {
        mm_coins_path: String,
        mm_conf_path: String,
    },
    Start {
        mm_conf_path: Option<String>,
        mm_coins_path: Option<String>,
        mm_log: Option<String>,
    },
    Stop,
    Status,
    SendStop,
    SetConfig {
        rpc_api_password: Option<String>,
        rpc_api_uri: Option<String>,
    },
    GetConfig,
    GetVersion,
}

pub fn process_cli() {
    let mut app = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .subcommand(
            SubCommand::with_name("init")
                .about("Initialize predefined mm2 coin set and configuration")
                .arg(
                    Arg::with_name("mm-coins-path")
                        .long("mm-coins-path")
                        .value_name("FILE")
                        .help("coin set file path")
                        .default_value("coins"),
                )
                .arg(
                    Arg::with_name("mm-conf-path")
                        .long("mm-conf-path")
                        .value_name("FILE")
                        .help("mm2 configuration file path")
                        .default_value("MM2.json"),
                ),
        )
        .subcommand(
            SubCommand::with_name("start")
                .about("Start mm2 service")
                .arg(
                    Arg::with_name("mm-conf-path")
                        .long("mm-conf-path")
                        .value_name("FILE")
                        .help("mm2 configuration file path"),
                )
                .arg(
                    Arg::with_name("mm-coins-path")
                        .long("mm-coins-path")
                        .value_name("FILE")
                        .help("coin set file path"),
                )
                .arg(
                    Arg::with_name("mm-log")
                        .long("mm-log")
                        .value_name("FILE")
                        .help("log file path"),
                ),
        )
        .subcommand(SubCommand::with_name("stop").about("Stop mm2 service"))
        .subcommand(SubCommand::with_name("status").about("Get mm2 running status"))
        .subcommand(
            SubCommand::with_name("send-stop")
                .about("Stop mm2 through the API")
                .arg(Arg::with_name("mm-conf-path")),
        )
        .subcommand(
            SubCommand::with_name("set-config")
                .about("Set adex cli configuration")
                .arg(
                    Arg::with_name("rpc-api-password")
                        .long("password")
                        .value_name("PASSWORD")
                        .help("password the use ADex RPC API"),
                )
                .arg(
                    Arg::with_name("rpc-api-uri")
                        .long("uri")
                        .value_name("URI")
                        .help("ADex RPC API Uri"),
                ),
        )
        .subcommand(SubCommand::with_name("get-config").about("Gets komodo adex cli configuration"))
        .subcommand(SubCommand::with_name("get-version").about("Gets version of intermediary mm2 service"));

    let matches = app.clone().get_matches();

    let command = match matches.subcommand() {
        ("init", Some(init_matches)) => {
            let mm_coins_path = init_matches.value_of("mm-coins-path").unwrap_or("coins").to_owned();
            let mm_conf_path = init_matches.value_of("mm-conf-path").unwrap_or("MM2.json").to_owned();
            Command::Init {
                mm_coins_path,
                mm_conf_path,
            }
        },
        ("start", Some(start_matches)) => {
            let mm_conf_path = start_matches.value_of("mm-conf-path").map(|s| s.to_owned());
            let mm_coins_path = start_matches.value_of("mm-coins-path").map(|s| s.to_owned());
            let mm_log = start_matches.value_of("mm-log").map(|s| s.to_owned());
            Command::Start {
                mm_conf_path,
                mm_coins_path,
                mm_log,
            }
        },
        ("stop", _) => Command::Stop,
        ("status", _) => Command::Status,
        ("send-stop", Some(start_stop_matches)) => Command::SendStop,
        ("set-config", Some(set_config_matches)) => {
            let rpc_api_password = set_config_matches.value_of("rpc-api-password").map(|s| s.to_owned());
            let rpc_api_uri = set_config_matches.value_of("rpc-api-uri").map(|s| s.to_owned());
            Command::SetConfig {
                rpc_api_password,
                rpc_api_uri,
            }
        },
        ("get-config", _) => Command::GetConfig,
        ("get-version", _) => Command::GetVersion,
        _ => {
            let _ = app
                .print_long_help()
                .map_err(|error| error!("Failed to print_long_help: {error}"));
            return;
        },
    };

    match command {
        Command::Init {
            mm_coins_path: coins_file,
            mm_conf_path: mm2_cfg_file,
        } => init(&mm2_cfg_file, &coins_file),
        Command::Start {
            mm_conf_path: mm2_cfg_file,
            mm_coins_path: coins_file,
            mm_log: log_file,
        } => start_process(&mm2_cfg_file, &coins_file, &log_file),
        Command::Stop => stop_process(),
        Command::Status => get_status(),
        Command::SendStop => send_stop(),
        Command::SetConfig {
            rpc_api_password,
            rpc_api_uri,
        } => set_config(rpc_api_password, rpc_api_uri),
        Command::GetConfig => show_config(),
        Command::GetVersion => get_version(),
    }
}
