use common::log::error;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::ops::Deref;

pub fn rewrite_data_file<T>(data: T, file: &str) -> Result<(), ()>
where
    T: Deref<Target = [u8]>,
{
    let mut writer = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(file)
        .map_err(|error| {
            error!("Failed to open {file}: {error}");
        })?;

    writer.write(&data).map_err(|error| {
        error!("Failed to write data into {file}: {error}");
    })?;
    Ok(())
}

pub fn rewrite_json_file<T>(value: &T, file: &str) -> Result<(), ()>
where
    T: Serialize,
{
    let data = serde_json::to_vec_pretty(value).map_err(|error| {
        error!("Failed to serialize data {error}");
    })?;
    rewrite_data_file(data, file)
}
