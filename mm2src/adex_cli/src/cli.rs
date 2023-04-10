use clap::{App, Arg, SubCommand};
use log::error;
use std::env;

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
        .subcommand(SubCommand::with_name("stop").about("Stop mm2 instance"))
        .subcommand(SubCommand::with_name("status").about("Get mm2 running status"));

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
    }
}
