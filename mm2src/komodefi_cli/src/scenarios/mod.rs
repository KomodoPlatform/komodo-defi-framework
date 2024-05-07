mod download_helper;
mod init_coins;
mod init_mm2_cfg;
mod inquire_extentions;
mod mm2_proc_mng;

use anyhow::Result;

use common::log::info;
use init_coins::init_coins;
use init_mm2_cfg::init_mm2_cfg;

use super::activation_scheme_db::init_activation_scheme;
use crate::cli::get_cli_root;

pub(super) use download_helper::download_binary_and_extract_to_bin_folder;
pub(super) use mm2_proc_mng::{get_status, start_process, stop_process};

pub(super) async fn init(cfg_file: &str, coins_file: &str) { let _ = init_impl(cfg_file, coins_file).await; }

async fn init_impl(cfg_file: &str, coins_file: &str) -> Result<()> {
    let root = get_cli_root()?;
    init_mm2_cfg(&root.join(cfg_file).to_string_lossy())?;
    init_coins(&root.join(coins_file).to_string_lossy()).await?;
    init_activation_scheme().await?;
    info!("Initialization done");
    Ok(())
}
