mod activation_scheme_impl;
mod init_activation_scheme;

use crate::activation_scheme::init_activation_scheme::ACTIVATION_SCHEME_FILE;
use crate::adex_config::AdexConfig;

pub use activation_scheme_impl::{get_activation_scheme, ActivationScheme};
pub use init_activation_scheme::init_activation_scheme;

use std::path::PathBuf;

const COIN_ACTIVATION_SOURCE: &str = "https://stats.kmd.io/api/table/coin_activation/";

fn get_activation_scheme_path() -> Result<PathBuf, ()> {
    let mut config_path = AdexConfig::get_config_dir()?;
    config_path.push(ACTIVATION_SCHEME_FILE);
    Ok(config_path)
}

#[tokio::test]
async fn test_activation_scheme() {
    init_activation_scheme();
    let scheme = get_activation_scheme();
    let kmd_scheme = scheme.get_activation_method("KMD");
    assert!(kmd_scheme.is_some());
    let kmd_scheme = kmd_scheme.unwrap();
    assert_eq!(kmd_scheme.get("method").unwrap().as_str().unwrap(), "electrum");
    assert_eq!(kmd_scheme.get("servers").unwrap().as_array().unwrap().iter().count(), 3);
}
