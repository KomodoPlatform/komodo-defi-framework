use common::HttpStatusCode;
use crypto::CryptoCtxError;
use enum_derives::EnumFromStringify;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;

use crate::lp_native_dex::{lp_init_continue_impl, MmInitError};
use crate::lp_wallet::{initialize_crypto_context, process_passphrase_logic, Passphrase, WalletInitError};

#[derive(Serialize, Display, SerializeErrorType, EnumFromStringify)]
#[serde(tag = "error_type", content = "error_data")]
pub enum CryptoCtxRpcError {
    #[display(fmt = "Unable to initialized crypto ctx")]
    InitializationFailed,
    #[display(fmt = "crypto ctx is already initialized")]
    AlreadyInitialized,
    InternalError(String),
    #[from_stringify("MmInitError")]
    MmInitError(String),
    #[from_stringify("WalletInitError")]
    WalletInitError(String),
}

impl From<CryptoCtxError> for CryptoCtxRpcError {
    fn from(e: CryptoCtxError) -> Self {
        match e {
            CryptoCtxError::NotInitialized => Self::InitializationFailed,
            CryptoCtxError::Internal(_) => Self::InternalError(e.to_string()),
        }
    }
}

impl HttpStatusCode for CryptoCtxRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::InitializationFailed | Self::AlreadyInitialized | Self::WalletInitError(_) | Self::MmInitError(_) => {
                StatusCode::BAD_REQUEST
            },
            Self::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Deserialize)]
pub struct CryptoCtxInitRequest {
    passphrase: String,
    wallet_name: Option<String>,
}

pub async fn init_crypto_ctx(ctx: MmArc, req: CryptoCtxInitRequest) -> MmResult<(), CryptoCtxRpcError> {
    common::log::info!("Initializing KDF with passphrase");
    if let Some(true) = ctx.initialized.get() {
        return MmError::err(CryptoCtxRpcError::AlreadyInitialized);
    };

    let CryptoCtxInitRequest {
        wallet_name,
        passphrase,
    } = req;
    let passphrase: Passphrase = Passphrase::Decrypted(passphrase);

    ctx.wallet_name
        .set(wallet_name.clone())
        .map_to_mm(|_| WalletInitError::InternalError("Already Initialized".to_string()))?;

    let passphrase = process_passphrase_logic(&ctx, wallet_name.as_deref(), Some(passphrase)).await?;

    if let Some(passphrase) = passphrase {
        initialize_crypto_context(&ctx, &passphrase)?;
    }

    lp_init_continue_impl(ctx).await?;

    Ok(())
}
