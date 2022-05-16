use crate::hw_client::{HwError, HwProcessingError, TrezorConnectProcessor};
use crate::hw_ctx::{HardwareWalletArc, HardwareWalletCtx};
use crate::key_pair_ctx::IguanaArc;
use crate::privkey::{key_pair_from_seed, PrivKeyError};
use derive_more::Display;
use hw_common::primitives::EcdsaCurve;
use keys::Public as PublicKey;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::{mm_error::NotEqual, prelude::*};
use parking_lot::RwLock;
use primitives::hash::H160;
use std::ops::Deref;
use std::sync::Arc;

pub type CryptoInitResult<T> = Result<T, MmError<CryptoInitError>>;

/// The derivation path generally consists of:
/// `m/purpose'/coin_type'/account'/change/address_index`.
/// For MarketMaker internal purposes, we decided to use a pubkey derived from the following path, where:
/// * `coin_type = 141` - KMD coin;
/// * `account = (2 ^ 31 - 1) = 2147483647` - latest available account index.
///   This number is chosen so that it does not cross with real accounts;
/// * `change = 0`, `address_index = 0` - nothing special.
pub(crate) const MM2_INTERNAL_DERIVATION_PATH: &str = "m/44'/141'/2147483647/0/0";
pub(crate) const MM2_INTERNAL_ECDSA_CURVE: EcdsaCurve = EcdsaCurve::Secp256k1;

#[derive(Debug, Display)]
pub enum CryptoInitError {
    NotInitialized,
    InitializedAlready,
    #[display(fmt = "jeezy says we cant use the nullstring as passphrase and I agree")]
    NullStringPassphrase,
    #[display(fmt = "Invalid passphrase: '{}'", _0)]
    InvalidPassphrase(PrivKeyError),
    Internal(String),
}

impl From<PrivKeyError> for CryptoInitError {
    fn from(e: PrivKeyError) -> Self { CryptoInitError::InvalidPassphrase(e) }
}

#[derive(Debug)]
pub enum HwCtxInitError<ProcessorError> {
    InitializingAlready,
    InitializedAlready,
    HwError(HwError),
    ProcessorError(ProcessorError),
}

impl<ProcessorError> From<HwProcessingError<ProcessorError>> for HwCtxInitError<ProcessorError> {
    fn from(e: HwProcessingError<ProcessorError>) -> Self {
        match e {
            HwProcessingError::HwError(hw_error) => HwCtxInitError::HwError(hw_error),
            HwProcessingError::ProcessorError(processor_error) => HwCtxInitError::ProcessorError(processor_error),
        }
    }
}

/// This is required for converting `MmError<HwProcessingError<E>>` into `MmError<InitHwCtxError<E>>`.
impl<E> NotEqual for HwCtxInitError<E> {}

pub struct CryptoCtx {
    iguana_ctx: IguanaArc,
    /// Can be initialized on [`CryptoCtx::init_hw_ctx_with_trezor`].
    hw_ctx: RwLock<HardwareWalletCtxState>,
}

impl CryptoCtx {
    pub fn from_ctx(ctx: &MmArc) -> CryptoInitResult<Arc<CryptoCtx>> {
        let ctx_field = ctx
            .crypto_ctx
            .lock()
            .map_to_mm(|poison| CryptoInitError::Internal(poison.to_string()))?;
        let ctx = match ctx_field.deref() {
            Some(ctx) => ctx,
            None => return MmError::err(CryptoInitError::NotInitialized),
        };
        ctx.clone()
            .downcast()
            .map_err(|_| MmError::new(CryptoInitError::Internal("Error casting the context field".to_owned())))
    }

    pub fn iguana_ctx(&self) -> &IguanaArc { &self.iguana_ctx }

    pub fn secp256k1_pubkey(&self) -> PublicKey { self.iguana_ctx.secp256k1_pubkey() }

    pub fn secp256k1_pubkey_hex(&self) -> String { hex::encode(&*self.secp256k1_pubkey()) }

    pub fn hw_ctx(&self) -> Option<HardwareWalletArc> { self.hw_ctx.read().to_option().cloned() }

    /// Returns an `RIPEMD160(SHA256(x))` where x is secp256k1 pubkey that identifies a Hardware Wallet device or an HD master private key.
    pub fn hd_wallet_rmd160(&self) -> Option<H160> { self.hw_ctx.read().to_option().map(|hw_ctx| hw_ctx.rmd160()) }

    pub fn init_with_iguana_passphrase(ctx: MmArc, passphrase: &str) -> CryptoInitResult<()> {
        let mut ctx_field = ctx
            .crypto_ctx
            .lock()
            .map_to_mm(|poison| CryptoInitError::Internal(poison.to_string()))?;
        if ctx_field.is_some() {
            return MmError::err(CryptoInitError::InitializedAlready);
        }

        if passphrase.is_empty() {
            return MmError::err(CryptoInitError::NullStringPassphrase);
        }

        let secp256k1_key_pair = key_pair_from_seed(passphrase)?;
        // We can't clone `secp256k1_key_pair`, but it's used later to initialize legacy `MmCtx` fields.
        let secp256k1_key_pair_for_legacy = key_pair_from_seed(passphrase)?;

        let rmd160 = secp256k1_key_pair.public().address_hash();
        let crypto_ctx = CryptoCtx {
            iguana_ctx: IguanaArc::from(secp256k1_key_pair),
            hw_ctx: RwLock::new(HardwareWalletCtxState::NotInitialized),
        };
        *ctx_field = Some(Arc::new(crypto_ctx));

        // TODO remove initializing legacy fields when lp_swap and lp_ordermatch support CryptoCtx.
        ctx.secp256k1_key_pair
            .pin(secp256k1_key_pair_for_legacy)
            .map_to_mm(CryptoInitError::Internal)?;
        ctx.rmd160.pin(rmd160).map_to_mm(CryptoInitError::Internal)?;

        Ok(())
    }

    pub async fn init_hw_ctx_with_trezor<Processor>(
        &self,
        processor: &Processor,
    ) -> MmResult<HardwareWalletArc, HwCtxInitError<Processor::Error>>
    where
        Processor: TrezorConnectProcessor + Sync,
    {
        {
            let mut state = self.hw_ctx.write();
            match state.deref() {
                HardwareWalletCtxState::NotInitialized => (),
                HardwareWalletCtxState::Initializing => return MmError::err(HwCtxInitError::InitializingAlready),
                HardwareWalletCtxState::Ready(_) => return MmError::err(HwCtxInitError::InitializedAlready),
            }

            *state = HardwareWalletCtxState::Initializing;
        }

        let (res, new_state) = match HardwareWalletCtx::init_with_trezor(processor).await {
            Ok(hw_ctx) => (Ok(hw_ctx.clone()), HardwareWalletCtxState::Ready(hw_ctx)),
            Err(e) => (Err(e), HardwareWalletCtxState::NotInitialized),
        };

        *self.hw_ctx.write() = new_state;
        res.mm_err(HwCtxInitError::from)
    }
}

enum HardwareWalletCtxState {
    NotInitialized,
    Initializing,
    Ready(HardwareWalletArc),
}

impl HardwareWalletCtxState {
    fn to_option(&self) -> Option<&HardwareWalletArc> {
        match self {
            HardwareWalletCtxState::Ready(hw_ctx) => Some(hw_ctx),
            _ => None,
        }
    }
}
