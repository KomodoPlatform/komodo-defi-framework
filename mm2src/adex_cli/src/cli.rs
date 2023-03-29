use clap::{App, Arg, SubCommand};
use log::error;
use std::env;

use crate::api_commands::{get_config, get_version, send_stop, set_config};
use crate::scenarios::{get_status, init, start_process, stop_process};

enum ConfigSubcommand {
    Set {
        set_password: bool,
        adex_uri: Option<String>,
    },
    Get,
}

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
    Kill,
    Status,
    Stop,
    Config(ConfigSubcommand),
    Version,
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
        .subcommand(SubCommand::with_name("kill").about("Stop mm2 service"))
        .subcommand(SubCommand::with_name("status").about("Get mm2 running status"))
        .subcommand(
            SubCommand::with_name("stop")
                .about("Stop mm2 through the API")
                .arg(Arg::with_name("mm-conf-path")),
        )
        .subcommand(
            SubCommand::with_name("config")
                .about("Config managements command set")
                .subcommand(
                    SubCommand::with_name("set")
                        .arg(
                            Arg::with_name("rpc-uri")
                                .long("rpc-uri")
                                .value_name("URI")
                                .help("ADex RPC API Uri. http://localhost::7783"),
                        )
                        .arg(
                            Arg::with_name("set-password")
                                .long("set-password")
                                .help("Set if you are going to set up a password"),
                        )
                        .about("Sets komodo adex cli configuration"),
                )
                .subcommand(SubCommand::with_name("get").about("Gets komodo adex cli configuration")),
        )
        .subcommand(SubCommand::with_name("version").about("Gets version of intermediary mm2 service"));

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
        ("kill", _) => Command::Kill,
        ("status", _) => Command::Status,
        ("stop", _) => Command::Stop,

        ("config", Some(config_matches)) => match config_matches.subcommand() {
            ("get", _) => Command::Config(ConfigSubcommand::Get),
            ("set", Some(config_set_matches)) => {
                let adex_uri = config_set_matches.value_of("rpc-uri").map(|s| s.to_owned());
                let set_password = config_set_matches.is_present("set-password");
                Command::Config(ConfigSubcommand::Set { set_password, adex_uri })
            },
            _ => {
                return;
            },
        },

        ("version", _) => Command::Version,
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
        Command::Kill => stop_process(),
        Command::Status => get_status(),
        Command::Stop => send_stop(),
        Command::Version => get_version(),
        Command::Config(ConfigSubcommand::Get) => get_config(),
        Command::Config(ConfigSubcommand::Set { set_password, adex_uri }) => set_config(set_password, adex_uri),
    }
}
