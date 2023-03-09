mod ibc_proto;
pub(crate) mod transfer_v1;

pub(crate) const IBC_OUT_SOURCE_PORT: &str = "transfer";
pub(crate) const IBC_OUT_TIMEOUT_IN_NANOS: u64 = 60000000000 * 15; // 15 minutes
pub(crate) const IBC_GAS_LIMIT_DEFAULT: u64 = 150_000;
