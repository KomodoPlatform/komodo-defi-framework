use directories::ProjectDirs;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

use crate::helpers::rewrite_json_file;

const PROJECT_QUALIFIER: &str = "com";
const PROJECT_COMPANY: &str = "komodoplatform";
const PROJECT_APP: &str = "adex-cli";
const ADEX_CFG: &str = "adex_cfg.json";

#[derive(Deserialize, Serialize, Debug, Default)]
pub struct AdexConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    rpc_password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rpc_uri: Option<String>,
}

impl AdexConfig {
    pub fn rpc_password(&self) -> String { self.rpc_password.as_ref().unwrap().clone() }
    pub fn rpc_uri(&self) -> String { self.rpc_uri.as_ref().unwrap().clone() }
}

impl Display for AdexConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if !self.is_set() {
            return writeln!(f, "adex configuration is not set");
        }
        if let Some(rpc_api_uri) = &self.rpc_uri {
            writeln!(f, "adex RPC API Url: {}", rpc_api_uri)?
        };

        if self.rpc_password.is_some() {
            writeln!(f, "adex RPC API pwd: *************")?
        }
        Ok(())
    }
}

impl AdexConfig {
    pub fn read_config() -> Result<AdexConfig, ()> {
        let config = AdexConfig::from_config_path().map_err(|_| error!("Failed to get adex_config"))?;
        match config {
            config @ AdexConfig {
                rpc_password: Some(_),
                rpc_uri: Some(_),
            } => Ok(config),
            _ => {
                warn!("Failed to process, adex_config is not fully set");
                Err(())
            },
        }
    }

    pub fn is_set(&self) -> bool { self.rpc_uri.is_some() && self.rpc_password.is_some() }

    pub fn get_config_dir() -> Result<PathBuf, ()> {
        let project_dirs = ProjectDirs::from(PROJECT_QUALIFIER, PROJECT_COMPANY, PROJECT_APP)
            .ok_or_else(|| error!("Failed to get project_dirs"))?;
        let config_path: PathBuf = project_dirs.config_dir().into();
        fs::create_dir_all(&config_path)
            .map_err(|error| error!("Failed to create config_dir: {config_path:?}, error: {error}"))?;
        Ok(config_path)
    }

    pub fn get_config_path() -> Result<PathBuf, ()> {
        let mut config_path = Self::get_config_dir()?;
        config_path.push(ADEX_CFG);
        Ok(config_path)
    }

    pub fn from_config_path() -> Result<AdexConfig, ()> {
        let config_path = Self::get_config_path()?;

        if !config_path.exists() {
            info!("Config is not set");
            return Err(());
        }
        Self::read_from(&config_path)
    }

    pub fn write_to_config_path(&self) -> Result<(), ()> {
        let config_path = Self::get_config_path()?;
        self.write_to(&config_path)
    }

    fn read_from(cfg_path: &Path) -> Result<AdexConfig, ()> {
        let adex_path_str = cfg_path.to_str().unwrap_or("Undefined");
        let adex_cfg_file = fs::File::open(cfg_path).map_err(|error| {
            error!("Failed to open: {adex_path_str}, error: {error}");
        })?;

        serde_json::from_reader(adex_cfg_file)
            .map_err(|error| error!("Failed to read adex_cfg to read from: {adex_path_str}, error: {error}"))
    }

    fn write_to(&self, cfg_path: &Path) -> Result<(), ()> {
        let Some(adex_path_str) = cfg_path.to_str() else {
            error!("Failed to get cfg_path as str");
            return Err(());
        };
        rewrite_json_file(self, adex_path_str)
    }

    pub fn set_rpc_password(&mut self, rpc_password: Option<String>) { self.rpc_password = rpc_password; }

    pub fn set_rpc_uri(&mut self, rpc_uri: Option<String>) { self.rpc_uri = rpc_uri; }
}
