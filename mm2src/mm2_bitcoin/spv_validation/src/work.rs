use crate::storage::BlockHeaderStorageOps;
use chain::{BlockHeader, BlockHeaderBits};
use derive_more::Display;
use keys::Network;
use lazy_static::lazy_static;
use primitives::compact::Compact;
use primitives::U256;
use std::cmp;

const RETARGETING_FACTOR: u32 = 4;
const TARGET_SPACING_SECONDS: u32 = 10 * 60;
const TARGET_TIMESPAN_SECONDS: u32 = 2 * 7 * 24 * 60 * 60;

/// The Target number of blocks equals to 2 weeks or 2016 blocks
const RETARGETING_INTERVAL: u32 = TARGET_TIMESPAN_SECONDS / TARGET_SPACING_SECONDS;

/// The upper and lower bounds for retargeting timespan
const MIN_TIMESPAN: u32 = TARGET_TIMESPAN_SECONDS / RETARGETING_FACTOR;
const MAX_TIMESPAN: u32 = TARGET_TIMESPAN_SECONDS * RETARGETING_FACTOR;

lazy_static! {
    static ref MAX_BITS_MAINNET: U256 = "00000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        .parse()
        .expect("hardcoded value should parse without errors");
}

fn is_retarget_height(height: u32) -> bool { height % RETARGETING_INTERVAL == 0 }

#[derive(Debug, Display)]
pub enum NextBlockBitsError {
    #[display(fmt = "Can't calculate next block bits for Network {:?}", _0)]
    UnsupportedNetwork(Network),
    Internal(String),
}

// todo: complete this code
fn next_block_bits(network: Network) -> Result<BlockHeaderBits, NextBlockBitsError> {
    match network {
        Network::Mainnet => unimplemented!(),
        Network::Testnet => unimplemented!(),
        // todo: add extensive???
        network => Err(NextBlockBitsError::UnsupportedNetwork(network)),
    }
}

fn range_constrain(value: i64, min: i64, max: i64) -> i64 { cmp::min(cmp::max(value, min), max) }

/// Returns constrained number of seconds since last retarget
fn retarget_timespan(retarget_timestamp: u32, last_timestamp: u32) -> u32 {
    // subtract unsigned 32 bit numbers in signed 64 bit space in
    // order to prevent underflow before applying the range constraint.
    let timespan = last_timestamp as i64 - retarget_timestamp as i64;
    range_constrain(timespan, MIN_TIMESPAN as i64, MAX_TIMESPAN as i64) as u32
}

pub async fn btc_mainnet_next_block_bits(
    last_block_header: BlockHeader,
    last_block_height: u32,
    store: &dyn BlockHeaderStorageOps,
) -> Result<BlockHeaderBits, NextBlockBitsError> {
    if last_block_height == 0 {
        return Err(NextBlockBitsError::Internal("Last block height can't be zero".into()));
    }

    let height = last_block_height + 1;

    if height % RETARGETING_INTERVAL == 0 {
        let retarget_ref = (height - RETARGETING_INTERVAL).into();
        // todo: remove hardcoded "BTC"
        let retarget_header = store.get_block_header("BTC", retarget_ref).await.unwrap().unwrap();
        // timestamp of block(height - RETARGETING_INTERVAL)
        let retarget_timestamp = retarget_header.time;
        // timestamp of current block
        let last_timestamp = last_block_header.time;

        let retarget: Compact = last_block_header.bits.into();
        let retarget: U256 = retarget.into();
        let retarget_timespan: U256 = retarget_timespan(retarget_timestamp, last_timestamp).into();
        let retarget: U256 = retarget * retarget_timespan;
        let target_timespan_seconds: U256 = TARGET_TIMESPAN_SECONDS.into();
        let retarget = retarget / target_timespan_seconds;

        let maximum = *MAX_BITS_MAINNET;

        if retarget > maximum {
            Ok(BlockHeaderBits::Compact(maximum.into()))
        } else {
            Ok(BlockHeaderBits::Compact(retarget.into()))
        }
    } else {
        Ok(last_block_header.bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    // todo: remove common from cargo.toml
    use crate::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
    use common::block_on;

    struct TestBlockHeadersStorage {}

    #[async_trait]
    impl BlockHeaderStorageOps for TestBlockHeadersStorage {
        async fn init(&self, _for_coin: &str) -> Result<(), BlockHeaderStorageError> { Ok(()) }

        async fn is_initialized_for(&self, _for_coin: &str) -> Result<bool, BlockHeaderStorageError> { Ok(true) }

        async fn add_block_headers_to_storage(
            &self,
            _for_coin: &str,
            _headers: HashMap<u64, BlockHeader>,
        ) -> Result<(), BlockHeaderStorageError> {
            Ok(())
        }

        async fn get_block_header(
            &self,
            _for_coin: &str,
            height: u64,
        ) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
            // https://live.blockcypher.com/btc/block/00000000a141216a896c54f211301c436e557a8d55900637bbdce14c6c7bddef/
            if height == 2016 {
                return Ok(Some("010000006397bb6abd4fc521c0d3f6071b5650389f0b4551bc40b4e6b067306900000000ace470aecda9c8818c8fe57688cd2a772b5a57954a00df0420a7dd546b6d2c576b0e7f49ffff001d33f0192f".into()));
            }

            // https://live.blockcypher.com/btc/block/00000000000000000012145f8ffa7218d2d04ca66b61835a2a5eaec33dffc098/
            if height == 604800 {
                return Ok(Some("000000208e244d2c55bc403caa5d6eaf0f922170e413eb1e02fb02000000000000000000e03b4d9df72d8db232a20bb2ff35c433a99f1467f391f75b5f62180d96f06d6aa4c4d65d3eb215179ef91633".into()));
            }

            Ok(None)
        }

        async fn get_block_header_raw(
            &self,
            _for_coin: &str,
            _height: u64,
        ) -> Result<Option<String>, BlockHeaderStorageError> {
            Ok(None)
        }
    }

    #[test]
    fn test_btc_mainnet_next_block_bits() {
        let storage = TestBlockHeadersStorage {};

        let last_header: BlockHeader = "000000201d758432ecd495a2177b44d3fe6c22af183461a0b9ea0d0000000000000000008283a1dfa795d9b68bd8c18601e443368265072cbf8c76bfe58de46edd303798035de95d3eb2151756fdb0e8".into();

        let next_block_bits = block_on(btc_mainnet_next_block_bits(last_header, 606815, &storage)).unwrap();

        assert_eq!(next_block_bits, BlockHeaderBits::Compact(387308498.into()));

        // check that bits for very early blocks that didn't change difficulty because of low hashrate is calculated correctly.
        let last_header: BlockHeader = "010000000d9c8c96715756b619116cc2160937fb26c655a2f8e28e3a0aff59c0000000007676252e8434de408ea31920d986aba297bd6f7c6f20756be08748713f7c135962719449ffff001df8c1cb01".into();

        let next_block_bits = block_on(btc_mainnet_next_block_bits(last_header, 4031, &storage)).unwrap();

        assert_eq!(next_block_bits, BlockHeaderBits::Compact(486604799.into()));

        // check that bits stay the same when the next block is not a retarget block
        // https://live.blockcypher.com/btc/block/00000000000000000002622f52b6afe70a5bb139c788e67f221ffc67a762a1e0/
        let last_header: BlockHeader = "00e0ff2f44d953fe12a047129bbc7164668c6d96f3e7a553528b02000000000000000000d0b950384cd23ab0854d1c8f23fa7a97411a6ffd92347c0a3aea4466621e4093ec09c762afa7091705dad220".into();

        let next_block_bits = block_on(btc_mainnet_next_block_bits(last_header, 744014, &storage)).unwrap();

        assert_eq!(next_block_bits, BlockHeaderBits::Compact(386508719.into()));
    }
}
