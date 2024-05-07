use crate::cli::get_cli_root;
use crate::error_anyhow;

use anyhow::{anyhow, Result};
use clap::Args;
use common::log::error;
use inquire::{Confirm, Text};
use std::fs::File;
use std::io::Read;

const DEFAULT_RPC_URL: &str = "127.0.0.1:7783";

#[derive(Args)]
#[group(required = false, multiple = true)]
pub(crate) struct SetConfigArgs {
    #[arg(long, short, help = "Set if you are going to set up a password")]
    pub(crate) password: Option<bool>,
    #[arg(long, short, visible_alias = "url", help = "KomoDeFi RPC API Uri. localhost:7783")]
    pub(crate) uri: Option<String>,
    #[arg(
        long,
        short,
        help = "Set `Yes` if you want to use secure connection with your mm2 rpc. RPC should supported secure!"
    )]
    pub(crate) secure_conn: Option<bool>,
    #[arg(long, short, help = "Set configuration from MM2.json config path")]
    pub(crate) from_path: bool,
}

impl SetConfigArgs {
    pub(crate) fn inquire(&mut self) -> Result<()> {
        self.inquire_secure_connection()?;
        self.inquire_uri()?;
        self.inquire_password()
    }

    fn inquire_password(&mut self) -> Result<()> {
        if self.password.is_none() {
            self.password = Confirm::new("Set if you are going to set up a password:")
                .with_default(false)
                .with_placeholder("No")
                .prompt()
                .map_err(|error| error_anyhow!("Failed to get password option: {error}"))?
                .into();
        }

        Ok(())
    }

    fn inquire_secure_connection(&mut self) -> Result<()> {
        if self.secure_conn.is_none() {
            self.secure_conn = Confirm::new("Use secure connection for rpc uri:")
                .with_default(false)
                .with_placeholder("No")
                .prompt()
                .map_err(|error| error_anyhow!("Failed to get secure_conn option: {error}"))?
                .into();
        }

        Ok(())
    }

    fn inquire_uri(&mut self) -> Result<()> {
        if self.uri.is_none() {
            self.uri = Text::new("What is the rpc_uri without https/http?:")
                .with_placeholder(DEFAULT_RPC_URL)
                .with_default(DEFAULT_RPC_URL)
                .prompt()
                .map_err(|error| error_anyhow!("Failed to get rpc_uri: {error}"))?
                .into();
        }
        Ok(())
    }

    pub(crate) fn source_from_path(&self) -> Result<(String, String)> {
        let mut root = get_cli_root()?;
        root.push("MM2.json");

        let mut file = File::open(root).map_err(|error| error_anyhow!("Failed to open path: {error}"))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .map_err(|error| error_anyhow!("Failed to read file: {error}"))?;

        let mm2: Mm2Config = serde_json::from_str(&String::from_utf8_lossy(&buffer))
            .map_err(|error| error_anyhow!("Failed to write json: {error}"))?;

        let scheme = if mm2.secure_conn.unwrap_or_default() {
            "https://"
        } else {
            "http://"
        };
        Ok((
            format!("{scheme}{}:{}", mm2.rpcip.trim(), mm2.rpcport),
            mm2.rpc_password.trim().to_string(),
        ))
    }
}

#[derive(serde::Deserialize)]
struct Mm2Config {
    rpcip: String,
    rpcport: u16,
    secure_conn: Option<bool>,
    rpc_password: String,
}
