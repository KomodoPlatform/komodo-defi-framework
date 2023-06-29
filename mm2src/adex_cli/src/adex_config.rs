use anyhow::{anyhow, bail, Result};
use directories::ProjectDirs;
use inquire::Password;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

use crate::adex_proc::SmartFractPrecision;
use crate::helpers::rewrite_json_file;
use crate::logging::{error_anyhow, warn_bail};

const PROJECT_QUALIFIER: &str = "com";
const PROJECT_COMPANY: &str = "komodoplatform";
const PROJECT_APP: &str = "adex-cli";
const ADEX_CFG: &str = "adex_cfg.json";

const PRICE_PRECISION_MIN: usize = 8;
const PRICE_PRECISION_MAX: usize = 8;
const VOLUME_PRECISION_MIN: usize = 2;
const VOLUME_PRECISION_MAX: usize = 5;
const VOLUME_PRECISION: SmartFractPrecision = (VOLUME_PRECISION_MIN, VOLUME_PRECISION_MAX);
const PRICE_PRECISION: SmartFractPrecision = (PRICE_PRECISION_MIN, PRICE_PRECISION_MAX);

pub(super) fn get_config() {
    let Ok(adex_cfg) = AdexConfigImpl::from_config_path() else { return; };
    info!("{}", adex_cfg)
}

pub(super) fn set_config(set_password: bool, rpc_api_uri: Option<String>) -> Result<()> {
    assert!(set_password || rpc_api_uri.is_some());
    let mut adex_cfg = AdexConfigImpl::from_config_path().unwrap_or_else(|_| AdexConfigImpl::default());

    if set_password {
        let rpc_password = Password::new("Enter RPC API password:")
            .prompt()
            .map_err(|error| error_anyhow!("Failed to get rpc_api_password: {error}"))?;
        adex_cfg.set_rpc_password(rpc_password);
    }

    if let Some(rpc_api_uri) = rpc_api_uri {
        adex_cfg.set_rpc_uri(rpc_api_uri);
    }

    adex_cfg.write_to_config_path()?;
    info!("Configuration has been set");

    Ok(())
}

pub(super) trait AdexConfig {
    fn rpc_password(&self) -> Option<String>;
    fn rpc_uri(&self) -> Option<String>;
    fn orderbook_price_precision(&self) -> &SmartFractPrecision;
    fn orderbook_volume_precision(&self) -> &SmartFractPrecision;
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub(super) struct AdexConfigImpl {
    #[serde(skip_serializing_if = "Option::is_none")]
    rpc_password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rpc_uri: Option<String>,
}

impl AdexConfig for AdexConfigImpl {
    fn rpc_password(&self) -> Option<String> { self.rpc_password.clone() }
    fn rpc_uri(&self) -> Option<String> { self.rpc_uri.clone() }
    fn orderbook_price_precision(&self) -> &SmartFractPrecision { &PRICE_PRECISION }
    fn orderbook_volume_precision(&self) -> &SmartFractPrecision { &VOLUME_PRECISION }
}

impl Display for AdexConfigImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if !self.is_set() {
            return writeln!(f, "adex configuration is not set");
        }
        writeln!(
            f,
            "mm2 RPC URL: {}",
            self.rpc_uri.as_ref().expect("Expected rpc_uri is set")
        )?;
        writeln!(f, "mm2 RPC password: *************")?;
        Ok(())
    }
}

impl AdexConfigImpl {
    #[cfg(test)]
    pub(super) fn new(rpc_password: &str, rpc_uri: &str) -> Self {
        Self {
            rpc_password: Some(rpc_password.to_string()),
            rpc_uri: Some(rpc_uri.to_string()),
        }
    }

    #[cfg(not(test))]
    pub(super) fn read_config() -> Result<AdexConfigImpl> {
        let config = AdexConfigImpl::from_config_path()?;
        if config.rpc_password.is_none() {
            warn!("Configuration is not complete, no rpc_password in there");
        }
        if config.rpc_uri.is_none() {
            warn!("Configuration is not complete, no rpc_uri in there");
        }
        Ok(config)
    }

    fn is_set(&self) -> bool { self.rpc_uri.is_some() && self.rpc_password.is_some() }

    pub(super) fn get_config_dir() -> Result<PathBuf> {
        let project_dirs = ProjectDirs::from(PROJECT_QUALIFIER, PROJECT_COMPANY, PROJECT_APP)
            .ok_or_else(|| error_anyhow!("Failed to get project_dirs"))?;
        let config_path: PathBuf = project_dirs.config_dir().into();
        fs::create_dir_all(&config_path)
            .map_err(|error| error_anyhow!("Failed to create config_dir: {config_path:?}, error: {error}"))?;
        Ok(config_path)
    }

    fn get_config_path() -> Result<PathBuf> {
        let mut config_path = Self::get_config_dir()?;
        config_path.push(ADEX_CFG);
        Ok(config_path)
    }

    fn from_config_path() -> Result<AdexConfigImpl> {
        let config_path = Self::get_config_path()?;

        if !config_path.exists() {
            warn_bail!("Config is not set")
        }
        Self::read_from(&config_path)
    }

    fn write_to_config_path(&self) -> Result<()> {
        let config_path = Self::get_config_path()?;
        self.write_to(&config_path)
    }

    fn read_from(cfg_path: &Path) -> Result<AdexConfigImpl> {
        let adex_path_str = cfg_path.to_str().unwrap_or("Undefined");
        let adex_cfg_file = fs::File::open(cfg_path)
            .map_err(|error| error_anyhow!("Failed to open: {adex_path_str}, error: {error}"))?;

        serde_json::from_reader(adex_cfg_file)
            .map_err(|error| error_anyhow!("Failed to read adex_cfg to read from: {adex_path_str}, error: {error}"))
    }

    fn write_to(&self, cfg_path: &Path) -> Result<()> {
        let adex_path_str = cfg_path
            .to_str()
            .ok_or_else(|| error_anyhow!("Failed to get cfg_path as str"))?;
        rewrite_json_file(self, adex_path_str)
    }

    fn set_rpc_password(&mut self, rpc_password: String) { self.rpc_password.replace(rpc_password); }

    fn set_rpc_uri(&mut self, rpc_uri: String) { self.rpc_uri.replace(rpc_uri); }
}
