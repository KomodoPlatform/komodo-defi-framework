use crypto::{EncryptedData, MnemonicError};
use enum_derives::EnumFromStringify;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_io::fs::{ensure_file_is_writable, list_files_by_extension};

use super::ReadPassphraseError;

type WalletsStorageResult<T> = Result<T, MmError<WalletsStorageError>>;

#[derive(Debug, Deserialize, Display, Serialize, EnumFromStringify)]
pub enum WalletsStorageError {
    #[display(fmt = "Error writing to file: {}", _0)]
    FsWriteError(String),
    #[display(fmt = "Error reading from file: {}", _0)]
    FsReadError(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
    #[display(fmt = "Mnemonic error: {}", _0)]
    #[from_stringify("MnemonicError")]
    MnemonicError(String),
    #[display(fmt = "Read Passphrase error: {}", _0)]
    #[from_stringify("ReadPassphraseError")]
    ReadPassphraseError(String),
}

/// Saves the passphrase to a file associated with the given wallet name.
///
/// # Returns
/// Result indicating success or an error.
pub(super) async fn save_encrypted_passphrase(
    ctx: &MmArc,
    wallet_name: &str,
    encrypted_passphrase_data: &EncryptedData,
) -> WalletsStorageResult<()> {
    let wallet_path = ctx.wallet_file_path(wallet_name);
    ensure_file_is_writable(&wallet_path).map_to_mm(WalletsStorageError::FsWriteError)?;
    mm2_io::fs::write_json(encrypted_passphrase_data, &wallet_path, true)
        .await
        .mm_err(|e| WalletsStorageError::FsWriteError(e.to_string()))
}

/// Reads the encrypted passphrase data from the file associated with the given wallet name, if available.
///
/// This function is responsible for retrieving the encrypted passphrase data from a file, if it exists.
/// The data is expected to be in the format of `EncryptedData`, which includes
/// all necessary components for decryption, such as the encryption algorithm, key derivation
///
/// # Returns
/// `io::Result<EncryptedPassphraseData>` - The encrypted passphrase data or an error if the
/// reading process fails.
///
/// # Errors
/// Returns an `io::Error` if the file cannot be read or the data cannot be deserialized into
/// `EncryptedData`.
pub(super) async fn read_encrypted_passphrase_if_available(ctx: &MmArc) -> WalletsStorageResult<Option<EncryptedData>> {
    let wallet_name = ctx
        .wallet_name
        .get()
        .ok_or(WalletsStorageError::Internal(
            "`wallet_name` not initialized yet!".to_string(),
        ))?
        .clone()
        .ok_or_else(|| WalletsStorageError::Internal("`wallet_name` cannot be None!".to_string()))?;
    let wallet_path = ctx.wallet_file_path(&wallet_name);
    mm2_io::fs::read_json(&wallet_path).await.mm_err(|e| {
        WalletsStorageError::FsReadError(format!(
            "Error reading passphrase from file {}: {}",
            wallet_path.display(),
            e
        ))
    })
}

pub(super) async fn read_all_wallet_names(ctx: &MmArc) -> WalletsStorageResult<impl Iterator<Item = String>> {
    let wallet_names = list_files_by_extension(&ctx.db_root(), "dat", false)
        .await
        .mm_err(|e| WalletsStorageError::FsReadError(format!("Error reading wallets directory: {}", e)))?;
    Ok(wallet_names)
}
