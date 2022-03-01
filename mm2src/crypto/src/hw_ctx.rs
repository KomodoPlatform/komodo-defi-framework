use crate::crypto_ctx::{MM2_INTERNAL_DERIVATION_PATH, MM2_INTERNAL_ECDSA_CURVE};
use crate::hw_client::{HwClient, HwError, HwProcessingError, TrezorConnectProcessor};
use crate::trezor::TrezorSession;
use crate::HwWalletType;
use bitcrypto::dhash160;
use common::log::warn;
use common::mm_error::prelude::*;
use hw_common::primitives::{DerivationPath, Secp256k1ExtendedPublicKey};
use keys::Public as PublicKey;
use parking_lot::Mutex as PaMutex;
use primitives::hash::{H160, H264};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;
use trezor::client::TrezorClient;
use trezor::utxo::TrezorUtxoCoin;
use trezor::{ProcessTrezorResponse, TrezorRequestProcessor};

pub(crate) const MM2_TREZOR_INTERNAL_COIN: TrezorUtxoCoin = TrezorUtxoCoin::Komodo;

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
    /// The pubkey derived from `MM2_INTERNAL_DERIVATION_PATH`.
    pub(crate) hw_internal_pubkey: H264,
    pub(crate) hw_wallet_type: HwWalletType,
    /// Please avoid locking multiple mutexes.
    /// The mutex hasn't be locked while the wallet is used
    /// because every variant of the Hardware Wallet client uses an internal mutex to operate with the device.
    /// Clone the `Option<HwClient>` instance instead.
    pub(crate) hw_wallet: PaMutex<Option<HwClient>>,
}

impl HardwareWalletCtx {
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
        let hw_wallet = self.hw_wallet.lock().clone();
        if let Some(HwClient::Trezor(connected_trezor)) = hw_wallet {
            match self.check_trezor(&connected_trezor, processor).await {
                Ok(()) => return Ok(connected_trezor),
                // The device could be unplugged. We should try to reconnect to the device.
                Err(e) => warn!("Error checking a connected device: '{}'. Trying to reconnect...", e),
            }
        }
        // Connect to a device.
        let trezor = HwClient::trezor(processor).await?;
        // Check if the connected device has the same public key as we used to initialize the app.
        self.check_trezor(&trezor, processor).await?;

        // Reinitialize the field to avoid reconnecting next time.
        *self.hw_wallet.lock() = Some(HwClient::Trezor(trezor.clone()));

        Ok(trezor)
    }

    pub fn secp256k1_pubkey(&self) -> PublicKey { PublicKey::Compressed(self.hw_internal_pubkey) }

    pub fn rmd160(&self) -> H160 { dhash160(self.hw_internal_pubkey.as_slice()) }

    pub(crate) async fn trezor_mm_internal_pubkey<Processor>(
        trezor: &mut TrezorSession<'_>,
        processor: &Processor,
    ) -> MmResult<H264, HwProcessingError<Processor::Error>>
    where
        Processor: TrezorRequestProcessor + Sync,
    {
        let path = DerivationPath::from_str(MM2_INTERNAL_DERIVATION_PATH)
            .expect("'MM2_INTERNAL_DERIVATION_PATH' is expected to be valid derivation path");
        let mm2_internal_xpub = trezor
            .get_public_key(path, MM2_TREZOR_INTERNAL_COIN, MM2_INTERNAL_ECDSA_CURVE)
            .await
            .mm_err(HwError::from)?
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
            return MmError::err(HwProcessingError::HwError(HwError::FoundUnexpectedDevice {
                actual_pubkey,
                expected_pubkey: self.hw_internal_pubkey,
            }));
        }
        Ok(())
    }
}
