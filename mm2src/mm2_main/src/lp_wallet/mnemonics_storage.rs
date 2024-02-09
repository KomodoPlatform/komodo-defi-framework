use crypto::EncryptedData;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_io::fs::ensure_file_is_writable;

type WalletsStorageResult<T> = Result<T, MmError<WalletsStorageError>>;

#[derive(Debug, Deserialize, Display, Serialize)]
pub enum WalletsStorageError {
    #[display(fmt = "{} db file is not writable", path)]
    DbFileIsNotWritable { path: String },
    #[display(fmt = "Error writing to file: {}", _0)]
    FsWriteError(String),
    #[display(fmt = "Error reading from file: {}", _0)]
    FsReadError(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

/// Saves the passphrase to a file associated with the given wallet name.
///
/// # Arguments
///
/// * `wallet_name` - The name of the wallet.
/// * `passphrase` - The passphrase to save.
///
/// # Returns
/// Result indicating success or an error.
pub(super) async fn save_encrypted_passphrase(
    ctx: &MmArc,
    wallet_name: &str,
    encrypted_passphrase_data: &EncryptedData,
) -> WalletsStorageResult<()> {
    let wallet_path = ctx.wallet_file_path(wallet_name);
    ensure_file_is_writable(&wallet_path).map_to_mm(|_| WalletsStorageError::DbFileIsNotWritable {
        path: wallet_path.display().to_string(),
    })?;
    mm2_io::fs::write_json(encrypted_passphrase_data, &wallet_path, true)
        .await
        .mm_err(|e| WalletsStorageError::FsWriteError(e.to_string()))
}

/// Reads the encrypted passphrase data from the file associated with the given wallet name.
///
/// This function is responsible for retrieving the encrypted passphrase data from a file.
/// The data is expected to be in the format of `EncryptedData`, which includes
/// all necessary components for decryption, such as the encryption algorithm, key derivation
/// details, salts, IV, ciphertext, and HMAC tag.
///
/// # Arguments
///
/// * `ctx` - The `MmArc` context, providing access to application configuration and state.
/// * `wallet_name` - The name of the wallet whose encrypted passphrase data is to be read.
///
/// # Returns
/// `io::Result<EncryptedPassphraseData>` - The encrypted passphrase data or an error if the
/// reading process fails.
///
/// # Errors
/// Returns an `io::Error` if the file cannot be read or the data cannot be deserialized into
/// `EncryptedData`.
pub(super) async fn read_encrypted_passphrase(ctx: &MmArc) -> WalletsStorageResult<Option<EncryptedData>> {
    let wallet_name = ctx
        .wallet_name
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
