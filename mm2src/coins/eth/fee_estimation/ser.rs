//! Serializable version of fee estimation data.
use crate::eth::{fee_estimation::eip1559, wei_to_gwei_decimal};
use crate::NumConversError;
use mm2_err_handle::mm_error::MmError;
use mm2_number::BigDecimal;

use std::convert::TryFrom;

/// Estimated fee per gas units
#[derive(Serialize)]
pub enum EstimationUnits {
    Gwei,
}

/// Priority level estimated max fee per gas
#[derive(Serialize)]
pub struct FeePerGasLevel {
    /// estimated max priority tip fee per gas in gwei
    pub max_priority_fee_per_gas: BigDecimal,
    /// estimated max fee per gas in gwei
    pub max_fee_per_gas: BigDecimal,
    /// estimated transaction min wait time in mempool in ms for this priority level
    pub min_wait_time: Option<u32>,
    /// estimated transaction max wait time in mempool in ms for this priority level
    pub max_wait_time: Option<u32>,
}

/// External struct for estimated fee per gas for several priority levels, in gwei
/// low/medium/high levels are supported
#[derive(Serialize)]
pub struct FeePerGasEstimated {
    /// base fee for the next block in gwei
    pub base_fee: BigDecimal,
    /// estimated low priority fee
    pub low: FeePerGasLevel,
    /// estimated medium priority fee
    pub medium: FeePerGasLevel,
    /// estimated high priority fee
    pub high: FeePerGasLevel,
    /// which estimator used
    pub source: String,
    /// base trend (up or down)
    pub base_fee_trend: String,
    /// priority trend (up or down)
    pub priority_fee_trend: String,
    /// fee units
    pub units: EstimationUnits,
}

impl TryFrom<eip1559::FeePerGasEstimated> for FeePerGasEstimated {
    type Error = MmError<NumConversError>;

    fn try_from(fees: eip1559::FeePerGasEstimated) -> Result<Self, Self::Error> {
        Ok(Self {
            base_fee: wei_to_gwei_decimal(fees.base_fee)?,
            low: FeePerGasLevel {
                max_fee_per_gas: wei_to_gwei_decimal(fees.low.max_fee_per_gas)?,
                max_priority_fee_per_gas: wei_to_gwei_decimal(fees.low.max_priority_fee_per_gas)?,
                min_wait_time: fees.low.min_wait_time,
                max_wait_time: fees.low.max_wait_time,
            },
            medium: FeePerGasLevel {
                max_fee_per_gas: wei_to_gwei_decimal(fees.medium.max_fee_per_gas)?,
                max_priority_fee_per_gas: wei_to_gwei_decimal(fees.medium.max_priority_fee_per_gas)?,
                min_wait_time: fees.medium.min_wait_time,
                max_wait_time: fees.medium.max_wait_time,
            },
            high: FeePerGasLevel {
                max_fee_per_gas: wei_to_gwei_decimal(fees.high.max_fee_per_gas)?,
                max_priority_fee_per_gas: wei_to_gwei_decimal(fees.high.max_priority_fee_per_gas)?,
                min_wait_time: fees.high.min_wait_time,
                max_wait_time: fees.high.max_wait_time,
            },
            source: fees.source.to_string(),
            base_fee_trend: fees.base_fee_trend,
            priority_fee_trend: fees.priority_fee_trend,
            units: EstimationUnits::Gwei,
        })
    }
}
