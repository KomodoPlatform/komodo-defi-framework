use crate::key_derivation::{derive_keys_for_mnemonic, KeyDerivationError};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use aes::Aes256;
use argon2::password_hash::SaltString;
use bip39::{Language, Mnemonic};
use common::drop_mutability;
use derive_more::Display;
use hmac::{Hmac, Mac};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use sha2::Sha256;

const ARGON2_ALGORITHM: &str = "Argon2id";
const ARGON2ID_VERSION: &str = "0x13";
const ARGON2ID_M_COST: u32 = 65536;
const ARGON2ID_T_COST: u32 = 2;
const ARGON2ID_P_COST: u32 = 1;
const DEFAULT_WORD_COUNT: u64 = 12;

type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;

#[derive(Debug, Display, PartialEq)]
pub enum MnemonicError {
    #[display(fmt = "BIP39 mnemonic error: {}", _0)]
    BIP39Error(String),
    #[display(fmt = "Error generating random bytes: {}", _0)]
    UnableToGenerateRandomBytes(String),
    #[display(fmt = "Error deriving key: {}", _0)]
    KeyDerivationError(String),
    #[display(fmt = "AES cipher error: {}", _0)]
    AESCipherError(String),
    #[display(fmt = "Error decoding string: {}", _0)]
    DecodeError(String),
    #[display(fmt = "Error verifying HMAC tag: {}", _0)]
    HMACError(String),
    Internal(String),
}

impl From<bip39::Error> for MnemonicError {
    fn from(e: bip39::Error) -> Self { MnemonicError::BIP39Error(e.to_string()) }
}

impl From<argon2::password_hash::Error> for MnemonicError {
    fn from(e: argon2::password_hash::Error) -> Self { MnemonicError::KeyDerivationError(e.to_string()) }
}

impl From<base64::DecodeError> for MnemonicError {
    fn from(e: base64::DecodeError) -> Self { MnemonicError::DecodeError(e.to_string()) }
}

impl From<KeyDerivationError> for MnemonicError {
    fn from(e: KeyDerivationError) -> Self { MnemonicError::KeyDerivationError(e.to_string()) }
}

/// Enum representing different encryption algorithms.
#[derive(Serialize, Deserialize, Debug)]
enum EncryptionAlgorithm {
    /// AES-256-CBC algorithm.
    AES256CBC,
    // Placeholder for future algorithms.
    // Future algorithms can be added here.
}

/// Parameters for the Argon2 key derivation function.
///
/// This struct defines the configuration parameters used by Argon2, one of the
/// most secure and widely used key derivation functions, especially for
/// password hashing.
#[derive(Serialize, Deserialize, Debug)]
pub struct Argon2Params {
    /// The specific variant of the Argon2 algorithm used (e.g., Argon2id).
    algorithm: String,

    /// The version of the Argon2 algorithm (e.g., 0x13 for the latest version).
    version: String,

    /// The memory cost parameter defining the memory usage of the algorithm.
    /// Expressed in kibibytes (KiB).
    m_cost: u32,

    /// The time cost parameter defining the execution time and number of
    /// iterations of the algorithm.
    t_cost: u32,

    /// The parallelism cost parameter defining the number of parallel threads.
    p_cost: u32,
}

impl Default for Argon2Params {
    fn default() -> Self {
        Argon2Params {
            algorithm: ARGON2_ALGORITHM.to_string(),
            version: ARGON2ID_VERSION.to_string(),
            m_cost: ARGON2ID_M_COST,
            t_cost: ARGON2ID_T_COST,
            p_cost: ARGON2ID_P_COST,
        }
    }
}

/// Enum representing different key derivation details.
///
/// This enum allows for flexible specification of various key derivation
/// algorithms and their parameters, making it easier to extend and support
/// multiple algorithms in the future.
#[derive(Serialize, Deserialize, Debug)]
pub enum KeyDerivationDetails {
    /// Argon2 algorithm with its specific parameters.
    Argon2(Argon2Params),
    // Placeholder for future algorithms.
    // Future algorithms can be added here.
}

impl Default for KeyDerivationDetails {
    fn default() -> Self { KeyDerivationDetails::Argon2(Argon2Params::default()) }
}

/// Represents encrypted mnemonic data for a wallet.
///
/// This struct encapsulates all essential components required to securely encrypt
/// and subsequently decrypt a wallet mnemonic. It is designed to be self-contained,
/// meaning it includes not only the encrypted data but also all the necessary metadata
/// and parameters for decryption. This makes the struct portable and convenient for
/// use in various scenarios, allowing decryption of the mnemonic in different
/// environments or applications, provided the correct password is supplied.
///
/// It includes the following:
/// - The encryption algorithm used, ensuring compatibility during decryption.
/// - Detailed key derivation details, including the algorithm and its parameters,
///   essential for recreating the encryption key from the user's password.
/// - The Base64-encoded salt for AES key derivation, IV (Initialization Vector),
///   and the ciphertext itself.
/// - If HMAC is used, it also includes the salt for HMAC key derivation and the HMAC tag,
///   which are crucial for ensuring the integrity and authenticity of the encrypted data.
///
/// The structure is typically used for wallet encryption in blockchain-based applications,
/// providing a robust and comprehensive approach to securing sensitive mnemonic data..
#[derive(Serialize, Deserialize, Debug)]
pub struct EncryptedMnemonicData {
    /// The encryption algorithm used to encrypt the mnemonic.
    /// Example: "AES-256-CBC".
    encryption_algorithm: EncryptionAlgorithm,

    /// Detailed information about the key derivation process. This includes
    /// the specific algorithm used (e.g., Argon2) and its parameters.
    key_derivation_details: KeyDerivationDetails,

    /// The salt used in the key derivation process for the AES key.
    /// Stored as a Base64-encoded string.
    salt_aes: String,

    /// The initialization vector (IV) used in the AES encryption process.
    /// The IV ensures that the encryption process produces unique ciphertext
    /// for the same plaintext and key when encrypted multiple times.
    /// Stored as a Base64-encoded string.
    iv: String,

    /// The encrypted mnemonic data. This is the ciphertext generated
    /// using the specified encryption algorithm, key, and IV.
    /// Stored as a Base64-encoded string.
    ciphertext: String,

    /// The salt used in the key derivation process for the HMAC key.
    /// This is applicable if HMAC is used for ensuring data integrity and authenticity.
    /// Stored as a Base64-encoded string.
    salt_hmac: String,

    /// The HMAC tag used for verifying the integrity and authenticity of the encrypted data.
    /// This tag is crucial for validating that the data has not been tampered with.
    /// Stored as a Base64-encoded string.
    tag: String,
}

/// Generates a new mnemonic passphrase.
///
/// This function creates a new mnemonic passphrase using a specified word count and randomness source.
/// The generated mnemonic is intended for use as a wallet mnemonic.
///
/// # Arguments
/// * `ctx` - The `MmArc` context containing the application configuration.
///
/// # Returns
/// `MmInitResult<String>` - The generated mnemonic passphrase or an error if generation fails.
///
/// # Errors
/// Returns `MmInitError::Internal` if mnemonic generation fails.
pub fn generate_mnemonic(ctx: &MmArc) -> MmResult<Mnemonic, MnemonicError> {
    let mut rng = bip39::rand_core::OsRng;
    let word_count = ctx.conf["word_count"].as_u64().unwrap_or(DEFAULT_WORD_COUNT) as usize;
    let mnemonic = Mnemonic::generate_in_with(&mut rng, Language::English, word_count)?;
    Ok(mnemonic)
}

/// Encrypts a mnemonic phrase using a specified password.
///
/// This function performs several operations:
/// - It generates salts for AES and HMAC key derivation.
/// - It derives the keys using the Argon2 algorithm.
/// - It encrypts the mnemonic using AES-256-CBC.
/// - It creates an HMAC tag for verifying the integrity and authenticity of the encrypted data.
///
/// # Arguments
/// * `mnemonic` - A `&str` reference to the mnemonic that needs to be encrypted.
/// * `password` - A `&str` reference to the password used for key derivation.
///
/// # Returns
/// `MmResult<EncryptedMnemonicData, MnemonicError>` - The result is either an `EncryptedMnemonicData`
/// struct containing all the necessary components for decryption, or a `MnemonicError` in case of failure.
///
/// # Errors
/// This function can return various errors related to key derivation, encryption, and data encoding.
pub fn encrypt_mnemonic(mnemonic: &str, password: &str) -> MmResult<EncryptedMnemonicData, MnemonicError> {
    use argon2::password_hash::rand_core::OsRng;

    // Generate salt for AES key
    let salt_aes = SaltString::generate(&mut OsRng);

    // Generate salt for HMAC key
    let salt_hmac = SaltString::generate(&mut OsRng);

    // Generate IV
    let mut iv = [0u8; 16];
    common::os_rng(&mut iv).map_to_mm(|e| MnemonicError::UnableToGenerateRandomBytes(e.to_string()))?;
    drop_mutability!(iv);

    // Derive AES and HMAC keys
    let (key_aes, key_hmac) = derive_keys_for_mnemonic(password, &salt_aes, &salt_hmac)?;

    // Create an AES-256-CBC cipher instance, encrypt the data with the key and the IV and get the ciphertext
    let msg_len = mnemonic.len();
    let buffer_len = msg_len + 16 - (msg_len % 16);
    let mut buffer = vec![0u8; buffer_len];
    buffer[..msg_len].copy_from_slice(mnemonic.as_bytes());
    let ciphertext = Aes256CbcEnc::new(&key_aes.into(), &iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, msg_len)
        .map_to_mm(|e| MnemonicError::AESCipherError(e.to_string()))?;

    // Create HMAC tag
    let mut mac = Hmac::<Sha256>::new_from_slice(&key_hmac).map_to_mm(|e| MnemonicError::Internal(e.to_string()))?;
    mac.update(ciphertext);
    mac.update(&iv);
    let tag = mac.finalize().into_bytes();

    let encrypted_mnemonic_data = EncryptedMnemonicData {
        encryption_algorithm: EncryptionAlgorithm::AES256CBC,
        key_derivation_details: KeyDerivationDetails::default(),
        salt_aes: salt_aes.as_str().to_string(),
        iv: base64::encode(&iv),
        ciphertext: base64::encode(&ciphertext),
        salt_hmac: salt_hmac.as_str().to_string(),
        tag: base64::encode(&tag),
    };

    Ok(encrypted_mnemonic_data)
}

/// Decrypts an encrypted mnemonic phrase using a specified password.
///
/// This function performs the reverse operations of `encrypt_mnemonic`. It:
/// - Decodes and re-creates the necessary salts, IV, and ciphertext from the `EncryptedMnemonicData`.
/// - Derives the AES and HMAC keys using the Argon2 algorithm.
/// - Verifies the integrity and authenticity of the data using the HMAC tag.
/// - Decrypts the mnemonic using AES-256-CBC.
///
/// # Arguments
/// * `encrypted_data` - A reference to the `EncryptedMnemonicData` containing the encrypted mnemonic and related metadata.
/// * `password` - A `&str` reference to the password used for key derivation.
///
/// # Returns
/// `MmResult<Mnemonic, MnemonicError>` - The result is either a `Mnemonic` instance if decryption is successful,
/// or a `MnemonicError` in case of failure.
///
/// # Errors
/// This function can return various errors related to decoding, key derivation, encryption, and HMAC verification.
pub fn decrypt_mnemonic(encrypted_data: &EncryptedMnemonicData, password: &str) -> MmResult<Mnemonic, MnemonicError> {
    // Decode the Base64-encoded values
    let iv = base64::decode(&encrypted_data.iv)?;
    let mut ciphertext = base64::decode(&encrypted_data.ciphertext)?;
    let tag = base64::decode(&encrypted_data.tag)?;

    // Re-create the salts from Base64-encoded strings
    let salt_aes = SaltString::from_b64(&encrypted_data.salt_aes)?;
    let salt_hmac = SaltString::from_b64(&encrypted_data.salt_hmac)?;

    // Re-create the keys from the password and salts
    let (key_aes, key_hmac) = derive_keys_for_mnemonic(password, &salt_aes, &salt_hmac)?;

    // Verify HMAC tag before decrypting
    let mut mac = Hmac::<Sha256>::new_from_slice(&key_hmac).map_to_mm(|e| MnemonicError::Internal(e.to_string()))?;
    mac.update(&ciphertext);
    mac.update(&iv);
    mac.verify_slice(&tag)
        .map_to_mm(|e| MnemonicError::HMACError(e.to_string()))?;

    // Decrypt the ciphertext
    let decrypted_data = Aes256CbcDec::new(&key_aes.into(), iv.as_slice().into())
        .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
        .map_to_mm(|e| MnemonicError::AESCipherError(e.to_string()))?;

    // Convert decrypted data back to a string
    let mnemonic_str =
        String::from_utf8(decrypted_data.to_vec()).map_to_mm(|e| MnemonicError::DecodeError(e.to_string()))?;
    let mnemonic = Mnemonic::parse_normalized(&mnemonic_str)?;
    Ok(mnemonic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_mnemonic() {
        let mnemonic = "tank abandon bind salon remove wisdom net size aspect direct source fossil";
        let password = "password";

        // Verify that the mnemonic is valid
        let parsed_mnemonic = Mnemonic::parse_normalized(mnemonic);
        assert!(parsed_mnemonic.is_ok());
        let parsed_mnemonic = parsed_mnemonic.unwrap();

        // Encrypt the mnemonic
        let encrypted_data = encrypt_mnemonic(mnemonic, password);
        assert!(encrypted_data.is_ok());
        let encrypted_data = encrypted_data.unwrap();

        // Decrypt the mnemonic
        let decrypted_mnemonic = decrypt_mnemonic(&encrypted_data, password);
        assert!(decrypted_mnemonic.is_ok());
        let decrypted_mnemonic = decrypted_mnemonic.unwrap();

        // Verify if decrypted mnemonic matches the original
        assert_eq!(decrypted_mnemonic, parsed_mnemonic);
    }

    #[test]
    fn test_mnemonic_with_last_byte_zero() {
        let mnemonic = "tank abandon bind salon remove wisdom net size aspect direct source fossil\0".to_string();
        let password = "password";

        // Encrypt the mnemonic
        let encrypted_data = encrypt_mnemonic(&mnemonic, password);
        assert!(encrypted_data.is_ok());
        let encrypted_data = encrypted_data.unwrap();

        // Decrypt the mnemonic
        let decrypted_mnemonic = decrypt_mnemonic(&encrypted_data, password);
        assert!(decrypted_mnemonic.is_err());

        // Verify that the error is due to parsing and not padding
        assert!(decrypted_mnemonic
            .unwrap_err()
            .to_string()
            .contains("mnemonic contains an unknown word (word 11)"));
    }
}
