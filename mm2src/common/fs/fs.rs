use crate::log::error;
use crate::mm_error::prelude::*;
use async_std::fs as async_fs;
use derive_more::Display;
use futures::AsyncWriteExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{self as json, Error as JsonError};
use std::ffi::OsStr;
use std::fs::{self, DirEntry};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub type FsJsonResult<T> = Result<T, MmError<FsJsonError>>;
pub type IoResult<T> = Result<T, MmError<io::Error>>;

#[derive(Display)]
pub enum FsJsonError {
    IoReading(io::Error),
    IoWriting(io::Error),
    Serializing(JsonError),
    Deserializing(JsonError),
}

pub mod file_lock;

pub fn slurp(path: &dyn AsRef<Path>) -> Result<Vec<u8>, String> { Ok(gstuff::slurp(path)) }

pub fn safe_slurp(path: &dyn AsRef<Path>) -> Result<Vec<u8>, String> {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(ref err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return ERR!("Can't open {:?}: {}", path.as_ref(), err),
    };
    let mut buf = Vec::new();
    try_s!(file.read_to_end(&mut buf));
    Ok(buf)
}

pub fn remove_file(path: &dyn AsRef<Path>) -> Result<(), String> {
    try_s!(fs::remove_file(path));
    Ok(())
}

pub async fn remove_file_async<P: AsRef<Path>>(path: P) -> IoResult<()> {
    Ok(async_fs::remove_file(path.as_ref()).await?)
}

pub fn write(path: &dyn AsRef<Path>, contents: &dyn AsRef<[u8]>) -> Result<(), String> {
    try_s!(fs::write(path, contents));
    Ok(())
}

/// Read a folder asynchronously and return a list of files.
pub async fn read_dir_async<P: AsRef<Path>>(dir: P) -> IoResult<Vec<PathBuf>> {
    use futures::StreamExt;

    let mut result = Vec::new();
    let mut entries = async_fs::read_dir(dir.as_ref()).await?;

    while let Some(entry) = entries.next().await {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                error!("Error '{}' reading from dir {}", e, dir.as_ref().display());
                continue;
            },
        };
        result.push(entry.path().into());
    }
    Ok(result)
}

/// Read a folder and return a list of files with their last-modified ms timestamps.
pub fn read_dir(dir: &dyn AsRef<Path>) -> Result<Vec<(u64, PathBuf)>, String> {
    let entries = try_s!(dir.as_ref().read_dir())
        .filter_map(|dir_entry| {
            let entry = match dir_entry {
                Ok(ent) => ent,
                Err(e) => {
                    error!("Error '{}' reading from dir {}", e, dir.as_ref().display());
                    return None;
                },
            };

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    error!("Error '{}' getting file {} meta", e, entry.path().display());
                    return None;
                },
            };

            let m_time = match metadata.modified() {
                Ok(time) => time,
                Err(e) => {
                    error!("Error '{}' getting file {} m_time", e, entry.path().display());
                    return None;
                },
            };

            let lm = m_time.duration_since(UNIX_EPOCH).expect("!duration_since").as_millis();
            assert!(lm < u64::MAX as u128);
            let lm = lm as u64;

            let path = entry.path();
            if path.extension() == Some(OsStr::new("json")) {
                Some((lm, path))
            } else {
                None
            }
        })
        .collect();

    Ok(entries)
}

pub async fn read_json<T>(path: &Path) -> FsJsonResult<Option<T>>
where
    T: DeserializeOwned,
{
    let content = match async_fs::read(path).await {
        Ok(content) => content,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return MmError::err(FsJsonError::IoReading(e)),
    };
    json::from_slice(&content).map_to_mm(FsJsonError::Deserializing)
}

/// Read the `dir_path` entries trying to deserialize each as the `T` type.
/// Please note that files that couldn't be deserialized are skipped.
pub async fn read_dir_json<T>(dir_path: &Path) -> FsJsonResult<Vec<T>>
where
    T: DeserializeOwned,
{
    let json_ext = Some(OsStr::new("json"));
    let entries: Vec<_> = read_dir_async(dir_path)
        .await
        .mm_err(FsJsonError::IoReading)?
        .into_iter()
        .filter(|path| path.extension() == json_ext)
        .collect();
    let type_name = std::any::type_name::<T>();

    let mut result = Vec::new();
    for file_path in entries {
        match read_json(&file_path).await {
            Ok(Some(t)) => result.push(t),
            Ok(None) => {
                error!(
                    "Expected '{}' type at the file {}, found 'None'",
                    type_name,
                    file_path.display()
                );
                continue;
            },
            Err(e) => {
                error!(
                    "Error reading '{}' from the file {}: {}",
                    type_name,
                    file_path.display(),
                    e
                );
                continue;
            },
        };
    }
    Ok(result)
}

pub async fn write_json<T>(t: &T, path: &Path) -> FsJsonResult<()>
where
    T: Serialize,
{
    let content = json::to_vec(t).map_to_mm(FsJsonError::Serializing)?;

    let fs_fut = async {
        let mut file = async_fs::File::create(&path).await?;
        file.write_all(&content).await?;
        file.flush().await?;
        Ok(())
    };

    let res: io::Result<_> = fs_fut.await;
    res.map_to_mm(FsJsonError::IoWriting)
}

pub fn json_dir_entries(path: &dyn AsRef<Path>) -> Result<Vec<DirEntry>, String> {
    Ok(try_s!(path.as_ref().read_dir())
        .filter_map(|dir_entry| {
            let entry = match dir_entry {
                Ok(ent) => ent,
                Err(e) => {
                    error!("Error '{}' reading from dir {}", e, path.as_ref().display());
                    return None;
                },
            };

            if entry.path().extension() == Some(OsStr::new("json")) {
                Some(entry)
            } else {
                None
            }
        })
        .collect())
}
