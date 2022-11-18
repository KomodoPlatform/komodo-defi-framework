use crate::hw_client::{HwClient, HwDeviceInfo, HwProcessingError, HwPubkey, TrezorConnectProcessor};
use crate::hw_error::HwError;
use crate::trezor::TrezorSession;
use crate::{mm2_internal_der_path, HwWalletType};
use bip32::ChildNumber;
use bitcrypto::dhash160;
use common::log::warn;
use futures::lock::Mutex as AsyncMutex;
use hw_common::primitives::{EcdsaCurve, Secp256k1ExtendedPublicKey};
use keys::Public as PublicKey;
use mm2_err_handle::prelude::*;
use primitives::hash::{H160, H264};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;
use trezor::client::TrezorClient;
use trezor::utxo::IGNORE_XPUB_MAGIC;
use trezor::{ProcessTrezorResponse, TrezorRequestProcessor};

const MM2_INTERNAL_ECDSA_CURVE: EcdsaCurve = EcdsaCurve::Secp256k1;
const MM2_TREZOR_INTERNAL_COIN: &str = "Komodo";
const SHOW_PUBKEY_ON_DISPLAY: bool = false;

#[derive(Clone)]
pub struct HardwareWalletArc(Arc<HardwareWalletCtx>);

impl Deref for HardwareWalletArc {
    type Target = HardwareWalletCtx;

    fn deref(&self) -> &Self::Target { &self.0 }
}

impl HardwareWalletArc {
    pub fn new(ctx: HardwareWalletCtx) -> HardwareWalletArc { HardwareWalletArc(Arc::new(ctx)) }
}

pub struct HardwareWalletCtx {
    /// The pubkey derived from `mm2_internal_der_path`.
    pub(crate) hw_internal_pubkey: H264,
    pub(crate) hw_wallet_type: HwWalletType,
    /// Please avoid locking multiple mutexes.
    /// The mutex hasn't to be locked while the client is used
    /// because every variant of `HwClient` uses an internal mutex to operate with the device.
    /// But it has to be locked while the client is initialized.
    pub(crate) hw_wallet: AsyncMutex<Option<HwClient>>,
}

impl HardwareWalletCtx {
    pub(crate) async fn init_with_trezor<Processor>(
        processor: &Processor,
    ) -> MmResult<(HwDeviceInfo, HardwareWalletArc), HwProcessingError<Processor::Error>>
    where
        Processor: TrezorConnectProcessor + Sync,
    {
        let trezor = HwClient::trezor(processor).await?;

        let (hw_device_info, hw_internal_pubkey) = {
            let (device_info, mut session) = trezor.new_session().await?;
            let hw_internal_pubkey = HardwareWalletCtx::trezor_mm_internal_pubkey(&mut session, processor).await?;
            (HwDeviceInfo::Trezor(device_info), hw_internal_pubkey)
        };

        let hw_client = HwClient::Trezor(trezor);
        let hw_ctx = HardwareWalletArc::new(HardwareWalletCtx {
            hw_internal_pubkey,
            hw_wallet_type: hw_client.hw_wallet_type(),
            hw_wallet: AsyncMutex::new(Some(hw_client)),
        });
        Ok((hw_device_info, hw_ctx))
    }

    pub fn hw_wallet_type(&self) -> HwWalletType { self.hw_wallet_type }

    /// Connects to a Trezor device and checks if MM was initialized from this particular device.
    pub async fn trezor<Processor>(
        &self,
        processor: &Processor,
    ) -> MmResult<TrezorClient, HwProcessingError<Processor::Error>>
    where
        Processor: TrezorConnectProcessor + Sync,
        Processor::Error: std::fmt::Display,
    {
        let mut hw_client = self.hw_wallet.lock().await;
        if let Some(HwClient::Trezor(connected_trezor)) = hw_client.deref() {
            match self.check_trezor(connected_trezor, processor).await {
                Ok(()) => return Ok(connected_trezor.clone()),
                // The device could be unplugged. We should try to reconnect to the device.
                Err(e) => warn!("Error checking hardware wallet device: '{}'. Trying to reconnect...", e),
            }
        }
        // Connect to a device.
        let trezor = HwClient::trezor(processor).await?;
        // Check if the connected device has the same public key as we used to initialize the app.
        self.check_trezor(&trezor, processor).await?;

        // Reinitialize the field to avoid reconnecting next time.
        *hw_client = Some(HwClient::Trezor(trezor.clone()));

        Ok(trezor)
    }

    pub fn secp256k1_pubkey(&self) -> PublicKey { PublicKey::Compressed(self.hw_internal_pubkey) }

    /// Returns `RIPEMD160(SHA256(x))` where x is a pubkey extracted from the Hardware wallet.
    pub fn rmd160(&self) -> H160 { h160_from_h264(&self.hw_internal_pubkey) }

    /// Returns serializable/deserializable Hardware wallet pubkey.
    pub fn hw_pubkey(&self) -> HwPubkey { hw_pubkey_from_h264(&self.hw_internal_pubkey) }

    pub(crate) async fn trezor_mm_internal_pubkey<Processor>(
        trezor: &mut TrezorSession<'_>,
        processor: &Processor,
    ) -> MmResult<H264, HwProcessingError<Processor::Error>>
    where
        Processor: TrezorRequestProcessor + Sync,
    {
        const ADDRESS_INDEX: Option<ChildNumber> = None;

        let path = mm2_internal_der_path(ADDRESS_INDEX);
        let mm2_internal_xpub = trezor
            .get_public_key(
                path,
                MM2_TREZOR_INTERNAL_COIN.to_string(),
                MM2_INTERNAL_ECDSA_CURVE,
                SHOW_PUBKEY_ON_DISPLAY,
                IGNORE_XPUB_MAGIC,
            )
            .await?
            .process(processor)
            .await?;
        let extended_pubkey = Secp256k1ExtendedPublicKey::from_str(&mm2_internal_xpub).map_to_mm(HwError::from)?;
        Ok(H264::from(extended_pubkey.public_key().serialize()))
    }

    async fn check_trezor<Processor>(
        &self,
        trezor: &TrezorClient,
        processor: &Processor,
    ) -> MmResult<(), HwProcessingError<Processor::Error>>
    where
        Processor: TrezorRequestProcessor + Sync,
    {
        let mut session = trezor.session().await.mm_err(HwError::from)?;
        let actual_pubkey = Self::trezor_mm_internal_pubkey(&mut session, processor).await?;
        if actual_pubkey != self.hw_internal_pubkey {
            let actual_pubkey = hw_pubkey_from_h264(&actual_pubkey);
            let expected_pubkey = self.hw_pubkey();
            return MmError::err(HwProcessingError::HwError(HwError::FoundUnexpectedDevice {
                actual_pubkey,
                expected_pubkey,
            }));
        }
        Ok(())
    }
}

/// Applies `RIPEMD160(SHA256(h264))` to the given `h264`.
fn h160_from_h264(h264: &H264) -> H160 { dhash160(h264.as_slice()) }

/// Converts `H264` into a serializable/deserializable Hardware wallet pubkey.
fn hw_pubkey_from_h264(h264: &H264) -> HwPubkey { HwPubkey::from(h160_from_h264(h264).as_slice()) }
