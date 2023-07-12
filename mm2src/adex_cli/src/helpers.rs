use anyhow::{anyhow, Result};
use common::log::error;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::ops::Deref;
#[cfg(unix)] use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::error_anyhow;

pub(crate) fn rewrite_data_file<T>(data: T, file: &str, _unix_mode: Option<u32>) -> Result<()>
where
    T: Deref<Target = [u8]>,
{
    let mut writer = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(file)
        .map_err(|error| error_anyhow!("Failed to open {file}: {error}"))?;

    writer
        .write(&data)
        .map_err(|error| error_anyhow!("Failed to write data into {file}: {error}"))?;

    #[cfg(unix)]
    if let Some(unix_mode) = _unix_mode {
        let mut perms = fs::metadata(file)?.permissions();
        perms.set_mode(unix_mode);
        fs::set_permissions(file, perms)?;
    }

    Ok(())
}

pub(crate) fn rewrite_json_file<T>(value: &T, file: &str, unix_mode: Option<u32>) -> Result<()>
where
    T: Serialize,
{
    let data = serde_json::to_vec_pretty(value).map_err(|error| error_anyhow!("Failed to serialize data {error}"))?;
    rewrite_data_file(data, file, unix_mode)
}

pub(crate) fn read_json_file<T>(file: &Path) -> Result<T>
where
    T: for<'a> Deserialize<'a>,
{
    let reader = fs::File::open(file).map_err(|error| error_anyhow!("Failed to open {file:?}, error: {error}"))?;
    serde_json::from_reader(reader)
        .map_err(|error| error_anyhow!("Failed to read json from data: {file:?}, error: {error}"))
}
