//! [`hidapi::HidApi`] is not thread-safe https://github.com/libusb/hidapi/issues/133 and synchronous.
//! We have the following workarounds:
//! 1) Spawn a thread where we initialize `hidapi`, request, read, write to devices;
//! 2) Wrap `HidApi` into an mutex, also make reading non-blocking using [`hidapi::HidDevice::set_blocking_mode`].
//!    Then we can work with the API from different threads and not block, waiting for chunks.
//! The problem with `hidapi` is in that we can't determine when the device is disconnected.
//! https://github.com/libusb/hidapi/issues/103#issuecomment-537336680

use common::log::warn;
use common::mm_error::prelude::*;
use derive_more::Display;
use futures::lock::Mutex as AsyncMutex;
use hidapi::HidApi;
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const BLOCKING_MOD: bool = false;

pub type HidResult<T> = Result<T, MmError<HidError>>;
type HidContextShared = Arc<AsyncMutex<HidContext>>;

#[derive(Display)]
pub enum HidError {
    /// Please note it's not the same as disconnected!
    DeviceNotInitializedYet,
    #[display(fmt = "Device is open already: {:?}", _0)]
    DeviceIsOpenAlready(HidDeviceInfo),
    #[display(fmt = "HID API has been initialized already")]
    InitializedAlready,
    ErrorInitializing(hidapi::HidError),
    ErrorGettingDevices(hidapi::HidError),
    ErrorOpeningDevice(hidapi::HidError),
    ErrorWritingChunk(hidapi::HidError),
    #[display(
        fmt = "Writing to the HID device descriptor has been interrupted. Tried to send '{}' bytes, but '{}' are only sent",
        chunk_len,
        sent
    )]
    WritingInterrupted {
        chunk_len: usize,
        sent: usize,
    },
    ErrorReadingChunk(hidapi::HidError),
    #[display(fmt = "Received chunk is too long: '{}', expected '{}'", actual, expected)]
    ReceivedChunkTooLong {
        actual: usize,
        expected: usize,
    },
    #[display(fmt = "Not enough info to connect to a HID device: {:?}", _0)]
    NotEnoughInfoToConnect(HidDeviceInfo),
    Internal(String),
}

static HID_API_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub struct HidInstance {
    ctx: HidContextShared,
}

impl HidInstance {
    /// Initialize the `hidapi::HidApi` in a new thread.
    pub async fn init() -> HidResult<HidInstance> {
        HidInstance::check_on_init()?;
        let api = HidApi::new().map_to_mm(HidError::ErrorInitializing)?;
        let ctx = Arc::new(AsyncMutex::new(HidContext::new(api)));
        Ok(HidInstance { ctx })
    }

    fn check_on_init() -> HidResult<()> {
        if HID_API_INITIALIZED.load(Ordering::Relaxed) {
            return MmError::err(HidError::InitializedAlready);
        }
        Ok(())
    }

    pub async fn device_list(&self) -> HidResult<Vec<HidDevice>> {
        let mut ctx = self.ctx.lock().await;
        let devices = ctx
            .device_list()?
            .into_iter()
            .map(|device_info| HidDevice {
                ctx: self.ctx.clone(),
                device_info,
            })
            .collect();
        Ok(devices)
    }
}

struct HidContext {
    api: HidApi,
    connected_devices: HashMap<HidDeviceInfo, hidapi::HidDevice>,
}

impl Drop for HidContext {
    fn drop(&mut self) {
        const EXPECTED_CURRENT: bool = true;
        if HID_API_INITIALIZED
            .compare_exchange(EXPECTED_CURRENT, false, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            warn!("HID API has been released already");
        }
    }
}

impl HidContext {
    fn new(api: HidApi) -> HidContext {
        HidContext {
            api,
            connected_devices: HashMap::new(),
        }
    }

    fn device_list(&mut self) -> HidResult<Vec<HidDeviceInfo>> {
        self.api.refresh_devices().map_to_mm(HidError::ErrorGettingDevices)?;
        Ok(self.api.device_list().map(HidDeviceInfo::from).collect())
    }
}

pub struct HidDevice {
    ctx: HidContextShared,
    device_info: HidDeviceInfo,
}

impl HidDevice {
    pub async fn connect(&self) -> HidResult<()> {
        let mut ctx = self.ctx.lock().await;
        if ctx.connected_devices.contains_key(&self.device_info) {
            return MmError::err(HidError::DeviceIsOpenAlready(self.device_info.clone()));
        }

        // `CString` is expected to end with a zero byte and has a length of at least one.
        let device = if self.device_info.path.as_bytes().len() > 1 {
            ctx.api
                .open_path(self.device_info.path.as_c_str())
                .map_to_mm(HidError::ErrorOpeningDevice)?
        } else {
            let serial_number = self
                .device_info
                .serial_number
                .as_ref()
                .or_mm_err(|| HidError::NotEnoughInfoToConnect(self.device_info.clone()))?;
            ctx.api
                .open_serial(self.device_info.vendor_id, self.device_info.product_id, &serial_number)
                .map_to_mm(HidError::ErrorOpeningDevice)?
        };
        device
            .set_blocking_mode(BLOCKING_MOD)
            .map_to_mm(HidError::ErrorInitializing)?;
        ctx.connected_devices.insert(self.device_info.clone(), device);
        Ok(())
    }

    /// Returns true if the device is open yet.
    /// Please note USB enumeration can be expensive on some OS.
    pub async fn is_open(&self) -> bool {
        let mut ctx = self.ctx.lock().await;
        if ctx.connected_devices.get(&self.device_info).is_none() {
            return false;
        }
        let devices = match ctx.device_list() {
            Ok(devices) => devices,
            Err(e) => {
                warn!("{}", e);
                return false;
            },
        };
        devices.contains(&self.device_info)
    }

    pub async fn write_chunk(&self, chunk: Vec<u8>) -> HidResult<()> {
        let ctx = self.ctx.lock().await;
        let chunk_len = chunk.len();

        let device = ctx
            .connected_devices
            .get(&self.device_info)
            .or_mm_err(|| HidError::DeviceNotInitializedYet)?;
        let sent = device.write(&chunk).map_to_mm(HidError::ErrorWritingChunk)?;
        if sent < chunk_len {
            return MmError::err(HidError::WritingInterrupted { chunk_len, sent });
        }
        Ok(())
    }

    /// # Important
    ///
    /// May return a chunk with the length less than `chunk_len`.
    pub async fn read_chunk(&self, chunk_len: usize) -> HidResult<Vec<u8>> {
        let ctx = self.ctx.lock().await;
        let device = ctx
            .connected_devices
            .get(&self.device_info)
            .or_mm_err(|| HidError::DeviceNotInitializedYet)?;
        let mut buf = vec![0; chunk_len];
        // Don't use [`HidDevice::read_timeout`] because we set [`HidDevice::set_blocking_mode`] to false.
        let received = device.read(&mut buf).map_to_mm(HidError::ErrorReadingChunk)?;
        if received > chunk_len {
            return MmError::err(HidError::ReceivedChunkTooLong {
                actual: received,
                expected: chunk_len,
            });
        }
        Ok(buf[0..received].to_vec())
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct HidDeviceInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub interface_number: i32,
    pub path: CString,
    pub serial_number: Option<String>,
    pub manufacturer_string: Option<String>,
    pub product_string: Option<String>,
}

impl From<&hidapi::DeviceInfo> for HidDeviceInfo {
    fn from(info: &hidapi::DeviceInfo) -> Self {
        HidDeviceInfo {
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            interface_number: info.interface_number(),
            path: info.path().to_owned(),
            serial_number: info.serial_number().map(str::to_owned),
            manufacturer_string: info.manufacturer_string().map(str::to_owned),
            product_string: info.product_string().map(str::to_owned),
        }
    }
}
