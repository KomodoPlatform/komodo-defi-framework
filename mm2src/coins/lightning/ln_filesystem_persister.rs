use crate::lightning::ln_platform::Platform;
use crate::lightning::ln_storage::{LightningStorage, NodesAddressesMap, NodesAddressesMapShared, Scorer};
use crate::lightning::ln_utils::{ChainMonitor, ChannelManager};
use async_trait::async_trait;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::Network;
use bitcoin_hashes::hex::ToHex;
use common::async_blocking;
use common::log::LogState;
use lightning::chain::channelmonitor::{ChannelMonitor, ChannelMonitorUpdate};
use lightning::chain::keysinterface::{InMemorySigner, KeysManager, Sign};
use lightning::chain::transaction::OutPoint;
use lightning::chain::{chainmonitor, ChannelMonitorUpdateErr};
use lightning::routing::network_graph::NetworkGraph;
use lightning::routing::scoring::ProbabilisticScoringParameters;
use lightning::util::ser::{Readable, ReadableArgs, Writeable};
use lightning_background_processor::Persister;
use lightning_persister::FilesystemPersister;
use mm2_io::fs::check_dir_operations;
use secp256k1::PublicKey;
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

#[cfg(target_family = "unix")] use std::os::unix::io::AsRawFd;

#[cfg(target_family = "windows")]
use {std::ffi::OsStr, std::os::windows::ffi::OsStrExt};

pub struct LightningFilesystemPersister {
    main_path: PathBuf,
    backup_path: Option<PathBuf>,
    channels_persister: FilesystemPersister,
}

impl LightningFilesystemPersister {
    /// Initialize a new LightningPersister and set the path to the individual channels'
    /// files.
    #[inline]
    pub fn new(main_path: PathBuf, backup_path: Option<PathBuf>) -> Self {
        Self {
            main_path: main_path.clone(),
            backup_path,
            channels_persister: FilesystemPersister::new(main_path.display().to_string()),
        }
    }

    /// Get the directory which was provided when this persister was initialized.
    #[inline]
    pub fn main_path(&self) -> PathBuf { self.main_path.clone() }

    /// Get the backup directory which was provided when this persister was initialized.
    #[inline]
    pub fn backup_path(&self) -> Option<PathBuf> { self.backup_path.clone() }

    /// Get the channels_persister which was initialized when this persister was initialized.
    #[inline]
    pub fn channels_persister(&self) -> &FilesystemPersister { &self.channels_persister }

    pub fn monitor_backup_path(&self) -> Option<PathBuf> {
        if let Some(mut backup_path) = self.backup_path() {
            backup_path.push("monitors");
            return Some(backup_path);
        }
        None
    }

    pub fn nodes_addresses_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("channel_nodes_data");
        path
    }

    pub fn nodes_addresses_backup_path(&self) -> Option<PathBuf> {
        if let Some(mut backup_path) = self.backup_path() {
            backup_path.push("channel_nodes_data");
            return Some(backup_path);
        }
        None
    }

    pub fn network_graph_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("network_graph");
        path
    }

    pub fn scorer_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("scorer");
        path
    }

    pub fn manager_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("manager");
        path
    }
}

#[derive(Clone)]
pub struct LightningPersisterShared(pub Arc<LightningFilesystemPersister>);

impl Deref for LightningPersisterShared {
    type Target = LightningFilesystemPersister;
    fn deref(&self) -> &LightningFilesystemPersister { self.0.deref() }
}

impl Persister<InMemorySigner, Arc<ChainMonitor>, Arc<Platform>, Arc<KeysManager>, Arc<Platform>, Arc<LogState>>
    for LightningPersisterShared
{
    fn persist_manager(&self, channel_manager: &ChannelManager) -> Result<(), std::io::Error> {
        FilesystemPersister::persist_manager(self.0.main_path().display().to_string(), channel_manager)?;
        if let Some(backup_path) = self.0.backup_path() {
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(backup_path)?;
            channel_manager.write(&mut BufWriter::new(file))?;
        }
        Ok(())
    }

    fn persist_graph(&self, network_graph: &NetworkGraph) -> Result<(), std::io::Error> {
        if FilesystemPersister::persist_network_graph(self.0.main_path().display().to_string(), network_graph).is_err()
        {
            // Persistence errors here are non-fatal as we can just fetch the routing graph
            // again later, but they may indicate a disk error which could be fatal elsewhere.
            eprintln!("Warning: Failed to persist network graph, check your disk and permissions");
        }

        Ok(())
    }
}

#[cfg(target_family = "windows")]
macro_rules! call {
    ($e: expr) => {
        if $e != 0 {
            return Ok(());
        } else {
            return Err(std::io::Error::last_os_error());
        }
    };
}

#[cfg(target_family = "windows")]
fn path_to_windows_str<T: AsRef<OsStr>>(path: T) -> Vec<winapi::shared::ntdef::WCHAR> {
    path.as_ref().encode_wide().chain(Some(0)).collect()
}

fn write_monitor_to_file<ChannelSigner: Sign>(
    mut path: PathBuf,
    filename: String,
    monitor: &ChannelMonitor<ChannelSigner>,
) -> std::io::Result<()> {
    // Do a crazy dance with lots of fsync()s to be overly cautious here...
    // We never want to end up in a state where we've lost the old data, or end up using the
    // old data on power loss after we've returned.
    // The way to atomically write a file on Unix platforms is:
    // open(tmpname), write(tmpfile), fsync(tmpfile), close(tmpfile), rename(), fsync(dir)
    path.push(filename);
    let filename_with_path = path.display().to_string();
    let tmp_filename = format!("{}.tmp", filename_with_path);

    {
        let mut f = fs::File::create(&tmp_filename)?;
        monitor.write(&mut f)?;
        f.sync_all()?;
    }
    // Fsync the parent directory on Unix.
    #[cfg(target_family = "unix")]
    {
        fs::rename(&tmp_filename, &filename_with_path)?;
        let path = Path::new(&filename_with_path).parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("can't find parent dir for {}", filename_with_path),
            )
        })?;
        let dir_file = fs::OpenOptions::new().read(true).open(path)?;
        unsafe {
            libc::fsync(dir_file.as_raw_fd());
        }
    }
    #[cfg(target_family = "windows")]
    {
        let src = PathBuf::from(tmp_filename);
        let dst = PathBuf::from(filename_with_path.clone());
        if Path::new(&filename_with_path).exists() {
            unsafe {
                winapi::um::winbase::ReplaceFileW(
                    path_to_windows_str(dst).as_ptr(),
                    path_to_windows_str(src).as_ptr(),
                    std::ptr::null(),
                    winapi::um::winbase::REPLACEFILE_IGNORE_MERGE_ERRORS,
                    std::ptr::null_mut() as *mut winapi::ctypes::c_void,
                    std::ptr::null_mut() as *mut winapi::ctypes::c_void,
                )
            };
        } else {
            call!(unsafe {
                winapi::um::winbase::MoveFileExW(
                    path_to_windows_str(src).as_ptr(),
                    path_to_windows_str(dst).as_ptr(),
                    winapi::um::winbase::MOVEFILE_WRITE_THROUGH | winapi::um::winbase::MOVEFILE_REPLACE_EXISTING,
                )
            });
        }
    }
    Ok(())
}

impl<ChannelSigner: Sign> chainmonitor::Persist<ChannelSigner> for LightningFilesystemPersister {
    fn persist_new_channel(
        &self,
        funding_txo: OutPoint,
        monitor: &ChannelMonitor<ChannelSigner>,
        update_id: chainmonitor::MonitorUpdateId,
    ) -> Result<(), ChannelMonitorUpdateErr> {
        self.channels_persister
            .persist_new_channel(funding_txo, monitor, update_id)?;
        if let Some(backup_path) = self.monitor_backup_path() {
            let filename = format!("{}_{}", funding_txo.txid.to_hex(), funding_txo.index);
            write_monitor_to_file(backup_path, filename, monitor)
                .map_err(|_| ChannelMonitorUpdateErr::PermanentFailure)?;
        }
        Ok(())
    }

    fn update_persisted_channel(
        &self,
        funding_txo: OutPoint,
        update: &Option<ChannelMonitorUpdate>,
        monitor: &ChannelMonitor<ChannelSigner>,
        update_id: chainmonitor::MonitorUpdateId,
    ) -> Result<(), ChannelMonitorUpdateErr> {
        self.channels_persister
            .update_persisted_channel(funding_txo, update, monitor, update_id)?;
        if let Some(backup_path) = self.monitor_backup_path() {
            let filename = format!("{}_{}", funding_txo.txid.to_hex(), funding_txo.index);
            write_monitor_to_file(backup_path, filename, monitor)
                .map_err(|_| ChannelMonitorUpdateErr::PermanentFailure)?;
        }
        Ok(())
    }
}

#[async_trait]
impl LightningStorage for LightningFilesystemPersister {
    type Error = std::io::Error;

    async fn init_fs(&self) -> Result<(), Self::Error> {
        let path = self.main_path();
        let backup_path = self.backup_path();
        async_blocking(move || {
            fs::create_dir_all(path.clone())?;
            if let Some(path) = backup_path {
                fs::create_dir_all(path.clone())?;
                check_dir_operations(&path)?;
            }
            check_dir_operations(&path)
        })
        .await
    }

    async fn is_fs_initialized(&self) -> Result<bool, Self::Error> {
        let dir_path = self.main_path();
        let backup_dir_path = self.backup_path();
        async_blocking(move || {
            if !dir_path.exists() || backup_dir_path.as_ref().map(|path| !path.exists()).unwrap_or(false) {
                Ok(false)
            } else if !dir_path.is_dir() {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{} is not a directory", dir_path.display()),
                ))
            } else if backup_dir_path.as_ref().map(|path| !path.is_dir()).unwrap_or(false) {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Backup path is not a directory",
                ))
            } else {
                let check_backup_ops = if let Some(backup_path) = backup_dir_path {
                    check_dir_operations(&backup_path).is_ok()
                } else {
                    true
                };
                check_dir_operations(&dir_path).map(|_| check_backup_ops)
            }
        })
        .await
    }

    async fn get_nodes_addresses(&self) -> Result<NodesAddressesMap, Self::Error> {
        let path = self.nodes_addresses_path();
        if !path.exists() {
            return Ok(HashMap::new());
        }
        async_blocking(move || {
            let file = fs::File::open(path)?;
            let reader = BufReader::new(file);
            let nodes_addresses: HashMap<String, SocketAddr> =
                serde_json::from_reader(reader).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            nodes_addresses
                .iter()
                .map(|(pubkey_str, addr)| {
                    let pubkey = PublicKey::from_str(pubkey_str)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                    Ok((pubkey, *addr))
                })
                .collect()
        })
        .await
    }

    async fn save_nodes_addresses(&self, nodes_addresses: NodesAddressesMapShared) -> Result<(), Self::Error> {
        let path = self.nodes_addresses_path();
        let backup_path = self.nodes_addresses_backup_path();
        async_blocking(move || {
            let nodes_addresses: HashMap<String, SocketAddr> = nodes_addresses
                .lock()
                .iter()
                .map(|(pubkey, addr)| (pubkey.to_string(), *addr))
                .collect();

            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)?;
            serde_json::to_writer(file, &nodes_addresses)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

            if let Some(path) = backup_path {
                let file = fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(path)?;
                serde_json::to_writer(file, &nodes_addresses)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            }

            Ok(())
        })
        .await
    }

    async fn get_network_graph(&self, network: Network) -> Result<NetworkGraph, Self::Error> {
        let path = self.network_graph_path();
        if !path.exists() {
            return Ok(NetworkGraph::new(genesis_block(network).header.block_hash()));
        }
        async_blocking(move || {
            let file = fs::File::open(path)?;
            common::log::info!("Reading the saved lightning network graph from file, this can take some time!");
            NetworkGraph::read(&mut BufReader::new(file))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
        })
        .await
    }

    async fn get_scorer(&self, network_graph: Arc<NetworkGraph>) -> Result<Scorer, Self::Error> {
        let path = self.scorer_path();
        if !path.exists() {
            return Ok(Scorer::new(ProbabilisticScoringParameters::default(), network_graph));
        }
        async_blocking(move || {
            let file = fs::File::open(path)?;
            Scorer::read(
                &mut BufReader::new(file),
                (ProbabilisticScoringParameters::default(), network_graph),
            )
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
        })
        .await
    }

    async fn save_scorer(&self, scorer: Arc<Mutex<Scorer>>) -> Result<(), Self::Error> {
        let path = self.scorer_path();
        async_blocking(move || {
            let scorer = scorer.lock().unwrap();
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)?;
            scorer.write(&mut BufWriter::new(file))
        })
        .await
    }
}
