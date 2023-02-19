use crate::helpers_validation::SPVError;
use crate::work::{DifficultyAlgorithm, RETARGETING_INTERVAL};
use chain::{BlockHeader, BlockHeaderBits};
use primitives::hash::H256;
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use std::num::NonZeroU64;
use std::str::FromStr;

fn from_hash_str<'de, D>(deserializer: D) -> Result<H256, D::Error>
where
    D: Deserializer<'de>,
{
    let hash: String = Deserialize::deserialize(deserializer)?;
    let hash = H256::from_str(&hash).map_err(Error::custom)?;
    Ok(hash.reversed())
}

fn from_bit_u32<'de, D>(deserializer: D) -> Result<BlockHeaderBits, D::Error>
where
    D: Deserializer<'de>,
{
    let bits: u32 = Deserialize::deserialize(deserializer)?;
    Ok(BlockHeaderBits::Compact(bits.into()))
}

/// Custom SPV starting block header configuration
#[derive(Debug, Clone, Deserialize)]
pub struct SPVBlockHeader {
    /// Valid `u32` representation of the block `height`.
    pub height: u64,
    /// Valid block header `hash`.
    #[serde(deserialize_with = "from_hash_str")]
    pub hash: H256,
    /// Valid `u32` representation of the `date` the block is mined in epoch.
    pub time: u32,
    /// Valid block header `bits`.
    #[serde(deserialize_with = "from_bit_u32")]
    pub bits: BlockHeaderBits,
}

impl SPVBlockHeader {
    pub(crate) fn from_block_header_and_height(header: &BlockHeader, height: u64) -> Self {
        Self {
            height,
            hash: header.hash(),
            time: header.time,
            bits: header.bits.clone(),
        }
    }
}

/// Validate that `max_stored_headers_value` is always greater than `retarget interval`.
fn validate_btc_max_stored_headers_value(max_stored_block_headers: u64) -> Result<(), SPVError> {
    if RETARGETING_INTERVAL > max_stored_block_headers as u32 {
        return Err(SPVError::InitialValidationError(format!(
            "max_stored_block_headers {max_stored_block_headers} must be greater than retargeting interval {RETARGETING_INTERVAL}",
        )));
    }

    Ok(())
}

/// Validate that starting block header is a retarget header.
fn validate_btc_spv_header_height(coin: &str, height: u64) -> Result<(), SPVError> {
    let is_retarget = height % RETARGETING_INTERVAL as u64;
    if is_retarget != 0 {
        return Err(SPVError::WrongRetargetHeight {
            coin: coin.to_string(),
            expected_height: height - is_retarget,
        });
    }

    Ok(())
}

/// Custom SPV block header configuration
#[derive(Clone, Debug, Deserialize)]
pub struct BlockHeaderValidationParams {
    pub difficulty_check: bool,
    pub constant_difficulty: bool,
    pub difficulty_algorithm: Option<DifficultyAlgorithm>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SPVConf {
    /// Where to start block headers sync from.
    pub starting_block_header: SPVBlockHeader,
    /// Max number of block headers to be stored in db, when reached, excessive headers will be deleted.
    pub max_stored_block_headers: Option<NonZeroU64>,
    /// The parameters that specify how the coin block headers should be validated. If None,
    /// headers will be saved in DB without validation, can be none if the coin's RPC server is trusted.
    pub validation_params: Option<BlockHeaderValidationParams>,
}

impl SPVConf {
    pub fn validate(&self, coin: &str) -> Result<(), SPVError> {
        if let Some(params) = &self.validation_params {
            if let Some(algo) = &params.difficulty_algorithm {
                match algo {
                    DifficultyAlgorithm::BitcoinMainnet => {
                        validate_btc_spv_header_height(coin, self.starting_block_header.height)?;
                        if let Some(max) = self.max_stored_block_headers {
                            validate_btc_max_stored_headers_value(max.into())?;
                        }
                    },
                    DifficultyAlgorithm::BitcoinTestnet => {
                        return Err(SPVError::Internal("Bitcoin Testnet is not supported yet.".to_string()))
                    },
                }
            }
        }

        Ok(())
    }

    /// Validate Starting block header from `RPC` against [`SPVConf::SPVBlockHeader`]
    pub fn validate_rpc_starting_header(&self, height: u64, rpc_header: &BlockHeader) -> Result<(), SPVError> {
        let rpc_header = SPVBlockHeader::from_block_header_and_height(rpc_header, height);
        let spv_header = &self.starting_block_header;

        // Currently, BlockHeader::Compact is used in spv block header validation but some coins from rpc will return
        // BlockHeader::U32, therefore, we need only the inner value in this case to validate the header bits.
        let rpc_header_bits: u32 = rpc_header.bits.into();
        let spv_header_bits: u32 = spv_header.bits.clone().into();
        if rpc_header_bits != spv_header_bits {
            return Err(SPVError::InitialValidationError(format!(
                "Starting block header bits not acceptable - expected: {spv_header_bits} - found: {rpc_header_bits}"
            )));
        };

        if rpc_header.hash != spv_header.hash {
            return Err(SPVError::InitialValidationError(format!(
                "Starting block header hash not acceptable - expected: {} - found: {}",
                spv_header.hash, rpc_header.hash,
            )));
        };

        if rpc_header.time != spv_header.time {
            return Err(SPVError::InitialValidationError(format!(
                "Starting block header time not acceptable - expected: {} - found: {}",
                spv_header.time, rpc_header.time,
            )));
        };

        Ok(())
    }
}
