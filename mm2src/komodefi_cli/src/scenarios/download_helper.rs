use crate::cli::get_cli_root;

use anyhow::Result;
use common::log::{debug, error, info};
use serde::Deserialize;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::process::{Command, Stdio};
use zip::ZipArchive;

const BINARY_HOST_URL: &str = "https://api.github.com/repos/KomodoPlatform/komodo-defi-framework/releases";

#[cfg(all(unix, not(target_os = "macos")))]
const DOWNLOAD_NAME: &str = "Linux-Release.";
#[cfg(target_os = "macos")]
const DOWNLOAD_NAME: &str = "Darwin-Release.";
#[cfg(target_os = "windows")]
const DOWNLOAD_NAME: &str = "Win64.";

#[cfg(not(target_os = "windows"))]
const BINARY_NAME: &str = "mm2";
#[cfg(target_os = "windows")]
const BINARY_NAME: &str = "mm2.exe";

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct Releases {
    assets: Vec<Asset>,
}

/// Let komodefi_cli download and extract release binary for use.
pub(crate) async fn download_binary_and_extract_to_bin_folder() -> Result<()> {
    // Create a reusable reqwest client with common headers
    let client = reqwest::Client::builder().user_agent("from-tty").build()?;
    let response = client.get(BINARY_HOST_URL).send().await?;
    let releases: Vec<Releases> = response.json().await?;

    // Extract the download URL for the latest release
    if let Some(download_url) = releases.get(0).and_then(|release| {
        release
            .assets
            .iter()
            .find(|asset| asset.name.contains(DOWNLOAD_NAME))
            .map(|asset| &asset.browser_download_url)
    }) {
        info!("release: {download_url}");
        // Download the ZIP file
        let zip_data = client.get(download_url).send().await?.bytes().await?;
        // Create directories if they don't exist
        let cli_root = get_cli_root()?;
        let mut zip_path = cli_root.clone();
        zip_path.push("mm2.zip");
        // Save the ZIP file
        let mut zip_file = File::create(&zip_path)?;
        zip_file.write_all(&zip_data)?;
        // Extract only mm2 binary file from the folder
        extract_file_from_zip(&zip_path, BINARY_NAME).await?;
        info!(
            "Binary downloaded and extracted to {} folder!",
            cli_root.to_string_lossy()
        );
        Ok(())
    } else {
        error!("No matching release found");
        Err(anyhow::anyhow!("No matching release found"))
    }
}

/// Extract binary file from zip file
async fn extract_file_from_zip(zip_path: &std::path::Path, file_name: &str) -> Result<(), anyhow::Error> {
    let file = File::open(zip_path)?;
    let reader = std::io::BufReader::new(file);
    let mut archive = ZipArchive::new(reader)?;

    // Create directories if they don't exist and extract binary
    let mut config_path = get_cli_root()?;
    archive.extract(&config_path)?;

    // Check binary version
    let version = get_binary_version(&format!("{}/{file_name}", config_path.to_string_lossy())).await?;
    info!("running {version}");

    // Delete zip
    config_path.push("mm2.zip");
    fs::remove_file(config_path)?;
    debug!("deleted downloaded_file.zip after use");

    Ok(())
}

async fn get_binary_version(binary_path: &str) -> Result<String> {
    info!("{binary_path:?}");
    let output = Command::new(binary_path)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(version)
    } else {
        let error_message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(anyhow::anyhow!("Failed to get binary version: {}", error_message))
    }
}
