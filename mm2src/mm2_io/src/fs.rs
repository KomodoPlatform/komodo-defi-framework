use std::ffi::OsStr;
use std::fs::{self, DirEntry};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use async_std::fs as async_fs;
use common::log::{error, LogOnError};
use derive_more::Display;
use futures::AsyncWriteExt;
use gstuff::{try_s, ERR, ERRL};
use mm2_err_handle::prelude::*;
use rand::random;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{self as json, Error as JsonError};

pub type FsJsonResult<T> = Result<T, MmError<FsJsonError>>;
pub type IoResult<T> = Result<T, MmError<io::Error>>;

#[derive(Display)]
pub enum FsJsonError {
    IoReading(io::Error),
    IoWriting(io::Error),
    Serializing(JsonError),
    Deserializing(JsonError),
}

#[inline]
pub fn invalid_data_err<Error>(msg: &str, err: Error) -> io::Error
where
    Error: std::fmt::Display,
{
    io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {}", msg, err))
}

pub fn check_dir_operations(dir_path: &Path) -> Result<(), io::Error> {
    let r: [u8; 32] = random();
    let mut check: Vec<u8> = Vec::with_capacity(r.len());
    let fname = dir_path.join("checkval");
    let mut fp = fs::File::create(&fname)?;
    fp.write_all(&r)?;
    drop(fp);
    let mut fp = fs::File::open(&fname)?;
    fp.read_to_end(&mut check)?;
    if check.len() != r.len() {
        return Err(io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!("Expected same data length when reading file: {:?}", fname),
        ));
    }
    if check != r {
        return Err(io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Expected the same {:?} data: {:?} != {:?}", fname, r, check),
        ));
    }
    Ok(())
}

/// Invokes `OS_ensure_directory`,
/// then prints an error and returns `false` if the directory is not writable.
pub fn ensure_dir_is_writable(dir_path: &Path) -> bool {
    if dir_path.exists() && !dir_path.is_dir() {
        error!("The {} is not a directory", dir_path.display());
        return false;
    } else if let Err(e) = std::fs::create_dir_all(dir_path) {
        error!("Could not create dir {}, error {}", dir_path.display(), e);
        return false;
    }
    check_dir_operations(dir_path).error_log_passthrough().is_ok()
}

pub fn ensure_file_is_writable(file_path: &Path) -> Result<(), String> {
    if fs::File::open(file_path).is_err() {
        // try to create file if opening fails
        if let Err(e) = fs::OpenOptions::new().write(true).create_new(true).open(file_path) {
            return ERR!("{} when trying to create the file {}", e, file_path.display());
        }
    } else {
        // try to open file in write append mode
        if let Err(e) = fs::OpenOptions::new().write(true).append(true).open(file_path) {
            return ERR!(
                "{} when trying to open the file {} in write mode",
                e,
                file_path.display()
            );
        }
    }
    Ok(())
}

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

pub async fn write_json<T>(t: &T, path: &Path, use_tmp_file: bool) -> FsJsonResult<()>
where
    T: Serialize,
{
    let content = json::to_vec(t).map_to_mm(FsJsonError::Serializing)?;

    let path_tmp = if use_tmp_file {
        PathBuf::from(format!("{}.tmp", path.display()))
    } else {
        path.to_path_buf()
    };

    let fs_fut = async {
        let mut file = async_fs::File::create(&path_tmp).await?;
        file.write_all(&content).await?;
        file.flush().await?;

        if use_tmp_file {
            async_fs::rename(path_tmp, path).await?;
        }
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
