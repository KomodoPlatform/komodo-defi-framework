use crate::hw_client::{HwClient, HwError, HwProcessingError, TrezorConnectProcessor};
use crate::hw_ctx::HardwareWalletCtx;
use crate::key_pair_ctx::KeyPairCtx;
use crate::HwResult;
use bitcrypto::dhash160;
use common::mm_ctx::{MmArc, MmWeak};
use common::mm_error::prelude::*;
use common::privkey::{key_pair_from_seed, PrivKeyError};
use derive_more::Display;
use hw_common::primitives::EcdsaCurve;
use keys::Public as PublicKey;
use parking_lot::Mutex as PaMutex;
use primitives::hash::H264;
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

pub enum CryptoCtx {
    KeyPair(KeyPairCtx),
    HardwareWallet(HardwareWalletCtx),
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

    pub fn secp256k1_pubkey(&self) -> PublicKey {
        match self {
            CryptoCtx::KeyPair(key_pair_ctx) => key_pair_ctx.secp256k1_pubkey(),
            CryptoCtx::HardwareWallet(hw_ctx) => hw_ctx.secp256k1_pubkey(),
        }
    }

    pub fn secp256k1_pubkey_hex(&self) -> String { hex::encode(&*self.secp256k1_pubkey()) }

    pub fn init_with_passphrase(ctx: MmArc, passphrase: &str) -> CryptoInitResult<()> {
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
        let crypto_ctx = CryptoCtx::KeyPair(KeyPairCtx { secp256k1_key_pair });
        *ctx_field = Some(Arc::new(crypto_ctx));

        // TODO remove initializing legacy fields when lp_swap and lp_ordermatch support CryptoCtx.
        ctx.secp256k1_key_pair
            .pin(secp256k1_key_pair_for_legacy)
            .map_to_mm(CryptoInitError::Internal)?;
        ctx.rmd160.pin(rmd160).map_to_mm(CryptoInitError::Internal)?;

        Ok(())
    }

    pub async fn init_with_trezor<Processor>(
        ctx_weak: MmWeak,
        processor: &Processor,
    ) -> MmResult<(), HwProcessingError<Processor::Error>>
    where
        Processor: TrezorConnectProcessor + Sync,
    {
        let trezor = HwClient::trezor(processor).await?;
        let mm_internal_pubkey = {
            let mut session = trezor.session().await?;
            HardwareWalletCtx::trezor_mm_internal_pubkey(&mut session, processor).await?
        };

        Ok(CryptoCtx::init_with_hw_wallet_internal_xpub(
            ctx_weak,
            HwClient::from(trezor),
            mm_internal_pubkey,
        )?)
    }

    fn init_with_hw_wallet_internal_xpub(
        ctx_weak: MmWeak,
        hw_client: HwClient,
        mm2_internal_pubkey: H264,
    ) -> HwResult<()> {
        let ctx = match MmArc::from_weak(&ctx_weak) {
            Some(ctx) => ctx,
            None => return MmError::err(HwError::Internal("MmArc is dropped".to_owned())),
        };

        let mut ctx_field = ctx
            .crypto_ctx
            .lock()
            .map_to_mm(|poison| HwError::Internal(poison.to_string()))?;
        if ctx_field.is_some() {
            return MmError::err(HwError::Internal("'crypto_ctx' is initialized already".to_owned()));
        }

        // TODO remove initializing legacy fields when lp_swap and lp_ordermatch support CryptoCtx.
        let rmd160 = dhash160(mm2_internal_pubkey.as_slice());
        ctx.rmd160.pin(rmd160).map_to_mm(HwError::Internal)?;

        let crypto_ctx = CryptoCtx::HardwareWallet(HardwareWalletCtx {
            mm2_internal_pubkey,
            hw_wallet_type: hw_client.hw_wallet_type(),
            hw_wallet: PaMutex::new(Some(hw_client)),
        });
        *ctx_field = Some(Arc::new(crypto_ctx));
        Ok(())
    }
}
