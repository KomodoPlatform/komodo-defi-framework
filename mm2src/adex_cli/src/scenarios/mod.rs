mod helpers;
mod init_coins;
mod init_mm2_cfg;
mod inquire_extentions;
mod mm2_proc_mng;

use init_coins::init_coins;
use init_mm2_cfg::init_mm2_cfg;
pub use mm2_proc_mng::{get_status, start_process, stop_process};

pub fn init(cfg_file: &str, coins_file: &str) {
    if init_mm2_cfg(cfg_file).is_err() {
        return;
    }
    let _ = init_coins(coins_file);
}
