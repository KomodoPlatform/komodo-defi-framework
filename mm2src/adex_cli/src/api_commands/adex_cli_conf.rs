use directories::ProjectDirs;
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::helpers::rewrite_json_file;

const PROJECT_QUALIFIER: &str = "com";
const PROJECT_COMPANY: &str = "komodoplatform";
const PROJECT_APP: &str = "adex-cli";
const ADEX_CFG: &str = "adex_cfg.json";

#[derive(Deserialize, Serialize)]
pub struct AdexCliConf {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rpc_api_password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rpc_api_uri: Option<String>,
}

pub fn show_config() {
    let Ok(adex_cfg) = AdexCliConf::from_config_path() else { return; };

    if let Some(rpc_api_uri) = adex_cfg.rpc_api_uri {
        info!("Adex RPC API Url: {}", rpc_api_uri)
    };

    if let Some(rpc_api_password) = adex_cfg.rpc_api_password {
        info!("Adex RPC API pwd: *************")
    };
}

pub fn set_config(rpc_api_password: Option<String>, rpc_api_uri: Option<String>) {
    match AdexCliConf::get_config_dir() {
        Ok(ref config_dir) => {
            if let Err(error) = fs::create_dir_all(config_dir) {
                error!("Failed to create config_dir: {config_dir:?}, error: {error}");
                return;
            };
        },
        Err(_) => return,
    }

    let mut adex_cfg = AdexCliConf::from_config_path().unwrap_or_else(|()| AdexCliConf::new());
    if rpc_api_password.is_some() {
        adex_cfg.rpc_api_password = rpc_api_password;
    }
    if rpc_api_uri.is_some() {
        adex_cfg.rpc_api_uri = rpc_api_uri;
    }
    if let Ok(_) = adex_cfg.write_to_config_path() {
        info!("Configuration has been set");
    }
}

impl AdexCliConf {
    fn new() -> Self {
        Self {
            rpc_api_password: None,
            rpc_api_uri: None,
        }
    }

    pub fn is_set(&self) -> bool { self.rpc_api_uri.is_some() && self.rpc_api_password.is_some() }

    pub fn get_config_dir() -> Result<PathBuf, ()> {
        let project_dirs = ProjectDirs::from(PROJECT_QUALIFIER, PROJECT_COMPANY, PROJECT_APP)
            .ok_or_else(|| error!("Failed to get project_dirs"))?;
        let mut config_path: PathBuf = project_dirs.config_dir().into();
        Ok(config_path)
    }

    pub fn get_config_path() -> Result<PathBuf, ()> {
        let mut config_path = Self::get_config_dir()?;
        config_path.push(ADEX_CFG);
        Ok(config_path)
    }

    pub fn from_config_path() -> Result<AdexCliConf, ()> {
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

    fn read_from(cfg_path: &Path) -> Result<AdexCliConf, ()> {
        let adex_path_str = cfg_path.to_str().unwrap_or("Undefined");
        let adex_cfg_file = fs::File::open(&cfg_path).map_err(|error| {
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
        rewrite_json_file(self, &adex_path_str)
    }
}
