use anyhow::{anyhow, Result};
use common::log::error;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::ops::Deref;
use std::path::Path;

use crate::error_anyhow;

pub(crate) fn rewrite_data_file<T>(data: T, file: &str) -> Result<()>
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

    Ok(())
}

pub(crate) fn rewrite_json_file<T>(value: &T, file: &str) -> Result<()>
where
    T: Serialize,
{
    let data = serde_json::to_vec_pretty(value).map_err(|error| error_anyhow!("Failed to serialize data {error}"))?;
    rewrite_data_file(data, file)
}

pub(crate) fn read_json_file<T>(file: &Path) -> Result<T>
where
    T: for<'a> Deserialize<'a>,
{
    let reader = fs::File::open(file).map_err(|error| error_anyhow!("Failed to open {file:?}, error: {error}"))?;
    serde_json::from_reader(reader)
        .map_err(|error| error_anyhow!("Failed to read json from data: {file:?}, error: {error}"))
}
